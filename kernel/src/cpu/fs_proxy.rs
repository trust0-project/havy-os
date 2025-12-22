//! Filesystem Proxy - Hart-aware filesystem access
//!
//! This module provides transparent filesystem access that works on any hart.
//! On Hart 0: Direct access via VFS_STATE
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
use crate::lock::utils::{VFS_STATE, FS_STATE, BLK_DEV};

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
// Internal: Try VFS first, fall back to legacy FS_STATE
// ═══════════════════════════════════════════════════════════════════════════════

/// Read using VFS if available, otherwise fall back to legacy FS_STATE
fn read_with_vfs_or_legacy(path: &str) -> Option<Vec<u8>> {
    // Try VFS first
    let mut vfs = VFS_STATE.write();
    if let Some(vfs) = vfs.as_mut() {
        return vfs.read_file(path);
    }
    drop(vfs);
    
    // Fall back to legacy FS_STATE
    let mut fs = FS_STATE.write();
    let mut blk = BLK_DEV.write();
    if let (Some(fs), Some(dev)) = (fs.as_mut(), blk.as_mut()) {
        fs.read_file(dev, path)
    } else {
        None
    }
}

/// Write using VFS if available, otherwise fall back to legacy FS_STATE
fn write_with_vfs_or_legacy(path: &str, data: &[u8]) -> Result<(), &'static str> {
    // Try VFS first
    let mut vfs = VFS_STATE.write();
    if let Some(vfs) = vfs.as_mut() {
        return vfs.write_file(path, data);
    }
    drop(vfs);
    
    // Fall back to legacy FS_STATE
    let mut fs = FS_STATE.write();
    let mut blk = BLK_DEV.write();
    if let (Some(fs), Some(dev)) = (fs.as_mut(), blk.as_mut()) {
        fs.write_file(dev, path, data)
    } else {
        Err("Filesystem not available")
    }
}

/// List using VFS if available, otherwise fall back to legacy FS_STATE
fn list_with_vfs_or_legacy(path: &str) -> Vec<FileInfo> {
    // Try VFS first
    let mut vfs = VFS_STATE.write();
    if let Some(vfs) = vfs.as_mut() {
        return vfs.list_dir(path)
            .into_iter()
            .map(|e| FileInfo {
                name: e.name,
                is_dir: e.is_dir,
                size: e.size as u64,
            })
            .collect();
    }
    drop(vfs);
    
    // Fall back to legacy FS_STATE
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
}

/// Check exists using VFS if available, otherwise fall back to legacy FS_STATE
fn exists_with_vfs_or_legacy(path: &str) -> bool {
    // Try VFS first
    let mut vfs = VFS_STATE.write();
    if let Some(vfs) = vfs.as_mut() {
        return vfs.exists(path);
    }
    drop(vfs);
    
    // Fall back to legacy FS_STATE
    let mut fs = FS_STATE.write();
    let mut blk = BLK_DEV.write();
    if let (Some(fs), Some(dev)) = (fs.as_mut(), blk.as_mut()) {
        fs.read_file(dev, path).is_some()
    } else {
        false
    }
}

/// Sync using VFS if available, otherwise fall back to legacy FS_STATE
fn sync_with_vfs_or_legacy() -> Result<(), &'static str> {
    // Try VFS first
    let mut vfs = VFS_STATE.write();
    if let Some(vfs) = vfs.as_mut() {
        return vfs.sync().map(|_| ());
    }
    drop(vfs);
    
    // Fall back to legacy FS_STATE
    let mut fs = FS_STATE.write();
    let mut blk = BLK_DEV.write();
    if let (Some(fs), Some(dev)) = (fs.as_mut(), blk.as_mut()) {
        fs.sync(dev).map(|_| ())
    } else {
        Err("Filesystem not available")
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Public API: Hart-aware filesystem functions
// ═══════════════════════════════════════════════════════════════════════════════

/// Read a file from the filesystem.
/// 
/// On Hart 0: Direct access via VFS_STATE (or legacy FS_STATE)
/// On secondary harts: Delegates to Hart 0 via io_router
pub fn fs_read(path: &str) -> Option<Vec<u8>> {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        read_with_vfs_or_legacy(path)
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
/// On Hart 0: Direct access via VFS_STATE (or legacy FS_STATE)
/// On secondary harts: Delegates to Hart 0 via io_router
pub fn fs_write(path: &str, data: &[u8]) -> Result<(), &'static str> {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        write_with_vfs_or_legacy(path, data)
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
/// On Hart 0: Direct access via VFS_STATE (or legacy FS_STATE)
/// On secondary harts: Delegates to Hart 0 via io_router
pub fn fs_list(path: &str) -> Vec<FileInfo> {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        list_with_vfs_or_legacy(path)
    } else {
        // Delegate to Hart 0
        let op = IoOp::FsList { path: String::from(path) };
        let result = request_io_blocking(DeviceType::Mmc, op);
        
        match result {
            IoResult::Ok(data) => {
                // Parse newline-separated "name:size" entries
                let text = core::str::from_utf8(&data).unwrap_or("");
                text.lines()
                    .filter(|s| !s.is_empty())
                    .filter_map(|line| {
                        // Parse "name:size" format
                        if let Some(colon_pos) = line.rfind(':') {
                            let name = &line[..colon_pos];
                            let size_str = &line[colon_pos + 1..];
                            let size = size_str.parse::<u64>().unwrap_or(0);
                            Some(FileInfo {
                                name: String::from(name),
                                is_dir: name.ends_with('/'),
                                size,
                            })
                        } else {
                            // No colon - treat whole line as name
                            Some(FileInfo {
                                name: String::from(line),
                                is_dir: line.ends_with('/'),
                                size: 0,
                            })
                        }
                    })
                    .collect()
            }
            IoResult::Err(_) => Vec::new(),
        }
    }
}

/// Check if a file exists.
///
/// On Hart 0: Direct access via VFS_STATE (or legacy FS_STATE)
/// On secondary harts: Delegates to Hart 0 via io_router
pub fn fs_exists(path: &str) -> bool {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        exists_with_vfs_or_legacy(path)
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
/// On Hart 0: Direct access via VFS_STATE (or legacy FS_STATE)
/// On secondary harts: Delegates to Hart 0 via io_router
pub fn fs_sync() -> Result<(), &'static str> {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        sync_with_vfs_or_legacy()
    } else {
        // Delegate to Hart 0
        let result = request_io_blocking(DeviceType::Mmc, IoOp::FsSync);
        
        match result {
            IoResult::Ok(_) => Ok(()),
            IoResult::Err(e) => Err(e),
        }
    }
}
