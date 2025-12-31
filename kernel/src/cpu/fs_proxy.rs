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

use crate::cpu::io_router::{
    DeviceType, IoOp, IoRequest, IoResult, RequestId,
    request_io, request_io_async, poll_io, is_io_complete,
};
use crate::lock::utils::{VFS_STATE, FS_STATE, BLK_DEV};

// Timeout for I/O requests (10 seconds)
const IO_TIMEOUT_MS: u64 = 30000;

// ═══════════════════════════════════════════════════════════════════════════════
// Async I/O Future Type
// ═══════════════════════════════════════════════════════════════════════════════

/// A future representing a pending I/O operation.
/// 
/// Use `poll()` to check for completion, or `is_complete()` to check without
/// consuming the result.
/// 
/// # Example
/// ```
/// let future = fs_read_async("/usr/bin/hello");
/// // Do other work...
/// if future.is_complete() {
///     let result = future.poll();
/// }
/// ```
pub struct IoFuture {
    request_id: RequestId,
}

impl IoFuture {
    /// Create a new I/O future for the given request ID
    pub fn new(request_id: RequestId) -> Self {
        Self { request_id }
    }
    
    /// Get the request ID for this future
    pub fn request_id(&self) -> RequestId {
        self.request_id
    }
    
    /// Poll for completion.
    /// Returns `Some(result)` if complete, `None` if still pending.
    /// 
    /// Note: This consumes the result - subsequent calls will return None.
    pub fn poll(&self) -> Option<IoResult> {
        poll_io(self.request_id)
    }
    
    /// Check if the I/O operation is complete without consuming the result.
    pub fn is_complete(&self) -> bool {
        is_io_complete(self.request_id)
    }
    
    /// Block until the I/O operation completes (with timeout).
    /// 
    /// This is a convenience method for code transitioning from blocking I/O.
    pub fn wait(&self, timeout_ms: u64) -> IoResult {
        use core::arch::asm;
        let start = crate::get_time_ms();
        
        loop {
            if let Some(result) = self.poll() {
                return result;
            }
            
            if timeout_ms > 0 {
                let elapsed = crate::get_time_ms() - start;
                if elapsed >= timeout_ms as i64 {
                    return IoResult::Err("I/O future timeout");
                }
            }
            
            // Yield CPU
            unsafe { asm!("wfi", options(nomem, nostack)); }
        }
    }
}

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
    use crate::device::uart::write_str;
    
    // Try VFS first
    let mut vfs = VFS_STATE.write();
    if let Some(vfs) = vfs.as_mut() {
        let result = vfs.read_file(path);
        return result;
    }
    drop(vfs);
    
    // Fall back to legacy FS_STATE
    let mut fs = FS_STATE.write();
    let mut blk = BLK_DEV.write();
    if let (Some(fs), Some(dev)) = (fs.as_mut(), blk.as_mut()) {
        let result = fs.read_file(dev, path);
        result
    } else {
        None
    }
}

/// Write using VFS if available, otherwise fall back to legacy FS_STATE
/// Uses non-blocking try_write to avoid deadlocks across harts
fn write_with_vfs_or_legacy(path: &str, data: &[u8]) -> Result<(), &'static str> {
    use crate::device::uart::{write_str, write_line};
    use core::arch::asm;
    
    let start = crate::get_time_ms();
    let timeout_ms = 5000; // 5 second timeout
    
    loop {
        // Try VFS first with non-blocking lock
        if let Some(mut vfs_guard) = VFS_STATE.try_write() {
            write_line("Got VFS lock");
            if let Some(vfs) = vfs_guard.as_mut() {
                write_line("Calling vfs.write_file...");
                let result = vfs.write_file(path, data);
                write_line("vfs.write_file returned");
                if result.is_err() {
                    write_str("VFS write_file error for: ");
                    write_line(path);
                }
                return result;
            }
            drop(vfs_guard);
            
            // VFS not initialized, try legacy FS_STATE
            if let Some(mut fs_guard) = FS_STATE.try_write() {
                if let Some(mut blk_guard) = BLK_DEV.try_write() {
                    if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
                        let result = fs.write_file(dev, path, data);
                        if result.is_err() {
                            write_str("Legacy FS write_file error for: ");
                            write_line(path);
                        }
                        return result;
                    }
                }
            }
            
            write_line("FS not available for write");
            return Err("Filesystem not available");
        }
        
        // Check timeout
        let elapsed = crate::get_time_ms() - start;
        if elapsed >= timeout_ms as i64 {
            write_line("fs_write: lock timeout");
            return Err("Lock timeout");
        }
        
        // Yield CPU briefly
        unsafe { asm!("wfi", options(nomem, nostack)); }
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
/// (Secondary harts in WASM don't have access to D1 MMC device)
pub fn fs_write(path: &str, data: &[u8]) -> Result<(), &'static str> {
    use crate::device::uart::{write_str, write_line};
    
    let hart_id = crate::get_hart_id();
    
    write_str("fs_write on hart ");
    write_hex(hart_id as u64);
    write_line("");
    
    if hart_id == 0 {
        write_with_vfs_or_legacy(path, data)
    } else {
        // Delegate to Hart 0 via io_router - secondary harts don't have D1 MMC
        write_line("fs_write: delegating to hart 0");
        let op = IoOp::FsWrite { 
            path: String::from(path), 
            data: data.to_vec() 
        };
        let result = request_io_blocking(DeviceType::Mmc, op);
        
        match result {
            IoResult::Ok(_) => {
                write_line("fs_write: delegation success");
                Ok(())
            }
            IoResult::Err(e) => {
                write_str("fs_write: delegation failed - ");
                write_line(e);
                Err(e)
            }
        }
    }
}

