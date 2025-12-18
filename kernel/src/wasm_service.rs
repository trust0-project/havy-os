//! WASM Worker Service for Per-Hart Execution
//!
//! This module provides a distributed WASM execution service where each hart
//! runs a dedicated worker that accepts WASM jobs via IPC. This enables:
//!
//! - Running WASM binaries on specific harts (hart affinity)
//! - Parallel WASM execution across multiple cores
//! - Load-based automatic job distribution
//! - Monitoring of per-hart WASM workload
//!
//! ## Architecture
//!
//! ```text
//! ┌──────────────┐     ┌──────────────┐     ┌──────────────┐
//! │   Hart 0     │     │   Hart 1     │     │   Hart 2     │
//! │  (Primary)   │     │  WASM Worker │     │  WASM Worker │
//! │              │     │              │     │              │
//! │  Shell/IO    │────▶│  Channel 1   │     │  Channel 2   │
//! │              │     │  Job Queue   │     │  Job Queue   │
//! └──────────────┘     └──────────────┘     └──────────────┘
//! ```
//!
//! ## Usage
//!
//! ```ignore
//! // Submit to specific hart
//! wasm_service::submit_job(wasm_bytes, args, Some(1));
//!
//! // Auto-select least loaded hart
//! wasm_service::submit_job(wasm_bytes, args, None);
//! ```

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::cpu::MAX_HARTS;
use crate::cpu::ipc::{Channel, ChannelId, Message, IPC};
use crate::Spinlock;
use crate::services::klogd::{klog_debug, klog_error, klog_info};

// ═══════════════════════════════════════════════════════════════════════════════
// JOB TYPES
// ═══════════════════════════════════════════════════════════════════════════════

/// Unique job identifier
pub type JobId = u32;

/// Status of a WASM job
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum JobStatus {
    /// Job is queued, waiting to be picked up
    Pending = 0,
    /// Job is currently executing
    Running = 1,
    /// Job completed successfully
    Completed = 2,
    /// Job failed with error
    Failed = 3,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Pending => "pending",
            JobStatus::Running => "running",
            JobStatus::Completed => "completed",
            JobStatus::Failed => "failed",
        }
    }
}

/// A WASM execution job
pub struct WasmJob {
    /// Unique job ID
    pub id: JobId,
    /// WASM binary data
    pub wasm_bytes: Vec<u8>,
    /// Command-line arguments
    pub args: Vec<String>,
    /// Target hart (None = any)
    pub target_hart: Option<usize>,
    /// Current status
    pub status: AtomicUsize,
    /// Error message if failed
    pub error: Spinlock<Option<String>>,
    /// Execution time in ms (when completed)
    pub exec_time_ms: AtomicU64,
}

impl WasmJob {
    pub fn new(id: JobId, wasm_bytes: Vec<u8>, args: Vec<String>, target_hart: Option<usize>) -> Self {
        Self {
            id,
            wasm_bytes,
            args,
            target_hart,
            status: AtomicUsize::new(JobStatus::Pending as usize),
            error: Spinlock::new(None),
            exec_time_ms: AtomicU64::new(0),
        }
    }

    pub fn get_status(&self) -> JobStatus {
        match self.status.load(Ordering::Acquire) {
            0 => JobStatus::Pending,
            1 => JobStatus::Running,
            2 => JobStatus::Completed,
            _ => JobStatus::Failed,
        }
    }

    pub fn set_status(&self, status: JobStatus) {
        self.status.store(status as usize, Ordering::Release);
    }

    pub fn set_error(&self, msg: String) {
        *self.error.lock() = Some(msg);
        self.set_status(JobStatus::Failed);
    }

