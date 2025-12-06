// mkdir - Create directories
//
// Usage:
//   mkdir <dir>           Create a directory
//   mkdir -p <dir>        Create parent directories as needed
//   mkdir -v <dir>        Verbose output

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{console_log, argc, argv, mkdir, get_cwd, is_dir};

    // Static buffers
    static mut ARG_BUF: [u8; 256] = [0u8; 256];
    static mut CWD_BUF: [u8; 256] = [0u8; 256];
    static mut PATH_BUF: [u8; 512] = [0u8; 512];
    static mut TEMP_BUF: [u8; 512] = [0u8; 512];

    #[no_mangle]
    pub extern "C" fn _start() {
        let arg_count = argc();
        
        // Note: command name is not passed, arg 0 is first real argument
        if arg_count < 1 {
            console_log("Usage: mkdir [-pv] <directory...>\n");
            return;
        }

        let mut create_parents = false;
        let mut verbose = false;
        let mut dirs_start = 0;

        // Parse flags (starting from arg 0)
        for i in 0..arg_count {
            let len = unsafe { argv(i, &mut ARG_BUF) };
            if let Some(len) = len {
                let arg = unsafe { &ARG_BUF[..len] };
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

        // Get current working directory
        let cwd_len = unsafe { get_cwd(&mut CWD_BUF).unwrap_or(1) };
        let cwd = unsafe { &CWD_BUF[..cwd_len] };

        // Process each directory argument
        for i in dirs_start..arg_count {
            let len = unsafe { argv(i, &mut ARG_BUF) };
            if let Some(len) = len {
                let dir = unsafe { &ARG_BUF[..len] };
                
                // Resolve path
                let path_len = if dir.starts_with(b"/") {
                    // Absolute path
                    unsafe {
                        PATH_BUF[..len].copy_from_slice(dir);
                    }
                    len
                } else {
                    // Relative path
                    unsafe {
                        let mut pos = 0;
                        PATH_BUF[..cwd_len].copy_from_slice(cwd);
                        pos = cwd_len;
                        if cwd_len > 1 || cwd[0] != b'/' {
                            PATH_BUF[pos] = b'/';
                            pos += 1;
                        }
                        PATH_BUF[pos..pos + len].copy_from_slice(dir);
                        pos + len
                    }
                };

                let path = unsafe { &PATH_BUF[..path_len] };

                if create_parents {
                    // Create all parent directories
                    create_parents_recursive(path, verbose);
                } else {
                    // Create single directory
                    let path_str = unsafe { core::str::from_utf8_unchecked(path) };
                    if mkdir(path_str) {
                        if verbose {
                            console_log("\x1b[1;32mmkdir:\x1b[0m created '");
                            print_bytes(path);
                            console_log("'\n");
                        }
                    } else {
                        console_log("\x1b[1;31mmkdir:\x1b[0m cannot create '");
                        print_bytes(path);
                        console_log("'\n");
                    }
                }
            }
        }
    }

    fn create_parents_recursive(path: &[u8], verbose: bool) {
        // Split path and create each component
        let mut current_len = 0;
        let mut i = 0;
        
        // Skip leading slash
        if !path.is_empty() && path[0] == b'/' {
            unsafe { TEMP_BUF[0] = b'/' };
            current_len = 1;
            i = 1;
        }

        while i < path.len() {
            // Find next slash or end
            let start = i;
            while i < path.len() && path[i] != b'/' {
                i += 1;
            }
            
            if i > start {
                // Copy this component
                let component = &path[start..i];
                if current_len > 1 {
                    unsafe { TEMP_BUF[current_len] = b'/' };
                    current_len += 1;
                }
                unsafe { TEMP_BUF[current_len..current_len + component.len()].copy_from_slice(component) };
                current_len += component.len();

                // Try to create this directory
                let current_path = unsafe { &TEMP_BUF[..current_len] };
                let path_str = unsafe { core::str::from_utf8_unchecked(current_path) };
                
                if !is_dir(path_str) {
                    if mkdir(path_str) {
                        if verbose {
                            console_log("\x1b[1;32mmkdir:\x1b[0m created '");
                            print_bytes(current_path);
                            console_log("'\n");
                        }
                    }
                }
            }
            
            // Skip slash
            if i < path.len() && path[i] == b'/' {
                i += 1;
            }
        }
    }

    fn print_bytes(bytes: &[u8]) {
        unsafe { mkfs::print(bytes.as_ptr(), bytes.len()) };
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}

