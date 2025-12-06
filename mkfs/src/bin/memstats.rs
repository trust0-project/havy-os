// memstats - Show memory statistics
//
// Usage:
//   memstats      Display heap memory usage with visual progress bar

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{console_log, get_heap_stats, print_int};

    #[no_mangle]
    pub extern "C" fn _start() {
        let Some(stats) = get_heap_stats() else {
            console_log("\x1b[1;31mError:\x1b[0m Could not retrieve memory statistics\n");
            return;
        };

        let total = stats.total_bytes;
        let used = stats.used_bytes;
        let free = total.saturating_sub(used);

        let total_kb = total / 1024;
        let used_kb = used / 1024;
        let free_kb = free / 1024;
        let percent = if total > 0 { (used * 100) / total } else { 0 };

        console_log("\n");
        console_log("\x1b[1;36m┌─────────────────────────────────────────────────────────────┐\x1b[0m\n");
        console_log("\x1b[1;36m│\x1b[0m              \x1b[1;97mHeap Memory Statistics\x1b[0m                         \x1b[1;36m│\x1b[0m\n");
        console_log("\x1b[1;36m├─────────────────────────────────────────────────────────────┤\x1b[0m\n");

        // Total line
        console_log("\x1b[1;36m│\x1b[0m  Total:   \x1b[1;97m");
        print_int(total_kb as i64);
        console_log(" KiB\x1b[0m");
        print_padding(total_kb, 49);
        console_log("\x1b[1;36m│\x1b[0m\n");

        // Used line
        console_log("\x1b[1;36m│\x1b[0m  Used:    \x1b[1;33m");
        print_int(used_kb as i64);
        console_log(" KiB\x1b[0m");
        print_padding(used_kb, 49);
        console_log("\x1b[1;36m│\x1b[0m\n");

        // Free line
        console_log("\x1b[1;36m│\x1b[0m  Free:    \x1b[1;32m");
        print_int(free_kb as i64);
        console_log(" KiB\x1b[0m");
        print_padding(free_kb, 49);
        console_log("\x1b[1;36m│\x1b[0m\n");

        console_log("\x1b[1;36m│\x1b[0m                                                             \x1b[1;36m│\x1b[0m\n");

        // Progress bar
        console_log("\x1b[1;36m│\x1b[0m  Usage:   [");
        
        let bar_width: u64 = 30;
        let filled = (percent * bar_width) / 100;
        
        for i in 0..bar_width {
            if i < filled {
                console_log("\x1b[1;32m█\x1b[0m");
            } else {
                console_log("\x1b[0;90m░\x1b[0m");
            }
        }
        
        console_log("] ");
        print_int(percent as i64);
        console_log("%");
        
        // Padding for percentage display
        print_percent_padding(percent);
        console_log("\x1b[1;36m│\x1b[0m\n");

        console_log("\x1b[1;36m└───────────────────────────────────────────────────────────┘\x1b[0m\n");
        console_log("\n");
    }

    /// Calculate number of digits in a number
    fn digit_count(mut n: u64) -> usize {
        if n == 0 {
            return 1;
        }
        let mut count = 0;
        while n > 0 {
            count += 1;
            n /= 10;
        }
        count
    }

    /// Print padding based on the value's width
    fn print_padding(value_kb: u64, target_width: usize) {
        // " KiB" is 4 chars, plus the number digits
        let value_len = digit_count(value_kb) + 4;
        let pad = if target_width > value_len { target_width - value_len } else { 0 };
        for _ in 0..pad {
            console_log(" ");
        }
    }

    /// Print padding for percentage display
    fn print_percent_padding(percent: u64) {
        // "X%" is 2 chars for single digit, "XX%" is 3 chars, "100%" is 4 chars
        let pct_len = digit_count(percent) + 1; // +1 for '%'
        let pad = if 14 > pct_len { 14 - pct_len } else { 0 };
        for _ in 0..pad {
            console_log(" ");
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}

