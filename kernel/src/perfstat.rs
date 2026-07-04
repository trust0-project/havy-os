//! Kernel-wide performance counters.
//!
//! Cheap relaxed atomics incremented from hot paths (scheduler, io_router,
//! allocator, display, traps). Read/reset via the `perfstat` userspace
//! command (SYS_PERFSTAT syscall).
//!
//! Counter order is ABI: userspace indexes the snapshot array by these ids,
//! so only append new counters, never reorder.

use core::sync::atomic::{AtomicU64, Ordering};

/// Counter ids. Keep in sync with `mkfs/src/bin/perfstat.rs`.
pub mod id {
    /// Scheduler: processes picked and run
    pub const SCHED_PICKS: usize = 0;
    /// Scheduler: successful work steals
    pub const SCHED_STEALS: usize = 1;
    /// Scheduler: steal attempts that found nothing
    pub const SCHED_STEAL_MISSES: usize = 2;
    /// Scheduler: daemon requeues
    pub const SCHED_REQUEUES: usize = 3;
    /// Scheduler: cross-hart migrations from load balancing
    pub const SCHED_MIGRATIONS: usize = 4;
    /// IPIs sent between harts
    pub const IPIS_SENT: usize = 5;
    /// io_router: requests submitted by secondary harts
    pub const IO_REQUESTS: usize = 6;
    /// io_router: requests dispatched on hart 0
    pub const IO_DISPATCHED: usize = 7;
    /// Heap: allocations
    pub const HEAP_ALLOCS: usize = 8;
    /// Heap: frees
    pub const HEAP_FREES: usize = 9;
    /// Heap: total bytes allocated (cumulative)
    pub const HEAP_ALLOC_BYTES: usize = 10;
    /// Log lines pushed to the kernel log buffer
    pub const LOG_LINES: usize = 11;
    /// gpuid service ticks (total scheduler invocations)
    pub const GPUID_TICKS: usize = 12;
    /// gpuid ticks that did real work (input, render, or flush)
    pub const GPUID_ACTIVE_TICKS: usize = 13;
    /// Display frames flushed (back -> front buffer)
    pub const FRAMES_FLUSHED: usize = 14;
    /// Total dirty pixels copied across all flushes
    pub const DIRTY_PIXELS: usize = 15;
    /// Timer interrupts handled
    pub const TIMER_IRQS: usize = 16;
    /// Syscalls handled
    pub const SYSCALLS: usize = 17;
    /// Idle WFI entries in hart_loop
    pub const IDLE_WFI: usize = 18;

    pub const COUNT: usize = 19;
}

const ZERO: AtomicU64 = AtomicU64::new(0);
static COUNTERS: [AtomicU64; id::COUNT] = [ZERO; id::COUNT];

/// Increment a counter by one.
#[inline(always)]
pub fn inc(counter: usize) {
    COUNTERS[counter].fetch_add(1, Ordering::Relaxed);
}

/// Add an arbitrary amount to a counter.
#[inline(always)]
pub fn add(counter: usize, n: u64) {
    COUNTERS[counter].fetch_add(n, Ordering::Relaxed);
}

/// Read one counter.
#[inline]
pub fn get(counter: usize) -> u64 {
    COUNTERS[counter].load(Ordering::Relaxed)
}

/// Copy all counters into `out`; returns how many were written.
pub fn snapshot(out: &mut [u64]) -> usize {
    let n = out.len().min(id::COUNT);
    for (i, slot) in out.iter_mut().take(n).enumerate() {
        *slot = COUNTERS[i].load(Ordering::Relaxed);
    }
    n
}

/// Reset all counters to zero.
pub fn reset() {
    for counter in COUNTERS.iter() {
        counter.store(0, Ordering::Relaxed);
    }
}
