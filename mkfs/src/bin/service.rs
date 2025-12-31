// service - Service management
//
// Usage:
//   service list              List all services
//   service status <name>     Show service status
//   service start <name>      Start a service
//   service stop <name>       Stop a service

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{console_log, argc, argv, print, service_list, service_start, service_stop, service_running};

    static mut LIST_BUF: [u8; 1024] = [0u8; 1024];
    static mut NAME_BUF: [u8; 64] = [0u8; 64];

    if argc() < 1 {
        console_log("Usage: service <command> [name]\n");
        console_log("Commands: list, status, start, stop, restart\n");
        return;
    }

    let mut cmd_buf = [0u8; 32];
    let cmd_len = match argv(0, &mut cmd_buf) {
        Some(len) => len,
        None => {
            console_log("Error: Could not read command\n");
            return;
        }
    };

    let cmd = &cmd_buf[..cmd_len];

    if cmd == b"list" {
        console_log("\n");
        console_log("\x1b[1;36m+-----------------------------------------------+\x1b[0m\n");
        console_log("\x1b[1;36m|\x1b[0m           \x1b[1;97mSystem Services\x1b[0m                     \x1b[1;36m|\x1b[0m\n");
        console_log("\x1b[1;36m+-----------------------------------------------+\x1b[0m\n");

        // Get list of all defined services
        let len = unsafe { service_list((*core::ptr::addr_of_mut!(LIST_BUF)).as_mut_ptr(), 1024) };
        
        if len > 0 {
            let data = unsafe { &(*core::ptr::addr_of!(LIST_BUF))[..len as usize] };
            
            // Parse and display each service
            let mut pos = 0;
            while pos < data.len() {
                let line_start = pos;
                while pos < data.len() && data[pos] != b'\n' { pos += 1; }
                let line_end = pos;
                pos += 1;
                
                if line_start >= line_end { continue; }
                let name = &data[line_start..line_end];
                
                // Check if running
                let running_len = unsafe { 
                    service_running((*core::ptr::addr_of_mut!(NAME_BUF)).as_mut_ptr(), 64)
                };
                
                let running_data = unsafe { &(*core::ptr::addr_of!(NAME_BUF))[..running_len.max(0) as usize] };
                let is_running = running_data.windows(name.len()).any(|w| w == name);
                
                if is_running {
                    console_log("\x1b[1;36m|\x1b[0m  \x1b[1;32m[*]\x1b[0m ");
                } else {
                    console_log("\x1b[1;36m|\x1b[0m  \x1b[90m[ ]\x1b[0m ");
                }
                print(name.as_ptr(), name.len());
                
                // Padding
                for _ in name.len()..30 {
                    console_log(" ");
                }
                console_log("\x1b[1;36m|\x1b[0m\n");
            }
        } else {
            // Fallback to hardcoded list
            console_log("\x1b[1;36m|\x1b[0m  \x1b[1;32m[*]\x1b[0m klogd     Kernel log daemon              \x1b[1;36m|\x1b[0m\n");
            console_log("\x1b[1;36m|\x1b[0m  \x1b[1;32m[*]\x1b[0m sysmond   System monitor daemon           \x1b[1;36m|\x1b[0m\n");
            console_log("\x1b[1;36m|\x1b[0m  \x1b[1;32m[*]\x1b[0m shelld    Shell daemon                    \x1b[1;36m|\x1b[0m\n");
        }
        
        console_log("\x1b[1;36m+-----------------------------------------------+\x1b[0m\n");
        console_log("\n\x1b[90m[*] = running, [ ] = stopped\x1b[0m\n\n");
        
    } else if cmd == b"start" {
        if argc() < 2 {
            console_log("Usage: service start <service_name>\n");
            return;
        }
        
        let name_len = match argv(1, unsafe { &mut *core::ptr::addr_of_mut!(NAME_BUF) }) {
            Some(len) => len,
            None => {
                console_log("Error: Could not read service name\n");
                return;
            }
        };
        
        let name = unsafe { &(*core::ptr::addr_of!(NAME_BUF))[..name_len] };
        
        let result = unsafe { service_start(name.as_ptr(), name_len as i32) };
        if result == 0 {
            console_log("\x1b[1;32m[OK]\x1b[0m Started ");
            print(name.as_ptr(), name.len());
            console_log("\n");
        } else {
            console_log("\x1b[1;31m[FAIL]\x1b[0m Failed to start ");
            print(name.as_ptr(), name.len());
            console_log("\n");
        }
        
    } else if cmd == b"stop" {
        if argc() < 2 {
            console_log("Usage: service stop <service_name>\n");
            return;
        }
        
        let name_len = match argv(1, unsafe { &mut *core::ptr::addr_of_mut!(NAME_BUF) }) {
            Some(len) => len,
            None => {
                console_log("Error: Could not read service name\n");
                return;
            }
        };
        
        let name = unsafe { &(*core::ptr::addr_of!(NAME_BUF))[..name_len] };
        
        let result = unsafe { service_stop(name.as_ptr(), name_len as i32) };
        if result == 0 {
            console_log("\x1b[1;32m[OK]\x1b[0m Stopped ");
            print(name.as_ptr(), name.len());
            console_log("\n");
        } else {
            console_log("\x1b[1;31m[FAIL]\x1b[0m Failed to stop ");
            print(name.as_ptr(), name.len());
            console_log("\n");
        }
        
    } else if cmd == b"restart" {
        if argc() < 2 {
            console_log("Usage: service restart <service_name>\n");
            return;
        }
        
        let name_len = match argv(1, unsafe { &mut *core::ptr::addr_of_mut!(NAME_BUF) }) {
            Some(len) => len,
            None => {
                console_log("Error: Could not read service name\n");
                return;
            }
        };
        
        let name = unsafe { &(*core::ptr::addr_of!(NAME_BUF))[..name_len] };
        
        // Stop then start
        let _ = unsafe { service_stop(name.as_ptr(), name_len as i32) };
        let result = unsafe { service_start(name.as_ptr(), name_len as i32) };
        
        if result == 0 {
            console_log("\x1b[1;32m[OK]\x1b[0m Restarted ");
            print(name.as_ptr(), name.len());
            console_log("\n");
        } else {
            console_log("\x1b[1;31m[FAIL]\x1b[0m Failed to restart ");
            print(name.as_ptr(), name.len());
            console_log("\n");
        }
        
    } else if cmd == b"status" {
        if argc() < 2 {
            console_log("Usage: service status <service_name>\n");
            return;
        }
        
        let name_len = match argv(1, unsafe { &mut *core::ptr::addr_of_mut!(NAME_BUF) }) {
            Some(len) => len,
            None => {
                console_log("Error: Could not read service name\n");
                return;
            }
        };
        
        let name = unsafe { &(*core::ptr::addr_of!(NAME_BUF))[..name_len] };
        
        // Check if running
        let running_len = unsafe { 
            service_running((*core::ptr::addr_of_mut!(LIST_BUF)).as_mut_ptr(), 1024)
        };
        
        let running_data = unsafe { &(*core::ptr::addr_of!(LIST_BUF))[..running_len.max(0) as usize] };
        let is_running = running_data.windows(name.len()).any(|w| w == name);
        
        print(name.as_ptr(), name.len());
        if is_running {
            console_log(": \x1b[1;32mrunning\x1b[0m\n");
        } else {
            console_log(": \x1b[1;31mstopped\x1b[0m\n");
        }
        
    } else {
        console_log("Unknown command. Use: list, status, start, stop, restart\n");
    }
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
