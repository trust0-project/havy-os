//! Init system - PID 1 process
//!
//! The init process is responsible for:
//! - Spawning system services (daemons)
//! - Running startup scripts from /etc/init.d/
//! - Reaping zombie processes
//! - System shutdown coordination
//!
//! Similar to Linux's init/systemd but much simpler.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::klog::{klog_debug, klog_error, klog_info};
use crate::Spinlock;

// Process management
use crate::process::{Priority, ProcessEntry};
use crate::sched::SCHEDULER as PROC_SCHEDULER;

/// Init system state
static INIT_STATE: Spinlock<InitState> = Spinlock::new(InitState::new());

/// Whether init has completed startup (public for secondary hart sync)
pub static INIT_COMPLETE: AtomicBool = AtomicBool::new(false);

/// Number of services started
static SERVICES_STARTED: AtomicUsize = AtomicUsize::new(0);

/// Service status
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatus {
    Stopped,
    Running,
    Failed,
}

impl ServiceStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ServiceStatus::Stopped => "stopped",
            ServiceStatus::Running => "running",
            ServiceStatus::Failed => "failed",
        }
    }
}

/// Service definition - describes a service that can be started/stopped
#[derive(Clone)]
pub struct ServiceDef {
    pub name: String,
    pub description: String,
    pub entry: ProcessEntry,
    pub priority: Priority,
    pub preferred_hart: Option<usize>,
}

/// Service runtime info
#[derive(Clone)]
pub struct ServiceInfo {
    pub name: String,
    pub pid: u32,
    pub status: ServiceStatus,
    pub started_at: u64,
    pub hart: Option<usize>,
}

/// Init state
struct InitState {
    /// Registered service definitions
    service_defs: Vec<ServiceDef>,
    /// Running services
    services: Vec<ServiceInfo>,
}

impl InitState {
    const fn new() -> Self {
        Self {
            service_defs: Vec::new(),
            services: Vec::new(),
        }
    }
}

// ===============================================================================
// INIT PROCESS
// ===============================================================================

/// Init process entry point - PID 1
///
/// This runs on the primary hart and is responsible for bringing up the system.
pub fn init_main() {
    // Use raw UART output to avoid any locking (heap allocator, klog buffer)
    // Secondary harts are spinning but may affect shared resources
    crate::uart::write_line("[init] Starting init system (PID 1)");

    // Phase 1: Create required directories
    crate::uart::write_line("[init] Phase 1: Creating system directories");
    ensure_directories();

    // Phase 2: Start system services
    crate::uart::write_line("[init] Phase 2: Starting system services");
    start_system_services();

    // Phase 3: Run init scripts
    crate::uart::write_line("[init] Phase 3: Running init scripts");
    run_init_scripts();

    // NOTE: We do NOT set INIT_COMPLETE here anymore!
    // The shell is spawned in main() after init_main() returns,
    // and INIT_COMPLETE is set only after shell is spawned.
    // This prevents secondary harts from racing with shell initialization.
    
    crate::uart::write_line("[init] Init phase complete, waiting for shell spawn...");
    
    // Write initial boot message to kernel.log (now safe to use allocator)
    write_boot_log();
}

/// Ensure required system directories exist
fn ensure_directories() {
    let dirs = ["/var", "/var/log", "/var/run", "/etc", "/tmp"];

    for dir in &dirs {
        // For our simple FS, we just ensure we can write a marker file
        // A real FS would have proper directory support
        // Directory ensured: dir (no-op in our simple FS)
        let _ = dir;
    }
}

