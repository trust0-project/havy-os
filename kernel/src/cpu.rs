//! CPU (Hart) Management
//!
//! This module provides the abstraction for CPU cores (harts in RISC-V terminology).
//! In the browser environment, each Web Worker represents an additional CPU.
//!
//! ## Terminology
//!
//! - **CPU/Hart**: A hardware thread that can execute instructions
//! - **Process**: A schedulable unit of work (see `process.rs`)
//! - **Web Worker**: Browser's mechanism for parallelism (= additional CPU)
//!
//! ## Design
//!
//! The kernel maintains a CPU table that tracks:
//! - Which CPUs are online and available
//! - What process (if any) each CPU is running
//! - CPU statistics (idle time, instructions, etc.)
//!
//! Web Workers are NOT processes - they ARE CPUs. The scheduler assigns
//! processes to CPUs, not the other way around.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::process::Pid;
use crate::Spinlock;
use crate::MAX_HARTS;

// ═══════════════════════════════════════════════════════════════════════════════
// CPU STATE
// ═══════════════════════════════════════════════════════════════════════════════

/// CPU operational state
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum CpuState {
    /// CPU is offline/not available
    Offline = 0,
    /// CPU is online and available for scheduling
    Online = 1,
    /// CPU is idle (no process assigned)
    Idle = 2,
    /// CPU is running a process
    Running = 3,
    /// CPU is halted (shutdown in progress)
    Halted = 4,
}

impl CpuState {
    pub fn from_u8(val: u8) -> Self {
        match val {
            0 => CpuState::Offline,
            1 => CpuState::Online,
            2 => CpuState::Idle,
            3 => CpuState::Running,
            _ => CpuState::Halted,
        }
    }

