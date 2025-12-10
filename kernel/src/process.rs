//! Process Management
//!
//! This module provides the core process abstraction for the kernel.
//! A process is the fundamental unit of execution that can be scheduled
//! across any available CPU (hart).
//!
//! ## Design Philosophy
//!
//! - **Process**: A schedulable unit of execution with its own state
//! - **Hart/CPU**: Hardware execution units (Web Workers in browser = additional CPUs)
//! - **Scheduler**: Distributes processes across available CPUs
//!
//! Web Workers are NOT tasks/processes - they are additional CPUs (harts) that
//! can execute processes. This is analogous to how Linux treats threads/cores.
//!
//! ## Process Lifecycle
//!
//! ```text
//! Created -> Ready -> Running -> Ready (time slice) -> Running -> Zombie
//!              ↓         ↓
//!          Blocked    Exit
//! ```

use crate::Spinlock;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

// Include the context switch assembly
core::arch::global_asm!(include_str!("switch_context.S"));

// External declaration for the context switch function
extern "C" {
    /// Switch from the current context to a new context.
    ///
    /// Saves all callee-saved registers to `old` and loads them from `new`.
    /// Returns when another context switches back to us.
    ///
    /// # Safety
    /// Both pointers must be valid Context structures.
    /// The new context must have a valid stack pointer and return address.
    pub fn switch_context(old: *mut Context, new: *mut Context);
}

// ═══════════════════════════════════════════════════════════════════════════════
// PROCESS IDENTIFIERS
// ═══════════════════════════════════════════════════════════════════════════════

/// Process identifier type
pub type Pid = u32;

/// Next PID counter
static NEXT_PID: AtomicU32 = AtomicU32::new(1);

/// Allocate a new unique PID
pub fn allocate_pid() -> Pid {
    NEXT_PID.fetch_add(1, Ordering::SeqCst)
}

// ═══════════════════════════════════════════════════════════════════════════════
// PROCESS STATE
// ═══════════════════════════════════════════════════════════════════════════════

/// Process states following Unix conventions
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum ProcessState {
    /// Process is being created
    Created = 0,
    /// Process is runnable, waiting for CPU time
    Ready = 1,
    /// Process is currently executing on a CPU
    Running = 2,
    /// Process is blocked/sleeping (waiting for I/O, timer, etc.)
    Blocked = 3,
    /// Process has been stopped (can be resumed with signal)
    Stopped = 4,
    /// Process has terminated, waiting for parent to reap
    Zombie = 5,
}

impl ProcessState {
    pub fn from_u8(val: u8) -> Self {
        match val {
            0 => ProcessState::Created,
            1 => ProcessState::Ready,
            2 => ProcessState::Running,
            3 => ProcessState::Blocked,
            4 => ProcessState::Stopped,
            _ => ProcessState::Zombie,
        }
    }

    /// Single character state code (like ps output)
    pub fn code(&self) -> &'static str {
        match self {
            ProcessState::Created => "C",
            ProcessState::Ready => "R",      // Runnable, waiting for CPU
            ProcessState::Running => "R+",   // Actually running on a CPU (like ps foreground)
            ProcessState::Blocked => "S",
            ProcessState::Stopped => "T",
            ProcessState::Zombie => "Z",
        }
    }

    /// Whether the process is runnable (can be scheduled)
    pub fn is_runnable(&self) -> bool {
        matches!(self, ProcessState::Ready)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// PROCESS PRIORITY
// ═══════════════════════════════════════════════════════════════════════════════

/// Process priority levels
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(u8)]
pub enum Priority {
    /// Lowest priority - idle tasks
    Idle = 0,
    /// Background work
    Low = 1,
    /// Default priority for user processes
    Normal = 2,
    /// System services
    High = 3,
    /// Real-time/critical processes
    Realtime = 4,
}

impl Priority {
    pub fn from_u8(val: u8) -> Self {
        match val {
            0 => Priority::Idle,
            1 => Priority::Low,
            2 => Priority::Normal,
            3 => Priority::High,
            _ => Priority::Realtime,
        }
    }
}