/// Get the least loaded secondary hart for scheduling a new service
/// Returns a secondary hart if available, falls back to hart 0 only if single-hart mode.
/// NOTE: Hart 0 (BSP) runs the shell loop and doesn't pick processes from the scheduler,
/// so we should prefer secondary harts for spawning daemon processes.
fn get_least_loaded_hart() -> usize {
    let num_harts = crate::HARTS_ONLINE.load(Ordering::Relaxed);
    if num_harts <= 1 {
        // Single hart mode - everything runs on hart 0
        return 0;
    }
    
    // Find least loaded secondary hart (avoiding BSP/hart 0)
    let mut best_hart = 1; // Start with first secondary hart
    let mut min_load = usize::MAX;
    
    for hart_id in 1..num_harts {
        if let Some(cpu) = crate::cpu::CPU_TABLE.get(hart_id) {
            if !cpu.is_online() {
                continue;
            }
            // Check queue length from scheduler
            let queue_len = PROC_SCHEDULER.queue_length(hart_id);
            if queue_len < min_load {
                min_load = queue_len;
                best_hart = hart_id;
            }
        }
    }
    
    best_hart
}

/// Start core system services
fn start_system_services() {
    let num_harts = crate::HARTS_ONLINE.load(Ordering::Relaxed);
    let num_secondary_harts = if num_harts > 1 { num_harts - 1 } else { 0 };
    
    // Use raw UART to avoid heap allocation during init
    crate::uart::write_str("[init] Harts available: ");
    crate::uart::write_u64(num_harts as u64);
    crate::uart::write_str(" (");
    crate::uart::write_u64(num_secondary_harts as u64);
    crate::uart::write_line(" secondary)");

    // Initialize WASM service for multi-hart execution
    if num_harts > 1 {
        crate::wasm_service::init(num_harts);
        crate::uart::write_line("[init] WASM service initialized");
    }

    // Register service definitions (for service list/status display)
    register_service_def(
        "klogd",
        "Kernel logger daemon - logs system memory stats",
        klogd_service,
        Priority::Normal,
        None,
    );
    
    register_service_def(
        "sysmond",
        "System monitor daemon - monitors system health",
        sysmond_service,
        Priority::Normal,
        None,
    );
    
    // ─── SPAWN DAEMONS ─────────────────────────────────────────────────────────────
    // 
    // With cooperative time-slicing, multiple daemons can run on the same hart.
    // Each daemon does one tick of work and returns, allowing the scheduler to
    // run the next process. This works even with just 1 secondary hart.
    //
    // - 0 secondary harts (single hart): Use shell-loop cooperative ticks on hart 0
    // - 1+ secondary harts:              Spawn as processes, they'll time-slice
    
    if num_secondary_harts >= 1 {
        // ─── PROCESS MODE (TIME-SLICING) ──────────────────────────────────────────
        // Daemons run as processes on secondary harts, time-slicing cooperatively
        // Note: Daemons use try_lock to avoid blocking shell commands
        
        let klogd_hart = get_least_loaded_hart();
        let klogd_pid = PROC_SCHEDULER.spawn_daemon_on_cpu("klogd", klogd_service, Priority::Low, Some(klogd_hart));
        register_service("klogd", klogd_pid, Some(klogd_hart));
        crate::uart::write_str("[init] klogd spawned (PID ");
        crate::uart::write_u64(klogd_pid as u64);
        crate::uart::write_str(") on CPU ");
        crate::uart::write_u64(klogd_hart as u64);
        crate::uart::write_line("");
        
        // Sysmond goes to the same or different hart based on load
        let sysmond_hart = get_least_loaded_hart();
        let sysmond_pid = PROC_SCHEDULER.spawn_daemon_on_cpu("sysmond", sysmond_service, Priority::Low, Some(sysmond_hart));
        register_service("sysmond", sysmond_pid, Some(sysmond_hart));
        crate::uart::write_str("[init] sysmond spawned (PID ");
        crate::uart::write_u64(sysmond_pid as u64);
        crate::uart::write_str(") on CPU ");
        crate::uart::write_u64(sysmond_hart as u64);
        crate::uart::write_line("");
        
    } else {
        // ─── SHELL-LOOP COOPERATIVE MODE ──────────────────────────────────────────
        // Single-hart mode: services are ticked by the shell loop on hart 0
        
        let klogd_pid = crate::process::allocate_pid();
        let sysmond_pid = crate::process::allocate_pid();
        
        register_service("klogd", klogd_pid, Some(0));
        register_service("sysmond", sysmond_pid, Some(0));
        
        crate::uart::write_str("[init] klogd (PID ");
        crate::uart::write_u64(klogd_pid as u64);
        crate::uart::write_line(") cooperative on CPU 0");
        crate::uart::write_str("[init] sysmond (PID ");
        crate::uart::write_u64(sysmond_pid as u64);
        crate::uart::write_line(") cooperative on CPU 0");
    }
}

