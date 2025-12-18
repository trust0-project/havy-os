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
use crate::{Spinlock, cpu};

// Process management
use cpu::process::{Priority, ProcessEntry};
use crate::sched::SCHEDULER as PROC_SCHEDULER;
use crate::services::gpuid::gpuid_service;
use crate::services::klogd::{self, klog_debug, klog_error, klog_info};
use crate::services::{httpd, netd, sysmond, tcpd};

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

/// Get the least loaded secondary hart for scheduling a new service
/// Returns a secondary hart if available, falls back to hart 0 only if single-hart mode.
/// NOTE: Hart 0 (BSP) runs the shell loop and doesn't pick processes from the scheduler,
/// so we should prefer secondary harts for spawning daemon processes.
pub fn get_least_loaded_hart() -> usize {
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

    // Special cleanup for gpuid: clear the display
    if name == "gpuid" {
        crate::platform::d1_display::clear_display();
    }

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
        let is_gpuid = name == "gpuid";
        svc.status = ServiceStatus::Stopped;
        svc.pid = 0;
        svc.hart = None;
        
        // Release lock before cleanup
        drop(state);
        
        // Special cleanup for gpuid: clear the display
        if is_gpuid {
            crate::platform::d1_display::clear_display();
        }
        
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
