//! Kernel logging infrastructure
//!
//! Provides a ring buffer for kernel messages that can be:
//! - Written to by any subsystem via klog!() macro
//! - Flushed to /var/log/kernel.log by the klogd daemon
//! - Viewed via dmesg command

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};

use crate::{Spinlock};

// Re-export log types from lock::state::log for backwards compatibility
pub use crate::lock::state::log::{
    LogLevel,
    LogEntry,
    LogBufferState as LogBuffer,
};

/// Maximum messages in the ring buffer
const LOG_BUFFER_SIZE: usize = 128;
const LOG_LINE_MAX: usize = 128;

/// State for klogd daemon
static KLOGD_LAST_RUN: AtomicI64 = AtomicI64::new(0);
static KLOGD_TICK: AtomicUsize = AtomicUsize::new(0);
static KLOGD_INITIALIZED: AtomicBool = AtomicBool::new(false);


// ===============================================================================
// LOG BUFFER SYSTEM
// Daemons write to an in-memory buffer, hart 0 flushes to disk
// This avoids VirtIO contention between harts
// ===============================================================================
/// A single log entry (for file logging)
pub(crate) struct SimpleLogEntry {
    data: [u8; LOG_LINE_MAX],
    len: usize,
    target: LogTarget,
}

/// Which log file to write to
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum LogTarget {
    Kernel,   // /var/log/kernel.log
    Sysmond,  // /var/log/sysmond.log
}

/// Simple log buffer state (for file logging)
pub(crate) struct SimpleLogBuffer {
    entries: [Option<SimpleLogEntry>; LOG_BUFFER_SIZE],
    count: usize,
    last_flush_ms: i64,
}

impl SimpleLogBuffer {
    pub(crate) const fn new() -> Self {
        const NONE: Option<SimpleLogEntry> = None;
        Self {
            entries: [NONE; LOG_BUFFER_SIZE],
            count: 0,
            last_flush_ms: 0,
        }
    }
    
    /// Add a log entry to the buffer
    pub(crate) fn push(&mut self, line: &str, target: LogTarget) {
        if self.count >= LOG_BUFFER_SIZE {
            // Buffer full, drop oldest entry (simple ring behavior)
            for i in 1..LOG_BUFFER_SIZE {
                self.entries[i - 1] = self.entries[i].take();
            }
            self.count = LOG_BUFFER_SIZE - 1;
        }
        
        let mut entry = SimpleLogEntry {
            data: [0u8; LOG_LINE_MAX],
            len: 0,
            target,
        };
        
        let bytes = line.as_bytes();
        let copy_len = bytes.len().min(LOG_LINE_MAX);
        entry.data[..copy_len].copy_from_slice(&bytes[..copy_len]);
        entry.len = copy_len;
        
        self.entries[self.count] = Some(entry);
        self.count += 1;
    }
    
    /// Take all entries for flushing
    pub(crate) fn drain(&mut self) -> Vec<(String, LogTarget)> {
        let mut result = Vec::with_capacity(self.count);
        for i in 0..self.count {
            if let Some(entry) = self.entries[i].take() {
                if let Ok(s) = core::str::from_utf8(&entry.data[..entry.len]) {
                    result.push((String::from(s), entry.target));
                }
            }
        }
        self.count = 0;
        result
    }
    
    /// Check if buffer has entries
    #[allow(dead_code)]
    pub(crate) fn has_entries(&self) -> bool {
        self.count > 0
    }
    
    /// Get time since last flush
    #[allow(dead_code)]
    pub(crate) fn time_since_flush(&self, now: i64) -> i64 {
        now - self.last_flush_ms
    }
    
    /// Update last flush time
    pub(crate) fn mark_flushed(&mut self, now: i64) {
        self.last_flush_ms = now;
    }
}

/// Global simple log buffer (for file logging)
static SIMPLE_LOG_BUFFER: Spinlock<SimpleLogBuffer> = Spinlock::new(SimpleLogBuffer::new());

/// Queue a log line to be written to a file
/// Safe to call from any hart
pub fn queue_log(line: &str, target: LogTarget) {
    let mut buffer = SIMPLE_LOG_BUFFER.lock();
    buffer.push(line, target);
}

/// Queue a log line for sysmond
pub fn queue_sysmond_log(line: &str) {
    queue_log(line, LogTarget::Sysmond);
}

/// Alias for flush_logs() - for backwards compatibility
pub fn flush_log_buffer() -> usize {
    flush_logs()
}

