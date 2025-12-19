//! Filesystem Proxy - Hart-aware filesystem access
//!
//! This module provides transparent filesystem access that works on any hart.
//! On Hart 0: Direct MMIO access via FS_STATE
//! On secondary harts: Delegates to Hart 0 via io_router
//!
//! # Example
//! ```
//! use crate::cpu::fs_proxy;
//!
//! // Works on any hart!
//! if let Some(data) = fs_proxy::fs_read("/usr/bin/hello") {
//!     // Process data...
//! }
//! ```

use alloc::string::String;
use alloc::vec::Vec;

use crate::cpu::io_router::{DeviceType, IoOp, IoRequest, IoResult, request_io};
use crate::lock::utils::{FS_STATE, BLK_DEV};

// Timeout for I/O requests (10 seconds)
const IO_TIMEOUT_MS: u64 = 10000;

// ═══════════════════════════════════════════════════════════════════════════════
// Helper: Submit I/O request to Hart 0
// ═══════════════════════════════════════════════════════════════════════════════

/// Submit an I/O request and wait for the result (blocking).
fn request_io_blocking(device: DeviceType, operation: IoOp) -> IoResult {
    let request = IoRequest::new(device, operation);
    request_io(request, IO_TIMEOUT_MS)
}

// ═══════════════════════════════════════════════════════════════════════════════
// Public API: Hart-aware filesystem functions
// ═══════════════════════════════════════════════════════════════════════════════

/// Read a file from the filesystem.
/// 
/// On Hart 0: Direct access via FS_STATE
/// On secondary harts: Delegates to Hart 0 via io_router
pub fn fs_read(path: &str) -> Option<Vec<u8>> {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        // Direct access on Hart 0
        let mut fs = FS_STATE.write();
        let mut blk = BLK_DEV.write();
        
        if let (Some(fs), Some(dev)) = (fs.as_mut(), blk.as_mut()) {
            fs.read_file(dev, path)
        } else {
            None
        }
    } else {
        // Delegate to Hart 0 via io_router
        let op = IoOp::FsRead { path: String::from(path) };
        let result = request_io_blocking(DeviceType::Mmc, op);
        
        match result {
            IoResult::Ok(data) => Some(data),
            IoResult::Err(_) => None,
        }
    }
}

/// Write data to a file.
///
/// On Hart 0: Direct access via FS_STATE
/// On secondary harts: Delegates to Hart 0 via io_router
pub fn fs_write(path: &str, data: &[u8]) -> Result<(), &'static str> {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        // Direct access on Hart 0
        let mut fs = FS_STATE.write();
        let mut blk = BLK_DEV.write();
        
        if let (Some(fs), Some(dev)) = (fs.as_mut(), blk.as_mut()) {
            fs.write_file(dev, path, data)
        } else {
            Err("Filesystem not available")
        }
    } else {
        // Delegate to Hart 0
        let op = IoOp::FsWrite { 
            path: String::from(path), 
            data: data.to_vec() 
        };
        let result = request_io_blocking(DeviceType::Mmc, op);
        
        match result {
            IoResult::Ok(_) => Ok(()),
            IoResult::Err(e) => Err(e),
        }
    }
}

/// File info returned by fs_list
#[derive(Clone, Debug)]
pub struct FileInfo {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// List directory contents.
///
/// On Hart 0: Direct access via FS_STATE
/// On secondary harts: Delegates to Hart 0 via io_router
pub fn fs_list(path: &str) -> Vec<FileInfo> {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        // Direct access on Hart 0
        let mut fs = FS_STATE.write();
        let mut blk = BLK_DEV.write();
        
        if let (Some(fs), Some(dev)) = (fs.as_mut(), blk.as_mut()) {
            fs.list_dir(dev, path)
                .into_iter()
                .map(|e| FileInfo {
                    name: e.name,
                    is_dir: e.is_dir,
                    size: e.size as u64,
                })
                .collect()
        } else {
            Vec::new()
        }
    } else {
        // Delegate to Hart 0
        let op = IoOp::FsList { path: String::from(path) };
        let result = request_io_blocking(DeviceType::Mmc, op);
        
        match result {
            IoResult::Ok(data) => {
                // Parse newline-separated paths
                let text = core::str::from_utf8(&data).unwrap_or("");
                text.lines()
                    .filter(|s| !s.is_empty())
                    .map(|name| FileInfo {
                        name: String::from(name),
                        is_dir: name.ends_with('/'),
                        size: 0, // Size not available via this method
                    })
                    .collect()
            }
            IoResult::Err(_) => Vec::new(),
        }
    }
}

/// Check if a file exists.
///
/// On Hart 0: Direct access via FS_STATE
/// On secondary harts: Delegates to Hart 0 via io_router
pub fn fs_exists(path: &str) -> bool {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        // Direct access on Hart 0
        let mut fs = FS_STATE.write();
        let mut blk = BLK_DEV.write();
        
        if let (Some(fs), Some(dev)) = (fs.as_mut(), blk.as_mut()) {
            fs.read_file(dev, path).is_some()
        } else {
            false
        }
    } else {
        // Delegate to Hart 0
        let op = IoOp::FsExists { path: String::from(path) };
        let result = request_io_blocking(DeviceType::Mmc, op);
        
        match result {
            IoResult::Ok(data) => data.first() == Some(&1),
            IoResult::Err(_) => false,
        }
    }
}

/// Sync filesystem to disk.
///
/// On Hart 0: Direct access via FS_STATE
/// On secondary harts: Delegates to Hart 0 via io_router
pub fn fs_sync() -> Result<(), &'static str> {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        // Direct access on Hart 0
        let mut fs = FS_STATE.write();
        let mut blk = BLK_DEV.write();
        
        if let (Some(fs), Some(dev)) = (fs.as_mut(), blk.as_mut()) {
            fs.sync(dev).map(|_| ())
        } else {
            Err("Filesystem not available")
        }
    } else {
        // Delegate to Hart 0
        let result = request_io_blocking(DeviceType::Mmc, IoOp::FsSync);
        
        match result {
            IoResult::Ok(_) => Ok(()),
            IoResult::Err(e) => Err(e),
        }
    }
}
