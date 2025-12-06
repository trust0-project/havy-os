// cat - Display file contents
//
// Usage:
//   cat <file>       Display contents of a file
//   cat -n <file>    Display with line numbers

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{console_log, argc, argv, get_cwd, print_int};
    use mkfs::syscalls::{print, fs_read};

    fn resolve_path(arg: &[u8], out: &mut [u8]) -> usize {
        let mut cwd = [0u8; 256];
        let cwd_len = get_cwd(&mut cwd);

        if arg.starts_with(b"/") {
            // Absolute path
            let len = arg.len().min(out.len());
            out[..len].copy_from_slice(&arg[..len]);
            len
        } else if let Some(cwd_len) = cwd_len {
            // Relative path
            let copy_len = cwd_len.min(out.len());
            out[..copy_len].copy_from_slice(&cwd[..copy_len]);
            let mut pos = copy_len;

            // Add separator if needed
            if pos < out.len() && pos > 0 && out[pos - 1] != b'/' {
                out[pos] = b'/';
                pos += 1;
            }

            // Copy filename
            let remaining = out.len() - pos;
            let copy_len = arg.len().min(remaining);
            out[pos..pos + copy_len].copy_from_slice(&arg[..copy_len]);
            pos + copy_len
        } else {
            // Fallback: treat as absolute from root
            if out.len() > 0 {
                out[0] = b'/';
            }
            let copy_len = arg.len().min(out.len() - 1);
            out[1..1 + copy_len].copy_from_slice(&arg[..copy_len]);
            1 + copy_len
        }
    }

    #[no_mangle]
    pub extern "C" fn _start() {
        let arg_count = argc();

        if arg_count < 1 {
            console_log("Usage: cat <filename>\n");
            return;
        }

        let mut show_line_numbers = false;
        let mut file_arg_idx: Option<usize> = None;

        // Parse arguments
        for i in 0..arg_count {
            let mut arg_buf = [0u8; 256];
            if let Some(len) = argv(i, &mut arg_buf) {
                let arg = &arg_buf[..len];

                if arg == b"-n" {
                    show_line_numbers = true;
                } else if !arg.starts_with(b"-") {
                    file_arg_idx = Some(i);
                }
            }
        }

        let file_idx = match file_arg_idx {
            Some(idx) => idx,
            None => {
                console_log("Usage: cat <filename>\n");
                return;
            }
        };

        // Get filename
        let mut filename_buf = [0u8; 256];
        let filename_len = match argv(file_idx, &mut filename_buf) {
            Some(len) => len,
            None => {
                console_log("\x1b[1;31mError:\x1b[0m Invalid filename\n");
                return;
            }
        };

        // Resolve path
        let mut path_buf = [0u8; 512];
        let path_len = resolve_path(&filename_buf[..filename_len], &mut path_buf);

        // Read file
        let mut content = [0u8; 1048576]; // 1MB max file size
        let read_len = unsafe {
            fs_read(path_buf.as_ptr(), path_len as i32, content.as_mut_ptr(), content.len() as i32)
        };

        if read_len < 0 {
            console_log("\x1b[1;31mError:\x1b[0m File not found: ");
            unsafe { print(path_buf.as_ptr(), path_len) };
            console_log("\n");
            return;
        }

        let content = &content[..read_len as usize];

        if show_line_numbers {
            let mut line_num = 1usize;
            let mut line_start = 0;

            for (i, &c) in content.iter().enumerate() {
                if c == b'\n' || i == content.len() - 1 {
                    let end = if c == b'\n' { i } else { i + 1 };

                    // Print line number
                    console_log("\x1b[0;90m");
                    // Right-align line number in 4 chars
                    if line_num < 10 {
                        console_log("   ");
                    } else if line_num < 100 {
                        console_log("  ");
                    } else if line_num < 1000 {
                        console_log(" ");
                    }
                    print_int(line_num as i64);
                    console_log("\x1b[0m | ");

                    // Print line content
                    unsafe { print(content[line_start..end].as_ptr(), end - line_start) };
                    console_log("\n");

                    line_num += 1;
                    line_start = i + 1;
                }
            }
        } else {
            // Print content directly
            unsafe { print(content.as_ptr(), content.len()) };

            // Add newline if file doesn't end with one
            if !content.is_empty() && content[content.len() - 1] != b'\n' {
                console_log("\n");
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
