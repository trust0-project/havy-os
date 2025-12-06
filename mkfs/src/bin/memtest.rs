// memtest - Memory allocation test
//
// Usage:
//   memtest              Run with 10 iterations
//   memtest <N>          Run with N iterations

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{console_log, get_heap_stats, print_int, argc, argv};

    // Static buffer for argument
    static mut ARG_BUF: [u8; 32] = [0u8; 32];
    
    // Static buffer for test data (1KB)
    static mut TEST_BUF: [u8; 1024] = [0u8; 1024];

    #[no_mangle]
    pub extern "C" fn _start() {
        // Parse iterations from arguments (arg 0 since command name is not passed)
        let iterations: usize = if argc() > 0 {
            let len = unsafe { argv(0, &mut ARG_BUF) };
            if let Some(len) = len {
                let arg = unsafe { &ARG_BUF[..len] };
                let n = parse_usize(arg);
                if n > 0 { n } else { 10 }
            } else {
                10
            }
        } else {
            10
        };

        console_log("Running ");
        print_int(iterations as i64);
        console_log(" memory test iterations...\n");

        // Get initial heap stats
        let (used_before, total) = if let Some(stats) = get_heap_stats() {
            (stats.used_bytes, stats.total_bytes)
        } else {
            console_log("\x1b[1;31mError:\x1b[0m Could not get heap stats\n");
            return;
        };
        
        let free_before = total.saturating_sub(used_before);

        console_log("  Before: used=");
        print_int(used_before as i64);
        console_log(" free=");
        print_int(free_before as i64);
        console_log("\n");

        let mut success_count = 0usize;
        let mut fail_count = 0usize;

        for i in 0..iterations {
            let pattern = ((i % 256) as u8).wrapping_add(0x42);

            // Fill buffer with pattern
            unsafe {
                for byte in TEST_BUF.iter_mut() {
                    *byte = pattern;
                }
            }

            // Verify pattern
            let mut ok = true;
            unsafe {
                for &byte in TEST_BUF.iter() {
                    if byte != pattern {
                        ok = false;
                        break;
                    }
                }
            }

            if ok {
                success_count += 1;
            } else {
                fail_count += 1;
            }
        }

        // Get final heap stats
        let (used_after, _) = if let Some(stats) = get_heap_stats() {
            (stats.used_bytes, stats.total_bytes)
        } else {
            (0, 0)
        };
        
        let free_after = total.saturating_sub(used_after);

        console_log("  After:  used=");
        print_int(used_after as i64);
        console_log(" free=");
        print_int(free_after as i64);
        console_log("\n");

        console_log("Results: ");
        print_int(success_count as i64);
        console_log(" passed, ");
        print_int(fail_count as i64);
        console_log(" failed.\n");

        // Note: In WASM mode, we use static buffers so no heap allocation occurs
        console_log("\n");
        console_log("\x1b[0;90mNote: WASM memtest uses static buffers (no dynamic allocation)\x1b[0m\n");
        console_log("\x1b[0;90m      For full heap testing, use kernel native memtest command\x1b[0m\n");
    }

    fn parse_usize(bytes: &[u8]) -> usize {
        let mut n: usize = 0;
        for &b in bytes {
            if b >= b'0' && b <= b'9' {
                n = n.saturating_mul(10).saturating_add((b - b'0') as usize);
            }
        }
        n
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}

