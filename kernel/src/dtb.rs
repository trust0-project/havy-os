//! Device Tree Blob (DTB) parser for OpenSBI compliance.
//!
//! This module captures and parses the Device Tree Blob passed by OpenSBI
//! at kernel entry via the `a1` register.
//!
//! ## OpenSBI Boot Protocol
//!
//! When OpenSBI transfers control to S-mode kernel:
//! - a0 = hartid (hardware thread ID)
//! - a1 = DTB physical address (8-byte aligned)
//!
//! ## Usage
//!
//! The DTB address must be captured early in boot before `a1` is clobbered.
//! Call `init(dtb_addr)` from the entry point, then use `get_*` functions
//! to query device information.

use alloc::string::String;
use core::sync::atomic::{AtomicUsize, Ordering};

/// FDT header magic number
const FDT_MAGIC: u32 = 0xd00dfeed;

/// Stored DTB address from OpenSBI (a1 at entry)
static DTB_ADDRESS: AtomicUsize = AtomicUsize::new(0);

/// Stored DTB size (from header)
static DTB_SIZE: AtomicUsize = AtomicUsize::new(0);

/// Initialize DTB support with the address passed by OpenSBI.
///
/// # Arguments
/// * `dtb_addr` - Physical address of the DTB (from `a1` register at entry)
///
/// # Safety
/// The DTB must remain valid in memory for the lifetime of the kernel.
pub fn init(dtb_addr: usize) {
    DTB_ADDRESS.store(dtb_addr, Ordering::Release);
    
    if dtb_addr != 0 {
        crate::klog::klog_info("dtb", &alloc::format!("DTB at 0x{:x}", dtb_addr));
        
        // Validate and parse the DTB header
        if let Some(size) = validate_dtb(dtb_addr) {
            DTB_SIZE.store(size, Ordering::Relaxed);
            crate::klog::klog_info("dtb", &alloc::format!("DTB valid, size: {} bytes", size));
        } else {
            crate::klog::klog_warning("dtb", "Invalid DTB magic - ignoring");
            DTB_ADDRESS.store(0, Ordering::Release);
        }
    }
}

/// Get the DTB address (0 if none provided or invalid).
pub fn get_address() -> usize {
    DTB_ADDRESS.load(Ordering::Acquire)
}

/// Get the DTB size in bytes (0 if not available).
#[allow(dead_code)]
pub fn get_size() -> usize {
    DTB_SIZE.load(Ordering::Relaxed)
}

/// Check if a valid DTB was provided.
#[allow(dead_code)]
pub fn is_available() -> bool {
    get_address() != 0
}

/// Validate the DTB header and return its size if valid.
fn validate_dtb(dtb_addr: usize) -> Option<usize> {
    // Read the first 8 bytes: magic (4) + totalsize (4)
    let magic = unsafe { 
        let ptr = dtb_addr as *const u32;
        u32::from_be(core::ptr::read_volatile(ptr))
    };
    
    if magic != FDT_MAGIC {
        return None;
    }
    
    let totalsize = unsafe {
        let ptr = (dtb_addr + 4) as *const u32;
        u32::from_be(core::ptr::read_volatile(ptr)) as usize
    };
    
    // Sanity check size (should be reasonable, not too small or huge)
    if totalsize < 48 || totalsize > 1024 * 1024 {
        return None;
    }
    
    Some(totalsize)
}

/// Basic device tree information extracted from header.
pub struct DtbInfo {
    pub address: usize,
    pub size: usize,
    pub version: u32,
}

/// Get basic DTB header information.
#[allow(dead_code)]
pub fn get_info() -> Option<DtbInfo> {
    let addr = get_address();
    if addr == 0 {
        return None;
    }
    
    let size = get_size();
    
    // Read version from header (offset 0x14)
    let version = unsafe {
        let ptr = (addr + 0x14) as *const u32;
        u32::from_be(core::ptr::read_volatile(ptr))
    };
    
    Some(DtbInfo {
        address: addr,
        size,
        version,
    })
}

/// Read a string property from the DTB strings block.
/// This is a low-level function for advanced DTB parsing.
#[allow(dead_code)]
pub fn read_string_at_offset(strings_offset: usize) -> Option<String> {
    let addr = get_address();
    if addr == 0 {
        return None;
    }
    
    // Read strings block offset from header (offset 0x0C)
    let strings_block_off = unsafe {
        let ptr = (addr + 0x0C) as *const u32;
        u32::from_be(core::ptr::read_volatile(ptr)) as usize
    };
    
    // Read string from strings block
    let string_addr = addr + strings_block_off + strings_offset;
    let mut len = 0usize;
    
    // Find null terminator (limit to 256 chars for safety)
    while len < 256 {
        let byte = unsafe { core::ptr::read_volatile((string_addr + len) as *const u8) };
        if byte == 0 {
            break;
        }
        len += 1;
    }
    
    if len == 0 {
        return None;
    }
    
    // Copy string bytes
    let mut bytes = alloc::vec::Vec::with_capacity(len);
    for i in 0..len {
        let byte = unsafe { core::ptr::read_volatile((string_addr + i) as *const u8) };
        bytes.push(byte);
    }
    
    String::from_utf8(bytes).ok()
}