    pub fn get_error(&self) -> Option<String> {
        self.error.lock().clone()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// WORKER STATE
// ═══════════════════════════════════════════════════════════════════════════════

/// Per-hart worker statistics
pub struct WorkerStats {
    /// Hart ID this worker runs on
    pub hart_id: usize,
    /// Total jobs executed
    pub jobs_completed: AtomicU64,
    /// Total jobs failed
    pub jobs_failed: AtomicU64,
    /// Total execution time (ms)
    pub total_exec_time_ms: AtomicU64,
    /// Currently running job ID (0 = none)
    pub current_job: AtomicU32,
    /// Jobs in queue (waiting)
    pub queue_depth: AtomicUsize,
    /// Worker is active
    pub active: AtomicBool,
}

impl WorkerStats {
    pub const fn new(hart_id: usize) -> Self {
        Self {
            hart_id,
            jobs_completed: AtomicU64::new(0),
            jobs_failed: AtomicU64::new(0),
            total_exec_time_ms: AtomicU64::new(0),
            current_job: AtomicU32::new(0),
            queue_depth: AtomicUsize::new(0),
            active: AtomicBool::new(false),
        }
    }

    /// Get load score (higher = busier)
    /// Combines queue depth and recent execution time
    pub fn load_score(&self) -> u64 {
        let queue = self.queue_depth.load(Ordering::Relaxed) as u64;
        let current = if self.current_job.load(Ordering::Relaxed) > 0 { 1 } else { 0 };
        queue * 10 + current * 5
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// GLOBAL SERVICE STATE
// ═══════════════════════════════════════════════════════════════════════════════

/// Global WASM service state
pub struct WasmService {
    /// Job registry (JobId -> Job)
    jobs: Spinlock<BTreeMap<JobId, Arc<WasmJob>>>,
    /// Per-hart worker stats
    workers: [WorkerStats; MAX_HARTS],
    /// Per-hart IPC channel IDs
    channels: Spinlock<[Option<ChannelId>; MAX_HARTS]>,
    /// Next job ID
    next_job_id: AtomicU32,
    /// Number of active workers
    num_workers: AtomicUsize,
    /// Service is initialized
    initialized: AtomicBool,
}

const fn create_worker_stats_array() -> [WorkerStats; MAX_HARTS] {
    let mut arr = [const { WorkerStats::new(0) }; MAX_HARTS];
    let mut i = 0;
    while i < MAX_HARTS {
        arr[i] = WorkerStats::new(i);
        i += 1;
    }
    arr
}

/// Global WASM service instance
pub static WASM_SERVICE: WasmService = WasmService {
    jobs: Spinlock::new(BTreeMap::new()),
    workers: create_worker_stats_array(),
    channels: Spinlock::new([None; MAX_HARTS]),
    next_job_id: AtomicU32::new(1),
    num_workers: AtomicUsize::new(0),
    initialized: AtomicBool::new(false),
};

impl WasmService {
    /// Initialize the WASM service with the number of harts
    pub fn init(&self, num_harts: usize) {
        if self.initialized.swap(true, Ordering::SeqCst) {
            return; // Already initialized
        }

        // Create IPC channels for each non-primary hart
        let mut channels = self.channels.lock();
        for hart_id in 1..num_harts {
            let channel_name = alloc::format!("wasm-hart-{}", hart_id);
            let channel = IPC.create_channel(&channel_name);
            channels[hart_id] = Some(channel.id);
            self.workers[hart_id].active.store(true, Ordering::Release);
        }

        self.num_workers.store(num_harts.saturating_sub(1), Ordering::Release);

        klog_info(
            "wasm-svc",
            &alloc::format!("WASM service initialized with {} workers", num_harts.saturating_sub(1)),
        );
    }

    /// Get the IPC channel for a hart
    pub fn get_channel(&self, hart_id: usize) -> Option<Arc<Channel>> {
        let channels = self.channels.lock();
        channels[hart_id].and_then(|id| IPC.get_channel(id))
    }

    /// Submit a WASM job for execution
    ///
    /// # Arguments
    /// * `wasm_bytes` - The WASM binary
    /// * `args` - Command-line arguments
    /// * `target_hart` - Specific hart to run on, or None for auto-selection
    ///
    /// # Returns
    /// Job ID on success
    pub fn submit_job(
        &self,
        wasm_bytes: Vec<u8>,
        args: Vec<String>,
        target_hart: Option<usize>,
    ) -> Result<JobId, &'static str> {
        if !self.initialized.load(Ordering::Acquire) {
            return Err("WASM service not initialized");
        }

        let job_id = self.next_job_id.fetch_add(1, Ordering::SeqCst);

        // Determine target hart
        let hart = match target_hart {
            Some(h) if h > 0 && h < MAX_HARTS => h,
            Some(0) => return Err("Cannot submit to hart 0 (primary)"),
            Some(_) => return Err("Invalid hart ID"),
            None => self.find_least_loaded_worker()?,
        };

        // Create job
        let job = Arc::new(WasmJob::new(job_id, wasm_bytes, args, Some(hart)));
        self.jobs.lock().insert(job_id, job.clone());

        // Send job notification to worker via IPC
        let channel = self.get_channel(hart).ok_or("Worker channel not found")?;
        
        // Message contains job ID as bytes
        let msg = Message::new(
            0, // sender PID (0 = kernel)
            job_id.to_le_bytes().to_vec(),
            1, // msg_type = 1 for job notification
        );

        channel.send(msg).map_err(|_| "Failed to send job to worker")?;

        // Update queue depth
        self.workers[hart].queue_depth.fetch_add(1, Ordering::Relaxed);

        klog_debug(
            "wasm-svc",
            &alloc::format!("Submitted job {} to hart {}", job_id, hart),
        );

        Ok(job_id)
    }

    /// Find the worker with the lowest load
    fn find_least_loaded_worker(&self) -> Result<usize, &'static str> {
        let num_workers = self.num_workers.load(Ordering::Relaxed);
        if num_workers == 0 {
            return Err("No WASM workers available");
        }

        let mut best_hart = 1;
        let mut best_score = u64::MAX;

        for hart_id in 1..=num_workers {
            if !self.workers[hart_id].active.load(Ordering::Relaxed) {
                continue;
            }

            let score = self.workers[hart_id].load_score();
            if score < best_score {
                best_score = score;
                best_hart = hart_id;
            }
        }

        Ok(best_hart)
    }

    /// Get a job by ID
    pub fn get_job(&self, job_id: JobId) -> Option<Arc<WasmJob>> {
        self.jobs.lock().get(&job_id).cloned()
    }

    /// Get worker stats for a hart
    pub fn get_worker_stats(&self, hart_id: usize) -> Option<&WorkerStats> {
        if hart_id < MAX_HARTS && self.workers[hart_id].active.load(Ordering::Relaxed) {
            Some(&self.workers[hart_id])
        } else {
            None
        }
    }

    /// Get all active workers
    pub fn list_workers(&self) -> Vec<(usize, u64, u64, u64, u32, usize)> {
        let num = self.num_workers.load(Ordering::Relaxed);
        let mut result = Vec::new();

        for hart_id in 1..=num {
            let w = &self.workers[hart_id];
            if w.active.load(Ordering::Relaxed) {
                result.push((
                    hart_id,
                    w.jobs_completed.load(Ordering::Relaxed),
                    w.jobs_failed.load(Ordering::Relaxed),
                    w.total_exec_time_ms.load(Ordering::Relaxed),
                    w.current_job.load(Ordering::Relaxed),
                    w.queue_depth.load(Ordering::Relaxed),
                ));
            }
        }

        result
    }

    /// List recent jobs
    pub fn list_jobs(&self, limit: usize) -> Vec<(JobId, JobStatus, Option<usize>, u64)> {
        self.jobs
            .lock()
            .values()
            .rev()
            .take(limit)
            .map(|j| (
                j.id,
                j.get_status(),
                j.target_hart,
                j.exec_time_ms.load(Ordering::Relaxed),
            ))
            .collect()
    }

    /// Clean up completed/failed jobs older than limit
    pub fn cleanup_jobs(&self, keep_recent: usize) -> usize {
        let mut jobs = self.jobs.lock();
        let total = jobs.len();
        if total <= keep_recent {
            return 0;
        }

        // Find jobs to remove (completed/failed, oldest first)
        let mut to_remove: Vec<JobId> = jobs
            .iter()
            .filter(|(_, j)| matches!(j.get_status(), JobStatus::Completed | JobStatus::Failed))
            .map(|(id, _)| *id)
            .collect();

        to_remove.sort();
        let remove_count = to_remove.len().saturating_sub(keep_recent / 2);
        
        for id in to_remove.into_iter().take(remove_count) {
            jobs.remove(&id);
        }

        remove_count
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// WORKER ENTRY POINT
// ═══════════════════════════════════════════════════════════════════════════════

/// Entry point for WASM worker running on a secondary hart
///
/// This function runs in a loop, polling the IPC channel for job notifications
/// and executing WASM binaries as they arrive.
pub fn worker_entry() {
    let hart_id = crate::get_hart_id();
    
    klog_info(
        "wasm-svc",
        &alloc::format!("WASM worker starting on hart {}", hart_id),
    );

    // Get our channel
    let channel = match WASM_SERVICE.get_channel(hart_id) {
        Some(ch) => ch,
        None => {
            klog_error(
                "wasm-svc",
                &alloc::format!("Hart {} has no WASM channel", hart_id),
            );
            return;
        }
    };

    loop {
        // Poll for job notifications
        if let Some(msg) = channel.try_recv() {
            if msg.msg_type == 1 && msg.data.len() >= 4 {
                // Extract job ID
                let job_id = u32::from_le_bytes([
                    msg.data[0],
                    msg.data[1],
                    msg.data[2],
                    msg.data[3],
                ]);

                // Decrement queue depth
                WASM_SERVICE.workers[hart_id].queue_depth.fetch_sub(1, Ordering::Relaxed);

                // Get the job
                if let Some(job) = WASM_SERVICE.get_job(job_id) {
                    execute_job(hart_id, &job);
                }
            }
        } else {
            // No job available, yield CPU
            core::hint::spin_loop();
            // Small delay to avoid burning CPU
            for _ in 0..1000 {
                core::hint::spin_loop();
            }
        }
    }
}

/// Execute a WASM job
fn execute_job(hart_id: usize, job: &WasmJob) {
    let stats = &WASM_SERVICE.workers[hart_id];
    
    // Mark job as running
    job.set_status(JobStatus::Running);
    stats.current_job.store(job.id, Ordering::Release);

    klog_debug(
        "wasm-svc",
        &alloc::format!("Hart {} executing job {}", hart_id, job.id),
    );

    let start_time = crate::get_time_ms();

    // Convert args to &str slice for wasm::execute
    let args: Vec<&str> = job.args.iter().map(|s| s.as_str()).collect();

    // Execute the WASM binary
    match crate::wasm::execute(&job.wasm_bytes, &args) {
        Ok(_) => {
            let exec_time = (crate::get_time_ms() - start_time) as u64;
            job.exec_time_ms.store(exec_time, Ordering::Relaxed);
            job.set_status(JobStatus::Completed);
            
            stats.jobs_completed.fetch_add(1, Ordering::Relaxed);
            stats.total_exec_time_ms.fetch_add(exec_time, Ordering::Relaxed);

            klog_debug(
                "wasm-svc",
                &alloc::format!("Job {} completed in {}ms", job.id, exec_time),
            );
        }
        Err(e) => {
            job.set_error(e);
            stats.jobs_failed.fetch_add(1, Ordering::Relaxed);

            klog_error(
                "wasm-svc",
                &alloc::format!("Job {} failed: {:?}", job.id, job.get_error()),
            );
        }
    }

    stats.current_job.store(0, Ordering::Release);
}

// ═══════════════════════════════════════════════════════════════════════════════
// PUBLIC API
// ═══════════════════════════════════════════════════════════════════════════════

/// Initialize the WASM service (call from kernel init)
pub fn init(num_harts: usize) {
    WASM_SERVICE.init(num_harts);
}

/// Submit a WASM job for execution
///
/// # Arguments
/// * `wasm_bytes` - The WASM binary
/// * `args` - Command-line arguments  
/// * `target_hart` - Specific hart (1+), or None for auto-selection
pub fn submit_job(
    wasm_bytes: Vec<u8>,
    args: Vec<String>,
    target_hart: Option<usize>,
) -> Result<JobId, &'static str> {
    WASM_SERVICE.submit_job(wasm_bytes, args, target_hart)
}

/// Get job status
pub fn job_status(job_id: JobId) -> Option<JobStatus> {
    WASM_SERVICE.get_job(job_id).map(|j| j.get_status())
}

/// List all workers with their stats
pub fn list_workers() -> Vec<(usize, u64, u64, u64, u32, usize)> {
    WASM_SERVICE.list_workers()
}

/// List recent jobs
pub fn list_jobs(limit: usize) -> Vec<(JobId, JobStatus, Option<usize>, u64)> {
    WASM_SERVICE.list_jobs(limit)
}

/// Get the least loaded worker hart
pub fn least_loaded_hart() -> Option<usize> {
    WASM_SERVICE.find_least_loaded_worker().ok()
}

