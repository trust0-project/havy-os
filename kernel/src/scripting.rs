// kernel/src/scripting.rs
//! Script discovery for WASM binaries in /usr/bin/
//!
//! This module provides script lookup functionality for the shell.
//! Scripts are WASM binaries located in /usr/bin/ directory.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::{   clint::get_time_ms, device::uart, lock::utils::{OUTPUT_BUFFER_SIZE, OUTPUT_CAPTURE, SHELL_CMD_STATE}, scripting, wasm};
/// Initialize shell command tracking
fn shell_cmd_init() {
    let mut state = SHELL_CMD_STATE.lock();
    state.session_start = get_time_ms() as u64;
}

/// Start tracking a shell command
pub fn shell_cmd_start(cmd_name: &str) {
    let mut state = SHELL_CMD_STATE.lock();
    state.start_command(cmd_name, get_time_ms() as u64);
}

/// Stop tracking the current shell command
pub fn shell_cmd_end() {
    let mut state = SHELL_CMD_STATE.lock();
    state.end_command(get_time_ms() as u64);
}

/// Write a string - respects capture mode
pub fn out_str(s: &str) {
    let mut cap = OUTPUT_CAPTURE.lock();
    if cap.capturing {
        for &b in s.as_bytes() {
            let idx = cap.len;
            if idx < OUTPUT_BUFFER_SIZE {
                cap.buffer[idx] = b;
                cap.len += 1;
            }
        }
    } else {
        drop(cap); // Release lock before UART
        uart::write_str(s);
    }
}




/// Write a string with newline - respects capture mode
fn out_line(s: &str) {
    out_str(s);
    out_str("\n");
}

/// Find a script/binary by name
/// 
/// Search order:
/// 1. If path contains '/', resolve as absolute or relative path
/// 2. Search /usr/bin/<name>
/// 3. Search root /<name>
/// 
/// Uses non-blocking lock acquisition with retry to avoid deadlock with daemons.
pub fn find_script(cmd: &str) -> Option<Vec<u8>> {
    let start = crate::get_time_ms();
    let timeout_ms = 5000; // 5 second timeout
    
    loop {
        let elapsed = crate::get_time_ms() - start;
        
        // Check timeout
        if elapsed > timeout_ms {
            return None;
        }
        
        // Try to acquire read lock on filesystem (non-blocking)
        let fs_guard = match crate::FS_STATE.try_read() {
            Some(guard) => guard,
            None => {
                // Filesystem busy, yield briefly
                for _ in 0..1000 {
                    core::hint::spin_loop();
                }
                continue;
            }
        };
        
        // Try to acquire write lock on block device (needed for I/O)
        let mut blk_guard = match crate::lock::utils::BLK_DEV.try_write() {
            Some(guard) => guard,
            None => {
                // Block device busy, release FS lock and retry
                drop(fs_guard);
                for _ in 0..1000 {
                    core::hint::spin_loop();
                }
                continue;
            }
        };

        // Got both locks, do the search
        if let (Some(fs), Some(dev)) = (fs_guard.as_ref(), blk_guard.as_mut()) {
            // If command contains '/', treat as path
            if cmd.contains('/') {
                let full_path = if cmd.starts_with('/') {
                    String::from(cmd)
                } else {
                    crate::resolve_path(cmd)
                };

                if let Some(content) = fs.read_file(dev, &full_path) {
                    return Some(content);
                }
                return None;
            }

            // Search /usr/bin/ first
            let usr_bin_path = format!("/usr/bin/{}", cmd);
            if let Some(content) = fs.read_file(dev, &usr_bin_path) {
                return Some(content);
            }

            // Search root as fallback
            if let Some(content) = fs.read_file(dev, cmd) {
                return Some(content);
            }
            
            // Not found
            return None;
        } else {
            // FS or BLK is None - not initialized
            return None;
        }
    }
}


/// Run a script from its bytes (WASM only)
pub fn run_script_bytes(bytes: &[u8], args: &str) {
    // Detect \0asm magic header for WASM binaries
    if bytes.len() >= 4
        && bytes[0] == 0x00
        && bytes[1] == 0x61
        && bytes[2] == 0x73
        && bytes[3] == 0x6D
    {
        let args_vec: Vec<&str> = args.split_whitespace().collect();
        if let Err(e) = wasm::execute(bytes, &args_vec) {
            out_str("\x1b[1;31mError:\x1b[0m ");
            out_line(&e);
        }
        return;
    }

    // Not a WASM binary
    out_line("\x1b[1;31mError:\x1b[0m Not a valid binary");
}


/// Execute a command (separated for cleaner redirection handling)
///
/// Commands are resolved in this order:
/// 1. Essential built-in commands (that require direct kernel access)
/// 2. Native commands (fast Rust implementations of common utilities)
/// 3. Scripts: searched in root, then /usr/bin/ directory (PATH-like)
pub fn execute_command(cmd: &[u8], args: &[u8]) {
    let cmd_str = core::str::from_utf8(cmd).unwrap_or("");
    let args_str = core::str::from_utf8(args).unwrap_or("");

    
    // =============================================================================
    // SCRIPT RESOLUTION (PATH-like)
    // Fallback to script-based commands for flexibility/customization
    // =============================================================================
    if let Some(script_bytes) = scripting::find_script(cmd_str) {
        // Track command CPU time
        shell_cmd_start(cmd_str);
        run_script_bytes(&script_bytes, args_str);
        shell_cmd_end();
        return;
    }

    // =============================================================================
    // COMMAND NOT FOUND
    // =============================================================================
    out_str("\x1b[1;31mCommand not found:\x1b[0m ");
    out_line(cmd_str);
    out_line("\x1b[0;90mTry 'help' for available commands, or check /usr/bin/ for scripts\x1b[0m");
}