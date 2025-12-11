//! VirtIO GPU Driver for Guest Kernel
//!
//! This driver interfaces with the VirtIO GPU device (Device ID 16) in the host
//! to provide framebuffer rendering capabilities. Uses embedded-graphics for drawing.

use core::sync::atomic::{AtomicBool, Ordering};
use alloc::vec::Vec;

use embedded_graphics::{
    draw_target::DrawTarget,
    geometry::{OriginDimensions, Size},
    pixelcolor::{Rgb888, RgbColor},
    Pixel,
};

/// VirtIO GPU MMIO base address (assigned by bus enumeration)
const VIRTIO_GPU_BASE: usize = 0x1000_5000; // Placeholder - will be discovered

/// VirtIO GPU Device ID
const VIRTIO_GPU_DEVICE_ID: u32 = 16;

/// Fixed framebuffer physical address (FRONT BUFFER)
/// This is what the host reads for display
const FRAMEBUFFER_ADDR: usize = 0x8100_0000;

/// Back buffer address for double-buffering
/// All rendering happens here, then copied to front buffer on flush
const BACK_BUFFER_ADDR: usize = 0x8120_0000;

/// MMIO register offsets
const MAGIC_VALUE_OFFSET: usize = 0x000;
const VERSION_OFFSET: usize = 0x004;
const DEVICE_ID_OFFSET: usize = 0x008;
const VENDOR_ID_OFFSET: usize = 0x00c;
const DEVICE_FEATURES_OFFSET: usize = 0x010;
const DEVICE_FEATURES_SEL_OFFSET: usize = 0x014;
const DRIVER_FEATURES_OFFSET: usize = 0x020;
const DRIVER_FEATURES_SEL_OFFSET: usize = 0x024;
const QUEUE_SEL_OFFSET: usize = 0x030;
const QUEUE_NUM_MAX_OFFSET: usize = 0x034;
const QUEUE_NUM_OFFSET: usize = 0x038;
const QUEUE_READY_OFFSET: usize = 0x044;
const QUEUE_NOTIFY_OFFSET: usize = 0x050;
const INTERRUPT_STATUS_OFFSET: usize = 0x060;
const INTERRUPT_ACK_OFFSET: usize = 0x064;
const STATUS_OFFSET: usize = 0x070;
const QUEUE_DESC_LOW_OFFSET: usize = 0x080;
const QUEUE_DESC_HIGH_OFFSET: usize = 0x084;
const QUEUE_DRIVER_LOW_OFFSET: usize = 0x090;
const QUEUE_DRIVER_HIGH_OFFSET: usize = 0x094;
const QUEUE_DEVICE_LOW_OFFSET: usize = 0x0a0;
const QUEUE_DEVICE_HIGH_OFFSET: usize = 0x0a4;

// Config space
const CONFIG_EVENTS_READ: usize = 0x100;
const CONFIG_NUM_SCANOUTS: usize = 0x108;

// VirtIO GPU Command Types
const VIRTIO_GPU_CMD_GET_DISPLAY_INFO: u32 = 0x0100;
const VIRTIO_GPU_CMD_RESOURCE_CREATE_2D: u32 = 0x0101;
const VIRTIO_GPU_CMD_RESOURCE_UNREF: u32 = 0x0102;
const VIRTIO_GPU_CMD_SET_SCANOUT: u32 = 0x0103;
const VIRTIO_GPU_CMD_RESOURCE_FLUSH: u32 = 0x0104;
const VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D: u32 = 0x0105;
const VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;

// VirtIO GPU Formats
const VIRTIO_GPU_FORMAT_R8G8B8A8_UNORM: u32 = 67;

// Device status flags
const STATUS_ACKNOWLEDGE: u32 = 1;
const STATUS_DRIVER: u32 = 2;
const STATUS_DRIVER_OK: u32 = 4;
const STATUS_FEATURES_OK: u32 = 8;

/// Control header for GPU commands (24 bytes)
#[repr(C)]
#[derive(Clone, Copy)]
struct VirtioGpuCtrlHdr {
    cmd_type: u32,
    flags: u32,
    fence_id: u64,
    ctx_id: u32,
    padding: u32,
}

/// Resource create 2D command
#[repr(C)]
struct VirtioGpuResourceCreate2d {
    hdr: VirtioGpuCtrlHdr,
    resource_id: u32,
    format: u32,
    width: u32,
    height: u32,
}

/// Memory entry for attach backing
#[repr(C)]
struct VirtioGpuMemEntry {
    addr: u64,
    length: u32,
    padding: u32,
}

