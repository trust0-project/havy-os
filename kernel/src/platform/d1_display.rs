//! Guest-owned scanout (reserved DRAM, stride padded to 256 bytes).
//!
//! Virt: 1024×768 at DRAM+16 MiB, doorbell at DRAM+0x00FF_F000.
//! D1:   480×480 ST7701 at DRAM+16 MiB (inside `0x4000_0000–0x6000_0000`).
//!
//! Pixels are ordinary DRAM stores. The host scrapes this one buffer;
//! flush fences and publishes version + dirty rect. No back→front copy.

use core::ptr::addr_of_mut;
use core::sync::atomic::{AtomicBool, Ordering};

use embedded_graphics::{
    draw_target::DrawTarget,
    geometry::{Dimensions, OriginDimensions, Size},
    pixelcolor::{Rgb888, RgbColor},
    primitives::Rectangle,
    Pixel,
};

use crate::platform::current;

pub const DISPLAY_WIDTH: u32 = current::DISPLAY_WIDTH;
pub const DISPLAY_HEIGHT: u32 = current::DISPLAY_HEIGHT;
pub const FB_STRIDE: usize = current::FB_STRIDE;
pub const FRAMEBUFFER_ADDR: usize = current::FB_ADDR;
pub const FB_META_ADDR: usize = current::FB_META_ADDR;

/// Historical doorbell offsets inside the reserved meta page (host scrape).
pub const DIRTY_RECT_ADDR: usize = FB_META_ADDR + 0xFE0;
pub const FRAME_VERSION_ADDR: usize = FB_META_ADDR + 0xFFC;

const FB_MAGIC: u32 = 0x4856_4642; // "HVFB"
const FB_PROTO: u32 = 1;
const PIXELS_PER_ROW: usize = FB_STRIDE / 4;

static D1_DISPLAY_AVAILABLE: AtomicBool = AtomicBool::new(false);

static mut DIRTY_MIN_X: u32 = DISPLAY_WIDTH;
static mut DIRTY_MIN_Y: u32 = DISPLAY_HEIGHT;
static mut DIRTY_MAX_X: u32 = 0;
static mut DIRTY_MAX_Y: u32 = 0;
static mut FRAME_DIRTY: bool = false;
static mut FRAME_VERSION: u32 = 0;
static mut PIXEL_BATCH_MODE: bool = false;

#[inline]
fn pixel_ptr(x: u32, y: u32) -> *mut u32 {
    let idx = (y as usize) * PIXELS_PER_ROW + (x as usize);
    (FRAMEBUFFER_ADDR as *mut u32).wrapping_add(idx)
}

#[inline]
fn pack_pixel(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | 0xFF00_0000
}

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

#[inline(always)]
pub fn is_frame_dirty() -> bool {
    unsafe { FRAME_DIRTY }
}

#[inline(always)]
pub fn get_frame_version() -> u32 {
    unsafe { FRAME_VERSION }
}

#[inline(always)]
pub fn begin_pixel_batch() {
    unsafe { PIXEL_BATCH_MODE = true; }
}

#[inline(always)]
pub fn end_pixel_batch() {
    unsafe { PIXEL_BATCH_MODE = false; }
}

/// Publish FB identity + heap stats into the reserved doorbell page.
pub fn publish_meta() {
    let (used, _free) = crate::allocator::heap_stats();
    let total = crate::allocator::heap_size();
    unsafe {
        let base = FB_META_ADDR as *mut u32;
        core::ptr::write(base, FB_MAGIC);
        core::ptr::write(base.add(1), FB_PROTO);
        let fb = FRAMEBUFFER_ADDR as u64;
        core::ptr::write(base.add(2) as *mut u64, fb);
        core::ptr::write(base.add(4), DISPLAY_WIDTH);
        core::ptr::write(base.add(5), DISPLAY_HEIGHT);
        core::ptr::write(base.add(6), FB_STRIDE as u32);
        core::ptr::write(base.add(7), 0u32); // XRGB8888
        core::ptr::write(base.add(8) as *mut u64, used as u64);
        core::ptr::write(base.add(10) as *mut u64, total as u64);
    }
}

pub struct GpuDriver {
    width: u32,
    height: u32,
    initialized: AtomicBool,
}

impl GpuDriver {
    pub const fn new() -> Self {
        Self {
            width: DISPLAY_WIDTH,
            height: DISPLAY_HEIGHT,
            initialized: AtomicBool::new(false),
        }
    }

