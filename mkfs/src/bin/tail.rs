// tail - Show last lines of a file
//
// Usage:
//   tail <file>           Show last 10 lines
//   tail -n <N> <file>    Show last N lines
//   tail -<N> <file>      Show last N lines (shorthand)

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{console_log, argc, argv, get_cwd};
    use mkfs::syscalls::{print, fs_read};

    fn parse_num(s: &[u8]) -> Option<usize> {
        if s.is_empty() {
            return None;
        }
        let mut result = 0usize;
        for &c in s {
            if c < b'0' || c > b'9' {
                return None;
            }
            result = result.checked_mul(10)?.checked_add((c - b'0') as usize)?;
        }
        Some(result)
    }

    fn resolve_path(arg: &[u8], out: &mut [u8]) -> usize {
        let mut cwd = [0u8; 256];
        let cwd_len = get_cwd(&mut cwd);

        if arg.starts_with(b"/") {
            let len = arg.len().min(out.len());
            out[..len].copy_from_slice(&arg[..len]);
            len
        } else if let Some(cwd_len) = cwd_len {
            let copy_len = cwd_len.min(out.len());
            out[..copy_len].copy_from_slice(&cwd[..copy_len]);
            let mut pos = copy_len;

            if pos < out.len() && pos > 0 && out[pos - 1] != b'/' {
                out[pos] = b'/';
                pos += 1;
            }

            let remaining = out.len() - pos;
            let copy_len = arg.len().min(remaining);
            out[pos..pos + copy_len].copy_from_slice(&arg[..copy_len]);
            pos + copy_len
        } else {
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
            console_log("Usage: tail [-n NUM] <file...>\n");
            return;
        }

        let mut num_lines = 10usize;
        let mut files: [(usize, usize); 16] = [(0, 0); 16];
        let mut file_count = 0usize;
        let mut args_storage = [0u8; 4096];
        let mut storage_pos = 0usize;

        // Parse arguments
        let mut i = 0usize;
        while i < arg_count {
            let mut arg_buf = [0u8; 256];
            let arg_len = match argv(i, &mut arg_buf) {
                Some(len) => len,
                None => {
                    i += 1;
                    continue;
                }
            };
            let arg = &arg_buf[..arg_len];

            if arg == b"-n" {
                // Next argument is the number
                i += 1;
                if i < arg_count {
                    let mut num_buf = [0u8; 16];
                    if let Some(num_len) = argv(i, &mut num_buf) {
                        if let Some(n) = parse_num(&num_buf[..num_len]) {
                            num_lines = n.max(1);
                        }
                    }
                }
            } else if arg.starts_with(b"-n") && arg.len() > 2 {
                // -nNUM format
                if let Some(n) = parse_num(&arg[2..]) {
                    num_lines = n.max(1);
                }
            } else if arg.starts_with(b"-") && arg.len() > 1 && arg[1] >= b'0' && arg[1] <= b'9' {
                // -NUM format
                if let Some(n) = parse_num(&arg[1..]) {
                    num_lines = n.max(1);
                }
            } else if !arg.starts_with(b"-") && file_count < 16 {
                // File argument
                let remaining = args_storage.len() - storage_pos;
                let copy_len = arg.len().min(remaining);
                if copy_len > 0 {
                    args_storage[storage_pos..storage_pos + copy_len].copy_from_slice(&arg[..copy_len]);
                    files[file_count] = (storage_pos, copy_len);
                    storage_pos += copy_len;
                    file_count += 1;
                }
            }

            i += 1;
        }

        if file_count == 0 {
            console_log("Usage: tail [-n NUM] <file...>\n");
            return;
        }

        let show_headers = file_count > 1;

        // Process each file
        for f in 0..file_count {
            let (start, len) = files[f];
            let file_arg = &args_storage[start..start + len];

            // Resolve path
            let mut path_buf = [0u8; 512];
            let path_len = resolve_path(file_arg, &mut path_buf);

            // Read file
            let mut content = [0u8; 1048576]; // 1MB max file size
            let read_len = unsafe {
                fs_read(path_buf.as_ptr(), path_len as i32, content.as_mut_ptr(), content.len() as i32)
            };

            if read_len < 0 {
                console_log("\x1b[1;31mtail:\x1b[0m cannot open '");
                unsafe { print(path_buf.as_ptr(), path_len) };
                console_log("': No such file\n");
                continue;
            }

            if show_headers {
                if f > 0 {
                    console_log("\n");
                }
                console_log("\x1b[1m==> ");
                unsafe { print(path_buf.as_ptr(), path_len) };
                console_log(" <==\x1b[0m\n");
            }

            let content = &content[..read_len as usize];

            // Count lines and find positions
            let mut line_positions: [usize; 1024] = [0; 1024];
            let mut line_count = 0usize;
            line_positions[0] = 0;

            for (idx, &c) in content.iter().enumerate() {
                if c == b'\n' && idx + 1 < content.len() && line_count + 1 < 1024 {
                    line_count += 1;
                    line_positions[line_count] = idx + 1;
                }
            }
            line_count += 1; // Total number of lines

            // Calculate start line
            let start_line = if line_count > num_lines {
                line_count - num_lines
            } else {
                0
            };

            // Print lines from start_line onwards
            for line_idx in start_line..line_count {
                let line_start = line_positions[line_idx];
                let line_end = if line_idx + 1 < line_count {
                    line_positions[line_idx + 1] - 1 // Exclude newline
                } else {
                    content.len()
                };

                if line_start < content.len() {
                    let end = line_end.min(content.len());
                    unsafe { print(content[line_start..end].as_ptr(), end - line_start) };
                    console_log("\n");
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
