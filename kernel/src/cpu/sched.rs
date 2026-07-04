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
//! Each hart owns three lock-free Chase-Lev work-stealing deques, one per
//! priority tier (High / Normal / Low). They are the *only* run-queue
//! structure: push, pop and steal are all O(1) and lock-free. Per-hart
//! queue lengths are tracked in plain atomics so load balancing never takes
//! a lock, and floating daemons only migrate when the local queue is
//! meaningfully longer than the least-loaded hart (hysteresis).
//!
//! ```text
//! hart 0: [High][Normal][Low]   <- owner pops LIFO
//! hart 1: [High][Normal][Low]   <- thieves steal FIFO (lock-free CAS)
//! hart N: [High][Normal][Low]
//! ```
//!
//! Killed processes are removed lazily: `kill` marks them Zombie and the
//! next pop discards them. Blocked (non-runnable) processes popped from a
//! queue are moved to a small per-hart parked list and re-enqueued when
//! they become Ready again (wait queues call `notify_woken` so the owning
//! hart is IPI'd immediately).

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::Spinlock;
use crate::cpu::chase_lev::{StealResult, WorkStealingDeque};
use crate::cpu::process::{
    PROCESS_TABLE, Pid, Priority, Process, ProcessEntry, ProcessInfo, ProcessState, allocate_pid,
};
use crate::cpu::{CPU_TABLE, MAX_HARTS};
use crate::services::klogd::{klog_debug, klog_info};

/// Number of priority tiers (High+Realtime / Normal / Low+Idle).
const NUM_TIERS: usize = 3;

/// A floating daemon migrates only when the local queue exceeds the least
/// loaded hart by more than this many entries (prevents ping-ponging).
const MIGRATION_THRESHOLD: usize = 2;

/// Map a process priority onto a queue tier (0 = highest).
#[inline]
fn tier_of(priority: Priority) -> usize {
    match priority {
        Priority::Realtime | Priority::High => 0,
        Priority::Normal => 1,
        Priority::Low | Priority::Idle => 2,
    }
}

/// Simple LCG for randomized victim selection
static RNG_STATE: AtomicU64 = AtomicU64::new(0xDEADBEEF_CAFEBABE);

#[inline]
fn next_random() -> usize {
    // Linear Congruential Generator (fast, good enough for victim selection)
    let old = RNG_STATE.load(Ordering::Relaxed);
    let new = old
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    RNG_STATE.store(new, Ordering::Relaxed);
    (new >> 33) as usize
}

/// The process scheduler
pub struct Scheduler {
    /// Per-hart, per-tier lock-free work-stealing deques for *floating*
    /// (no-affinity) processes. Thieves may steal from these.
    queues: [[WorkStealingDeque<Arc<Process>>; NUM_TIERS]; MAX_HARTS],

    /// Per-hart local queue for CPU-pinned processes. Only the owning hart
    /// pops from it; thieves never touch it. Without this, a pinned daemon
    /// (e.g. shelld) is repeatedly stolen by idle harts, found un-runnable
    /// (wrong affinity), and bounced back - starving its owner hart. This
    /// is why interactive input stalled with 3+ idle harts.
    local: [WorkStealingDeque<Arc<Process>>; MAX_HARTS],

    /// Per-hart total queued entries (approximate; maintained at push/pop).
    /// Read lock-free by load balancing.
    queue_lens: [AtomicUsize; MAX_HARTS],

    /// Per-hart parked processes: popped from a queue while not runnable
    /// (Blocked/Stopped/Created) or sleeping until a deadline. Swept back
    /// into the run queue by the owning hart once Ready and due.
    parked: [Spinlock<Vec<Arc<Process>>>; MAX_HARTS],