/// Attach backing command
#[repr(C)]
struct VirtioGpuResourceAttachBacking {
    hdr: VirtioGpuCtrlHdr,
    resource_id: u32,
    nr_entries: u32,
    // followed by VirtioGpuMemEntry array
}

/// Set scanout command
#[repr(C)]
struct VirtioGpuSetScanout {
    hdr: VirtioGpuCtrlHdr,
    r_x: u32,
    r_y: u32,
    r_width: u32,
    r_height: u32,
    scanout_id: u32,
    resource_id: u32,
}

/// Transfer to host 2D command
#[repr(C)]
struct VirtioGpuTransferToHost2d {
    hdr: VirtioGpuCtrlHdr,
    r_x: u32,
    r_y: u32,
    r_width: u32,
    r_height: u32,
    offset: u64,
    resource_id: u32,
    padding: u32,
}

/// Resource flush command
#[repr(C)]
struct VirtioGpuResourceFlush {
    hdr: VirtioGpuCtrlHdr,
    r_x: u32,
    r_y: u32,
    r_width: u32,
    r_height: u32,
    resource_id: u32,
    padding: u32,
}

/// VirtIO GPU driver state
pub struct GpuDriver {
    base: usize,
    width: u32,
    height: u32,
    resource_id: u32,
    initialized: AtomicBool,
}

impl GpuDriver {
    /// Probe for VirtIO GPU device at potential base addresses
    pub fn probe() -> Option<Self> {
        use crate::uart;
        
        // Try all VirtIO device addresses (0x1000_1000 + n*0x1000 for n=0..7)
        // The GPU device is added dynamically, so scan all slots
        const VIRTIO_BASE: usize = 0x1000_1000;
        const VIRTIO_STRIDE: usize = 0x1000;
        
        for i in 0..8 {
            let base = VIRTIO_BASE + i * VIRTIO_STRIDE;
            unsafe {
                let magic = core::ptr::read_volatile((base + MAGIC_VALUE_OFFSET) as *const u32);
                let device_id = core::ptr::read_volatile((base + DEVICE_ID_OFFSET) as *const u32);
                
                // Debug: show what we find at each slot
                uart::write_str("    +- Slot ");
                uart::write_u64(i as u64);
                uart::write_str(" @ 0x");
                uart::write_hex(base as u64);
                uart::write_str(": magic=0x");
                uart::write_hex(magic as u64);
                uart::write_str(", device_id=");
                uart::write_u64(device_id as u64);
                uart::write_line("");
                
                if magic == 0x7472_6976 && device_id == VIRTIO_GPU_DEVICE_ID {
                    uart::write_line("    +- Found GPU device!");
                    return Some(Self {
                        base,
                        width: 0,
                        height: 0,
                        resource_id: 1,
                        initialized: AtomicBool::new(false),
                    });
                }
            }
        }
        None
    }

