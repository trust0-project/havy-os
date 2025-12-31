// ps - List processes
//
// Usage:
//   ps        Show all running processes

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{console_log, print, print_int, ps_list};

    // Use a static buffer to avoid large stack allocations
    static mut BUF: [u8; 2048] = [0u8; 2048];

    fn parse_u64(bytes: &[u8]) -> u64 {
        let mut n: u64 = 0;
        for &b in bytes {
            if b >= b'0' && b <= b'9' {
                n = n.saturating_mul(10).saturating_add((b - b'0') as u64);
            } else {
                break;
            }
        }
        n
    }

    fn print_padded_int(n: i64, width: usize) {
        let mut temp = if n == 0 { 1 } else { n.abs() };
        let mut digits = 0usize;
        while temp > 0 {
            digits += 1;
            temp /= 10;
        }
        if n < 0 {
            digits += 1;
        }
        for _ in 0..width.saturating_sub(digits) {
            console_log(" ");
        }
        print_int(n);
    }

    fn display_task(line: &[u8]) {
        // Format from kernel: pid:name:state:priority:cpu_time_ms:uptime_ms
        let mut colon_pos = [0usize; 5];
        let mut colon_count = 0;
        
        for (i, &b) in line.iter().enumerate() {
            if b == b':' && colon_count < 5 {
                colon_pos[colon_count] = i;
                colon_count += 1;
            }
        }
        
        if colon_count < 5 {
            return;
        }
        
        let pid_slice = &line[0..colon_pos[0]];
        let name_slice = &line[colon_pos[0]+1..colon_pos[1]];
        let state_slice = &line[colon_pos[1]+1..colon_pos[2]];
        let priority_slice = &line[colon_pos[2]+1..colon_pos[3]];
        let cpu_time_slice = &line[colon_pos[3]+1..colon_pos[4]];
        let uptime_slice = &line[colon_pos[4]+1..];
        
        let pid = parse_u64(pid_slice);
        let cpu_time_ms = parse_u64(cpu_time_slice);
        let uptime_ms = parse_u64(uptime_slice);
        let uptime_sec = uptime_ms / 1000;

        // Color based on state
        let color = if state_slice == b"R+" || state_slice == b"R" {
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
        print(state_slice.as_ptr(), state_slice.len());
        for _ in 0..(6 - state_slice.len()) { console_log(" "); }
        console_log(" ");
        
        // Priority (4 chars, left-aligned)
        print(priority_slice.as_ptr(), priority_slice.len());
        for _ in 0..(4 - priority_slice.len()) { console_log(" "); }
        console_log(" ");
        
        // CPU time in ms (7 chars, right-aligned)
        print_padded_int(cpu_time_ms as i64, 7);
        console_log("ms ");
        
        // Uptime (6 chars + "s", right-aligned)
        print_padded_int(uptime_sec as i64, 6);
        console_log("s  ");
        
        // Name
        print(name_slice.as_ptr(), name_slice.len());
        console_log("\x1b[0m\n");
    }

    // Header - updated to reflect actual data: cpu_time_ms and uptime
    console_log("\x1b[1;36m  PID  STATE  PRI  CPU TIME  UPTIME  NAME\x1b[0m\n");
    console_log("\x1b[90m-------------------------------------------------------\x1b[0m\n");

    let len = unsafe { ps_list((*core::ptr::addr_of_mut!(BUF)).as_mut_ptr(), 2048) };
    
    if len < 0 {
        console_log("\x1b[1;31mError:\x1b[0m Failed to get process list\n");
    } else if len == 0 {
        console_log("\x1b[90m  (no processes)\x1b[0m\n");
    } else {
        let data = unsafe { &(*core::ptr::addr_of!(BUF))[..len as usize] };
        let mut start = 0;
        
        for i in 0..len as usize {
            if data[i] == b'\n' {
                if i > start {
                    display_task(&data[start..i]);
                }
                start = i + 1;
            }
        }
        if start < len as usize {
            display_task(&data[start..len as usize]);
        }
    }

    console_log("\n");
    console_log("\x1b[90mStates: R=Ready/Running S=Sleeping Z=Zombie | CPU TIME: accumulated ms\x1b[0m\n");
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
