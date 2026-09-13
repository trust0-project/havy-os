//! Damage compositor with z-ordered child windows.
//!
//! HDL mode: the window list is z-order only. Closing dirties the scene;
//! there is no full-screen `u32` restore. Legacy (`fill_rect`) still
//! snapshots the desktop once for blit-restore.

use alloc::vec::Vec;

use crate::platform::d1_display;

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn intersects(self, o: Rect) -> bool {
        let ax2 = self.x + self.w as i32;
        let ay2 = self.y + self.h as i32;
        let bx2 = o.x + o.w as i32;
        let by2 = o.y + o.h as i32;
        self.x < bx2 && ax2 > o.x && self.y < by2 && ay2 > o.y
    }

    pub fn union(self, o: Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        let x2 = (self.x + self.w as i32).max(o.x + o.w as i32);
        let y2 = (self.y + self.h as i32).max(o.y + o.h as i32);
        Rect {
            x,
            y,
            w: (x2 - x).max(0) as u32,
            h: (y2 - y).max(0) as u32,
        }
    }

    pub fn contains(self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w as i32 && py < self.y + self.h as i32
    }
}

#[derive(Clone, Copy)]
pub struct Window {
    pub kind: usize,
    pub geom: Rect,
}

static mut DESKTOP: Option<Vec<u32>> = None;
static mut DESKTOP_W: usize = 0;
static mut DESKTOP_H: usize = 0;
static mut WINDOWS: Vec<Window> = Vec::new();

fn screen() -> (usize, usize) {
    (
        d1_display::DISPLAY_WIDTH as usize,
        d1_display::DISPLAY_HEIGHT as usize,
    )
}

/// Reusable full-screen snapshot. Allocates once; grows only if size changes.
fn desktop_pixels(w: usize, h: usize) -> &'static mut [u32] {
    let needed = w * h;
    unsafe {
        let buf = DESKTOP.get_or_insert_with(|| alloc::vec![0u32; needed]);
        if buf.len() != needed {
            buf.resize(needed, 0);
        }
        DESKTOP_W = w;
        DESKTOP_H = h;
        buf.as_mut_slice()
    }
}

/// Snapshot the current framebuffer as the desktop layer.
pub fn capture_desktop() {
    if !crate::ui::scene::use_immediate_painter() {
        return;
    }
    let (w, h) = screen();
    d1_display::with_gpu(|gpu| {
        let buf = desktop_pixels(w, h);
        gpu.read_rect_fast(0, 0, w, h, buf);
    });
}

pub fn windows() -> &'static [Window] {
    unsafe { WINDOWS.as_slice() }
}

pub fn focused() -> Option<Window> {
    unsafe { WINDOWS.last().copied() }
}

pub fn focused_kind() -> Option<usize> {
    unsafe { WINDOWS.last().map(|w| w.kind) }
}

pub fn is_open(kind: usize) -> bool {
    unsafe { WINDOWS.iter().any(|w| w.kind == kind) }
}

pub fn get_geom(kind: usize) -> Option<Rect> {
    unsafe { WINDOWS.iter().find(|w| w.kind == kind).map(|w| w.geom) }
}

pub fn set_geom(kind: usize, geom: Rect) {
    unsafe {
        if let Some(w) = WINDOWS.iter_mut().find(|w| w.kind == kind) {
            w.geom = geom;
        }
    }
}

/// Raise `kind` to the top (or open it).
pub fn raise_or_open(kind: usize, geom: Rect) {
    unsafe {
        WINDOWS.retain(|w| w.kind != kind);
        WINDOWS.push(Window { kind, geom });
    }
}

pub fn close(kind: usize) -> Option<Rect> {
    unsafe {
        let geom = WINDOWS.iter().find(|w| w.kind == kind).map(|w| w.geom);
        WINDOWS.retain(|w| w.kind != kind);
        geom
    }
}

pub fn close_all() {
    unsafe {
        WINDOWS.clear();
    }
}

pub fn hit_window(x: i32, y: i32) -> Option<usize> {
    unsafe {
        for w in WINDOWS.iter().rev() {
            if w.geom.contains(x, y) {
                return Some(w.kind);
            }
        }
        None
    }
}

/// Restore `rect` from the desktop cache, then the caller redraws windows.
pub fn blit_desktop(rect: Rect) {
    if !crate::ui::scene::use_immediate_painter() {
        return;
    }
    unsafe {
        let Some(ref desk) = DESKTOP else { return };
        let dw = DESKTOP_W;
        let dh = DESKTOP_H;
        let x = rect.x.max(0) as u32;
        let y = rect.y.max(0) as u32;
        let w = rect.w.min(dw as u32 - x.min(dw as u32));
        let h = rect.h.min(dh as u32 - y.min(dh as u32));
        if w == 0 || h == 0 {
            return;
        }
        d1_display::with_gpu(|gpu| {
            let mut row = alloc::vec![0u32; w as usize];
            for r in 0..h {
                let src_y = (y + r) as usize;
                if src_y >= dh {
                    break;
                }
                let start = src_y * dw + x as usize;
                let n = w as usize;
                if start + n <= desk.len() {
                    row[..n].copy_from_slice(&desk[start..start + n]);
                    gpu.blit_rect(x, y + r, n, 1, &row[..n]);
                }
            }
        });
    }
}

pub fn any_open() -> bool {
    unsafe { !WINDOWS.is_empty() }
}
