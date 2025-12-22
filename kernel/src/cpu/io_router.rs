//! I/O Request Routing for Multi-Hart SMP
//!
//! Secondary harts in the emulator cannot access MMIO devices directly.
//! This module provides a request/response pattern to delegate I/O operations
//! to Hart 0, which has exclusive access to hardware devices.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────┐     ┌─────────────┐     ┌─────────────────┐
//! │ Secondary Hart  │────>│  IO_QUEUE   │────>│  Hart 0         │
//! │ (request_io)    │     │  (shared)   │     │  (dispatch_io)  │
//! └─────────────────┘     └─────────────┘     └─────────────────┘
//!          │                                           │
//!          │              ┌─────────────┐              │
//!          └──────────────│ IO_RESULTS  │<─────────────┘
//!           (poll/wait)   │  (shared)   │   (store result)
//!                         └─────────────┘
//! ```
//!
//! ## Device Ownership (from spec)
//!
//! | Device          | Hart 0       | Secondary Harts    |
//! |-----------------|--------------|-------------------|
//! | D1 MMC (SD)     | Direct MMIO  | Via this module   |
//! | D1 Display      | Direct MMIO  | Via this module   |
//! | D1 EMAC (Net)   | Direct MMIO  | Via this module   |
//! | VirtIO Disk/Net | Direct MMIO  | Via this module   |
//! | UART            | Direct MMIO  | Shared buffer     |
//! | CLINT           | Shared       | Shared            |

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::Spinlock;
use crate::cpu::get_hart_id;

/// Maximum number of pending I/O requests
const MAX_PENDING_REQUESTS: usize = 64;

// ═══════════════════════════════════════════════════════════════════════════════
// TYPES
// ═══════════════════════════════════════════════════════════════════════════════

/// Device types that can be accessed via I/O routing
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceType {
    /// SD Card / Block device (D1 MMC)
    Mmc,
    /// Network interface (D1 EMAC)
    Network,
    /// GPU framebuffer / Display
    Display,
    /// Serial console (UART) - note: UART has shared output buffer
    Uart,
    /// VirtIO block device
    VirtioBlock,
    /// VirtIO network device
    VirtioNet,
    /// Audio codec (D1 Audio)
    Audio,
}

impl DeviceType {
    /// Get a human-readable name for this device type
    pub fn as_str(&self) -> &'static str {
        match self {
            DeviceType::Mmc => "mmc",
            DeviceType::Network => "network",
            DeviceType::Display => "display",
            DeviceType::Uart => "uart",
            DeviceType::VirtioBlock => "virtio-blk",
            DeviceType::VirtioNet => "virtio-net",
            DeviceType::Audio => "audio",
        }
    }
}

/// I/O operation types
#[derive(Clone, Debug)]
pub enum IoOp {
    /// Read data from device at offset
    Read { offset: u64, len: usize },
    /// Write data to device at offset
    Write { offset: u64, data: Vec<u8> },
    /// Device-specific control operation
    Ioctl { cmd: u32, arg: u64 },
    /// Flush pending writes to device
    Flush,
    /// Query device status/capabilities
    Status,
    
    // ═══════════════════════════════════════════════════════════════════════
    // Filesystem operations (for fs_proxy)
    // ═══════════════════════════════════════════════════════════════════════
    
    /// Read a file from the filesystem
    FsRead { path: alloc::string::String },
    /// Write data to a file
    FsWrite { path: alloc::string::String, data: Vec<u8> },
    /// List directory contents
    FsList { path: alloc::string::String },
    /// Check if file exists
    FsExists { path: alloc::string::String },
    /// Sync filesystem to disk
    FsSync,
    
    // ═══════════════════════════════════════════════════════════════════════
    // Display operations (for display_proxy)
    // ═══════════════════════════════════════════════════════════════════════
    
