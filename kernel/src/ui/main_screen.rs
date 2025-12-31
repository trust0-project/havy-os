//! MainScreen Screen
//!
//! Interactive main_screen screen showing system information,
//! hardware stats, and quick action buttons.

use alloc::format;
use core::fmt::Write;

use embedded_graphics::{
    mono_font::{ascii::{FONT_7X14, FONT_9X15_BOLD}, MonoTextStyle},
    pixelcolor::Rgb888,
    prelude::*,
    primitives::{Circle, Line, PrimitiveStyle, Rectangle, RoundedRectangle},
    text::Text,
};

use crate::platform::d1_display;
use crate::platform::d1_touch::{self, ABS_X, ABS_Y, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, BTN_TOUCH,
    EV_ABS, KEY_DOWN, KEY_ENTER, KEY_LEFT, KEY_RIGHT, KEY_UP};

use super::cursor::{
    get_cursor_pos, invalidate_cursor_backup, restore_cursor_backup, set_cursor_pos, set_mouse_button,
};
use super::manager::with_ui;
use super::widgets::Window;
use super::{draw_image, LOGO_SMALL, LOGO_SMALL_SIZE};

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

/// Flag to track if static content has been drawn (labels, lines, etc.)
/// When true, only dynamic content (buttons, stats) needs updating
static mut MAIN_SCREEN_STATIC_DRAWN: bool = false;

/// Last selected button - used to only redraw changed buttons
static mut MAIN_SCREEN_LAST_SELECTED: Option<usize> = None;

/// Currently open child window (None = main screen, Some(index) = button window open)
static mut MAIN_SCREEN_OPEN_WINDOW: Option<usize> = None;

// Window backing store - saves region behind child window for instant restore on close
// Terminal window: 700x500 at (162, 134), shadow: +8 pixels, total ~708x508
const WINDOW_BACKING_W: usize = 710;
const WINDOW_BACKING_H: usize = 510;
const WINDOW_BACKING_X: u32 = 160;
const WINDOW_BACKING_Y: u32 = 132;
static mut WINDOW_BACKING_STORE: [u32; WINDOW_BACKING_W * WINDOW_BACKING_H] = [0; WINDOW_BACKING_W * WINDOW_BACKING_H];
static mut WINDOW_BACKING_VALID: bool = false;

/// Last time hardware stats were updated (in ms)
static mut MAIN_SCREEN_LAST_HW_UPDATE: i64 = 0;

/// Hardware stats update interval in ms
const MAIN_SCREEN_HW_UPDATE_INTERVAL: i64 = 2000; // Update every 2 seconds

// Terminal window state
const TERMINAL_INPUT_MAX: usize = 256;
const TERMINAL_OUTPUT_MAX: usize = 2048;
static mut TERMINAL_INPUT_BUFFER: [u8; TERMINAL_INPUT_MAX] = [0; TERMINAL_INPUT_MAX];
static mut TERMINAL_INPUT_LEN: usize = 0;
static mut TERMINAL_OUTPUT_BUFFER: [u8; TERMINAL_OUTPUT_MAX] = [0; TERMINAL_OUTPUT_MAX];
static mut TERMINAL_OUTPUT_LEN: usize = 0;
/// Whether a command is currently executing (shows Cancel button instead of Run)
static mut TERMINAL_COMMAND_RUNNING: bool = false;
/// Whether a cancel has been requested (checked by should_cancel syscall)
static mut TERMINAL_CANCEL_REQUESTED: bool = false;

/// Check if a point is inside a main_screen button, returns button index if hit
pub fn hit_test_main_screen_button(x: i32, y: i32) -> Option<usize> {
    // Button positions (must match draw_main_screen_content)
    // Network and Terminal buttons, aligned left (adjusted for 1024x768)
    let buttons = [
        (30, 500, 110, 32),   // Network (aligned with left column)
        (150, 500, 110, 32),  // Terminal
    ];
    
    for (i, (bx, by, bw, bh)) in buttons.iter().enumerate() {
        if x >= *bx && x < bx + (*bw as i32) && y >= *by && y < by + (*bh as i32) {
            return Some(i);
        }
    }
    None
}

/// Save the region behind the child window before opening it
fn save_window_backing() {
    d1_display::with_gpu(|gpu| {
        unsafe {
            // FAST: Use read_rect_fast to copy entire rows at once (410 copies vs 210K reads)
            gpu.read_rect_fast(
                WINDOW_BACKING_X, WINDOW_BACKING_Y,
                WINDOW_BACKING_W, WINDOW_BACKING_H,
                &mut *core::ptr::addr_of_mut!(WINDOW_BACKING_STORE)
            );
            WINDOW_BACKING_VALID = true;
        }
    });
}

/// Restore the region behind the child window (for instant close)
fn restore_window_backing() {
    if !unsafe { WINDOW_BACKING_VALID } {
        return;
    }
    d1_display::with_gpu(|gpu| {
        unsafe {
            // FAST: Use blit_rect to copy entire rows at once (410 copies vs 210K pixels)
            gpu.blit_rect(
                WINDOW_BACKING_X, WINDOW_BACKING_Y,
                WINDOW_BACKING_W, WINDOW_BACKING_H,
                &WINDOW_BACKING_STORE
            );
            WINDOW_BACKING_VALID = false;
        }
    });
}