// ===============================================================================
// PUBLIC SERVICE CONTROL API
// ===============================================================================

/// Start a service by name
/// Returns Ok(()) on success, Err(message) on failure
pub fn start_service(name: &str) -> Result<(), &'static str> {
    let state = INIT_STATE.lock();

    // Check if already running
    if let Some(svc) = state.services.iter().find(|s| s.name == name) {
        if svc.status == ServiceStatus::Running {
            return Err("Service is already running");
        }
    }

    // Find service definition
    let def = state
        .service_defs
        .iter()
        .find(|d| d.name == name)
        .ok_or("Service not found")?;

    let entry = def.entry;
    let priority = def.priority;
    let preferred_hart = def.preferred_hart;
    let name_owned = def.name.clone();

    drop(state); // Release lock before spawning

    // Determine target CPU - use preferred or find least loaded
    let target_cpu = preferred_hart.unwrap_or_else(get_least_loaded_hart);
    
    // Spawn using process scheduler
    let pid = PROC_SCHEDULER.spawn_on_cpu(
        &name_owned,
        entry,
        priority,
        Some(target_cpu),
    );
    register_service(&name_owned, pid, Some(target_cpu));

    // Wake the target hart
    if target_cpu != 0 {
        crate::send_ipi(target_cpu);
    }
    
    klog_info("init", &format!("Started {} (PID {}) on CPU {}", name_owned, pid, target_cpu));

    Ok(())
}

/// Stop a service by name
/// Returns Ok(()) on success, Err(message) on failure
pub fn stop_service(name: &str) -> Result<(), &'static str> {
    let state = INIT_STATE.lock();

    // Find the running service
    let svc = state
        .services
        .iter()
        .find(|s| s.name == name)
        .ok_or("Service not found")?;

    if svc.status != ServiceStatus::Running {
        return Err("Service is not running");
    }

    let pid = svc.pid;
    drop(state); // Release lock before killing

    // Kill the process
    if pid > 0 {
        crate::sched::kill(pid);
    }

    // Mark as stopped
    mark_service_stopped(name);

    Ok(())
}

/// Stop a service by PID (used by kill syscall)
/// Returns true if a service with that PID was found and stopped
pub fn stop_service_by_pid(pid: u32) -> bool {
    let mut state = INIT_STATE.lock();
    
    // Find the running service with this PID
    if let Some(svc) = state.services.iter_mut().find(|s| s.pid == pid && s.status == ServiceStatus::Running) {
        let name = svc.name.clone();
        svc.status = ServiceStatus::Stopped;
        svc.pid = 0;
        svc.hart = None;
        
        klog_info("init", &format!("Stopped service '{}' (PID {})", name, pid));
        return true;
    }
    
    false
}

/// Restart a service by name
/// Returns Ok(()) on success, Err(message) on failure
pub fn restart_service(name: &str) -> Result<(), &'static str> {
    // Stop if running (ignore error if not running)
    let _ = stop_service(name);

    // Small delay to let things settle
    for _ in 0..10000 {
        core::hint::spin_loop();
    }

    // Start the service
    start_service(name)
}

/// Get status of a service
pub fn service_status(name: &str) -> Option<ServiceStatus> {
    let state = INIT_STATE.lock();
    state
        .services
        .iter()
        .find(|s| s.name == name)
        .map(|s| s.status)
}

/// Get detailed info about a service
pub fn get_service_info(name: &str) -> Option<ServiceInfo> {
    let state = INIT_STATE.lock();
    state.services.iter().find(|s| s.name == name).cloned()
}

