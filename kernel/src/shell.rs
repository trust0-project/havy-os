//! Shell Service
//!
//! Interactive command-line shell that runs as a schedulable process.
//! This allows the shell to run on any hart, making all harts equal.
//!
//! ## Design
//!
//! The shell is implemented as a high-priority daemon process that:
//! - Polls UART for input (non-blocking with yield)
//! - Processes commands when a full line is received
//! - Yields to the scheduler between iterations
//!
//! This cooperative design allows other processes to run even on single-hart systems.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::uart;
use crate::Spinlock;

// ═══════════════════════════════════════════════════════════════════════════════
// SHELL STATE
// ═══════════════════════════════════════════════════════════════════════════════

/// Shell input buffer size
const BUFFER_SIZE: usize = 128;

/// Command history size
const HISTORY_SIZE: usize = 16;

/// Shell state - protected by spinlock for cross-hart access
struct ShellState {
    /// Current input buffer
    buffer: [u8; BUFFER_SIZE],
    /// Current buffer length
    len: usize,
    
    /// Command history
    history: [[u8; BUFFER_SIZE]; HISTORY_SIZE],
    history_lens: [usize; HISTORY_SIZE],
    history_count: usize,
    history_pos: usize,
    browsing_history: bool,
    
    /// Last newline char for handling \r\n sequences
    last_newline: u8,
    
    /// Escape sequence state (0=normal, 1=got ESC, 2=got ESC[)
    esc_state: u8,
    
    /// Whether shell is initialized
    initialized: bool,
    
    /// Whether shell is in tail follow mode
    tail_follow_mode: bool,
    tail_follow_path: [u8; BUFFER_SIZE],
    tail_follow_path_len: usize,
    tail_follow_last_size: usize,
}

impl ShellState {
    const fn new() -> Self {
        Self {
            buffer: [0u8; BUFFER_SIZE],
            len: 0,
            history: [[0u8; BUFFER_SIZE]; HISTORY_SIZE],
            history_lens: [0; HISTORY_SIZE],
            history_count: 0,
            history_pos: 0,
            browsing_history: false,
            last_newline: 0,
            esc_state: 0,
            initialized: false,
            tail_follow_mode: false,
            tail_follow_path: [0u8; BUFFER_SIZE],
            tail_follow_path_len: 0,
            tail_follow_last_size: 0,
        }
    }
}

/// Global shell state
static SHELL_STATE: Spinlock<ShellState> = Spinlock::new(ShellState::new());

/// Shell PID (for process tracking)
static SHELL_PID: AtomicUsize = AtomicUsize::new(0);

// ═══════════════════════════════════════════════════════════════════════════════
// SHELL SERVICE ENTRY POINT
// ═══════════════════════════════════════════════════════════════════════════════

/// Shell service entry point
///
/// This is called by the scheduler as a daemon process.
/// It does one iteration of shell work and returns, allowing
/// other processes to run (cooperative multitasking).
pub fn shell_service() {
    let hart_id = crate::get_hart_id();
    
    // Initialize on first run
    {
        let mut state = SHELL_STATE.lock();
        if !state.initialized {
            state.initialized = true;
            drop(state);
            
            // Initialize shell components
            crate::cwd_init();
            
            // Store our PID
            if let Some(cpu) = crate::cpu::CPU_TABLE.get(hart_id) {
                if let Some(pid) = cpu.running_process() {
                    SHELL_PID.store(pid as usize, Ordering::Relaxed);
                }
            }
            
            // Print initial prompt
            print_prompt();
        }
    }
    
    // Do one iteration of shell work
    shell_tick();
}

