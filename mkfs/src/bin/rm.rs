// rm - Remove files or directories
//
// Usage:
//   rm <file>            Remove a file
//   rm -r <dir>          Remove directory and contents recursively
//   rm -f <file>         Force removal (no error if not exists)
//   rm -v <file>         Verbose output

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use core::ptr::{addr_of, addr_of_mut};
    use mkfs::{console_log, argc, argv, get_cwd, remove_file, is_dir, list_dir};

    // Static buffers
    static mut ARG_BUF: [u8; 256] = [0u8; 256];
    static mut CWD_BUF: [u8; 256] = [0u8; 256];
    static mut PATH_BUF: [u8; 512] = [0u8; 512];
    static mut LIST_BUF: [u8; 8192] = [0u8; 8192];
    static mut CHILD_BUF: [u8; 512] = [0u8; 512];

    #[no_mangle]
    pub extern "C" fn _start() {
        let arg_count = argc();
        
        // Note: command name is not passed, arg 0 is first real argument
        if arg_count < 1 {
            console_log("Usage: rm [-rfv] <file...>\n");
            return;
        }

        let mut recursive = false;
        let mut force = false;
        let mut verbose = false;
        let mut files_start = 0;

        // Parse flags (starting from arg 0)
        for i in 0..arg_count {
            let len = unsafe { argv(i, &mut *addr_of_mut!(ARG_BUF)) };
            if let Some(len) = len {
                let arg = unsafe { &(*addr_of!(ARG_BUF))[..len] };
                if arg.starts_with(b"-") {
                    for &ch in &arg[1..] {
                        match ch {
                            b'r' | b'R' => recursive = true,
                            b'f' => force = true,
                            b'v' => verbose = true,
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
            console_log("Usage: rm [-rfv] <file...>\n");
            return;
        }

        // Get current working directory
        let cwd_len = unsafe { get_cwd(&mut *addr_of_mut!(CWD_BUF)).unwrap_or(1) };
        let cwd = unsafe { &(*addr_of!(CWD_BUF))[..cwd_len] };

        // Process each file argument
        for i in files_start..arg_count {
            let len = unsafe { argv(i, &mut *addr_of_mut!(ARG_BUF)) };
            if let Some(len) = len {
                let file_arg = unsafe { &(*addr_of!(ARG_BUF))[..len] };
                
                // Resolve path
                let path_len = if file_arg.starts_with(b"/") {
                    // Absolute path
                    unsafe {
                        PATH_BUF[..len].copy_from_slice(file_arg);
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
                        PATH_BUF[pos..pos + len].copy_from_slice(file_arg);
                        pos + len
                    }
                };

                let path = unsafe { &PATH_BUF[..path_len] };
                let path_str = unsafe { core::str::from_utf8_unchecked(path) };

                let is_directory = is_dir(path_str);

                if is_directory && !recursive {
                    console_log("\x1b[1;31mrm:\x1b[0m cannot remove '");
                    print_bytes(path);
                    console_log("': Is a directory (use -r)\n");
                    continue;
                }

                if is_directory {
                    // Remove directory contents first
                    remove_dir_contents(path, verbose);
                    
                    // Remove directory marker
                    let mut dir_path_len = path_len;
                    unsafe {
                        PATH_BUF[dir_path_len] = b'/';
                        dir_path_len += 1;
                    }
                    let dir_path = unsafe { &PATH_BUF[..dir_path_len] };
                    let dir_path_str = unsafe { core::str::from_utf8_unchecked(dir_path) };
                    
                    if remove_file(dir_path_str) {
                        if verbose {
                            console_log("\x1b[1;32mremoved directory\x1b[0m '");
                            print_bytes(path);
                            console_log("'\n");
                        }
                    }
                } else {
                    if remove_file(path_str) {
                        if verbose {
                            console_log("\x1b[1;32mremoved\x1b[0m '");
                            print_bytes(path);
                            console_log("'\n");
                        }
                    } else if !force {
                        console_log("\x1b[1;31mrm:\x1b[0m cannot remove '");
                        print_bytes(path);
                        console_log("': No such file\n");
                    }
                }
            }
        }
    }

    fn remove_dir_contents(path: &[u8], verbose: bool) {
        let path_str = unsafe { core::str::from_utf8_unchecked(path) };
        
        // List all files
        let list_len = unsafe { list_dir("/", &mut *addr_of_mut!(LIST_BUF)) };
        let Some(list_len) = list_len else { return };
        
        // Build prefix for matching
        let prefix_len = path.len() + 1; // path + "/"
        
        // Collect children matching this prefix
        let list_data = unsafe { &(*addr_of!(LIST_BUF))[..list_len] };
        let mut children: [([u8; 512], usize); 64] = [([0u8; 512], 0); 64];
        let mut child_count = 0;
        
        let mut start = 0;
        for i in 0..list_len {
            if list_data[i] == b'\n' {
                if i > start {
                    let line = &list_data[start..i];
                    // Parse "name:size"
                    if let Some(colon) = line.iter().position(|&b| b == b':') {
                        let name = &line[..colon];
                        // Check if this file is under our directory
                        if name.len() > path.len() && name.starts_with(path) && name[path.len()] == b'/' {
                            if child_count < 64 {
                                let entry_len = name.len();
                                children[child_count].0[..entry_len].copy_from_slice(name);
                                children[child_count].1 = entry_len;
                                child_count += 1;
                            }
                        }
                    }
                }
                start = i + 1;
            }
        }
        
        // Sort by depth (deepest first) - simple bubble sort
        for i in 0..child_count {
            for j in 0..child_count - 1 - i {
                let depth_j = count_slashes(&children[j].0[..children[j].1]);
                let depth_j1 = count_slashes(&children[j + 1].0[..children[j + 1].1]);
                if depth_j < depth_j1 {
                    // Swap
                    let tmp = children[j];
                    children[j] = children[j + 1];
                    children[j + 1] = tmp;
                }
            }
        }
        
        // Remove children
        for i in 0..child_count {
            let child = &children[i].0[..children[i].1];
            let child_str = unsafe { core::str::from_utf8_unchecked(child) };
            if remove_file(child_str) && verbose {
                console_log("\x1b[1;32mremoved\x1b[0m '");
                print_bytes(child);
                console_log("'\n");
            }
        }
    }

    fn count_slashes(s: &[u8]) -> usize {
        s.iter().filter(|&&b| b == b'/').count()
    }

    fn print_bytes(bytes: &[u8]) {
        unsafe { mkfs::print(bytes.as_ptr(), bytes.len()) };
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}