/// List all registered services (definitions)
pub fn list_service_defs() -> Vec<(String, String)> {
    let state = INIT_STATE.lock();
    state
        .service_defs
        .iter()
        .map(|d| (d.name.clone(), d.description.clone()))
        .collect()
}

/// Register a service definition (what the service is and how to start it)
pub fn register_service_def(
    name: &str,
    description: &str,
    entry: ProcessEntry,
    priority: Priority,
    preferred_hart: Option<usize>,
) {
    let mut state = INIT_STATE.lock();
    state.service_defs.push(ServiceDef {
        name: String::from(name),
        description: String::from(description),
        entry,
        priority,
        preferred_hart,
    });
}

/// Register a running service instance
pub fn register_service(name: &str, pid: u32, hart: Option<usize>) {
    let mut state = INIT_STATE.lock();

    // Update existing or add new
    if let Some(svc) = state.services.iter_mut().find(|s| s.name == name) {
        svc.pid = pid;
        svc.status = ServiceStatus::Running;
        svc.started_at = crate::get_time_ms() as u64;
        svc.hart = hart;
    } else {
        state.services.push(ServiceInfo {
            name: String::from(name),
            pid,
            status: ServiceStatus::Running,
            started_at: crate::get_time_ms() as u64,
            hart,
        });
    }
    SERVICES_STARTED.fetch_add(1, Ordering::Relaxed);
}

/// Mark a service as stopped
fn mark_service_stopped(name: &str) {
    let mut state = INIT_STATE.lock();
    if let Some(svc) = state.services.iter_mut().find(|s| s.name == name) {
        svc.status = ServiceStatus::Stopped;
        svc.pid = 0;
        svc.hart = None;
    }
}

/// Run init scripts from /etc/init.d/
/// Note: Init scripts must be WASM binaries
fn run_init_scripts() {
    let mut fs_guard = crate::FS_STATE.write();
    let mut blk_guard = crate::BLK_DEV.write();

    if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
        // Look for init scripts
        let files = fs.list_dir(dev, "/");
        for file in files {
            if file.name.starts_with("/etc/init.d/") {
                let script_name = &file.name[12..]; // Strip "/etc/init.d/"
                
                // Read the script content
                if let Some(content) = fs.read_file(dev, &file.name) {
                    // Check if it's a WASM binary
                    if content.len() >= 4 
                        && content[0] == 0x00 
                        && content[1] == 0x61 
                        && content[2] == 0x73 
                        && content[3] == 0x6D 
                    {
                        klog_info("init", &format!("Running init script: {}", script_name));
                        drop(blk_guard);
                        drop(fs_guard);
                        
                        // Execute WASM binary
                        if let Err(e) = crate::wasm::execute(&content, &[]) {
                            klog_error("init", &format!("Init script error: {}", e));
                        }
                        return; // Re-acquire locks would be complex, just return
                    } else {
                        // Not a WASM binary, skip (legacy text scripts)
                        klog_debug("init", &format!("Skipping non-WASM init script: {}", script_name));
                    }
                }
            }
        }
    }
}

/// Write boot information to kernel.log
fn write_boot_log() {
    let timestamp = crate::get_time_ms();
    let num_harts = crate::HARTS_ONLINE.load(Ordering::Relaxed);
    let services = SERVICES_STARTED.load(Ordering::Relaxed);

    let boot_msg = format!(
        "=== BAVY OS Boot Log ===\n\
         Boot time: {}ms\n\
         Harts online: {}\n\
         Services started: {}\n\
         ========================\n",
        timestamp, num_harts, services
    );

    // Write to kernel.log
    let mut fs_guard = crate::FS_STATE.write();
    let mut blk_guard = crate::BLK_DEV.write();

    if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
        if let Err(e) = fs.write_file(dev, "/var/log/kernel.log", boot_msg.as_bytes()) {
            klog_error("init", &format!("Failed to write boot log: {}", e));
        } else {
            // Sync to ensure data is written to disk
            let _ = fs.sync(dev);
        }
    }
}

