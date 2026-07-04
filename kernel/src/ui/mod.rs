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

use crate::platform::d1_display;
use crate::uart;

// Module declarations
pub mod boot;
pub mod colors;
pub mod cursor;
pub mod main_screen;
pub mod manager;
pub mod widgets;

// Re-export commonly used items at the module root for backwards compatibility
pub use cursor::{
    get_cursor_pos, set_cursor_pos,
};
pub use main_screen::{
    handle_main_screen_input, setup_main_screen,
    update_main_screen_hardware_stats,
};
pub use manager::{
    with_ui, UiManager, UI_MANAGER,
};

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

/// Draw an embedded RGBA image to the framebuffer (fast blit).
///
/// Uses batch mode so the per-pixel dirty tracking is skipped and a single
/// dirty-rect mark covers the whole image. Opaque runs within a row are
/// coalesced into `fill_hline` bulk writes; only alpha-tested edge pixels
/// fall back to per-pixel stores.
pub(crate) fn draw_image(gpu: &mut d1_display::GpuDriver, x: u32, y: u32, width: u32, height: u32, pixels: &[u8]) {
    d1_display::begin_pixel_batch();
    for row in 0..height {
        let mut col = 0u32;
        while col < width {
            let i = ((row * width + col) * 4) as usize;
            if i + 3 >= pixels.len() {
                break;
            }
            let a = pixels[i + 3];
            if a <= 128 {
                col += 1;
                continue;
            }
            // Extend an opaque run of same-colored pixels for a bulk fill.
            let r = pixels[i];
            let g = pixels[i + 1];
            let b = pixels[i + 2];
            let mut run = 1u32;
            while col + run < width {
                let j = ((row * width + col + run) * 4) as usize;
                if j + 3 >= pixels.len() {
                    break;
                }
                if pixels[j + 3] <= 128
                    || pixels[j] != r
                    || pixels[j + 1] != g
                    || pixels[j + 2] != b
                {
                    break;
                }
                run += 1;
            }
            if run >= 4 {
                gpu.fill_hline(x + col, y + row, run, r, g, b);
            } else {
                for k in 0..run {
                    gpu.set_pixel(x + col + k, y + row, r, g, b);
                }
            }
            col += run;
        }
    }
    d1_display::end_pixel_batch();
    d1_display::mark_dirty(x, y, width, height);
}

