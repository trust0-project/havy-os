//! Unified Display Driver for HAVY OS
//!
//! This driver provides framebuffer rendering for both:
//! - D1 SoC Display Engine (DE2 + TCON + MIPI DSI)
//! - Emulator mode (direct framebuffer access)
//!
//! # Display Pipeline (D1 Hardware)
//! ```text
//! Framebuffer → DE2 Mixer → TCON LCD → MIPI DSI → Panel
//! ```
//!
//! # Display Resolution
//! 1024x768 pixels, XRGB8888 format (32-bit BGRA)

use core::sync::atomic::{AtomicBool, Ordering};
use alloc::vec::Vec;

use embedded_graphics::{
    draw_target::DrawTarget,
    geometry::{OriginDimensions, Size},
    pixelcolor::{Rgb888, RgbColor},
    primitives::Rectangle,
    Pixel,
};

// =============================================================================
// Constants
// =============================================================================

/// Display dimensions (1024x768)
pub const DISPLAY_WIDTH: u32 = 1024;
pub const DISPLAY_HEIGHT: u32 = 768;
const FRAMEBUFFER_SIZE: usize = (DISPLAY_WIDTH * DISPLAY_HEIGHT * 4) as usize;

/// Fixed framebuffer physical address (FRONT BUFFER)
/// This is what the emulator reads for display
pub const FRAMEBUFFER_ADDR: usize = 0x8100_0000;

/// Back buffer address for double-buffering
/// All rendering happens here, then copied to front buffer on flush
/// NOTE: Front buffer (1024*768*4 = 3.1MB) ends at 0x8130_0000
///       So back buffer must be at or after 0x8130_0000
pub const BACK_BUFFER_ADDR: usize = 0x8140_0000;

/// Frame version address - VM reads this u32 to detect new frames
/// Located just before framebuffer for easy access
pub const FRAME_VERSION_ADDR: usize = 0x80FF_FFFC;

/// Global flag to track if display was initialized
static D1_DISPLAY_AVAILABLE: AtomicBool = AtomicBool::new(false);

// =============================================================================
// Dirty Rectangle Tracking
// =============================================================================

/// Dirty rectangle bounds for partial flush optimization
/// Only the dirty region is copied from back buffer to front buffer
static mut DIRTY_MIN_X: u32 = DISPLAY_WIDTH;
static mut DIRTY_MIN_Y: u32 = DISPLAY_HEIGHT;
static mut DIRTY_MAX_X: u32 = 0;
static mut DIRTY_MAX_Y: u32 = 0;
static mut FRAME_DIRTY: bool = false;

/// Frame version counter - increments each time flush() actually copies data
/// Browser can compare this to skip fetching unchanged frames
static mut FRAME_VERSION: u32 = 0;

/// Mark a rectangular region as dirty
#[inline(always)]
pub fn mark_dirty(x: u32, y: u32, width: u32, height: u32) {
    unsafe {
        DIRTY_MIN_X = DIRTY_MIN_X.min(x);
        DIRTY_MIN_Y = DIRTY_MIN_Y.min(y);
        DIRTY_MAX_X = DIRTY_MAX_X.max((x + width).min(DISPLAY_WIDTH));
        DIRTY_MAX_Y = DIRTY_MAX_Y.max((y + height).min(DISPLAY_HEIGHT));
        FRAME_DIRTY = true;
    }
}

/// Mark entire screen as dirty (for clear operations or external draws)
#[inline(always)]
pub fn mark_all_dirty() {
    unsafe {
        DIRTY_MIN_X = 0;
        DIRTY_MIN_Y = 0;
        DIRTY_MAX_X = DISPLAY_WIDTH;
        DIRTY_MAX_Y = DISPLAY_HEIGHT;
        FRAME_DIRTY = true;
    }
}

/// Reset dirty tracking after flush
#[inline(always)]
fn reset_dirty() {
    unsafe {
        DIRTY_MIN_X = DISPLAY_WIDTH;
        DIRTY_MIN_Y = DISPLAY_HEIGHT;
        DIRTY_MAX_X = 0;
        DIRTY_MAX_Y = 0;
        FRAME_DIRTY = false;
    }
}

