//! MainScreen Screen
//!
//! Interactive main_screen screen showing system information,
//! hardware stats, and quick action buttons.

use alloc::format;
use alloc::vec::Vec;
use core::fmt::Write;

use embedded_graphics::{
    mono_font::{
        ascii::{FONT_7X14, FONT_9X15_BOLD},
        MonoTextStyle,
    },
    pixelcolor::{Rgb888, RgbColor},
    prelude::*,
    primitives::{Circle, Line, PrimitiveStyle, Rectangle, RoundedRectangle},
    text::Text,
};

use crate::input::{
    self, ABS_X, ABS_Y, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, BTN_TOUCH, EV_ABS, KEY_DOWN, KEY_ENTER,
    KEY_LEFT, KEY_RIGHT, KEY_UP,
};
use crate::platform::d1_display;

use super::cursor::{
    get_cursor_pos, invalidate_cursor_backup, is_left_button_pressed, restore_cursor_backup,
    set_cursor_pos, set_mouse_button,
};
use super::manager::with_ui;
use super::scene::{self, ATLAS_BOLD, ATLAS_UI, CursorKind};
use super::widgets::{Checkbox, Panel, ProgressBar, RadioButton, Window};
use super::{compositor, draw_image, LOGO_SMALL, LOGO_SMALL_SIZE, SCREEN_HEIGHT, SCREEN_WIDTH};

// Re-export cursor state for internal use
use super::cursor::CURSOR_X;
use super::cursor::CURSOR_Y;

/// Version extracted from Cargo.toml at compile time
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Hardware info for main_screen screen (fetched at runtime)
pub struct HardwareInfo {
    pub cpu_count: usize,
    pub memory_used_kb: usize,
    pub memory_total_kb: usize,
    pub disk_used_kb: usize,
    pub disk_total_kb: usize,
    pub network_available: bool,
    pub ip_addr: [u8; 4], // IP address as 4 octets
}

/// Get current hardware information from the system
pub fn get_hardware_info() -> HardwareInfo {
    use core::sync::atomic::Ordering;

    // Get CPU count from HARTS_ONLINE
    let cpu_count = crate::HARTS_ONLINE.load(Ordering::Relaxed);

    // Get comprehensive memory stats (includes kernel, stacks, heap, framebuffers)
    // GPU is always enabled when we're in the main screen UI
    let stats = crate::allocator::memory_stats(cpu_count, true);
    let memory_used_kb = stats.total_used / 1024;
    let memory_total_kb = stats.total_available / 1024;

    // Get disk usage from filesystem
    let (disk_used_kb, disk_total_kb) = {
        let fs_guard = crate::FS_STATE.read();
        if let Some(ref fs) = *fs_guard {
            let (used, total) = fs.disk_usage_bytes();
            ((used / 1024) as usize, (total / 1024) as usize)
        } else {
            (0, 0)
        }
    };

    // Check if network is available and get IP
    // Use is_ip_assigned() which checks if we have a valid IP (not 0.0.0.0)
    // This is more reliable than lock-based checks which may fail due to contention
    let (network_available, ip_addr) = {
        let ip = crate::net::get_my_ip();
        let octets = ip.octets();
        let has_ip = crate::net::is_ip_assigned();
        (has_ip, [octets[0], octets[1], octets[2], octets[3]])
    };

    HardwareInfo {
        cpu_count,
        memory_used_kb,
        memory_total_kb,
        disk_used_kb,
        disk_total_kb,
        network_available,
        ip_addr,
    }
}

/// Selected button index for keyboard navigation
static mut MAIN_SCREEN_SELECTED_BUTTON: usize = 0;

/// Keyboard / pointer selection (Network=0, Terminal=1, Settings=2).
/// Scene slice reads this from `gpuid` only.
pub fn selected_button() -> usize {
    unsafe { MAIN_SCREEN_SELECTED_BUTTON }
}

/// Flag to track if static content has been drawn (labels, lines, etc.)
/// When true, only dynamic content (buttons, stats) needs updating
static mut MAIN_SCREEN_STATIC_DRAWN: bool = false;

/// Last selected button - used to only redraw changed buttons
static mut MAIN_SCREEN_LAST_SELECTED: Option<usize> = None;

/// Currently open child window (None = main screen, Some(index) = button window open)
static mut MAIN_SCREEN_OPEN_WINDOW: Option<usize> = None;

// Window stack lives in `compositor`. Desktop snapshot replaces BSS backing.

fn compact() -> bool {
    SCREEN_WIDTH < 800
}

fn paint_legacy() -> bool {
    scene::use_immediate_painter()
}

fn scene_dirty() {
    scene::notify_dirty();
}

#[derive(Clone, Copy)]
struct ChildGeom {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

static mut CHILD_X: i32 = 0;
static mut CHILD_Y: i32 = 0;
static mut CHILD_W: u32 = 0;
static mut CHILD_H: u32 = 0;
static mut WINDOW_DRAG: bool = false;
static mut DRAG_OFF_X: i32 = 0;
static mut DRAG_OFF_Y: i32 = 0;

fn default_term_geom() -> ChildGeom {
    if compact() {
        let w = (SCREEN_WIDTH as u32).saturating_sub(16).max(200);
        let h = (SCREEN_HEIGHT as u32).saturating_sub(80).max(160);
        ChildGeom { x: 8, y: 8, w, h }
    } else {
        // Keep the action-button row (y≈500) uncovered so a second window can open.
        ChildGeom {
            x: 40,
            y: 24,
            w: 700,
            h: 460,
        }
    }
}

fn default_geom_for(idx: usize) -> ChildGeom {
    match idx {
        1 => default_term_geom(),
        2 => default_settings_geom(),
        _ => default_net_geom(),
    }
}

fn move_focused(new_geom: ChildGeom) {
    let old = geom_to_rect(child_geom());
    set_child_geom(new_geom);
    if let Some(kind) = compositor::focused_kind() {
        compositor::set_geom(kind, geom_to_rect(child_geom()));
        compose_after(old.union(geom_to_rect(child_geom())));
    }
}

fn hit_settings_widgets(x: i32, y: i32) -> bool {
    let g = child_geom();
    // checkbox
    if x >= g.x + 24 && x < g.x + 200 && y >= g.y + 70 && y < g.y + 88 {
        unsafe {
            SETTINGS_SHOW_NET = !SETTINGS_SHOW_NET;
        }
        scene_dirty();
        if paint_legacy() {
            draw_child_window_inner(2);
            redraw_net_badge();
        }
        return true;
    }
    if x >= g.x + 24 && x < g.x + 200 && y >= g.y + 110 && y < g.y + 128 {
        unsafe {
            SETTINGS_THEME = 0;
        }
        scene_dirty();
        if paint_legacy() {
            draw_child_window_inner(2);
        }
        return true;
    }
    if x >= g.x + 24 && x < g.x + 200 && y >= g.y + 136 && y < g.y + 154 {
        unsafe {
            SETTINGS_THEME = 1;
        }
        scene_dirty();
        if paint_legacy() {
            draw_child_window_inner(2);
        }
        return true;
    }
    false
}

fn default_net_geom() -> ChildGeom {
    if compact() {
        let w = (SCREEN_WIDTH as u32).saturating_sub(24).max(200);
        let h = (SCREEN_HEIGHT as u32).saturating_sub(80).max(160);
        ChildGeom { x: 12, y: 12, w, h }
    } else {
        ChildGeom {
            x: 200,
            y: 40,
            w: 500,
            h: 400,
        }
    }
}

fn child_geom() -> ChildGeom {
    unsafe {
        ChildGeom {
            x: CHILD_X,
            y: CHILD_Y,
            w: CHILD_W,
            h: CHILD_H,
        }
    }
}

fn set_child_geom(g: ChildGeom) {
    let max_x = (SCREEN_WIDTH as u32).saturating_sub(g.w) as i32;
    let max_y = (SCREEN_HEIGHT as i32).saturating_sub(g.h as i32);
    unsafe {
        CHILD_X = g.x.clamp(0, max_x.max(0));
        CHILD_Y = g.y.clamp(0, max_y.max(0));
        CHILD_W = g.w;
        CHILD_H = g.h;
    }
}

fn sx(x: i32) -> i32 {
    x * SCREEN_WIDTH / 1024
}
fn sy(y: i32) -> i32 {
    y * SCREEN_HEIGHT / 768
}

/// Last time hardware stats were updated (in ms)
static mut MAIN_SCREEN_LAST_HW_UPDATE: i64 = 0;

/// Hardware stats update interval in ms
const MAIN_SCREEN_HW_UPDATE_INTERVAL: i64 = 2000; // Update every 2 seconds

// Terminal window state
const TERMINAL_INPUT_MAX: usize = 256;
const TERM_SCROLLBACK: usize = 256;
const TERM_COLS: usize = 96;

#[derive(Clone, Copy)]
struct TermLine {
    text: [u8; TERM_COLS],
    len: u8,
    fg: u8, // 0 = default green, 30–37 SGR
}

static mut TERMINAL_INPUT_BUFFER: [u8; TERMINAL_INPUT_MAX] = [0; TERMINAL_INPUT_MAX];
static mut TERMINAL_INPUT_LEN: usize = 0;
static mut TERM_LINES: [TermLine; TERM_SCROLLBACK] = [TermLine {
    text: [0; TERM_COLS],
    len: 0,
    fg: 0,
}; TERM_SCROLLBACK];
static mut TERM_HEAD: usize = 0;
static mut TERM_COUNT: usize = 0;
static mut TERM_SGR: u8 = 0;
static mut TERM_COL: usize = 0;

fn term_clear() {
    unsafe {
        TERM_HEAD = 0;
        TERM_COUNT = 0;
        TERM_SGR = 0;
        TERM_COL = 0;
    }
}

fn term_push_line(line: TermLine) {
    unsafe {
        if TERM_COUNT < TERM_SCROLLBACK {
            TERM_LINES[(TERM_HEAD + TERM_COUNT) % TERM_SCROLLBACK] = line;
            TERM_COUNT += 1;
        } else {
            TERM_LINES[TERM_HEAD] = line;
            TERM_HEAD = (TERM_HEAD + 1) % TERM_SCROLLBACK;
        }
    }
}

fn term_append_bytes(bytes: &[u8]) {
    unsafe {
        let mut i = 0;
        let mut cur = if TERM_COUNT == 0 {
            term_push_line(TermLine {
                text: [0; TERM_COLS],
                len: 0,
                fg: TERM_SGR,
            });
            TERM_LINES[(TERM_HEAD + TERM_COUNT - 1) % TERM_SCROLLBACK]
        } else {
            TERM_LINES[(TERM_HEAD + TERM_COUNT - 1) % TERM_SCROLLBACK]
        };
        while i < bytes.len() {
            let b = bytes[i];
            if b == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                i += 2;
                let mut code: u8 = 0;
                while i < bytes.len() && !bytes[i].is_ascii_alphabetic() {
                    if bytes[i].is_ascii_digit() {
                        code = code.saturating_mul(10).saturating_add(bytes[i] - b'0');
                    } else if bytes[i] == b';' {
                        code = 0;
                    }
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
                TERM_SGR = code;
                cur.fg = TERM_SGR;
                continue;
            }
            if b == b'\n' {
                let idx = (TERM_HEAD + TERM_COUNT - 1) % TERM_SCROLLBACK;
                TERM_LINES[idx] = cur;
                term_push_line(TermLine {
                    text: [0; TERM_COLS],
                    len: 0,
                    fg: TERM_SGR,
                });
                cur = TermLine {
                    text: [0; TERM_COLS],
                    len: 0,
                    fg: TERM_SGR,
                };
                i += 1;
                continue;
            }
            if b == b'\r' {
                i += 1;
                continue;
            }
            if (b == 0x08 || b == 0x7f) && cur.len > 0 {
                cur.len -= 1;
                i += 1;
                continue;
            }
            if cur.len as usize >= TERM_COLS {
                let idx = (TERM_HEAD + TERM_COUNT - 1) % TERM_SCROLLBACK;
                TERM_LINES[idx] = cur;
                term_push_line(TermLine {
                    text: [0; TERM_COLS],
                    len: 0,
                    fg: TERM_SGR,
                });
                cur = TermLine {
                    text: [0; TERM_COLS],
                    len: 0,
                    fg: TERM_SGR,
                };
            }
            if b >= 32 {
                cur.text[cur.len as usize] = b;
                cur.len += 1;
            }
            i += 1;
        }
        if TERM_COUNT > 0 {
            TERM_LINES[(TERM_HEAD + TERM_COUNT - 1) % TERM_SCROLLBACK] = cur;
        }
    }
}

fn term_sgr_rgb(fg: u8) -> Rgb888 {
    match fg {
        31 | 91 => Rgb888::new(220, 80, 80),
        32 | 92 => Rgb888::new(80, 200, 120),
        33 | 93 => Rgb888::new(230, 180, 80),
        34 | 94 => Rgb888::new(80, 140, 220),
        35 | 95 => Rgb888::new(180, 80, 200),
        36 | 96 => Rgb888::new(80, 200, 200),
        37 | 97 => Rgb888::WHITE,
        30 | 90 => Rgb888::new(100, 100, 120),
        _ => Rgb888::new(80, 200, 120),
    }
}

fn term_line_at(i: usize) -> Option<TermLine> {
    unsafe {
        if i >= TERM_COUNT {
            return None;
        }
        Some(TERM_LINES[(TERM_HEAD + i) % TERM_SCROLLBACK])
    }
}
/// Whether a command is currently executing (shows Cancel button instead of Run)
static mut TERMINAL_COMMAND_RUNNING: bool = false;
/// Whether a cancel has been requested (checked by should_cancel syscall)
static mut TERMINAL_CANCEL_REQUESTED: bool = false;

/// Check if a point is inside a main_screen button, returns button index if hit.
/// Index 0 (Network) is the Phase 2 HDL slice control; hit-test stays in
/// guest logical pixels.
pub fn hit_test_main_screen_button(x: i32, y: i32) -> Option<usize> {
    let by = sy(500);
    let bw = 110;
    let bh = 32;
    let buttons = [
        (sx(30), by, bw, bh),
        (sx(150), by, bw, bh),
        (sx(270), by, bw, bh),
    ];

    for (i, (bx, by, bw, bh)) in buttons.iter().enumerate() {
        if x >= *bx && x < bx + (*bw as i32) && y >= *by && y < by + (*bh as i32) {
            return Some(i);
        }
    }
    None
}

fn geom_to_rect(g: ChildGeom) -> compositor::Rect {
    compositor::Rect {
        x: g.x,
        y: g.y,
        w: g.w,
        h: g.h,
    }
}

fn rect_to_geom(r: compositor::Rect) -> ChildGeom {
    ChildGeom {
        x: r.x,
        y: r.y,
        w: r.w,
        h: r.h,
    }
}

fn compose_after(damage: compositor::Rect) {
    if paint_legacy() {
        compositor::blit_desktop(damage);
        for w in compositor::windows() {
            set_child_geom(rect_to_geom(w.geom));
            draw_child_window_inner(w.kind);
        }
    }
    unsafe {
        MAIN_SCREEN_OPEN_WINDOW = compositor::focused_kind();
    }
    scene_dirty();
}

/// Save the desktop once so overlapping windows can restore by compose.
/// HDL mode: windows are scene nodes — no 3 MiB snapshot.
fn save_window_backing() {
    if paint_legacy() && compositor::windows().is_empty() {
        compositor::capture_desktop();
    }
}

fn restore_window_backing() {
    if let Some(kind) = unsafe { MAIN_SCREEN_OPEN_WINDOW } {
        if let Some(r) = compositor::close(kind) {
            compose_after(r);
        }
    }
    unsafe {
        WINDOW_DRAG = false;
        CHILD_W = 0;
        MAIN_SCREEN_OPEN_WINDOW = compositor::focused_kind();
    }
}

/// Get button name for child window title
fn get_button_name(index: usize) -> &'static str {
    match index {
        0 => "Network",
        1 => "Terminal",
        2 => "Settings",
        _ => "Unknown",
    }
}

