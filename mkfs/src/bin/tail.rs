// tail - Show last lines of a file
//
// Usage:
//   tail <file>           Show last 10 lines
//   tail -n <N> <file>    Show last N lines
//   tail -<N> <file>      Show last N lines (shorthand)
//   tail -f <file>        Follow file (print new content, press 'q' or Ctrl+C to exit)

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{console_log, argc, argv, get_cwd};
    use mkfs::syscalls::{print, fs_read, sleep_ms, console_available, console_read, terminal_refresh, should_cancel};

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

    /// Print last N lines of content
    fn print_last_lines(content: &[u8], num_lines: usize) {
        // Count lines and find positions
        let mut line_positions: [usize; 512] = [0; 512];
        let mut line_count = 0usize;
        line_positions[0] = 0;

        for (idx, &c) in content.iter().enumerate() {
            if c == b'\n' && idx + 1 < content.len() && line_count + 1 < 512 {
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

    /// Check if user wants to quit (pressed 'q' or Ctrl+C)
    fn check_quit() -> bool {
        if unsafe { console_available() } == 1 {
            let mut buf = [0u8; 16];
            let n = unsafe { console_read(buf.as_mut_ptr(), buf.len() as i32) };
            if n > 0 {
                for i in 0..n as usize {
                    // 'q' or 'Q' or Ctrl+C (0x03)
                    if buf[i] == b'q' || buf[i] == b'Q' || buf[i] == 0x03 {
                        return true;
                    }
                }
            }
        }
        false
    }

    #[no_mangle]
    pub extern "C" fn _start() {
        let arg_count = argc();

        if arg_count < 1 {
            console_log("Usage: tail [-f] [-n NUM] <file>\n");
            return;
        }

        let mut num_lines = 10usize;
        let mut follow_mode = false;
        let mut file_path: Option<([u8; 512], usize)> = None;

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

            if arg == b"-f" {
                follow_mode = true;
            } else if arg == b"-n" {
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
            } else if !arg.starts_with(b"-") && file_path.is_none() {
                // File argument - resolve path
                let mut path_buf = [0u8; 512];
                let path_len = resolve_path(arg, &mut path_buf);
                file_path = Some((path_buf, path_len));
            }

            i += 1;
        }

        let (path_buf, path_len) = match file_path {
            Some(p) => p,
            None => {
                console_log("Usage: tail [-f] [-n NUM] <file>\n");
                return;
            }
        };

        // Read file initially
        let mut content = [0u8; 32768]; // 32KB buffer
        let read_len = unsafe {
            fs_read(path_buf.as_ptr(), path_len as i32, content.as_mut_ptr(), content.len() as i32)
        };

        if read_len < 0 {
            console_log("\x1b[1;31mtail:\x1b[0m cannot open '");
            unsafe { print(path_buf.as_ptr(), path_len) };
            console_log("': No such file\n");
            return;
        }

        // Print last N lines initially
        print_last_lines(&content[..read_len as usize], num_lines);

        // If follow mode, keep polling for new content
        if follow_mode {
            console_log("\x1b[90m[Following - press 'q' to quit]\x1b[0m\n");
            
            let mut last_len = read_len as usize;
            let mut poll_count = 0u32;
            
            loop {
                // Check for cancellation (Cancel button, Ctrl+C, or 'q' key)
                // This is handled kernel-side for fastest response
                if unsafe { should_cancel() } == 1 {
                    console_log("\n\x1b[90m[Cancelled]\x1b[0m\n");
                    break;
                }
                
                // Re-read file to check for new content
                let new_read_len = unsafe {
                    fs_read(path_buf.as_ptr(), path_len as i32, content.as_mut_ptr(), content.len() as i32)
                };
                
                if new_read_len > 0 {
                    let new_len = new_read_len as usize;
                    
                    // If file has grown, print new content
                    if new_len > last_len {
                        let new_content = &content[last_len..new_len];
                        unsafe { print(new_content.as_ptr(), new_content.len()) };
                        last_len = new_len;
                    }
                }
                
                // Refresh terminal display to show any new output, then sleep
                unsafe { terminal_refresh() };
                unsafe { sleep_ms(100) };  // Fast polling for responsiveness
                
                poll_count += 1;
                
                // Safety limit - exit after ~5 minutes (3000 * 100ms = 300s)
                if poll_count > 3000 {
                    console_log("\n\x1b[90m[Timeout - maximum follow duration reached]\x1b[0m\n");
                    break;
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
