// rm - Remove files and directories
//
// Usage:
//   rm <file>            Remove a file
//   rm -r <dir>          Remove a directory recursively
//   rm -v <file>         Verbose output

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{console_log, argc, argv, get_cwd, print, remove_file, is_dir, file_exists, list_dir};

    static mut ARG_BUF: [u8; 256] = [0u8; 256];
    static mut CWD_BUF: [u8; 256] = [0u8; 256];
    static mut PATH_BUF: [u8; 512] = [0u8; 512];
    static mut LIST_BUF: [u8; 2048] = [0u8; 2048];
    static mut SUB_PATH: [u8; 512] = [0u8; 512];

    fn remove_recursive(path: &str, verbose: bool) -> bool {
        // Check if it's a directory
        if is_dir(path) {
            // List directory contents
            let mut list_buf = [0u8; 2048];
            if let Some(len) = list_dir(path, &mut list_buf) {
                let data = &list_buf[..len];
                let mut pos = 0;
                
                // Parse each entry and remove
                while pos < len {
                    let line_start = pos;
                    while pos < len && data[pos] != b'\n' { pos += 1; }
                    let line_end = pos;
                    pos += 1;
                    
                    if line_start >= line_end { continue; }
                    let line = &data[line_start..line_end];
                    
                    // Find the colon separator (name:size format)
                    let mut colon = line.len();
                    for (i, &c) in line.iter().enumerate() {
                        if c == b':' { colon = i; break; }
                    }
                    if colon == 0 { continue; }
                    
                    let entry_name = &line[..colon];
                    
                    // Build full path for entry
                    let mut sub_path_buf = [0u8; 512];
                    let mut sub_len = 0;
                    for (i, &b) in path.as_bytes().iter().enumerate() {
                        if i < 512 { sub_path_buf[i] = b; sub_len += 1; }
                    }
                    if sub_len < 512 && sub_len > 0 && sub_path_buf[sub_len - 1] != b'/' {
                        sub_path_buf[sub_len] = b'/';
                        sub_len += 1;
                    }
                    for &b in entry_name {
                        if sub_len < 512 { sub_path_buf[sub_len] = b; sub_len += 1; }
                    }
                    
                    let sub_path_str = unsafe { core::str::from_utf8_unchecked(&sub_path_buf[..sub_len]) };
                    
                    // Recursively remove
                    let _ = remove_recursive(sub_path_str, verbose);
                }
            }
            
            // Now remove the empty directory
            if remove_file(path) {
                if verbose {
                    console_log("\x1b[1;32mrm:\x1b[0m removed '");
                    console_log(path);
                    console_log("'\n");
                }
                true
            } else {
                false
            }
        } else {
            // It's a file, just remove it
            if remove_file(path) {
                if verbose {
                    console_log("\x1b[1;32mrm:\x1b[0m removed '");
                    console_log(path);
                    console_log("'\n");
                }
                true
            } else {
                console_log("\x1b[1;31mrm:\x1b[0m cannot remove '");
                console_log(path);
                console_log("'\n");
                false
            }
        }
    }

    let arg_count = argc();
    
    if arg_count < 1 {
        console_log("Usage: rm [-rv] <file...>\n");
        return;
    }

    let mut verbose = false;
    let mut recursive = false;
    let mut files_start = 0;

    // Parse flags
    for i in 0..arg_count {
        let len = unsafe { argv(i, &mut *core::ptr::addr_of_mut!(ARG_BUF)) };
        if let Some(len) = len {
            let arg = unsafe { &(*core::ptr::addr_of!(ARG_BUF))[..len] };
            if arg.starts_with(b"-") {
                for &ch in &arg[1..] {
                    match ch {
                        b'v' => verbose = true,
                        b'r' | b'R' => recursive = true,
                        b'f' => {} // force - ignored
                        _ => {}
                    }
                }
                files_start = i + 1;
            } else {
                break;
            }
        }
    }

    if files_start >= arg_count {
        console_log("Usage: rm [-rv] <file...>\n");
        return;
    }

    // Get CWD
    let cwd_len = unsafe { get_cwd(&mut *core::ptr::addr_of_mut!(CWD_BUF)).unwrap_or(1) };
    let cwd = unsafe { &(*core::ptr::addr_of!(CWD_BUF))[..cwd_len] };

    // Process each file
    for i in files_start..arg_count {
        let len = unsafe { argv(i, &mut *core::ptr::addr_of_mut!(ARG_BUF)) };
        if let Some(len) = len {
            let file_arg = unsafe { &(*core::ptr::addr_of!(ARG_BUF))[..len] };
            
            let path_len = if file_arg.starts_with(b"/") {
                unsafe {
                    (*core::ptr::addr_of_mut!(PATH_BUF))[..len].copy_from_slice(file_arg);
                }
                len
            } else {
                unsafe {
                    let mut pos = 0;
                    (*core::ptr::addr_of_mut!(PATH_BUF))[..cwd_len].copy_from_slice(cwd);
                    pos = cwd_len;
                    if cwd_len > 1 || cwd[0] != b'/' {
                        (*core::ptr::addr_of_mut!(PATH_BUF))[pos] = b'/';
                        pos += 1;
                    }
                    (*core::ptr::addr_of_mut!(PATH_BUF))[pos..pos + len].copy_from_slice(file_arg);
                    pos + len
                }
            };

            let path = unsafe { &(*core::ptr::addr_of!(PATH_BUF))[..path_len] };
            let path_str = unsafe { core::str::from_utf8_unchecked(path) };

            if !file_exists(path_str) {
                console_log("\x1b[1;31mrm:\x1b[0m cannot remove '");
                print(path.as_ptr(), path.len());
                console_log("': No such file or directory\n");
                continue;
            }

            if is_dir(path_str) && !recursive {
                console_log("\x1b[1;31mrm:\x1b[0m cannot remove '");
                print(path.as_ptr(), path.len());
                console_log("': Is a directory (use -r)\n");
                continue;
            }

            if recursive && is_dir(path_str) {
                remove_recursive(path_str, verbose);
            } else {
                if remove_file(path_str) {
                    if verbose {
                        console_log("\x1b[1;32mrm:\x1b[0m removed '");
                        print(path.as_ptr(), path.len());
                        console_log("'\n");
                    }
                } else {
                    console_log("\x1b[1;31mrm:\x1b[0m cannot remove '");
                    print(path.as_ptr(), path.len());
                    console_log("'\n");
                }
            }
        }
    }
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
