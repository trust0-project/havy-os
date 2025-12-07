// top - Process monitor
//
// Usage:
//   top              Display system status and process list
//   top -n <N>       Update N times (default: 1)
//   top -b           Batch mode (no screen clear)

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{
        console_log, get_time, get_heap_stats, get_hart_count, get_ps_list,
        get_version, print_int, argc, argv, sleep,
    };

    // Static buffers
    static mut ARG_BUF: [u8; 32] = [0u8; 32];
    static mut VERSION_BUF: [u8; 32] = [0u8; 32];
    static mut PS_BUF: [u8; 4096] = [0u8; 4096];

    // Task entry for sorting
    #[derive(Clone, Copy)]
    struct TaskEntry {
        pid: u64,
        name: [u8; 32],
        name_len: usize,
        state: [u8; 8],
        state_len: usize,
        priority: [u8; 8],
        priority_len: usize,
        cpu: i64,  // Hart number (-1 if not running)
        uptime: u64,
    }

    // Static array for tasks
    static mut TASKS: [TaskEntry; 32] = [TaskEntry {
        pid: 0,
        name: [0u8; 32],
        name_len: 0,
        state: [0u8; 8],
        state_len: 0,
        priority: [0u8; 8],
        priority_len: 0,
        cpu: -1,
        uptime: 0,
    }; 32];

    #[no_mangle]
    pub extern "C" fn _start() {
        let mut iterations = 1;
        let mut batch_mode = false;

        // Parse arguments (starting from 0 since command name is not passed)
        let arg_count = argc();
        let mut i = 0;
        while i < arg_count {
            let len = unsafe { argv(i, &mut ARG_BUF) };
            if let Some(len) = len {
                let arg = unsafe { &ARG_BUF[..len] };
                if arg == b"-n" {
                    i += 1;
                    if i < arg_count {
                        if let Some(n_len) = unsafe { argv(i, &mut ARG_BUF) } {
                            iterations = parse_usize(unsafe { &ARG_BUF[..n_len] });
                            if iterations == 0 {
                                iterations = 1;
                            }
                        }
                    }
                } else if arg == b"-b" {
                    batch_mode = true;
                }
            }
            i += 1;
        }

        for iter_num in 0..iterations {
            if !batch_mode && iter_num == 0 {
                // Clear screen
                console_log("\x1b[2J\x1b[H");
            }

            display_top(batch_mode);

            if iterations > 1 && iter_num < iterations - 1 {
                sleep(1000);
                if !batch_mode {
                    console_log("\x1b[2J\x1b[H");
                }
            }
        }
    }

    fn display_top(batch_mode: bool) {
        let uptime = get_time();
        let harts = get_hart_count();

        // Get version
        let version_len = unsafe { get_version(&mut VERSION_BUF).unwrap_or(0) };

        // Header
        if batch_mode {
            console_log("===================================================================\n");
            console_log("BAVY OS v");
            if version_len > 0 {
                unsafe { print_bytes(&VERSION_BUF[..version_len]) };
            }
            console_log(" - ");
            print_uptime(uptime / 1000);
            console_log(" up, ");
            print_int(harts as i64);
            console_log(" hart(s)\n");
        } else {
            console_log("\x1b[1;36m===================================================================\x1b[0m\n");
            console_log("\x1b[1;97m BAVY OS v");
            if version_len > 0 {
                unsafe { print_bytes(&VERSION_BUF[..version_len]) };
            }
            console_log("\x1b[0m - ");
            print_uptime(uptime / 1000);
            console_log(" up, \x1b[1;32m");
            print_int(harts as i64);
            console_log("\x1b[0m hart(s)\n");
        }

        // Memory bar
        if let Some(stats) = get_heap_stats() {
            let total = stats.total_bytes;
            let used = stats.used_bytes;
            let pct = if total > 0 { (used * 100) / total } else { 0 };
            let bar_width: u64 = 30;
            let filled = (pct * bar_width) / 100;

            console_log("Mem: [");
            for j in 0..bar_width {
                if j < filled {
                    if pct > 80 {
                        console_log("\x1b[1;31m#\x1b[0m");
                    } else if pct > 60 {
                        console_log("\x1b[1;33m#\x1b[0m");
                    } else {
                        console_log("\x1b[1;32m#\x1b[0m");
                    }
                } else {
                    console_log("\x1b[0;90m.\x1b[0m");
                }
            }
            console_log("] ");
            print_int(pct as i64);
            console_log("% (");
            print_int((used / 1024) as i64);
            console_log("/");
            print_int((total / 1024) as i64);
            console_log(" KB)\n");
        }

        // Get and parse tasks
        let ps_len = unsafe { get_ps_list(&mut PS_BUF) };
        let task_count = if let Some(ps_len) = ps_len {
            parse_tasks(unsafe { &PS_BUF[..ps_len] })
        } else {
            0
        };

        // Count states
        let mut running = 0usize;
        let mut sleeping = 0usize;
        for i in 0..task_count {
            let state = unsafe { &TASKS[i].state[..TASKS[i].state_len] };
            if state == b"R+" {
                running += 1;
            } else if state == b"S" {
                sleeping += 1;
            }
        }

        console_log("Tasks: \x1b[1m");
        print_int(task_count as i64);
        console_log("\x1b[0m total, \x1b[1;32m");
        print_int(running as i64);
        console_log("\x1b[0m running, \x1b[1;33m");
        print_int(sleeping as i64);
        console_log("\x1b[0m sleeping\n\n");

        console_log("\x1b[1;7m  PID  STATE  PRI  CPU    UPTIME  NAME                          \x1b[0m\n");

        // Sort tasks by PID (ascending) - simple bubble sort
        for i in 0..task_count {
            for j in 0..task_count - 1 - i {
                unsafe {
                    if TASKS[j].pid > TASKS[j + 1].pid {
                        // Swap
                        let tmp = TASKS[j];
                        TASKS[j] = TASKS[j + 1];
                        TASKS[j + 1] = tmp;
                    }
                }
            }
        }

        // Display tasks
        for i in 0..task_count {
            let task = unsafe { &TASKS[i] };
            let state = &task.state[..task.state_len];

            let color = if state == b"R+" {
                "\x1b[1;32m"
            } else if state == b"S" {
                "\x1b[33m"
            } else if state == b"Z" {
                "\x1b[1;31m"
            } else {
                ""
            };

            console_log(color);

            // PID (5 chars, right-aligned)
            print_padded_int(task.pid as i64, 5);
            console_log("  ");

            // State (6 chars, left-aligned)
            print_bytes(state);
            pad_spaces(6 - task.state_len.min(6));
            console_log(" ");

            // Priority (6 chars, left-aligned)
            let priority = &task.priority[..task.priority_len];
            print_bytes(priority);
            pad_spaces(6 - task.priority_len.min(6));
            console_log(" ");

            // CPU (hart number, 3 chars, right-aligned)
            print_padded_int(task.cpu as i64, 3);
            console_log(" ");

            // Uptime
            print_uptime((task.uptime / 1000) as i64);
            console_log("  ");

            // Name
            let name = &task.name[..task.name_len];
            print_bytes(name);
            console_log("\x1b[0m\n");
        }

        console_log("\n\x1b[1;36m-----------------------------------------------------------------\x1b[0m\n");
    }

    fn parse_tasks(data: &[u8]) -> usize {
        let mut count = 0;
        let mut start = 0;

        for i in 0..data.len() {
            if data[i] == b'\n' {
                if i > start && count < 32 {
                    parse_task_line(&data[start..i], count);
                    count += 1;
                }
                start = i + 1;
            }
        }
        // Handle last line if no trailing newline
        if start < data.len() && count < 32 {
            parse_task_line(&data[start..], count);
            count += 1;
        }
        count
    }

    fn parse_task_line(line: &[u8], idx: usize) {
        // Format: "pid:name:state:priority:cpu_time:uptime"
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
        let name_slice = &line[colon_pos[0] + 1..colon_pos[1]];
        let state_slice = &line[colon_pos[1] + 1..colon_pos[2]];
        let priority_slice = &line[colon_pos[2] + 1..colon_pos[3]];
        let cpu_slice = &line[colon_pos[3] + 1..colon_pos[4]];
        let uptime_slice = &line[colon_pos[4] + 1..];

        unsafe {
            TASKS[idx].pid = parse_u64(pid_slice);
            TASKS[idx].name_len = name_slice.len().min(32);
            TASKS[idx].name[..TASKS[idx].name_len].copy_from_slice(&name_slice[..TASKS[idx].name_len]);
            TASKS[idx].state_len = state_slice.len().min(8);
            TASKS[idx].state[..TASKS[idx].state_len].copy_from_slice(&state_slice[..TASKS[idx].state_len]);
            TASKS[idx].priority_len = priority_slice.len().min(8);
            TASKS[idx].priority[..TASKS[idx].priority_len].copy_from_slice(&priority_slice[..TASKS[idx].priority_len]);
            TASKS[idx].cpu = parse_i64(cpu_slice);
            TASKS[idx].uptime = parse_u64(uptime_slice);
        }
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

    fn parse_usize(bytes: &[u8]) -> usize {
        parse_u64(bytes) as usize
    }

    fn print_uptime(total_sec: i64) {
        let hours = total_sec / 3600;
        let mins = (total_sec % 3600) / 60;
        let secs = total_sec % 60;

        if hours > 0 {
            print_int(hours);
            console_log("h ");
        }
        if hours > 0 || mins > 0 {
            print_int(mins);
            console_log("m ");
        }
        print_int(secs);
        console_log("s");
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
        pad_spaces(width.saturating_sub(digits));
        print_int(n);
    }

    fn pad_spaces(count: usize) {
        for _ in 0..count {
            console_log(" ");
        }
    }

    fn print_bytes(bytes: &[u8]) {
        unsafe { mkfs::print(bytes.as_ptr(), bytes.len()) };
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}

