// htop - Interactive process/hart viewer
//
// Usage:
//   htop        Display hart status, WASM workers, and process information

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{
        console_log, get_time, get_hart_count, get_worker_count, get_worker_stats,
        get_heap_stats, print_int,
    };

    #[no_mangle]
    pub extern "C" fn _start() {
        let uptime_ms = get_time();
        let uptime_sec = uptime_ms / 1000;
        let hart_count = get_hart_count();
        let worker_count = get_worker_count();

        // Header
        console_log("\x1b[2J\x1b[H"); // Clear screen and move to top
        console_log("\x1b[1;36m╔════════════════════════════════════════════════════════════════════╗\x1b[0m\n");
        console_log("\x1b[1;36m║\x1b[0m                  \x1b[1;97mBAVY OS - Hart Monitor (htop)\x1b[0m                   \x1b[1;36m║\x1b[0m\n");
        console_log("\x1b[1;36m╠════════════════════════════════════════════════════════════════════╣\x1b[0m\n");

        // System info row
        console_log("\x1b[1;36m║\x1b[0m  \x1b[1;33mUptime:\x1b[0m ");
        print_uptime(uptime_sec);
        console_log("   \x1b[1;33mHarts:\x1b[0m ");
        print_int(hart_count as i64);
        console_log("   \x1b[1;33mWorkers:\x1b[0m ");
        print_int(worker_count as i64);

        // Pad to fill line
        pad_to_width(50);
        console_log("\x1b[1;36m║\x1b[0m\n");

        // Memory info
        if let Some(heap) = get_heap_stats() {
            let used_kb = heap.used_bytes / 1024;
            let total_kb = heap.total_bytes / 1024;
            let pct = if heap.total_bytes > 0 {
                (heap.used_bytes * 100 / heap.total_bytes) as i64
            } else {
                0
            };

            console_log("\x1b[1;36m║\x1b[0m  \x1b[1;33mMemory:\x1b[0m ");
            print_int(used_kb as i64);
            console_log(" / ");
            print_int(total_kb as i64);
            console_log(" KiB (");
            print_int(pct);
            console_log("%)");
            pad_to_width(47);
            console_log("\x1b[1;36m║\x1b[0m\n");
        }

        console_log("\x1b[1;36m╠════════════════════════════════════════════════════════════════════╣\x1b[0m\n");

        // Hart status header
        console_log("\x1b[1;36m║\x1b[0m  \x1b[1;97mHart  Status     Jobs     Failed    Queue    Exec Time\x1b[0m       \x1b[1;36m║\x1b[0m\n");
        console_log("\x1b[1;36m║\x1b[0m  \x1b[90m─────────────────────────────────────────────────────────\x1b[0m     \x1b[1;36m║\x1b[0m\n");

        // Hart 0 (primary - shell/IO)
        console_log("\x1b[1;36m║\x1b[0m  \x1b[1;32m  0\x1b[0m   \x1b[1;32m●\x1b[0m Primary  \x1b[90m(shell/io)\x1b[0m                              \x1b[1;36m║\x1b[0m\n");

        // Worker harts
        if worker_count == 0 {
            console_log("\x1b[1;36m║\x1b[0m  \x1b[90m  (no WASM workers - single hart mode)\x1b[0m                      \x1b[1;36m║\x1b[0m\n");
        } else {
            for i in 0..worker_count {
                if let Some(stats) = get_worker_stats(i) {
                    console_log("\x1b[1;36m║\x1b[0m  ");
                    
                    // Hart ID
                    if stats.hart_id < 10 {
                        console_log("  ");
                    } else {
                        console_log(" ");
                    }
                    print_int(stats.hart_id as i64);
                    console_log("   ");

                    // Status indicator
                    if stats.current_job > 0 {
                        console_log("\x1b[1;33m●\x1b[0m Running ");
                    } else if stats.queue_depth > 0 {
                        console_log("\x1b[1;34m●\x1b[0m Queued  ");
                    } else {
                        console_log("\x1b[1;32m●\x1b[0m Idle    ");
                    }

                    // Jobs completed (6 chars)
                    print_padded_int(stats.jobs_completed as i64, 6);
                    console_log("   ");

                    // Jobs failed (6 chars)
                    print_padded_int(stats.jobs_failed as i64, 6);
                    console_log("    ");

                    // Queue depth (3 chars)
                    print_padded_int(stats.queue_depth as i64, 3);
                    console_log("      ");

                    // Exec time
                    print_exec_time(stats.total_exec_ms);

                    console_log("  \x1b[1;36m║\x1b[0m\n");
                }
            }
        }

        console_log("\x1b[1;36m╠════════════════════════════════════════════════════════════════════╣\x1b[0m\n");

        // Usage hints
        console_log("\x1b[1;36m║\x1b[0m  \x1b[1;33mCommands:\x1b[0m                                                      \x1b[1;36m║\x1b[0m\n");
        console_log("\x1b[1;36m║\x1b[0m    \x1b[97mwasmrun <file> [--hart N]\x1b[0m  Run WASM on specific hart         \x1b[1;36m║\x1b[0m\n");
        console_log("\x1b[1;36m║\x1b[0m    \x1b[97mps\x1b[0m                         Show all processes                \x1b[1;36m║\x1b[0m\n");
        console_log("\x1b[1;36m╚════════════════════════════════════════════════════════════════════╝\x1b[0m\n");
    }

    fn print_uptime(total_sec: i64) {
        let hours = total_sec / 3600;
        let minutes = (total_sec % 3600) / 60;
        let seconds = total_sec % 60;

        if hours > 0 {
            print_int(hours);
            console_log("h ");
        }
        if hours > 0 || minutes > 0 {
            print_int(minutes);
            console_log("m ");
        }
        print_int(seconds);
        console_log("s");
    }

    fn print_padded_int(n: i64, width: usize) {
        // Calculate digits
        let mut temp = if n == 0 { 1 } else { n };
        let mut digits = 0;
        while temp > 0 {
            digits += 1;
            temp /= 10;
        }
        
        // Print leading spaces
        for _ in digits..width {
            console_log(" ");
        }
        print_int(n);
    }

    fn print_exec_time(ms: u64) {
        if ms < 1000 {
            print_int(ms as i64);
            console_log("ms");
        } else if ms < 60000 {
            print_int((ms / 1000) as i64);
            console_log(".");
            print_int(((ms % 1000) / 100) as i64);
            console_log("s");
        } else {
            print_int((ms / 60000) as i64);
            console_log("m ");
            print_int(((ms % 60000) / 1000) as i64);
            console_log("s");
        }
    }

    fn pad_to_width(_remaining: usize) {
        // Simplified padding - just add some spaces
        console_log("            ");
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}

