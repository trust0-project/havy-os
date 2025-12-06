// wget - Download files from the web

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{console_log, argc, argv, get_cwd, http_fetch, write_file, print_int};
    use mkfs::syscalls::print;

    #[no_mangle]
    pub extern "C" fn _start() {
        let arg_count = argc();

        if arg_count < 1 {
            console_log("Usage: wget <url> [-O file]\n");
            return;
        }

        // Get URL (first arg)
        let mut url_buf = [0u8; 256];
        let url_len = match argv(0, &mut url_buf) {
            Some(len) => len,
            None => {
                console_log("Error: Invalid URL\n");
                return;
            }
        };

        // Check for -O option (args: url -O path)
        let mut rel_path_buf = [0u8; 128];
        let mut rel_path_len: Option<usize> = None;

        let mut i = 1;
        while i < arg_count {
            let mut opt_buf = [0u8; 8];
            if let Some(opt_len) = argv(i, &mut opt_buf) {
                if opt_len == 2 && opt_buf[0] == b'-' && opt_buf[1] == b'O' {
                    // Next arg is the output path
                    if i + 1 < arg_count {
                        rel_path_len = argv(i + 1, &mut rel_path_buf);
                    }
                    break;
                }
            }
            i += 1;
        }

        console_log("Fetching: ");
        unsafe { print(url_buf.as_ptr(), url_len) };
        console_log("\n");

        // Make HTTP request
        let url_str = unsafe { core::str::from_utf8_unchecked(&url_buf[..url_len]) };
        let mut resp_buf = [0u8; 16384];
        
        match http_fetch(url_str, &mut resp_buf) {
            Some(resp_len) => {
                console_log("Received ");
                print_int(resp_len as i64);
                console_log(" bytes\n");

                if let Some(path_len) = rel_path_len {
                    // Build absolute path
                    let mut abs_path_buf = [0u8; 256];
                    let abs_path_len: usize;

                    // Check if path is already absolute
                    if rel_path_buf[0] == b'/' {
                        // Already absolute, just copy
                        let len = path_len.min(256);
                        abs_path_buf[..len].copy_from_slice(&rel_path_buf[..len]);
                        abs_path_len = len;
                    } else {
                        // Relative path - prepend CWD
                        let mut cwd_buf = [0u8; 128];
                        if let Some(cwd_len) = get_cwd(&mut cwd_buf) {
                            abs_path_buf[..cwd_len].copy_from_slice(&cwd_buf[..cwd_len]);

                            let mut idx = cwd_len;
                            // Add slash if CWD doesn't end with one
                            if idx > 0 && abs_path_buf[idx - 1] != b'/' {
                                abs_path_buf[idx] = b'/';
                                idx += 1;
                            }

                            // Append relative path
                            let remaining = 256 - idx;
                            let to_copy = path_len.min(remaining);
                            abs_path_buf[idx..idx + to_copy].copy_from_slice(&rel_path_buf[..to_copy]);
                            abs_path_len = idx + to_copy;
                        } else {
                            // Default to root
                            abs_path_buf[0] = b'/';
                            let len = path_len.min(255);
                            abs_path_buf[1..1 + len].copy_from_slice(&rel_path_buf[..len]);
                            abs_path_len = 1 + len;
                        }
                    }

                    console_log("Saving to: ");
                    unsafe { print(abs_path_buf.as_ptr(), abs_path_len) };
                    console_log("\n");

                    let path_str = unsafe { core::str::from_utf8_unchecked(&abs_path_buf[..abs_path_len]) };
                    if write_file(path_str, &resp_buf[..resp_len]) {
                        console_log("OK: Wrote ");
                        print_int(resp_len as i64);
                        console_log(" bytes\n");
                    } else {
                        console_log("Error: Write failed\n");
                    }
                } else {
                    // Print to stdout
                    unsafe { print(resp_buf.as_ptr(), resp_len) };
                    if resp_len > 0 && resp_buf[resp_len - 1] != b'\n' {
                        console_log("\n");
                    }
                }
            }
            None => {
                console_log("Error: Request failed\n");
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