impl Default for Priority {
    fn default() -> Self {
        Priority::Normal
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// PROCESS FLAGS
// ═══════════════════════════════════════════════════════════════════════════════

bitflags::bitflags! {
    /// Process flags
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct ProcessFlags: u32 {
        /// Process is a kernel thread (runs in kernel space)
        const KERNEL = 1 << 0;
        /// Process is a system daemon
        const DAEMON = 1 << 1;
        /// Process should restart on exit
        const RESTART_ON_EXIT = 1 << 2;
        /// Process has CPU affinity set
        const CPU_AFFINITY = 1 << 3;
        /// Process is the init process (PID 1)
        const INIT = 1 << 4;
        /// Process is currently in a syscall
        const IN_SYSCALL = 1 << 5;
    }
}

impl Default for ProcessFlags {
    fn default() -> Self {
        ProcessFlags::empty()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// CONTEXT (CPU Register State for Context Switching)
// ═══════════════════════════════════════════════════════════════════════════════

/// Default kernel stack size per process (4KB)
pub const KSTACK_SIZE: usize = 4096;

/// Saved CPU context for context switching.
///
/// On RISC-V, we save all callee-saved registers plus ra (return address)
/// and sp (stack pointer). The caller-saved registers are saved by the
/// caller before calling switch_context, so we don't need them here.
///
/// Layout matches what switch_context.S expects.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Context {
    /// Return address (where to resume)
    pub ra: u64,
    /// Stack pointer
    pub sp: u64,
    /// Callee-saved registers s0-s11
    pub s: [u64; 12],
}

impl Context {
    /// Create a zeroed context
    pub const fn zero() -> Self {
        Self {
            ra: 0,
            sp: 0,
            s: [0; 12],
        }
    }

    /// Create a new context ready to start executing at `entry` with stack `sp`
    pub fn new(entry: u64, sp: u64) -> Self {
        Self {
            ra: entry,
            sp,
            s: [0; 12],
        }
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::zero()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// PROCESS CONTROL BLOCK (PCB)
// ═══════════════════════════════════════════════════════════════════════════════

/// Process entry point function type
pub type ProcessEntry = fn();

/// Process Control Block - the kernel's representation of a process
///
/// This is analogous to Linux's `task_struct` but simplified for our needs.
/// Each process has its own PCB that tracks all execution-related state.
#[repr(align(64))] // Cache line alignment for multi-CPU access
pub struct Process {
    // ─── Identity ───────────────────────────────────────────────────────────
    /// Unique process identifier
    pub pid: Pid,
    /// Human-readable process name
    pub name: String,
    /// Parent process ID (0 for init)
    pub ppid: Pid,

    // ─── Scheduling ─────────────────────────────────────────────────────────
    /// Current process state (atomic for cross-CPU visibility)
    state: AtomicUsize,
    /// Process priority
    pub priority: Priority,
    /// CPU affinity (-1 = any CPU, else specific CPU ID)
    pub cpu_affinity: AtomicUsize,
    /// CPU currently executing this process (usize::MAX if not running)
    pub current_cpu: AtomicUsize,

    // ─── Execution ──────────────────────────────────────────────────────────
    /// Process entry point
    pub entry: ProcessEntry,
    /// Process flags
    pub flags: ProcessFlags,
    /// Exit code (valid when Zombie)
    pub exit_code: AtomicUsize,

    // ─── Context Switching ───────────────────────────────────────────────────
    /// Saved CPU context (registers) for context switching.
    /// Wrapped in UnsafeCell because it's mutated during context switch
    /// while the Process is behind an Arc.
    pub context: UnsafeCell<Context>,
    /// Kernel stack for this process (heap allocated)
    /// The top of this stack is used as SP when context switching into the process.
    pub kstack: Option<Box<[u8; KSTACK_SIZE]>>,

    // ─── Statistics ─────────────────────────────────────────────────────────
    /// Creation timestamp (ms since boot)
    pub created_at: u64,
    /// Total CPU time consumed (ms)
    pub cpu_time_ms: AtomicU64,
    /// Number of times scheduled
    pub schedule_count: AtomicU64,
}

// SAFETY: Process uses UnsafeCell for context, but context is only accessed
// during context switch which is synchronized by scheduler state transitions.
// The scheduler ensures only one CPU accesses a process's context at a time.
unsafe impl Sync for Process {}

impl Process {
    /// Get a raw pointer to the context for use in context switching.
    ///
    /// # Safety
    /// The caller must ensure exclusive access during context switch.
    /// This is enforced by the scheduler: only the CPU that picks a process
    /// can switch into it, and only while it's marked Running.
    #[inline]
    pub fn context_ptr(&self) -> *mut Context {
        self.context.get()
    }

    /// Create a new process with its own kernel stack.
    ///
    /// The process context is initialized so that when switched to,
    /// it will start executing at the entry point function.
    pub fn new(pid: Pid, name: &str, entry: ProcessEntry) -> Self {
        // Allocate kernel stack
        let kstack = Box::new([0u8; KSTACK_SIZE]);
        // Stack grows down on RISC-V, so SP points to top of stack
        let stack_top = kstack.as_ptr() as u64 + KSTACK_SIZE as u64;
        // Initialize context to start at entry function with proper stack
        let context = Context::new(entry as u64, stack_top);

        Self {
            pid,
            name: String::from(name),
            ppid: 0,
            state: AtomicUsize::new(ProcessState::Created as usize),
            priority: Priority::Normal,
            cpu_affinity: AtomicUsize::new(usize::MAX), // Any CPU
            current_cpu: AtomicUsize::new(usize::MAX),  // Not running
            entry,
            flags: ProcessFlags::empty(),
            exit_code: AtomicUsize::new(0),
            context: UnsafeCell::new(context),
            kstack: Some(kstack),
            created_at: crate::get_time_ms() as u64,
            cpu_time_ms: AtomicU64::new(0),
            schedule_count: AtomicU64::new(0),
        }
    }

    /// Create a new kernel process (daemon)
    pub fn new_kernel(pid: Pid, name: &str, entry: ProcessEntry) -> Self {
        let mut proc = Self::new(pid, name, entry);
        proc.flags = ProcessFlags::KERNEL | ProcessFlags::DAEMON;
        proc.priority = Priority::High;
        proc
    }

    /// Create a daemon process that restarts on exit
    pub fn new_daemon(pid: Pid, name: &str, entry: ProcessEntry) -> Self {
        let mut proc = Self::new(pid, name, entry);
        proc.flags = ProcessFlags::DAEMON | ProcessFlags::RESTART_ON_EXIT;
        proc.priority = Priority::Normal;
        proc
    }

    // ─── State Management ───────────────────────────────────────────────────

    /// Get current process state
    #[inline]
    pub fn state(&self) -> ProcessState {
        ProcessState::from_u8(self.state.load(Ordering::Acquire) as u8)
    }

    /// Set process state
    #[inline]
    pub fn set_state(&self, state: ProcessState) {
        self.state.store(state as usize, Ordering::Release);
    }

    /// Mark process as ready to run
    pub fn mark_ready(&self) {
        self.set_state(ProcessState::Ready);
    }

    /// Mark process as running on specified CPU
    pub fn mark_running(&self, cpu_id: usize) {
        self.current_cpu.store(cpu_id, Ordering::Release);
        self.set_state(ProcessState::Running);
        self.schedule_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Mark process as blocked
    pub fn mark_blocked(&self) {
        self.current_cpu.store(usize::MAX, Ordering::Release);
        self.set_state(ProcessState::Blocked);
    }

    /// Mark process as exited
    pub fn mark_exited(&self, exit_code: usize) {
        self.exit_code.store(exit_code, Ordering::Release);
        self.current_cpu.store(usize::MAX, Ordering::Release);
        self.set_state(ProcessState::Zombie);
    }

    // ─── CPU Affinity ───────────────────────────────────────────────────────

    /// Set CPU affinity (restrict to specific CPU)
    pub fn set_cpu_affinity(&self, cpu_id: usize) {
        self.cpu_affinity.store(cpu_id, Ordering::Release);
    }

    /// Clear CPU affinity (can run on any CPU)
    pub fn clear_cpu_affinity(&self) {
        self.cpu_affinity.store(usize::MAX, Ordering::Release);
    }

    /// Get CPU affinity (usize::MAX = any)
    pub fn get_cpu_affinity(&self) -> Option<usize> {
        let affinity = self.cpu_affinity.load(Ordering::Acquire);
        if affinity == usize::MAX {
            None
        } else {
            Some(affinity)
        }
    }

    /// Check if process can run on specified CPU
    pub fn can_run_on_cpu(&self, cpu_id: usize) -> bool {
        let affinity = self.cpu_affinity.load(Ordering::Acquire);
        affinity == usize::MAX || affinity == cpu_id
    }

    // ─── Statistics ─────────────────────────────────────────────────────────

    /// Add CPU time
    pub fn add_cpu_time(&self, ms: u64) {
        self.cpu_time_ms.fetch_add(ms, Ordering::Relaxed);
    }

    /// Get total CPU time
    pub fn cpu_time(&self) -> u64 {
        self.cpu_time_ms.load(Ordering::Relaxed)
    }

    /// Get current CPU (usize::MAX if not running)
    pub fn current_cpu(&self) -> Option<usize> {
        let cpu = self.current_cpu.load(Ordering::Acquire);
        if cpu == usize::MAX {
            None
        } else {
            Some(cpu)
        }
    }

    // ─── Flags ──────────────────────────────────────────────────────────────

    /// Check if this is a kernel process
    pub fn is_kernel(&self) -> bool {
        self.flags.contains(ProcessFlags::KERNEL)
    }

    /// Check if this is a daemon
    pub fn is_daemon(&self) -> bool {
        self.flags.contains(ProcessFlags::DAEMON)
    }

    /// Check if process should restart on exit
    pub fn should_restart(&self) -> bool {
        self.flags.contains(ProcessFlags::RESTART_ON_EXIT)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// PROCESS INFO (for reporting)
// ═══════════════════════════════════════════════════════════════════════════════

/// Snapshot of process information for reporting
#[derive(Clone)]
pub struct ProcessInfo {
    pub pid: Pid,
    pub ppid: Pid,
    pub name: String,
    pub state: ProcessState,
    pub priority: Priority,
    pub cpu: Option<usize>,
    pub cpu_time_ms: u64,
    pub uptime_ms: u64,
    pub flags: ProcessFlags,
}

impl Process {
    /// Get a snapshot of process info
    pub fn info(&self, current_time: u64) -> ProcessInfo {
        ProcessInfo {
            pid: self.pid,
            ppid: self.ppid,
            name: self.name.clone(),
            state: self.state(),
            priority: self.priority,
            cpu: self.current_cpu(),
            cpu_time_ms: self.cpu_time(),
            uptime_ms: current_time.saturating_sub(self.created_at),
            flags: self.flags,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// PROCESS TABLE
// ═══════════════════════════════════════════════════════════════════════════════

/// Global process table
pub struct ProcessTable {
    /// All processes indexed by PID
    processes: Spinlock<BTreeMap<Pid, Arc<Process>>>,
}

impl ProcessTable {
    /// Create a new process table
    pub const fn new() -> Self {
        Self {
            processes: Spinlock::new(BTreeMap::new()),
        }
    }

    /// Register a new process
    pub fn register(&self, process: Arc<Process>) {
        self.processes.lock().insert(process.pid, process);
    }

    /// Unregister a process
    pub fn unregister(&self, pid: Pid) -> Option<Arc<Process>> {
        self.processes.lock().remove(&pid)
    }

    /// Get a process by PID
    pub fn get(&self, pid: Pid) -> Option<Arc<Process>> {
        self.processes.lock().get(&pid).cloned()
    }

    /// List all processes
    pub fn list(&self) -> Vec<Arc<Process>> {
        self.processes.lock().values().cloned().collect()
    }

    /// Get process count
    pub fn count(&self) -> usize {
        self.processes.lock().len()
    }

    /// Find processes matching a predicate
    pub fn find<F>(&self, predicate: F) -> Vec<Arc<Process>>
    where
        F: Fn(&Process) -> bool,
    {
        self.processes
            .lock()
            .values()
            .filter(|p| predicate(p))
            .cloned()
            .collect()
    }

    /// Reap zombie processes (remove from table)
    pub fn reap_zombies(&self) -> Vec<Pid> {
        let mut processes = self.processes.lock();
        let zombies: Vec<Pid> = processes
            .iter()
            .filter(|(_, p)| {
                p.state() == ProcessState::Zombie && !p.should_restart()
            })
            .map(|(pid, _)| *pid)
            .collect();

        for pid in &zombies {
            processes.remove(pid);
        }

        zombies
    }
}

/// Global process table instance
pub static PROCESS_TABLE: ProcessTable = ProcessTable::new();

// ═══════════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_entry() {}

    #[test]
    fn test_process_creation() {
        let proc = Process::new(1, "test", dummy_entry);
        assert_eq!(proc.pid, 1);
        assert_eq!(proc.name, "test");
        assert_eq!(proc.state(), ProcessState::Created);
    }

    #[test]
    fn test_process_state_transitions() {
        let proc = Process::new(1, "test", dummy_entry);
        
        proc.mark_ready();
        assert_eq!(proc.state(), ProcessState::Ready);
        
        proc.mark_running(0);
        assert_eq!(proc.state(), ProcessState::Running);
        assert_eq!(proc.current_cpu(), Some(0));
        
        proc.mark_blocked();
        assert_eq!(proc.state(), ProcessState::Blocked);
        assert_eq!(proc.current_cpu(), None);
        
        proc.mark_exited(42);
        assert_eq!(proc.state(), ProcessState::Zombie);
    }

    #[test]
    fn test_cpu_affinity() {
        let proc = Process::new(1, "test", dummy_entry);
        
        // Default: can run anywhere
        assert!(proc.can_run_on_cpu(0));
        assert!(proc.can_run_on_cpu(1));
        assert!(proc.can_run_on_cpu(7));
        
        // Set affinity
        proc.set_cpu_affinity(2);
        assert!(!proc.can_run_on_cpu(0));
        assert!(!proc.can_run_on_cpu(1));
        assert!(proc.can_run_on_cpu(2));
        
        // Clear affinity
        proc.clear_cpu_affinity();
        assert!(proc.can_run_on_cpu(0));
    }
}