    /// Earliest next_run_at deadline among this hart's parked processes
    /// (u64::MAX when none). Lets the idle path program a long timer
    /// without oversleeping a daemon's period.
    parked_deadline: [AtomicU64; MAX_HARTS],

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
        const INIT_TIER: WorkStealingDeque<Arc<Process>> = WorkStealingDeque::new();
        const INIT_HART: [WorkStealingDeque<Arc<Process>>; NUM_TIERS] = [INIT_TIER; NUM_TIERS];
        const INIT_LOCAL: WorkStealingDeque<Arc<Process>> = WorkStealingDeque::new();
        const INIT_LEN: AtomicUsize = AtomicUsize::new(0);
        const INIT_PARKED: Spinlock<Vec<Arc<Process>>> = Spinlock::new(Vec::new());
        const INIT_DEADLINE: AtomicU64 = AtomicU64::new(u64::MAX);

        Self {
            queues: [INIT_HART; MAX_HARTS],
            local: [INIT_LOCAL; MAX_HARTS],
            queue_lens: [INIT_LEN; MAX_HARTS],
            parked: [INIT_PARKED; MAX_HARTS],
            parked_deadline: [INIT_DEADLINE; MAX_HARTS],
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

        pid
    }

    /// Allocate a PID without spawning (for kernel-integrated services)
    pub fn allocate_pid(&self) -> Pid {
        allocate_pid()
    }

    // ─── Queue Management ───────────────────────────────────────────────────

    /// Enqueue a process on a specific CPU's run queue.
    ///
    /// Lock-free: one deque push + one atomic increment. An IPI is sent only
    /// when the target hart's queue was empty (it may be sleeping in WFI);
    /// busy harts will naturally pick the work up on their next loop.
    fn enqueue(&self, cpu_id: usize, process: Arc<Process>) {
        let cpu = cpu_id.min(self.num_cpus() - 1);
        let pinned = process.get_cpu_affinity().is_some();
        let tier = tier_of(process.priority);

        // Increment the length BEFORE the push: a concurrent thief could
        // otherwise pop the entry and decrement before our increment,
        // underflowing the counter.
        let was_empty = self.queue_lens[cpu].fetch_add(1, Ordering::Release) == 0;
        if pinned {
            // Pinned processes go to the owner-only local queue so thieves
            // never steal-and-bounce them (which starves the owner hart).
            self.local[cpu].push(process);
        } else {
            self.queues[cpu][tier].push(process);
        }

        let current_hart = crate::get_hart_id();
        if cpu != current_hart && was_empty {
            crate::send_ipi(cpu);
        }
    }

    /// Park a process on this hart (blocked, or sleeping until deadline).
    fn park(&self, cpu_id: usize, process: Arc<Process>) {
        process.set_park_hart(cpu_id);
        let deadline = if process.state().is_runnable() {
            process.next_run_at()
        } else {
            u64::MAX // Blocked: woken explicitly via notify_woken
        };
        self.parked_deadline[cpu_id].fetch_min(deadline, Ordering::AcqRel);
        self.parked[cpu_id].lock().push(process);
    }

    /// Sweep this hart's parked list, re-enqueuing processes that became
    /// Ready and due. Cold-ish path: bounded by the number of parked
    /// daemons, guarded by one cheap lock only this hart touches.
    fn sweep_parked(&self, cpu_id: usize) {
        let now = crate::get_time_ms() as u64;
        if self.parked_deadline[cpu_id].load(Ordering::Acquire) > now {
            // Nothing due yet; blocked entries are re-admitted via the
            // deadline reset in notify_woken.
            return;
        }

        let mut woken: Vec<Arc<Process>> = Vec::new();
        {
            let mut parked = self.parked[cpu_id].lock();
            let mut min_deadline = u64::MAX;
            let mut i = 0;
            while i < parked.len() {
                let p = &parked[i];
                let state = p.state();
                if matches!(state, ProcessState::Zombie) {
                    let p = parked.swap_remove(i);
                    p.clear_park_hart();
                    drop(p);
                } else if state.is_runnable() && p.next_run_at() <= now {
                    let p = parked.swap_remove(i);
                    p.clear_park_hart();
                    woken.push(p);
                } else {
                    if state.is_runnable() {
                        min_deadline = min_deadline.min(p.next_run_at());
                    }
                    i += 1;
                }
            }
            self.parked_deadline[cpu_id].store(min_deadline, Ordering::Release);
        }
        for p in woken {
            self.enqueue(cpu_id, p);
        }
    }

