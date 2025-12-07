// cputest - CPU benchmark (prime counting) with multi-hart support
//
// Usage:
//   cputest              Run with default limit (100000)
//   cputest <limit>      Count primes up to <limit>
//
// Compares serial vs parallel prime counting across available harts.

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{
        console_log, get_time, argc, argv,
        get_worker_count, get_hart_count, submit_wasm_job, get_job_status, JobStatus,
        set_parallel_result, sum_parallel_results, clear_parallel_results,
        sleep, read_file,
    };

    static mut ARG_BUF: [u8; 32] = [0u8; 32];
    static mut NUM_BUF: [u8; 20] = [0u8; 20];
    static mut ARGS_BUF: [u8; 64] = [0u8; 64];
    // Use regular array - WASM memory is zero-initialized anyway
    static mut WASM_BUF: [u8; 16384] = [0u8; 16384]; // 16KB for WASM binary

    #[no_mangle]
    pub extern "C" fn _start() {
        // Check if we're a worker
        if argc() >= 4 {
            let len = unsafe { argv(0, &mut ARG_BUF) };
            if let Some(len) = len {
                if len == 6 && is_worker_arg(unsafe { &ARG_BUF[..6] }) {
                    run_as_worker();
                    return;
                }
            }
        }
        run_as_primary();
    }

    #[inline(never)]
    fn is_worker_arg(arg: &[u8]) -> bool {
        arg[0] == b'w' && arg[1] == b'o' && arg[2] == b'r' && 
        arg[3] == b'k' && arg[4] == b'e' && arg[5] == b'r'
    }

    fn run_as_worker() {
        let slot = parse_arg(1);
        let start = parse_arg(2);
        let end = parse_arg(3);
        let count = count_primes_range(start, end);
        set_parallel_result(slot as usize, count as u64);
    }

    fn run_as_primary() {
        let limit = if argc() > 0 {
            let len = unsafe { argv(0, &mut ARG_BUF) };
            if let Some(len) = len {
                let n = parse_u32(unsafe { &ARG_BUF[..len] });
                if n > 0 { n } else { 100_000 }
            } else { 100_000 }
        } else { 100_000 };

        let num_harts = get_hart_count() as u32;
        let num_workers = get_worker_count() as u32;

        console_log("\n");
        console_log("\x1b[1;36m╔═══════════════════════════════════════════════════════════════════════╗\x1b[0m\n");
        console_log("\x1b[1;36m║\x1b[0m                      \x1b[1;97mCPU BENCHMARK - Prime Counting\x1b[0m                  \x1b[1;36m║\x1b[0m\n");
        console_log("\x1b[1;36m╚═══════════════════════════════════════════════════════════════════════╝\x1b[0m\n\n");

        console_log("  \x1b[1;33mConfiguration:\x1b[0m\n");
        console_log("    Range: 2 to ");
        print_u32(limit);
        console_log("\n    Harts online: ");
        print_u32(num_harts);
        console_log(" (");
        print_u32(num_workers);
        console_log(" workers)\n\n");

        // Serial execution
        console_log("  \x1b[1;33m[1/2] Serial Execution\x1b[0m (single hart)\n");
        console_log("        Computing primes...");

        let serial_start = get_time() as u32;
        let serial_count = count_primes_range(2, limit);
        let serial_time = (get_time() as u32).wrapping_sub(serial_start);

        console_log(" done!\n        Result: \x1b[1;97m");
        print_u32(serial_count);
        console_log("\x1b[0m primes in \x1b[1;97m");
        print_u32(serial_time);
        console_log("\x1b[0m ms\n\n");

        // Parallel execution
        if num_workers > 0 {
            console_log("  \x1b[1;33m[2/2] Parallel Execution\x1b[0m (");
            print_u32(num_workers + 1);
            console_log(" harts)\n        Distributing work...\n");

            let wasm_len = match read_file("/usr/bin/cputest", unsafe { &mut WASM_BUF }) {
                Some(len) => len,
                None => {
                    console_log("        \x1b[1;31mError:\x1b[0m Could not read WASM\n");
                    return;
                }
            };
            let wasm_bytes = unsafe { &WASM_BUF[..wasm_len] };

            clear_parallel_results();
            let total = num_workers + 1;
            let parallel_start = get_time() as u32;

            // Submit to workers
            let mut job_ids = [0u32; 16];
            let mut submitted = 0u32;

            for i in 0..num_workers {
                let slot = i + 1;
                let (ws, we) = work_range(2, limit, slot, total);
                let args = build_args(slot, ws, we);
                if let Some(id) = submit_wasm_job(wasm_bytes, args, None) {
                    job_ids[submitted as usize] = id;
                    submitted += 1;
                }
            }

            // Primary does slot 0
            let (my_start, my_end) = work_range(2, limit, 0, total);
            let my_count = count_primes_range(my_start, my_end);
            set_parallel_result(0, my_count as u64);

            // Wait for workers
            console_log("        Waiting for workers...");
            let timeout = (get_time() as u32) + 60000;
            loop {
                let mut done = true;
                for i in 0..submitted {
                    if let Some(s) = get_job_status(job_ids[i as usize]) {
                        if s != JobStatus::Completed && s != JobStatus::Failed {
                            done = false;
                            break;
                        }
                    }
                }
                if done { break; }
                if (get_time() as u32) > timeout {
                    console_log(" TIMEOUT!\n");
                    return;
                }
                sleep(10);
            }

            let parallel_time = (get_time() as u32).wrapping_sub(parallel_start);
            let parallel_count = sum_parallel_results(0, total as usize) as u32;

            console_log(" done!\n        Result: \x1b[1;97m");
            print_u32(parallel_count);
            console_log("\x1b[0m primes in \x1b[1;97m");
            print_u32(parallel_time);
            console_log("\x1b[0m ms\n\n");

            // Summary
            console_log("\x1b[1;36m────────────────────────────────────────────────────────────────────────\x1b[0m\n");
            console_log("  \x1b[1;33mResults:\x1b[0m\n\n");

            if serial_count == parallel_count {
                console_log("    \x1b[1;32m[OK]\x1b[0m Results match\n\n");
            } else {
                console_log("    \x1b[1;31m[X]\x1b[0m MISMATCH! Serial=");
                print_u32(serial_count);
                console_log(" Parallel=");
                print_u32(parallel_count);
                console_log("\n\n");
            }

            console_log("    Serial:   ");
            print_u32(serial_time);
            console_log(" ms\n    Parallel: ");
            print_u32(parallel_time);
            console_log(" ms\n");

            if parallel_time > 0 {
                let speedup_x10 = (serial_time * 10) / parallel_time;
                console_log("    Speedup:  \x1b[1;32m");
                print_u32(speedup_x10 / 10);
                console_log(".");
                print_u32(speedup_x10 % 10);
                console_log("x\x1b[0m\n");
            }
        } else {
            console_log("  \x1b[1;33m[2/2] Parallel\x1b[0m - \x1b[90mSkipped (no workers)\x1b[0m\n");
        }

        console_log("\n\x1b[1;36m════════════════════════════════════════════════════════════════════════\x1b[0m\n\n");
    }

    fn work_range(start: u32, end: u32, slot: u32, total: u32) -> (u32, u32) {
        let range = end - start;
        let per_worker = range / total;
        let remainder = range % total;
        let ws = start + slot * per_worker + if slot < remainder { slot } else { remainder };
        let we = ws + per_worker + if slot < remainder { 1 } else { 0 };
        (ws, we)
    }

    fn build_args(slot: u32, start: u32, end: u32) -> &'static str {
        unsafe {
            let mut pos = 0;
            // "worker\n"
            for &b in b"worker\n" { ARGS_BUF[pos] = b; pos += 1; }
            pos += write_u32(&mut ARGS_BUF[pos..], slot);
            ARGS_BUF[pos] = b'\n'; pos += 1;
            pos += write_u32(&mut ARGS_BUF[pos..], start);
            ARGS_BUF[pos] = b'\n'; pos += 1;
            pos += write_u32(&mut ARGS_BUF[pos..], end);
            core::str::from_utf8_unchecked(&ARGS_BUF[..pos])
        }
    }

    fn count_primes_range(start: u32, end: u32) -> u32 {
        let mut count = 0u32;
        let mut n = start;
        while n < end {
            if is_prime(n) { count += 1; }
            n += 1;
        }
        count
    }

    fn is_prime(n: u32) -> bool {
        if n < 2 { return false; }
        if n == 2 { return true; }
        if n % 2 == 0 { return false; }
        let mut i = 3u32;
        while i * i <= n {
            if n % i == 0 { return false; }
            i += 2;
        }
        true
    }

    fn print_u32(n: u32) {
        console_log(u32_to_str(n));
    }

    fn u32_to_str(mut n: u32) -> &'static str {
        unsafe {
            let mut i = NUM_BUF.len();
            if n == 0 {
                i -= 1;
                NUM_BUF[i] = b'0';
            } else {
                while n > 0 && i > 0 {
                    i -= 1;
                    NUM_BUF[i] = b'0' + (n % 10) as u8;
                    n /= 10;
                }
            }
            core::str::from_utf8_unchecked(&NUM_BUF[i..])
        }
    }

    fn write_u32(buf: &mut [u8], mut n: u32) -> usize {
        if n == 0 { buf[0] = b'0'; return 1; }
        let mut temp = [0u8; 10];
        let mut i = 0;
        while n > 0 {
            temp[i] = b'0' + (n % 10) as u8;
            n /= 10;
            i += 1;
        }
        for j in 0..i { buf[j] = temp[i - 1 - j]; }
        i
    }

    fn parse_arg(idx: usize) -> u32 {
        static mut BUF: [u8; 16] = [0u8; 16];
        let len = unsafe { argv(idx, &mut BUF) };
        if let Some(len) = len {
            parse_u32(unsafe { &BUF[..len] })
        } else { 0 }
    }

    fn parse_u32(bytes: &[u8]) -> u32 {
        let mut n = 0u32;
        for &b in bytes {
            if b >= b'0' && b <= b'9' {
                n = n.wrapping_mul(10).wrapping_add((b - b'0') as u32);
            }
        }
        n
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
