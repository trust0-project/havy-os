// htop - Interactive process/hart viewer
//
// Usage:
//   htop        Display system status and running processes

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use core::ptr::{addr_of, addr_of_mut};
    use mkfs::{
        console_log, get_time, get_hart_count, get_worker_count,
        get_heap_stats, print_int, ps_list,
    };

    // Static buffers
    static mut PS_BUF: [u8; 2048] = [0u8; 2048];

    // Box width: 70 chars content between | borders
    const BOX_WIDTH: usize = 70;

    #[no_mangle]
    pub extern "C" fn _start() {
        let uptime_ms = get_time();
        let uptime_sec = uptime_ms / 1000;
        let hart_count = get_hart_count();
        let worker_count = get_worker_count();

        // Header
        console_log("\x1b[2J\x1b[H"); // Clear screen
        console_log("\x1b[1;36m+----------------------------------------------------------------------+\x1b[0m\n");
        console_log("\x1b[1;36m|\x1b[0m                  \x1b[1;97mBAVY OS - System Monitor (htop)\x1b[0m                   \x1b[1;36m|\x1b[0m\n");
        console_log("\x1b[1;36m+----------------------------------------------------------------------+\x1b[0m\n");

        // System info row - calculate total chars used
        console_log("\x1b[1;36m|\x1b[0m  ");
        let mut used = 2; // "  " prefix
        
        console_log("\x1b[1;33mUptime:\x1b[0m ");
        used += 8; // "Uptime: "
        used += print_uptime_count(uptime_sec);
        print_uptime(uptime_sec);
        
        console_log("   \x1b[1;33mHarts:\x1b[0m ");
        used += 11; // "   Harts: "
        used += digit_count(hart_count as u64);
        print_int(hart_count as i64);
        
        console_log("   \x1b[1;33mWorkers:\x1b[0m ");
        used += 13; // "   Workers: "
        used += digit_count(worker_count as u64);
        print_int(worker_count as i64);
        
        pad_to_box(used);
        console_log("\x1b[1;36m|\x1b[0m\n");

        // Memory info
        if let Some(heap) = get_heap_stats() {
            let used_kb = heap.used_bytes / 1024;
            let total_kb = heap.total_bytes / 1024;
            let pct = if heap.total_bytes > 0 {
                (heap.used_bytes * 100 / heap.total_bytes) as i64
            } else {
                0
            };

            console_log("\x1b[1;36m|\x1b[0m  ");
            let mut used = 2;
            
            console_log("\x1b[1;33mMemory:\x1b[0m ");
            used += 8;
            used += digit_count(used_kb);
            print_int(used_kb as i64);
            console_log(" / ");
            used += 3;
            used += digit_count(total_kb);
            print_int(total_kb as i64);
            console_log(" KiB (");
            used += 6;
            used += digit_count(pct as u64);
            print_int(pct);
            console_log("%)");
            used += 2;
            
            // Memory bar (fixed 24 chars: " [" + 20 bar + "]")
            console_log(" [");
            used += 2;
            let bar_width = 20;
            let filled = (pct as usize * bar_width / 100).min(bar_width);
            for i in 0..bar_width {
                if i < filled {
                    console_log("\x1b[1;32m#\x1b[0m");
                } else {
                    console_log("\x1b[90m.\x1b[0m");
                }
            }
            console_log("]");
            used += bar_width + 1;
            
            pad_to_box(used);
            console_log("\x1b[1;36m|\x1b[0m\n");
        }

        console_log("\x1b[1;36m+----------------------------------------------------------------------+\x1b[0m\n");

        // Process list header
        console_log("\x1b[1;36m|\x1b[0m  \x1b[1;97m  PID  STATE  HART  UPTIME  NAME                            \x1b[0m      \x1b[1;36m|\x1b[0m\n");
        console_log("\x1b[1;36m|\x1b[0m  \x1b[90m--------------------------------------------------------------------\x1b[0m  \x1b[1;36m|\x1b[0m\n");

        // Get process list
        let ps_len = unsafe { ps_list((*addr_of_mut!(PS_BUF)).as_mut_ptr(), 2048) };
        
        if ps_len > 0 {
            let data = unsafe { &(*addr_of!(PS_BUF))[..ps_len as usize] };
            let mut start = 0;
            let mut count = 0;
            
            for i in 0..ps_len as usize {
                if data[i] == b'\n' {
                    if i > start && count < 8 { // Show max 8 processes
                        display_process(&data[start..i]);
                        count += 1;
                    }
                    start = i + 1;
                }
            }
            // Handle last line
            if start < ps_len as usize && count < 8 {
                display_process(&data[start..ps_len as usize]);
            }
        } else {
            console_log("\x1b[1;36m|\x1b[0m  \x1b[90m(no processes)\x1b[0m                                                      \x1b[1;36m|\x1b[0m\n");
        }

        console_log("\x1b[1;36m+----------------------------------------------------------------------+\x1b[0m\n");

        // Legend
        console_log("\x1b[1;36m|\x1b[0m  \x1b[1;33mLegend:\x1b[0m \x1b[1;32mR+\x1b[0m=Running  R=Ready  \x1b[33mS\x1b[0m=Sleeping  \x1b[31mZ\x1b[0m=Zombie                \x1b[1;36m|\x1b[0m\n");
        console_log("\x1b[1;36m|\x1b[0m  \x1b[1;33mCommands:\x1b[0m ps, kill <pid>, service status                          \x1b[1;36m|\x1b[0m\n");
        console_log("\x1b[1;36m+----------------------------------------------------------------------+\x1b[0m\n");
    }

    fn pad_to_box(used: usize) {
        let remaining = if BOX_WIDTH > used { BOX_WIDTH - used } else { 0 };
        for _ in 0..remaining {
            console_log(" ");
        }
    }

    fn digit_count(mut n: u64) -> usize {
        if n == 0 { return 1; }
        let mut count = 0;
        while n > 0 {
            count += 1;
            n /= 10;
        }
        count
    }

    fn print_uptime_count(total_sec: i64) -> usize {
        let hours = total_sec / 3600;
        let minutes = (total_sec % 3600) / 60;
        let seconds = total_sec % 60;
        let mut len = 0;
        if hours > 0 {
            len += digit_count(hours as u64) + 2; // "Xh "
        }
        if hours > 0 || minutes > 0 {
            len += digit_count(minutes as u64) + 2; // "Xm "
        }
        len += digit_count(seconds as u64) + 1; // "Xs"
        len
    }

    fn display_process(line: &[u8]) {
        // Parse: "pid:name:state:priority:cpu:uptime"
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
        let cpu_slice = &line[colon_pos[3]+1..colon_pos[4]];
        let uptime_slice = &line[colon_pos[4]+1..];
        
        let pid = parse_u64(pid_slice);
        let cpu = parse_u64(cpu_slice);
        let uptime_ms = parse_u64(uptime_slice);
        let uptime_sec = uptime_ms / 1000;

        console_log("\x1b[1;36m|\x1b[0m  ");
        let mut used: usize = 2;
        
        // Color based on state
        if state_slice == b"R+" {
            console_log("\x1b[1;32m");
        } else if state_slice == b"S" {
            console_log("\x1b[33m");
        } else if state_slice == b"Z" {
            console_log("\x1b[31m");
        }

        // PID (5 chars)
        print_padded_int(pid as i64, 5);
        console_log("  ");
        used += 7;

        // State (5 chars)
        print_bytes(state_slice);
        for _ in state_slice.len()..5 {
            console_log(" ");
        }
        console_log("  ");
        used += 7;

        // Hart (4 chars)
        print_padded_int(cpu as i64, 4);
        console_log("  ");
        used += 6;

        // Uptime (6 chars)
        print_padded_int(uptime_sec as i64, 5);
        console_log("s ");
        used += 7;

        // Name (remaining space minus padding for border)
        let max_name = 32;
        if name_slice.len() <= max_name {
            print_bytes(name_slice);
            used += name_slice.len();
        } else {
            print_bytes(&name_slice[..max_name-2]);
            console_log("..");
            used += max_name;
        }

        console_log("\x1b[0m");
        pad_to_box(used);
        console_log("\x1b[1;36m|\x1b[0m\n");
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
        let mut digits = 0;
        while temp > 0 {
            digits += 1;
            temp /= 10;
        }
        if n < 0 {
            digits += 1;
        }
        
        for _ in digits..width {
            console_log(" ");
        }
        print_int(n);
    }

    fn print_bytes(bytes: &[u8]) {
        unsafe { mkfs::print(bytes.as_ptr(), bytes.len()) };
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
