//! GUI Command Process Service
//!
//! This module provides async command execution for the GUI terminal.
//! Commands are queued and executed in a separate process in U-mode,
//! with results polled by the gpuid service.

use alloc::string::String;
use alloc::vec::Vec;
use crate::lock::Spinlock;
use core::sync::atomic::{AtomicBool, Ordering};

/// Maximum output buffer size
const GUI_CMD_OUTPUT_MAX: usize = 8192;

/// Pending GUI command to execute
pub struct GuiCmd {
    pub cmd: String,
    pub args: String,
}

/// Result of GUI command execution
pub struct GuiCmdResult {
    pub exit_code: i32,
    pub output: Vec<u8>,
}

/// Pending command queue (single command at a time)
static GUI_CMD_PENDING: Spinlock<Option<GuiCmd>> = Spinlock::new(None);

/// Completed command result
static GUI_CMD_RESULT: Spinlock<Option<GuiCmdResult>> = Spinlock::new(None);

/// Flag indicating a command is currently executing
pub static GUI_CMD_RUNNING: AtomicBool = AtomicBool::new(false);

/// Flag to signal the gui_cmd process that work is available
static GUI_CMD_WORK_AVAILABLE: AtomicBool = AtomicBool::new(false);

/// Output capture buffer for GUI commands
static GUI_OUTPUT_BUFFER: Spinlock<GuiOutputBuffer> = Spinlock::new(GuiOutputBuffer::new());

struct GuiOutputBuffer {
    buffer: [u8; GUI_CMD_OUTPUT_MAX],
    len: usize,
    capturing: bool,
}

impl GuiOutputBuffer {
    const fn new() -> Self {
        Self {
            buffer: [0u8; GUI_CMD_OUTPUT_MAX],
            len: 0,
            capturing: false,
        }
    }
    
    fn start_capture(&mut self) {
        self.len = 0;
        self.capturing = true;
    }
    
    fn stop_capture(&mut self) -> Vec<u8> {
        self.capturing = false;
        Vec::from(&self.buffer[..self.len])
    }
    
    fn write(&mut self, data: &[u8]) {
        if !self.capturing {
            return;
        }
        for &b in data {
            if self.len < GUI_CMD_OUTPUT_MAX {
                self.buffer[self.len] = b;
                self.len += 1;
            }
        }
    }
}

/// Queue a command for async execution
/// Returns false if a command is already running
pub fn queue_command(cmd: &str, args: &str) -> bool {
    if GUI_CMD_RUNNING.load(Ordering::SeqCst) {
        return false; // Already running a command
    }
    
    // Clear any previous result
    {
        let mut result = GUI_CMD_RESULT.lock();
        *result = None;
    }
    
    // Queue the new command
    {
        let mut pending = GUI_CMD_PENDING.lock();
        *pending = Some(GuiCmd {
            cmd: String::from(cmd),
            args: String::from(args),
        });
    }
    
    // Signal work available
    GUI_CMD_WORK_AVAILABLE.store(true, Ordering::SeqCst);
    
    true
}

/// Check if a command is currently running
pub fn is_running() -> bool {
    GUI_CMD_RUNNING.load(Ordering::SeqCst)
}

/// Check if result is ready (non-blocking)
/// Returns the result and clears it
pub fn poll_result() -> Option<GuiCmdResult> {
    let mut result = GUI_CMD_RESULT.lock();
    result.take()
}

/// Signal completion from ELF exit handler
/// Called by restore_kernel_context when a GUI command process exits
pub fn signal_completion(exit_code: i32) {
    use crate::lock::utils::OUTPUT_CAPTURE;
    use crate::lock::state::output::OUTPUT_BUFFER_SIZE;
    use crate::device::uart::write_line;
    
    write_line(&alloc::format!("[GUI_CMD] signal_completion called, exit_code={}", exit_code));
    
    // Capture the output from the shared OUTPUT_CAPTURE mechanism
    let output = {
        let mut cap = OUTPUT_CAPTURE.lock();
        cap.capturing = false;
        let len = cap.len.min(OUTPUT_BUFFER_SIZE);
        write_line(&alloc::format!("[GUI_CMD] Captured {} bytes from OUTPUT_CAPTURE", len));
        Vec::from(&cap.buffer[..len])
    };
    
    // Store result
    {
        let mut result = GUI_CMD_RESULT.lock();
        *result = Some(GuiCmdResult {
            exit_code,
            output,
        });
    }
    
    write_line("[GUI_CMD] Result stored, clearing running flag");
    
    // Clear running flag
    GUI_CMD_RUNNING.store(false, Ordering::SeqCst);
    
    // Also clear GUI context
    crate::scripting::set_gui_context(false);
    
    write_line("[GUI_CMD] signal_completion done");
}