/// Last touch coordinates for debug display
static mut LAST_TOUCH_X: i32 = 0;
static mut LAST_TOUCH_Y: i32 = 0;
static mut LAST_TOUCH_COUNT: u32 = 0;

/// Update debug info for touch events (called when touch is detected)
pub fn update_touch_debug(x: i32, y: i32) {
    unsafe {
        LAST_TOUCH_X = x;
        LAST_TOUCH_Y = y;
        LAST_TOUCH_COUNT = LAST_TOUCH_COUNT.wrapping_add(1);
    }
}

/// Update just the dynamic hardware stats section of the main_screen screen
/// HDL mode: mutate labels via a dirty scene (complete snapshot), not fill_rect.
pub fn update_main_screen_hardware_stats() {
    // Don't update hardware stats if a child window is open (it would draw over the window)
    if compositor::any_open() {
        return;
    }

    let now = crate::get_time_ms();

    // Check if enough time has passed since last update
    let should_update = unsafe {
        if now - MAIN_SCREEN_LAST_HW_UPDATE < MAIN_SCREEN_HW_UPDATE_INTERVAL {
            return;
        }
        MAIN_SCREEN_LAST_HW_UPDATE = now;
        true
    };

    if !should_update {
        return;
    }

    scene_dirty();
    if !paint_legacy() {
        return;
    }

    // Get fresh hardware info
    let hw = get_hardware_info();

    // Only redraw the hardware stats area
    d1_display::with_gpu(|gpu| {
        let col1_x = 30;
        let text_style = MonoTextStyle::new(&FONT_7X14, Rgb888::new(200, 200, 210));

        // OPTIMIZATION: Use direct fill_rect for clearing instead of Rectangle::with_fill()
        // This bypasses embedded_graphics overhead and uses our fast fill_hline internally
        // Clear top section: CPU (y=210) and Memory (y=225) - window background color (28, 28, 38)
        gpu.fill_rect(col1_x as u32, 200, 300, 28, 28, 28, 38);
        // Clear bottom section: Disk (y=255) and Network (y=270)
        gpu.fill_rect(col1_x as u32, 245, 300, 37, 28, 28, 38);

        // Redraw dynamic values
        let mut cpu_buf = [0u8; 32];
        let cpu_str = format_cpu_str(hw.cpu_count, &mut cpu_buf);
        let _ = Text::new(cpu_str, Point::new(col1_x, 210), text_style).draw(gpu);

        let mut mem_buf = [0u8; 48];
        let mem_str = format_memory_str(hw.memory_used_kb, hw.memory_total_kb, &mut mem_buf);
        let _ = Text::new(mem_str, Point::new(col1_x, 225), text_style).draw(gpu);

        let mut disk_buf = [0u8; 48];
        let disk_str = format_disk_str(hw.disk_used_kb, hw.disk_total_kb, &mut disk_buf);
        let _ = Text::new(disk_str, Point::new(col1_x, 255), text_style).draw(gpu);

        let mut net_buf = [0u8; 48];
        let net_str = format_network_str(hw.network_available, &hw.ip_addr, &mut net_buf);
        let _ = Text::new(net_str, Point::new(col1_x, 270), text_style).draw(gpu);

        // Update date/time or uptime in status bar
        // OPTIMIZATION: Use direct fill_rect for status bar time clearing
        gpu.fill_rect(400, 742, 150, 26, 25, 25, 35);

        // Try to get host date/time from RTC, fall back to uptime
        let time_str = if let Some(dt) = crate::device::rtc::get_datetime() {
            // Display as: "Dec 16 15:30"
            let month_name = match dt.month {
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
            format!(
                "{} {:02} {:02}:{:02}",
                month_name, dt.day, dt.hour, dt.minute
            )
        } else {
            // Fall back to uptime if RTC not available
            let uptime_ms = crate::get_time_ms() as u64;
            let uptime_secs = uptime_ms / 1000;
            let hours = uptime_secs / 3600;
            let minutes = (uptime_secs % 3600) / 60;
            let seconds = uptime_secs % 60;
            format!("Up: {:02}:{:02}:{:02}", hours, minutes, seconds)
        };
        let _ = Text::new(&time_str, Point::new(410, 756), text_style).draw(gpu);
    });
    // Flush deferred to end of gpuid tick
}

/// Fast update of just the quick action buttons (for keyboard navigation)
/// This is MUCH faster than redrawing the entire screen
pub fn update_main_screen_buttons(selected_button: usize) {
    scene_dirty();
    if !paint_legacy() {
        return;
    }
    // Hide cursor first (restore pixels) to prevent ghost when redrawing over it
    restore_cursor_backup();

    d1_display::with_gpu(|gpu| {
        // Button definitions - Network and Terminal, left aligned (adjusted for 1024x768)
        let buttons = [
            ("Network", sx(30)),
            ("Terminal", sx(150)),
            ("Settings", sx(270)),
        ];

        // Clear the buttons area
        gpu.fill_rect(sx(28) as u32, sy(498) as u32, 360, 38, 28, 28, 38);

        // Redraw all buttons
        for (i, (label, x)) in buttons.iter().enumerate() {
            let is_selected = i == selected_button;
            let bg_color = if is_selected {
                Rgb888::new(80, 140, 200)
            } else {
                Rgb888::new(50, 50, 70)
            };
            let border_color = if is_selected {
                Rgb888::new(120, 180, 240)
            } else {
                Rgb888::new(60, 60, 80)
            };

            // Button background (110 width)
            let _ = RoundedRectangle::with_equal_corners(
                Rectangle::new(Point::new(*x, sy(500)), Size::new(110, 32)),
                Size::new(4, 4),
            )
            .into_styled(PrimitiveStyle::with_fill(bg_color))
            .draw(gpu);

            // Button border
            let _ = RoundedRectangle::with_equal_corners(
                Rectangle::new(Point::new(*x, sy(500)), Size::new(110, 32)),
                Size::new(4, 4),
            )
            .into_styled(PrimitiveStyle::with_stroke(
                border_color,
                if is_selected { 2 } else { 1 },
            ))
            .draw(gpu);

            let text_color = if is_selected {
                Rgb888::WHITE
            } else {
                Rgb888::new(200, 200, 210)
            };
            let btn_text_style = MonoTextStyle::new(&FONT_7X14, text_color);
            let _ = Text::new(label, Point::new(*x + 8, sy(520)), btn_text_style).draw(gpu);
        }
    });

    // Invalidate backup and force cursor redraw with fresh background
    invalidate_cursor_backup();
    // Flush deferred to end of gpuid tick
}

/// Rebuild the retained scene from live main-screen state (owner / gpuid).
pub(crate) fn rebuild_scene() {
    let hw = get_hardware_info();
    let selected = unsafe { MAIN_SCREEN_SELECTED_BUTTON };
    let show_net = unsafe { SETTINGS_SHOW_NET };
    let theme = unsafe { SETTINGS_THEME };
    scene::rebuild(|b| {
        emit_desktop(b, &hw, selected, show_net);
        for w in compositor::windows() {
            match w.kind {
                0 => emit_network_window(b, w.geom),
                1 => emit_terminal_window(b, w.geom),
                2 => emit_settings_window(b, w.geom, show_net, theme),
                _ => {}
            }
        }
        if scene::include_guest_cursor() {
            let (cx, cy) = get_cursor_pos();
            let kind = if compositor::is_open(1) && hit_test_terminal_input(cx, cy) {
                CursorKind::IBeam
            } else {
                CursorKind::Arrow
            };
            b.set_cursor(cx, cy, kind);
        }
    });
}

fn emit_desktop(b: &mut scene::Builder<'_>, hw: &HardwareInfo, selected: usize, show_net: bool) {
    let win_w = (sx(1004) as u32).min((SCREEN_WIDTH - 20).max(1) as u32);
    let win_h = (sy(710) as u32).min((SCREEN_HEIGHT - 40).max(1) as u32);
    b.window_chrome(
        sx(10),
        sy(10),
        win_w,
        win_h,
        b"HAVY OS - System Information",
        false,
        false,
        (28, 28, 38),
        (40, 40, 55),
        (255, 255, 255),
    );

    let col1_x = 30;
    b.label_str(col1_x, 70, 80, 140, 200, ATLAS_UI, "About This System");
    b.line(col1_x, 75, col1_x + 150, 75, 60, 60, 80, 1);
    b.label_str(col1_x, 95, 200, 200, 210, ATLAS_UI, "OS Name:      HAVY OS");

    let mut ver = [0u8; 48];
    let ver_n = stack_write(&mut ver, format_args!("Version:      {}", VERSION));
    b.label(col1_x, 110, 200, 200, 210, ATLAS_UI, &ver[..ver_n]);
    b.label_str(
        col1_x,
        140,
        200,
        200,
        210,
        ATLAS_UI,
        "Architecture: RISC-V RV64GC",
    );
    b.label_str(
        col1_x,
        155,
        200,
        200,
        210,
        ATLAS_UI,
        "Platform:     Virtual Machine",
    );

    b.label_str(col1_x, 185, 80, 140, 200, ATLAS_UI, "Hardware");
    b.line(col1_x, 190, col1_x + 100, 190, 60, 60, 80, 1);

    let mut cpu_buf = [0u8; 32];
    let cpu_str = format_cpu_str(hw.cpu_count, &mut cpu_buf);
    b.label_str(col1_x, 210, 200, 200, 210, ATLAS_UI, cpu_str);

    let mut mem_buf = [0u8; 48];
    let mem_str = format_memory_str(hw.memory_used_kb, hw.memory_total_kb, &mut mem_buf);
    b.label_str(col1_x, 225, 200, 200, 210, ATLAS_UI, mem_str);
    let mem_ratio = if hw.memory_total_kb > 0 {
        hw.memory_used_kb as f32 / hw.memory_total_kb as f32
    } else {
        0.0
    };
    let mut mem_bar = ProgressBar::new(col1_x, 228, 180, 6);
    mem_bar.set_progress(mem_ratio);
    mem_bar.emit(b);

    let mut disp = [0u8; 40];
    let disp_n = stack_write(
        &mut disp,
        format_args!(
            "Display:      {}x{}",
            d1_display::DISPLAY_WIDTH,
            d1_display::DISPLAY_HEIGHT
        ),
    );
    b.label(col1_x, 240, 200, 200, 210, ATLAS_UI, &disp[..disp_n]);

    let mut disk_buf = [0u8; 48];
    let disk_str = format_disk_str(hw.disk_used_kb, hw.disk_total_kb, &mut disk_buf);
    b.label_str(col1_x, 255, 200, 200, 210, ATLAS_UI, disk_str);

    let mut net_buf = [0u8; 48];
    let net_str = format_network_str(hw.network_available, &hw.ip_addr, &mut net_buf);
    b.label_str(col1_x, 270, 200, 200, 210, ATLAS_UI, net_str);

    let col2_x = if compact() { sx(30) } else { 550 };
    b.label_str(col2_x, 70, 80, 140, 200, ATLAS_UI, "Features");
    b.line(col2_x, 75, col2_x + 100, 75, 60, 60, 80, 1);
    let features = [
        "Multi-core SMP support",
        "Preemptive scheduler",
        "VirtIO device drivers",
        "TCP/IP networking (smoltcp)",
        "Simple File System",
        "WASM application runtime",
        "GPU-accelerated display",
        "Interactive shell",
    ];
    for (i, feature) in features.iter().enumerate() {
        let y = 95 + (i as i32 * 20);
        b.check(col2_x, y - 10);
        b.label_str(col2_x + 20, y, 200, 200, 210, ATLAS_UI, feature);
    }

    b.label_str(col1_x, 470, 80, 140, 200, ATLAS_UI, "Quick Actions");
    b.line(col1_x, 475, col1_x + 120, 475, 60, 60, 80, 1);
    b.label_str(
        col1_x,
        488,
        100,
        100,
        120,
        ATLAS_UI,
        "Use arrows to select, Enter to open",
    );
    let buttons = [
        (b"Network" as &[u8], sx(30)),
        (b"Terminal", sx(150)),
        (b"Settings", sx(270)),
    ];
    for (i, (label, x)) in buttons.iter().enumerate() {
        b.button(*x, sy(500), 110, 32, i == selected, label);
    }

    let services_x = if compact() { sx(250) } else { 700 };
    b.label_str(services_x, 310, 80, 140, 200, ATLAS_UI, "Running Services");
    b.line(services_x, 315, services_x + 140, 315, 60, 60, 80, 1);
    let services = ["shell", "httpd", "tcpd", "sysmond"];
    for (i, name) in services.iter().enumerate() {
        let y = 335 + (i as i32 * 18);
        b.dot(services_x, y, 8, 80, 200, 120);
        b.label_str(services_x + 14, y + 6, 200, 200, 210, ATLAS_UI, name);
    }

    b.label_str(
        col1_x,
        560,
        160,
        160,
        175,
        ATLAS_UI,
        "HAVY OS is a lightweight operating system written in Rust, running on a",
    );
    b.label_str(
        col1_x,
        575,
        160,
        160,
        175,
        ATLAS_UI,
        "RISC-V virtual machine in your browser.",
    );

    b.line(30, 610, 994, 610, 60, 60, 80, 1);
    b.label_str(
        30,
        630,
        120,
        120,
        140,
        ATLAS_UI,
        "Built with: Rust, embedded-graphics, smoltcp, wasmi",
    );
    b.label_str(
        30,
        645,
        120,
        120,
        140,
        ATLAS_UI,
        "License: MIT | github.com/elribonazo/riscv-vm",
    );
    b.panel(870, 620, 120, 24, 80, 140, 200, 4);
    let mut badge = [0u8; 16];
    let badge_n = stack_write(&mut badge, format_args!("v{}", VERSION));
    b.label(890, 636, 200, 200, 210, ATLAS_UI, &badge[..badge_n]);

    let status_y = SCREEN_HEIGHT - 30;
    b.panel(0, status_y, SCREEN_WIDTH as u32, 30, 25, 25, 35, 0);
    b.line(0, status_y, SCREEN_WIDTH, status_y, 60, 60, 80, 1);
    b.label_str(
        10,
        status_y + 18,
        200,
        200,
        210,
        ATLAS_UI,
        "HAVY OS | GPU Active",
    );

    let mut clock = [0u8; scene::MAX_LABEL];
    let clock_n = scene::write_clock(&mut clock);
    b.label(
        sx(460),
        status_y + 18,
        200,
        200,
        210,
        ATLAS_UI,
        &clock[..clock_n as usize],
    );

    if show_net {
        let (nr, ng, nb) = if hw.network_available {
            (80, 200, 120)
        } else {
            (150, 150, 160)
        };
        b.dot(sx(870), status_y + 7, 10, nr, ng, nb);
        b.label_str(sx(884), status_y + 18, 200, 200, 210, ATLAS_UI, "NET");
    }
    b.dot(sx(920), status_y + 7, 10, 80, 200, 120);
    b.label_str(sx(934), status_y + 18, 200, 200, 210, ATLAS_UI, "CPU");
    b.dot(sx(970), status_y + 7, 10, 230, 180, 80);
    b.label_str(sx(984), status_y + 18, 200, 200, 210, ATLAS_UI, "MEM");
}

fn emit_network_window(b: &mut scene::Builder<'_>, g: compositor::Rect) {
    b.window_chrome(
        g.x,
        g.y,
        g.w,
        g.h,
        b"Network Statistics",
        true,
        true,
        (28, 28, 38),
        (40, 40, 55),
        (255, 255, 255),
    );

    let is_online = crate::net::is_ip_assigned();
    let ip = crate::net::get_my_ip();
    let ip_octets = ip.octets();
    let gateway = crate::net::GATEWAY.octets();
    let dns = crate::net::DNS_SERVER.octets();
    let prefix = crate::net::PREFIX_LEN;

    let x = g.x + 20;
    let mut y = g.y + 60;
    b.label_str(x, y, 230, 180, 80, ATLAS_UI, "Device:");
    y += 16;
    b.label_str(x + 10, y, 255, 255, 255, ATLAS_UI, "Type:    VirtIO Network Device");
    y += 14;
    b.label_str(x + 10, y, 255, 255, 255, ATLAS_UI, "Address: 0x10001000");
    y += 14;
    if is_online {
        b.label_str(x + 10, y, 80, 200, 120, ATLAS_UI, "Status:  * ONLINE");
    } else {
        b.label_str(x + 10, y, 220, 80, 80, ATLAS_UI, "Status:  X OFFLINE");
    }
    y += 22;
    b.label_str(x, y, 230, 180, 80, ATLAS_UI, "Configuration:");
    y += 16;

    let mut ip_buf = [0u8; 32];
    let ip_n = stack_write(
        &mut ip_buf,
        format_args!(
            "{}.{}.{}.{}/{}",
            ip_octets[0], ip_octets[1], ip_octets[2], ip_octets[3], prefix
        ),
    );
    b.label_str(x + 10, y, 255, 255, 255, ATLAS_UI, "IP:      ");
    b.label(x + 64, y, 255, 255, 255, ATLAS_UI, &ip_buf[..ip_n]);
    y += 14;

    let mut gw_buf = [0u8; 24];
    let gw_n = stack_write(
        &mut gw_buf,
        format_args!("{}.{}.{}.{}", gateway[0], gateway[1], gateway[2], gateway[3]),
    );
    b.label_str(x + 10, y, 255, 255, 255, ATLAS_UI, "Gateway: ");
    b.label(x + 64, y, 255, 255, 255, ATLAS_UI, &gw_buf[..gw_n]);
    y += 14;

    let mut dns_buf = [0u8; 24];
    let dns_n = stack_write(
        &mut dns_buf,
        format_args!("{}.{}.{}.{}", dns[0], dns[1], dns[2], dns[3]),
    );
    b.label_str(x + 10, y, 255, 255, 255, ATLAS_UI, "DNS:     ");
    b.label(x + 64, y, 255, 255, 255, ATLAS_UI, &dns_buf[..dns_n]);
    y += 22;

    b.label_str(x, y, 230, 180, 80, ATLAS_UI, "Protocol Stack:");
    y += 16;
    b.label_str(x + 10, y, 255, 255, 255, ATLAS_UI, "smoltcp - Lightweight TCP/IP");
    y += 14;
    b.label_str(x + 10, y, 255, 255, 255, ATLAS_UI, "ICMP, UDP, TCP, ARP");

    b.label_str(
        g.x + 20,
        g.y + g.h as i32 - 20,
        100,
        100,
        120,
        ATLAS_UI,
        "Press ESC or click red button to close",
    );
}

fn emit_terminal_window(b: &mut scene::Builder<'_>, g: compositor::Rect) {
    b.window_chrome(
        g.x,
        g.y,
        g.w,
        g.h,
        b"Terminal",
        true,
        true,
        (28, 28, 38),
        (40, 40, 55),
        (255, 255, 255),
    );

    let content_x = g.x + 15;
    let content_y = g.y + 45;
    let input_w = g.w.saturating_sub(120).max(80);
    let out_w = g.w.saturating_sub(30).max(80);
    let out_h = g.h.saturating_sub(150).max(40);

    b.label_str(content_x, content_y, 100, 100, 120, ATLAS_UI, "Command:");
    let input_y = content_y + 10;
    b.panel(content_x, input_y, input_w, 28, 80, 80, 100, 4);
    b.panel(
        content_x + 1,
        input_y + 1,
        input_w.saturating_sub(2),
        26,
        18,
        18,
        28,
        3,
    );

    let input_len = unsafe { TERMINAL_INPUT_LEN };
    let vis = ((input_w.saturating_sub(14) / 7) as usize).max(1);
    let start = input_len.saturating_sub(vis);
    let input_bytes = unsafe { &TERMINAL_INPUT_BUFFER[start..input_len] };
    b.clip_push(content_x + 1, input_y + 1, input_w.saturating_sub(2), 26);
    b.label(content_x + 7, input_y + 19, 255, 255, 255, ATLAS_UI, input_bytes);
    b.clip_pop();
    let cursor_x = content_x + 7 + ((input_len - start) as i32 * 7);
    if cursor_x < content_x + input_w as i32 - 10 {
        b.caret(cursor_x, input_y + 5, 16, 200, 200, 220);
    }

    let btn_x = content_x + input_w as i32 + 10;
    let is_running = unsafe { TERMINAL_COMMAND_RUNNING };
    let (br, bg, bb, btn_text, text_x) = if is_running {
        (200, 80, 80, b"Cancel" as &[u8], btn_x + 14)
    } else {
        (80, 140, 200, b"Run" as &[u8], btn_x + 25)
    };
    b.panel(btn_x, input_y, 80, 28, br, bg, bb, 4);
    b.label(text_x, input_y + 19, 255, 255, 255, ATLAS_UI, btn_text);

    let output_label_y = input_y + 40;
    let mut cwd_buf = [0u8; 96];
    let cwd_n = {
        let cwd = crate::lock::utils::CWD_STATE.lock();
        let n = cwd.len.min(94);
        cwd_buf[..n].copy_from_slice(&cwd.path[..n]);
        cwd_buf[n] = b'$';
        n + 1
    };
    b.label(content_x, output_label_y, 100, 100, 120, ATLAS_UI, &cwd_buf[..cwd_n]);

    let output_y = output_label_y + 10;
    b.panel(content_x, output_y, out_w, out_h, 60, 60, 80, 4);
    b.panel(
        content_x + 1,
        output_y + 1,
        out_w.saturating_sub(2),
        out_h.saturating_sub(2),
        10,
        10,
        15,
        3,
    );

    let visible = core::cmp::min(
        ((out_h as i32 - 20) / 15).max(1) as usize,
        unsafe { TERM_COUNT },
    );
    let start_line = unsafe { TERM_COUNT.saturating_sub(visible) };
    b.clip_push(content_x + 1, output_y + 1, out_w.saturating_sub(2), out_h.saturating_sub(2));
    let mut ty = output_y + 15;
    for i in 0..visible {
        if let Some(line) = term_line_at(start_line + i) {
            let rgb = term_sgr_rgb(line.fg);
            b.label(
                content_x + 7,
                ty,
                rgb.r(),
                rgb.g(),
                rgb.b(),
                ATLAS_UI,
                &line.text[..line.len as usize],
            );
            ty += 15;
        }
    }
    b.clip_pop();

    b.label_str(
        g.x + 20,
        g.y + g.h as i32 - 15,
        100,
        100,
        120,
        ATLAS_UI,
        "Press ESC to close, Enter to run command",
    );
}

fn emit_settings_window(
    b: &mut scene::Builder<'_>,
    g: compositor::Rect,
    show_net: bool,
    theme: usize,
) {
    let (bg, title_bg, hint, title) = if theme == 1 {
        ((240u8, 240, 245), (220u8, 220, 230), (80u8, 80, 90), (20u8, 20, 30))
    } else {
        ((28, 28, 38), (40, 40, 55), (100, 100, 120), (255, 255, 255))
    };
    b.panel(g.x + 8, g.y + 8, g.w, g.h, 5, 5, 10, 0);
    b.panel(g.x, g.y, g.w, g.h, bg.0, bg.1, bg.2, 0);
    b.panel(g.x, g.y, g.w, 32, title_bg.0, title_bg.1, title_bg.2, 0);
    b.dot(g.x + 12, g.y + 10, 12, 220, 80, 80);
    b.label(g.x + 80, g.y + 22, title.0, title.1, title.2, ATLAS_BOLD, b"Settings");

    let cb = Checkbox::new("Show NET indicator", g.x + 24, g.y + 70, show_net);
    cb.emit(b);
    let r0 = RadioButton::new("Dark theme", g.x + 24, g.y + 110, theme == 0);
    let r1 = RadioButton::new("Light theme", g.x + 24, g.y + 136, theme == 1);
    r0.emit(b, bg);
    r1.emit(b, bg);
    b.label(
        g.x + 24,
        g.y + g.h as i32 - 24,
        hint.0,
        hint.1,
        hint.2,
        ATLAS_UI,
        b"Click widgets to toggle. ESC closes.",
    );
}

fn hit_test_terminal_input(x: i32, y: i32) -> bool {
    let Some(g) = compositor::get_geom(1) else {
        return false;
    };
    let content_x = g.x + 15;
    let input_y = g.y + 55;
    let input_w = g.w.saturating_sub(120).max(80) as i32;
    x >= content_x && x < content_x + input_w && y >= input_y && y < input_y + 28
}

fn stack_write(buf: &mut [u8], args: core::fmt::Arguments<'_>) -> usize {
    let mut w = StackFmt { buf, pos: 0 };
    let _ = w.write_fmt(args);
    w.pos
}

struct StackFmt<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl Write for StackFmt<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let remaining = self.buf.len().saturating_sub(self.pos);
        let n = bytes.len().min(remaining);
        if n == 0 {
            return if bytes.is_empty() {
                Ok(())
            } else {
                Err(core::fmt::Error)
            };
        }
        self.buf[self.pos..self.pos + n].copy_from_slice(&bytes[..n]);
        self.pos += n;
        if n < bytes.len() {
            Err(core::fmt::Error)
        } else {
            Ok(())
        }
    }
}

