//! UI Manager for Kernel Graphics
//!
//! Uses embedded-graphics to render UI elements (text, buttons, boxes)
//! to the VirtIO GPU framebuffer.
//!
//! This module is organized into submodules:
//! - `colors`: Theme color constants
//! - `cursor`: Mouse/cursor handling
//! - `widgets`: UI widget components (Button, Label, etc.)
//! - `manager`: UiManager and global state
//! - `main_screen`: Main screen functionality
//! - `boot`: Boot screen setup

use crate::d1_display;

// Module declarations
pub mod boot;
pub mod colors;
pub mod cursor;
pub mod main_screen;
pub mod manager;
pub mod widgets;

// Re-export commonly used items at the module root for backwards compatibility
pub use colors::*;
pub use cursor::{
    draw_cursor, get_cursor_pos, get_mouse_buttons, hide_cursor, invalidate_cursor_backup,
    is_left_button_pressed, set_cursor_pos, set_mouse_button,
};
pub use main_screen::{
    get_hardware_info, handle_main_screen_input, hit_test_main_screen_button, setup_main_screen,
    update_main_screen_buttons, update_main_screen_hardware_stats, HardwareInfo,
};
pub use manager::{
    init, is_initialized, poll_input, render_and_flush, with_ui, UiManager, UI_MANAGER,
};
pub use widgets::*;
pub use boot::setup_boot_screen;

// Embedded Trust0 logo (64x64 RGBA = 16KB)
static LOGO_DATA: &[u8] = include_bytes!("logo.raw");
const LOGO_WIDTH: u32 = 64;
const LOGO_HEIGHT: u32 = 64;

// Small logo for title bars (24x24 RGBA = 2KB)
pub(crate) static LOGO_SMALL: &[u8] = include_bytes!("logo_small.raw");
pub(crate) const LOGO_SMALL_SIZE: u32 = 24;

// Screen resolution constants
pub const SCREEN_WIDTH: i32 = 1024;
pub const SCREEN_HEIGHT: i32 = 768;

/// Draw an embedded RGBA image to the framebuffer (fast blit)
pub(crate) fn draw_image(gpu: &mut d1_display::GpuDriver, x: u32, y: u32, width: u32, height: u32, pixels: &[u8]) {
    for row in 0..height {
        for col in 0..width {
            let i = ((row * width + col) * 4) as usize;
            if i + 3 < pixels.len() {
                let r = pixels[i];
                let g = pixels[i + 1];
                let b = pixels[i + 2];
                let a = pixels[i + 3];
                // Only draw non-transparent pixels
                if a > 128 {
                    gpu.set_pixel(x + col, y + row, r, g, b);
                }
            }
        }
    }
}