    /// Try to pop a runnable, due process from this hart's own queues.
    /// Pinned (local) work is checked first, then the floating priority
    /// tiers highest-first.
    fn pop_local(&self, cpu_id: usize) -> Option<Arc<Process>> {
        let now = crate::get_time_ms() as u64;

        // Owner-only local queue (pinned processes).
        loop {
            match self.local[cpu_id].pop() {
                Some(process) => {
                    self.queue_lens[cpu_id].fetch_sub(1, Ordering::Release);
                    match process.state() {
                        s if s.is_runnable() && process.next_run_at() <= now => {
                            return Some(process);
                        }
                        ProcessState::Zombie => continue,
                        _ => {
                            self.park(cpu_id, process);
                            continue;
                        }
                    }
                }
                None => break,
            }
        }

        // Floating priority tiers.
        for tier in 0..NUM_TIERS {
            loop {
                match self.queues[cpu_id][tier].pop() {
                    Some(process) => {
                        self.queue_lens[cpu_id].fetch_sub(1, Ordering::Release);
                        match process.state() {
                            s if s.is_runnable() && process.next_run_at() <= now => {
                                return Some(process);
                            }
                            ProcessState::Zombie => {
                                // Killed while queued - discard lazily.
                                continue;
                            }
                            _ => {
                                // Blocked, or sleeping until a deadline.
                                self.park(cpu_id, process);
                                continue;
                            }
                        }
                    }
                    None => break,
                }
            }
        }
        None
    }

    /// Pick next process to run on a CPU
    pub fn pick_next(&self, cpu_id: usize) -> Option<Arc<Process>> {
        // Re-admit any parked processes that were woken.
        self.sweep_parked(cpu_id);

        // Own queues first (priority-ordered, LIFO for cache warmth).
        if let Some(process) = self.pop_local(cpu_id) {
            crate::perfstat::inc(crate::perfstat::id::SCHED_PICKS);
            return Some(process);
        }

        // Work stealing with randomized victim selection.
        let num_cpus = self.num_cpus();
        if num_cpus <= 1 {
            return None;
        }

        let start = next_random() % num_cpus;

        for i in 0..num_cpus {
            let victim = (start + i) % num_cpus;
            if victim == cpu_id {
                continue;
            }
            // Cheap filter: skip victims with nothing queued.
            if self.queue_lens[victim].load(Ordering::Acquire) == 0 {
                continue;
            }

            // Steal higher tiers first. Hart 0's High tier is reserved for
            // its own latency-critical services (it also runs all I/O
            // dispatch), but its Normal/Low work may be relieved.
            let first_tier = if victim == 0 { 1 } else { 0 };
            let now = crate::get_time_ms() as u64;

            for tier in first_tier..NUM_TIERS {
                match self.queues[victim][tier].steal() {
                    StealResult::Success(process) => {
                        self.queue_lens[victim].fetch_sub(1, Ordering::Release);

                        match process.state() {
                            s if s.is_runnable() && process.next_run_at() <= now => {
                                if process.can_run_on_cpu(cpu_id) {
                                    crate::perfstat::inc(crate::perfstat::id::SCHED_PICKS);
                                    crate::perfstat::inc(crate::perfstat::id::SCHED_STEALS);
                                    return Some(process);
                                }
                                // Pinned elsewhere - give it back.
                                self.enqueue(victim, process);
                            }
                            ProcessState::Zombie => {
                                // Discard lazily.
                            }
                            _ => {
                                // Blocked or sleeping - park on the victim.
                                self.park(victim, process);
                            }
                        }
                    }
                    StealResult::Retry => {
                        // Contention - try the next victim rather than spin.
                        break;
                    }
                    StealResult::Empty => {}
                }
            }
        }

        crate::perfstat::inc(crate::perfstat::id::SCHED_STEAL_MISSES);
        None
    }

