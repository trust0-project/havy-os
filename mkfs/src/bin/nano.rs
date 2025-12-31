// nano - Text file viewer
//
// Usage:
//   nano <filename>     View file contents with line numbers
//   nano -h             Show help

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{console_log, argc, argv, get_cwd, file_exists, print_int, print, fs_read};

    fn print_help() {
        console_log("\x1b[1mnano\x1b[0m - Text file viewer (BAVY Edition)\n\n");
        console_log("\x1b[1mUSAGE:\x1b[0m\n");
        console_log("    nano <filename>\n\n");
        console_log("\x1b[1mOPTIONS:\x1b[0m\n");
        console_log("    -h, --help  Show this help message\n\n");
        console_log("\x1b[90mNote: This is a read-only viewer.\x1b[0m\n");
    }

    fn print_num_padded(n: i32, width: usize) {
        let mut digits = 0;
        let mut tmp = n;
        if tmp == 0 {
            digits = 1;
        } else {
            while tmp > 0 {
                digits += 1;
                tmp /= 10;
            }
        }
        for _ in digits..width {
            console_log(" ");
        }
        print_int(n as i64);
    }

    let arg_count = argc();

    if arg_count < 1 {
        print_help();
        return;
    }

    let mut arg_buf = [0u8; 256];
    let arg_len = match argv(0, &mut arg_buf) {
        Some(len) => len,
        None => {
            print_help();
            return;
        }
    };

    let arg = &arg_buf[..arg_len];

    if arg == b"-h" || arg == b"--help" {
        print_help();
        return;
    }

    // Build absolute path if needed
    let mut path_buf = [0u8; 512];
    let path_len: usize;

    if arg[0] == b'/' {
        path_buf[..arg_len].copy_from_slice(arg);
        path_len = arg_len;
    } else {
        let mut cwd_buf = [0u8; 256];
        if let Some(cwd_len) = get_cwd(&mut cwd_buf) {
            path_buf[..cwd_len].copy_from_slice(&cwd_buf[..cwd_len]);
            let mut idx = cwd_len;
            if idx > 0 && path_buf[idx - 1] != b'/' {
                path_buf[idx] = b'/';
                idx += 1;
            }
            path_buf[idx..idx + arg_len].copy_from_slice(arg);
            path_len = idx + arg_len;
        } else {
            path_buf[0] = b'/';
            path_buf[1..1 + arg_len].copy_from_slice(arg);
            path_len = 1 + arg_len;
        }
    }

    // Check if file exists
    let path_str = unsafe { core::str::from_utf8_unchecked(&path_buf[..path_len]) };
    if !file_exists(path_str) {
        console_log("\x1b[31mError: File not found: \x1b[0m");
        print(path_buf.as_ptr(), path_len);
        console_log("\n");
        return;
    }

    // Read file contents
    static mut CONTENT_BUF: [u8; 65536] = [0u8; 65536]; // 64KB max
    let content_len = unsafe {
        fs_read(path_buf.as_ptr(), path_len as i32, (*core::ptr::addr_of_mut!(CONTENT_BUF)).as_mut_ptr(), 65536)
    };

    if content_len < 0 {
        console_log("\x1b[31mError: Failed to read file\x1b[0m\n");
        return;
    }

    // Print header
    console_log("\x1b[7m  File: ");
    print(path_buf.as_ptr(), path_len);
    console_log(" \x1b[0m\n");
    console_log("\x1b[90m────────────────────────────────────────────────────────────\x1b[0m\n");

    if content_len == 0 {
        console_log("\x1b[90m(empty file)\x1b[0m\n");
        return;
    }

    let content = unsafe { &(*core::ptr::addr_of!(CONTENT_BUF))[..content_len as usize] };
    let mut line_count = 1;
    for &c in content {
        if c == b'\n' {
            line_count += 1;
        }
    }

    let num_width = if line_count >= 1000 { 4 } else if line_count >= 100 { 3 } else { 2 };

    let mut line_num = 1;
    let mut line_start = 0;

    for i in 0..content.len() {
        if content[i] == b'\n' || i == content.len() - 1 {
            let line_end = if content[i] == b'\n' { i } else { i + 1 };

            console_log("\x1b[90m");
            print_num_padded(line_num, num_width);
            console_log(" |\x1b[0m ");

            if line_end > line_start {
                print(content.as_ptr().wrapping_add(line_start), line_end - line_start);
            }
            console_log("\n");

            line_num += 1;
            line_start = i + 1;
        }
    }

    console_log("\x1b[90m────────────────────────────────────────────────────────────\x1b[0m\n");
    console_log("\x1b[90m");
    print_int(content_len as i64);
    console_log(" bytes, ");
    print_int(line_count as i64);
    console_log(" lines\x1b[0m\n");
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