/// Get button name for child window title
fn get_button_name(index: usize) -> &'static str {
    match index {
        0 => "Network",
        1 => "Terminal",
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
/// This is much more efficient than redrawing the entire screen
pub fn update_main_screen_hardware_stats() {
    // Don't update hardware stats if a child window is open (it would draw over the window)
    if unsafe { MAIN_SCREEN_OPEN_WINDOW.is_some() } {
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
    
    // Get fresh hardware info
    let hw = get_hardware_info();
    
    // Only redraw the hardware stats area
    d1_display::with_gpu(|gpu| {
        let col1_x = 30;
        let text_style = MonoTextStyle::new(&FONT_7X14, Rgb888::new(200, 200, 210));
        
        // Clear just the dynamic hardware stats (NOT the static Display line at y=240)
        // Lines to clear: CPU (y=210), Memory (y=225), Disk (y=255), Network (y=270)
        // FONT_7X14 means text extends ~10px above baseline
        let clear_color = Rgb888::new(28, 28, 38); // Window background color
        
        // Clear top section: CPU (y=210) and Memory (y=225) only
        // Stop at y=228 to avoid Display line (baseline y=240, text starts ~y=230)
        let _ = Rectangle::new(Point::new(col1_x, 200), Size::new(300, 28))
            .into_styled(PrimitiveStyle::with_fill(clear_color))
            .draw(gpu);
        // Clear bottom section: Disk (y=255) and Network (y=270) only
        // Start at y=245 to avoid Display line
        let _ = Rectangle::new(Point::new(col1_x, 245), Size::new(300, 37))
            .into_styled(PrimitiveStyle::with_fill(clear_color))
            .draw(gpu);
        
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
        let status_bar_bg = Rgb888::new(25, 25, 35);
        // Clear the time display area on the right side
        let _ = Rectangle::new(Point::new(400, 742), Size::new(150, 26))
            .into_styled(PrimitiveStyle::with_fill(status_bar_bg))
            .draw(gpu);
        
        // Try to get host date/time from RTC, fall back to uptime
        let time_str = if let Some(dt) = crate::device::rtc::get_datetime() {
            // Display as: "Dec 16 15:30"
            let month_name = match dt.month {
                1 => "Jan", 2 => "Feb", 3 => "Mar", 4 => "Apr",
                5 => "May", 6 => "Jun", 7 => "Jul", 8 => "Aug",
                9 => "Sep", 10 => "Oct", 11 => "Nov", 12 => "Dec",
                _ => "???"
            };
            format!("{} {:02} {:02}:{:02}", month_name, dt.day, dt.hour, dt.minute)
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
    // Hide cursor first (restore pixels) to prevent ghost when redrawing over it
    restore_cursor_backup();
    
    d1_display::with_gpu(|gpu| {
        // Button definitions - Network and Terminal, left aligned (adjusted for 1024x768)
        let buttons = [
            ("Network", 30),
            ("Terminal", 150),
        ];
        
        // Clear the buttons area (adjusted for 1024x768: wider for 2 buttons)
        gpu.fill_rect(28, 498, 240, 38, 28, 28, 38);
        
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
                Rectangle::new(Point::new(*x, 500), Size::new(110, 32)),
                Size::new(4, 4),
            )
            .into_styled(PrimitiveStyle::with_fill(bg_color))
            .draw(gpu);
            
            // Button border
            let _ = RoundedRectangle::with_equal_corners(
                Rectangle::new(Point::new(*x, 500), Size::new(110, 32)),
                Size::new(4, 4),
            )
            .into_styled(PrimitiveStyle::with_stroke(border_color, if is_selected { 2 } else { 1 }))
            .draw(gpu);
            
            let text_color = if is_selected {
                Rgb888::WHITE
            } else {
                Rgb888::new(200, 200, 210)
            };
            let btn_text_style = MonoTextStyle::new(&FONT_7X14, text_color);
            let _ = Text::new(label, Point::new(*x + 8, 520), btn_text_style).draw(gpu);
        }
    });
    
    // Invalidate backup and force cursor redraw with fresh background
    invalidate_cursor_backup();
    // Flush deferred to end of gpuid tick
}


/// Setup a main_screen screen showing embedded_graphics capabilities with dynamic hardware info
pub fn setup_main_screen() {
    // Get hardware info
    let hw = get_hardware_info();
    
    // Reset selected button and update time
    unsafe { 
        MAIN_SCREEN_SELECTED_BUTTON = 0;
        MAIN_SCREEN_STATIC_DRAWN = false;  // Force full redraw on setup
        MAIN_SCREEN_LAST_SELECTED = None;
        MAIN_SCREEN_LAST_HW_UPDATE = crate::get_time_ms();
    }
    
    // Enable main_screen mode to prevent UI manager from overwriting our direct GPU draws
    with_ui(|ui_mgr| {
        ui_mgr.clear();
        ui_mgr.set_main_screen_mode(true);
    });
    
    draw_main_screen_content(&hw, unsafe { MAIN_SCREEN_SELECTED_BUTTON });
}

/// Draw a child window (opened by clicking a button)
/// Draws ONLY the child window on top of existing content for maximum speed
fn draw_child_window(button_index: usize) {
    // PERFORMANCE: Save region behind window for instant restore on close
    save_window_backing();
    
    match button_index {
        0 => draw_network_window(),
        1 => draw_terminal_window(),
        _ => {}
    }
}

/// Draw the Network Statistics window content
fn draw_network_window() {
    // Pre-compute network info BEFORE entering GPU closure (avoid locks inside)
    // Use is_ip_assigned() which checks for valid IP without needing locks
    let is_online = crate::net::is_ip_assigned();
    
    let ip = crate::net::get_my_ip();
    let ip_octets = ip.octets();
    let gateway = crate::net::GATEWAY.octets();
    let dns = crate::net::DNS_SERVER.octets();
    let prefix = crate::net::PREFIX_LEN;
    
    // Pre-format strings to avoid allocations in GPU closure
    let ip_str = format!("{}.{}.{}.{}/{}", 
        ip_octets[0], ip_octets[1], ip_octets[2], ip_octets[3], prefix);
    let gw_str = format!("{}.{}.{}.{}", 
        gateway[0], gateway[1], gateway[2], gateway[3]);
    let dns_str = format!("{}.{}.{}.{}", 
        dns[0], dns[1], dns[2], dns[3]);
    
    d1_display::with_gpu(|gpu| {
        // Shadow + window background in one batch (centered for 1024x768)
        gpu.fill_rect(268, 188, 500, 400, 5, 5, 10);  // Shadow
        gpu.fill_rect(260, 180, 500, 400, 28, 28, 38);  // Window bg
        gpu.fill_rect(260, 180, 500, 32, 40, 40, 55);  // Title bar
        
        // Border (stroke only)
        let _ = Rectangle::new(Point::new(260, 180), Size::new(500, 400))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);
        
        // Traffic light buttons
        let _ = Circle::new(Point::new(272, 190), 12)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(220, 80, 80)))
            .draw(gpu);
        let _ = Circle::new(Point::new(292, 190), 12)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(230, 180, 80)))
            .draw(gpu);
        let _ = Circle::new(Point::new(312, 190), 12)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(80, 200, 120)))
            .draw(gpu);
        
        // Title + logo
        let title_style = MonoTextStyle::new(&FONT_9X15_BOLD, Rgb888::WHITE);
        let _ = Text::new("Network Statistics", Point::new(430, 202), title_style).draw(gpu);
        // Small logo aligned to the right of the header (window is at x=260, width=500)
        draw_image(gpu, 260 + 500 - LOGO_SMALL_SIZE - 8, 184, LOGO_SMALL_SIZE, LOGO_SMALL_SIZE, LOGO_SMALL);
        
        // Content styles
        let label_style = MonoTextStyle::new(&FONT_7X14, Rgb888::new(230, 180, 80));
        let value_style = MonoTextStyle::new(&FONT_7X14, Rgb888::WHITE);
        let hint_style = MonoTextStyle::new(&FONT_7X14, Rgb888::new(100, 100, 120));
        
        let x = 280;
        let mut y = 240;
        
        // Device section - use static strings
        let _ = Text::new("Device:", Point::new(x, y), label_style).draw(gpu);
        y += 16;
        let _ = Text::new("Type:    VirtIO Network Device", Point::new(x + 10, y), value_style).draw(gpu);
        y += 14;
        let _ = Text::new("Address: 0x10001000", Point::new(x + 10, y), value_style).draw(gpu);
        y += 14;
        
        // Status
        if is_online {
            let _ = Text::new("Status:  * ONLINE", Point::new(x + 10, y), 
                MonoTextStyle::new(&FONT_7X14, Rgb888::new(80, 200, 120))).draw(gpu);
        } else {
            let _ = Text::new("Status:  X OFFLINE", Point::new(x + 10, y), 
                MonoTextStyle::new(&FONT_7X14, Rgb888::new(220, 80, 80))).draw(gpu);
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
        let _ = Text::new("smoltcp - Lightweight TCP/IP", Point::new(x + 10, y), value_style).draw(gpu);
        y += 14;
        let _ = Text::new("ICMP, UDP, TCP, ARP", Point::new(x + 10, y), value_style).draw(gpu);
        
        // Close hint
        let _ = Text::new("Press ESC or click red button to close", Point::new(330, 560), hint_style).draw(gpu);
    });
}