    /// Re-queue a process after its time slice expires.
    ///
    /// Pinned processes go back to their hart. Floating processes stay local
    /// unless the local queue is more than MIGRATION_THRESHOLD entries longer
    /// than the least-loaded hart (hysteresis avoids constant migration).
    pub fn requeue(&self, process: Arc<Process>, current_cpu: usize) {
        process.mark_ready();
        crate::perfstat::inc(crate::perfstat::id::SCHED_REQUEUES);

        // Sleeping daemon (called sched::sleep_current_ms): park directly
        // instead of cycling through the run queue.
        let now = crate::get_time_ms() as u64;
        if process.next_run_at() > now && process.get_cpu_affinity().is_none() {
            self.park(current_cpu, process);
            return;
        }

        let target_cpu = match process.get_cpu_affinity() {
            Some(pinned_cpu) => pinned_cpu,
            None => {
                let local_len = self.queue_lens[current_cpu].load(Ordering::Relaxed);
                if local_len == 0 {
                    current_cpu
                } else {
                    let (min_cpu, min_len) = self.least_loaded_snapshot();
                    if local_len > min_len + MIGRATION_THRESHOLD {
                        min_cpu
                    } else {
                        current_cpu
                    }
                }
            }
        };

        if target_cpu != current_cpu {
            crate::perfstat::inc(crate::perfstat::id::SCHED_MIGRATIONS);
        }
        self.enqueue(target_cpu, process);
    }

    // ─── Load Balancing ─────────────────────────────────────────────────────

    /// Lock-free snapshot of the least-loaded ready hart: (cpu, len).
    /// Prefers non-BSP harts on ties (hart 0 runs the I/O dispatcher).
    fn least_loaded_snapshot(&self) -> (usize, usize) {
        let num_cpus = self.num_cpus();
        let mut best_cpu = 0;
        let mut min_len = usize::MAX;

        for cpu_id in 0..num_cpus {
            if cpu_id != 0 && !crate::cpu::is_hart_ready(cpu_id) {
                // Skip harts that aren't scheduling yet (unless nothing else)
                if let Some(cpu) = CPU_TABLE.get(cpu_id) {
                    if !cpu.is_online() {
                        continue;
                    }
                }
            }
            let len = self.queue_lens[cpu_id].load(Ordering::Relaxed);
            if len < min_len || (len == min_len && best_cpu == 0 && cpu_id != 0) {
                min_len = len;
                best_cpu = cpu_id;
            }
        }
        (best_cpu, min_len)
    }

    /// Find the least loaded CPU for spawning a new process.
    /// Lock-free: scans per-hart atomic queue lengths.
    pub fn find_least_loaded_cpu(&self) -> usize {
        let num_cpus = self.num_cpus();
        if num_cpus == 1 {
            return 0;
        }

        // Prefer an idle, ready, non-BSP hart.
        for cpu_id in 1..num_cpus {
            if crate::cpu::is_hart_ready(cpu_id)
                && self.queue_lens[cpu_id].load(Ordering::Relaxed) == 0
            {
                return cpu_id;
            }
        }

        self.least_loaded_snapshot().0
    }

    /// Get queue length for a CPU (approximate, lock-free)
    pub fn queue_length(&self, cpu_id: usize) -> usize {
        if cpu_id < MAX_HARTS {
            self.queue_lens[cpu_id].load(Ordering::Relaxed)
        } else {
            0
        }
    }

    /// Get total queued processes (approximate, lock-free)
    pub fn total_queued(&self) -> usize {
        let num_cpus = self.num_cpus();
        (0..num_cpus).map(|cpu| self.queue_length(cpu)).sum()
    }

