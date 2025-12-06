// dmesg - Display kernel ring buffer
//
// Usage:
//   dmesg           Show all kernel log messages
//   dmesg -n <N>    Show last N messages
//   dmesg -h        Show help

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{console_log, argc, argv, get_klog};
    use mkfs::syscalls::print;

    fn print_help() {
        console_log("\x1b[1mdmesg\x1b[0m - Display kernel ring buffer\n\n");
        console_log("\x1b[1mUSAGE:\x1b[0m\n");
        console_log("    dmesg [OPTIONS]\n\n");
        console_log("\x1b[1mOPTIONS:\x1b[0m\n");
        console_log("    -n <N>      Show last N messages (default: 100)\n");
        console_log("    -h, --help  Show this help message\n\n");
        console_log("\x1b[1mEXAMPLES:\x1b[0m\n");
        console_log("    dmesg           Show all kernel messages\n");
        console_log("    dmesg -n 10     Show last 10 messages\n");
    }

    fn parse_int(s: &[u8]) -> Option<usize> {
        if s.is_empty() {
            return None;
        }
        let mut result: usize = 0;
        for &c in s {
            if c < b'0' || c > b'9' {
                return None;
            }
            result = result.checked_mul(10)?.checked_add((c - b'0') as usize)?;
        }
        Some(result)
    }

    #[no_mangle]
    pub extern "C" fn _start() {
        let arg_count = argc();

        // Default: show up to 100 messages (kernel limit)
        let mut count: usize = 100;

        // Parse arguments
        let mut i = 0;
        while i < arg_count {
            let mut arg_buf = [0u8; 32];
            let arg_len = match argv(i, &mut arg_buf) {
                Some(len) => len,
                None => {
                    i += 1;
                    continue;
                }
            };

            let arg = &arg_buf[..arg_len];

            // Check for help flag
            if arg == b"-h" || arg == b"--help" {
                print_help();
                return;
            }

            // Check for -n option
            if arg == b"-n" {
                // Next argument should be the count
                if i + 1 < arg_count {
                    let mut num_buf = [0u8; 16];
                    if let Some(num_len) = argv(i + 1, &mut num_buf) {
                        if let Some(n) = parse_int(&num_buf[..num_len]) {
                            count = n.max(1).min(100);
                            i += 1; // Skip the number argument
                        } else {
                            console_log("\x1b[31mError: Invalid count for -n\x1b[0m\n");
                            return;
                        }
                    }
                } else {
                    console_log("\x1b[31mError: -n requires a number argument\x1b[0m\n");
                    return;
                }
            }

            i += 1;
        }

        // Fetch kernel log entries
        let mut buf = [0u8; 40960];
        match get_klog(count, &mut buf) {
            Some(0) => {
                console_log("\x1b[90m(No kernel log entries)\x1b[0m\n");
            }
            Some(len) => {
                unsafe { print(buf.as_ptr(), len) };
            }
            None => {
                console_log("\x1b[31mError: Failed to read kernel log\x1b[0m\n");
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