/// Setup a main_screen screen showing embedded_graphics capabilities with dynamic hardware info
pub fn setup_main_screen() {
    // Get hardware info
    let hw = get_hardware_info();

    // Reset selected button and update time
    unsafe {
        MAIN_SCREEN_SELECTED_BUTTON = 0;
        MAIN_SCREEN_STATIC_DRAWN = false; // Force full redraw on setup
        MAIN_SCREEN_LAST_SELECTED = None;
        MAIN_SCREEN_LAST_HW_UPDATE = crate::get_time_ms();
    }

    // Enable main_screen mode to prevent UI manager from overwriting our direct GPU draws
    with_ui(|ui_mgr| {
        ui_mgr.clear();
        ui_mgr.set_main_screen_mode(true);
    });

    scene::init();
    rebuild_scene();
    scene::publish_if_dirty();
    if paint_legacy() {
        draw_main_screen_content(&hw, unsafe { MAIN_SCREEN_SELECTED_BUTTON });
    }
}

/// Draw a child window (opened by clicking a button)
/// Draws ONLY the child window on top of existing content for maximum speed
fn draw_child_window(button_index: usize) {
    save_window_backing();
    compositor::raise_or_open(button_index, geom_to_rect(child_geom()));
    unsafe {
        MAIN_SCREEN_OPEN_WINDOW = Some(button_index);
    }
    scene_dirty();
    if paint_legacy() {
        draw_child_window_inner(button_index);
    }
}