    pub fn init(&mut self) -> Result<(), &'static str> {
        self.initialized.store(true, Ordering::Release);
        Ok(())
    }

    pub fn init_clear_buffers(&mut self) {
        let pixel64: u64 = 0xFF000000_FF000000;
        let words = (FB_STRIDE / 8) * (self.height as usize);
        unsafe {
            let ptr64 = FRAMEBUFFER_ADDR as *mut u64;
            for i in 0..words {
                core::ptr::write(ptr64.add(i), pixel64);
            }
        }
        core::sync::atomic::fence(Ordering::Release);
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire)
    }

    #[inline(always)]
    pub fn set_pixel(&mut self, x: u32, y: u32, r: u8, g: u8, b: u8) {
        if x < self.width && y < self.height {
            let pixel = pack_pixel(r, g, b);
            unsafe {
                core::ptr::write(pixel_ptr(x, y), pixel);
                if !PIXEL_BATCH_MODE {
                    mark_dirty(x, y, 1, 1);
                }
            }
        }
    }

    pub fn clear(&mut self, r: u8, g: u8, b: u8) {
        let pixel = pack_pixel(r, g, b);
        let pixel64 = (pixel as u64) | ((pixel as u64) << 32);
        let words = (FB_STRIDE / 8) * (self.height as usize);
        unsafe {
            let ptr64 = FRAMEBUFFER_ADDR as *mut u64;
            for i in 0..words {
                core::ptr::write(ptr64.add(i), pixel64);
            }
        }
        mark_all_dirty();
    }

    #[inline]
    pub fn fill_hline(&mut self, x: u32, y: u32, width: u32, r: u8, g: u8, b: u8) {
        if y >= self.height || x >= self.width || width == 0 {
            return;
        }
        let w = width.min(self.width - x) as usize;
        let pixel = pack_pixel(r, g, b);
        unsafe {
            let row = pixel_ptr(x, y);
            if w >= 2 {
                let pixel64 = (pixel as u64) | ((pixel as u64) << 32);
                let ptr64 = row as *mut u64;
                let pairs = w / 2;
                for i in 0..pairs {
                    core::ptr::write(ptr64.add(i), pixel64);
                }
                if w % 2 == 1 {
                    core::ptr::write(row.add(w - 1), pixel);
                }
            } else {
                for i in 0..w {
                    core::ptr::write(row.add(i), pixel);
                }
            }
        }
        mark_dirty(x, y, w as u32, 1);
    }

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

    #[inline]
    pub fn get_pixel(&self, x: u32, y: u32) -> u32 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        unsafe { core::ptr::read(pixel_ptr(x, y) as *const u32) }
    }

    #[inline]
    pub fn put_pixel(&mut self, x: u32, y: u32, pixel: u32) {
        if x >= self.width || y >= self.height {
            return;
        }
        unsafe { core::ptr::write(pixel_ptr(x, y), pixel); }
        mark_dirty(x, y, 1, 1);
    }

    #[inline]
    pub fn read_rect(&self, x: u32, y: u32, w: usize, h: usize, buf: &mut [u32]) -> usize {
        let mut count = 0;
        for row in 0..h {
            let cy = y + row as u32;
            if cy >= self.height { break; }
            for col in 0..w {
                let cx = x + col as u32;
                if cx >= self.width { continue; }
                let idx = row * w + col;
                if idx < buf.len() {
                    buf[idx] = self.get_pixel(cx, cy);
                    count += 1;
                }
            }
        }
        count
    }

    #[inline]
    pub fn read_rect_fast(&self, x: u32, y: u32, w: usize, h: usize, buf: &mut [u32]) -> usize {
        if w == 0 || h == 0 || buf.len() < w * h {
            return 0;
        }
        let mut count = 0;
        unsafe {
            for row in 0..h {
                let cy = y + row as u32;
                if cy >= self.height { break; }
                let actual_w = if x + w as u32 > self.width {
                    (self.width - x) as usize
                } else {
                    w
                };
                if actual_w > 0 && x < self.width {
                    core::ptr::copy_nonoverlapping(
                        pixel_ptr(x, cy) as *const u32,
                        buf.as_mut_ptr().add(row * w),
                        actual_w,
                    );
                    count += actual_w;
                }
            }
        }
        count
    }

    #[inline]
    pub fn write_rect(&mut self, x: u32, y: u32, w: usize, h: usize, buf: &[u32], mask: &[u8]) {
        unsafe {
            for row in 0..h {
                let cy = y + row as u32;
                if cy >= self.height { break; }
                for col in 0..w {
                    let cx = x + col as u32;
                    if cx >= self.width { continue; }
                    let idx = row * w + col;
                    if idx < buf.len() && idx < mask.len() && mask[idx] != 0 {
                        core::ptr::write(pixel_ptr(cx, cy), buf[idx]);
                    }
                }
            }
        }
        mark_dirty(x, y, w as u32, h as u32);
    }

    #[inline]
    pub fn blit_rect(&mut self, x: u32, y: u32, w: usize, h: usize, buf: &[u32]) {
        if w == 0 || h == 0 || buf.len() < w * h {
            return;
        }
        unsafe {
            for row in 0..h {
                let cy = y + row as u32;
                if cy >= self.height { break; }
                let actual_w = if x + w as u32 > self.width {
                    (self.width - x) as usize
                } else {
                    w
                };
                if actual_w > 0 && x < self.width {
                    core::ptr::copy_nonoverlapping(
                        buf.as_ptr().add(row * w),
                        pixel_ptr(x, cy),
                        actual_w,
                    );
                }
            }
        }
        mark_dirty(x, y, w as u32, h as u32);
    }

    /// Row-wise blit of packed XRGB pixels (source stride = `w` pixels).
    #[inline]
    pub fn blit_row(&mut self, x: u32, y: u32, pixels: &[u32]) {
        if y >= self.height || x >= self.width || pixels.is_empty() {
            return;
        }
        let w = pixels.len().min((self.width - x) as usize);
        if w == 0 {
            return;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(pixels.as_ptr(), pixel_ptr(x, y), w);
        }
        mark_dirty(x, y, w as u32, 1);
    }

    #[inline]
    pub fn draw_cursor_bitmap(&mut self, x: i32, y: i32, w: usize, h: usize, bitmap: &[u8]) {
        for row in 0..h {
            let cy = y + row as i32;
            if cy < 0 || cy >= self.height as i32 { continue; }
            for col in 0..w {
                let cx = x + col as i32;
                if cx < 0 || cx >= self.width as i32 { continue; }
                let pixel_type = bitmap[row * w + col];
                let color = match pixel_type {
                    1 => 0xFF000000u32,
                    2 => 0xFFFFFFFFu32,
                    _ => continue,
                };
                unsafe { core::ptr::write(pixel_ptr(cx as u32, cy as u32), color); }
            }
        }
        let clip_x = x.max(0) as u32;
        let clip_y = y.max(0) as u32;
        mark_dirty(clip_x, clip_y, w as u32, h as u32);
    }

    pub fn flush(&self) {
        if !self.is_initialized() {
            return;
        }
        crate::platform::d1_display::flush();
    }

    pub fn framebuffer_ptr(&self) -> *const u32 {
        FRAMEBUFFER_ADDR as *const u32
    }

    pub fn framebuffer_bytes(&self) -> &[u8] {
        let fb_size = FB_STRIDE * self.height as usize;
        unsafe { core::slice::from_raw_parts(FRAMEBUFFER_ADDR as *const u8, fb_size) }
    }
}

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

    fn fill_solid(
        &mut self,
        area: &Rectangle,
        color: Self::Color,
    ) -> Result<(), Self::Error> {
        let clipped = area.intersection(&self.bounding_box());
        if let Some(br) = clipped.bottom_right() {
            let x = clipped.top_left.x.max(0) as u32;
            let y = clipped.top_left.y.max(0) as u32;
            let w = (br.x as u32).saturating_sub(x) + 1;
            let h = (br.y as u32).saturating_sub(y) + 1;
            if w > 0 && h > 0 {
                begin_pixel_batch();
                self.fill_rect(x, y, w, h, color.r(), color.g(), color.b());
                end_pixel_batch();
                mark_dirty(x, y, w, h);
            }
        }
        Ok(())
    }

    /// Row blit: convert each scanline to XRGB and copy it in one go.
    fn fill_contiguous<I>(
        &mut self,
        area: &Rectangle,
        colors: I,
    ) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Self::Color>,
    {
        let fb = self.bounding_box();
        let clipped = area.intersection(&fb);
        let mut colors = colors.into_iter();
        let area_w = area.size.width as i32;
        let area_h = area.size.height as i32;
        if area_w == 0 || area_h == 0 {
            return Ok(());
        }

        let mut row_buf = [0u32; 1024];
        begin_pixel_batch();
        for row in 0..area_h {
            let py = area.top_left.y + row;
            let mut n = 0usize;
            let mut start_x: Option<u32> = None;
            for col in 0..area_w {
                let color = match colors.next() {
                    Some(c) => c,
                    None => {
                        end_pixel_batch();
                        self.mark_area_dirty(area, &fb);
                        return Ok(());
                    }
                };
                let px = area.top_left.x + col;
                if px >= 0
                    && py >= 0
                    && (px as u32) < self.width
                    && (py as u32) < self.height
                    && n < row_buf.len()
                    && clipped.contains(embedded_graphics::prelude::Point::new(px, py))
                {
                    if start_x.is_none() {
                        start_x = Some(px as u32);
                    }
                    row_buf[n] = pack_pixel(color.r(), color.g(), color.b());
                    n += 1;
                }
            }
            if let Some(sx) = start_x {
                if py >= 0 {
                    self.blit_row(sx, py as u32, &row_buf[..n]);
                }
            }
        }
        end_pixel_batch();
        self.mark_area_dirty(area, &fb);
        Ok(())
    }

    fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
        GpuDriver::clear(self, color.r(), color.g(), color.b());
        Ok(())
    }
}