/// Draw the Terminal window content
fn draw_terminal_window() {
    // Window dimensions: 700x500, centered on 1024x768
    // Position: (162, 134) - use u32 for fill_rect, cast to i32 for Points
    const WIN_X: u32 = 162;
    const WIN_Y: u32 = 134;
    const WIN_W: u32 = 700;
    const WIN_H: u32 = 500;
    
    d1_display::with_gpu(|gpu| {
        // Shadow + window background in one batch
        gpu.fill_rect(WIN_X + 8, WIN_Y + 8, WIN_W, WIN_H, 5, 5, 10);  // Shadow
        gpu.fill_rect(WIN_X, WIN_Y, WIN_W, WIN_H, 28, 28, 38);  // Window bg
        gpu.fill_rect(WIN_X, WIN_Y, WIN_W, 32, 40, 40, 55);  // Title bar
        
        // Border (stroke only)
        let _ = Rectangle::new(Point::new(WIN_X as i32, WIN_Y as i32), Size::new(WIN_W, WIN_H))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);
        
        // Traffic light buttons (cast to i32 for Point)
        let _ = Circle::new(Point::new(WIN_X as i32 + 12, WIN_Y as i32 + 10), 12)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(220, 80, 80)))
            .draw(gpu);
        let _ = Circle::new(Point::new(WIN_X as i32 + 32, WIN_Y as i32 + 10), 12)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(230, 180, 80)))
            .draw(gpu);
        let _ = Circle::new(Point::new(WIN_X as i32 + 52, WIN_Y as i32 + 10), 12)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(80, 200, 120)))
            .draw(gpu);
        
        // Title + logo (centered)
        let title_style = MonoTextStyle::new(&FONT_9X15_BOLD, Rgb888::WHITE);
        let _ = Text::new("Terminal", Point::new(WIN_X as i32 + 310, WIN_Y as i32 + 22), title_style).draw(gpu);
        draw_image(gpu, WIN_X + WIN_W - LOGO_SMALL_SIZE as u32 - 8, WIN_Y + 4, LOGO_SMALL_SIZE, LOGO_SMALL_SIZE, LOGO_SMALL);
        
        let hint_style = MonoTextStyle::new(&FONT_7X14, Rgb888::new(100, 100, 120));
        let value_style = MonoTextStyle::new(&FONT_7X14, Rgb888::WHITE);
        
        // Content area starts at WIN_Y + 40
        let content_y = WIN_Y + 45;
        let content_x = WIN_X + 15;
        
        // Command label
        let _ = Text::new("Command:", Point::new(content_x as i32, content_y as i32), hint_style).draw(gpu);
        
        // Input field background (dark) - wider: 580px
        let input_y = content_y + 10;
        let _ = RoundedRectangle::with_equal_corners(
            Rectangle::new(Point::new(content_x as i32, input_y as i32), Size::new(580, 28)),
            Size::new(4, 4),
        )
        .into_styled(PrimitiveStyle::with_fill(Rgb888::new(18, 18, 28)))
        .draw(gpu);
        
        // Input field border
        let _ = RoundedRectangle::with_equal_corners(
            Rectangle::new(Point::new(content_x as i32, input_y as i32), Size::new(580, 28)),
            Size::new(4, 4),
        )
        .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(80, 80, 100), 1))
        .draw(gpu);
        
        // Draw current input text
        let input_text = unsafe {
            core::str::from_utf8(&TERMINAL_INPUT_BUFFER[..TERMINAL_INPUT_LEN]).unwrap_or("")
        };
        let _ = Text::new(input_text, Point::new(content_x as i32 + 7, input_y as i32 + 19), value_style).draw(gpu);
        
        // Draw cursor (always visible, simple block cursor)
        let cursor_x = content_x as i32 + 7 + (unsafe { TERMINAL_INPUT_LEN } as i32 * 7);
        if cursor_x < content_x as i32 + 570 {  // Don't draw cursor past input field
            let _ = Rectangle::new(Point::new(cursor_x, input_y as i32 + 5), Size::new(2, 16))
                .into_styled(PrimitiveStyle::with_fill(Rgb888::new(200, 200, 220)))
                .draw(gpu);
        }
        
        // Run/Cancel button (right of input field) - red Cancel when running, blue Run otherwise
        let btn_x = content_x as i32 + 590;
        let is_running = unsafe { TERMINAL_COMMAND_RUNNING };
        let (btn_color, btn_text) = if is_running {
            (Rgb888::new(200, 80, 80), "Cancel")  // Red cancel button
        } else {
            (Rgb888::new(80, 140, 200), "Run")    // Blue run button
        };
        let _ = RoundedRectangle::with_equal_corners(
            Rectangle::new(Point::new(btn_x, input_y as i32), Size::new(80, 28)),
            Size::new(4, 4),
        )
        .into_styled(PrimitiveStyle::with_fill(btn_color))
        .draw(gpu);
        let text_x = if is_running { btn_x + 14 } else { btn_x + 25 }; // Center text differently
        let _ = Text::new(btn_text, Point::new(text_x, input_y as i32 + 19), 
            MonoTextStyle::new(&FONT_7X14, Rgb888::WHITE)).draw(gpu);
        
        // CWD label (shows current working directory)
        let output_label_y = input_y + 40;
        let cwd = crate::utils::cwd_get();
        let cwd_label = alloc::format!("{}$", cwd);
        let _ = Text::new(&cwd_label, Point::new(content_x as i32, output_label_y as i32), hint_style).draw(gpu);
        
        // Output area background - larger: 670x340
        let output_y = output_label_y + 10;
        let _ = RoundedRectangle::with_equal_corners(
            Rectangle::new(Point::new(content_x as i32, output_y as i32), Size::new(670, 340)),
            Size::new(4, 4),
        )
        .into_styled(PrimitiveStyle::with_fill(Rgb888::new(10, 10, 15)))
        .draw(gpu);
        
        // Output area border
        let _ = RoundedRectangle::with_equal_corners(
            Rectangle::new(Point::new(content_x as i32, output_y as i32), Size::new(670, 340)),
            Size::new(4, 4),
        )
        .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
        .draw(gpu);
        
        // Draw output text (multi-line) - now fits ~22 lines
        let output_text = unsafe {
            core::str::from_utf8(&TERMINAL_OUTPUT_BUFFER[..TERMINAL_OUTPUT_LEN]).unwrap_or("")
        };
        
        let output_style = MonoTextStyle::new(&FONT_7X14, Rgb888::new(80, 200, 120));
        let mut y = output_y as i32 + 15;
        let max_chars_per_line = 92;  // 670px / 7px per char ≈ 95, leave margin
        let mut line_count = 0;
        
        for line in output_text.lines() {
            if line_count >= 22 {
                break;
            }
            // Truncate long lines
            let display_line = if line.len() > max_chars_per_line {
                &line[..max_chars_per_line]
            } else {
                line
            };
            let _ = Text::new(display_line, Point::new(content_x as i32 + 7, y), output_style).draw(gpu);
            y += 15;
            line_count += 1;
        }
        
        // Close hint at bottom
        let _ = Text::new("Press ESC to close, Enter to run command", Point::new(WIN_X as i32 + 200, WIN_Y as i32 + WIN_H as i32 - 15), hint_style).draw(gpu);
    });
}

