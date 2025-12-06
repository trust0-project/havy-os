// service - Service management
//
// Usage:
//   service --list              List available services
//   service --status-all        Show status of all running services
//   service <name> start        Start a service
//   service <name> stop         Stop a service
//   service <name> restart      Restart a service
//   service <name> status       Show service status

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{
        console_log, argc, argv, get_service_defs, get_running_services,
        start_service, stop_service, restart_service, get_service_status, print_int,
    };

    // Static buffers
    static mut ARG_BUF: [u8; 128] = [0u8; 128];
    static mut ARG2_BUF: [u8; 64] = [0u8; 64];
    static mut LIST_BUF: [u8; 4096] = [0u8; 4096];
    static mut STATUS_BUF: [u8; 64] = [0u8; 64];

    #[no_mangle]
    pub extern "C" fn _start() {
        let arg_count = argc();
        
        // Note: command name is not passed, so arg 0 is the first real argument
        if arg_count < 1 {
            console_log("Usage: service <name> {start|stop|restart|status}\n");
            console_log("       service --list\n");
            return;
        }

        // Get first argument (index 0 since command name not passed)
        let arg1_len = unsafe { argv(0, &mut ARG_BUF) };
        let Some(arg1_len) = arg1_len else {
            console_log("Error: Could not read argument\n");
            return;
        };
        let arg1 = unsafe { &ARG_BUF[..arg1_len] };

        // Check for --list or -l
        if arg1 == b"--list" || arg1 == b"-l" {
            list_service_definitions();
            return;
        }

        // Check for --status-all or -a
        if arg1 == b"--status-all" || arg1 == b"-a" {
            show_all_status();
            return;
        }

        // Otherwise, expect <name> <command>
        if arg_count < 2 {
            console_log("Usage: service ");
            print_bytes(arg1);
            console_log(" {start|stop|restart|status}\n");
            return;
        }

        let arg2_len = unsafe { argv(1, &mut ARG2_BUF) };
        let Some(arg2_len) = arg2_len else {
            console_log("Error: Could not read command\n");
            return;
        };
        let arg2 = unsafe { &ARG2_BUF[..arg2_len] };
        let name = unsafe { core::str::from_utf8_unchecked(arg1) };

        match arg2 {
            b"start" => {
                console_log("Starting ");
                print_bytes(arg1);
                console_log("...\n");
                if start_service(name) {
                    console_log("\x1b[1;32m[OK]\x1b[0m\n");
                } else {
                    console_log("\x1b[1;31m[FAIL]\x1b[0m Service not found or already running\n");
                }
            }
            b"stop" => {
                console_log("Stopping ");
                print_bytes(arg1);
                console_log("...\n");
                if stop_service(name) {
                    console_log("\x1b[1;32m[OK]\x1b[0m\n");
                } else {
                    console_log("\x1b[1;31m[FAIL]\x1b[0m Service not found or not running\n");
                }
            }
            b"restart" => {
                console_log("Restarting ");
                print_bytes(arg1);
                console_log("...\n");
                if restart_service(name) {
                    console_log("\x1b[1;32m[OK]\x1b[0m\n");
                } else {
                    console_log("\x1b[1;31m[FAIL]\x1b[0m\n");
                }
            }
            b"status" => {
                show_service_status(name, arg1);
            }
            _ => {
                console_log("Unknown command: ");
                print_bytes(arg2);
                console_log("\nValid commands: start, stop, restart, status\n");
            }
        }
    }

    fn list_service_definitions() {
        console_log("\x1b[1;36mAvailable services:\x1b[0m\n");
        
        let len = unsafe { get_service_defs(&mut LIST_BUF) };
        let Some(len) = len else {
            console_log("  (none)\n");
            return;
        };

        if len == 0 {
            console_log("  (none)\n");
            return;
        }

        // Parse and display "name:description\n" format
        let data = unsafe { &LIST_BUF[..len] };
        let mut start = 0;
        for i in 0..len {
            if data[i] == b'\n' {
                if i > start {
                    let line = &data[start..i];
                    if let Some(colon) = line.iter().position(|&b| b == b':') {
                        let name = &line[..colon];
                        let desc = &line[colon + 1..];
                        console_log("  ");
                        print_bytes(name);
                        console_log(" - ");
                        print_bytes(desc);
                        console_log("\n");
                    }
                }
                start = i + 1;
            }
        }
    }

    fn show_all_status() {
        console_log("\x1b[1;36mService Status:\x1b[0m\n");
        
        let len = unsafe { get_running_services(&mut LIST_BUF) };
        let Some(len) = len else {
            console_log("  (no services)\n");
            return;
        };

        if len == 0 {
            console_log("  (no services)\n");
            return;
        }

        // Parse and display "name:status:pid\n" format
        let data = unsafe { &LIST_BUF[..len] };
        let mut start = 0;
        for i in 0..len {
            if data[i] == b'\n' {
                if i > start {
                    let line = &data[start..i];
                    // Find first colon (after name)
                    if let Some(colon1) = line.iter().position(|&b| b == b':') {
                        let name = &line[..colon1];
                        let rest = &line[colon1 + 1..];
                        // Find second colon (after status)
                        if let Some(colon2) = rest.iter().position(|&b| b == b':') {
                            let status = &rest[..colon2];
                            
                            let color = if status == b"running" {
                                "\x1b[1;32m"
                            } else if status == b"stopped" {
                                "\x1b[1;31m"
                            } else {
                                "\x1b[1;33m"
                            };
                            
                            console_log("  ");
                            print_bytes(name);
                            // Pad name to 12 chars
                            for _ in name.len()..12 {
                                console_log(" ");
                            }
                            console_log(color);
                            print_bytes(status);
                            console_log("\x1b[0m\n");
                        }
                    }
                }
                start = i + 1;
            }
        }
    }

    fn show_service_status(name: &str, name_bytes: &[u8]) {
        let status_len = unsafe { get_service_status(name, &mut STATUS_BUF) };
        
        // Get running services to find PID
        let list_len = unsafe { get_running_services(&mut LIST_BUF) };
        
        let mut found = false;
        let mut pid: i64 = 0;
        
        if let Some(list_len) = list_len {
            let data = unsafe { &LIST_BUF[..list_len] };
            let mut start = 0;
            for i in 0..list_len {
                if data[i] == b'\n' {
                    if i > start {
                        let line = &data[start..i];
                        // Parse "name:status:pid"
                        if let Some(colon1) = line.iter().position(|&b| b == b':') {
                            let svc_name = &line[..colon1];
                            if svc_name == name_bytes {
                                found = true;
                                let rest = &line[colon1 + 1..];
                                if let Some(colon2) = rest.iter().position(|&b| b == b':') {
                                    let pid_str = &rest[colon2 + 1..];
                                    pid = parse_int(pid_str);
                                }
                            }
                        }
                    }
                    start = i + 1;
                }
            }
        }

        if !found {
            console_log("Service '");
            print_bytes(name_bytes);
            console_log("' not found\n");
            return;
        }

        console_log("● ");
        print_bytes(name_bytes);
        console_log("\n");

        if let Some(status_len) = status_len {
            let status = unsafe { &STATUS_BUF[..status_len] };
            if status == b"running" {
                console_log("   \x1b[1;32mActive: running\x1b[0m\n");
                console_log("   PID: ");
                print_int(pid);
                console_log("\n");
            } else {
                console_log("   \x1b[1;31mActive: ");
                print_bytes(status);
                console_log("\x1b[0m\n");
            }
        }
    }

    fn parse_int(bytes: &[u8]) -> i64 {
        let mut n: i64 = 0;
        for &b in bytes {
            if b >= b'0' && b <= b'9' {
                n = n * 10 + (b - b'0') as i64;
            }
        }
        n
    }

    fn print_bytes(bytes: &[u8]) {
        unsafe { mkfs::print(bytes.as_ptr(), bytes.len()) };
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}

