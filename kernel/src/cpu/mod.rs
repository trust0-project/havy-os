use core::{arch::asm, cell::UnsafeCell, sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering}};

use alloc::vec::Vec;

use crate::{ Spinlock, boot::BOOT_READY, clint::get_time_ms, constants::{CLINT_MSIP_BASE, SCHED_DIAG_CAN_SCHEDULE, SCHED_DIAG_HART_ID, SCHED_DIAG_PICK_COUNT, SCHED_DIAG_PICK_RESULT, SCHED_DIAG_PROCESS_NAME, SCHED_DIAG_PROCESS_PID, SCHED_DIAG_REQUEUE_OK}, cpu::{self, process::{Context, Pid}}, fence_acquire, fence_memory, init, sbi, services::{gpuid, httpd, klogd::{self, klog_info}, netd, shelld, sysmond, tcpd}, trap, utils::update_sysinfo};
use crate::dtb::DTB_ADDR;

pub mod sched;
pub mod process;
pub mod ipc;
pub mod io_router;
pub mod fs_proxy;
pub mod display_proxy;
pub mod net_proxy;
pub mod audio_proxy;

pub(crate) const MAX_HARTS: usize = 128;
pub(crate) static HARTS_ONLINE: AtomicUsize = AtomicUsize::new(0);
pub(crate) const CLINT_HART_COUNT: usize = 0x0200_0F00;

/// Tracks which harts are actively running hart_loop() and ready for scheduling.
/// This is set AFTER a hart enters hart_loop(), not just when it goes online.
/// Used by the scheduler to avoid sending work to harts still in boot spin-loops.
pub(crate) static HART_READY: [AtomicBool; MAX_HARTS] = {
    const INIT: AtomicBool = AtomicBool::new(false);
    [INIT; MAX_HARTS]
};

/// Check if a hart is ready for scheduling (actively running hart_loop).
/// 
/// This is different from "online" - a hart can be online but still waiting
/// for INIT_COMPLETE before entering the scheduling loop.
#[inline]
pub fn is_hart_ready(hart_id: usize) -> bool {
    hart_id < MAX_HARTS && HART_READY[hart_id].load(Ordering::Acquire)
}

/// Read the hart count from the CLINT register (set by emulator)
pub(crate) fn get_expected_harts() -> usize {
    let count = unsafe { core::ptr::read_volatile(CLINT_HART_COUNT as *const u32) } as usize;
    // Clamp to valid range [1, MAX_HARTS]
    if count == 0 {
        1
    } else {
        count.min(MAX_HARTS)
    }
}

/// Sleep for approximately the given milliseconds using WFI.
/// Uses WFI instruction to actually sleep the hart, saving CPU cycles.
/// Timer interrupts wake the hart every ~10ms ensuring proper delay.
#[inline(never)]
fn wfi_delay_ms(ms: u64) {
    let start = crate::get_time_ms() as u64;
    let target = start + ms;
    while (crate::get_time_ms() as u64) < target {
        // Sleep until timer interrupt wakes us
        unsafe {
            core::arch::asm!("wfi", options(nomem, nostack));
        }
    }
}


/// Alias for compatibility - redirects to wfi_delay_ms
#[inline(always)]
pub(crate) fn spin_delay_ms(ms: u64) {
    wfi_delay_ms(ms);
}