/// Flush all queued log entries to their respective files
/// Should only be called from hart 0 to avoid VirtIO contention
/// Returns number of entries flushed
pub fn flush_logs() -> usize {
    let entries = {
        let mut buffer = SIMPLE_LOG_BUFFER.lock();
        let entries = buffer.drain();
        buffer.mark_flushed(crate::get_time_ms());
        entries
    };
    
    let count = entries.len();
    if count == 0 {
        return 0;
    }
    
    // Group entries by target
    let mut kernel_lines = Vec::new();
    let mut sysmond_lines = Vec::new();
    
    for (line, target) in entries {
        match target {
            LogTarget::Kernel => kernel_lines.push(line),
            LogTarget::Sysmond => sysmond_lines.push(line),
        }
    }
    
    // Write to files (need FS access)
    let fs_guard = crate::lock::utils::FS_STATE.write();
    let blk_guard = crate::lock::utils::BLK_DEV.write();
    
    if let (Some(ref mut fs), Some(ref mut dev)) = (
        fs_guard.as_ref().map(|_| ()),
        blk_guard.as_ref().map(|_| ()),
    ) {
        // Get mutable references
        drop(fs_guard);
        drop(blk_guard);
        
        let mut fs_guard = crate::lock::utils::FS_STATE.write();
        let mut blk_guard = crate::lock::utils::BLK_DEV.write();
        
        if let (Some(ref mut fs), Some(ref mut dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
            // Append kernel log lines
            if !kernel_lines.is_empty() {
                let mut content = fs.read_file(dev, "/var/log/kernel.log")
                    .map(|v| String::from_utf8_lossy(&v).into_owned())
                    .unwrap_or_default();
                
                for line in kernel_lines {
                    content.push_str(&line);
                    content.push('\n');
                }
                
                let _ = fs.write_file(dev, "/var/log/kernel.log", content.as_bytes());
            }
            
            // Append sysmond log lines
            if !sysmond_lines.is_empty() {
                let mut content = fs.read_file(dev, "/var/log/sysmond.log")
                    .map(|v| String::from_utf8_lossy(&v).into_owned())
                    .unwrap_or_default();
                
                for line in sysmond_lines {
                    content.push_str(&line);
                    content.push('\n');
                }
                
                let _ = fs.write_file(dev, "/var/log/sysmond.log", content.as_bytes());
            }
            
            // Sync once at the end
            let _ = fs.sync(dev);
        }
    }
    
    count
}

/// Global kernel log buffer
/// Note: Console output is disabled by default to avoid UART contention during boot
pub static KLOG: LogBuffer = LogBuffer::new_console_disabled();

// ═══════════════════════════════════════════════════════════════════════════════
// PUBLIC LOGGING FUNCTIONS
// ═══════════════════════════════════════════════════════════════════════════════

/// Log an emergency message
pub fn klog_emergency(subsystem: &str, message: &str) {
    KLOG.log(LogLevel::Emergency, subsystem, message);
}

/// Log an alert message
pub fn klog_alert(subsystem: &str, message: &str) {
    KLOG.log(LogLevel::Alert, subsystem, message);
}

/// Log a critical message
pub fn klog_critical(subsystem: &str, message: &str) {
    KLOG.log(LogLevel::Critical, subsystem, message);
}

/// Log an error message
pub fn klog_error(subsystem: &str, message: &str) {
    KLOG.log(LogLevel::Error, subsystem, message);
}

/// Log a warning message
pub fn klog_warning(subsystem: &str, message: &str) {
    KLOG.log(LogLevel::Warning, subsystem, message);
}

/// Log a notice message
pub fn klog_notice(subsystem: &str, message: &str) {
    KLOG.log(LogLevel::Notice, subsystem, message);
}

/// Log an info message
pub fn klog_info(subsystem: &str, message: &str) {
    KLOG.log(LogLevel::Info, subsystem, message);
}

/// Log a debug message
pub fn klog_debug(subsystem: &str, message: &str) {
    KLOG.log(LogLevel::Debug, subsystem, message);
}

/// Log a trace message
pub fn klog_trace(subsystem: &str, message: &str) {
    KLOG.log(LogLevel::Trace, subsystem, message);
}

/// Set the minimum log level to display
pub fn set_log_level(level: LogLevel) {
    KLOG.set_level(level);
}

/// Enable or disable console output
pub fn set_console_output(enabled: bool) {
    KLOG.set_console(enabled);
}

/// Append a line to the kernel log (queued for hart 0 to flush)
/// Safe to call from any hart
fn append_to_log(line: &str) -> bool {
    queue_log(line, LogTarget::Kernel);
    true
}



/// Run klogd work if 5 seconds have passed since last run
pub fn klogd_tick() {
    let now = crate::get_time_ms();
    let last = KLOGD_LAST_RUN.load(Ordering::Relaxed);

    // First run: initialize (but delay filesystem access by 10 seconds)
    if !KLOGD_INITIALIZED.load(Ordering::Relaxed) {
        // Wait 10 seconds after boot before initializing
        // This avoids VirtIO contention with shell on secondary harts
        if now < 10000 {
            return;
        }
        
        KLOGD_INITIALIZED.store(true, Ordering::Relaxed);
        KLOGD_LAST_RUN.store(now, Ordering::Relaxed);
        
        // Write initial log entry
        let log_line = format!("[{}] klogd: started", now);
        append_to_log(&log_line);
        return;
    }

    // Check if 5 seconds have passed
    if now - last < 5000 {
        return;
    }

    // Update timing
    KLOGD_LAST_RUN.store(now, Ordering::Relaxed);
    let tick = KLOGD_TICK.fetch_add(1, Ordering::Relaxed) + 1;

    // Collect and log memory stats
    let (heap_used, heap_free) = crate::allocator::heap_stats();
    let log_line = format!(
        "[{}] klogd[{}]: heap_used={}KB heap_free={}KB",
        now, tick, heap_used / 1024, heap_free / 1024
    );
    append_to_log(&log_line);
}


/// Daemon service entry point for klogd
/// Cooperative time-slicing: does one tick of work and returns.
/// The scheduler will requeue this daemon to run again.
/// Note: klogd_tick has internal timing (runs every 5 seconds)
pub fn klogd_service() {
    // Quick check: only do real work if 4+ seconds since last run
    // This reduces the frequency of even attempting to acquire locks
    let now = crate::get_time_ms();
    let last = KLOGD_LAST_RUN.load(Ordering::Relaxed);
    
    if KLOGD_INITIALIZED.load(Ordering::Relaxed) && (now - last) < 4000 {
        // Not time yet - sleep longer to save CPU
        return;
    }
    
    // Time to potentially do work
    klogd_tick();
}