/// One iteration of shell work
///
/// Polls UART for input and processes any available bytes.
/// Returns quickly to allow other processes to run.
fn shell_tick() {
    // Poll for input (non-blocking)
    // Process up to 16 bytes per tick to stay responsive
    for _ in 0..16 {
        if !uart::Console::is_rx_ready_public() {
            break;
        }
        
        let byte = uart::Console::new().read_byte();
        if byte == 0 {
            break;
        }
        
        process_input_byte(byte);
    }
    
    // In single-hart mode, we need to run daemon tasks cooperatively
    // In multi-hart mode, daemons run as separate processes on secondary harts
    let num_harts = crate::HARTS_ONLINE.load(core::sync::atomic::Ordering::Relaxed);
    if num_harts <= 1 {
        crate::init::klogd_tick();
        crate::init::sysmond_tick();
        // Tick network daemons in single-hart mode
        crate::netd::tick();
        crate::tcpd::tick();
        crate::httpd::tick();
    }
    
    // ALWAYS tick GPU UI on hart 0 regardless of SMP mode
    // The d1_touch device is only available on the main thread (hart 0),
    // so input polling must happen here, not on secondary harts
    crate::init::gpuid_tick();
    
    // Poll tail follow mode (always)
    crate::poll_tail_follow();
    
    // Poll network (always - smoltcp needs regular polling)
    crate::poll_network();
}

/// Process a single input byte
fn process_input_byte(byte: u8) {
    let mut state = SHELL_STATE.lock();
    
    // Handle Ctrl+C
    if byte == 0x03 {
        if state.tail_follow_mode {
            state.tail_follow_mode = false;
            drop(state);
            crate::TAIL_FOLLOW_STATE.lock().stop();
            uart::write_line("");
            uart::write_line("\x1b[2m--- tail -f stopped ---\x1b[0m");
            print_prompt();
            return;
        }
        drop(state);
        if crate::cancel_running_command() {
            print_prompt();
        }
        return;
    }
    
    // In follow mode, 'q' also exits
    if state.tail_follow_mode && (byte == b'q' || byte == b'Q') {
        state.tail_follow_mode = false;
        drop(state);
        crate::TAIL_FOLLOW_STATE.lock().stop();
        uart::write_line("");
        uart::write_line("\x1b[2m--- tail -f stopped ---\x1b[0m");
        print_prompt();
        return;
    }
    
    // Ignore other input while in follow mode
    if state.tail_follow_mode {
        return;
    }
    
    // Handle escape sequences
    if state.esc_state == 1 {
        if byte == b'[' {
            state.esc_state = 2;
            return;
        } else {
            state.esc_state = 0;
        }
    } else if state.esc_state == 2 {
        state.esc_state = 0;
        match byte {
            b'A' => {
                // Up arrow - history navigation
                handle_history_up(&mut state);
                return;
            }
            b'B' => {
                // Down arrow - history navigation
                handle_history_down(&mut state);
                return;
            }
            b'C' | b'D' => {
                // Right/Left arrow - ignore
                return;
            }
            _ => {
                return;
            }
        }
    }
    
    match byte {
        0x1b => {
            // ESC - start of escape sequence
            state.esc_state = 1;
        }
        b'\r' | b'\n' => {
            // Handle \r\n sequences
            if (state.last_newline == b'\r' && byte == b'\n')
                || (state.last_newline == b'\n' && byte == b'\r')
            {
                state.last_newline = 0;
                return;
            }
            state.last_newline = byte;
            drop(state);
            uart::write_line("");
            handle_enter();
        }
        8 | 0x7f => {
            // Backspace / Delete
            if state.len > 0 {
                state.len -= 1;
                uart::write_str("\u{8} \u{8}");
            }
        }
        b'\t' => {
            // Tab - autocomplete
            state.last_newline = 0;
            let len = state.len;
            let mut buffer = state.buffer;
            drop(state);
            let new_len = crate::handle_tab_completion(&mut buffer, len);
            let mut state = SHELL_STATE.lock();
            state.buffer = buffer;
            state.len = new_len;
        }
        _ => {
            // Regular character
            state.last_newline = 0;
            let current_len = state.len;
            if current_len < BUFFER_SIZE {
                state.buffer[current_len] = byte;
                state.len = current_len + 1;
                drop(state);
                uart::write_byte(byte);
            }
        }
    }
}