/// Check if frame has any dirty pixels
#[inline(always)]
pub fn is_frame_dirty() -> bool {
    unsafe { FRAME_DIRTY }
}

/// Get the current frame version (increments each flush that copies data)
/// Browser uses this to skip fetching unchanged frames
#[inline(always)]
pub fn get_frame_version() -> u32 {
    unsafe { FRAME_VERSION }
}

// =============================================================================
// GpuDriver - Main rendering interface
// =============================================================================

/// GPU Driver for framebuffer rendering
/// Provides pixel operations, drawing primitives, and embedded-graphics support
pub struct GpuDriver {
    width: u32,
    height: u32,
    initialized: AtomicBool,
}

impl GpuDriver {
    /// Create a new GPU driver
    pub const fn new() -> Self {
        Self {
            width: DISPLAY_WIDTH,
            height: DISPLAY_HEIGHT,
            initialized: AtomicBool::new(false),
        }
    }

    /// Initialize the GPU driver
    pub fn init(&mut self) -> Result<(), &'static str> {
        // Clear both framebuffers to black
        let fb_size = (self.width * self.height) as usize;
        unsafe {
            let front_ptr = FRAMEBUFFER_ADDR as *mut u32;
            let back_ptr = BACK_BUFFER_ADDR as *mut u32;
            for i in 0..fb_size {
                core::ptr::write_volatile(front_ptr.add(i), 0xFF000000); // Opaque black
                core::ptr::write_volatile(back_ptr.add(i), 0xFF000000);
            }
        }

