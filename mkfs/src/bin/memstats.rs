// memstats - Show memory statistics
//
// Usage:
//   memstats      Display heap memory usage

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{console_log, get_heap_stats, print_int};

    let stats = get_heap_stats();
    
    let used_kb = stats.used_bytes / 1024;
    let total_kb = stats.total_bytes / 1024;
    let free_kb = total_kb.saturating_sub(used_kb);
    let percent = if stats.total_bytes > 0 {
        (stats.used_bytes * 100 / stats.total_bytes) as i64
    } else {
        0
    };

    console_log("\n");
    console_log("\x1b[1;36m+-------------------------------------------------------------+\x1b[0m\n");
    console_log("\x1b[1;36m|\x1b[0m              \x1b[1;97mHeap Memory Statistics\x1b[0m                         \x1b[1;36m|\x1b[0m\n");
    console_log("\x1b[1;36m+-------------------------------------------------------------+\x1b[0m\n");
    console_log("\x1b[1;36m|\x1b[0m                                                             \x1b[1;36m|\x1b[0m\n");

    // Used
    console_log("\x1b[1;36m|\x1b[0m  Used:       \x1b[1;33m");
    print_int(used_kb as i64);
    console_log(" KB\x1b[0m");
    pad_for_value(used_kb as usize);
    console_log("\x1b[1;36m|\x1b[0m\n");

    // Free
    console_log("\x1b[1;36m|\x1b[0m  Free:       \x1b[1;32m");
    print_int(free_kb as i64);
    console_log(" KB\x1b[0m");
    pad_for_value(free_kb as usize);
    console_log("\x1b[1;36m|\x1b[0m\n");

    // Total
    console_log("\x1b[1;36m|\x1b[0m  Total:      \x1b[1;97m");
    print_int(total_kb as i64);
    console_log(" KB\x1b[0m");
    pad_for_value(total_kb as usize);
    console_log("\x1b[1;36m|\x1b[0m\n");

    console_log("\x1b[1;36m|\x1b[0m                                                             \x1b[1;36m|\x1b[0m\n");

    // Usage bar
    console_log("\x1b[1;36m|\x1b[0m  Usage:      ");
    
    // Draw usage bar (30 chars)
    let filled = (percent * 30 / 100) as usize;
    console_log("\x1b[42m"); // Green background
    for _ in 0..filled {
        console_log(" ");
    }
    console_log("\x1b[0m\x1b[100m"); // Gray background
    for _ in filled..30 {
        console_log(" ");
    }
    console_log("\x1b[0m ");
    print_int(percent);
    console_log("%     \x1b[1;36m|\x1b[0m\n");

    console_log("\x1b[1;36m|\x1b[0m                                                             \x1b[1;36m|\x1b[0m\n");
    console_log("\x1b[1;36m+-------------------------------------------------------------+\x1b[0m\n");
    console_log("\n");

    fn pad_for_value(val: usize) {
        let digits = digit_count(val);
        let padding = 40 - digits - 3; // " KB" is 3 chars
        for _ in 0..padding {
            mkfs::console_log(" ");
        }
    }

    fn digit_count(mut n: usize) -> usize {
        if n == 0 { return 1; }
        let mut count = 0;
        while n > 0 {
            count += 1;
            n /= 10;
        }
        count
    }
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