fn write_hex(val: u64) {
    use crate::device::uart::write_str;
    let mut buf = [0u8; 18];
    buf[0] = b'0';
    buf[1] = b'x';
    let hex_chars = b"0123456789abcdef";
    for i in 0..16 {
        let nibble = ((val >> (60 - i * 4)) & 0xF) as usize;
        buf[2 + i] = hex_chars[nibble];
    }
    if let Ok(s) = core::str::from_utf8(&buf[..18]) {
        write_str(s);
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

// ═══════════════════════════════════════════════════════════════════════════════
// Async Filesystem API
// ═══════════════════════════════════════════════════════════════════════════════

/// Read a file asynchronously (non-blocking).
///
/// Returns immediately with an IoFuture that can be polled for completion.
/// This allows the calling hart to do other work while waiting for the I/O.
///
/// # Example
/// ```
/// let future = fs_read_async("/usr/bin/hello");
/// // Do other work while I/O is pending...
/// if future.is_complete() {
///     if let Some(IoResult::Ok(data)) = future.poll() {
///         // Process data
///     }
/// }
/// ```
pub fn fs_read_async(path: &str) -> IoFuture {
    let op = IoOp::FsRead { path: String::from(path) };
    let request = IoRequest::new(DeviceType::Mmc, op);
    let request_id = request_io_async(request);
    IoFuture::new(request_id)
}

/// Write data to a file asynchronously (non-blocking).
///
/// Returns immediately with an IoFuture that can be polled for completion.
pub fn fs_write_async(path: &str, data: &[u8]) -> IoFuture {
    let op = IoOp::FsWrite { 
        path: String::from(path), 
        data: data.to_vec() 
    };
    let request = IoRequest::new(DeviceType::Mmc, op);
    let request_id = request_io_async(request);
    IoFuture::new(request_id)
}

/// List directory contents asynchronously (non-blocking).
///
/// Returns immediately with an IoFuture that can be polled for completion.
/// The result will be a serialized list that needs to be parsed.
pub fn fs_list_async(path: &str) -> IoFuture {
    let op = IoOp::FsList { path: String::from(path) };
    let request = IoRequest::new(DeviceType::Mmc, op);
    let request_id = request_io_async(request);
    IoFuture::new(request_id)
}

/// Check if a file exists asynchronously (non-blocking).
///
/// Returns immediately with an IoFuture that can be polled for completion.
pub fn fs_exists_async(path: &str) -> IoFuture {
    let op = IoOp::FsExists { path: String::from(path) };
    let request = IoRequest::new(DeviceType::Mmc, op);
    let request_id = request_io_async(request);
    IoFuture::new(request_id)
}

/// Sync filesystem asynchronously (non-blocking).
///
/// Returns immediately with an IoFuture that can be polled for completion.
pub fn fs_sync_async() -> IoFuture {
    let request = IoRequest::new(DeviceType::Mmc, IoOp::FsSync);
    let request_id = request_io_async(request);
    IoFuture::new(request_id)
}