        self.initialized.store(true, Ordering::Release);
        Ok(())
    }

    /// Get display width
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Get display height
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Check if GPU is initialized
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire)
    }

    /// Set a pixel in the back buffer (RGBA format)
    pub fn set_pixel(&mut self, x: u32, y: u32, r: u8, g: u8, b: u8) {
        if x < self.width && y < self.height {
            let idx = (y * self.width + x) as usize;
            // BGRA format: 0xAABBGGRR (little-endian)
            let pixel = ((r as u32) << 0) | ((g as u32) << 8) | ((b as u32) << 16) | 0xFF000000;
            unsafe {
                let fb_ptr = BACK_BUFFER_ADDR as *mut u32;
                core::ptr::write_volatile(fb_ptr.add(idx), pixel);
            }
            mark_dirty(x, y, 1, 1);
        }
    }

    /// Clear the back buffer using 64-bit writes for double speed
    pub fn clear(&mut self, r: u8, g: u8, b: u8) {
        let pixel = ((r as u32) << 0) | ((g as u32) << 8) | ((b as u32) << 16) | 0xFF000000;
        let pixel64 = (pixel as u64) | ((pixel as u64) << 32);
        let fb_size = (self.width * self.height) as usize;
        unsafe {
            let fb_ptr = BACK_BUFFER_ADDR as *mut u64;
            // Write pairs of pixels (64-bit) - 2× faster than 32-bit
            let pairs = fb_size / 2;
            for i in 0..pairs {
                core::ptr::write_volatile(fb_ptr.add(i), pixel64);
            }
            // Handle odd pixel if needed
            if fb_size % 2 != 0 {
                let last_ptr = (BACK_BUFFER_ADDR as *mut u32).add(fb_size - 1);
                core::ptr::write_volatile(last_ptr, pixel);
            }
        }
        mark_all_dirty();
    }

    /// Fast horizontal line fill (much faster than pixel-by-pixel for rectangles)
    #[inline]
    pub fn fill_hline(&mut self, x: u32, y: u32, width: u32, r: u8, g: u8, b: u8) {
        if y >= self.height || x >= self.width || width == 0 {
            return;
        }
        let w = width.min(self.width - x) as usize;
        let pixel = ((r as u32) << 0) | ((g as u32) << 8) | ((b as u32) << 16) | 0xFF000000;
        let start_idx = (y * self.width + x) as usize;
        unsafe {
            let fb_ptr = BACK_BUFFER_ADDR as *mut u32;
            // Use 64-bit writes for longer lines
            if w >= 4 {
                let pixel64 = (pixel as u64) | ((pixel as u64) << 32);
                let ptr64 = fb_ptr.add(start_idx) as *mut u64;
                let pairs = w / 2;
                for i in 0..pairs {
                    core::ptr::write_volatile(ptr64.add(i), pixel64);
                }
                // Handle remaining pixels
                for i in (pairs * 2)..w {
                    core::ptr::write_volatile(fb_ptr.add(start_idx + i), pixel);
                }
            } else {
                for i in 0..w {
                    core::ptr::write_volatile(fb_ptr.add(start_idx + i), pixel);
                }
            }
        }
        mark_dirty(x, y, w as u32, 1);
    }

    /// Fast filled rectangle using horizontal line fills
    #[inline]
    pub fn fill_rect(&mut self, x: u32, y: u32, width: u32, height: u32, r: u8, g: u8, b: u8) {
        if y >= self.height || x >= self.width || width == 0 || height == 0 {
            return;
        }
        let h = height.min(self.height.saturating_sub(y));
        for row in 0..h {
            self.fill_hline(x, y + row, width, r, g, b);
        }
    }

    /// Read a pixel from the back buffer (returns RGBA as u32)
    #[inline]
    pub fn get_pixel(&self, x: u32, y: u32) -> u32 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        let idx = (y * self.width + x) as usize;
        unsafe {
            let fb_ptr = BACK_BUFFER_ADDR as *const u32;
            core::ptr::read_volatile(fb_ptr.add(idx))
        }
    }

    /// Set a pixel in the back buffer directly (for cursor restore)
    #[inline]
    pub fn put_pixel(&mut self, x: u32, y: u32, pixel: u32) {
        if x >= self.width || y >= self.height {
            return;
        }
        let idx = (y * self.width + x) as usize;
        unsafe {
            let fb_ptr = BACK_BUFFER_ADDR as *mut u32;
            core::ptr::write_volatile(fb_ptr.add(idx), pixel);
        }
        mark_dirty(x, y, 1, 1);
    }

    /// Read a rectangle of pixels into a buffer (for cursor backup)
    /// Returns number of pixels read
    #[inline]
    pub fn read_rect(&self, x: u32, y: u32, w: usize, h: usize, buf: &mut [u32]) -> usize {
        let mut count = 0;
        unsafe {
            let fb_ptr = BACK_BUFFER_ADDR as *const u32;
            for row in 0..h {
                let cy = y + row as u32;
                if cy >= self.height { break; }
                let row_start = (cy * self.width) as usize;
                for col in 0..w {
                    let cx = x + col as u32;
                    if cx >= self.width { continue; }
                    let idx = row * w + col;
                    if idx < buf.len() {
                        buf[idx] = core::ptr::read_volatile(fb_ptr.add(row_start + cx as usize));
                        count += 1;
                    }
                }
            }
        }
        count
    }

    /// Fast read a rectangle of pixels into a buffer - copies entire rows at once
    /// This is much faster than read_rect for large regions
    #[inline]
    pub fn read_rect_fast(&self, x: u32, y: u32, w: usize, h: usize, buf: &mut [u32]) -> usize {
        if w == 0 || h == 0 || buf.len() < w * h {
            return 0;
        }
        let mut count = 0;
        unsafe {
            let fb_ptr = BACK_BUFFER_ADDR as *const u32;
            let screen_width = self.width as usize;
            
            for row in 0..h {
                let cy = y + row as u32;
                if cy >= self.height { break; }
                
                // Calculate actual width to copy (clip to screen edge)
                let actual_w = if x + w as u32 > self.width {
                    (self.width - x) as usize
                } else {
                    w
                };
                
                if actual_w > 0 && x < self.width {
                    let fb_offset = (cy as usize * screen_width) + x as usize;
                    let buf_offset = row * w;
                    
                    // Copy entire row at once - FAST
                    core::ptr::copy_nonoverlapping(
                        fb_ptr.add(fb_offset),
                        buf.as_mut_ptr().add(buf_offset),
                        actual_w
                    );
                    count += actual_w;
                }
            }
        }
        count
    }

    /// Write a rectangle of pixels to the back buffer (for cursor restore)
    /// Skips pixels with mask value 0
    #[inline]
    pub fn write_rect(&mut self, x: u32, y: u32, w: usize, h: usize, buf: &[u32], mask: &[u8]) {
        unsafe {
            let fb_ptr = BACK_BUFFER_ADDR as *mut u32;
            for row in 0..h {
                let cy = y + row as u32;
                if cy >= self.height { break; }
                let row_start = (cy * self.width) as usize;
                for col in 0..w {
                    let cx = x + col as u32;
                    if cx >= self.width { continue; }
                    let idx = row * w + col;
                    // Only write pixels where mask is non-zero (cursor was drawn there)
                    if idx < buf.len() && idx < mask.len() && mask[idx] != 0 {
                        core::ptr::write_volatile(fb_ptr.add(row_start + cx as usize), buf[idx]);
                    }
                }
            }
        }
        // Mark the entire rect as dirty (mask means we touched this area)
        mark_dirty(x, y, w as u32, h as u32);
    }

    /// Fast blit a rectangle to the back buffer - copies entire rows at once
    /// This is much faster than write_rect for large regions (no mask checking)
    #[inline]
    pub fn blit_rect(&mut self, x: u32, y: u32, w: usize, h: usize, buf: &[u32]) {
        if w == 0 || h == 0 || buf.len() < w * h {
            return;
        }
        unsafe {
            let fb_ptr = BACK_BUFFER_ADDR as *mut u32;
            let screen_width = self.width as usize;
            
            for row in 0..h {
                let cy = y + row as u32;
                if cy >= self.height { break; }
                
                // Calculate actual width to copy (clip to screen edge)
                let actual_w = if x + w as u32 > self.width {
                    (self.width - x) as usize
                } else {
                    w
                };
                
                if actual_w > 0 && x < self.width {
                    let fb_offset = (cy as usize * screen_width) + x as usize;
                    let buf_offset = row * w;
                    
                    // Copy entire row at once - FAST
                    core::ptr::copy_nonoverlapping(
                        buf.as_ptr().add(buf_offset),
                        fb_ptr.add(fb_offset),
                        actual_w
                    );
                }
            }
        }
        mark_dirty(x, y, w as u32, h as u32);
    }

    /// Draw cursor bitmap directly to framebuffer (batched write)
    #[inline]
    pub fn draw_cursor_bitmap(&mut self, x: i32, y: i32, w: usize, h: usize, bitmap: &[u8]) {
        unsafe {
            let fb_ptr = BACK_BUFFER_ADDR as *mut u32;
            for row in 0..h {
                let cy = y + row as i32;
                if cy < 0 || cy >= self.height as i32 { continue; }
                let row_start = (cy as u32 * self.width) as usize;
                for col in 0..w {
                    let cx = x + col as i32;
                    if cx < 0 || cx >= self.width as i32 { continue; }
                    let pixel_type = bitmap[row * w + col];
                    let color = match pixel_type {
                        1 => 0xFF000000u32, // Black border
                        2 => 0xFFFFFFFFu32, // White fill
                        _ => continue,       // Transparent
                    };
                    core::ptr::write_volatile(fb_ptr.add(row_start + cx as usize), color);
                }
            }
        }
        // Mark cursor area as dirty
        let clip_x = x.max(0) as u32;
        let clip_y = y.max(0) as u32;
        mark_dirty(clip_x, clip_y, w as u32, h as u32);
    }

    /// Copy dirty region of back buffer to front buffer and flush to display
    /// Uses the optimized dirty rect tracking for minimal memory transfers
    pub fn flush(&self) {
        if !self.is_initialized() {
            return;
        }
        // Delegate to the module-level optimized flush
        crate::d1_display::flush();
    }

    /// Get raw framebuffer pointer (for direct memory access)
    pub fn framebuffer_ptr(&self) -> *const u32 {
        FRAMEBUFFER_ADDR as *const u32
    }

    /// Get framebuffer as bytes
    pub fn framebuffer_bytes(&self) -> &[u8] {
        let fb_size = (self.width * self.height * 4) as usize;
        unsafe {
            core::slice::from_raw_parts(FRAMEBUFFER_ADDR as *const u8, fb_size)
        }
    }
}