fn draw_child_window_inner(button_index: usize) {
    match button_index {
        0 => draw_network_window(),
        1 => draw_terminal_window(),
        2 => draw_settings_window(),
        _ => {}
    }
}

static mut SETTINGS_SHOW_NET: bool = true;
static mut SETTINGS_THEME: usize = 0;

fn default_settings_geom() -> ChildGeom {
    if compact() {
        let w = (SCREEN_WIDTH as u32).saturating_sub(24).max(200);
        let h = (SCREEN_HEIGHT as u32).saturating_sub(80).max(160);
        ChildGeom { x: 12, y: 12, w, h }
    } else {
        ChildGeom {
            x: 240,
            y: 48,
            w: 460,
            h: 360,
        }
    }
}

fn draw_settings_window() {
    if !paint_legacy() {
        return;
    }
    let g = child_geom();
    let show_net = unsafe { SETTINGS_SHOW_NET };
    let theme = unsafe { SETTINGS_THEME };
    let (bg, title_bg, hint_col, title_col) = if theme == 1 {
        (
            (240u8, 240, 245),
            (220u8, 220, 230),
            Rgb888::new(80, 80, 90),
            Rgb888::new(20, 20, 30),
        )
    } else {
        (
            (28, 28, 38),
            (40, 40, 55),
            Rgb888::new(100, 100, 120),
            Rgb888::WHITE,
        )
    };
    d1_display::with_gpu(|gpu| {
        gpu.fill_rect(g.x as u32 + 8, g.y as u32 + 8, g.w, g.h, 5, 5, 10);
        let panel = Panel::new(g.x, g.y, g.w, g.h).with_title("Settings");
        let _ = panel.draw(gpu);
        gpu.fill_rect(g.x as u32, g.y as u32, g.w, g.h, bg.0, bg.1, bg.2);
        gpu.fill_rect(
            g.x as u32, g.y as u32, g.w, 32, title_bg.0, title_bg.1, title_bg.2,
        );
        let _ = Circle::new(Point::new(g.x + 12, g.y + 10), 12)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(220, 80, 80)))
            .draw(gpu);
        let title_style = MonoTextStyle::new(&FONT_9X15_BOLD, title_col);
        let _ = Text::new("Settings", Point::new(g.x + 80, g.y + 22), title_style).draw(gpu);
        let cb = Checkbox::new("Show NET indicator", g.x + 24, g.y + 70, show_net);
        let _ = cb.draw(gpu);
        let r0 = RadioButton::new("Dark theme", g.x + 24, g.y + 110, theme == 0);
        let r1 = RadioButton::new("Light theme", g.x + 24, g.y + 136, theme == 1);
        let _ = r0.draw(gpu);
        let _ = r1.draw(gpu);
        let hint = MonoTextStyle::new(&FONT_7X14, hint_col);
        let _ = Text::new(
            "Click widgets to toggle. ESC closes.",
            Point::new(g.x + 24, g.y + g.h as i32 - 24),
            hint,
        )
        .draw(gpu);
    });
}

fn redraw_net_badge() {
    if !paint_legacy() {
        scene_dirty();
        return;
    }
    let status_y = SCREEN_HEIGHT - 30;
    let show = unsafe { SETTINGS_SHOW_NET };
    d1_display::with_gpu(|gpu| {
        gpu.fill_rect(sx(868) as u32, (status_y + 4) as u32, 50, 22, 25, 25, 35);
        if show {
            let hw = get_hardware_info();
            let net_color = if hw.network_available {
                Rgb888::new(80, 200, 120)
            } else {
                Rgb888::new(150, 150, 160)
            };
            let _ = Circle::new(Point::new(sx(870), status_y + 7), 10)
                .into_styled(PrimitiveStyle::with_fill(net_color))
                .draw(gpu);
            let text_style = MonoTextStyle::new(&FONT_7X14, Rgb888::new(200, 200, 210));
            let _ = Text::new("NET", Point::new(sx(884), status_y + 18), text_style).draw(gpu);
        }
    });
}

