//! Process Scheduler
//!
//! This module provides the process scheduler that assigns processes to CPUs.
//! The scheduler is responsible for:
//!
//! - Maintaining per-CPU run queues
//! - Picking the next process to run on each CPU
//! - Work stealing (idle CPUs take work from busy ones)
//! - Priority-based scheduling
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                        Process Scheduler                            │
//! ├─────────────────────────────────────────────────────────────────────┤
//! │  CPU 0 Run Queue    │  CPU 1 Run Queue    │  CPU N Run Queue       │
//! │  [P3] [P1] [P7]     │  [P2] [P5]          │  [P4] [P8] [P9]        │
//! └─────────────────────────────────────────────────────────────────────┘
//!                              │
//!         ┌────────────────────┼────────────────────┐
//!         ▼                    ▼                    ▼
//!     ┌───────┐           ┌───────┐            ┌───────┐
//!     │ CPU 0 │           │ CPU 1 │            │ CPU N │
//!     │ (BSP) │           │(Worker)│           │(Worker)│
//!     └───────┘           └───────┘            └───────┘
//! ```
//!
//! CPUs (Web Workers in browser) are just execution units. The scheduler
//! assigns processes to CPUs based on load and affinity.

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::cpu::{ self, CPU_TABLE, MAX_HARTS};
use crate::cpu::process::{allocate_pid, Priority, Process, ProcessEntry, ProcessInfo,  Pid, PROCESS_TABLE};
use crate::Spinlock;
use crate::services::klogd::{klog_debug, klog_info, klog_trace};

// ═══════════════════════════════════════════════════════════════════════════════
// RUN QUEUE
// ═══════════════════════════════════════════════════════════════════════════════

/// Per-CPU run queue containing ready processes
pub struct RunQueue {
    /// Processes waiting to run (priority sorted, higher priority first)
    queue: VecDeque<Arc<Process>>,
}

impl RunQueue {
    /// Create a new empty run queue
    pub const fn new() -> Self {
        Self {
            queue: VecDeque::new(),
        }
    }

    /// Add a process to the queue (maintains priority order)
    pub fn enqueue(&mut self, process: Arc<Process>) {
        let priority = process.priority;
        
        // Find insertion point (higher priority = earlier position)
        let mut insert_pos = self.queue.len();
        for (i, p) in self.queue.iter().enumerate() {
            if p.priority < priority {
                insert_pos = i;
                break;
            }
        }
        
        self.queue.insert(insert_pos, process);
    }

    /// Get the next runnable process
    pub fn dequeue(&mut self) -> Option<Arc<Process>> {
        // Find first process in Ready state
        for i in 0..self.queue.len() {
            if self.queue[i].state().is_runnable() {
                return self.queue.remove(i);
            }
        }
        None
    }

    /// Peek at the next process without removing it
    pub fn peek(&self) -> Option<&Arc<Process>> {
        self.queue.iter().find(|p| p.state().is_runnable())
    }

    /// Remove a specific process by PID
    pub fn remove(&mut self, pid: Pid) -> Option<Arc<Process>> {
        if let Some(pos) = self.queue.iter().position(|p| p.pid == pid) {
            self.queue.remove(pos)
        } else {
            None
        }
    }

    /// Steal a process from the back of the queue (for work stealing)
    pub fn steal(&mut self) -> Option<Arc<Process>> {
        // Only steal if we have more than one process
        if self.queue.len() > 1 {
            // Steal lowest priority (from back)
            self.queue.pop_back()
        } else {
            None
        }
    }