/// Universal hart idle loop - Round-Robin Scheduler
///
/// This loop continuously runs all processes assigned to this hart in round-robin fashion.
/// Each process's entry function is called repeatedly (it should do one "tick" of work).
/// This is cooperative multitasking - processes must return promptly to allow others to run.
///
/// All harts (including hart 0) use this same loop after initialization is complete.
/// This ensures all harts are treated equally for process scheduling.
///
/// For daemons with infinite loops, they should:
/// 1. Do one iteration of their work
/// 2. Return (yielding to the scheduler)
/// The scheduler will call them again on the next round.
pub(crate) fn hart_loop(hart_id: usize) -> ! {
    // Mark this hart as ready for scheduling BEFORE entering the loop
    // This signals to the scheduler that we're actively picking up work
    if hart_id < MAX_HARTS {
        HART_READY[hart_id].store(true, Ordering::Release);
    }
    
    loop {
        let mut did_work = false;
        
        // Run scheduler round-robin: pick a process, run one tick, requeue, repeat
        // All harts participate in scheduling once the scheduler is active
        let can_schedule = sched::SCHEDULER.is_active();
            
        if can_schedule {
            if let Some(process) = sched::SCHEDULER.pick_next(hart_id) {
                did_work = true;
                
                // Mark CPU as running this process
                if let Some(cpu) = CPU_TABLE.get(hart_id) {
                    cpu.assign_process(process.pid, get_time_ms() as u64);
                }

                // Mark process as running on this CPU
                process.mark_running(hart_id);

                let start_time = get_time_ms() as u64;

                // Execute ONE TICK of the process
                // Daemons should do one iteration of work and return
                (process.entry)();

                // Process returned - update stats
                let elapsed = (get_time_ms() as u64).saturating_sub(start_time);
                process.add_cpu_time(elapsed);

                // Clear CPU's current process
                if let Some(cpu) = cpu::CPU_TABLE.get(hart_id) {
                    cpu.clear_process(get_time_ms() as u64, elapsed);
                }

                // Requeue daemon processes for the next round
                // Non-daemon processes are one-shot and exit
                if process.is_daemon() {
                    sched::requeue(process, hart_id);
                } else {
                    sched::SCHEDULER.exit(process.pid, 0);
                }
            }
        }

        // Hart 0 runs periodic tasks (log buffer flush, sysinfo update, etc.)
        if hart_id == 0 {
            klogd::flush_log_buffer();
            klogd::klogd_tick();
            sysmond::sysmond_tick();
            // Update system info MMIO device (for emulator UI)
            update_sysinfo();
            // Process I/O requests from secondary harts
            io_router::dispatch_io();
        }

        // If no work was done, sleep immediately via WFI
        // This saves host CPU cycles - the hart will wake on:
        // - Timer interrupt (every ~10ms via SBI)
        // - IPI (when new work is queued for this hart)
        // - External interrupt
        if !did_work {
            // Check for pending IPI first
            if is_my_msip_pending() {
                clear_my_msip();
            } else {
                // Sleep until interrupt - saves CPU power
                unsafe {
                    core::arch::asm!("wfi", options(nomem, nostack));
                }
            }
        }
    }
}

/// Send an Inter-Processor Interrupt to the specified hart.
///
/// This triggers a `SupervisorSoftwareInterrupt` on the target hart,
/// waking it from WFI if sleeping.
///
/// # Arguments
/// * `hart_id` - The target hart ID (0-127)
///
/// Uses SBI to send the IPI (required for S-mode operation).
#[inline]
pub fn send_ipi(hart_id: usize) {
    if hart_id >= MAX_HARTS {
        return; // Invalid hart ID, silently ignore
    }

    // Use SBI to send IPI - hart_mask has bit N set for the target hart
    sbi::send_ipi(1u64 << hart_id, 0);
}

/// Send IPI to all harts except the caller.
///
/// Useful for broadcast notifications.
#[allow(dead_code)]
pub fn send_ipi_all_others() {
    let my_hart = get_hart_id();
    let expected_harts = get_expected_harts();
    for hart in 0..expected_harts {
        if hart != my_hart {
            send_ipi(hart);
        }
    }
}

/// Clear the software interrupt for a hart.
///
/// Must be called by the target hart to acknowledge the IPI.
/// Uses SBI for S-mode operation.
#[inline]
pub fn clear_msip(_hart_id: usize) {
    sbi::clear_ipi();
}

/// Clear the software interrupt for the current hart via SBI.
#[inline]
#[allow(dead_code)]
pub fn clear_my_msip() {
    sbi::clear_ipi();
}




/// Check if software interrupt is pending for a hart.
#[inline]
#[allow(dead_code)]
pub fn is_msip_pending(hart_id: usize) -> bool {
    if hart_id >= MAX_HARTS {
        return false;
    }
    let msip_addr = CLINT_MSIP_BASE + (hart_id * 4);
    unsafe {
        let val = core::ptr::read_volatile(msip_addr as *const u32);
        val & 1 != 0
    }
}

/// Check if software interrupt is pending for current hart.
#[inline]
#[allow(dead_code)]
pub fn is_my_msip_pending() -> bool {
    is_msip_pending(get_hart_id())
}


