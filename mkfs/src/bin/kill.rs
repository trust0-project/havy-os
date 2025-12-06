// kill - Terminate a process by PID
//
// Usage:
//   kill <pid>     Terminate the process with the given PID
//   kill           Show usage information

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{argc, argv, console_log, kill_process, print_int, KillResult};

    #[no_mangle]
    pub extern "C" fn _start() {
        // Need at least one argument (the PID)
        if argc() < 2 {
            console_log("Usage: kill <pid>\n");
            console_log("\n");
            console_log("Terminate a process by its PID.\n");
            console_log("Use 'ps' to list running processes.\n");
            return;
        }

        // Get the PID argument
        let mut pid_buf = [0u8; 16];
        let Some(len) = argv(1, &mut pid_buf) else {
            console_log("\x1b[1;31mError:\x1b[0m Could not read PID argument\n");
            return;
        };

        // Parse the PID
        let pid_str = match core::str::from_utf8(&pid_buf[..len]) {
            Ok(s) => s.trim(),
            Err(_) => {
                console_log("\x1b[1;31mError:\x1b[0m Invalid PID format\n");
                return;
            }
        };

        let pid = match parse_pid(pid_str) {
            Some(p) => p,
            None => {
                console_log("\x1b[1;31mError:\x1b[0m Invalid PID: ");
                console_log(pid_str);
                console_log("\n");
                return;
            }
        };

        if pid == 0 {
            console_log("\x1b[1;31mError:\x1b[0m Invalid PID: 0\n");
            return;
        }

        // Attempt to kill the process
        match kill_process(pid) {
            KillResult::Success => {
                console_log("\x1b[1;32m✓\x1b[0m Killed process ");
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

    /// Parse a string as a positive integer PID
    fn parse_pid(s: &str) -> Option<u32> {
        if s.is_empty() {
            return None;
        }

        let mut result: u32 = 0;
        for c in s.bytes() {
            if c < b'0' || c > b'9' {
                return None;
            }
            let digit = (c - b'0') as u32;
            result = result.checked_mul(10)?.checked_add(digit)?;
        }
        Some(result)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}