// ===============================================================================
// SYSTEM SERVICES (long-running daemons on secondary harts)
// ===============================================================================

/// Spin-delay for approximately the given milliseconds
/// Uses busy-waiting since secondary harts don't have timer interrupts
#[inline(never)]
fn spin_delay_ms(ms: u64) {
    let start = crate::get_time_ms() as u64;
    let target = start + ms;
    while (crate::get_time_ms() as u64) < target {
        // Yield CPU hints to save power
        for _ in 0..100 {
            core::hint::spin_loop();
        }
    }
}

// ===============================================================================
// LOG BUFFER SYSTEM
// Daemons write to an in-memory buffer, hart 0 flushes to disk
// This avoids VirtIO contention between harts
// ===============================================================================

/// Maximum log entries to buffer before forcing a flush
const LOG_BUFFER_SIZE: usize = 32;
/// Maximum length of each log line
const LOG_LINE_MAX: usize = 128;

/// A single log entry
struct LogEntry {
    data: [u8; LOG_LINE_MAX],
    len: usize,
    target: LogTarget,
}

/// Which log file to write to
#[derive(Clone, Copy, PartialEq)]
enum LogTarget {
    Kernel,   // /var/log/kernel.log
    Sysmond,  // /var/log/sysmond.log
}

/// Log buffer state
struct LogBuffer {
    entries: [Option<LogEntry>; LOG_BUFFER_SIZE],
    count: usize,
    last_flush_ms: i64,
}

impl LogBuffer {
    const fn new() -> Self {
        const NONE: Option<LogEntry> = None;
        Self {
            entries: [NONE; LOG_BUFFER_SIZE],
            count: 0,
            last_flush_ms: 0,
        }
    }
    
