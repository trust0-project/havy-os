//! Wait Queue for Blocking I/O Operations
//!
//! This module provides a simple wait queue mechanism that allows processes
//! to yield the CPU while waiting for a condition to be met.
//!
//! ## RISC-V Compliance
//!
//! - Uses `fence` instructions for proper memory ordering
//! - Integrates with the scheduler's yield mechanism
//! - Supports multi-hart environments with atomic operations

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering, fence};

/// A simple wait queue for blocking operations.
///
/// Processes call `wait()` to yield until `wake_all()` is called.
/// This is used for VirtIO descriptor availability.
pub struct WaitQueue {
    /// Flag indicating a wakeup has occurred
    wakeup_pending: AtomicBool,
    /// Number of processes currently waiting
    waiters: AtomicUsize,
}

impl WaitQueue {
    /// Create a new wait queue
    pub const fn new() -> Self {
        Self {
            wakeup_pending: AtomicBool::new(false),
            waiters: AtomicUsize::new(0),
        }
    }

    /// Wait until notified or timeout.
    ///
    /// This function yields the CPU while waiting, allowing other processes
    /// to run. It implements a simple spin-yield loop with proper memory barriers.
    ///
    /// ## Arguments
    /// * `timeout_ms` - Maximum time to wait in milliseconds
    ///
    /// ## Returns
    /// * `true` if woken by notification
    /// * `false` if timeout expired
    pub fn wait(&self, timeout_ms: i64) -> bool {
        let start = crate::get_time_ms();
        
        // Register as a waiter
        self.waiters.fetch_add(1, Ordering::SeqCst);
        
        // Clear any pending wakeup (we're starting fresh)
        self.wakeup_pending.store(false, Ordering::Release);
        
        loop {
            // Check if woken
            if self.wakeup_pending.swap(false, Ordering::Acquire) {
                self.waiters.fetch_sub(1, Ordering::SeqCst);
                return true;
            }
            
            // Check timeout
            let elapsed = crate::get_time_ms() - start;
            if elapsed > timeout_ms {
                self.waiters.fetch_sub(1, Ordering::SeqCst);
                return false;
            }
            
            // Memory fence before yielding (RISC-V weak memory model)
            fence(Ordering::SeqCst);
            
            // Yield the CPU to allow other processes to run
            // This integrates with the scheduler
            crate::sched::yield_now();
            
            // Brief spin to reduce scheduler overhead
            for _ in 0..100 {
                core::hint::spin_loop();
            }
        }
    }

    /// Wake all waiting processes.
    ///
    /// Called when the condition is met (e.g., VirtIO operation completed).
    pub fn wake_all(&self) {
        // Set wakeup flag
        self.wakeup_pending.store(true, Ordering::Release);
        
        // Memory fence ensures the wakeup is visible
        fence(Ordering::SeqCst);
    }

    /// Check if there are waiters (for debugging)
    #[allow(dead_code)]
    pub fn has_waiters(&self) -> bool {
        self.waiters.load(Ordering::Relaxed) > 0
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Static Wait Queue for VirtIO Block Device
// ═══════════════════════════════════════════════════════════════════════════════

/// Global wait queue for VirtIO block descriptor availability
pub static BLK_WAIT_QUEUE: WaitQueue = WaitQueue::new();

/// Wait for block device descriptors to become available
pub fn wait_for_blk_descriptors() -> bool {
    BLK_WAIT_QUEUE.wait(5000) // 5 second timeout
}

/// Notify that block device descriptors are now available
pub fn wake_blk_waiters() {
    BLK_WAIT_QUEUE.wake_all();
}