    // ─── Process Management ─────────────────────────────────────────────────

    /// Get a process by PID
    pub fn get_process(&self, pid: Pid) -> Option<Arc<Process>> {
        PROCESS_TABLE.get(pid)
    }

    /// Kill a process by PID.
    ///
    /// Removal from run queues is lazy: the Zombie entry is discarded the
    /// next time a hart pops it (lock-free deques cannot remove mid-queue).
    pub fn kill(&self, pid: Pid) -> bool {
        if let Some(process) = PROCESS_TABLE.get(pid) {
            process.mark_exited(137); // SIGKILL-like

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

/// Notify the scheduler that a parked (blocked) process became Ready.
///
/// Wait queues call this after mark_ready() so the hart holding the parked
/// entry wakes immediately (it may be sleeping in WFI with a long idle
/// timer) instead of at its next scheduled tick.
pub fn notify_woken(process: &Process) {
    if let Some(hart) = process.get_park_hart() {
        // A blocked entry parks with deadline u64::MAX; force the next
        // sweep to look at the list.
        process.set_next_run_at(0);
        SCHEDULER.parked_deadline[hart].store(0, Ordering::Release);
        if hart != crate::get_hart_id() {
            crate::send_ipi(hart);
        }
    }
}

/// Put the currently running daemon to sleep for at least `ms` milliseconds.
///
/// Called by service tick functions before returning: the scheduler parks
/// the process until the deadline instead of re-running it every hart-loop
/// iteration. This is what lets idle harts actually reach WFI.
pub fn sleep_current_ms(ms: u64) {
    let hart_id = crate::get_hart_id();
    if let Some(cpu) = crate::cpu::CPU_TABLE.get(hart_id) {
        if let Some(pid) = cpu.running_process() {
            if let Some(process) = PROCESS_TABLE.get(pid) {
                let now = crate::get_time_ms() as u64;
                process.set_next_run_at(now + ms);
            }
        }
    }
}

/// Milliseconds until the next parked-daemon deadline on this hart,
/// clamped to [1, max_ms]. Used by the idle path to program a long timer
/// without oversleeping periodic services.
pub fn idle_timer_delta_ms(hart_id: usize, max_ms: u64) -> u64 {
    if hart_id >= MAX_HARTS {
        return 1;
    }
    // Runnable work queued: stay at fine granularity.
    if SCHEDULER.queue_lens[hart_id].load(Ordering::Relaxed) > 0 {
        return 1;
    }
    let deadline = SCHEDULER.parked_deadline[hart_id].load(Ordering::Acquire);
    if deadline == u64::MAX {
        return max_ms;
    }
    let now = crate::get_time_ms() as u64;
    deadline.saturating_sub(now).clamp(1, max_ms)
}

// ═══════════════════════════════════════════════════════════════════════════════
// YIELDING (cooperative checkpoints)
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
        YIELD_PENDING[hart_id].store(true, Ordering::Release);
    }
}

/// Called from interrupt handler to request a yield.
///
/// This is used by the timer interrupt to trigger preemption. Long-running
/// kernel loops (io_router waits, shell command execution, wasmi slices)
/// poll `yield_pending()` as a cooperative checkpoint and return to the
/// hart loop when set.
pub fn yield_from_interrupt() {
    let hart_id = crate::get_hart_id();

    if hart_id < MAX_HARTS {
        YIELD_PENDING[hart_id].store(true, Ordering::Release);
    }
}

/// Check if a yield is pending for this hart (consumes the flag)
pub fn yield_pending(hart_id: usize) -> bool {
    if hart_id < MAX_HARTS {
        YIELD_PENDING[hart_id].swap(false, Ordering::AcqRel)
    } else {
        false
    }
}

/// Clear yield pending flag
pub fn clear_yield_pending(hart_id: usize) {
    if hart_id < MAX_HARTS {
        YIELD_PENDING[hart_id].store(false, Ordering::Release);
    }
}
