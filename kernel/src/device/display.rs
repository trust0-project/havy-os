//! Display device abstraction
//!
//! Provides a unified interface for display devices:
//! - D1 Display Engine (DE2 + TCON + DSI)
//! - (Legacy) VirtIO GPU

use alloc::boxed::Box;
use core::ptr::addr_of_mut;

/// Pixel format for framebuffer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 32-bit ARGB (alpha in high byte)
    Argb8888,
    /// 32-bit XRGB (ignore alpha)
    Xrgb8888,
    /// 16-bit RGB565
    Rgb565,
    /// 24-bit RGB (no alpha)
    Rgb888,
}

impl PixelFormat {
    /// Bytes per pixel for this format
    pub fn bytes_per_pixel(&self) -> usize {
        match self {
            PixelFormat::Argb8888 | PixelFormat::Xrgb8888 => 4,
            PixelFormat::Rgb888 => 3,
            PixelFormat::Rgb565 => 2,
        }
    }
}

/// Display device trait
///
/// Implemented by display drivers (D1 DE2+TCON+DSI, VirtIO GPU, etc.)
pub trait DisplayDevice: Send + Sync {
    /// Get display width in pixels
    fn width(&self) -> u32;

    /// Get display height in pixels
    fn height(&self) -> u32;

    /// Get pixel format
    fn pixel_format(&self) -> PixelFormat;

    /// Get stride (bytes per row)
    fn stride(&self) -> usize {
        self.width() as usize * self.pixel_format().bytes_per_pixel()
    }

    /// Get mutable access to framebuffer
    ///
    /// Returns slice of raw pixel data in the device's pixel format
    fn framebuffer(&mut self) -> &mut [u8];

    /// Flush framebuffer to display
    ///
    /// For double-buffered displays, swaps buffers.
    /// For direct framebuffers, may be a no-op.
    fn flush(&mut self);

    /// Set a single pixel
    fn set_pixel(&mut self, x: u32, y: u32, color: u32) {
        if x >= self.width() || y >= self.height() {
            return;
        }
        let offset = (y as usize * self.stride()) + (x as usize * 4);
        let fb = self.framebuffer();
        if offset + 4 <= fb.len() {
            fb[offset..offset + 4].copy_from_slice(&color.to_le_bytes());
        }
    }

    /// Fill entire screen with a color
    fn fill(&mut self, color: u32) {
        let w = self.width();
        let h = self.height();
        for y in 0..h {
            for x in 0..w {
                self.set_pixel(x, y, color);
            }
        }
    }

    /// Clear screen to black
    fn clear(&mut self) {
        self.fill(0xFF000000); // Black with full alpha
    }
}

/// Global display device instance
static mut DISPLAY_DEVICE: Option<Box<dyn DisplayDevice>> = None;

/// Initialize the global display device
///
/// # Safety
/// Must only be called once during kernel init
pub unsafe fn init_display_device(device: Box<dyn DisplayDevice>) {
    DISPLAY_DEVICE = Some(device);
}

/// Get a mutable reference to the global display device
pub fn display_device_mut() -> Option<&'static mut dyn DisplayDevice> {
    unsafe { (*addr_of_mut!(DISPLAY_DEVICE)).as_mut().map(|d| d.as_mut()) }
}

/// Get display resolution
pub fn get_resolution() -> Option<(u32, u32)> {
    unsafe { DISPLAY_DEVICE.as_ref().map(|d| (d.width(), d.height())) }
}