/// Draw the Network Statistics window content
fn draw_network_window() {
    if !paint_legacy() {
        return;
    }
    // Pre-compute network info BEFORE entering GPU closure (avoid locks inside)
    // Use is_ip_assigned() which checks for valid IP without needing locks
    let is_online = crate::net::is_ip_assigned();

    let ip = crate::net::get_my_ip();
    let ip_octets = ip.octets();
    let gateway = crate::net::GATEWAY.octets();
    let dns = crate::net::DNS_SERVER.octets();
    let prefix = crate::net::PREFIX_LEN;

    // Pre-format strings to avoid allocations in GPU closure
    let ip_str = format!(
        "{}.{}.{}.{}/{}",
        ip_octets[0], ip_octets[1], ip_octets[2], ip_octets[3], prefix
    );
    let gw_str = format!(
        "{}.{}.{}.{}",
        gateway[0], gateway[1], gateway[2], gateway[3]
    );
    let dns_str = format!("{}.{}.{}.{}", dns[0], dns[1], dns[2], dns[3]);

    let g = child_geom();
    let win_x = g.x as u32;
    let win_y = g.y as u32;
    let win_w = g.w;
    let win_h = g.h;

    d1_display::with_gpu(|gpu| {
        gpu.fill_rect(win_x + 8, win_y + 8, win_w, win_h, 5, 5, 10);
        gpu.fill_rect(win_x, win_y, win_w, win_h, 28, 28, 38);
        gpu.fill_rect(win_x, win_y, win_w, 32, 40, 40, 55);

        let _ = Rectangle::new(Point::new(g.x, g.y), Size::new(win_w, win_h))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);

        let _ = Circle::new(Point::new(g.x + 12, g.y + 10), 12)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(220, 80, 80)))
            .draw(gpu);
        let _ = Circle::new(Point::new(g.x + 32, g.y + 10), 12)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(230, 180, 80)))
            .draw(gpu);
        let _ = Circle::new(Point::new(g.x + 52, g.y + 10), 12)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(80, 200, 120)))
            .draw(gpu);

        let title_style = MonoTextStyle::new(&FONT_9X15_BOLD, Rgb888::WHITE);
        let _ = Text::new(
            "Network Statistics",
            Point::new(g.x + 170, g.y + 22),
            title_style,
        )
        .draw(gpu);
        draw_image(
            gpu,
            win_x + win_w - LOGO_SMALL_SIZE - 8,
            win_y + 4,
            LOGO_SMALL_SIZE,
            LOGO_SMALL_SIZE,
            LOGO_SMALL,
        );

        // Content styles
        let label_style = MonoTextStyle::new(&FONT_7X14, Rgb888::new(230, 180, 80));
        let value_style = MonoTextStyle::new(&FONT_7X14, Rgb888::WHITE);
        let hint_style = MonoTextStyle::new(&FONT_7X14, Rgb888::new(100, 100, 120));

        let x = g.x + 20;
        let mut y = g.y + 60;

        // Device section - use static strings
        let _ = Text::new("Device:", Point::new(x, y), label_style).draw(gpu);
        y += 16;
        let _ = Text::new(
            "Type:    VirtIO Network Device",
            Point::new(x + 10, y),
            value_style,
        )
        .draw(gpu);
        y += 14;
        let _ = Text::new("Address: 0x10001000", Point::new(x + 10, y), value_style).draw(gpu);
        y += 14;

        // Status
        if is_online {
            let _ = Text::new(
                "Status:  * ONLINE",
                Point::new(x + 10, y),
                MonoTextStyle::new(&FONT_7X14, Rgb888::new(80, 200, 120)),
            )
            .draw(gpu);
        } else {
            let _ = Text::new(
                "Status:  X OFFLINE",
                Point::new(x + 10, y),
                MonoTextStyle::new(&FONT_7X14, Rgb888::new(220, 80, 80)),
            )
            .draw(gpu);
        }
        y += 22;

        // Configuration
        let _ = Text::new("Configuration:", Point::new(x, y), label_style).draw(gpu);
        y += 16;

        // Use pre-formatted strings
        let _ = Text::new("IP:      ", Point::new(x + 10, y), value_style).draw(gpu);
        let _ = Text::new(&ip_str, Point::new(x + 64, y), value_style).draw(gpu);
        y += 14;
        let _ = Text::new("Gateway: ", Point::new(x + 10, y), value_style).draw(gpu);
        let _ = Text::new(&gw_str, Point::new(x + 64, y), value_style).draw(gpu);
        y += 14;
        let _ = Text::new("DNS:     ", Point::new(x + 10, y), value_style).draw(gpu);
        let _ = Text::new(&dns_str, Point::new(x + 64, y), value_style).draw(gpu);
        y += 22;

        // Protocol Stack
        let _ = Text::new("Protocol Stack:", Point::new(x, y), label_style).draw(gpu);
        y += 16;
        let _ = Text::new(
            "smoltcp - Lightweight TCP/IP",
            Point::new(x + 10, y),
            value_style,
        )
        .draw(gpu);
        y += 14;
        let _ = Text::new("ICMP, UDP, TCP, ARP", Point::new(x + 10, y), value_style).draw(gpu);

        // Close hint
        let _ = Text::new(
            "Press ESC or click red button to close",
            Point::new(g.x + 20, g.y + win_h as i32 - 20),
            hint_style,
        )
        .draw(gpu);
    });
}

/// Draw the Terminal window content
fn draw_terminal_window() {
    scene_dirty();
    if !paint_legacy() {
        return;
    }
    let g = child_geom();
    let win_x = g.x as u32;
    let win_y = g.y as u32;
    let win_w = g.w;
    let win_h = g.h;

    d1_display::with_gpu(|gpu| {
        // Shadow + window background in one batch
        gpu.fill_rect(win_x + 8, win_y + 8, win_w, win_h, 5, 5, 10);
        gpu.fill_rect(win_x, win_y, win_w, win_h, 28, 28, 38);
        gpu.fill_rect(win_x, win_y, win_w, 32, 40, 40, 55);

        let _ = Rectangle::new(Point::new(g.x, g.y), Size::new(win_w, win_h))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);

        // Traffic light buttons (cast to i32 for Point)
        let _ = Circle::new(Point::new(g.x + 12, g.y + 10), 12)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(220, 80, 80)))
            .draw(gpu);
        let _ = Circle::new(Point::new(g.x + 32, g.y + 10), 12)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(230, 180, 80)))
            .draw(gpu);
        let _ = Circle::new(Point::new(g.x + 52, g.y + 10), 12)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(80, 200, 120)))
            .draw(gpu);

        let title_style = MonoTextStyle::new(&FONT_9X15_BOLD, Rgb888::WHITE);
        let _ = Text::new(
            "Terminal",
            Point::new(g.x + (win_w as i32 / 2) - 36, g.y + 22),
            title_style,
        )
        .draw(gpu);
        draw_image(
            gpu,
            win_x + win_w - LOGO_SMALL_SIZE - 8,
            win_y + 4,
            LOGO_SMALL_SIZE,
            LOGO_SMALL_SIZE,
            LOGO_SMALL,
        );

        let hint_style = MonoTextStyle::new(&FONT_7X14, Rgb888::new(100, 100, 120));
        let value_style = MonoTextStyle::new(&FONT_7X14, Rgb888::WHITE);

        let content_y = win_y + 45;
        let content_x = win_x + 15;
        let input_w = win_w.saturating_sub(120).max(80);
        let out_w = win_w.saturating_sub(30).max(80);
        let out_h = win_h.saturating_sub(150).max(40);

        // Command label
        let _ = Text::new(
            "Command:",
            Point::new(content_x as i32, content_y as i32),
            hint_style,
        )
        .draw(gpu);

        // Input field background (dark) - wider: 580px
        let input_y = content_y + 10;
        let _ = RoundedRectangle::with_equal_corners(
            Rectangle::new(
                Point::new(content_x as i32, input_y as i32),
                Size::new(input_w, 28),
            ),
            Size::new(4, 4),
        )
        .into_styled(PrimitiveStyle::with_fill(Rgb888::new(18, 18, 28)))
        .draw(gpu);

        // Input field border
        let _ = RoundedRectangle::with_equal_corners(
            Rectangle::new(
                Point::new(content_x as i32, input_y as i32),
                Size::new(input_w, 28),
            ),
            Size::new(4, 4),
        )
        .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(80, 80, 100), 1))
        .draw(gpu);

        // Draw current input text
        let input_text = unsafe {
            core::str::from_utf8(&TERMINAL_INPUT_BUFFER[..TERMINAL_INPUT_LEN]).unwrap_or("")
        };
        let _ = Text::new(
            input_text,
            Point::new(content_x as i32 + 7, input_y as i32 + 19),
            value_style,
        )
        .draw(gpu);

        // Draw cursor (always visible, simple block cursor)
        let cursor_x = content_x as i32 + 7 + (unsafe { TERMINAL_INPUT_LEN } as i32 * 7);
        if cursor_x < content_x as i32 + input_w as i32 - 10 {
            let _ = Rectangle::new(Point::new(cursor_x, input_y as i32 + 5), Size::new(2, 16))
                .into_styled(PrimitiveStyle::with_fill(Rgb888::new(200, 200, 220)))
                .draw(gpu);
        }

        // Run/Cancel button (right of input field) - red Cancel when running, blue Run otherwise
        let btn_x = content_x as i32 + input_w as i32 + 10;
        let is_running = unsafe { TERMINAL_COMMAND_RUNNING };
        let (btn_color, btn_text) = if is_running {
            (Rgb888::new(200, 80, 80), "Cancel") // Red cancel button
        } else {
            (Rgb888::new(80, 140, 200), "Run") // Blue run button
        };
        let _ = RoundedRectangle::with_equal_corners(
            Rectangle::new(Point::new(btn_x, input_y as i32), Size::new(80, 28)),
            Size::new(4, 4),
        )
        .into_styled(PrimitiveStyle::with_fill(btn_color))
        .draw(gpu);
        let text_x = if is_running { btn_x + 14 } else { btn_x + 25 }; // Center text differently
        let _ = Text::new(
            btn_text,
            Point::new(text_x, input_y as i32 + 19),
            MonoTextStyle::new(&FONT_7X14, Rgb888::WHITE),
        )
        .draw(gpu);

        // CWD label (shows current working directory)
        let output_label_y = input_y + 40;
        let cwd = crate::utils::cwd_get();
        let cwd_label = alloc::format!("{}$", cwd);
        let _ = Text::new(
            &cwd_label,
            Point::new(content_x as i32, output_label_y as i32),
            hint_style,
        )
        .draw(gpu);

        // Output area background - larger: 670x340
        let output_y = output_label_y + 10;
        let _ = RoundedRectangle::with_equal_corners(
            Rectangle::new(
                Point::new(content_x as i32, output_y as i32),
                Size::new(out_w, out_h),
            ),
            Size::new(4, 4),
        )
        .into_styled(PrimitiveStyle::with_fill(Rgb888::new(10, 10, 15)))
        .draw(gpu);

        // Output area border
        let _ = RoundedRectangle::with_equal_corners(
            Rectangle::new(
                Point::new(content_x as i32, output_y as i32),
                Size::new(out_w, out_h),
            ),
            Size::new(4, 4),
        )
        .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
        .draw(gpu);

        let mut y = output_y as i32 + 15;
        let visible = core::cmp::min(22, unsafe { TERM_COUNT });
        let start = unsafe { TERM_COUNT.saturating_sub(visible) };
        for i in 0..visible {
            if let Some(line) = term_line_at(start + i) {
                let text = core::str::from_utf8(&line.text[..line.len as usize]).unwrap_or("");
                let style = MonoTextStyle::new(&FONT_7X14, term_sgr_rgb(line.fg));
                let _ = Text::new(text, Point::new(content_x as i32 + 7, y), style).draw(gpu);
                y += 15;
            }
        }

        // Close hint at bottom
        let _ = Text::new(
            "Press ESC to close, Enter to run command",
            Point::new(g.x + 20, g.y + win_h as i32 - 15),
            hint_style,
        )
        .draw(gpu);
    });
}

/// Fast partial redraw of ONLY the input field (for responsive typing)
/// This is much faster than redrawing the entire terminal window
fn draw_terminal_input_only() {
    scene_dirty();
    if !paint_legacy() {
        return;
    }
    let g = child_geom();
    let content_x = g.x as u32 + 15;
    let input_y = g.y as u32 + 55;
    let input_w = g.w.saturating_sub(120).max(80);

    d1_display::with_gpu(|gpu| {
        let value_style = MonoTextStyle::new(&FONT_7X14, Rgb888::WHITE);
        gpu.fill_rect(
            content_x + 1,
            input_y + 1,
            input_w.saturating_sub(2),
            26,
            18,
            18,
            28,
        );

        let input_text = unsafe {
            core::str::from_utf8(&TERMINAL_INPUT_BUFFER[..TERMINAL_INPUT_LEN]).unwrap_or("")
        };
        let _ = Text::new(
            input_text,
            Point::new(content_x as i32 + 7, input_y as i32 + 19),
            value_style,
        )
        .draw(gpu);

        let cursor_x = content_x as i32 + 7 + (unsafe { TERMINAL_INPUT_LEN } as i32 * 7);
        if cursor_x < content_x as i32 + input_w as i32 - 10 {
            let _ = Rectangle::new(Point::new(cursor_x, input_y as i32 + 5), Size::new(2, 16))
                .into_styled(PrimitiveStyle::with_fill(Rgb888::new(200, 200, 220)))
                .draw(gpu);
        }
    });
}