// =============================================================================
// embedded-graphics DrawTarget implementation
// =============================================================================

impl OriginDimensions for GpuDriver {
    fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }
}

impl DrawTarget for GpuDriver {
    type Color = Rgb888;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coord, color) in pixels.into_iter() {
            if coord.x >= 0 && coord.y >= 0 {
                let x = coord.x as u32;
                let y = coord.y as u32;
                if x < self.width && y < self.height {
                    self.set_pixel(x, y, color.r(), color.g(), color.b());
                }
            }
        }
        Ok(())
    }

    fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
        GpuDriver::clear(self, color.r(), color.g(), color.b());
        Ok(())
    }
}

// =============================================================================
// Global GPU driver instance and module-level functions
// =============================================================================

/// Global GPU driver instance
static mut GPU_DRIVER: Option<GpuDriver> = None;

/// Initialize the global GPU driver
/// This should be called early in boot to enable framebuffer rendering
pub fn init() -> Result<(), &'static str> {
    let mut gpu = GpuDriver::new();
    gpu.init()?;
    unsafe {
        GPU_DRIVER = Some(gpu);
    }
    D1_DISPLAY_AVAILABLE.store(true, Ordering::Release);
    Ok(())
}

/// Check if display is available
pub fn is_available() -> bool {
    D1_DISPLAY_AVAILABLE.load(Ordering::Relaxed)
}

