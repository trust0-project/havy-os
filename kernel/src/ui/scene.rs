//! Retained scene graph → complete HDL snapshot (Phase 4).
//!
//! Nodes are a fixed-size array (no `Vec` growth, no 3 MiB bitmap). `gpuid`
//! on hart 0 is the only mutator. Mailbox publication happens only from that
//! owner, and only when the scene is dirty.
//!
//! HDL mode (`MAILBOX_PRESENT`): publish a complete snapshot; after the first
//! successful publish (or `HOST_ACCEPTED`) the old `fill_rect` painter stops
//! for this boot. Fallback (D1 / QEMU / `?hdl=0`): `raster_soft` interprets
//! the same list into `GpuDriver`. Encode scratch lives in BSS, not on the
//! 4 KiB process kstack.

#![allow(dead_code)]

use core::fmt::Write;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};

use crate::platform::d1_display;
use crate::ui::hdl::{self, EncodeError, EncodeSpec, Op};
use crate::ui::hdl_mailbox;
use crate::ui::input_queue;
use crate::ui::raster_soft;
use crate::ui::{SCREEN_HEIGHT, SCREEN_WIDTH};
use crate::Spinlock;

/// Hard cap on retained nodes. Hundreds for the real desktop; bounded.
pub const MAX_NODES: usize = 256;
/// Encoder working set (Clear + chrome + windows + terminal rows).
pub const MAX_OPS: usize = 384;
/// Glyph side-array budget (labels + visible terminal rows).
pub const MAX_SIDE: usize = 16384;
/// Encode scratch: header + records + side. Always ≤ 64 KiB.
pub const ENCODE_CAP: usize = 49152;
/// Longest label (welcome line, cwd, terminal row).
pub const MAX_LABEL: usize = 96;
/// Button caption (`"Network"` / `"Cancel"`).
pub const MAX_BUTTON_LABEL: usize = 16;

pub const ATLAS_UI: u16 = 0;
pub const ATLAS_BOLD: u16 = 1;
pub const TEX_LOGO: u16 = 0;
pub const TEX_LOGO_SMALL: u16 = 1;
pub const TEX_CURSOR: u16 = 2;

const CLOCK_INTERVAL_MS: u64 = 1000;
const BTN_RADIUS: u16 = 4;
const CURSOR_W: u16 = 12;
const CURSOR_H: u16 = 16;

const _: () = {
    assert!(MAX_NODES >= 64);
    assert!(MAX_OPS >= 64);
    assert!(MAX_SIDE >= 256);
    assert!(ENCODE_CAP >= 32 + MAX_OPS * 32 + MAX_SIDE);
    assert!(ENCODE_CAP <= hdl::MAX_FRAME_BYTES);
};

const DESKTOP_RGB: (u8, u8, u8) = (0x15, 0x15, 0x1E);

/// Mouse cursor drawn as the last HDL record (D1 / unpublished FB only).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CursorKind {
    Hidden,
    Arrow,
    IBeam,
}

/// Retained node kinds from the roadmap (`ui/scene.rs`).
#[derive(Clone, Copy)]
pub enum Node {
    Empty,
    Panel {
        x: i16,
        y: i16,
        w: u16,
        h: u16,
        color: u32,
        radius: u16,
    },
    Label {
        x: i16,
        y: i16,
        color: u32,
        atlas_id: u16,
        text: [u8; MAX_LABEL],
        len: u8,
    },
    Button {
        x: i16,
        y: i16,
        w: u16,
        h: u16,
        selected: bool,
        label: [u8; MAX_BUTTON_LABEL],
        len: u8,
    },
    GlyphRun {
        x: i16,
        y: i16,
        color: u32,
        atlas_id: u16,
        text: [u8; MAX_LABEL],
        len: u8,
    },
    Image {
        x: i16,
        y: i16,
        w: u16,
        h: u16,
        tex_id: u16,
    },
    Line {
        x0: i16,
        y0: i16,
        x1: i16,
        y1: i16,
        color: u32,
        width: u16,
    },
    Dot {
        x: i16,
        y: i16,
        d: u16,
        color: u32,
    },
    Check {
        x: i16,
        y: i16,
    },
    Progress {
        x: i16,
        y: i16,
        w: u16,
        h: u16,
        fill_w: u16,
    },
    Caret {
        x: i16,
        y: i16,
        h: u16,
        color: u32,
    },
    ClipPush {
        x: i16,
        y: i16,
        w: u16,
        h: u16,
    },
    ClipPop,
}

