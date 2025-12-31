// mkdir - Create directories
//
// Usage:
//   mkdir <dir>           Create a directory
//   mkdir -p <dir>        Create parent directories as needed
//   mkdir -v <dir>        Verbose output

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{console_log, argc, argv, mkdir, get_cwd, print};

    static mut ARG_BUF: [u8; 256] = [0u8; 256];
    static mut CWD_BUF: [u8; 256] = [0u8; 256];
    static mut PATH_BUF: [u8; 512] = [0u8; 512];
    static mut TEMP_BUF: [u8; 512] = [0u8; 512];

    fn is_dir_path(path: &str) -> bool {
        // Simple check - if mkdir succeeded or path ends with '/'
        mkfs::file_exists(path)
    }

    let arg_count = argc();
    
    if arg_count < 1 {
        console_log("Usage: mkdir [-pv] <directory...>\n");
        return;
    }

    let mut create_parents = false;
    let mut verbose = false;
    let mut dirs_start = 0;

    // Parse flags
    for i in 0..arg_count {
        let len = unsafe { argv(i, &mut *core::ptr::addr_of_mut!(ARG_BUF)) };
        if let Some(len) = len {
            let arg = unsafe { &(*core::ptr::addr_of!(ARG_BUF))[..len] };
            if arg.starts_with(b"-") {
                for &ch in &arg[1..] {
                    match ch {
                        b'p' => create_parents = true,
                        b'v' => verbose = true,
                        _ => {}
                    }
                }
                dirs_start = i + 1;
            } else {
                break;
            }
        }
    }

    if dirs_start >= arg_count {
        console_log("Usage: mkdir [-pv] <directory...>\n");
        return;
    }

    // Get CWD
    let cwd_len = unsafe { get_cwd(&mut *core::ptr::addr_of_mut!(CWD_BUF)).unwrap_or(1) };
    let cwd = unsafe { &(*core::ptr::addr_of!(CWD_BUF))[..cwd_len] };

    // Process each directory
    for i in dirs_start..arg_count {
        let len = unsafe { argv(i, &mut *core::ptr::addr_of_mut!(ARG_BUF)) };
        if let Some(len) = len {
            let dir = unsafe { &(*core::ptr::addr_of!(ARG_BUF))[..len] };
            
            // Resolve path
            let path_len = if dir.starts_with(b"/") {
                unsafe {
                    (*core::ptr::addr_of_mut!(PATH_BUF))[..len].copy_from_slice(dir);
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
                    (*core::ptr::addr_of_mut!(PATH_BUF))[pos..pos + len].copy_from_slice(dir);
                    pos + len
                }
            };

            let path = unsafe { &(*core::ptr::addr_of!(PATH_BUF))[..path_len] };

            if create_parents {
                // Create all parent directories
                let mut current_len = 0;
                let mut idx = 0;
                
                if !path.is_empty() && path[0] == b'/' {
                    unsafe { (*core::ptr::addr_of_mut!(TEMP_BUF))[0] = b'/' };
                    current_len = 1;
                    idx = 1;
                }

                while idx < path.len() {
                    let start = idx;
                    while idx < path.len() && path[idx] != b'/' {
                        idx += 1;
                    }
                    
                    if idx > start {
                        let component = &path[start..idx];
                        if current_len > 1 {
                            unsafe { (*core::ptr::addr_of_mut!(TEMP_BUF))[current_len] = b'/' };
                            current_len += 1;
                        }
                        unsafe { (*core::ptr::addr_of_mut!(TEMP_BUF))[current_len..current_len + component.len()].copy_from_slice(component) };
                        current_len += component.len();

                        let current_path = unsafe { &(*core::ptr::addr_of!(TEMP_BUF))[..current_len] };
                        let path_str = unsafe { core::str::from_utf8_unchecked(current_path) };
                        
                        if !is_dir_path(path_str) {
                            if mkdir(path_str) {
                                if verbose {
                                    console_log("\x1b[1;32mmkdir:\x1b[0m created '");
                                    print(current_path.as_ptr(), current_path.len());
                                    console_log("'\n");
                                }
                            }
                        }
                    }
                    
                    if idx < path.len() && path[idx] == b'/' {
                        idx += 1;
                    }
                }
            } else {
                let path_str = unsafe { core::str::from_utf8_unchecked(path) };
                if mkdir(path_str) {
                    if verbose {
                        console_log("\x1b[1;32mmkdir:\x1b[0m created '");
                        print(path.as_ptr(), path.len());
                        console_log("'\n");
                    }
                } else {
                    console_log("\x1b[1;31mmkdir:\x1b[0m cannot create '");
                    print(path.as_ptr(), path.len());
                    console_log("'\n");
                }
            }
        }
    }
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