    /// Initialize the GPU device
    pub fn init(&mut self) -> Result<(), &'static str> {
        unsafe {
            // Reset device
            core::ptr::write_volatile((self.base + STATUS_OFFSET) as *mut u32, 0);
            
            // Acknowledge
            core::ptr::write_volatile(
                (self.base + STATUS_OFFSET) as *mut u32,
                STATUS_ACKNOWLEDGE
            );
            
            // Driver
            core::ptr::write_volatile(
                (self.base + STATUS_OFFSET) as *mut u32,
                STATUS_ACKNOWLEDGE | STATUS_DRIVER
            );
            
            // Feature negotiation (no special features needed)
            core::ptr::write_volatile((self.base + DEVICE_FEATURES_SEL_OFFSET) as *mut u32, 0);
            let _features = core::ptr::read_volatile((self.base + DEVICE_FEATURES_OFFSET) as *const u32);
            core::ptr::write_volatile((self.base + DRIVER_FEATURES_SEL_OFFSET) as *mut u32, 0);
            core::ptr::write_volatile((self.base + DRIVER_FEATURES_OFFSET) as *mut u32, 0);
            
            // Features OK
            core::ptr::write_volatile(
                (self.base + STATUS_OFFSET) as *mut u32,
                STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK
            );
            
            // Verify features OK
            let status = core::ptr::read_volatile((self.base + STATUS_OFFSET) as *const u32);
            if (status & STATUS_FEATURES_OK) == 0 {
                return Err("Features not accepted");
            }
            
            // Get display info to learn dimensions
            // For now, use default dimensions
            self.width = 800;
            self.height = 600;
            
            // Clear both framebuffers to black (front and back for double-buffering)
            let fb_size = (self.width * self.height) as usize;
            let front_ptr = FRAMEBUFFER_ADDR as *mut u32;
            let back_ptr = BACK_BUFFER_ADDR as *mut u32;
            for i in 0..fb_size {
                core::ptr::write_volatile(front_ptr.add(i), 0xFF000000); // Opaque black
                core::ptr::write_volatile(back_ptr.add(i), 0xFF000000);
            }
            
            // Driver OK
            core::ptr::write_volatile(
                (self.base + STATUS_OFFSET) as *mut u32,
                STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK
            );
            
            self.initialized.store(true, Ordering::Release);
        }
        
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
            // RGBA format: 0xAABBGGRR (little-endian) - but Canvas expects RGBA so reorder
            let pixel = ((r as u32) << 0) | ((g as u32) << 8) | ((b as u32) << 16) | 0xFF000000;
            unsafe {
                let fb_ptr = BACK_BUFFER_ADDR as *mut u32;
                core::ptr::write_volatile(fb_ptr.add(idx), pixel);
            }
        }
    }

    /// Clear the back buffer to a solid color
    pub fn clear(&mut self, r: u8, g: u8, b: u8) {
        let pixel = ((r as u32) << 0) | ((g as u32) << 8) | ((b as u32) << 16) | 0xFF000000;
        let fb_size = (self.width * self.height) as usize;
        unsafe {
            let fb_ptr = BACK_BUFFER_ADDR as *mut u32;
            for i in 0..fb_size {
                core::ptr::write_volatile(fb_ptr.add(i), pixel);
            }
        }
    }

    /// Copy back buffer to front buffer and flush to display
    /// This ensures tear-free rendering by updating front buffer atomically
    pub fn flush(&self) {
        if !self.is_initialized() {
            return;
        }
        
        unsafe {
            // Copy back buffer to front buffer using fast bulk copy
            let fb_size_bytes = (self.width * self.height * 4) as usize;
            let src_ptr = BACK_BUFFER_ADDR as *const u8;
            let dst_ptr = FRAMEBUFFER_ADDR as *mut u8;
            
            // Use fast memcpy-style copy (non-overlapping)
            core::ptr::copy_nonoverlapping(src_ptr, dst_ptr, fb_size_bytes);
            
            // Notify queue 0 (control queue) for host to read
            core::ptr::write_volatile((self.base + QUEUE_NOTIFY_OFFSET) as *mut u32, 0);
        }
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

// Implement embedded-graphics DrawTarget trait
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
        self.clear(color.r(), color.g(), color.b());
        Ok(())
    }
}

/// Global GPU driver instance
static mut GPU_DRIVER: Option<GpuDriver> = None;

/// Initialize the global GPU driver
pub fn init() -> Result<(), &'static str> {
    if let Some(mut gpu) = GpuDriver::probe() {
        gpu.init()?;
        unsafe {
            GPU_DRIVER = Some(gpu);
        }
        Ok(())
    } else {
        Err("VirtIO GPU device not found")
    }
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

/// Check if GPU is available
pub fn is_available() -> bool {
    unsafe { GPU_DRIVER.is_some() }
}

/// Flush the display (transfer and present)
pub fn flush() {
    unsafe {
        if let Some(ref gpu) = GPU_DRIVER {
            gpu.flush();
        }
    }
}

/// Clear the display to black and flush
/// Used when gpuid service is stopped to clear the framebuffer
/// NOTE: This writes directly to the FRONT buffer (FRAMEBUFFER_ADDR) for immediate effect,
/// since get_gpu_frame() in the VM reads from the front buffer, not the back buffer.
pub fn clear_display() {
    // Clear front buffer directly (bypassing double-buffering)
    // This is necessary because:
    // 1. The VM's get_gpu_frame() reads from FRAMEBUFFER_ADDR (front buffer)
    // 2. clear() writes to BACK_BUFFER_ADDR, which wouldn't be visible immediately
    const FB_WIDTH: u32 = 800;
    const FB_HEIGHT: u32 = 600;
    let fb_size = (FB_WIDTH * FB_HEIGHT) as usize;
    let black_pixel: u32 = 0xFF000000; // Opaque black (RGBA)
    
    unsafe {
        let front_ptr = FRAMEBUFFER_ADDR as *mut u32;
        for i in 0..fb_size {
            core::ptr::write_volatile(front_ptr.add(i), black_pixel);
        }
    }
    
    // Also clear back buffer so next render starts clean
    unsafe {
        let back_ptr = BACK_BUFFER_ADDR as *mut u32;
        for i in 0..fb_size {
            core::ptr::write_volatile(back_ptr.add(i), black_pixel);
        }
    }
}