struct Scene {
    clear: u32,
    width: u16,
    height: u16,
    nodes: [Node; MAX_NODES],
    node_count: usize,
    cursor_x: i16,
    cursor_y: i16,
    cursor_kind: CursorKind,
}

impl Scene {
    const fn empty() -> Self {
        Self {
            clear: 0,
            width: 0,
            height: 0,
            nodes: [Node::Empty; MAX_NODES],
            node_count: 0,
            cursor_x: 0,
            cursor_y: 0,
            cursor_kind: CursorKind::Hidden,
        }
    }
}

struct Scratch {
    ops: [Op; MAX_OPS],
    side: [u8; MAX_SIDE],
    buf: [u8; ENCODE_CAP],
}

impl Scratch {
    const fn new() -> Self {
        Self {
            ops: [Op::Nop; MAX_OPS],
            side: [0u8; MAX_SIDE],
            buf: [0u8; ENCODE_CAP],
        }
    }
}

static SCENE: Spinlock<Scene> = Spinlock::new(Scene::empty());
static SCRATCH: Spinlock<Scratch> = Spinlock::new(Scratch::new());
static DIRTY: AtomicBool = AtomicBool::new(false);
static LAST_CLOCK_MS: AtomicU64 = AtomicU64::new(0);
static PUBLISHED_ONCE: AtomicBool = AtomicBool::new(false);
static SKIP_IMMEDIATE: AtomicBool = AtomicBool::new(false);
static ENCODE_FAIL_LOGGED: AtomicBool = AtomicBool::new(false);
static CURSOR_PUB_X: AtomicI32 = AtomicI32::new(i32::MIN);
static CURSOR_PUB_Y: AtomicI32 = AtomicI32::new(i32::MIN);

/// Builder used by widgets / `main_screen` to emit nodes (not `fill_rect`).
pub struct Builder<'a> {
    scene: &'a mut Scene,
}

