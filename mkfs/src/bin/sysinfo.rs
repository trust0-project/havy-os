// sysinfo - Display system information
//
// Usage:
//   sysinfo      Display system information

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{console_log, get_time, is_net_available, print_int};

    let uptime_ms = get_time();
    let uptime_sec = uptime_ms / 1000;

    console_log("\n");
    console_log("\x1b[1;35m+-------------------------------------------------------------+\x1b[0m\n");
    console_log("\x1b[1;35m|\x1b[0m              \x1b[1;97mBAVY OS System Information\x1b[0m                     \x1b[1;35m|\x1b[0m\n");
    console_log("\x1b[1;35m+-------------------------------------------------------------+\x1b[0m\n");

    console_log("\x1b[1;35m|\x1b[0m  Kernel:       \x1b[1;97mBAVY OS v0.1.0\x1b[0m                              \x1b[1;35m|\x1b[0m\n");
    console_log("\x1b[1;35m|\x1b[0m  Architecture: \x1b[1;97mRISC-V 64-bit (RV64GC)\x1b[0m                       \x1b[1;35m|\x1b[0m\n");
    console_log("\x1b[1;35m|\x1b[0m  Mode:         \x1b[1;97mSupervisor Mode (S-Mode)\x1b[0m                     \x1b[1;35m|\x1b[0m\n");
    console_log("\x1b[1;35m|\x1b[0m  Runtime:      \x1b[1;97mNative RISC-V ELF\x1b[0m                            \x1b[1;35m|\x1b[0m\n");
    console_log("\x1b[1;35m|\x1b[0m                                                             \x1b[1;35m|\x1b[0m\n");

    // Network status
    if is_net_available() {
        console_log("\x1b[1;35m|\x1b[0m  Network:      \x1b[1;32m* Online\x1b[0m                                    \x1b[1;35m|\x1b[0m\n");
    } else {
        console_log("\x1b[1;35m|\x1b[0m  Network:      \x1b[1;31m* Offline\x1b[0m                                   \x1b[1;35m|\x1b[0m\n");
    }

    console_log("\x1b[1;35m|\x1b[0m  Filesystem:   \x1b[1;32m* Mounted (SFS)\x1b[0m                             \x1b[1;35m|\x1b[0m\n");
    console_log("\x1b[1;35m|\x1b[0m                                                             \x1b[1;35m|\x1b[0m\n");

    // Uptime
    console_log("\x1b[1;35m|\x1b[0m  Uptime:       \x1b[1;97m");
    print_int(uptime_sec);
    console_log(" seconds\x1b[0m                                  \x1b[1;35m|\x1b[0m\n");

    console_log("\x1b[1;35m+-------------------------------------------------------------+\x1b[0m\n");
    console_log("\n");
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
