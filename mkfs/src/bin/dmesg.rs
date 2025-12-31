// dmesg - Display kernel ring buffer
//
// Usage:
//   dmesg           Show all kernel log messages
//   dmesg -n <N>    Show last N messages
//   dmesg -h        Show help

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{console_log, argc, argv, get_klog, print};

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

    let arg_count = argc();
    let mut count: usize = 100;

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

        if arg == b"-h" || arg == b"--help" {
            console_log("\x1b[1mdmesg\x1b[0m - Display kernel ring buffer\n\n");
            console_log("Usage: dmesg [-n <N>]\n\n");
            console_log("Options:\n");
            console_log("  -n <N>  Show last N messages (default: 100)\n");
            return;
        }

        if arg == b"-n" {
            if i + 1 < arg_count {
                let mut num_buf = [0u8; 16];
                if let Some(num_len) = argv(i + 1, &mut num_buf) {
                    if let Some(n) = parse_int(&num_buf[..num_len]) {
                        count = n.max(1).min(100);
                        i += 1;
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

    let mut buf = [0u8; 4096];  // 4KB - must fit in 8KB stack
    match get_klog(count, &mut buf) {
        Some(0) => {
            console_log("\x1b[90m(No kernel log entries)\x1b[0m\n");
        }
        Some(len) => {
            print(buf.as_ptr(), len);
        }
        None => {
            console_log("\x1b[31mError: Failed to read kernel log\x1b[0m\n");
        }
    }
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
