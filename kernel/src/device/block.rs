//! Block device abstraction
//!
//! Provides a unified interface for block storage devices:
//! - D1 MMC/SD card controller
//! - (Legacy) VirtIO block device

use alloc::boxed::Box;

/// Block device error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockError {
    /// Device not ready or not present
    NotReady,
    /// Invalid sector number
    InvalidSector,
    /// Read operation failed
    ReadFailed,
    /// Write operation failed
    WriteFailed,
    /// Device I/O timeout
    Timeout,
    /// Buffer size mismatch
    BufferSize,
}

/// Block device trait
///
/// Implemented by storage device drivers (D1 MMC, VirtIO block, etc.)
pub trait BlockDevice: Send + Sync {
    /// Read sectors from the device
    ///
    /// # Arguments
    /// * `start_sector` - First sector to read (512 bytes each)
    /// * `buf` - Buffer to read into (must be multiple of 512 bytes)
    ///
    /// # Returns
    /// * `Ok(())` on success
    /// * `Err(BlockError)` on failure
    fn read(&self, start_sector: u64, buf: &mut [u8]) -> Result<(), BlockError>;

    /// Write sectors to the device
    ///
    /// # Arguments
    /// * `start_sector` - First sector to write
    /// * `buf` - Data to write (must be multiple of 512 bytes)
    fn write(&self, start_sector: u64, buf: &[u8]) -> Result<(), BlockError>;

    /// Get total number of sectors
    fn sector_count(&self) -> u64;

    /// Get sector size (typically 512 bytes)
    fn sector_size(&self) -> usize {
        512
    }

    /// Check if device is read-only
    fn is_read_only(&self) -> bool {
        false
    }

    /// Flush any cached writes to the device
    fn flush(&self) -> Result<(), BlockError> {
        Ok(())
    }
}

/// Global block device instance
static mut BLOCK_DEVICE: Option<Box<dyn BlockDevice>> = None;

/// Initialize the global block device
///
/// # Safety
/// Must only be called once during kernel init
pub unsafe fn init_block_device(device: Box<dyn BlockDevice>) {
    BLOCK_DEVICE = Some(device);
}

/// Get a reference to the global block device
pub fn block_device() -> Option<&'static dyn BlockDevice> {
    unsafe { BLOCK_DEVICE.as_ref().map(|d| d.as_ref()) }
}

/// Read sectors using the global block device
pub fn read_sectors(start: u64, buf: &mut [u8]) -> Result<(), BlockError> {
    block_device()
        .ok_or(BlockError::NotReady)?
        .read(start, buf)
}

/// Write sectors using the global block device
pub fn write_sectors(start: u64, buf: &[u8]) -> Result<(), BlockError> {
    block_device()
        .ok_or(BlockError::NotReady)?
        .write(start, buf)
}