/// Fast partial redraw of ONLY the output area (for responsive command output)
/// This is much faster than redrawing the entire terminal window
fn draw_terminal_output_only() {
    scene_dirty();
    if !paint_legacy() {
        return;
    }
    let g = child_geom();
    let content_x = g.x as u32 + 15;
    let output_y = g.y as u32 + 105;
    let out_w = g.w.saturating_sub(30).max(80);
    let out_h = g.h.saturating_sub(150).max(40);

    d1_display::with_gpu(|gpu| {
        gpu.fill_rect(
            content_x + 1,
            output_y + 1,
            out_w.saturating_sub(2),
            out_h.saturating_sub(2),
            10,
            10,
            15,
        );

        let mut y = output_y as i32 + 15;
        let visible = core::cmp::min(((out_h as i32 - 20) / 15).max(1) as usize, unsafe {
            TERM_COUNT
        });
        let start = unsafe { TERM_COUNT.saturating_sub(visible) };
        for i in 0..visible {
            if let Some(line) = term_line_at(start + i) {
                let text = core::str::from_utf8(&line.text[..line.len as usize]).unwrap_or("");
                let style = MonoTextStyle::new(&FONT_7X14, term_sgr_rgb(line.fg));
                let _ = Text::new(text, Point::new(content_x as i32 + 7, y), style).draw(gpu);
                y += 15;
            }
        }
    });
}

/// Fast partial redraw of ONLY the Run/Cancel button (for responsive button state changes)
/// This is much faster than redrawing the entire terminal window
fn draw_terminal_button_only() {
    scene_dirty();
    if !paint_legacy() {
        return;
    }
    let g = child_geom();
    let content_x = g.x as u32 + 15;
    let input_y = g.y as u32 + 55;
    let input_w = g.w.saturating_sub(120).max(80);
    let btn_x = (content_x + input_w + 10) as i32;

    d1_display::with_gpu(|gpu| {
        let is_running = unsafe { TERMINAL_COMMAND_RUNNING };
        let (btn_color, btn_text) = if is_running {
            (Rgb888::new(200, 80, 80), "Cancel") // Red cancel button
        } else {
            (Rgb888::new(80, 140, 200), "Run") // Blue run button
        };

        // Clear button area and redraw
        let _ = RoundedRectangle::with_equal_corners(
            Rectangle::new(Point::new(btn_x, input_y as i32), Size::new(80, 28)),
            Size::new(4, 4),
        )
        .into_styled(PrimitiveStyle::with_fill(btn_color))
        .draw(gpu);

        let text_x = if is_running { btn_x + 14 } else { btn_x + 25 };
        let _ = Text::new(
            btn_text,
            Point::new(text_x, input_y as i32 + 19),
            MonoTextStyle::new(&FONT_7X14, Rgb888::WHITE),
        )
        .draw(gpu);
    });
}

/// Execute a command in the terminal window and capture output
///
/// This initiates command execution. For U-mode ELFs (the normal case),
/// execute_command does not return - control flow goes through:
/// sret -> U-mode -> SYS_EXIT -> trap -> restore_kernel_context -> signal_completion -> hart_loop
///
/// The result is later picked up by check_gui_command_completion() in gpuid_tick().
fn terminal_execute_command() {
    use crate::device::uart::write_line;
    use crate::lock::utils::OUTPUT_CAPTURE;
    use crate::services::gui_cmd::GUI_CMD_RUNNING;
    use core::sync::atomic::Ordering;

    let cmd_len = unsafe { TERMINAL_INPUT_LEN };
    if cmd_len == 0 {
        return;
    }

    // Get the command string
    let cmd_bytes = unsafe { &TERMINAL_INPUT_BUFFER[..cmd_len] };
    let cmd_str = match core::str::from_utf8(cmd_bytes) {
        Ok(s) => s.trim(),
        Err(_) => return,
    };

    if cmd_str.is_empty() {
        return;
    }

    // Check if already running
    if GUI_CMD_RUNNING.load(Ordering::SeqCst) {
        return;
    }

    // Split into command and arguments
    let mut parts = cmd_str.splitn(2, ' ');
    let cmd = parts.next().unwrap_or("");
    let args = parts.next().unwrap_or("");

    // Mark command as running in GUI (so Cancel button shows)
    unsafe {
        TERMINAL_COMMAND_RUNNING = true;
    }
    draw_terminal_button_only();
    if paint_legacy() {
        d1_display::flush();
    }

    // Clear input immediately
    unsafe {
        TERMINAL_INPUT_LEN = 0;
    }
    draw_terminal_input_only();

    // Set up for U-mode execution with GUI return path:
    // 1. Set GUI context so restore_kernel_context takes the GUI path
    crate::scripting::set_gui_context(true);

    // 2. Mark GUI_CMD as running so signal_completion stores the result properly
    GUI_CMD_RUNNING.store(true, Ordering::SeqCst);

    // 3. Start output capture (signal_completion will stop and capture it)
    {
        let mut cap = OUTPUT_CAPTURE.lock();
        cap.capturing = true;
        cap.len = 0;
    }

    // Execute command - for U-mode ELFs this does NOT return!
    // Control goes: sret -> U-mode -> SYS_EXIT -> trap -> restore_kernel_context
    //            -> signal_completion -> hart_loop
    // check_gui_command_completion() in gpuid_tick will poll for the result.
    crate::scripting::execute_command(cmd.as_bytes(), args.as_bytes());

    // Stop output capture
    let output = {
        let mut cap = OUTPUT_CAPTURE.lock();
        cap.capturing = false;
        let len = cap.len.min(crate::lock::state::output::OUTPUT_BUFFER_SIZE);
        alloc::vec::Vec::from(&cap.buffer[..len])
    };

    // Clear GUI context
    crate::scripting::set_gui_context(false);
    GUI_CMD_RUNNING.store(false, Ordering::SeqCst);

    term_clear();
    term_append_bytes(&output);
    unsafe {
        TERMINAL_COMMAND_RUNNING = false;
    }

    draw_terminal_output_only();
    draw_terminal_button_only();
    // No explicit flush: gpuid's deferred end-of-tick flush presents this
    // frame (avoids a redundant second dirty-rect copy).
}

/// Check for GUI command completion and update terminal output
/// Called from gpuid tick to poll for results
pub fn check_gui_command_completion() {
    use crate::device::uart::{write_line, write_str};

    // Only check if a command is running
    if !unsafe { TERMINAL_COMMAND_RUNNING } {
        return;
    }

    // Poll for result
    if let Some(result) = crate::services::gui_cmd::poll_result() {
        let mut buf = [0u8; 8];

        term_clear();
        term_append_bytes(&result.output);

        // Mark command as finished
        unsafe {
            TERMINAL_COMMAND_RUNNING = false;
        }

        // Update UI (gpuid's deferred flush presents it - no explicit flush)
        draw_terminal_output_only();
        draw_terminal_button_only();
    }
}

/// Refresh terminal output during WASM execution (called by terminal_refresh syscall)
/// Copies current OUTPUT_CAPTURE to TERMINAL_OUTPUT and redraws the window
pub fn refresh_terminal_output() {
    use crate::lock::state::output::OUTPUT_BUFFER_SIZE;
    use crate::lock::utils::OUTPUT_CAPTURE;

    // Only refresh if terminal window is open and command is running
    let window_open = unsafe { MAIN_SCREEN_OPEN_WINDOW };
    if window_open != Some(1) {
        // Not terminal window
        return;
    }

    // Copy current output capture to terminal buffer
    {
        let cap = OUTPUT_CAPTURE.lock();
        let len = cap.len.min(OUTPUT_BUFFER_SIZE);
        term_clear();
        term_append_bytes(&cap.buffer[..len]);
    }

    // Fast partial redraw of just the output area (much faster than full window redraw)
    draw_terminal_output_only();
    // Flush deferred to end of gpuid tick
}

/// Check if command cancellation was requested (for WASM syscall)
/// Returns true if cancel was requested, clearing the flag
pub fn should_cancel() -> bool {
    unsafe {
        let requested = TERMINAL_CANCEL_REQUESTED;
        if requested {
            TERMINAL_CANCEL_REQUESTED = false; // Clear after reading
        }
        requested
    }
}

/// Request cancellation of running command (called by Cancel button or Ctrl+C)
pub fn request_cancel() {
    unsafe {
        if TERMINAL_COMMAND_RUNNING {
            TERMINAL_CANCEL_REQUESTED = true;
            // Also add "^C" to output
            term_append_bytes(b"^C\n");
            draw_terminal_window();
            // Flush deferred to end of gpuid tick
        }
    }
}

/// Clear cancellation flag (called at command start)
fn clear_cancel() {
    unsafe {
        TERMINAL_CANCEL_REQUESTED = false;
    }
}

/// Handle terminal window input (keyboard chars)
fn handle_terminal_input(key_code: u16, _key_value: i32) -> bool {
    use crate::device::uart::{write_line, write_str};
    use crate::input::{KEY_BACKSPACE, KEY_ENTER};
    match key_code {
        KEY_ENTER => {
            terminal_execute_command();
            true
        }
        KEY_BACKSPACE => {
            unsafe {
                if TERMINAL_INPUT_LEN > 0 {
                    TERMINAL_INPUT_LEN -= 1;
                    // Fast partial redraw of input field only
                    draw_terminal_input_only();
                    // Flush deferred to end of gpuid tick
                }
            }
            true
        }
        _ => false,
    }
}

/// Helper to format u16 as string
fn format_u16(n: u16, buf: &mut [u8; 8]) -> &str {
    let mut i = buf.len();
    let mut n = n;
    if n == 0 {
        buf[7] = b'0';
        return core::str::from_utf8(&buf[7..]).unwrap();
    }
    while n > 0 && i > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    core::str::from_utf8(&buf[i..]).unwrap()
}

/// Handle character input for terminal (from ASCII key events)
fn handle_terminal_char(ch: u8) {
    // Handle Ctrl+C (0x03) - request cancellation
    if ch == 0x03 {
        unsafe {
            TERMINAL_INPUT_LEN = 0;
        } // Clear input
        request_cancel();
        return;
    }

    if ch >= 0x20 && ch < 0x7F {
        // Printable ASCII
        unsafe {
            if TERMINAL_INPUT_LEN < TERMINAL_INPUT_MAX - 1 {
                TERMINAL_INPUT_BUFFER[TERMINAL_INPUT_LEN] = ch;
                TERMINAL_INPUT_LEN += 1;
                // Fast partial redraw of input field only
                draw_terminal_input_only();
                // Flush deferred to end of gpuid tick
            }
        }
    }
}

/// Check if terminal send button was clicked
fn hit_test_terminal_send_button(x: i32, y: i32) -> bool {
    let g = child_geom();
    let content_x = g.x + 15;
    let input_y = g.y + 55;
    let input_w = g.w.saturating_sub(120).max(80) as i32;
    let btn_x = content_x + input_w + 10;
    x >= btn_x && x < btn_x + 80 && y >= input_y && y < input_y + 28
}

/// Redraw the main_screen screen with the given selected button index
/// Public entry point that calls inner function
fn draw_main_screen_content(hw: &HardwareInfo, selected_button: usize) {
    draw_main_screen_content_inner(hw, selected_button);
}

