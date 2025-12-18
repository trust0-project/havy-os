// ps - List processes
//
// Usage:
//   ps        Show all running processes

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use core::ptr::{addr_of, addr_of_mut};
    use mkfs::{console_log, print_int, ps_list};

    // Use a static buffer to avoid large stack allocations
    static mut BUF: [u8; 2048] = [0u8; 2048];

    #[no_mangle]
    pub extern "C" fn _start() {
        // Header
        console_log("\x1b[1;36m  PID  STATE  PRI  CPU    UPTIME  NAME\x1b[0m\n");
        console_log("\x1b[90m-----------------------------------------------------\x1b[0m\n");

        // Get process list
        let len = unsafe { ps_list((*addr_of_mut!(BUF)).as_mut_ptr(), 2048) };
        
        if len < 0 {
            console_log("\x1b[1;31mError:\x1b[0m Failed to get process list\n");
        } else if len == 0 {
            console_log("\x1b[90m  (no processes)\x1b[0m\n");
        } else {
            // Parse and display each process
            let data = unsafe { &(*addr_of!(BUF))[..len as usize] };
            let mut start = 0;
            
            for i in 0..len as usize {
                if data[i] == b'\n' {
                    if i > start {
                        display_task(&data[start..i]);
                    }
                    start = i + 1;
                }
            }
            // Handle last line if no trailing newline
            if start < len as usize {
                display_task(&data[start..len as usize]);
            }
        }

        console_log("\n");
        console_log("\x1b[90mStates: R=Ready R+=Running S=Sleeping Z=Zombie | CPU: hart # (-1 = not running)\x1b[0m\n");
    }

    fn display_task(line: &[u8]) {
        // Parse: "pid:name:state:priority:cpu:uptime"
        // Find field boundaries by scanning for colons
        let mut colon_pos = [0usize; 5];
        let mut colon_count = 0;
        
        for (i, &b) in line.iter().enumerate() {
            if b == b':' && colon_count < 5 {
                colon_pos[colon_count] = i;
                colon_count += 1;
            }
        }
        
        if colon_count < 5 {
            return; // Invalid line
        }
        
        // Extract fields as slices
        let pid_slice = &line[0..colon_pos[0]];
        let name_slice = &line[colon_pos[0]+1..colon_pos[1]];
        let state_slice = &line[colon_pos[1]+1..colon_pos[2]];
        let priority_slice = &line[colon_pos[2]+1..colon_pos[3]];
        let cpu_slice = &line[colon_pos[3]+1..colon_pos[4]];
        let uptime_slice = &line[colon_pos[4]+1..];
        
        let pid = parse_u64(pid_slice);
        let cpu = parse_u64(cpu_slice);  // Hart number (-1 if not running)
        let uptime_ms = parse_u64(uptime_slice);
        let uptime_sec = uptime_ms / 1000;

        // Color based on state
        let color = if state_slice == b"R+" {
            "\x1b[1;32m"
        } else if state_slice == b"S" {
            "\x1b[33m"
        } else if state_slice == b"Z" {
            "\x1b[31m"
        } else {
            "\x1b[0m"
        };

        console_log(color);

        // PID (5 chars, right-aligned)
        print_padded_int(pid as i64, 5);
        console_log("  ");

        // State (6 chars, left-aligned)
        print_bytes(state_slice);
        pad_spaces(6 - state_slice.len());
        console_log(" ");

        // Priority (6 chars, left-aligned)
        print_bytes(priority_slice);
        pad_spaces(6 - priority_slice.len());
        console_log(" ");

        // CPU (hart number, 3 chars, right-aligned)
        print_padded_int(cpu as i64, 3);
        console_log(" ");

        // Uptime (7 chars + "s", right-aligned)
        print_padded_int(uptime_sec as i64, 7);
        console_log("s  ");

        // Name
        print_bytes(name_slice);
        console_log("\x1b[0m\n");
    }

    fn parse_u64(bytes: &[u8]) -> u64 {
        let mut n: u64 = 0;
        for &b in bytes {
            if b >= b'0' && b <= b'9' {
                n = n.saturating_mul(10).saturating_add((b - b'0') as u64);
            } else {
                break;  // Stop at first non-digit!
            }
        }
        n
    }

    fn parse_i64(bytes: &[u8]) -> i64 {
        let mut n: i64 = 0;
        let mut negative = false;
        let mut started = false;

        for &b in bytes {
            match b {
                b'-' if !started => {
                    negative = true;
                    started = true;
                }
                b'0'..=b'9' => {
                    n = n.saturating_mul(10).saturating_add((b - b'0') as i64);
                    started = true;
                }
                _ if started => break,  // Stop at first non-digit after starting!
                _ => {}
            }
        }

        if negative { -n } else { n }
    }

    fn print_padded_int(n: i64, width: usize) {
        // Calculate digits
        let mut temp = if n == 0 { 1 } else { n.abs() };
        let mut digits = 0usize;
        while temp > 0 {
            digits += 1;
            temp /= 10;
        }
        if n < 0 {
            digits += 1;
        }

        // Print leading spaces
        pad_spaces(width.saturating_sub(digits));
        print_int(n);
    }

    fn print_bytes(bytes: &[u8]) {
        unsafe { mkfs::print(bytes.as_ptr(), bytes.len()) };
    }
    
    fn pad_spaces(count: usize) {
        for _ in 0..count {
            console_log(" ");
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}

