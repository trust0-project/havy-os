use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::{format, string::ToString};

use crate::{
    boot::console::{print_info, print_section, print_status},
    cpu::{self, process::{Pid, Priority, ProcessEntry}, sched},
    fence_memory, init,
    services::{
        gpuid::{self, gpuid_service},
        httpd,
        klogd::{self, klog_debug, klog_error, klog_info},
        netd,
        shelld::{self, shell_tick},
        sysmond,
        tcpd,
    }, trap,
};

static SERVICES_STARTED: AtomicUsize = AtomicUsize::new(0);


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
    let mut blk_guard = crate::lock::utils::BLK_DEV.write();

    if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
        if let Err(e) = fs.write_file(dev, "/var/log/kernel.log", boot_msg.as_bytes()) {
            klog_error("init", &format!("Failed to write boot log: {}", e));
        } else {
            // Sync to ensure data is written to disk
            let _ = fs.sync(dev);
        }
    }
}





/// Run init scripts from /etc/init.d/
/// Note: Init scripts must be WASM binaries
fn run_init_scripts() {
    let mut fs_guard = crate::FS_STATE.write();
    let mut blk_guard = crate::lock::utils::BLK_DEV.write();

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

/// Daemon service entry point for netd (network daemon)
/// Polls for IP assignment from relay. High priority service.
pub fn netd_service() {
    netd::tick();
}

fn schedule_service(
    name: &str, 
    description: &str,
    entry: ProcessEntry, 
    priority: Priority, 
    cpu_affinity: Option<usize>
) {    

    let affinity_str = match cpu_affinity {
        Some(hart) => format!("hart {}", hart),
        None => format!("any hart"),
    };
    print_status(&format!("Scheduling service: {} ({})", name, affinity_str), true);
    print_info("Registering service definition", &format!("{}", name));
    let hart = if let Some(hart) = cpu_affinity {
        hart
    } else {
        sched::SCHEDULER.find_least_loaded_cpu()
    };
    init::register_service_def(
        name,
        description,
        entry,
        priority,
        Some(hart),
    );
   
    let pid = sched::SCHEDULER.spawn_daemon_on_cpu(name, entry, priority, Some(hart));
    print_info("Started service", &format!("{} (PID {}, {})", name, pid, hart));
    init::register_service(name, pid, Some(hart));
}

pub fn init_services() {

    print_section("SERVICES");
    schedule_service(
        "klogd",
        "Kernel logger daemon - logs system memory stats",
        klogd::klogd_service,
        Priority::Normal,
        None,
    );

    schedule_service(
        "sysmond",
        "System monitor daemon - monitors system health",
        sysmond::sysmond_service,
        Priority::Normal,
        None,
    );
   
    let has_gpu = crate::platform::d1_display::is_available();
    let has_net = crate::NET_STATE.try_lock()
        .map(|g| g.is_some())
        .unwrap_or(false);

        schedule_service(
            "shelld",
            "Shell daemon - handles interactive command input",
            shelld::shell_service,
            Priority::High,
            None,  // Testing: keep on hart 0
        );

    if has_net {
        schedule_service(
            "netd",
            "Network daemon - handles IP assignment from relay",
            netd::netd_service,
            Priority::High,
            None,  // Can run on any hart
        );
    
        schedule_service(
            "tcpd",
            "TCP daemon - listens on port 30, responds with hello",
            tcpd::tcpd_service,
            Priority::Normal,
            None,
        );
    
        schedule_service(
            "httpd",
            "HTTP server daemon - listens on port 80, serves web content",
            httpd::httpd_service,
            Priority::Normal,
            None,
        );
    }

    if has_gpu {
        schedule_service(
            "gpuid",
            "GPU UI daemon - handles keyboard input and display updates",
            gpuid_service,
            Priority::High,
            None,  // Can run on any hart (touch driver is thread-safe)
        );
    }


    let services = init::service_count();
    print_status( &format!("System services started ({})", services),  services > 0);

    write_boot_log();
    run_init_scripts();
    print_status("Trap handlers initialized", true);

   
}




