// kernel/src/scripting.rs
//! Script discovery for native RISC-V binaries in /usr/bin/
//!
//! This module provides script lookup functionality for the shell.
//! Scripts are native ELF binaries located in /usr/bin/ directory.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::{clint::get_time_ms, device::uart, lock::utils::{OUTPUT_BUFFER_SIZE, OUTPUT_CAPTURE, SHELL_CMD_STATE}, scripting, wasm};

/// Flag indicating we're running from GUI context (need S-mode execution)
static GUI_CONTEXT: AtomicBool = AtomicBool::new(false);

/// Set GUI context mode - commands will use S-mode execution that returns normally
pub fn set_gui_context(enabled: bool) {
    GUI_CONTEXT.store(enabled, Ordering::SeqCst);
}

/// Check if we're in GUI context
pub fn is_gui_context() -> bool {
    GUI_CONTEXT.load(Ordering::SeqCst)
}
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
/// Uses fs_proxy for hart-aware filesystem access - works on any hart.
pub fn find_script(cmd: &str) -> Option<Vec<u8>> {
    use crate::cpu::fs_proxy;
    
    // If command contains '/', treat as path
    if cmd.contains('/') {
        let full_path = if cmd.starts_with('/') {
            String::from(cmd)
        } else {
            crate::resolve_path(cmd)
        };
        return fs_proxy::fs_read(&full_path);
    }

    // Search /usr/bin/ first
    let usr_bin_path = format!("/usr/bin/{}", cmd);
    if let Some(content) = fs_proxy::fs_read(&usr_bin_path) {
        return Some(content);
    }

    // Search root as fallback
    fs_proxy::fs_read(cmd)
}


/// Run a script from its bytes
/// 
/// Supports both native RISC-V ELF binaries (preferred) and WASM binaries (legacy).
pub fn run_script_bytes(bytes: &[u8], args: &str) {
    use core::arch::asm;
    
    // CRITICAL: Capture return frame at ABSOLUTE FUNCTION START
    // before ANY function calls (is_elf, load_elf, split, collect, etc.)
    let caller_ra: u64;
    let caller_sp: u64;
    unsafe {
        asm!(
            "mv {ra}, ra",
            "mv {sp}, sp",
            ra = out(reg) caller_ra,
            sp = out(reg) caller_sp,
        );
    }
    
    // Detect ELF magic (0x7f 'E' 'L' 'F') - native RISC-V binary (preferred)
    if crate::elf_loader::is_elf(bytes) {
        match crate::elf_loader::load_elf(bytes) {
            Ok(loaded) => {
                // Split args into vector
                let args_vec: Vec<&str> = args.split_whitespace().collect();
                
                // Always use U-mode execution (proper RISC-V spec compliance)
                // The gui_cmd process handles GUI execution, shell uses shelld
                // Both use the same execute_elf path - the difference is in how
                // restore_kernel_context handles the exit (via gui_mode flag)
                let exit_code = crate::elf_loader::execute_elf(&loaded, &args_vec, caller_ra, caller_sp);
                
                if exit_code != 0 {
                    out_str("\x1b[1;31mExited with code:\x1b[0m ");
                    out_line(&alloc::format!("{}", exit_code));
                }
            }
            Err(e) => {
                out_str("\x1b[1;31mELF load error:\x1b[0m ");
                out_line(&alloc::format!("{:?}", e));
            }
        }
        return;
    }
    
    // Detect WASM magic (\0asm) - legacy fallback
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

    // Not a recognized binary format
    out_line("\x1b[1;31mError:\x1b[0m Not a valid binary (expected ELF or WASM)");
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