/// Fast partial redraw of ONLY the input field (for responsive typing)
/// This is much faster than redrawing the entire terminal window
fn draw_terminal_input_only() {
    // Must match coordinates from draw_terminal_window
    const WIN_X: u32 = 162;
    const WIN_Y: u32 = 134;
    const CONTENT_X: u32 = WIN_X + 15;
    const INPUT_Y: u32 = WIN_Y + 55;
    
    d1_display::with_gpu(|gpu| {
        let value_style = MonoTextStyle::new(&FONT_7X14, Rgb888::WHITE);
        
        // Clear only the input field interior (not the border)
        // Input field is at (CONTENT_X, INPUT_Y) with size (580, 28)
        gpu.fill_rect(CONTENT_X + 1, INPUT_Y + 1, 578, 26, 18, 18, 28);
        
        // Draw current input text
        let input_text = unsafe {
            core::str::from_utf8(&TERMINAL_INPUT_BUFFER[..TERMINAL_INPUT_LEN]).unwrap_or("")
        };
        let _ = Text::new(input_text, Point::new(CONTENT_X as i32 + 7, INPUT_Y as i32 + 19), value_style).draw(gpu);
        
        // Draw cursor (always visible, simple block cursor)
        let cursor_x = CONTENT_X as i32 + 7 + (unsafe { TERMINAL_INPUT_LEN } as i32 * 7);
        if cursor_x < CONTENT_X as i32 + 570 {  // Don't draw cursor past input field
            let _ = Rectangle::new(Point::new(cursor_x, INPUT_Y as i32 + 5), Size::new(2, 16))
                .into_styled(PrimitiveStyle::with_fill(Rgb888::new(200, 200, 220)))
                .draw(gpu);
        }
    });
}