impl Builder<'_> {
    pub fn panel(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        r: u8,
        g: u8,
        b: u8,
        radius: u16,
    ) {
        push_node(
            self.scene,
            Node::Panel {
                x: pos_i16(x),
                y: pos_i16(y),
                w: dim_u16(w as i32),
                h: dim_u16(h as i32),
                color: opaque(r, g, b),
                radius,
            },
        );
    }

    pub fn label(&mut self, x: i32, y: i32, r: u8, g: u8, b: u8, atlas_id: u16, text: &[u8]) {
        let mut buf = [0u8; MAX_LABEL];
        let len = copy_text(&mut buf, text);
        push_node(
            self.scene,
            Node::Label {
                x: pos_i16(x),
                y: pos_i16(y),
                color: opaque(r, g, b),
                atlas_id,
                text: buf,
                len,
            },
        );
    }

    pub fn label_str(&mut self, x: i32, y: i32, r: u8, g: u8, b: u8, atlas_id: u16, text: &str) {
        self.label(x, y, r, g, b, atlas_id, text.as_bytes());
    }

    pub fn line(
        &mut self,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        r: u8,
        g: u8,
        b: u8,
        width: u16,
    ) {
        push_node(
            self.scene,
            Node::Line {
                x0: pos_i16(x0),
                y0: pos_i16(y0),
                x1: pos_i16(x1),
                y1: pos_i16(y1),
                color: opaque(r, g, b),
                width,
            },
        );
    }

    pub fn button(&mut self, x: i32, y: i32, w: u32, h: u32, selected: bool, label: &[u8]) {
        let mut buf = [0u8; MAX_BUTTON_LABEL];
        let len = copy_text(&mut buf, label);
        push_node(
            self.scene,
            Node::Button {
                x: pos_i16(x),
                y: pos_i16(y),
                w: dim_u16(w as i32),
                h: dim_u16(h as i32),
                selected,
                label: buf,
                len,
            },
        );
    }

    pub fn image(&mut self, x: i32, y: i32, w: u32, h: u32, tex_id: u16) {
        push_node(
            self.scene,
            Node::Image {
                x: pos_i16(x),
                y: pos_i16(y),
                w: dim_u16(w as i32),
                h: dim_u16(h as i32),
                tex_id,
            },
        );
    }

    pub fn dot(&mut self, x: i32, y: i32, d: u32, r: u8, g: u8, b: u8) {
        push_node(
            self.scene,
            Node::Dot {
                x: pos_i16(x),
                y: pos_i16(y),
                d: dim_u16(d as i32),
                color: opaque(r, g, b),
            },
        );
    }

    pub fn check(&mut self, x: i32, y: i32) {
        push_node(
            self.scene,
            Node::Check {
                x: pos_i16(x),
                y: pos_i16(y),
            },
        );
    }

    pub fn progress(&mut self, x: i32, y: i32, w: u32, h: u32, fill_w: u32) {
        push_node(
            self.scene,
            Node::Progress {
                x: pos_i16(x),
                y: pos_i16(y),
                w: dim_u16(w as i32),
                h: dim_u16(h as i32),
                fill_w: dim_u16(fill_w as i32),
            },
        );
    }

    pub fn caret(&mut self, x: i32, y: i32, h: u32, r: u8, g: u8, b: u8) {
        push_node(
            self.scene,
            Node::Caret {
                x: pos_i16(x),
                y: pos_i16(y),
                h: dim_u16(h as i32),
                color: opaque(r, g, b),
            },
        );
    }

    pub fn clip_push(&mut self, x: i32, y: i32, w: u32, h: u32) {
        push_node(
            self.scene,
            Node::ClipPush {
                x: pos_i16(x),
                y: pos_i16(y),
                w: dim_u16(w as i32),
                h: dim_u16(h as i32),
            },
        );
    }

    pub fn clip_pop(&mut self) {
        push_node(self.scene, Node::ClipPop);
    }

    pub fn set_cursor(&mut self, x: i32, y: i32, kind: CursorKind) {
        self.scene.cursor_x = pos_i16(x);
        self.scene.cursor_y = pos_i16(y);
        self.scene.cursor_kind = kind;
    }

    /// Chrome matching `Window::draw_fast` (shadow optional, traffic lights optional).
    pub fn window_chrome(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        title: &[u8],
        controls: bool,
        shadow: bool,
        bg: (u8, u8, u8),
        title_bg: (u8, u8, u8),
        title_rgb: (u8, u8, u8),
    ) {
        if shadow {
            self.panel(x + 8, y + 8, w, h, 5, 5, 10, 0);
        }
        self.panel(x, y, w, h, bg.0, bg.1, bg.2, 0);
        self.line(x, y, x + w as i32 - 1, y, 60, 60, 80, 1);
        self.line(
            x,
            y + h as i32 - 1,
            x + w as i32 - 1,
            y + h as i32 - 1,
            60,
            60,
            80,
            1,
        );
        self.line(x, y, x, y + h as i32 - 1, 60, 60, 80, 1);
        self.line(
            x + w as i32 - 1,
            y,
            x + w as i32 - 1,
            y + h as i32 - 1,
            60,
            60,
            80,
            1,
        );
        self.panel(x, y, w, 32, title_bg.0, title_bg.1, title_bg.2, 0);
        if controls {
            self.dot(x + 12, y + 10, 12, 220, 80, 80);
            self.dot(x + 32, y + 10, 12, 230, 180, 80);
            self.dot(x + 52, y + 10, 12, 80, 200, 120);
        }
        let title_x = x + (w as i32 / 2) - ((title.len() as i32 * 9) / 2);
        self.label(
            title_x,
            y + 22,
            title_rgb.0,
            title_rgb.1,
            title_rgb.2,
            ATLAS_BOLD,
            title,
        );
        let logo_x = x + w as i32 - crate::ui::LOGO_SMALL_SIZE as i32 - 8;
        self.image(
            logo_x,
            y + 4,
            crate::ui::LOGO_SMALL_SIZE,
            crate::ui::LOGO_SMALL_SIZE,
            TEX_LOGO_SMALL,
        );
    }
}