    /// Number of processes in queue
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Check if queue is empty
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Check if a process is in this queue
    pub fn contains(&self, pid: Pid) -> bool {
        self.queue.iter().any(|p| p.pid == pid)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// SCHEDULER
// ═══════════════════════════════════════════════════════════════════════════════

/// Creates const-initialized run queue array
const fn create_queue_array() -> [Spinlock<RunQueue>; MAX_HARTS] {
    const INIT_QUEUE: Spinlock<RunQueue> = Spinlock::new(RunQueue {
        queue: VecDeque::new(),
    });
    [INIT_QUEUE; MAX_HARTS]
}

/// The process scheduler
pub struct Scheduler {
    /// Per-CPU run queues
    queues: [Spinlock<RunQueue>; MAX_HARTS],
    
    /// Number of CPUs available for scheduling
    num_cpus: AtomicUsize,
    
    /// Scheduler is active
    active: AtomicBool,
    
    /// Total processes spawned
    spawn_count: AtomicUsize,
}

impl Scheduler {
    /// Create a new scheduler
    pub const fn new() -> Self {
        Self {
            queues: create_queue_array(),
            num_cpus: AtomicUsize::new(1),
            active: AtomicBool::new(false),
            spawn_count: AtomicUsize::new(0),
        }
    }

    /// Initialize the scheduler
    pub fn init(&self, num_cpus: usize) {
        self.num_cpus.store(num_cpus.max(1), Ordering::Release);
        self.active.store(true, Ordering::Release);
        
        klog_info(
            "sched",
            &alloc::format!("Scheduler initialized with {} CPUs", num_cpus),
        );
    }

    /// Check if scheduler is active
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    /// Get number of CPUs
    pub fn num_cpus(&self) -> usize {
        self.num_cpus.load(Ordering::Relaxed)
    }

    // ─── Process Spawning ───────────────────────────────────────────────────

    /// Spawn a new process
    pub fn spawn(&self, name: &str, entry: ProcessEntry, priority: Priority) -> Pid {
        self.spawn_on_cpu(name, entry, priority, None)
    }

    /// Spawn a process with CPU affinity
    pub fn spawn_on_cpu(
        &self,
        name: &str,
        entry: ProcessEntry,
        priority: Priority,
        cpu_affinity: Option<usize>,
    ) -> Pid {
        let pid = allocate_pid();
        let mut process = Process::new(pid, name, entry);
        process.priority = priority;
        
        if let Some(cpu_id) = cpu_affinity {
            process.set_cpu_affinity(cpu_id);
        }
        
        let process = Arc::new(process);
        
        // Register in process table
        PROCESS_TABLE.register(process.clone());
        
        // Determine target CPU
        let target_cpu = cpu_affinity.unwrap_or_else(|| self.find_least_loaded_cpu());
        
        // Mark as ready and enqueue
        process.mark_ready();
        self.enqueue(target_cpu, process);
        
        self.spawn_count.fetch_add(1, Ordering::Relaxed);
        
        klog_debug(
            "sched",
            &alloc::format!("Spawned '{}' (PID {}) on CPU {}", name, pid, target_cpu),
        );
        
        // Send IPI to wake target CPU if not BSP
        if target_cpu != 0 {
            crate::send_ipi(target_cpu);
        }
        
        pid
    }

    /// Spawn a daemon process (kernel service)
    pub fn spawn_daemon(&self, name: &str, entry: ProcessEntry, priority: Priority) -> Pid {
        self.spawn_daemon_on_cpu(name, entry, priority, None)
    }

    /// Spawn a daemon process with CPU affinity
    /// Daemons are requeued after each tick for cooperative time-slicing
    pub fn spawn_daemon_on_cpu(
        &self,
        name: &str,
        entry: ProcessEntry,
        priority: Priority,
        cpu_affinity: Option<usize>,
    ) -> Pid {
        let pid = allocate_pid();
        let mut process = Process::new_daemon(pid, name, entry);
        
        // Set the requested priority (new_daemon defaults to Normal)
        process.priority = priority;
        
        if let Some(cpu_id) = cpu_affinity {
            process.set_cpu_affinity(cpu_id);
        }
        
        let process = Arc::new(process);
        
        PROCESS_TABLE.register(process.clone());
        
        let target_cpu = cpu_affinity.unwrap_or_else(|| self.find_least_loaded_cpu());
        process.mark_ready();
        self.enqueue(target_cpu, process);
        
        self.spawn_count.fetch_add(1, Ordering::Relaxed);
        
        klog_debug(
            "sched",
            &alloc::format!("Spawned daemon '{}' (PID {}) on CPU {}", name, pid, target_cpu),
        );
        
        // Wake target CPU
        if target_cpu != 0 {
            crate::send_ipi(target_cpu);
        }
        
        pid
    }

    /// Allocate a PID without spawning (for kernel-integrated services)
    pub fn allocate_pid(&self) -> Pid {
        allocate_pid()
    }

    // ─── Queue Management ───────────────────────────────────────────────────

    /// Enqueue a process on a specific CPU's run queue
    fn enqueue(&self, cpu_id: usize, process: Arc<Process>) {
        let cpu = cpu_id.min(self.num_cpus() - 1);
        self.queues[cpu].lock().enqueue(process);
    }

    /// Pick next process to run on a CPU
    pub fn pick_next(&self, cpu_id: usize) -> Option<Arc<Process>> {
        // First try our own queue
        if let Some(process) = self.queues[cpu_id].lock().dequeue() {
            return Some(process);
        }
        
        // Try work stealing from other CPUs
        let num_cpus = self.num_cpus();
        for other_cpu in 0..num_cpus {
            if other_cpu != cpu_id {
                if let Some(process) = self.queues[other_cpu].lock().steal() {
                    // Check if process can run on this CPU
                    if process.can_run_on_cpu(cpu_id) {
                        klog_trace(
                            "sched",
                            &alloc::format!(
                                "CPU {} stole '{}' from CPU {}",
                                cpu_id, process.name, other_cpu
                            ),
                        );
                        return Some(process);
                    } else {
                        // Put it back, wrong affinity
                        self.queues[other_cpu].lock().enqueue(process);
                    }
                }
            }
        }
        
        None
    }

    /// Re-queue a process after its time slice expires
    pub fn requeue(&self, process: Arc<Process>, cpu_id: usize) {
        process.mark_ready();
        self.enqueue(cpu_id, process);
    }

    // ─── Load Balancing ─────────────────────────────────────────────────────

    /// Find the least loaded CPU for spawning a new process
    pub fn find_least_loaded_cpu(&self) -> usize {
        let num_cpus = self.num_cpus();
        if num_cpus == 1 {
            return 0;
        }
        
        // First, try to find an idle non-BSP CPU
        for cpu_id in 1..num_cpus {
            if let Some(cpu) = CPU_TABLE.get(cpu_id) {
                if cpu.is_online() && cpu.is_idle() {
                    let queue_len = self.queues[cpu_id].lock().len();
                    if queue_len == 0 {
                        return cpu_id;
                    }
                }
            }
        }
        
        // Find CPU with shortest queue (prefer non-BSP)
        let mut best_cpu = 0;
        let mut min_load = self.queues[0].lock().len();
        
        for cpu_id in 1..num_cpus {
            if let Some(cpu) = CPU_TABLE.get(cpu_id) {
                if !cpu.is_online() {
                    continue;
                }
            }
            
            let load = self.queues[cpu_id].lock().len();
            // Prefer non-BSP with same or lower load
            if load < min_load || (load == min_load && cpu_id != 0) {
                min_load = load;
                best_cpu = cpu_id;
            }
        }
        
        best_cpu
    }

    /// Get queue length for a CPU
    pub fn queue_length(&self, cpu_id: usize) -> usize {
        if cpu_id < MAX_HARTS {
            self.queues[cpu_id].lock().len()
        } else {
            0
        }
    }

    /// Get total queued processes
    pub fn total_queued(&self) -> usize {
        let num_cpus = self.num_cpus();
        (0..num_cpus).map(|cpu| self.queue_length(cpu)).sum()
    }

    // ─── Process Management ─────────────────────────────────────────────────

    /// Get a process by PID
    pub fn get_process(&self, pid: Pid) -> Option<Arc<Process>> {
        PROCESS_TABLE.get(pid)
    }

    /// Kill a process by PID
    pub fn kill(&self, pid: Pid) -> bool {
        if let Some(process) = PROCESS_TABLE.get(pid) {
            process.mark_exited(137); // SIGKILL-like
            
            // Remove from run queues
            let num_cpus = self.num_cpus();
            for cpu_id in 0..num_cpus {
                self.queues[cpu_id].lock().remove(pid);
            }
            
            // If daemon with restart, let it restart
            if !process.should_restart() {
                PROCESS_TABLE.unregister(pid);
            }
            
            klog_info(
                "sched",
                &alloc::format!("Killed process '{}' (PID {})", process.name, pid),
            );
            
            true
        } else {
            false
        }
    }

    /// Complete a process (exit with code)
    pub fn exit(&self, pid: Pid, exit_code: usize) {
        if let Some(process) = PROCESS_TABLE.get(pid) {
            process.mark_exited(exit_code);
            
            klog_debug(
                "sched",
                &alloc::format!(
                    "Process '{}' (PID {}) exited with code {}",
                    process.name, pid, exit_code
                ),
            );
            
            // Handle daemon restart
            if process.should_restart() {
                let name = process.name.clone();
                let entry = process.entry;
                let priority = process.priority;
                
                klog_info(
                    "sched",
                    &alloc::format!("Restarting daemon '{}'", name),
                );
                
                self.spawn_daemon(&name, entry, priority);
            }
        }
    }

    /// Reap zombie processes
    pub fn reap_zombies(&self) -> usize {
        PROCESS_TABLE.reap_zombies().len()
    }

    // ─── Information ────────────────────────────────────────────────────────

    /// List all processes
    pub fn list_processes(&self) -> Vec<ProcessInfo> {
        let current_time = crate::get_time_ms() as u64;
        PROCESS_TABLE
            .list()
            .iter()
            .map(|p| p.info(current_time))
            .collect()
    }

    /// Get process count
    pub fn process_count(&self) -> usize {
        PROCESS_TABLE.count()
    }

    /// Get spawn count
    pub fn spawn_count(&self) -> usize {
        self.spawn_count.load(Ordering::Relaxed)
    }
}

/// Global scheduler instance
pub static SCHEDULER: Scheduler = Scheduler::new();

// ═══════════════════════════════════════════════════════════════════════════════
// CONVENIENCE FUNCTIONS
// ═══════════════════════════════════════════════════════════════════════════════

/// Initialize the scheduler
pub fn init(num_cpus: usize) {
    SCHEDULER.init(num_cpus);
}

/// Spawn a process
pub fn spawn(name: &str, entry: ProcessEntry, priority: Priority) -> Pid {
    SCHEDULER.spawn(name, entry, priority)
}

/// Spawn a daemon
pub fn spawn_daemon(name: &str, entry: ProcessEntry, priority: Priority) -> Pid {
    SCHEDULER.spawn_daemon(name, entry, priority)
}

/// Get next process for a CPU to run
pub fn pick_next(cpu_id: usize) -> Option<Arc<Process>> {
    SCHEDULER.pick_next(cpu_id)
}

/// Re-queue a process
pub fn requeue(process: Arc<Process>, cpu_id: usize) {
    SCHEDULER.requeue(process, cpu_id)
}

/// Kill a process
pub fn kill(pid: Pid) -> bool {
    SCHEDULER.kill(pid)
}

/// List all processes
pub fn list_processes() -> Vec<ProcessInfo> {
    SCHEDULER.list_processes()
}

// ═══════════════════════════════════════════════════════════════════════════════
// CONTEXT SWITCHING & YIELDING
// ═══════════════════════════════════════════════════════════════════════════════

/// Flag indicating if a yield was requested from interrupt context
static YIELD_PENDING: [AtomicBool; MAX_HARTS] = {
    const INIT: AtomicBool = AtomicBool::new(false);
    [INIT; MAX_HARTS]
};

/// Voluntarily yield the CPU to another process.
///
/// This should be called by processes that want to give up their time slice.
/// For cooperative multitasking, processes should call this periodically.
pub fn yield_now() {
    let hart_id = crate::get_hart_id();
    
    // Get current process if any
    if let Some(cpu) = crate::cpu::CPU_TABLE.get(hart_id) {
        if let Some(pid) = cpu.running_process() {
            if let Some(process) = SCHEDULER.get_process(pid) {
                // Mark process as ready (it's voluntarily yielding)
                process.mark_ready();
                
                // Requeue the process
                SCHEDULER.requeue(process, hart_id);
            }
        }
    }
    
    // The actual context switch happens in the hart loop
    // This function just marks the yield as pending
    if hart_id < MAX_HARTS {
        YIELD_PENDING[hart_id].store(true, core::sync::atomic::Ordering::Release);
    }
}

/// Called from interrupt handler to request a yield.
///
/// This is used by the timer interrupt to trigger preemption.
/// The actual context switch is deferred until it's safe.
pub fn yield_from_interrupt() {
    let hart_id = crate::get_hart_id();
    
    if hart_id < MAX_HARTS {
        YIELD_PENDING[hart_id].store(true, core::sync::atomic::Ordering::Release);
    }
}

/// Yield the current process and switch back to the scheduler.
///
/// This performs an actual context switch:
/// 1. Saves the current process's registers to its Context
/// 2. Restores the scheduler's registers from its Context
/// 3. Returns to the scheduler's hart_loop to pick the next process
///
/// Called from:
/// - Timer interrupt (preemptive scheduling)
/// - Process voluntarily yielding (cooperative scheduling)
///
/// # Safety
/// Only safe to call from the current hart's running process context.
pub fn yield_current() {
    let hart_id = crate::get_hart_id();
    
    // Get current process and CPU contexts
    if let Some(cpu) = crate::cpu::CPU_TABLE.get(hart_id) {
        if let Some(pid) = cpu.running_process() {
            if let Some(process) = cpu::process::PROCESS_TABLE.get(pid) {
                let scheduler_ctx = cpu.scheduler_context_ptr();
                let process_ctx = process.context_ptr();
                
                // Mark process as ready (can be scheduled again)
                process.mark_ready();
                
                // Switch from process context back to scheduler context
                // This saves all callee-saved registers to process_ctx
                // and restores them from scheduler_ctx, then returns to
                // wherever the scheduler was when it switched to us.
                unsafe {
                    cpu::process::switch_context(process_ctx, scheduler_ctx);
                }
                
                // We return here when the scheduler switches back to us
            }
        }
    }
}

/// Check if a yield is pending for this hart
pub fn yield_pending(hart_id: usize) -> bool {
    if hart_id < MAX_HARTS {
        YIELD_PENDING[hart_id].swap(false, core::sync::atomic::Ordering::AcqRel)
    } else {
        false
    }
}

/// Clear yield pending flag
pub fn clear_yield_pending(hart_id: usize) {
    if hart_id < MAX_HARTS {
        YIELD_PENDING[hart_id].store(false, core::sync::atomic::Ordering::Release);
    }
}