/// Fast partial redraw of ONLY the output area (for responsive command output)
/// This is much faster than redrawing the entire terminal window
fn draw_terminal_output_only() {
    // Must match coordinates from draw_terminal_window
    const WIN_X: u32 = 162;
    const WIN_Y: u32 = 134;
    const CONTENT_X: u32 = WIN_X + 15;
    // OUTPUT_Y = input_y + 40 + 10 = (WIN_Y + 55) + 40 + 10 = WIN_Y + 105
    const OUTPUT_Y: u32 = WIN_Y + 105;
    
    d1_display::with_gpu(|gpu| {
        let output_style = MonoTextStyle::new(&FONT_7X14, Rgb888::new(80, 200, 120));
        
        // Clear only the output area interior (not the border)
        // Output area is at (CONTENT_X, OUTPUT_Y) with size (670, 340)
        gpu.fill_rect(CONTENT_X + 1, OUTPUT_Y + 1, 668, 338, 10, 10, 15);
        
        // Draw output text (multi-line)
        let output_text = unsafe {
            core::str::from_utf8(&TERMINAL_OUTPUT_BUFFER[..TERMINAL_OUTPUT_LEN]).unwrap_or("")
        };
        
        let max_chars_per_line = 92;  // 670px / 7px per char ≈ 95, leave margin
        let mut y = OUTPUT_Y as i32 + 15;
        let mut line_count = 0;
        
        for line in output_text.lines() {
            if line_count >= 22 {
                break;
            }
            let display_line = if line.len() > max_chars_per_line {
                &line[..max_chars_per_line]
            } else {
                line
            };
            let _ = Text::new(display_line, Point::new(CONTENT_X as i32 + 7, y), output_style).draw(gpu);
            y += 15;
            line_count += 1;
        }
    });
}