fn mark_dirty() {
    DIRTY.store(true, Ordering::Release);
}

/// Widgets / input: scene mutation, not a 3 MiB restore.
pub fn notify_dirty() {
    mark_dirty();
}

/// True when gpuid should not idle-sleep: a snapshot is waiting or the clock is due.
#[inline]
pub fn wants_tick() -> bool {
    DIRTY.load(Ordering::Acquire) || clock_due()
}

#[inline]
pub fn is_dirty() -> bool {
    DIRTY.load(Ordering::Acquire)
}

/// Old immediate painter (`fill_rect` in `main_screen`). False after HDL
/// publish on virt, or after a successful `raster_soft` present on fallback.
pub fn use_immediate_painter() -> bool {
    if SKIP_IMMEDIATE.load(Ordering::Acquire) {
        return false;
    }
    if hdl_mailbox::host_accepted() {
        return false;
    }
    true
}

/// Native / D1 / unpublished FB: guest cursor is the last HDL record.
/// Browser HDL (mailbox present): CSS cursor; mouse-move must not republish.
pub fn include_guest_cursor() -> bool {
    !hdl_mailbox::mailbox_present()
}

fn clock_due() -> bool {
    let now = crate::get_time_ms() as u64;
    let last = LAST_CLOCK_MS.load(Ordering::Relaxed);
    now.wrapping_sub(last) >= CLOCK_INTERVAL_MS
}