/// Inner function to draw main main_screen content (used by both normal draw and child window background).
///
/// Phase 4: skipped when HDL has been published (or `raster_soft` presented).
/// Dual-draw only until that point (`paint_legacy`).
fn draw_main_screen_content_inner(hw: &HardwareInfo, selected_button: usize) {
    // Check if static content is already drawn - skip expensive operations if so
    let static_drawn = unsafe { MAIN_SCREEN_STATIC_DRAWN };

    d1_display::with_gpu(|gpu| {
        // Only clear and draw static content if not already cached
        if !static_drawn {
            // Clear to dark background (desktop) - EXPENSIVE, skip if already drawn
            let _ = gpu.clear(0x15, 0x15, 0x1E);
        }

        // === Draw Window using reusable Window component (no controls) ===
        let window = Window::new(
            "HAVY OS - System Information",
            sx(10),
            sy(10),
            (sx(1004) as u32).min((SCREEN_WIDTH - 20).max(1) as u32),
            (sy(710) as u32).min((SCREEN_HEIGHT - 40).max(1) as u32),
        )
        .with_controls(false); // Hide traffic light buttons on main window
        let _content = window.draw_fast(gpu);

        // Content is positioned relative to window content area
        let text_style = MonoTextStyle::new(&FONT_7X14, Rgb888::new(200, 200, 210));
        let accent_style = MonoTextStyle::new(&FONT_7X14, Rgb888::new(80, 140, 200));

        // === Left Column: About ===
        let col1_x = 30;
        let _ = Text::new("About This System", Point::new(col1_x, 70), accent_style).draw(gpu);
        let _ = Line::new(Point::new(col1_x, 75), Point::new(col1_x + 150, 75))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);

        let _ = Text::new("OS Name:      HAVY OS", Point::new(col1_x, 95), text_style).draw(gpu);
        // Use version from Cargo.toml
        let version_str = format!("Version:      {}", VERSION);
        let _ = Text::new(&version_str, Point::new(col1_x, 110), text_style).draw(gpu);
        let _ = Text::new(
            "Architecture: RISC-V RV64GC",
            Point::new(col1_x, 140),
            text_style,
        )
        .draw(gpu);
        let _ = Text::new(
            "Platform:     Virtual Machine",
            Point::new(col1_x, 155),
            text_style,
        )
        .draw(gpu);

        // Hardware info section with dynamic values
        let _ = Text::new("Hardware", Point::new(col1_x, 185), accent_style).draw(gpu);
        let _ = Line::new(Point::new(col1_x, 190), Point::new(col1_x + 100, 190))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);

        // Dynamic CPU count
        let mut cpu_buf = [0u8; 32];
        let cpu_str = format_cpu_str(hw.cpu_count, &mut cpu_buf);
        let _ = Text::new(cpu_str, Point::new(col1_x, 210), text_style).draw(gpu);

        // Dynamic memory (used / total)
        let mut mem_buf = [0u8; 48];
        let mem_str = format_memory_str(hw.memory_used_kb, hw.memory_total_kb, &mut mem_buf);
        let _ = Text::new(mem_str, Point::new(col1_x, 225), text_style).draw(gpu);
        let mem_ratio = if hw.memory_total_kb > 0 {
            hw.memory_used_kb as f32 / hw.memory_total_kb as f32
        } else {
            0.0
        };
        let mut mem_bar = ProgressBar::new(col1_x, 228, 180, 6);
        mem_bar.set_progress(mem_ratio);
        let _ = mem_bar.draw(gpu);

        let display_str = format!(
            "Display:      {}x{}",
            d1_display::DISPLAY_WIDTH,
            d1_display::DISPLAY_HEIGHT
        );
        let _ = Text::new(&display_str, Point::new(col1_x, 240), text_style).draw(gpu);

        // Dynamic disk (used / total)
        let mut disk_buf = [0u8; 48];
        let disk_str = format_disk_str(hw.disk_used_kb, hw.disk_total_kb, &mut disk_buf);
        let _ = Text::new(disk_str, Point::new(col1_x, 255), text_style).draw(gpu);

        // Dynamic network with IP address
        let mut net_buf = [0u8; 48];
        let net_str = format_network_str(hw.network_available, &hw.ip_addr, &mut net_buf);
        let _ = Text::new(net_str, Point::new(col1_x, 270), text_style).draw(gpu);

        // === Right Column: Features ===
        let col2_x = if compact() { sx(30) } else { 550 };
        let _ = Text::new("Features", Point::new(col2_x, 70), accent_style).draw(gpu);
        let _ = Line::new(Point::new(col2_x, 75), Point::new(col2_x + 100, 75))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);

        // Feature checkmarks
        let features = [
            "Multi-core SMP support",
            "Preemptive scheduler",
            "VirtIO device drivers",
            "TCP/IP networking (smoltcp)",
            "Simple File System",
            "WASM application runtime",
            "GPU-accelerated display",
            "Interactive shell",
        ];

        for (i, feature) in features.iter().enumerate() {
            let y = 95 + (i as i32 * 20);
            // Checkmark
            let _ = Rectangle::new(Point::new(col2_x, y - 10), Size::new(12, 12))
                .into_styled(PrimitiveStyle::with_fill(Rgb888::new(80, 200, 120)))
                .draw(gpu);
            let _ = Line::new(Point::new(col2_x + 2, y - 4), Point::new(col2_x + 5, y - 1))
                .into_styled(PrimitiveStyle::with_stroke(Rgb888::WHITE, 2))
                .draw(gpu);
            let _ = Line::new(
                Point::new(col2_x + 5, y - 1),
                Point::new(col2_x + 10, y - 8),
            )
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::WHITE, 2))
            .draw(gpu);
            let _ = Text::new(feature, Point::new(col2_x + 20, y), text_style).draw(gpu);
        }

        // === Quick Actions with keyboard selection (adjusted for 1024x768) ===
        let _ = Text::new("Quick Actions", Point::new(col1_x, 470), accent_style).draw(gpu);
        let _ = Line::new(Point::new(col1_x, 475), Point::new(col1_x + 120, 475))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);

        // Navigation hint
        let hint_style = MonoTextStyle::new(&FONT_7X14, Rgb888::new(100, 100, 120));
        let _ = Text::new(
            "Use arrows to select, Enter to open",
            Point::new(col1_x, 488),
            hint_style,
        )
        .draw(gpu);

        // Mark static content as drawn so next time we skip the expensive clear
        unsafe {
            MAIN_SCREEN_STATIC_DRAWN = true;
        }

        // Network and Terminal buttons, left aligned (adjusted for 1024x768)
        let buttons = [
            ("Network", sx(30)),
            ("Terminal", sx(150)),
            ("Settings", sx(270)),
        ];

        for (i, (label, x)) in buttons.iter().enumerate() {
            let is_selected = i == selected_button;
            let bg_color = if is_selected {
                Rgb888::new(80, 140, 200) // Highlight selected
            } else {
                Rgb888::new(50, 50, 70)
            };
            let border_color = if is_selected {
                Rgb888::new(120, 180, 240)
            } else {
                Rgb888::new(60, 60, 80)
            };

            // Button background (110 width)
            let _ = RoundedRectangle::with_equal_corners(
                Rectangle::new(Point::new(*x, sy(500)), Size::new(110, 32)),
                Size::new(4, 4),
            )
            .into_styled(PrimitiveStyle::with_fill(bg_color))
            .draw(gpu);

            // Button border
            let _ = RoundedRectangle::with_equal_corners(
                Rectangle::new(Point::new(*x, sy(500)), Size::new(110, 32)),
                Size::new(4, 4),
            )
            .into_styled(PrimitiveStyle::with_stroke(
                border_color,
                if is_selected { 2 } else { 1 },
            ))
            .draw(gpu);

            let text_color = if is_selected {
                Rgb888::WHITE
            } else {
                Rgb888::new(200, 200, 210)
            };
            let btn_text_style = MonoTextStyle::new(&FONT_7X14, text_color);
            let _ = Text::new(label, Point::new(*x + 8, sy(520)), btn_text_style).draw(gpu);
        }

        // === Running Services (positioned to not overlap with buttons) ===
        let services_x = if compact() { sx(250) } else { 700 };
        let _ = Text::new(
            "Running Services",
            Point::new(services_x, 310),
            accent_style,
        )
        .draw(gpu);
        let _ = Line::new(
            Point::new(services_x, 315),
            Point::new(services_x + 140, 315),
        )
        .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
        .draw(gpu);

        let services = [
            ("shell", true),
            ("httpd", true),
            ("tcpd", true),
            ("sysmond", true),
        ];

        // Services in a vertical list for cleaner layout
        for (i, (name, running)) in services.iter().enumerate() {
            let x = services_x;
            let y = 335 + (i as i32 * 18);
            let color = if *running {
                Rgb888::new(80, 200, 120)
            } else {
                Rgb888::new(150, 150, 160)
            };
            let _ = Circle::new(Point::new(x, y), 8)
                .into_styled(PrimitiveStyle::with_fill(color))
                .draw(gpu);
            let _ = Text::new(name, Point::new(x + 14, y + 6), text_style).draw(gpu);
        }

        // === Welcome Message (adjusted for 1024x768) ===
        let welcome_style = MonoTextStyle::new(&FONT_7X14, Rgb888::new(160, 160, 175));
        let _ = Text::new(
            "HAVY OS is a lightweight operating system written in Rust, running on a",
            Point::new(col1_x, 560),
            welcome_style,
        )
        .draw(gpu);
        let _ = Text::new(
            "RISC-V virtual machine in your browser.",
            Point::new(col1_x, 575),
            welcome_style,
        )
        .draw(gpu);

        // === Footer info ===
        let _ = Line::new(Point::new(30, 610), Point::new(994, 610))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);

        let footer_style = MonoTextStyle::new(&FONT_7X14, Rgb888::new(120, 120, 140));
        let _ = Text::new(
            "Built with: Rust, embedded-graphics, smoltcp, wasmi",
            Point::new(30, 630),
            footer_style,
        )
        .draw(gpu);
        let _ = Text::new(
            "License: MIT | github.com/elribonazo/riscv-vm",
            Point::new(30, 645),
            footer_style,
        )
        .draw(gpu);

        // Version badge - use version from Cargo.toml
        let _ = RoundedRectangle::with_equal_corners(
            Rectangle::new(Point::new(870, 620), Size::new(120, 24)),
            Size::new(4, 4),
        )
        .into_styled(PrimitiveStyle::with_fill(Rgb888::new(80, 140, 200)))
        .draw(gpu);
        let badge_version = format!("v{}", VERSION);
        let _ = Text::new(&badge_version, Point::new(890, 636), text_style).draw(gpu);

        // === Status Bar (at 1024x768 screen bottom) ===
        let status_y = SCREEN_HEIGHT - 30;
        let _ = Rectangle::new(Point::new(0, status_y), Size::new(SCREEN_WIDTH as u32, 30))
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(25, 25, 35)))
            .draw(gpu);

        let _ = Text::new(
            "HAVY OS | GPU Active",
            Point::new(10, status_y + 18),
            text_style,
        )
        .draw(gpu);

        // Display date/time from RTC, or uptime as fallback
        let time_str = if let Some(dt) = crate::device::rtc::get_datetime() {
            let month_name = match dt.month {
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
            format!(
                "{} {:02} {:02}:{:02}",
                month_name, dt.day, dt.hour, dt.minute
            )
        } else {
            let uptime_ms = crate::get_time_ms() as u64;
            let uptime_secs = uptime_ms / 1000;
            let hours = uptime_secs / 3600;
            let minutes = (uptime_secs % 3600) / 60;
            let seconds = uptime_secs % 60;
            format!("Up: {:02}:{:02}:{:02}", hours, minutes, seconds)
        };
        let _ = Text::new(&time_str, Point::new(sx(460), status_y + 18), text_style).draw(gpu);

        // Status indicators
        if unsafe { SETTINGS_SHOW_NET } {
            let net_color = if hw.network_available {
                Rgb888::new(80, 200, 120)
            } else {
                Rgb888::new(150, 150, 160)
            };
            let _ = Circle::new(Point::new(sx(870), status_y + 7), 10)
                .into_styled(PrimitiveStyle::with_fill(net_color))
                .draw(gpu);
            let _ = Text::new("NET", Point::new(sx(884), status_y + 18), text_style).draw(gpu);
        }

        let _ = Circle::new(Point::new(sx(920), status_y + 7), 10)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(80, 200, 120)))
            .draw(gpu);
        let _ = Text::new("CPU", Point::new(sx(934), status_y + 18), text_style).draw(gpu);

        let _ = Circle::new(Point::new(sx(970), status_y + 7), 10)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(230, 180, 80)))
            .draw(gpu);
        let _ = Text::new("MEM", Point::new(sx(984), status_y + 18), text_style).draw(gpu);
    });

    // Snapshot the desktop before overlaying child windows.
    if paint_legacy() && !static_drawn {
        compositor::capture_desktop();
    }
    if compositor::any_open() {
        for w in compositor::windows() {
            draw_child_window_inner(w.kind);
        }
    }
    // Flush deferred to end of gpuid tick
}