/// Fast partial redraw of ONLY the Run/Cancel button (for responsive button state changes)
/// This is much faster than redrawing the entire terminal window
fn draw_terminal_button_only() {
    // Must match coordinates from draw_terminal_window
    const WIN_X: u32 = 162;
    const WIN_Y: u32 = 134;
    const CONTENT_X: u32 = WIN_X + 15;
    const INPUT_Y: u32 = WIN_Y + 55;
    const BTN_X: i32 = CONTENT_X as i32 + 590;
    
    d1_display::with_gpu(|gpu| {
        let is_running = unsafe { TERMINAL_COMMAND_RUNNING };
        let (btn_color, btn_text) = if is_running {
            (Rgb888::new(200, 80, 80), "Cancel")  // Red cancel button
        } else {
            (Rgb888::new(80, 140, 200), "Run")    // Blue run button
        };
        
        // Clear button area and redraw
        let _ = RoundedRectangle::with_equal_corners(
            Rectangle::new(Point::new(BTN_X, INPUT_Y as i32), Size::new(80, 28)),
            Size::new(4, 4),
        )
        .into_styled(PrimitiveStyle::with_fill(btn_color))
        .draw(gpu);
        
        let text_x = if is_running { BTN_X + 14 } else { BTN_X + 25 };
        let _ = Text::new(btn_text, Point::new(text_x, INPUT_Y as i32 + 19), 
            MonoTextStyle::new(&FONT_7X14, Rgb888::WHITE)).draw(gpu);
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
    unsafe { TERMINAL_COMMAND_RUNNING = true; }
    draw_terminal_button_only();
    d1_display::flush();
    
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
    
    // Update terminal output buffer with result
    unsafe {
        TERMINAL_OUTPUT_LEN = 0;
        let mut i = 0;
        while i < output.len() && TERMINAL_OUTPUT_LEN < TERMINAL_OUTPUT_MAX {
            if output[i] == 0x1b && i + 1 < output.len() && output[i + 1] == b'[' {
                i += 2;
                while i < output.len() && !output[i].is_ascii_alphabetic() {
                    i += 1;
                }
                if i < output.len() {
                    i += 1;
                }
                continue;
            }
            TERMINAL_OUTPUT_BUFFER[TERMINAL_OUTPUT_LEN] = output[i];
            TERMINAL_OUTPUT_LEN += 1;
            i += 1;
        }
        TERMINAL_COMMAND_RUNNING = false;
    }
    
    draw_terminal_output_only();
    draw_terminal_button_only();
    d1_display::flush();
}

/// Check for GUI command completion and update terminal output
/// Called from gpuid tick to poll for results
pub fn check_gui_command_completion() {
    use crate::device::uart::{write_str, write_line};
    
    // Only check if a command is running
    if !unsafe { TERMINAL_COMMAND_RUNNING } {
        return;
    }
    
    
    // Poll for result
    if let Some(result) = crate::services::gui_cmd::poll_result() {
        let mut buf = [0u8; 8];
        
        // Command completed - update output display
        unsafe {
            TERMINAL_OUTPUT_LEN = 0;
            let mut i = 0;
            while i < result.output.len() && TERMINAL_OUTPUT_LEN < TERMINAL_OUTPUT_MAX {
                // Skip ANSI escape sequences
                if result.output[i] == 0x1b && i + 1 < result.output.len() && result.output[i + 1] == b'[' {
                    // Find the end of the escape sequence (letter)
                    i += 2;
                    while i < result.output.len() && !result.output[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    if i < result.output.len() {
                        i += 1; // Skip the letter
                    }
                    continue;
                }
                TERMINAL_OUTPUT_BUFFER[TERMINAL_OUTPUT_LEN] = result.output[i];
                TERMINAL_OUTPUT_LEN += 1;
                i += 1;
            }
        }

        
        // Mark command as finished
        unsafe { TERMINAL_COMMAND_RUNNING = false; }
        
        // Update UI
        draw_terminal_output_only();
        draw_terminal_button_only();
        d1_display::flush();
    }
}

/// Refresh terminal output during WASM execution (called by terminal_refresh syscall)
/// Copies current OUTPUT_CAPTURE to TERMINAL_OUTPUT and redraws the window
pub fn refresh_terminal_output() {
    use crate::lock::utils::OUTPUT_CAPTURE;
    use crate::lock::state::output::OUTPUT_BUFFER_SIZE;
    
    // Only refresh if terminal window is open and command is running
    let window_open = unsafe { MAIN_SCREEN_OPEN_WINDOW };
    if window_open != Some(1) {  // Not terminal window
        return;
    }
    
    // Copy current output capture to terminal buffer
    {
        let cap = OUTPUT_CAPTURE.lock();
        let len = cap.len.min(OUTPUT_BUFFER_SIZE).min(TERMINAL_OUTPUT_MAX);
        
        unsafe {
            TERMINAL_OUTPUT_LEN = 0;  // Clear first
            // Copy with ANSI escape code filtering (simplified)
            let mut i = 0;
            while i < len && TERMINAL_OUTPUT_LEN < TERMINAL_OUTPUT_MAX {
                let output = &cap.buffer[..len];
                if output[i] == 0x1b && i + 1 < len && output[i + 1] == b'[' {
                    // Skip ANSI escape sequence
                    i += 2;
                    while i < len && !output[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    if i < len {
                        i += 1; // Skip the letter
                    }
                    continue;
                }
                TERMINAL_OUTPUT_BUFFER[TERMINAL_OUTPUT_LEN] = cap.buffer[i];
                TERMINAL_OUTPUT_LEN += 1;
                i += 1;
            }
        }
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
            TERMINAL_CANCEL_REQUESTED = false;  // Clear after reading
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
            if TERMINAL_OUTPUT_LEN + 3 < TERMINAL_OUTPUT_MAX {
                TERMINAL_OUTPUT_BUFFER[TERMINAL_OUTPUT_LEN] = b'^';
                TERMINAL_OUTPUT_BUFFER[TERMINAL_OUTPUT_LEN + 1] = b'C';
                TERMINAL_OUTPUT_BUFFER[TERMINAL_OUTPUT_LEN + 2] = b'\n';
                TERMINAL_OUTPUT_LEN += 3;
            }
            draw_terminal_window();
            // Flush deferred to end of gpuid tick
        }
    }
}

/// Clear cancellation flag (called at command start)
fn clear_cancel() {
    unsafe { TERMINAL_CANCEL_REQUESTED = false; }
}

/// Handle terminal window input (keyboard chars)
fn handle_terminal_input(key_code: u16, _key_value: i32) -> bool {
    use crate::platform::d1_touch::{KEY_BACKSPACE, KEY_ENTER};
    use crate::device::uart::{write_str, write_line};
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
        unsafe { TERMINAL_INPUT_LEN = 0; }  // Clear input
        request_cancel();
        return;
    }
    
    if ch >= 0x20 && ch < 0x7F {  // Printable ASCII
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
    // Send button position for 700x500 window:
    // WIN_X=162, content_x=177, btn_x=content_x+590=767, input_y=WIN_Y+55=189
    // Button size: 80x28
    x >= 767 && x < 847 && y >= 189 && y < 217
}



/// Redraw the main_screen screen with the given selected button index
/// Public entry point that calls inner function
fn draw_main_screen_content(hw: &HardwareInfo, selected_button: usize) {
    draw_main_screen_content_inner(hw, selected_button);
}

/// Inner function to draw main main_screen content (used by both normal draw and child window background)
fn draw_main_screen_content_inner(hw: &HardwareInfo, selected_button: usize) {
    // Check if a child window is open - we'll draw it on top after main content
    let open_window = unsafe { MAIN_SCREEN_OPEN_WINDOW };
    
    // Check if static content is already drawn - skip expensive operations if so
    let static_drawn = unsafe { MAIN_SCREEN_STATIC_DRAWN };
    
    d1_display::with_gpu(|gpu| {
        // Only clear and draw static content if not already cached
        if !static_drawn {
            // Clear to dark background (desktop) - EXPENSIVE, skip if already drawn
            let _ = gpu.clear(0x15, 0x15, 0x1E);
        }
        
        // === Draw Window using reusable Window component (no controls) ===
        let window = Window::new("HAVY OS - System Information", 10, 10, 1004, 710)
            .with_controls(false);  // Hide traffic light buttons on main window
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
        let _ = Text::new("Architecture: RISC-V RV64GC", Point::new(col1_x, 140), text_style).draw(gpu);
        let _ = Text::new("Platform:     Virtual Machine", Point::new(col1_x, 155), text_style).draw(gpu);
        
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
        
        let _ = Text::new("Display:      1024x768 VirtIO GPU", Point::new(col1_x, 240), text_style).draw(gpu);
        
        // Dynamic disk (used / total)
        let mut disk_buf = [0u8; 48];
        let disk_str = format_disk_str(hw.disk_used_kb, hw.disk_total_kb, &mut disk_buf);
        let _ = Text::new(disk_str, Point::new(col1_x, 255), text_style).draw(gpu);
        
        // Dynamic network with IP address
        let mut net_buf = [0u8; 48];
        let net_str = format_network_str(hw.network_available, &hw.ip_addr, &mut net_buf);
        let _ = Text::new(net_str, Point::new(col1_x, 270), text_style).draw(gpu);
        
        // === Right Column: Features ===
        let col2_x = 550;
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
            let _ = Line::new(Point::new(col2_x + 5, y - 1), Point::new(col2_x + 10, y - 8))
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
        let _ = Text::new("Use arrows to select, Enter to open", Point::new(col1_x, 488), hint_style).draw(gpu);
        
        // Mark static content as drawn so next time we skip the expensive clear
        unsafe { MAIN_SCREEN_STATIC_DRAWN = true; }
        
        // Network and Terminal buttons, left aligned (adjusted for 1024x768)
        let buttons = [
            ("Network", 30),
            ("Terminal", 150),
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
                Rectangle::new(Point::new(*x, 500), Size::new(110, 32)),
                Size::new(4, 4),
            )
            .into_styled(PrimitiveStyle::with_fill(bg_color))
            .draw(gpu);
            
            // Button border
            let _ = RoundedRectangle::with_equal_corners(
                Rectangle::new(Point::new(*x, 500), Size::new(110, 32)),
                Size::new(4, 4),
            )
            .into_styled(PrimitiveStyle::with_stroke(border_color, if is_selected { 2 } else { 1 }))
            .draw(gpu);
            
            let text_color = if is_selected {
                Rgb888::WHITE
            } else {
                Rgb888::new(200, 200, 210)
            };
            let btn_text_style = MonoTextStyle::new(&FONT_7X14, text_color);
            let _ = Text::new(label, Point::new(*x + 8, 520), btn_text_style).draw(gpu);
        }
        
        // === Running Services (positioned to not overlap with buttons) ===
        let services_x = 700;
        let _ = Text::new("Running Services", Point::new(services_x, 310), accent_style).draw(gpu);
        let _ = Line::new(Point::new(services_x, 315), Point::new(services_x + 140, 315))
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
            let color = if *running { Rgb888::new(80, 200, 120) } else { Rgb888::new(150, 150, 160) };
            let _ = Circle::new(Point::new(x, y), 8)
                .into_styled(PrimitiveStyle::with_fill(color))
                .draw(gpu);
            let _ = Text::new(name, Point::new(x + 14, y + 6), text_style).draw(gpu);
        }
        
        // === Welcome Message (adjusted for 1024x768) ===
        let welcome_style = MonoTextStyle::new(&FONT_7X14, Rgb888::new(160, 160, 175));
        let _ = Text::new("HAVY OS is a lightweight operating system written in Rust, running on a", Point::new(col1_x, 560), welcome_style).draw(gpu);
        let _ = Text::new("RISC-V virtual machine in your browser.", Point::new(col1_x, 575), welcome_style).draw(gpu);
        
        // === Footer info ===
        let _ = Line::new(Point::new(30, 610), Point::new(994, 610))
            .into_styled(PrimitiveStyle::with_stroke(Rgb888::new(60, 60, 80), 1))
            .draw(gpu);
        
        let footer_style = MonoTextStyle::new(&FONT_7X14, Rgb888::new(120, 120, 140));
        let _ = Text::new("Built with: Rust, embedded-graphics, smoltcp, wasmi", Point::new(30, 630), footer_style).draw(gpu);
        let _ = Text::new("License: MIT | github.com/elribonazo/riscv-vm", Point::new(30, 645), footer_style).draw(gpu);
        
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
        let _ = Rectangle::new(Point::new(0, 738), Size::new(1024, 30))
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(25, 25, 35)))
            .draw(gpu);
        
        let _ = Text::new("HAVY OS | GPU Active", Point::new(10, 756), text_style).draw(gpu);
        
        // Display date/time from RTC, or uptime as fallback
        let time_str = if let Some(dt) = crate::device::rtc::get_datetime() {
            let month_name = match dt.month {
                1 => "Jan", 2 => "Feb", 3 => "Mar", 4 => "Apr",
                5 => "May", 6 => "Jun", 7 => "Jul", 8 => "Aug",
                9 => "Sep", 10 => "Oct", 11 => "Nov", 12 => "Dec",
                _ => "???"
            };
            format!("{} {:02} {:02}:{:02}", month_name, dt.day, dt.hour, dt.minute)
        } else {
            let uptime_ms = crate::get_time_ms() as u64;
            let uptime_secs = uptime_ms / 1000;
            let hours = uptime_secs / 3600;
            let minutes = (uptime_secs % 3600) / 60;
            let seconds = uptime_secs % 60;
            format!("Up: {:02}:{:02}:{:02}", hours, minutes, seconds)
        };
        let _ = Text::new(&time_str, Point::new(460, 756), text_style).draw(gpu);
        
        // Status indicators
        let net_color = if hw.network_available {
            Rgb888::new(80, 200, 120)
        } else {
            Rgb888::new(150, 150, 160)
        };
        let _ = Circle::new(Point::new(870, 745), 10)
            .into_styled(PrimitiveStyle::with_fill(net_color))
            .draw(gpu);
        let _ = Text::new("NET", Point::new(884, 756), text_style).draw(gpu);
        
        let _ = Circle::new(Point::new(920, 745), 10)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(80, 200, 120)))
            .draw(gpu);
        let _ = Text::new("CPU", Point::new(934, 756), text_style).draw(gpu);
        
        let _ = Circle::new(Point::new(970, 745), 10)
            .into_styled(PrimitiveStyle::with_fill(Rgb888::new(230, 180, 80)))
            .draw(gpu);
        let _ = Text::new("MEM", Point::new(984, 756), text_style).draw(gpu);
    });
    
    // If a child window is open, draw it on top of the main content
    if let Some(win_idx) = open_window {
        draw_child_window(win_idx);
    }
    // Flush deferred to end of gpuid tick
}

