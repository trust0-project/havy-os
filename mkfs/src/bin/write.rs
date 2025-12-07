// write - Write content to a file
//
// Usage:
//   write <filename> <content...>    Write content to file

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{console_log, argc, argv, get_cwd, write_file};
    use mkfs::syscalls::print;

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

        if arg_count < 2 {
            console_log("Usage: write <filename> <content...>\n");
            console_log("Example: write test.txt Hello World!\n");
            return;
        }

        // Get filename (first argument)
        let mut filename_buf = [0u8; 256];
        let filename_len = match argv(0, &mut filename_buf) {
            Some(len) => len,
            None => {
                console_log("\x1b[1;31mError:\x1b[0m Invalid filename\n");
                return;
            }
        };

        // Resolve path
        let mut path_buf = [0u8; 512];
        let path_len = resolve_path(&filename_buf[..filename_len], &mut path_buf);
        let path_str = unsafe { core::str::from_utf8_unchecked(&path_buf[..path_len]) };

        // Collect content from remaining arguments
        let mut content = [0u8; 8192];
        let mut content_len = 0usize;

        for i in 1..arg_count {
            let mut arg_buf = [0u8; 1024];
            if let Some(arg_len) = argv(i, &mut arg_buf) {
                // Add space between arguments
                if content_len > 0 && content_len < content.len() {
                    content[content_len] = b' ';
                    content_len += 1;
                }

                // Copy argument
                let copy_len = arg_len.min(content.len() - content_len);
                if copy_len > 0 {
                    content[content_len..content_len + copy_len].copy_from_slice(&arg_buf[..copy_len]);
                    content_len += copy_len;
                }
            }
        }

        // Write file
        if write_file(path_str, &content[..content_len]) {
            console_log("\x1b[1;32m[OK]\x1b[0m Written to ");
            unsafe { print(path_buf.as_ptr(), path_len) };
            console_log("\n");
        } else {
            console_log("\x1b[1;31mError:\x1b[0m Failed to write to ");
            unsafe { print(path_buf.as_ptr(), path_len) };
            console_log("\n");
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