/// Handle input for main_screen screen (keyboard navigation and mouse)
/// Returns Some(button_index) if Enter was pressed on a button
pub fn handle_main_screen_input(event: input::InputEvent) -> Option<usize> {
    // Check if a child window is open
    let open_window = unsafe { MAIN_SCREEN_OPEN_WINDOW };

    // Handle mouse position events
    if event.event_type == EV_ABS {
        match event.code {
            ABS_X => {
                set_cursor_pos(event.value, unsafe { CURSOR_Y });
            }
            ABS_Y => {
                set_cursor_pos(unsafe { CURSOR_X }, event.value);
                if unsafe { WINDOW_DRAG } && is_left_button_pressed() && open_window.is_some() {
                    let (cx, cy) = get_cursor_pos();
                    let g = child_geom();
                    move_focused(ChildGeom {
                        x: cx - unsafe { DRAG_OFF_X },
                        y: cy - unsafe { DRAG_OFF_Y },
                        w: g.w,
                        h: g.h,
                    });
                }
            }
            _ => {}
        }
        return None;
    }

    // Handle character events (typed characters respecting keyboard layout)
    // These come from browser with actual character codes (e.g., '/' from Shift+7)
    if event.event_type == input::EV_CHAR {
        // If Terminal window is open, handle the character
        if let Some(win_idx) = open_window {
            if win_idx == 1 && event.code > 0 && event.code < 128 {
                handle_terminal_char(event.code as u8);
                return None;
            }
        }
        return None;
    }

    // Handle mouse button events and touch events
    if event.event_type == input::EV_KEY {
        match event.code {
            BTN_LEFT | BTN_RIGHT | BTN_MIDDLE | BTN_TOUCH => {
                let pressed = event.value == 1;
                set_mouse_button(event.code, pressed);

                // On left mouse button or touch press
                if (event.code == BTN_LEFT || event.code == BTN_TOUCH) && pressed {
                    let (x, y) = get_cursor_pos();

                    // Update debug info for touch tracking
                    update_touch_debug(x, y);

                    // If child window is open, check for close button click or Terminal send button
                    if let Some(hit) = compositor::hit_window(x, y) {
                        if let Some(grect) = compositor::get_geom(hit) {
                            set_child_geom(rect_to_geom(grect));
                            compositor::raise_or_open(hit, grect);
                            unsafe {
                                MAIN_SCREEN_OPEN_WINDOW = Some(hit);
                            }
                            let g = child_geom();
                            let close_btn_x = g.x + 12;
                            let close_btn_y = g.y + 10;
                            let dx = x - close_btn_x;
                            let dy = y - close_btn_y;
                            if dx * dx + dy * dy < 12 * 12 {
                                unsafe {
                                    TERMINAL_INPUT_LEN = 0;
                                    term_clear();
                                }
                                restore_window_backing();
                                return None;
                            }
                            if y >= g.y && y < g.y + 32 && x >= g.x + 70 && x < g.x + g.w as i32 {
                                unsafe {
                                    WINDOW_DRAG = true;
                                    DRAG_OFF_X = x - g.x;
                                    DRAG_OFF_Y = y - g.y;
                                }
                                return None;
                            }
                            if hit == 1 && hit_test_terminal_send_button(x, y) {
                                if unsafe { TERMINAL_COMMAND_RUNNING } {
                                    request_cancel();
                                } else {
                                    terminal_execute_command();
                                }
                                return None;
                            }
                            if hit == 2 && hit_settings_widgets(x, y) {
                                return None;
                            }
                        }
                    } else if let Some(button_idx) = hit_test_main_screen_button(x, y) {
                        if button_idx == 1 {
                            unsafe {
                                TERMINAL_INPUT_LEN = 0;
                            }
                            term_clear();
                        }
                        unsafe {
                            MAIN_SCREEN_SELECTED_BUTTON = button_idx;
                            WINDOW_DRAG = false;
                        }
                        set_child_geom(default_geom_for(button_idx));
                        draw_child_window(button_idx);
                        return Some(button_idx);
                    }
                } else if event.code == BTN_LEFT || event.code == BTN_TOUCH {
                    unsafe {
                        WINDOW_DRAG = false;
                    }
                }
                return None;
            }
            _ => {}
        }
    }

    // Handle keyboard events
    if !event.is_key_press() {
        return None;
    }

    // If child window is open
    if let Some(win_idx) = open_window {
        use crate::input::KEY_ESC;

        // ESC handling: if command is running, cancel it; otherwise close the window
        if event.code == KEY_ESC {
            if unsafe { TERMINAL_COMMAND_RUNNING } {
                // Command is running - ESC cancels it
                request_cancel();
                return None;
            } else {
                // No command running - close the child window
                unsafe {
                    TERMINAL_INPUT_LEN = 0;
                    term_clear();
                }
                restore_window_backing();
                // Flush deferred to end of gpuid tick
                return None;
            }
        }

        // Terminal captures typing. Network/Settings fall through so LEFT/RIGHT/ENTER
        // can still open another overlapping window.
        if win_idx == 1 {
            if handle_terminal_input(event.code, event.value) {
                return None;
            }
            let ch = key_code_to_ascii(event.code);
            if ch != 0 {
                handle_terminal_char(ch);
                return None;
            }
            return None;
        }
    }

    match event.code {
        KEY_LEFT => {
            // Navigate to previous button
            unsafe {
                if MAIN_SCREEN_SELECTED_BUTTON > 0 {
                    MAIN_SCREEN_SELECTED_BUTTON -= 1;
                    update_main_screen_buttons(MAIN_SCREEN_SELECTED_BUTTON);
                }
            }
            None
        }
        KEY_RIGHT => {
            // Navigate to next button (2 buttons: 0 and 1)
            unsafe {
                if MAIN_SCREEN_SELECTED_BUTTON < 2 {
                    MAIN_SCREEN_SELECTED_BUTTON += 1;
                    update_main_screen_buttons(MAIN_SCREEN_SELECTED_BUTTON);
                }
            }
            None
        }
        KEY_UP | KEY_DOWN => {
            // No vertical navigation between buttons
            None
        }
        KEY_ENTER => {
            // Open child window for selected button
            let button_idx = unsafe { MAIN_SCREEN_SELECTED_BUTTON };
            // Clear terminal state when opening terminal
            if button_idx == 1 {
                unsafe {
                    TERMINAL_INPUT_LEN = 0;
                }
                term_clear();
            }
            unsafe {
                MAIN_SCREEN_OPEN_WINDOW = Some(button_idx);
                WINDOW_DRAG = false;
            }
            set_child_geom(default_geom_for(button_idx));
            draw_child_window(button_idx);
            // Flush deferred to end of gpuid tick
            Some(button_idx)
        }
        _ => None,
    }
}

/// Convert a key code to ASCII character (basic US keyboard layout)
fn key_code_to_ascii(code: u16) -> u8 {
    // Linux input key codes (from linux/input-event-codes.h)
    // Numbers: KEY_1=2, KEY_2=3, ... KEY_0=11
    // Letters: KEY_Q=16, KEY_W=17, ...
    match code {
        // Number row
        2 => b'1',
        3 => b'2',
        4 => b'3',
        5 => b'4',
        6 => b'5',
        7 => b'6',
        8 => b'7',
        9 => b'8',
        10 => b'9',
        11 => b'0',
        12 => b'-',
        13 => b'=',

        // First letter row: QWERTYUIOP
        16 => b'q',
        17 => b'w',
        18 => b'e',
        19 => b'r',
        20 => b't',
        21 => b'y',
        22 => b'u',
        23 => b'i',
        24 => b'o',
        25 => b'p',
        26 => b'[',
        27 => b']',

        // Second letter row: ASDFGHJKL
        30 => b'a',
        31 => b's',
        32 => b'd',
        33 => b'f',
        34 => b'g',
        35 => b'h',
        36 => b'j',
        37 => b'k',
        38 => b'l',
        39 => b';',
        40 => b'\'',

        // Third letter row: ZXCVBNM
        44 => b'z',
        45 => b'x',
        46 => b'c',
        47 => b'v',
        48 => b'b',
        49 => b'n',
        50 => b'm',
        51 => b',',
        52 => b'.',
        53 => b'/',

        // Space
        57 => b' ',

        // Punctuation
        41 => b'`',
        43 => b'\\',

        _ => 0,
    }
}

// Helper function to format CPU string
fn format_cpu_str(count: usize, buf: &mut [u8; 32]) -> &str {
    struct BufWriter<'a> {
        buf: &'a mut [u8],
        pos: usize,
    }
    impl<'a> Write for BufWriter<'a> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let bytes = s.as_bytes();
            let remaining = self.buf.len() - self.pos;
            let to_copy = bytes.len().min(remaining);
            self.buf[self.pos..self.pos + to_copy].copy_from_slice(&bytes[..to_copy]);
            self.pos += to_copy;
            Ok(())
        }
    }

    let mut writer = BufWriter { buf: buf, pos: 0 };
    let _ = write!(
        writer,
        "CPU:          {} Core{} @ RISC-V",
        count,
        if count == 1 { "" } else { "s" }
    );
    let len = writer.pos;
    core::str::from_utf8(&buf[..len]).unwrap_or("CPU: Unknown")
}

// Helper function to format memory string (used / total KB)
fn format_memory_str(used_kb: usize, total_kb: usize, buf: &mut [u8; 48]) -> &str {
    struct BufWriter<'a> {
        buf: &'a mut [u8],
        pos: usize,
    }
    impl<'a> Write for BufWriter<'a> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let bytes = s.as_bytes();
            let remaining = self.buf.len() - self.pos;
            let to_copy = bytes.len().min(remaining);
            self.buf[self.pos..self.pos + to_copy].copy_from_slice(&bytes[..to_copy]);
            self.pos += to_copy;
            Ok(())
        }
    }

    let mut writer = BufWriter { buf: buf, pos: 0 };
    let _ = write!(writer, "Memory:       {}/{} KB", used_kb, total_kb);
    let len = writer.pos;
    core::str::from_utf8(&buf[..len]).unwrap_or("Memory: Unknown")
}

// Helper function to format disk string (used / total KB)
fn format_disk_str(used_kb: usize, total_kb: usize, buf: &mut [u8; 48]) -> &str {
    struct BufWriter<'a> {
        buf: &'a mut [u8],
        pos: usize,
    }
    impl<'a> Write for BufWriter<'a> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let bytes = s.as_bytes();
            let remaining = self.buf.len() - self.pos;
            let to_copy = bytes.len().min(remaining);
            self.buf[self.pos..self.pos + to_copy].copy_from_slice(&bytes[..to_copy]);
            self.pos += to_copy;
            Ok(())
        }
    }

    let mut writer = BufWriter { buf: buf, pos: 0 };
    let _ = write!(writer, "Storage:      {}/{} KB", used_kb, total_kb);
    let len = writer.pos;
    core::str::from_utf8(&buf[..len]).unwrap_or("Storage: Unknown")
}

// Helper function to format network string with IP
fn format_network_str<'a>(available: bool, ip: &[u8; 4], buf: &'a mut [u8; 48]) -> &'a str {
    struct BufWriter<'a> {
        buf: &'a mut [u8],
        pos: usize,
    }
    impl<'a> Write for BufWriter<'a> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let bytes = s.as_bytes();
            let remaining = self.buf.len() - self.pos;
            let to_copy = bytes.len().min(remaining);
            self.buf[self.pos..self.pos + to_copy].copy_from_slice(&bytes[..to_copy]);
            self.pos += to_copy;
            Ok(())
        }
    }

    let mut writer = BufWriter { buf: buf, pos: 0 };
    if available {
        let _ = write!(
            writer,
            "Network:      {}.{}.{}.{}",
            ip[0], ip[1], ip[2], ip[3]
        );
    } else {
        let _ = write!(writer, "Network:      Not connected");
    }
    let len = writer.pos;
    core::str::from_utf8(&buf[..len]).unwrap_or("Network: Unknown")
}