impl GpuDriver {
    #[inline]
    fn mark_area_dirty(&self, area: &Rectangle, fb: &Rectangle) {
        let clipped = area.intersection(fb);
        if let Some(br) = clipped.bottom_right() {
            let x = clipped.top_left.x.max(0) as u32;
            let y = clipped.top_left.y.max(0) as u32;
            let w = (br.x as u32).saturating_sub(x) + 1;
            let h = (br.y as u32).saturating_sub(y) + 1;
            if w > 0 && h > 0 {
                mark_dirty(x, y, w, h);
            }
        }
    }
}

static mut GPU_DRIVER: Option<GpuDriver> = None;

pub fn init() -> Result<(), &'static str> {
    let mut gpu = GpuDriver::new();
    gpu.init()?;
    unsafe {
        GPU_DRIVER = Some(gpu);
    }
    D1_DISPLAY_AVAILABLE.store(true, Ordering::Release);
    publish_meta();
    #[cfg(feature = "d1")]
    crate::platform::d1_de::init();
    #[cfg(not(feature = "d1"))]
    crate::device::virtio_gpu::init();
    Ok(())
}

pub fn is_available() -> bool {
    D1_DISPLAY_AVAILABLE.load(Ordering::Relaxed)
}

pub fn init_clear_buffers() {
    with_gpu(|gpu| {
        gpu.init_clear_buffers();
    });
    mark_all_dirty();
    flush();
}

