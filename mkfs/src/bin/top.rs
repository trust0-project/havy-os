// top - System monitor (simplified)
//
// Usage:
//   top          Display running processes

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{console_log, get_time, print_int, ps_list, print};

    static mut BUF: [u8; 2048] = [0u8; 2048];

    let uptime_ms = get_time();
    let uptime_sec = uptime_ms / 1000;

    console_log("\x1b[2J\x1b[H"); // Clear screen

    console_log("\x1b[7m top - BAVY OS System Monitor \x1b[0m\n");
    console_log("Uptime: ");
    print_int(uptime_sec);
    console_log("s\n\n");

    console_log("\x1b[1;36m  PID  STATE  PRI  CPU TIME  UPTIME  NAME\x1b[0m\n");
    console_log("\x1b[90m-------------------------------------------------------\x1b[0m\n");

    let len = unsafe { ps_list((*core::ptr::addr_of_mut!(BUF)).as_mut_ptr(), 2048) };
    
    if len < 0 {
        console_log("\x1b[1;31mError:\x1b[0m Failed to get process list\n");
        return;
    }

    if len > 0 {
        let data = unsafe { &(*core::ptr::addr_of!(BUF))[..len as usize] };
        print(data.as_ptr(), data.len());
    }

    console_log("\n\x1b[90mPress Ctrl+C to exit\x1b[0m\n");
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