/// Handle Enter key - execute command
fn handle_enter() {
    let mut state = SHELL_STATE.lock();
    let len = state.len;
    let buffer = state.buffer;
    
    // Save to history if non-empty
    if len > 0 {
        let idx = state.history_count % HISTORY_SIZE;
        state.history[idx][..len].copy_from_slice(&buffer[..len]);
        state.history_lens[idx] = len;
        state.history_count += 1;
    }
    
    // Check for tail -f command
    if let Some((path, num_lines)) = crate::parse_tail_follow_command(&buffer[..len]) {
        let resolved = crate::resolve_path(&path);
        drop(state);
        
        let (success, initial_size) = crate::start_tail_follow(&resolved, num_lines);
        if success {
            let mut state = SHELL_STATE.lock();
            state.tail_follow_mode = true;
            drop(state);
            crate::TAIL_FOLLOW_STATE.lock().start(&resolved, initial_size);
        } else {
            print_prompt();
        }
    } else {
        // Execute command
        let mut count = 0;
        drop(state);
        crate::handle_line(&buffer, len, &mut count);
        print_prompt();
    }
    
    // Reset state for next command
    let mut state = SHELL_STATE.lock();
    state.len = 0;
    state.browsing_history = false;
    state.history_pos = 0;
}

/// Handle up arrow - navigate history up
fn handle_history_up(state: &mut ShellState) {
    if state.history_count == 0 {
        return;
    }
    
    let max_pos = if state.history_count < HISTORY_SIZE {
        state.history_count
    } else {
        HISTORY_SIZE
    };
    
    if state.history_pos < max_pos {
        if !state.browsing_history {
            state.browsing_history = true;
            state.history_pos = 0;
        }
        
        if state.history_pos < max_pos {
            // Clear current line
            clear_input_line(state.len);
            
            // Get command from history
            let idx = (state.history_count - 1 - state.history_pos) % HISTORY_SIZE;
            state.len = state.history_lens[idx];
            state.buffer[..state.len].copy_from_slice(&state.history[idx][..state.len]);
            
            // Display the command
            uart::write_bytes(&state.buffer[..state.len]);
            
            if state.history_pos + 1 < max_pos {
                state.history_pos += 1;
            }
        }
    }
}

/// Handle down arrow - navigate history down
fn handle_history_down(state: &mut ShellState) {
    if state.browsing_history && state.history_pos > 0 {
        state.history_pos -= 1;
        
        // Clear current line
        clear_input_line(state.len);
        
        if state.history_pos == 0 {
            // Back to empty line
            state.browsing_history = false;
            state.len = 0;
        } else {
            // Get command from history
            let idx = (state.history_count - state.history_pos) % HISTORY_SIZE;
            state.len = state.history_lens[idx];
            state.buffer[..state.len].copy_from_slice(&state.history[idx][..state.len]);
            
            uart::write_bytes(&state.buffer[..state.len]);
        }
    } else if state.browsing_history {
        clear_input_line(state.len);
        state.browsing_history = false;
        state.len = 0;
    }
}

/// Clear the current input line
fn clear_input_line(len: usize) {
    for _ in 0..len {
        uart::write_str("\u{8} \u{8}");
    }
}

/// Print the shell prompt
fn print_prompt() {
    let cwd = crate::get_cwd();
    uart::write_str("\x1b[1;32mbavy\x1b[0m:\x1b[1;34m");
    uart::write_str(&cwd);
    uart::write_str("\x1b[0m$ ");
}

// ═══════════════════════════════════════════════════════════════════════════════
// PUBLIC API
// ═══════════════════════════════════════════════════════════════════════════════

/// Get shell PID for process listings
pub fn get_shell_pid() -> Option<u32> {
    let pid = SHELL_PID.load(Ordering::Relaxed);
    if pid > 0 {
        Some(pid as u32)
    } else {
        None
    }
}

/// Check if shell is running
pub fn is_shell_running() -> bool {
    SHELL_STATE.lock().initialized
}