pub fn with_gpu<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut GpuDriver) -> R,
{
    unsafe { (*addr_of_mut!(GPU_DRIVER)).as_mut().map(f) }
}

/// Fence pixel stores, then publish dirty rect + version. Host scrapes this buffer.
pub fn flush() {
    unsafe {
        if !FRAME_DIRTY {
            return;
        }

        let min_x = DIRTY_MIN_X;
        let min_y = DIRTY_MIN_Y;
        let max_x = DIRTY_MAX_X;
        let max_y = DIRTY_MAX_Y;

        if min_x >= max_x || min_y >= max_y {
            reset_dirty();
            return;
        }

        core::sync::atomic::fence(Ordering::Release);

        crate::perfstat::inc(crate::perfstat::id::FRAMES_FLUSHED);
        crate::perfstat::add(
            crate::perfstat::id::DIRTY_PIXELS,
            ((max_x - min_x) as u64) * ((max_y - min_y) as u64),
        );

        FRAME_VERSION = FRAME_VERSION.wrapping_add(1);

        let dirty_rect_ptr = DIRTY_RECT_ADDR as *mut u32;
        core::ptr::write_volatile(dirty_rect_ptr, min_x);
        core::ptr::write_volatile(dirty_rect_ptr.add(1), min_y);
        core::ptr::write_volatile(dirty_rect_ptr.add(2), max_x);
        core::ptr::write_volatile(dirty_rect_ptr.add(3), max_y);
        core::ptr::write_volatile(FRAME_VERSION_ADDR as *mut u32, FRAME_VERSION);

        publish_meta();
        #[cfg(not(feature = "d1"))]
        crate::device::virtio_gpu::flush_rect(min_x, min_y, max_x.saturating_sub(min_x), max_y.saturating_sub(min_y));
        #[cfg(feature = "d1")]
        crate::platform::d1_de::kick();
        reset_dirty();
    }
}

pub fn clear_display() {
    let fb_size_bytes = FB_STRIDE * DISPLAY_HEIGHT as usize;
    unsafe {
        core::ptr::write_bytes(FRAMEBUFFER_ADDR as *mut u8, 0, fb_size_bytes);
        // Opaque black first pixel so a version bump is visible.
        core::ptr::write(FRAMEBUFFER_ADDR as *mut u32, 0xFF000000);
    }
    core::sync::atomic::fence(Ordering::Release);
    mark_all_dirty();
}