/// Entry point for secondary harts (called after waking from WFI).
///
/// This function is called after the secondary hart has:
/// 1. Been woken by an IPI from the primary hart
/// 2. Checked that BOOT_READY is true
///
/// # Arguments
/// * `hart_id` - This hart's ID (1, 2, 3, ...)
fn secondary_hart_entry(hart_id: usize) -> ! {
    // Wait for primary boot to complete (double-check after WFI wake)
    while !BOOT_READY.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }

    // Memory fence ensures we see all init writes from primary hart
    // This is critical for RISC-V weak memory model
    fence_memory();

    // Register this hart as online (legacy counter)
    HARTS_ONLINE.fetch_add(1, Ordering::SeqCst);

    // Wait for init to complete (scheduler init + service spawning)
    // This is critical: we must not enter hart_loop until init_main() finishes
    // Otherwise we get lock contention with klog and other subsystems
    while !init::INIT_COMPLETE.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }
    
    // Acquire fence after INIT_COMPLETE check ensures we see all
    // initialization writes (heap allocator, filesystem, services)
    fence_acquire();
    
    // Mark CPU as online in the new CPU table (now that it's initialized)
    if let Some(cpu) = CPU_TABLE.get(hart_id) {
        cpu.online();
    }

    // Initialize trap handlers for this hart
    trap::init(hart_id);

    // Enter the hart loop (same loop used by all harts)
    hart_loop(hart_id);
}


/// Get the current hart ID from tp register.
///
/// The hart ID is stored in the tp (thread pointer) register during boot
/// by the _mp_hook function. This works in both M-mode and S-mode.
#[inline]
pub fn get_hart_id() -> usize {
    let id: usize;
    unsafe {
        asm!("mv {}, tp", out(reg) id, options(nomem, nostack));
    }
    id
}

/// Secondary hart parking loop.
///
/// Waits for IPI, then transfers to secondary_hart_entry.
///
/// # Safety
/// Called very early in boot, before Rust runtime is fully initialized.
#[inline(never)]
unsafe fn secondary_hart_park(hart_id: usize) -> ! {
    // Wait for IPI to wake us
    loop {
        asm!("wfi", options(nomem, nostack));

        // Check if this was our wake-up call
        let msip = is_msip_pending(hart_id);
        if msip {
            // Clear the interrupt
            clear_msip(hart_id);
            break;
        }
        // Spurious wakeup - go back to sleep
    }

    // Transfer to secondary entry point
    secondary_hart_entry(hart_id);
}




/// Multi-processing hook called by riscv-rt before main().
///
/// - Hart 0: Returns true to continue to main()
/// - Other harts: Enter parking loop, call secondary_hart_entry when woken
///
/// # Safety
/// This is called very early in boot, before Rust runtime is fully initialized.
/// Only use assembly and no allocations.
///
/// # S-mode Note
/// In S-mode (riscv-rt s-mode feature), we cannot read mhartid CSR.
/// riscv-rt passes hart ID as function parameter. We store it in tp 
/// (thread pointer) for later access via get_hart_id().
#[export_name = "_mp_hook"]
#[inline(never)]
pub unsafe extern "C" fn mp_hook(hart_id: usize, dtb_addr: usize) -> bool {
    // Capture DTB address from a1 (OpenSBI passes DTB pointer here)
    // Must be done early before a1 is clobbered by Rust code
    if hart_id == 0 && dtb_addr != 0 {
        DTB_ADDR.store(dtb_addr, Ordering::Release);
    }
    
    // Store hart_id in tp for later access via get_hart_id()
    asm!(
        "mv tp, {0}",
        in(reg) hart_id,
        options(nomem, nostack, preserves_flags)
    );
    
    if hart_id == 0 {
        // Primary hart: continue to main()
        true
    } else {
        // Secondary hart: park and wait for IPI
        secondary_hart_park(hart_id);
    }
}



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

    // ─── Context Switching ──────────────────────────────────────────────────

    /// Scheduler context - stores the register state of the scheduler loop
    /// when switching into a process. When the process yields or is preempted,
    /// we switch back to this context to resume the scheduler.
    scheduler_context: UnsafeCell<Context>,
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
            scheduler_context: UnsafeCell::new(Context::zero()),
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

    // ─── Context Switching ───────────────────────────────────────────────────

    /// Get a pointer to the scheduler context for use in context switching.
    ///
    /// # Safety
    /// The caller must ensure this is only used during context switching
    /// operations on this specific CPU. Only the hart that owns this CPU
    /// should call this method.
    #[inline]
    pub fn scheduler_context_ptr(&self) -> *mut Context {
        self.scheduler_context.get()
    }
}

// SAFETY: Cpu uses UnsafeCell for scheduler_context, but the context is only
// accessed by the hart that owns this CPU entry. The scheduler ensures that
// context switches only happen on the owning hart.
unsafe impl Sync for Cpu {}

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

    klog_info(
        "cpu",
        &alloc::format!("{} CPUs online", CPU_TABLE.num_online()),
    );
}