    /// Flush display (copy dirty region from back buffer to front buffer)
    DisplayFlush,
    /// Clear display to black
    DisplayClear,
    /// Mark entire screen as dirty
    DisplayMarkAllDirty,
    /// Check if display is available
    DisplayIsAvailable,
    
    // ═══════════════════════════════════════════════════════════════════════
    // Touch operations (for display_proxy)
    // ═══════════════════════════════════════════════════════════════════════
    
    /// Poll for touch events
    TouchPoll,
    /// Get next touch event (serialized as [type:2][code:2][value:4])
    TouchNextEvent,
    /// Check if touch events are pending
    TouchHasEvents,
    
    // ═══════════════════════════════════════════════════════════════════════
    // Network operations (for net_proxy)
    // ═══════════════════════════════════════════════════════════════════════
    
    /// Poll the network stack
    NetPoll { timestamp_ms: i64 },
    /// Check if IP is assigned
    NetIsIpAssigned,
    /// Get assigned IP address (returns 4 bytes)
    NetGetIp,
    
    // ═══════════════════════════════════════════════════════════════════════
    // Audio operations (for audio_proxy)
    // ═══════════════════════════════════════════════════════════════════════
    
    /// Write an audio sample to the FIFO (32-bit stereo: L[15:0], R[31:16])
    AudioWriteSample { sample: u32 },
    /// Enable or disable audio playback
    AudioSetEnabled { enabled: bool },
    /// Set sample rate in Hz (e.g., 48000)
    AudioSetSampleRate { rate: u32 },
    /// Get current buffer fill level
    AudioGetBufferLevel,
    /// Check if buffer is full
    AudioIsBufferFull,
    /// Check if buffer is empty
    AudioIsBufferEmpty,
}

/// Request ID type
pub type RequestId = u64;

/// I/O request from a secondary hart to Hart 0
#[derive(Clone)]
pub struct IoRequest {
    /// Unique request ID (for matching responses)
    pub request_id: RequestId,
    /// Hart that submitted this request
    pub source_hart: usize,
    /// Target device
    pub device: DeviceType,
    /// Operation to perform
    pub operation: IoOp,
}

impl IoRequest {
    /// Create a new I/O request
    pub fn new(device: DeviceType, operation: IoOp) -> Self {
        static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
        
        Self {
            request_id: NEXT_REQUEST_ID.fetch_add(1, Ordering::SeqCst),
            source_hart: get_hart_id(),
            device,
            operation,
        }
    }
    
    /// Create a read request
    pub fn read(device: DeviceType, offset: u64, len: usize) -> Self {
        Self::new(device, IoOp::Read { offset, len })
    }
    
    /// Create a write request
    pub fn write(device: DeviceType, offset: u64, data: Vec<u8>) -> Self {
        Self::new(device, IoOp::Write { offset, data })
    }
    
    /// Create an ioctl request
    pub fn ioctl(device: DeviceType, cmd: u32, arg: u64) -> Self {
        Self::new(device, IoOp::Ioctl { cmd, arg })
    }
}

/// Result of an I/O operation
#[derive(Clone, Debug)]
pub enum IoResult {
    /// Operation completed successfully with optional data
    Ok(Vec<u8>),
    /// Operation failed with error message
    Err(&'static str),
}

impl IoResult {
    /// Check if the result is Ok
    pub fn is_ok(&self) -> bool {
        matches!(self, IoResult::Ok(_))
    }
    
    /// Check if the result is Err
    pub fn is_err(&self) -> bool {
        matches!(self, IoResult::Err(_))
    }
    
    /// Get the data if Ok, or None if Err
    pub fn data(&self) -> Option<&[u8]> {
        match self {
            IoResult::Ok(data) => Some(data),
            IoResult::Err(_) => None,
        }
    }
    
