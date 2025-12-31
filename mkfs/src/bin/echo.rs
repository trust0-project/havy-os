// echo - Print arguments to stdout
//
// Usage:
//   echo <text>       Print text followed by newline
//   echo -n <text>    Print text without newline

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{console_log, argc, argv, print};

    let arg_count = argc();

    let mut no_newline = false;
    let mut start_idx = 0;

    // Check for -n flag
    if arg_count > 0 {
        let mut arg_buf = [0u8; 16];
        if let Some(len) = argv(0, &mut arg_buf) {
            if len == 2 && arg_buf[0] == b'-' && arg_buf[1] == b'n' {
                no_newline = true;
                start_idx = 1;
            }
        }
    }

    // Print all arguments
    let mut first = true;
    for i in start_idx..arg_count {
        let mut arg_buf = [0u8; 1024];
        if let Some(len) = argv(i, &mut arg_buf) {
            if !first {
                console_log(" ");
            }
            first = false;
            // Print raw bytes
            print(arg_buf.as_ptr(), len);
        }
    }

    if !no_newline {
        console_log("\n");
    }
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
