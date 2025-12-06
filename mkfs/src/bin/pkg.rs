// pkg - Simple package manager
//
// Usage:
//   pkg list              List installed packages
//   pkg install <url>     Install package from URL
//   pkg help              Show help

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{console_log, argc, argv, list_files, is_net_available, http_fetch, write_file, print_int};
    use mkfs::syscalls::{print, fs_read};

    fn print_help() {
        console_log("\x1b[1;36mpkg\x1b[0m - BAVY Package Manager\n\n");
        console_log("\x1b[1mUSAGE:\x1b[0m\n");
        console_log("    pkg <command> [args]\n\n");
        console_log("\x1b[1mCOMMANDS:\x1b[0m\n");
        console_log("    list              List installed packages in /usr/bin\n");
        console_log("    install <url>     Download and install a WASM package\n");
        console_log("    info <name>       Show package info\n");
        console_log("    help              Show this help message\n\n");
        console_log("\x1b[1mEXAMPLES:\x1b[0m\n");
        console_log("    pkg list\n");
        console_log("    pkg install https://example.com/app.wasm\n");
        console_log("    pkg info cowsay\n");
    }

    fn cmd_list() {
        console_log("\x1b[1;36mInstalled Packages\x1b[0m\n");
        console_log("\x1b[90m─────────────────────────────────────\x1b[0m\n\n");

        // Read directory listing
        let mut buf = [0u8; 4096];
        match list_files(&mut buf) {
            Some(len) if len > 0 => {
                // Parse and display the file list (format: "path:size\n")
                let content = &buf[..len];
                let mut count = 0;
                let mut start = 0;

                for i in 0..content.len() {
                    if content[i] == b'\n' || i == content.len() - 1 {
                        let end = if content[i] == b'\n' { i } else { i + 1 };
                        if end > start {
                            // Find the colon to get just the name
                            let line = &content[start..end];
                            let mut colon_pos = line.len();
                            for (j, &c) in line.iter().enumerate().rev() {
                                if c == b':' {
                                    colon_pos = j;
                                    break;
                                }
                            }
                            let path = &line[..colon_pos];
                            
                            // Check if it's in /usr/bin/
                            if path.starts_with(b"/usr/bin/") && path.len() > 9 {
                                let name = &path[9..];
                                console_log("  \x1b[32m●\x1b[0m ");
                                unsafe { print(name.as_ptr(), name.len()) };
                                console_log("\n");
                                count += 1;
                            }
                        }
                        start = i + 1;
                    }
                }

                console_log("\n\x1b[90m");
                print_int(count as i64);
                console_log(" package(s) installed\x1b[0m\n");
            }
            _ => {
                // Fallback: show known packages
                console_log("\x1b[33mNote: Directory listing not available.\x1b[0m\n");
                console_log("\x1b[33mKnown system packages:\x1b[0m\n\n");
                console_log("  \x1b[32m●\x1b[0m cowsay     ASCII art cow\n");
                console_log("  \x1b[32m●\x1b[0m dmesg      Kernel log viewer\n");
                console_log("  \x1b[32m●\x1b[0m hello      Test WASM binary\n");
                console_log("  \x1b[32m●\x1b[0m help       Show available commands\n");
                console_log("  \x1b[32m●\x1b[0m nano       Text file viewer\n");
                console_log("  \x1b[32m●\x1b[0m pkg        Package manager (this)\n");
                console_log("  \x1b[32m●\x1b[0m wget       Download files\n");
                console_log("\n\x1b[90mRun 'ls /usr/bin' to see all installed binaries.\x1b[0m\n");
            }
        }
    }

    fn cmd_install(url: &[u8]) {
        // Check network
        if !is_net_available() {
            console_log("\x1b[31mError: Network not available\x1b[0m\n");
            return;
        }

        // Extract filename from URL
        let mut name_start = 0;
        for i in (0..url.len()).rev() {
            if url[i] == b'/' {
                name_start = i + 1;
                break;
            }
        }

        if name_start >= url.len() {
            console_log("\x1b[31mError: Could not determine package name from URL\x1b[0m\n");
            return;
        }

        let name = &url[name_start..];

        // Remove .wasm extension if present for display
        let display_len = if name.len() > 5 && &name[name.len()-5..] == b".wasm" {
            name.len() - 5
        } else {
            name.len()
        };

        console_log("\x1b[1;36mInstalling package:\x1b[0m ");
        unsafe { print(name.as_ptr(), display_len) };
        console_log("\n\n");

        console_log("  \x1b[90m→\x1b[0m Downloading... ");

        // Fetch the package
        let url_str = unsafe { core::str::from_utf8_unchecked(url) };
        let mut resp_buf = [0u8; 1048576]; // 1MB max package size
        
        match http_fetch(url_str, &mut resp_buf) {
            Some(resp_len) => {
                console_log("\x1b[32mOK\x1b[0m (");
                print_int(resp_len as i64);
                console_log(" bytes)\n");

                // Build destination path: /usr/bin/<name>
                let mut dest_path = [0u8; 256];
                let prefix = b"/usr/bin/";
                dest_path[..prefix.len()].copy_from_slice(prefix);

                // Use name without .wasm extension
                let dest_name_len = display_len.min(256 - prefix.len());
                dest_path[prefix.len()..prefix.len() + dest_name_len].copy_from_slice(&name[..dest_name_len]);
                let dest_len = prefix.len() + dest_name_len;

                console_log("  \x1b[90m→\x1b[0m Installing to ");
                unsafe { print(dest_path.as_ptr(), dest_len) };
                console_log("... ");

                // Write the file
                let dest_str = unsafe { core::str::from_utf8_unchecked(&dest_path[..dest_len]) };
                if write_file(dest_str, &resp_buf[..resp_len]) {
                    console_log("\x1b[32mOK\x1b[0m\n\n");
                    console_log("\x1b[32m✓ Package installed successfully!\x1b[0m\n");
                    console_log("  Run '\x1b[1m");
                    unsafe { print(name.as_ptr(), display_len) };
                    console_log("\x1b[0m' to use it.\n");
                } else {
                    console_log("\x1b[31mFailed\x1b[0m\n");
                    console_log("\x1b[31mError: Could not write to /usr/bin/\x1b[0m\n");
                }
            }
            None => {
                console_log("\x1b[31mFailed\x1b[0m\n");
                console_log("\x1b[31mError: Download failed\x1b[0m\n");
            }
        }
    }

    fn cmd_info(name: &[u8]) {
        // Build path to /usr/bin/<name>
        let mut path = [0u8; 256];
        let prefix = b"/usr/bin/";
        path[..prefix.len()].copy_from_slice(prefix);
        let name_len = name.len().min(256 - prefix.len());
        path[prefix.len()..prefix.len() + name_len].copy_from_slice(&name[..name_len]);
        let path_len = prefix.len() + name_len;

        // Try to read the file to get its size
        let mut buf = [0u8; 1048576]; // 1MB buffer
        let len = unsafe {
            fs_read(path.as_ptr(), path_len as i32, buf.as_mut_ptr(), buf.len() as i32)
        };

        console_log("\x1b[1;36mPackage Info:\x1b[0m ");
        unsafe { print(name.as_ptr(), name.len()) };
        console_log("\n");
        console_log("\x1b[90m─────────────────────────────────────\x1b[0m\n");

        if len < 0 {
            console_log("\x1b[31mPackage not found\x1b[0m\n");
            return;
        }

        console_log("  Location: ");
        unsafe { print(path.as_ptr(), path_len) };
        console_log("\n");
        console_log("  Size:     ");
        print_int(len as i64);
        console_log(" bytes\n");
        console_log("  Type:     WASM binary\n");
    }

    #[no_mangle]
    pub extern "C" fn _start() {
        let arg_count = argc();

        if arg_count < 1 {
            print_help();
            return;
        }

        // Get command
        let mut cmd_buf = [0u8; 32];
        let cmd_len = match argv(0, &mut cmd_buf) {
            Some(len) => len,
            None => {
                print_help();
                return;
            }
        };

        let cmd = &cmd_buf[..cmd_len];

        match cmd {
            b"list" | b"ls" => cmd_list(),
            b"install" | b"i" => {
                if arg_count < 2 {
                    console_log("\x1b[31mError: Missing URL argument\x1b[0m\n");
                    console_log("Usage: pkg install <url>\n");
                    return;
                }
                let mut url_buf = [0u8; 512];
                if let Some(url_len) = argv(1, &mut url_buf) {
                    cmd_install(&url_buf[..url_len]);
                }
            }
            b"info" => {
                if arg_count < 2 {
                    console_log("\x1b[31mError: Missing package name\x1b[0m\n");
                    console_log("Usage: pkg info <name>\n");
                    return;
                }
                let mut name_buf = [0u8; 64];
                if let Some(name_len) = argv(1, &mut name_buf) {
                    cmd_info(&name_buf[..name_len]);
                }
            }
            b"help" | b"-h" | b"--help" => print_help(),
            _ => {
                console_log("\x1b[31mUnknown command: \x1b[0m");
                unsafe { print(cmd.as_ptr(), cmd.len()) };
                console_log("\n\nRun 'pkg help' for usage.\n");
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