    /// Whether this CPU is available for scheduling
    pub fn is_available(&self) -> bool {
        matches!(self, CpuState::Online | CpuState::Idle)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// CPU STRUCTURE
// ═══════════════════════════════════════════════════════════════════════════════

/// CPU descriptor - represents a single CPU core (hart)
///
/// This is cache-line aligned to prevent false sharing when multiple
/// CPUs update their own descriptors concurrently.
#[repr(align(64))]
pub struct Cpu {
    /// CPU ID (hart ID in RISC-V)
    pub id: usize,

    /// Current CPU state
    state: AtomicUsize,

    /// PID of process currently running on this CPU (0 = none)
    pub current_process: AtomicU32,

    /// Whether this CPU is the bootstrap processor (BSP)
    pub is_bsp: bool,

    // ─── Statistics ─────────────────────────────────────────────────────────

    /// Total time spent running processes (ms)
    pub busy_time_ms: AtomicU64,

    /// Total time spent idle (ms)
    pub idle_time_ms: AtomicU64,

    /// Number of context switches on this CPU
    pub context_switches: AtomicU64,

    /// Number of interrupts handled on this CPU
    pub interrupts: AtomicU64,

    /// Timestamp of when this CPU went idle (for idle time tracking)
    idle_start: AtomicU64,

    /// Whether CPU is in interrupt handler
    in_interrupt: AtomicBool,
}

impl Cpu {
    /// Create a new CPU descriptor
    pub const fn new(id: usize) -> Self {
        Self {
            id,
            state: AtomicUsize::new(CpuState::Offline as usize),
            current_process: AtomicU32::new(0),
            is_bsp: false,
            busy_time_ms: AtomicU64::new(0),
            idle_time_ms: AtomicU64::new(0),
            context_switches: AtomicU64::new(0),
            interrupts: AtomicU64::new(0),
            idle_start: AtomicU64::new(0),
            in_interrupt: AtomicBool::new(false),
        }
    }

    /// Create the bootstrap processor
    pub const fn new_bsp() -> Self {
        let mut cpu = Self::new(0);
        cpu.is_bsp = true;
        cpu
    }

    // ─── State Management ───────────────────────────────────────────────────

    /// Get current CPU state
    #[inline]
    pub fn state(&self) -> CpuState {
        CpuState::from_u8(self.state.load(Ordering::Acquire) as u8)
    }

    /// Set CPU state
    #[inline]
    fn set_state(&self, state: CpuState) {
        self.state.store(state as usize, Ordering::Release);
    }

    /// Bring CPU online
    pub fn online(&self) {
        self.set_state(CpuState::Online);
    }

    /// Mark CPU as offline
    pub fn offline(&self) {
        self.set_state(CpuState::Offline);
    }

    /// Check if CPU is online
    pub fn is_online(&self) -> bool {
        !matches!(self.state(), CpuState::Offline | CpuState::Halted)
    }

    // ─── Process Assignment ─────────────────────────────────────────────────

    /// Assign a process to run on this CPU
    pub fn assign_process(&self, pid: Pid, current_time: u64) {
        // End idle period if we were idle
        let idle_start = self.idle_start.swap(0, Ordering::Relaxed);
        if idle_start > 0 {
            let idle_duration = current_time.saturating_sub(idle_start);
            self.idle_time_ms.fetch_add(idle_duration, Ordering::Relaxed);
        }

        self.current_process.store(pid, Ordering::Release);
        self.set_state(CpuState::Running);
        self.context_switches.fetch_add(1, Ordering::Relaxed);
    }

    /// Clear current process (CPU becomes idle)
    pub fn clear_process(&self, current_time: u64, busy_duration: u64) {
        self.current_process.store(0, Ordering::Release);
        self.set_state(CpuState::Idle);
        self.idle_start.store(current_time, Ordering::Relaxed);
        self.busy_time_ms.fetch_add(busy_duration, Ordering::Relaxed);
    }

    /// Get currently running process PID (0 = none)
    pub fn running_process(&self) -> Option<Pid> {
        let pid = self.current_process.load(Ordering::Acquire);
        if pid == 0 {
            None
        } else {
            Some(pid)
        }
    }

    /// Check if CPU is idle
    pub fn is_idle(&self) -> bool {
        self.current_process.load(Ordering::Relaxed) == 0
    }

    // ─── Interrupt Handling ─────────────────────────────────────────────────

    /// Enter interrupt handler
    pub fn enter_interrupt(&self) {
        self.in_interrupt.store(true, Ordering::Release);
        self.interrupts.fetch_add(1, Ordering::Relaxed);
    }

    /// Exit interrupt handler
    pub fn exit_interrupt(&self) {
        self.in_interrupt.store(false, Ordering::Release);
    }

    /// Check if CPU is in interrupt handler
    pub fn is_in_interrupt(&self) -> bool {
        self.in_interrupt.load(Ordering::Acquire)
    }

    // ─── Statistics ─────────────────────────────────────────────────────────

    /// Get CPU utilization as percentage (0-100)
    pub fn utilization(&self) -> u8 {
        let busy = self.busy_time_ms.load(Ordering::Relaxed);
        let idle = self.idle_time_ms.load(Ordering::Relaxed);
        let total = busy + idle;
        if total == 0 {
            0
        } else {
            ((busy * 100) / total).min(100) as u8
        }
    }

    /// Get total context switches
    pub fn context_switch_count(&self) -> u64 {
        self.context_switches.load(Ordering::Relaxed)
    }

    /// Get total interrupts handled
    pub fn interrupt_count(&self) -> u64 {
        self.interrupts.load(Ordering::Relaxed)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// CPU TABLE
// ═══════════════════════════════════════════════════════════════════════════════

/// Creates a const-initialized array of CPUs
const fn create_cpu_array() -> [Cpu; MAX_HARTS] {
    let mut arr = [const { Cpu::new(0) }; MAX_HARTS];
    let mut i = 0;
    while i < MAX_HARTS {
        arr[i] = Cpu::new(i);
        i += 1;
    }
    // Mark CPU 0 as BSP
    arr[0].is_bsp = true;
    arr
}

/// Global CPU table
pub struct CpuTable {
    /// All CPUs (indexed by CPU ID)
    cpus: [Cpu; MAX_HARTS],

    /// Number of CPUs currently online
    pub online_count: AtomicUsize,

    /// CPU ID of the current hart (per-hart value accessed via mhartid)
    /// This is set during initialization
    current_cpu_id: Spinlock<Option<fn() -> usize>>,
}

impl CpuTable {
    /// Create a new CPU table
    pub const fn new() -> Self {
        Self {
            cpus: create_cpu_array(),
            online_count: AtomicUsize::new(0),
            current_cpu_id: Spinlock::new(None),
        }
    }

    /// Set the function to get current CPU ID
    pub fn set_cpu_id_fn(&self, f: fn() -> usize) {
        *self.current_cpu_id.lock() = Some(f);
    }

    /// Get current CPU ID
    pub fn current_id(&self) -> usize {
        self.current_cpu_id
            .lock()
            .map(|f| f())
            .unwrap_or(0)
    }

    /// Get current CPU
    pub fn current(&self) -> &Cpu {
        &self.cpus[self.current_id()]
    }

    /// Get CPU by ID
    pub fn get(&self, id: usize) -> Option<&Cpu> {
        if id < MAX_HARTS {
            Some(&self.cpus[id])
        } else {
            None
        }
    }

    /// Bring a CPU online
    pub fn bring_online(&self, id: usize) -> bool {
        if id >= MAX_HARTS {
            return false;
        }
        self.cpus[id].online();
        self.online_count.fetch_add(1, Ordering::AcqRel);
        true
    }

    /// Take a CPU offline
    pub fn take_offline(&self, id: usize) -> bool {
        if id >= MAX_HARTS {
            return false;
        }
        self.cpus[id].offline();
        self.online_count.fetch_sub(1, Ordering::AcqRel);
        true
    }

    /// Get number of online CPUs
    pub fn num_online(&self) -> usize {
        self.online_count.load(Ordering::Acquire)
    }

    /// Get all online CPU IDs
    pub fn online_cpus(&self) -> Vec<usize> {
        (0..MAX_HARTS)
            .filter(|&id| self.cpus[id].is_online())
            .collect()
    }

    /// Get all idle CPU IDs
    pub fn idle_cpus(&self) -> Vec<usize> {
        (0..MAX_HARTS)
            .filter(|&id| self.cpus[id].is_online() && self.cpus[id].is_idle())
            .collect()
    }

    /// Find idle CPU for scheduling (prefers non-BSP for work distribution)
    pub fn find_idle_cpu(&self) -> Option<usize> {
        // First try to find an idle non-BSP CPU
        for id in 1..MAX_HARTS {
            if self.cpus[id].is_online() && self.cpus[id].is_idle() {
                return Some(id);
            }
        }
        // Fall back to BSP if it's idle
        if self.cpus[0].is_online() && self.cpus[0].is_idle() {
            return Some(0);
        }
        None
    }

    /// Find least loaded CPU (lowest context switches, prefers idle)
    pub fn find_least_loaded(&self) -> usize {
        let num_online = self.online_count.load(Ordering::Relaxed);
        if num_online <= 1 {
            return 0;
        }

        let mut best_id = 0;
        let mut best_score = u64::MAX;

        for id in 0..MAX_HARTS {
            let cpu = &self.cpus[id];
            if !cpu.is_online() {
                continue;
            }

            // Score: lower is better
            // Idle CPUs get score 0, otherwise use utilization
            let score = if cpu.is_idle() {
                // Prefer non-BSP idle CPUs
                if id == 0 { 1 } else { 0 }
            } else {
                cpu.utilization() as u64 + 100 // Running CPUs are less preferred
            };

            if score < best_score {
                best_score = score;
                best_id = id;
            }
        }

        best_id
    }
}

/// Global CPU table
pub static CPU_TABLE: CpuTable = CpuTable::new();

// ═══════════════════════════════════════════════════════════════════════════════
// CPU INFORMATION
// ═══════════════════════════════════════════════════════════════════════════════

/// Snapshot of CPU information for reporting
#[derive(Clone)]
pub struct CpuInfo {
    pub id: usize,
    pub state: CpuState,
    pub is_bsp: bool,
    pub running_process: Option<Pid>,
    pub utilization: u8,
    pub context_switches: u64,
    pub interrupts: u64,
}

impl Cpu {
    /// Get CPU info snapshot
    pub fn info(&self) -> CpuInfo {
        CpuInfo {
            id: self.id,
            state: self.state(),
            is_bsp: self.is_bsp,
            running_process: self.running_process(),
            utilization: self.utilization(),
            context_switches: self.context_switch_count(),
            interrupts: self.interrupt_count(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// INITIALIZATION
// ═══════════════════════════════════════════════════════════════════════════════

/// Initialize the CPU subsystem
pub fn init(get_hart_id: fn() -> usize, num_cpus: usize) {
    CPU_TABLE.set_cpu_id_fn(get_hart_id);
    
    // Bring CPUs online
    for id in 0..num_cpus.min(MAX_HARTS) {
        CPU_TABLE.bring_online(id);
    }

    crate::klog::klog_info(
        "cpu",
        &alloc::format!("{} CPUs online", CPU_TABLE.num_online()),
    );
}