/// Write to GUI output buffer (called by syscall output functions)
pub fn write_output(data: &[u8]) {
    let mut buf = GUI_OUTPUT_BUFFER.lock();
    buf.write(data);
}

/// Check if GUI output capture is active
pub fn is_capturing() -> bool {
    let buf = GUI_OUTPUT_BUFFER.lock();
    buf.capturing
}

/// GUI command process tick function
/// This runs as a daemon process that checks for pending commands each tick.
/// It must return quickly to allow other processes to run.
pub fn gui_cmd_service() {
    use crate::device::uart::write_line;
    
    // Check if work is available
    if !GUI_CMD_WORK_AVAILABLE.load(Ordering::SeqCst) {
        return; // No work - yield to scheduler
    }
    
    write_line("[GUI_CMD] work available - processing");
    
    // Clear work flag
    GUI_CMD_WORK_AVAILABLE.store(false, Ordering::SeqCst);
    
    // Get the pending command
    let cmd_opt = {
        let mut pending = GUI_CMD_PENDING.lock();
        pending.take()
    };
    
    let Some(cmd) = cmd_opt else {
        write_line("[GUI_CMD] no pending command - returning");
        return;
    };
    
    write_line(&alloc::format!("[GUI_CMD] executing: '{}'", cmd.cmd));
    
    // Mark as running
    GUI_CMD_RUNNING.store(true, Ordering::SeqCst);
    
    // Start output capture using the shared OUTPUT_CAPTURE mechanism
    {
        use crate::lock::utils::OUTPUT_CAPTURE;
        let mut cap = OUTPUT_CAPTURE.lock();
        cap.capturing = true;
        cap.len = 0;
    }
    
    write_line("[GUI_CMD] calling execute_command");
    
    // Execute the command
    // This will trigger sret to U-mode, and on exit SYS_EXIT will call
    // signal_completion() via restore_kernel_context()
    crate::scripting::execute_command(cmd.cmd.as_bytes(), cmd.args.as_bytes());
    
    // Note: For U-mode execution, control flow does NOT return here.
    // The command exits via SYS_EXIT -> trap_handler -> restore_kernel_context
    // which calls signal_completion() and re-enters hart_loop.
    //
    // For S-mode execution (fallback), control returns here normally.
    // In that case, we need to manually signal completion:
    if GUI_CMD_RUNNING.load(Ordering::SeqCst) {
        use crate::device::uart::write_line;
        write_line("[GUI_CMD] S-mode fallback - command returned");
        
        // Still running means S-mode returned normally - capture output
        use crate::lock::utils::OUTPUT_CAPTURE;
        use crate::lock::state::output::OUTPUT_BUFFER_SIZE;
        let output = {
            let mut cap = OUTPUT_CAPTURE.lock();
            cap.capturing = false;
            let len = cap.len.min(OUTPUT_BUFFER_SIZE);
            write_line(&alloc::format!("[GUI_CMD] Captured {} bytes", len));
            Vec::from(&cap.buffer[..len])
        };
        signal_completion_with_output(0, output);
        write_line("[GUI_CMD] signal_completion_with_output called");
    }
}

/// Signal completion with captured output (for S-mode fallback)
pub fn signal_completion_with_output(exit_code: i32, output: Vec<u8>) {
    // Store result
    {
        let mut result = GUI_CMD_RESULT.lock();
        *result = Some(GuiCmdResult {
            exit_code,
            output,
        });
    }
    
    // Clear running flag
    GUI_CMD_RUNNING.store(false, Ordering::SeqCst);
}