pub(crate) fn pos_i16(v: i32) -> i16 {
    v.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

fn dim_u16(v: i32) -> u16 {
    v.clamp(0, u16::MAX as i32) as u16
}

pub fn opaque(r: u8, g: u8, b: u8) -> u32 {
    hdl::pack_aarrggbb(0xFF, r, g, b)
}

fn copy_text<const N: usize>(dst: &mut [u8; N], src: &[u8]) -> u8 {
    let n = src.len().min(N);
    dst[..n].copy_from_slice(&src[..n]);
    if n < N {
        dst[n..].fill(0);
    }
    n as u8
}

/// Owner-only: reset graph and mark dirty. `main_screen` then [`rebuild`]s.
pub fn init() {
    if !input_queue::is_owner() {
        return;
    }
    SKIP_IMMEDIATE.store(false, Ordering::Release);
    PUBLISHED_ONCE.store(false, Ordering::Relaxed);
    CURSOR_PUB_X.store(i32::MIN, Ordering::Relaxed);
    CURSOR_PUB_Y.store(i32::MIN, Ordering::Relaxed);
    let mut scene = SCENE.lock();
    scene.clear = opaque(DESKTOP_RGB.0, DESKTOP_RGB.1, DESKTOP_RGB.2);
    scene.width = dim_u16(SCREEN_WIDTH);
    scene.height = dim_u16(SCREEN_HEIGHT);
    scene.nodes = [Node::Empty; MAX_NODES];
    scene.node_count = 0;
    scene.cursor_kind = CursorKind::Hidden;
    LAST_CLOCK_MS.store(crate::get_time_ms() as u64, Ordering::Relaxed);
    drop(scene);
    mark_dirty();
}

/// Phase 2 name kept so call sites can migrate.
#[inline]
pub fn init_slice() {
    init();
}

fn push_node(scene: &mut Scene, node: Node) {
    if scene.node_count >= MAX_NODES {
        return;
    }
    scene.nodes[scene.node_count] = node;
    scene.node_count += 1;
}

/// Replace the node list. Caller must be the owner (hart 0).
pub fn rebuild<F: FnOnce(&mut Builder<'_>)>(f: F) {
    if !input_queue::is_owner() {
        return;
    }
    let mut scene = SCENE.lock();
    scene.clear = opaque(DESKTOP_RGB.0, DESKTOP_RGB.1, DESKTOP_RGB.2);
    scene.width = dim_u16(SCREEN_WIDTH);
    scene.height = dim_u16(SCREEN_HEIGHT);
    scene.nodes = [Node::Empty; MAX_NODES];
    scene.node_count = 0;
    scene.cursor_kind = CursorKind::Hidden;
    {
        let mut b = Builder {
            scene: &mut *scene,
        };
        f(&mut b);
    }
    drop(scene);
    mark_dirty();
}

/// Pull clock / cursor dirty bits, then rebuild from `main_screen` state.
pub fn sync_from_owner() {
    if !input_queue::is_owner() {
        return;
    }

    if clock_due() {
        LAST_CLOCK_MS.store(crate::get_time_ms() as u64, Ordering::Relaxed);
        mark_dirty();
    }

    if include_guest_cursor() {
        let (x, y) = crate::ui::cursor::get_cursor_pos();
        if x != CURSOR_PUB_X.load(Ordering::Relaxed) || y != CURSOR_PUB_Y.load(Ordering::Relaxed) {
            mark_dirty();
        }
    }

    if DIRTY.load(Ordering::Acquire) {
        crate::ui::main_screen::rebuild_scene();
        if include_guest_cursor() {
            let (x, y) = crate::ui::cursor::get_cursor_pos();
            CURSOR_PUB_X.store(x, Ordering::Relaxed);
            CURSOR_PUB_Y.store(y, Ordering::Relaxed);
        }
    }
}

/// Encode a complete snapshot. Publish to the mailbox when present; otherwise
/// `raster_soft` into the scanout (D1 / kill-switch). Stops the old painter
/// after a successful HDL present on virt, or a successful software raster.
pub fn publish_if_dirty() {
    if !input_queue::is_owner() {
        return;
    }
    if !DIRTY.load(Ordering::Acquire) {
        return;
    }

    let mut scratch_guard = SCRATCH.lock();
    let scratch = &mut *scratch_guard;
    let encoded = {
        let scene = SCENE.lock();
        match encode_nodes(&scene, &mut scratch.ops, &mut scratch.side) {
            Some((op_count, side_len, width, height)) => {
                let spec = EncodeSpec {
                    seq: 0,
                    width,
                    height,
                    abi_major: hdl::ABI_MAJOR,
                    abi_minor: hdl::ABI_MINOR,
                    flags: 0,
                };
                match hdl::encode(
                    &spec,
                    &scratch.ops[..op_count],
                    &scratch.side[..side_len],
                    &mut scratch.buf,
                ) {
                    Ok(n) => Some(n),
                    Err(EncodeError::BufferTooSmall | EncodeError::TooLarge) => None,
                }
            }
            None => None,
        }
    };

    let Some(nbytes) = encoded else {
        if !ENCODE_FAIL_LOGGED.swap(true, Ordering::Relaxed) {
            crate::services::klogd::klog_warning("hdl", "scene encode overflow; keeping last frame");
        }
        DIRTY.store(false, Ordering::Release);
        return;
    };

    let bytes = &scratch.buf[..nbytes];
    let mut presented = false;

    if hdl_mailbox::mailbox_present() {
        match hdl_mailbox::publish_frame(bytes) {
            Ok(_seq) => {
                presented = true;
                if !PUBLISHED_ONCE.swap(true, Ordering::Relaxed) {
                    crate::services::klogd::klog_info(
                        "hdl",
                        "desktop frame published (complete snapshot)",
                    );
                    crate::device::uart::write_line("[hdl] desktop published");
                }
            }
            Err(hdl_mailbox::PublishError::MailboxAbsent) => {}
            Err(_) => {
                DIRTY.store(false, Ordering::Release);
                return;
            }
        }
    } else {
        let ok = d1_display::with_gpu(|gpu| raster_soft::raster_bytes(gpu, bytes).is_ok())
            .unwrap_or(false);
        presented = ok;
    }

    if presented {
        // Virt + mailbox: stop fill_rect so we do not keep millions of guest
        // pixel stores (FB may go stale — host scrapes HDL). Unpublished
        // (D1 / QEMU / ?hdl=0): raster_soft is the FB painter; skip fill_rect
        // once it has succeeded so we do not dual-draw OpClear + widgets.
        SKIP_IMMEDIATE.store(true, Ordering::Release);
    }

    DIRTY.store(false, Ordering::Release);
}

fn encode_nodes(
    scene: &Scene,
    ops: &mut [Op],
    side: &mut [u8],
) -> Option<(usize, usize, u16, u16)> {
    if ops.is_empty() {
        return None;
    }
    let mut n = 0usize;
    let mut side_used = 0usize;

    ops[n] = Op::Clear { color: scene.clear };
    n += 1;

    for node in scene.nodes.iter().take(scene.node_count) {
        match *node {
            Node::Empty => {}
            Node::Panel {
                x,
                y,
                w,
                h,
                color,
                radius,
            } => {
                push_op(
                    ops,
                    &mut n,
                    Op::FillRect {
                        x,
                        y,
                        w,
                        h,
                        color,
                        radius,
                    },
                )?;
            }
            Node::Label {
                x,
                y,
                color,
                atlas_id,
                text,
                len,
            }
            | Node::GlyphRun {
                x,
                y,
                color,
                atlas_id,
                text,
                len,
            } => {
                if len == 0 {
                    continue;
                }
                let (count, glyphs_off) = push_glyphs(side, &mut side_used, &text[..len as usize])?;
                push_op(
                    ops,
                    &mut n,
                    Op::GlyphRun {
                        x,
                        y,
                        color,
                        atlas_id,
                        count,
                        glyphs_off,
                    },
                )?;
            }
            Node::Button {
                x,
                y,
                w,
                h,
                selected,
                label,
                len,
            } => {
                encode_button(
                    ops,
                    &mut n,
                    side,
                    &mut side_used,
                    x,
                    y,
                    w,
                    h,
                    selected,
                    &label[..len as usize],
                )?;
            }
            Node::Image { x, y, w, h, tex_id } => {
                push_op(
                    ops,
                    &mut n,
                    Op::Image {
                        x,
                        y,
                        w,
                        h,
                        tex_id,
                        flags: 0,
                        u0: 0,
                        v0: 0,
                        u1: 65535,
                        v1: 65535,
                    },
                )?;
            }
            Node::Line {
                x0,
                y0,
                x1,
                y1,
                color,
                width,
            } => {
                push_op(
                    ops,
                    &mut n,
                    Op::Line {
                        x0,
                        y0,
                        x1,
                        y1,
                        color,
                        width,
                    },
                )?;
            }
            Node::Dot { x, y, d, color } => {
                let radius = d / 2;
                push_op(
                    ops,
                    &mut n,
                    Op::FillRect {
                        x,
                        y,
                        w: d,
                        h: d,
                        color,
                        radius,
                    },
                )?;
            }
            Node::Check { x, y } => {
                push_op(
                    ops,
                    &mut n,
                    Op::FillRect {
                        x,
                        y,
                        w: 12,
                        h: 12,
                        color: opaque(80, 200, 120),
                        radius: 0,
                    },
                )?;
                push_op(
                    ops,
                    &mut n,
                    Op::Line {
                        x0: x.saturating_add(2),
                        y0: y.saturating_add(6),
                        x1: x.saturating_add(5),
                        y1: y.saturating_add(9),
                        color: opaque(255, 255, 255),
                        width: 2,
                    },
                )?;
                push_op(
                    ops,
                    &mut n,
                    Op::Line {
                        x0: x.saturating_add(5),
                        y0: y.saturating_add(9),
                        x1: x.saturating_add(10),
                        y1: y.saturating_add(2),
                        color: opaque(255, 255, 255),
                        width: 2,
                    },
                )?;
            }
            Node::Progress {
                x,
                y,
                w,
                h,
                fill_w,
            } => {
                push_op(
                    ops,
                    &mut n,
                    Op::FillRect {
                        x,
                        y,
                        w,
                        h,
                        color: opaque(50, 50, 70),
                        radius: 0,
                    },
                )?;
                if fill_w > 0 {
                    push_op(
                        ops,
                        &mut n,
                        Op::FillRect {
                            x,
                            y,
                            w: fill_w.min(w),
                            h,
                            color: opaque(80, 140, 200),
                            radius: 0,
                        },
                    )?;
                }
                push_op(
                    ops,
                    &mut n,
                    Op::Line {
                        x0: x,
                        y0: y,
                        x1: x.saturating_add(w as i16).saturating_sub(1),
                        y1: y,
                        color: opaque(60, 60, 80),
                        width: 1,
                    },
                )?;
                push_op(
                    ops,
                    &mut n,
                    Op::Line {
                        x0: x,
                        y0: y.saturating_add(h as i16).saturating_sub(1),
                        x1: x.saturating_add(w as i16).saturating_sub(1),
                        y1: y.saturating_add(h as i16).saturating_sub(1),
                        color: opaque(60, 60, 80),
                        width: 1,
                    },
                )?;
            }
            Node::Caret { x, y, h, color } => {
                push_op(
                    ops,
                    &mut n,
                    Op::FillRect {
                        x,
                        y,
                        w: 2,
                        h,
                        color,
                        radius: 0,
                    },
                )?;
            }
            Node::ClipPush { x, y, w, h } => {
                push_op(ops, &mut n, Op::ClipPush { x, y, w, h })?;
            }
            Node::ClipPop => {
                push_op(ops, &mut n, Op::ClipPop)?;
            }
        }
    }

    encode_cursor(scene, ops, &mut n)?;

    Some((n, side_used, scene.width, scene.height))
}

fn encode_cursor(scene: &Scene, ops: &mut [Op], n: &mut usize) -> Option<()> {
    match scene.cursor_kind {
        CursorKind::Hidden => Some(()),
        CursorKind::Arrow => push_op(
            ops,
            n,
            Op::Image {
                x: scene.cursor_x,
                y: scene.cursor_y,
                w: CURSOR_W,
                h: CURSOR_H,
                tex_id: TEX_CURSOR,
                flags: 0,
                u0: 0,
                v0: 0,
                u1: 65535,
                v1: 65535,
            },
        ),
        CursorKind::IBeam => {
            let x = scene.cursor_x;
            let y = scene.cursor_y;
            let color = opaque(220, 220, 230);
            push_op(
                ops,
                n,
                Op::FillRect {
                    x,
                    y,
                    w: 6,
                    h: 2,
                    color,
                    radius: 0,
                },
            )?;
            push_op(
                ops,
                n,
                Op::FillRect {
                    x: x.saturating_add(2),
                    y: y.saturating_add(2),
                    w: 2,
                    h: 12,
                    color,
                    radius: 0,
                },
            )
        }
    }
}

fn encode_button(
    ops: &mut [Op],
    n: &mut usize,
    side: &mut [u8],
    side_used: &mut usize,
    x: i16,
    y: i16,
    w: u16,
    h: u16,
    selected: bool,
    label: &[u8],
) -> Option<()> {
    let fill = if selected {
        opaque(80, 140, 200)
    } else {
        opaque(50, 50, 70)
    };
    let border = if selected {
        opaque(120, 180, 240)
    } else {
        opaque(60, 60, 80)
    };
    let text_color = if selected {
        opaque(255, 255, 255)
    } else {
        opaque(200, 200, 210)
    };
    let inset = if selected { 2i16 } else { 1i16 };
    let inner_w = w.saturating_sub((inset as u16).saturating_mul(2));
    let inner_h = h.saturating_sub((inset as u16).saturating_mul(2));
    let inner_r = BTN_RADIUS.saturating_sub(inset as u16);

    push_op(
        ops,
        n,
        Op::FillRect {
            x,
            y,
            w,
            h,
            color: border,
            radius: BTN_RADIUS,
        },
    )?;
    push_op(
        ops,
        n,
        Op::FillRect {
            x: x.saturating_add(inset),
            y: y.saturating_add(inset),
            w: inner_w,
            h: inner_h,
            color: fill,
            radius: inner_r,
        },
    )?;

    let (count, glyphs_off) = push_glyphs(side, side_used, label)?;
    let text_x = x.saturating_add(8);
    let text_y = y.saturating_add((h as i16 / 2).saturating_add(4));
    push_op(
        ops,
        n,
        Op::GlyphRun {
            x: text_x,
            y: text_y,
            color: text_color,
            atlas_id: ATLAS_UI,
            count,
            glyphs_off,
        },
    )?;
    Some(())
}

fn push_op(ops: &mut [Op], n: &mut usize, op: Op) -> Option<()> {
    let slot = ops.get_mut(*n)?;
    *slot = op;
    *n += 1;
    Some(())
}

fn push_glyphs(side: &mut [u8], used: &mut usize, text: &[u8]) -> Option<(u16, u32)> {
    let off = *used as u32;
    if text.len() > u16::MAX as usize {
        return None;
    }
    for &b in text {
        let gid = ascii_glyph(b);
        let start = *used;
        let end = start.checked_add(2)?;
        let dst = side.get_mut(start..end)?;
        dst.copy_from_slice(&gid.to_le_bytes());
        *used = end;
    }
    Some((text.len() as u16, off))
}

fn ascii_glyph(b: u8) -> u16 {
    if (0x20..=0x7F).contains(&b) {
        (b - 0x20) as u16
    } else {
        (b'?' - 0x20) as u16
    }
}

pub fn write_clock(buf: &mut [u8; MAX_LABEL]) -> u8 {
    buf.fill(0);
    let mut w = SliceWriter {
        buf: &mut buf[..],
        pos: 0,
    };
    let ok = if let Some(dt) = crate::device::rtc::get_datetime() {
        let month = match dt.month {
            1 => "Jan",
            2 => "Feb",
            3 => "Mar",
            4 => "Apr",
            5 => "May",
            6 => "Jun",
            7 => "Jul",
            8 => "Aug",
            9 => "Sep",
            10 => "Oct",
            11 => "Nov",
            12 => "Dec",
            _ => "???",
        };
        write!(w, "{} {:02} {:02}:{:02}", month, dt.day, dt.hour, dt.minute)
    } else {
        let uptime_secs = (crate::get_time_ms() as u64) / 1000;
        let hours = uptime_secs / 3600;
        let minutes = (uptime_secs % 3600) / 60;
        let seconds = uptime_secs % 60;
        write!(w, "Up: {:02}:{:02}:{:02}", hours, minutes, seconds)
    };
    if ok.is_err() {
        return 0;
    }
    w.pos.min(MAX_LABEL) as u8
}

struct SliceWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl Write for SliceWriter<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let end = self.pos.checked_add(bytes.len()).ok_or(core::fmt::Error)?;
        let dst = self.buf.get_mut(self.pos..end).ok_or(core::fmt::Error)?;
        dst.copy_from_slice(bytes);
        self.pos = end;
        Ok(())
    }
}