/// Get access to the global GPU driver
pub fn with_gpu<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut GpuDriver) -> R,
{
    unsafe {
        GPU_DRIVER.as_mut().map(f)
    }
}

/// Flush the display (transfer and present)
/// Only copies the dirty rectangle region from back buffer to front buffer.
/// Skips copy entirely if nothing has changed since last flush.
pub fn flush() {
    unsafe {
        // Skip if nothing changed
        if !FRAME_DIRTY {
            return;
        }
        
        // Get dirty bounds
        let min_x = DIRTY_MIN_X;
        let min_y = DIRTY_MIN_Y;
        let max_x = DIRTY_MAX_X;
        let max_y = DIRTY_MAX_Y;
        
        // Check for valid dirty rect
        if min_x >= max_x || min_y >= max_y {
            reset_dirty();
            return;
        }
        
        // Copy only the dirty rectangle row by row
        let dirty_width = (max_x - min_x) as usize;
        
        let src_base = BACK_BUFFER_ADDR as *const u8;
        let dst_base = FRAMEBUFFER_ADDR as *mut u8;
        
        for y in min_y..max_y {
            let row_offset = (y * DISPLAY_WIDTH + min_x) as usize * 4;
            let src_row = src_base.add(row_offset);
            let dst_row = dst_base.add(row_offset);
            core::ptr::copy_nonoverlapping(src_row, dst_row, dirty_width * 4);
        }
        
        // Increment frame version so browser knows to fetch new frame
        FRAME_VERSION = FRAME_VERSION.wrapping_add(1);
        
        // Write version to memory so VM can read it
        let version_ptr = FRAME_VERSION_ADDR as *mut u32;
        core::ptr::write_volatile(version_ptr, FRAME_VERSION);
        
        // Reset dirty tracking for next frame
        reset_dirty();
    }
}

/// Clear the display to black and flush
/// Used when gpuid service is stopped to clear the framebuffer
/// NOTE: This writes directly to both buffers for immediate effect
pub fn clear_display() {
    const FB_WIDTH: u32 = DISPLAY_WIDTH;
    const FB_HEIGHT: u32 = DISPLAY_HEIGHT;
    let fb_size = (FB_WIDTH * FB_HEIGHT) as usize;
    let black_pixel: u32 = 0xFF000000; // Opaque black (BGRA)

    unsafe {
        // Clear front buffer
        let front_ptr = FRAMEBUFFER_ADDR as *mut u32;
        for i in 0..fb_size {
            core::ptr::write_volatile(front_ptr.add(i), black_pixel);
        }

        // Clear back buffer
        let back_ptr = BACK_BUFFER_ADDR as *mut u32;
        for i in 0..fb_size {
            core::ptr::write_volatile(back_ptr.add(i), black_pixel);
        }
    }
}
