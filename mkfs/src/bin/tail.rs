// tail - Show last lines of a file
//
// Usage:
//   tail <file>           Show last 10 lines
//   tail -n <N> <file>    Show last N lines
//   tail -<N> <file>      Show last N lines (shorthand)

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{console_log, argc, argv, get_cwd, print, fs_read};

    fn parse_num(s: &[u8]) -> Option<usize> {
        if s.is_empty() { return None; }
        let mut result = 0usize;
        for &c in s {
            if c < b'0' || c > b'9' { return None; }
            result = result.checked_mul(10)?.checked_add((c - b'0') as usize)?;
        }
        Some(result)
    }

    fn resolve_path(arg: &[u8], out: &mut [u8], cwd: &[u8], cwd_len: Option<usize>) -> usize {
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
            if out.len() > 0 { out[0] = b'/'; }
            let copy_len = arg.len().min(out.len() - 1);
            out[1..1 + copy_len].copy_from_slice(&arg[..copy_len]);
            1 + copy_len
        }
    }

    fn print_last_lines(content: &[u8], num_lines: usize) {
        let mut line_positions: [usize; 512] = [0; 512];
        let mut line_count = 0usize;
        line_positions[0] = 0;

        for (idx, &c) in content.iter().enumerate() {
            if c == b'\n' && idx + 1 < content.len() && line_count + 1 < 512 {
                line_count += 1;
                line_positions[line_count] = idx + 1;
            }
        }
        line_count += 1;

        let start_line = if line_count > num_lines { line_count - num_lines } else { 0 };

        for line_idx in start_line..line_count {
            let line_start = line_positions[line_idx];
            let line_end = if line_idx + 1 < line_count {
                line_positions[line_idx + 1] - 1
            } else {
                content.len()
            };

            if line_start < content.len() {
                let end = line_end.min(content.len());
                print(content[line_start..end].as_ptr(), end - line_start);
                console_log("\n");
            }
        }
    }

    let arg_count = argc();

    if arg_count < 1 {
        console_log("Usage: tail [-n NUM] <file>\n");
        return;
    }

    let mut num_lines = 10usize;
    let mut file_path: Option<([u8; 512], usize)> = None;

    let mut cwd = [0u8; 256];
    let cwd_len = get_cwd(&mut cwd);

    let mut i = 0usize;
    while i < arg_count {
        let mut arg_buf = [0u8; 256];
        let arg_len = match argv(i, &mut arg_buf) {
            Some(len) => len,
            None => { i += 1; continue; }
        };
        let arg = &arg_buf[..arg_len];

        if arg == b"-n" {
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
            if let Some(n) = parse_num(&arg[2..]) {
                num_lines = n.max(1);
            }
        } else if arg.starts_with(b"-") && arg.len() > 1 && arg[1] >= b'0' && arg[1] <= b'9' {
            if let Some(n) = parse_num(&arg[1..]) {
                num_lines = n.max(1);
            }
        } else if !arg.starts_with(b"-") && file_path.is_none() {
            let mut path_buf = [0u8; 512];
            let path_len = resolve_path(arg, &mut path_buf, &cwd, cwd_len);
            file_path = Some((path_buf, path_len));
        }

        i += 1;
    }

    let (path_buf, path_len) = match file_path {
        Some(p) => p,
        None => {
            console_log("Usage: tail [-n NUM] <file>\n");
            return;
        }
    };

    static mut CONTENT: [u8; 32768] = [0u8; 32768]; // 32KB buffer
    let read_len = unsafe {
        fs_read(path_buf.as_ptr(), path_len as i32, (*core::ptr::addr_of_mut!(CONTENT)).as_mut_ptr(), 32768)
    };

    if read_len < 0 {
        console_log("\x1b[1;31mtail:\x1b[0m cannot open '");
        print(path_buf.as_ptr(), path_len);
        console_log("': No such file\n");
        return;
    }

    let content = unsafe { &(*core::ptr::addr_of!(CONTENT))[..read_len as usize] };
    print_last_lines(content, num_lines);
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
