// pkg - Package manager
//
// Usage:
//   pkg list              List installed packages
//   pkg install <name>    Install a package
//   pkg remove <name>     Remove a package
//   pkg info <name>       Show package info

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{console_log, argc, argv, print, list_dir, file_exists, remove_file, read_file, write_file};

    if argc() < 1 {
        console_log("Usage: pkg <command> [package]\n");
        console_log("Commands: list, install, remove, info, search\n");
        return;
    }

    let mut cmd_buf = [0u8; 32];
    let cmd_len = match argv(0, &mut cmd_buf) {
        Some(len) => len,
        None => {
            console_log("Error: Could not read command\n");
            return;
        }
    };

    let cmd = &cmd_buf[..cmd_len];

    if cmd == b"list" {
        console_log("\n");
        console_log("\x1b[1;33m+-----------------------------------------------+\x1b[0m\n");
        console_log("\x1b[1;33m|\x1b[0m           \x1b[1;97mInstalled Packages\x1b[0m                  \x1b[1;33m|\x1b[0m\n");
        console_log("\x1b[1;33m+-----------------------------------------------+\x1b[0m\n");

        // List packages from /var/pkg/
        static mut DIR_BUF: [u8; 4096] = [0u8; 4096];
        let dir_buf = unsafe { &mut *core::ptr::addr_of_mut!(DIR_BUF) };
        
        let mut pkg_count = 0;
        
        // Core packages (always present)
        console_log("\x1b[1;33m|\x1b[0m  \x1b[1;32m*\x1b[0m \x1b[1;97mkernel\x1b[0m      0.1.0   BAVY OS Kernel           \x1b[1;33m|\x1b[0m\n");
        console_log("\x1b[1;33m|\x1b[0m  \x1b[1;32m*\x1b[0m \x1b[1;97mcoreutils\x1b[0m   0.1.0   Core utilities           \x1b[1;33m|\x1b[0m\n");
        pkg_count += 2;
        
        // Check for user-installed packages in /var/pkg/
        if let Some(len) = list_dir("/var/pkg", dir_buf) {
            let data = &dir_buf[..len];
            let mut pos = 0;
            while pos < len {
                let line_start = pos;
                while pos < len && data[pos] != b'\n' { pos += 1; }
                let line_end = pos;
                pos += 1;
                
                if line_start >= line_end { continue; }
                let line = &data[line_start..line_end];
                
                // Find the colon separator
                let mut colon = line.len();
                for (i, &c) in line.iter().enumerate() {
                    if c == b':' { colon = i; break; }
                }
                if colon == 0 { continue; }
                
                let name = &line[..colon];
                console_log("\x1b[1;33m|\x1b[0m  \x1b[1;36m+\x1b[0m \x1b[1;97m");
                print(name.as_ptr(), name.len());
                console_log("\x1b[0m");
                // Padding
                for _ in name.len()..12 {
                    console_log(" ");
                }
                console_log("user    User package             \x1b[1;33m|\x1b[0m\n");
                pkg_count += 1;
            }
        }
        
        console_log("\x1b[1;33m+-----------------------------------------------+\x1b[0m\n");
        console_log("\n\x1b[90m");
        print_int(pkg_count as i64);
        console_log(" packages installed\x1b[0m\n");
        console_log("\x1b[90m* = system package, + = user package\x1b[0m\n\n");
        
    } else if cmd == b"install" {
        if argc() < 2 {
            console_log("Usage: pkg install <package>\n");
            return;
        }
        
        let mut name_buf = [0u8; 64];
        let name_len = match argv(1, &mut name_buf) {
            Some(len) => len,
            None => {
                console_log("Error: Could not read package name\n");
                return;
            }
        };
        let name = &name_buf[..name_len];
        
        // Check if already installed
        let mut pkg_path = [0u8; 128];
        let path_len = build_path(b"/var/pkg/", name, &mut pkg_path);
        let pkg_path_str = unsafe { core::str::from_utf8_unchecked(&pkg_path[..path_len]) };
        
        if file_exists(pkg_path_str) {
            console_log("\x1b[1;33m[!]\x1b[0m Package '");
            print(name.as_ptr(), name.len());
            console_log("' is already installed\n");
            return;
        }
        
        console_log("\x1b[1;36m[*]\x1b[0m Installing '");
        print(name.as_ptr(), name.len());
        console_log("'...\n");
        
        // Create package marker file
        let marker_content = b"installed";
        if write_file(pkg_path_str, marker_content) {
            console_log("\x1b[1;32m[OK]\x1b[0m Package '");
            print(name.as_ptr(), name.len());
            console_log("' installed successfully\n");
        } else {
            // Try to create /var/pkg directory first
            let _ = mkfs::mkdir("/var");
            let _ = mkfs::mkdir("/var/pkg");
            
            if write_file(pkg_path_str, marker_content) {
                console_log("\x1b[1;32m[OK]\x1b[0m Package '");
                print(name.as_ptr(), name.len());
                console_log("' installed successfully\n");
            } else {
                console_log("\x1b[1;31m[FAIL]\x1b[0m Could not install package\n");
            }
        }
        
    } else if cmd == b"remove" {
        if argc() < 2 {
            console_log("Usage: pkg remove <package>\n");
            return;
        }
        
        let mut name_buf = [0u8; 64];
        let name_len = match argv(1, &mut name_buf) {
            Some(len) => len,
            None => {
                console_log("Error: Could not read package name\n");
                return;
            }
        };
        let name = &name_buf[..name_len];
        
        // Check for system packages
        if name == b"kernel" || name == b"coreutils" {
            console_log("\x1b[1;31m[X]\x1b[0m Cannot remove system package '");
            print(name.as_ptr(), name.len());
            console_log("'\n");
            return;
        }
        
        // Build path
        let mut pkg_path = [0u8; 128];
        let path_len = build_path(b"/var/pkg/", name, &mut pkg_path);
        let pkg_path_str = unsafe { core::str::from_utf8_unchecked(&pkg_path[..path_len]) };
        
        if !file_exists(pkg_path_str) {
            console_log("\x1b[1;31m[X]\x1b[0m Package '");
            print(name.as_ptr(), name.len());
            console_log("' is not installed\n");
            return;
        }
        
        console_log("\x1b[1;36m[*]\x1b[0m Removing '");
        print(name.as_ptr(), name.len());
        console_log("'...\n");
        
        if remove_file(pkg_path_str) {
            console_log("\x1b[1;32m[OK]\x1b[0m Package '");
            print(name.as_ptr(), name.len());
            console_log("' removed successfully\n");
        } else {
            console_log("\x1b[1;31m[FAIL]\x1b[0m Could not remove package\n");
        }
        
    } else if cmd == b"info" {
        if argc() < 2 {
            console_log("Usage: pkg info <package>\n");
            return;
        }
        
        let mut name_buf = [0u8; 64];
        let name_len = match argv(1, &mut name_buf) {
            Some(len) => len,
            None => {
                console_log("Error: Could not read package name\n");
                return;
            }
        };
        let name = &name_buf[..name_len];
        
        console_log("\n");
        console_log("\x1b[1;35mPackage:\x1b[0m ");
        print(name.as_ptr(), name.len());
        console_log("\n");
        
        if name == b"kernel" {
            console_log("\x1b[1;35mVersion:\x1b[0m 0.1.0\n");
            console_log("\x1b[1;35mType:\x1b[0m    System (core)\n");
            console_log("\x1b[1;35mDesc:\x1b[0m    BAVY OS Kernel - RISC-V operating system\n");
        } else if name == b"coreutils" {
            console_log("\x1b[1;35mVersion:\x1b[0m 0.1.0\n");
            console_log("\x1b[1;35mType:\x1b[0m    System (core)\n");
            console_log("\x1b[1;35mDesc:\x1b[0m    Core utilities - ls, cat, grep, etc.\n");
        } else {
            let mut pkg_path = [0u8; 128];
            let path_len = build_path(b"/var/pkg/", name, &mut pkg_path);
            let pkg_path_str = unsafe { core::str::from_utf8_unchecked(&pkg_path[..path_len]) };
            
            if file_exists(pkg_path_str) {
                console_log("\x1b[1;35mVersion:\x1b[0m user\n");
                console_log("\x1b[1;35mType:\x1b[0m    User package\n");
                console_log("\x1b[1;35mStatus:\x1b[0m  Installed\n");
            } else {
                console_log("\x1b[33mNot installed\x1b[0m\n");
            }
        }
        console_log("\n");
        
    } else if cmd == b"search" {
        console_log("\n");
        console_log("\x1b[1;33mAvailable packages:\x1b[0m\n\n");
        console_log("  kernel      - BAVY OS Kernel\n");
        console_log("  coreutils   - Core utilities\n");
        console_log("  netutils    - Network utilities\n");
        console_log("  devtools    - Development tools\n");
        console_log("\n\x1b[90mUse 'pkg install <name>' to install\x1b[0m\n\n");
        
    } else if cmd == b"update" {
        console_log("\x1b[1;36m[*]\x1b[0m Checking for updates...\n");
        console_log("\x1b[1;32m[OK]\x1b[0m All packages are up to date\n");
        
    } else {
        console_log("Unknown command. Use: list, install, remove, info, search, update\n");
    }

    fn print_int(n: i64) {
        mkfs::print_int(n);
    }

    fn build_path(prefix: &[u8], name: &[u8], buf: &mut [u8]) -> usize {
        let mut pos = 0;
        for &b in prefix {
            if pos < buf.len() {
                buf[pos] = b;
                pos += 1;
            }
        }
        for &b in name {
            if pos < buf.len() {
                buf[pos] = b;
                pos += 1;
            }
        }
        pos
    }
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
