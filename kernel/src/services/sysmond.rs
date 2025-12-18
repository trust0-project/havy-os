use core::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};


use alloc::format;

use crate::{PROC_SCHEDULER, services::klogd::LogTarget};


/// State for sysmond daemon  
static SYSMOND_LAST_RUN: AtomicI64 = AtomicI64::new(0);
static SYSMOND_TICK: AtomicUsize = AtomicUsize::new(0);
static SYSMOND_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Append a line to the sysmond log (queued for hart 0 to flush)
/// Safe to call from any hart
fn append_to_sysmond_log(line: &str) -> bool {
    crate::services::klogd::queue_log(line, LogTarget::Sysmond);
    true
}


/// Run sysmond work if 10 seconds have passed since last run
pub fn sysmond_tick() {
    let now = crate::get_time_ms();
    let last = SYSMOND_LAST_RUN.load(Ordering::Relaxed);

    // First run: initialize (delay 15 seconds to avoid contention)
    if !SYSMOND_INITIALIZED.load(Ordering::Relaxed) {
        if now < 15000 {
            return; // Wait for initial delay
        }
        SYSMOND_INITIALIZED.store(true, Ordering::Relaxed);
        SYSMOND_LAST_RUN.store(now, Ordering::Relaxed);
        
        // Write initial log entry
        let log_line = format!("[{}] sysmond: started", now);
        append_to_sysmond_log(&log_line);
        return;
    }

    // Check if 10 seconds have passed
    if now - last < 10000 {
        return;
    }

    SYSMOND_LAST_RUN.store(now, Ordering::Relaxed);
    let tick = SYSMOND_TICK.fetch_add(1, Ordering::Relaxed) + 1;

    // Collect and log system stats
    let process_count = PROC_SCHEDULER.process_count();
    let queued_count = PROC_SCHEDULER.total_queued();
    let num_harts = crate::HARTS_ONLINE.load(Ordering::Relaxed);

    // Reap zombies
    let reaped = PROC_SCHEDULER.reap_zombies();

    let log_line = format!(
        "[{}] sysmond[{}]: procs={} queued={} harts={} reaped={}",
        now, tick, process_count, queued_count, num_harts, reaped
    );
    append_to_sysmond_log(&log_line);
}


/// Daemon service entry point for sysmond
/// Cooperative time-slicing: does one tick of work and returns.
/// The scheduler will requeue this daemon to run again.
/// Note: sysmond_tick has internal timing (runs every 10 seconds)
pub fn sysmond_service() {
    // Quick check: only do real work if 9+ seconds since last run
    let now = crate::get_time_ms();
    let last = SYSMOND_LAST_RUN.load(Ordering::Relaxed);
    
    if SYSMOND_INITIALIZED.load(Ordering::Relaxed) && (now - last) < 9000 {
        // Not time yet - sleep longer to save CPU
        return;
    }
    
    // Time to potentially do work
    sysmond_tick();
}