    /// Get the error message if Err, or None if Ok
    pub fn error(&self) -> Option<&'static str> {
        match self {
            IoResult::Ok(_) => None,
            IoResult::Err(e) => Some(e),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// GLOBAL STATE
// ═══════════════════════════════════════════════════════════════════════════════

/// Completion slot for a single request
pub struct CompletionSlot {
    /// Whether this slot has a completed result
    complete: AtomicBool,
    /// The result (protected by complete flag)
    result: Spinlock<Option<IoResult>>,
}

impl CompletionSlot {
    pub const fn new() -> Self {
        Self {
            complete: AtomicBool::new(false),
            result: Spinlock::new(None),
        }
    }
    
    /// Store a result in this slot
    pub fn store(&self, result: IoResult) {
        *self.result.lock() = Some(result);
        self.complete.store(true, Ordering::Release);
    }
    
    /// Check if result is ready
    pub fn is_complete(&self) -> bool {
        self.complete.load(Ordering::Acquire)
    }
    
    /// Take the result (returns None if not complete or already taken)
    pub fn take(&self) -> Option<IoResult> {
        if self.complete.load(Ordering::Acquire) {
            let result = self.result.lock().take();
            if result.is_some() {
                self.complete.store(false, Ordering::Release);
            }
            result
        } else {
            None
        }
    }
    
    /// Reset this slot for reuse
    pub fn reset(&self) {
        self.complete.store(false, Ordering::Release);
        *self.result.lock() = None;
    }
}

/// Global I/O request queue
pub static IO_QUEUE: Spinlock<VecDeque<IoRequest>> = Spinlock::new(VecDeque::new());

/// Completion slots indexed by (request_id % MAX_PENDING_REQUESTS)
static IO_COMPLETIONS: [CompletionSlot; MAX_PENDING_REQUESTS] = {
    const INIT: CompletionSlot = CompletionSlot::new();
    [INIT; MAX_PENDING_REQUESTS]
};

/// Statistics
static REQUESTS_SUBMITTED: AtomicU64 = AtomicU64::new(0);
static REQUESTS_COMPLETED: AtomicU64 = AtomicU64::new(0);

// ═══════════════════════════════════════════════════════════════════════════════
// API FOR SECONDARY HARTS
// ═══════════════════════════════════════════════════════════════════════════════

/// Submit an I/O request and wait for completion.
///
/// This function is called by secondary harts to request I/O from Hart 0.
/// It blocks until the request is completed or times out.
///
/// # Arguments
/// * `request` - The I/O request to submit
/// * `timeout_ms` - Maximum time to wait in milliseconds (0 = no timeout)
///
/// # Returns
/// * `IoResult::Ok(data)` - Operation succeeded with returned data
/// * `IoResult::Err(msg)` - Operation failed with error message
pub fn request_io(request: IoRequest, timeout_ms: u64) -> IoResult {
    let request_id = request.request_id;
    let slot_idx = (request_id as usize) % MAX_PENDING_REQUESTS;
    
    // Reset the completion slot
    IO_COMPLETIONS[slot_idx].reset();
    
    // Submit request to queue
    IO_QUEUE.lock().push_back(request);
    REQUESTS_SUBMITTED.fetch_add(1, Ordering::Relaxed);
    
    // Send IPI to Hart 0 to wake it up
    if get_hart_id() != 0 {
        crate::cpu::send_ipi(0);
    }
    
    // Wait for completion
    let start = crate::get_time_ms();
    loop {
        // Check if complete
        if let Some(result) = IO_COMPLETIONS[slot_idx].take() {
            return result;
        }
        
        // Check timeout
        if timeout_ms > 0 {
            let elapsed = crate::get_time_ms() - start;
            if elapsed >= timeout_ms as i64 {
                return IoResult::Err("I/O request timeout");
            }
        }
        
        // Yield CPU (use WFI to save power)
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

/// Submit an I/O request without waiting (fire-and-forget).
///
/// Useful for operations where the caller doesn't need the result.
///
/// # Returns
/// * Request ID that can be used to poll for completion later
pub fn request_io_async(request: IoRequest) -> RequestId {
    let request_id = request.request_id;
    let slot_idx = (request_id as usize) % MAX_PENDING_REQUESTS;
    
    // Reset the completion slot
    IO_COMPLETIONS[slot_idx].reset();
    
    // Submit request to queue
    IO_QUEUE.lock().push_back(request);
    REQUESTS_SUBMITTED.fetch_add(1, Ordering::Relaxed);
    
    // Send IPI to Hart 0
    if get_hart_id() != 0 {
        crate::cpu::send_ipi(0);
    }
    
    request_id
}

/// Check if an async request has completed.
///
/// # Returns
/// * `Some(result)` if the request is complete
/// * `None` if the request is still pending
pub fn poll_io(request_id: RequestId) -> Option<IoResult> {
    let slot_idx = (request_id as usize) % MAX_PENDING_REQUESTS;
    IO_COMPLETIONS[slot_idx].take()
}

// ═══════════════════════════════════════════════════════════════════════════════
// API FOR HART 0 (I/O DISPATCHER)
// ═══════════════════════════════════════════════════════════════════════════════

/// Check if there are pending I/O requests.
pub fn has_pending_requests() -> bool {
    !IO_QUEUE.lock().is_empty()
}

/// Get the number of pending requests.
pub fn pending_count() -> usize {
    IO_QUEUE.lock().len()
}

/// Dequeue the next I/O request for processing.
///
/// Called by Hart 0's I/O dispatcher to get the next request.
pub fn dequeue_request() -> Option<IoRequest> {
    IO_QUEUE.lock().pop_front()
}

/// Complete an I/O request with a result.
///
/// Called by Hart 0 after processing a request.
pub fn complete_request(request_id: RequestId, result: IoResult) {
    let slot_idx = (request_id as usize) % MAX_PENDING_REQUESTS;
    IO_COMPLETIONS[slot_idx].store(result);
    REQUESTS_COMPLETED.fetch_add(1, Ordering::Relaxed);
}

/// Process all pending I/O requests.
///
/// This is the main dispatcher function called by Hart 0.
/// It dequeues requests and routes them to the appropriate device handlers.
///
/// # Returns
/// Number of requests processed
pub fn dispatch_io() -> usize {
    let hart_id = get_hart_id();
    
    // Only Hart 0 should run the dispatcher
    if hart_id != 0 {
        return 0;
    }
    
    let mut processed = 0;
    
    while let Some(request) = dequeue_request() {
        let result = handle_request(&request);
        complete_request(request.request_id, result);
        
        // Wake the requesting hart
        if request.source_hart != 0 {
            crate::cpu::send_ipi(request.source_hart);
        }
        
        processed += 1;
    }
    
    processed
}

/// Handle a single I/O request by routing to the appropriate device.
fn handle_request(request: &IoRequest) -> IoResult {
    match request.device {
        DeviceType::Mmc | DeviceType::VirtioBlock => handle_block_request(request),
        DeviceType::Network | DeviceType::VirtioNet => handle_network_request(request),
        DeviceType::Display => handle_display_request(request),
        DeviceType::Uart => handle_uart_request(request),
        DeviceType::Audio => handle_audio_request(request),
    }
}

/// Handle block device (MMC/VirtIO) requests
fn handle_block_request(request: &IoRequest) -> IoResult {
    match &request.operation {
        IoOp::Read { offset, len } => {
            // Route to block device driver
            let fs = crate::lock::utils::FS_STATE.read();
            let blk = crate::lock::utils::BLK_DEV.read();
            
            if let (Some(_fs), Some(_dev)) = (fs.as_ref(), blk.as_ref()) {
                // Note: Direct block reads would go here
                // For now, return placeholder - actual implementation depends on FS API
                let _ = (offset, len);
                IoResult::Err("Block read not implemented via I/O router")
            } else {
                IoResult::Err("Block device not available")
            }
        }
        IoOp::Write { offset, data } => {
            let _ = (offset, data);
            IoResult::Err("Block write not implemented via I/O router")
        }
        IoOp::Flush => {
            let mut fs = crate::lock::utils::FS_STATE.write();
            let mut blk = crate::lock::utils::BLK_DEV.write();
            
            if let (Some(fs), Some(dev)) = (fs.as_mut(), blk.as_mut()) {
                match fs.sync(dev) {
                    Ok(_) => IoResult::Ok(Vec::new()),
                    Err(e) => IoResult::Err(e),
                }
            } else {
                IoResult::Err("Block device not available")
            }
        }
        IoOp::Status => {
            let blk = crate::lock::utils::BLK_DEV.read();
            if blk.is_some() {
                IoResult::Ok(b"online".to_vec())
            } else {
                IoResult::Ok(b"offline".to_vec())
            }
        }
        IoOp::Ioctl { cmd, arg } => {
            let _ = (cmd, arg);
            IoResult::Err("Block ioctl not implemented")
        }
        
        // ═══════════════════════════════════════════════════════════════════════
        // Filesystem operations
        // ═══════════════════════════════════════════════════════════════════════
        
        IoOp::FsRead { path } => {
            // Check if path is under a VFS mount point (9P filesystem)
            if path.starts_with("/mnt/") {
                // Use VFS for mounted filesystems
                let mut vfs_guard = crate::lock::utils::VFS_STATE.write();
                if let Some(vfs) = vfs_guard.as_mut() {
                    let result = vfs.read_file(path);
                    drop(vfs_guard);
                    match result {
                        Some(data) => IoResult::Ok(data),
                        None => IoResult::Err("File not found"),
                    }
                } else {
                    IoResult::Err("VFS not available")
                }
            } else {
                // Use FS_STATE directly for root filesystem (SFS)
                let mut fs = crate::lock::utils::FS_STATE.write();
                let mut blk = crate::lock::utils::BLK_DEV.write();
                
                if let (Some(fs), Some(dev)) = (fs.as_mut(), blk.as_mut()) {
                    match fs.read_file(dev, path) {
                        Some(data) => IoResult::Ok(data),
                        None => IoResult::Err("File not found"),
                    }
                } else {
                    IoResult::Err("Filesystem not available")
                }
            }
        }
        
        IoOp::FsWrite { path, data } => {
            // Use VFS for mount point routing
            let mut vfs_guard = crate::lock::utils::VFS_STATE.write();
            if let Some(vfs) = vfs_guard.as_mut() {
                match vfs.write_file(path, data) {
                    Ok(()) => IoResult::Ok(Vec::new()),
                    Err(e) => IoResult::Err(e),
                }
            } else {
                drop(vfs_guard);
                // Fallback to legacy FS_STATE
                let mut fs = crate::lock::utils::FS_STATE.write();
                let mut blk = crate::lock::utils::BLK_DEV.write();
                
                if let (Some(fs), Some(dev)) = (fs.as_mut(), blk.as_mut()) {
                    match fs.write_file(dev, path, data) {
                        Ok(()) => IoResult::Ok(Vec::new()),
                        Err(e) => IoResult::Err(e),
                    }
                } else {
                    IoResult::Err("Filesystem not available")
                }
            }
        }
        
        IoOp::FsList { path } => {
            // Use VFS for mount point visibility
            let mut vfs_guard = crate::lock::utils::VFS_STATE.write();
            if let Some(vfs) = vfs_guard.as_mut() {
                let entries = vfs.list_dir(path);
                // Serialize entries as "name:size\n" format (compatible with ls binary)
                let mut result = Vec::new();
                for entry in entries {
                    result.extend_from_slice(entry.name.as_bytes());
                    result.push(b':');
                    // Format size as string
                    let size_str = alloc::format!("{}", entry.size);
                    result.extend_from_slice(size_str.as_bytes());
                    result.push(b'\n');
                }
                IoResult::Ok(result)
            } else {
                // Fall back to legacy FS_STATE
                let mut fs = crate::lock::utils::FS_STATE.write();
                let mut blk = crate::lock::utils::BLK_DEV.write();
                
                if let (Some(fs), Some(dev)) = (fs.as_mut(), blk.as_mut()) {
                    let entries = fs.list_dir(dev, path);
                    let mut result = Vec::new();
                    for entry in entries {
                        result.extend_from_slice(entry.name.as_bytes());
                        result.push(b':');
                        let size_str = alloc::format!("{}", entry.size);
                        result.extend_from_slice(size_str.as_bytes());
                        result.push(b'\n');
                    }
                    IoResult::Ok(result)
                } else {
                    IoResult::Err("Filesystem not available")
                }
            }
        }
        
        IoOp::FsExists { path } => {
            let mut fs = crate::lock::utils::FS_STATE.write();
            let mut blk = crate::lock::utils::BLK_DEV.write();
            
            if let (Some(fs), Some(dev)) = (fs.as_mut(), blk.as_mut()) {
                // Try to read file to check existence
                let exists = fs.read_file(dev, path).is_some();
                IoResult::Ok(alloc::vec![if exists { 1 } else { 0 }])
            } else {
                IoResult::Err("Filesystem not available")
            }
        }
        
        IoOp::FsSync => {
            let mut fs = crate::lock::utils::FS_STATE.write();
            let mut blk = crate::lock::utils::BLK_DEV.write();
            
            if let (Some(fs), Some(dev)) = (fs.as_mut(), blk.as_mut()) {
                match fs.sync(dev) {
                    Ok(_) => IoResult::Ok(Vec::new()),
                    Err(e) => IoResult::Err(e),
                }
            } else {
                IoResult::Err("Filesystem not available")
            }
        }
        
        // Display/Touch/Network operations should not reach block handler
        _ => IoResult::Err("Operation not supported for block device"),
    }
}

/// Handle network device requests
fn handle_network_request(request: &IoRequest) -> IoResult {
    match &request.operation {
        IoOp::Status => {
            let net = crate::lock::utils::NET_STATE.lock();
            if net.is_some() {
                if crate::net::is_ip_assigned() {
                    let ip = crate::net::get_my_ip();
                    let octets = ip.octets();
                    let status = alloc::format!(
                        "online {}.{}.{}.{}",
                        octets[0], octets[1], octets[2], octets[3]
                    );
                    IoResult::Ok(status.into_bytes())
                } else {
                    IoResult::Ok(b"online no-ip".to_vec())
                }
            } else {
                IoResult::Ok(b"offline".to_vec())
            }
        }
        IoOp::NetPoll { timestamp_ms } => {
            let mut net = crate::lock::utils::NET_STATE.lock();
            if let Some(state) = net.as_mut() {
                state.poll(*timestamp_ms);
            }
            IoResult::Ok(Vec::new())
        }
        IoOp::NetIsIpAssigned => {
            let assigned = crate::net::is_ip_assigned();
            IoResult::Ok(alloc::vec![if assigned { 1 } else { 0 }])
        }
        IoOp::NetGetIp => {
            let ip = crate::net::get_my_ip();
            let octets = ip.octets();
            IoResult::Ok(octets.to_vec())
        }
        _ => IoResult::Err("Network operation not implemented via I/O router"),
    }
}

/// Handle display device requests
fn handle_display_request(request: &IoRequest) -> IoResult {
    match &request.operation {
        IoOp::Status => {
            if crate::platform::d1_display::is_available() {
                IoResult::Ok(b"online".to_vec())
            } else {
                IoResult::Ok(b"offline".to_vec())
            }
        }
        IoOp::Flush | IoOp::DisplayFlush => {
            crate::platform::d1_display::flush();
            IoResult::Ok(Vec::new())
        }
        IoOp::DisplayClear => {
            crate::platform::d1_display::clear_display();
            IoResult::Ok(Vec::new())
        }
        IoOp::DisplayMarkAllDirty => {
            crate::platform::d1_display::mark_all_dirty();
            IoResult::Ok(Vec::new())
        }
        IoOp::DisplayIsAvailable => {
            let available = crate::platform::d1_display::is_available();
            IoResult::Ok(alloc::vec![if available { 1 } else { 0 }])
        }
        IoOp::TouchPoll => {
            crate::platform::d1_touch::poll();
            IoResult::Ok(Vec::new())
        }
        IoOp::TouchNextEvent => {
            if let Some(event) = crate::platform::d1_touch::next_event() {
                // Serialize event: [type:2][code:2][value:4] = 8 bytes
                let mut data = Vec::with_capacity(8);
                data.extend_from_slice(&event.event_type.to_le_bytes());
                data.extend_from_slice(&event.code.to_le_bytes());
                data.extend_from_slice(&event.value.to_le_bytes());
                IoResult::Ok(data)
            } else {
                IoResult::Ok(Vec::new()) // Empty = no event
            }
        }
        IoOp::TouchHasEvents => {
            let has = crate::platform::d1_touch::has_events();
            IoResult::Ok(alloc::vec![if has { 1 } else { 0 }])
        }
        _ => IoResult::Err("Display operation not implemented via I/O router"),
    }
}

/// Handle audio device requests
fn handle_audio_request(request: &IoRequest) -> IoResult {
    match &request.operation {
        IoOp::AudioWriteSample { sample } => {
            let success = crate::platform::d1_audio::write_sample(*sample);
            IoResult::Ok(alloc::vec![if success { 1 } else { 0 }])
        }
        IoOp::AudioSetEnabled { enabled } => {
            crate::platform::d1_audio::set_enabled(*enabled);
            IoResult::Ok(Vec::new())
        }
        IoOp::AudioSetSampleRate { rate } => {
            crate::platform::d1_audio::set_sample_rate(*rate);
            IoResult::Ok(Vec::new())
        }
        IoOp::AudioGetBufferLevel => {
            let level = crate::platform::d1_audio::buffer_level();
            IoResult::Ok(level.to_le_bytes().to_vec())
        }
        IoOp::AudioIsBufferFull => {
            let full = crate::platform::d1_audio::is_buffer_full();
            IoResult::Ok(alloc::vec![if full { 1 } else { 0 }])
        }
        IoOp::AudioIsBufferEmpty => {
            let empty = crate::platform::d1_audio::is_buffer_empty();
            IoResult::Ok(alloc::vec![if empty { 1 } else { 0 }])
        }
        IoOp::Status => {
            if crate::platform::d1_audio::is_initialized() {
                IoResult::Ok(b"online".to_vec())
            } else {
                IoResult::Ok(b"offline".to_vec())
            }
        }
        _ => IoResult::Err("Audio operation not implemented via I/O router"),
    }
}

/// Handle UART device requests
fn handle_uart_request(request: &IoRequest) -> IoResult {
    match &request.operation {
        IoOp::Write { data, .. } => {
            // Write to UART (this is safe from any hart via shared buffer)
            crate::device::uart::write_bytes(data);
            IoResult::Ok(Vec::new())
        }
        IoOp::Status => {
            IoResult::Ok(b"online".to_vec())
        }
        _ => IoResult::Err("UART operation not implemented via I/O router"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// STATISTICS
// ═══════════════════════════════════════════════════════════════════════════════

/// Get I/O router statistics
pub fn stats() -> (u64, u64, usize) {
    (
        REQUESTS_SUBMITTED.load(Ordering::Relaxed),
        REQUESTS_COMPLETED.load(Ordering::Relaxed),
        pending_count(),
    )
}