    /// Add a log entry to the buffer
    fn push(&mut self, line: &str, target: LogTarget) {
        if self.count >= LOG_BUFFER_SIZE {
            // Buffer full, drop oldest entry (simple ring behavior)
            for i in 1..LOG_BUFFER_SIZE {
                self.entries[i - 1] = self.entries[i].take();
            }
            self.count = LOG_BUFFER_SIZE - 1;
        }
        
        let mut entry = LogEntry {
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
    fn drain(&mut self) -> Vec<(String, LogTarget)> {
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
}

/// Global log buffer protected by spinlock
static LOG_BUFFER: crate::Spinlock<LogBuffer> = crate::Spinlock::new(LogBuffer::new());

/// Queue a log entry (safe to call from any hart)
fn queue_log(line: &str, target: LogTarget) {
    let mut buffer = LOG_BUFFER.lock();
    buffer.push(line, target);
}

/// Flush pending log entries to disk (ONLY call from hart 0!)
/// Returns the number of entries flushed
pub fn flush_log_buffer() -> usize {
    // Only hart 0 should call this
    let hart_id = crate::get_hart_id();
    if hart_id != 0 {
        return 0;
    }
    
    // Get entries from buffer
    let entries = {
        let mut buffer = LOG_BUFFER.lock();
        
        // Only flush every 5 seconds or if buffer is getting full
        let now = crate::get_time_ms();
        if buffer.count < LOG_BUFFER_SIZE / 2 && now - buffer.last_flush_ms < 5000 {
            return 0;
        }
        buffer.last_flush_ms = now;
        buffer.drain()
    };
    
    if entries.is_empty() {
        return 0;
    }
    
    let count = entries.len();
    
    // Now we can safely access the filesystem (we're on hart 0)
    let mut fs_guard = crate::FS_STATE.write();
    let mut blk_guard = crate::BLK_DEV.write();
    
    if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
        // Group entries by target
        let mut kernel_lines = Vec::new();
        let mut sysmond_lines = Vec::new();
        
        for (line, target) in entries {
            match target {
                LogTarget::Kernel => kernel_lines.push(line),
                LogTarget::Sysmond => sysmond_lines.push(line),
            }
        }
        
        // Write kernel log entries
        if !kernel_lines.is_empty() {
            let existing = fs
                .read_file(dev, "/var/log/kernel.log")
                .map(|v| String::from_utf8_lossy(&v).into_owned())
                .unwrap_or_default();
            
            let trimmed = if existing.len() > 16384 {
                String::from(&existing[existing.len() - 16384..])
            } else {
                existing
            };
            
            let mut new_content = trimmed;
            for line in kernel_lines {
                new_content.push_str(&line);
                new_content.push('\n');
            }
            
            let _ = fs.write_file(dev, "/var/log/kernel.log", new_content.as_bytes());
        }
        
        // Write sysmond log entries
        if !sysmond_lines.is_empty() {
            let existing = fs
                .read_file(dev, "/var/log/sysmond.log")
                .map(|v| String::from_utf8_lossy(&v).into_owned())
                .unwrap_or_default();
            
            let trimmed = if existing.len() > 8192 {
                String::from(&existing[existing.len() - 8192..])
            } else {
                existing
            };
            
            let mut new_content = trimmed;
            for line in sysmond_lines {
                new_content.push_str(&line);
                new_content.push('\n');
            }
            
            let _ = fs.write_file(dev, "/var/log/sysmond.log", new_content.as_bytes());
        }
        
        // Sync once at the end
        let _ = fs.sync(dev);
    }
    
    count
}

/// Append a line to the kernel log (queued for hart 0 to flush)
/// Safe to call from any hart
fn append_to_log(line: &str) -> bool {
    queue_log(line, LogTarget::Kernel);
    true
}

/// Append a line to the sysmond log (queued for hart 0 to flush)
/// Safe to call from any hart
fn append_to_sysmond_log(line: &str) -> bool {
    queue_log(line, LogTarget::Sysmond);
    true
}

// ===============================================================================
// COOPERATIVE DAEMON TICKS
// These functions do one unit of work and return immediately.
// Called from the shell loop on hart 0.
// ===============================================================================

use core::sync::atomic::AtomicI64;

/// State for klogd daemon
static KLOGD_LAST_RUN: AtomicI64 = AtomicI64::new(0);
static KLOGD_TICK: AtomicUsize = AtomicUsize::new(0);
static KLOGD_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// State for sysmond daemon  
static SYSMOND_LAST_RUN: AtomicI64 = AtomicI64::new(0);
static SYSMOND_TICK: AtomicUsize = AtomicUsize::new(0);
static SYSMOND_INITIALIZED: AtomicBool = AtomicBool::new(false);

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
        // Not time yet - yield briefly and return
        spin_delay_ms(10);
        return;
    }
    
    // Time to potentially do work
    klogd_tick();
    spin_delay_ms(10);
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
        // Not time yet - yield briefly and return
        spin_delay_ms(10);
        return;
    }
    
    // Time to potentially do work
    sysmond_tick();
    spin_delay_ms(10);
}

/// WASM worker service entry point
/// This daemon runs on secondary harts and executes WASM jobs via IPC
pub fn wasm_worker_service() {
    // This enters an infinite loop processing WASM jobs
    crate::wasm_service::worker_entry();
}

// ===============================================================================
// UTILITY FUNCTIONS
// ===============================================================================

/// Check if init has completed
pub fn is_init_complete() -> bool {
    INIT_COMPLETE.load(Ordering::Acquire)
}

/// Get list of all services (running and stopped)
pub fn list_services() -> Vec<ServiceInfo> {
    let state = INIT_STATE.lock();

    // Return all services, adding stopped ones from definitions
    let mut result = state.services.clone();

    // Add any defined services that aren't in the running list
    for def in &state.service_defs {
        if !result.iter().any(|s| s.name == def.name) {
            result.push(ServiceInfo {
                name: def.name.clone(),
                pid: 0,
                status: ServiceStatus::Stopped,
                started_at: 0,
                hart: None,
            });
        }
    }

    result
}

/// Get number of services started
pub fn service_count() -> usize {
    SERVICES_STARTED.load(Ordering::Relaxed)
}