/// Handle input for main_screen screen (keyboard navigation and mouse)
/// Returns Some(button_index) if Enter was pressed on a button
pub fn handle_main_screen_input(event: d1_touch::InputEvent) -> Option<usize> {
    // Debug: log all non-ABS events (ABS events are too spammy)
    if event.event_type != EV_ABS && event.event_type != d1_touch::EV_SYN {
        use crate::device::uart::{write_str, write_line};
        let mut buf = [0u8; 8];
        write_str(format_u16(event.event_type, &mut buf));
        write_str(" code=");
        let mut buf2 = [0u8; 8];
        write_str(format_u16(event.code, &mut buf2));
        write_line("");
    }
    
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
            }
            _ => {}
        }
        return None;
    }
    
    // Handle character events (typed characters respecting keyboard layout)
    // These come from browser with actual character codes (e.g., '/' from Shift+7)
    if event.event_type == d1_touch::EV_CHAR {
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
    if event.event_type == d1_touch::EV_KEY {
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
                    if let Some(win_idx) = open_window {
                        // Close button position depends on which window is open
                        // Network window (idx 0): at (260, 180) - close button at (260 + 12, 180 + 10)
                        // Terminal window (idx 1): at (162, 134) - close button at (162 + 12, 134 + 10)
                        let (win_x, win_y) = if win_idx == 1 { (162, 134) } else { (260, 180) };
                        let close_btn_x = win_x + 12;
                        let close_btn_y = win_y + 10;
                        let dx = x - close_btn_x;
                        let dy = y - close_btn_y;
                        // Check if click is within 12px of button center (button is 12px diameter)
                        if dx * dx + dy * dy < 12 * 12 {
                            // Close the child window - use backing store for instant restore
                            // Clear terminal state on close
                            unsafe {
                                TERMINAL_INPUT_LEN = 0;
                                TERMINAL_OUTPUT_LEN = 0;
                                MAIN_SCREEN_OPEN_WINDOW = None;
                            }
                            restore_window_backing();
                            // Flush deferred to end of gpuid tick
                            return None;
                        }
                        
                        // If Terminal window is open, check for Run/Cancel button click
                        if win_idx == 1 && hit_test_terminal_send_button(x, y) {
                            // If command is running, this is a Cancel button
                            if unsafe { TERMINAL_COMMAND_RUNNING } {
                                request_cancel();
                            } else {
                                terminal_execute_command();
                            }
                            return None;
                        }
                    } else {
                        // Main window - check for button clicks
                        if let Some(button_idx) = hit_test_main_screen_button(x, y) {
                            // Open the child window for this button
                            // Clear terminal state when opening terminal
                            if button_idx == 1 {
                                unsafe {
                                    TERMINAL_INPUT_LEN = 0;
                                    TERMINAL_OUTPUT_LEN = 0;
                                }
                            }
                            unsafe {
                                MAIN_SCREEN_SELECTED_BUTTON = button_idx;
                                MAIN_SCREEN_OPEN_WINDOW = Some(button_idx);
                            }
                            draw_child_window(button_idx);
                            // Flush deferred to end of gpuid tick
                            return Some(button_idx);
                        }
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
        use crate::platform::d1_touch::KEY_ESC;
        
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
                    TERMINAL_OUTPUT_LEN = 0;
                    MAIN_SCREEN_OPEN_WINDOW = None;
                }
                restore_window_backing();
                // Flush deferred to end of gpuid tick
                return None;
            }
        }
        
        // If Terminal window is open, handle keyboard input
        if win_idx == 1 {
            // Handle special keys
            if handle_terminal_input(event.code, event.value) {
                return None;
            }
            
            // Handle printable characters (key codes 2-13 are numbers, 16-25 are letters etc)
            // Convert key code to ASCII character
            let ch = key_code_to_ascii(event.code);
            if ch != 0 {
                handle_terminal_char(ch);
                return None;
            }
        }
        
        return None;
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
                if MAIN_SCREEN_SELECTED_BUTTON < 1 {
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
                    TERMINAL_OUTPUT_LEN = 0;
                }
            }
            unsafe { MAIN_SCREEN_OPEN_WINDOW = Some(button_idx); }
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
        2 => b'1', 3 => b'2', 4 => b'3', 5 => b'4', 6 => b'5',
        7 => b'6', 8 => b'7', 9 => b'8', 10 => b'9', 11 => b'0',
        12 => b'-', 13 => b'=',
        
        // First letter row: QWERTYUIOP
        16 => b'q', 17 => b'w', 18 => b'e', 19 => b'r', 20 => b't',
        21 => b'y', 22 => b'u', 23 => b'i', 24 => b'o', 25 => b'p',
        26 => b'[', 27 => b']',
        
        // Second letter row: ASDFGHJKL
        30 => b'a', 31 => b's', 32 => b'd', 33 => b'f', 34 => b'g',
        35 => b'h', 36 => b'j', 37 => b'k', 38 => b'l',
        39 => b';', 40 => b'\'',
        
        // Third letter row: ZXCVBNM
        44 => b'z', 45 => b'x', 46 => b'c', 47 => b'v', 48 => b'b',
        49 => b'n', 50 => b'm',
        51 => b',', 52 => b'.', 53 => b'/',
        
        // Space
        57 => b' ',
        
        // Punctuation
        41 => b'`', 43 => b'\\',
        
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
    let _ = write!(writer, "CPU:          {} Core{} @ RISC-V", count, if count == 1 { "" } else { "s" });
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
        let _ = write!(writer, "Network:      {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
    } else {
        let _ = write!(writer, "Network:      Not connected");
    }
    let len = writer.pos;
    core::str::from_utf8(&buf[..len]).unwrap_or("Network: Unknown")
}
