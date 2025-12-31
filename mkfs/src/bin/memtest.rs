// memtest - Memory test
//
// Usage:
//   memtest              Run with 10 iterations
//   memtest <N>          Run with N iterations

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{console_log, print_int, argc, argv};

    static mut TEST_BUF: [u8; 1024] = [0u8; 1024];

    fn parse_usize(bytes: &[u8]) -> usize {
        let mut n: usize = 0;
        for &b in bytes {
            if b >= b'0' && b <= b'9' {
                n = n.saturating_mul(10).saturating_add((b - b'0') as usize);
            }
        }
        n
    }

    let iterations: usize = if argc() > 0 {
        let mut arg_buf = [0u8; 32];
        if let Some(len) = argv(0, &mut arg_buf) {
            let n = parse_usize(&arg_buf[..len]);
            if n > 0 { n } else { 10 }
        } else { 10 }
    } else { 10 };

    console_log("Running ");
    print_int(iterations as i64);
    console_log(" memory test iterations...\n");

    let mut success_count = 0usize;
    let mut fail_count = 0usize;

    for i in 0..iterations {
        let pattern = ((i % 256) as u8).wrapping_add(0x42);

        // Fill buffer with pattern
        unsafe {
            for byte in (*core::ptr::addr_of_mut!(TEST_BUF)).iter_mut() {
                *byte = pattern;
            }
        }

        // Verify pattern
        let mut ok = true;
        unsafe {
            for &byte in (*core::ptr::addr_of!(TEST_BUF)).iter() {
                if byte != pattern {
                    ok = false;
                    break;
                }
            }
        }

        if ok { success_count += 1; } else { fail_count += 1; }
    }

    console_log("Results: ");
    print_int(success_count as i64);
    console_log(" passed, ");
    print_int(fail_count as i64);
    console_log(" failed.\n");

    console_log("\n\x1b[90mNote: Native memtest uses static buffers (no dynamic allocation)\x1b[0m\n");
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
