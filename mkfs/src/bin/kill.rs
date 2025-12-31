// kill - Terminate a process by PID
//
// Usage:
//   kill <pid>     Terminate the process with the given PID
//   kill           Show usage information

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{argc, argv, console_log, kill_process, print_int, print, KillResult};

    static mut PID_BUF: [u8; 16] = [0u8; 16];

    fn parse_pid(bytes: &[u8]) -> Option<u32> {
        if bytes.is_empty() {
            return None;
        }
        let mut result: u32 = 0;
        for &c in bytes {
            if c < b'0' || c > b'9' {
                return None;
            }
            let digit = (c - b'0') as u32;
            result = result.checked_mul(10)?.checked_add(digit)?;
        }
        Some(result)
    }

    let arg_count = argc();
    
    if arg_count < 1 {
        console_log("Usage: kill <pid>\n");
        console_log("\n");
        console_log("Terminate a process by its PID.\n");
        console_log("Use 'ps' to list running processes.\n");
        return;
    }

    let len = unsafe {
        match argv(0, &mut *core::ptr::addr_of_mut!(PID_BUF)) {
            Some(l) => l,
            None => {
                console_log("\x1b[1;31mError:\x1b[0m Could not read PID argument\n");
                return;
            }
        }
    };

    let pid_bytes = unsafe { &(*core::ptr::addr_of!(PID_BUF))[..len] };
    let pid = match parse_pid(pid_bytes) {
        Some(p) => p,
        None => {
            console_log("\x1b[1;31mError:\x1b[0m Invalid PID: ");
            unsafe { print((*core::ptr::addr_of!(PID_BUF)).as_ptr(), len) };
            console_log("\n");
            return;
        }
    };

    if pid == 0 {
        console_log("\x1b[1;31mError:\x1b[0m Invalid PID: 0\n");
        return;
    }

    match kill_process(pid) {
        KillResult::Success => {
            console_log("\x1b[1;32m[OK]\x1b[0m Killed process ");
            print_int(pid as i64);
            console_log("\n");
        }
        KillResult::CannotKill => {
            console_log("\x1b[1;31mError:\x1b[0m Cannot kill init (PID 1)\n");
        }
        KillResult::NotFound => {
            console_log("\x1b[1;31mError:\x1b[0m Process ");
            print_int(pid as i64);
            console_log(" not found\n");
        }
        KillResult::InvalidPid => {
            console_log("\x1b[1;31mError:\x1b[0m Invalid PID\n");
        }
    }
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
