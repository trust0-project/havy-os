// sysinfo - Display system information
//
// Usage:
//   sysinfo      Display system information including kernel, memory, network, etc.

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use core::ptr::{addr_of, addr_of_mut};
    use mkfs::{
        console_log, get_time, get_heap_stats, get_net_info, is_fs_available,
        is_net_available, format_ipv4, print_int, get_version,
    };

    // Static buffer for version string
    static mut VERSION_BUF: [u8; 32] = [0u8; 32];
    static mut IP_BUF: [u8; 16] = [0u8; 16];

    #[no_mangle]
    pub extern "C" fn _start() {
        // Get version
        let version_len = unsafe { 
            if let Some(len) = get_version(&mut *addr_of_mut!(VERSION_BUF)) {
                len
            } else {
                0
            }
        };
        
        let uptime_ms = get_time();
        let uptime_sec = uptime_ms / 1000;

        console_log("\n");
        console_log("\x1b[1;35m+-------------------------------------------------------------+\x1b[0m\n");
        console_log("\x1b[1;35m|\x1b[0m              \x1b[1;97mBAVY OS System Information\x1b[0m                     \x1b[1;35m|\x1b[0m\n");
        console_log("\x1b[1;35m+-------------------------------------------------------------+\x1b[0m\n");

        // Kernel version
        console_log("\x1b[1;35m|\x1b[0m  Kernel:       \x1b[1;97mBAVY OS v");
        if version_len > 0 {
            unsafe { print_bytes(&(*addr_of!(VERSION_BUF))[..version_len]) };
        } else {
            console_log("?");
        }
        console_log("\x1b[0m");
        // Pad to align
        let version_display_len = 10 + version_len; // "BAVY OS v" + version
        pad_spaces(44 - version_display_len.min(44));
        console_log("\x1b[1;35m|\x1b[0m\n");

        console_log("\x1b[1;35m|\x1b[0m  Architecture: \x1b[1;97mRISC-V 64-bit (RV64GC)\x1b[0m                       \x1b[1;35m|\x1b[0m\n");
        console_log("\x1b[1;35m|\x1b[0m  Mode:         \x1b[1;97mMachine Mode (M-Mode)\x1b[0m                        \x1b[1;35m|\x1b[0m\n");
        console_log("\x1b[1;35m|\x1b[0m  Runtime:      \x1b[1;97mJavaScript + Native\x1b[0m                          \x1b[1;35m|\x1b[0m\n");
        console_log("\x1b[1;35m|\x1b[0m                                                             \x1b[1;35m|\x1b[0m\n");

        // Network status
        if is_net_available() {
            if let Some(info) = get_net_info() {
                let ip_len = unsafe { format_ipv4(&info.ip, &mut *addr_of_mut!(IP_BUF)) };
                console_log("\x1b[1;35m|\x1b[0m  Network:      \x1b[1;32m* Online\x1b[0m  IP: \x1b[1;97m");
                unsafe { print_bytes(&(*addr_of!(IP_BUF))[..ip_len]) };
                console_log("\x1b[0m");
                pad_spaces(29 - ip_len.min(29));
                console_log("\x1b[1;35m|\x1b[0m\n");
            } else {
                console_log("\x1b[1;35m|\x1b[0m  Network:      \x1b[1;32m* Online\x1b[0m                                    \x1b[1;35m|\x1b[0m\n");
            }
        } else {
            console_log("\x1b[1;35m|\x1b[0m  Network:      \x1b[1;31m* Offline\x1b[0m                                  \x1b[1;35m|\x1b[0m\n");
        }

        // Filesystem status
        if is_fs_available() {
            console_log("\x1b[1;35m|\x1b[0m  Filesystem:   \x1b[1;32m* Mounted\x1b[0m                                  \x1b[1;35m|\x1b[0m\n");
        } else {
            console_log("\x1b[1;35m|\x1b[0m  Filesystem:   \x1b[1;31m* Not mounted\x1b[0m                              \x1b[1;35m|\x1b[0m\n");
        }

        console_log("\x1b[1;35m|\x1b[0m                                                             \x1b[1;35m|\x1b[0m\n");

        // Memory
        if let Some(stats) = get_heap_stats() {
            let used_kb = stats.used_bytes / 1024;
            let total_kb = stats.total_bytes / 1024;
            console_log("\x1b[1;35m|\x1b[0m  Memory:       \x1b[1;97m");
            print_int(used_kb as i64);
            console_log(" / ");
            print_int(total_kb as i64);
            console_log(" KiB\x1b[0m");
            // Calculate padding
            let mem_len = digit_count(used_kb) + digit_count(total_kb) + 9; // " / " + " KiB"
            pad_spaces(44 - mem_len.min(44));
            console_log("\x1b[1;35m|\x1b[0m\n");
        }

        // Uptime
        console_log("\x1b[1;35m|\x1b[0m  Uptime:       \x1b[1;97m");
        print_int(uptime_sec);
        console_log(" seconds\x1b[0m");
        let uptime_len = digit_count(uptime_sec as u64) + 8; // " seconds"
        pad_spaces(44 - uptime_len.min(44));
        console_log("\x1b[1;35m|\x1b[0m\n");

        console_log("\x1b[1;35m+-------------------------------------------------------------+\x1b[0m\n");
        console_log("\n");
    }

    fn pad_spaces(count: usize) {
        for _ in 0..count {
            console_log(" ");
        }
    }

    fn digit_count(mut n: u64) -> usize {
        if n == 0 {
            return 1;
        }
        let mut count = 0;
        while n > 0 {
            count += 1;
            n /= 10;
        }
        count
    }

    fn print_bytes(bytes: &[u8]) {
        unsafe { mkfs::print(bytes.as_ptr(), bytes.len()) };
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}

