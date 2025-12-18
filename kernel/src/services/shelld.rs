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

use alloc::string::String;
use alloc::vec::Vec;

use crate::PING_STATE;
use crate::lock::utils::BLK_DEV;
use crate::lock::utils::COMMAND_RUNNING;
use crate::lock::utils::FS_STATE;
use crate::lock::utils::TAIL_FOLLOW_STATE;
use crate::net;
use crate::services::netd;
use crate::uart;
use crate::Spinlock;
use crate::utils::poll_tail_follow;
use crate::utils::print_prompt;
use crate::utils::resolve_path;

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

/// Find common prefix among strings
fn find_common_prefix(strings: &[alloc::string::String]) -> alloc::string::String {
    use alloc::string::String;

    if strings.is_empty() {
        return String::new();
    }

    let first = &strings[0];
    let mut prefix_len = first.len();

    for s in strings.iter().skip(1) {
        let mut common = 0;
        for (a, b) in first.chars().zip(s.chars()) {
            if a == b && common < prefix_len {
                common += 1;
            } else {
                break;
            }
        }
        prefix_len = common;
    }

    String::from(&first[..prefix_len])
}




 /// Handle tab completion
/// Returns the new buffer length after completion
pub fn handle_tab_completion(buffer: &mut [u8], len: usize) -> usize {
    use alloc::string::String;
    use alloc::vec::Vec;

    if len == 0 {
        return 0;
    }

    let input = match core::str::from_utf8(&buffer[..len]) {
        Ok(s) => s,
        Err(_) => return len,
    };

    // Find the word being completed (last space-separated token)
    let last_space = input.rfind(' ');
    let (prefix, word_to_complete) = match last_space {
        Some(pos) => (&input[..=pos], &input[pos + 1..]),
        None => ("", input),
    };

    let is_command = prefix.is_empty();

    let mut matches: Vec<String> = Vec::new();

    if is_command {
        // Complete commands - check built-ins first
        let builtins = [
            "clear", "pwd", "ping", "nslookup", "node", "help", "ls", "cat",
            "echo", "cowsay", "sysinfo", "ip", "netstat", "memstats", "uptime", "write", "wget", "cd",
            "shutdown",
        ];

        for cmd in builtins.iter() {
            if cmd.starts_with(word_to_complete) {
                matches.push(String::from(*cmd));
            }
        }

        // Also check /usr/bin/ for scripts
        {
            let mut fs_guard = FS_STATE.write();
            let mut blk_guard = BLK_DEV.write();
            if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
                let files = fs.list_dir(dev, "/");
                for f in files {
                    if f.name.starts_with("/usr/bin/") {
                        let script_name = &f.name[9..]; // Strip "/usr/bin/"
                        if script_name.starts_with(word_to_complete) {
                            // Avoid duplicates with builtins
                            if !matches.iter().any(|m| m == script_name) {
                                matches.push(String::from(script_name));
                            }
                        }
                    }
                }
            }
        }
    } else {
        // Complete file/directory paths
        let path_to_complete = if word_to_complete.starts_with('/') {
            String::from(word_to_complete)
        } else {
            resolve_path(word_to_complete)
        };

        // Find the directory part and file prefix
        let (dir_path, file_prefix) = if let Some(last_slash) = path_to_complete.rfind('/') {
            if last_slash == 0 {
                ("/", &path_to_complete[1..])
            } else {
                (
                    &path_to_complete[..last_slash],
                    &path_to_complete[last_slash + 1..],
                )
            }
        } else {
            ("/", path_to_complete.as_str())
        };

        {
            let mut fs_guard = FS_STATE.write();
            let mut blk_guard = BLK_DEV.write();
            if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
                let files = fs.list_dir(dev, "/");
                let mut seen_dirs: Vec<String> = Vec::new();

                for f in files {
                    // Check if file is in the target directory
                    let check_prefix = if dir_path == "/" { "/" } else { dir_path };

                    if !f.name.starts_with(check_prefix) {
                        continue;
                    }

                    // Get the part after the directory
                    let relative = if dir_path == "/" {
                        &f.name[1..]
                    } else if f.name.len() > check_prefix.len() + 1 {
                        &f.name[check_prefix.len() + 1..]
                    } else {
                        continue;
                    };

                    // Get just the immediate child (first path component)
                    let child_name = if let Some(slash_pos) = relative.find('/') {
                        &relative[..slash_pos]
                    } else {
                        relative
                    };

                    if child_name.is_empty() {
                        continue;
                    }

                    // Check if it matches the prefix
                    if !child_name.starts_with(file_prefix) {
                        continue;
                    }

                    // Check if this is a directory (has more path after)
                    let is_dir = relative.len() > child_name.len();

                    let completion = if is_dir {
                        let dir_name = String::from(child_name) + "/";
                        if seen_dirs.contains(&dir_name) {
                            continue;
                        }
                        seen_dirs.push(dir_name.clone());
                        dir_name
                    } else {
                        String::from(child_name)
                    };

                    if !matches.iter().any(|m| m == &completion) {
                        matches.push(completion);
                    }
                }
            }
        }
    }

    matches.sort();

    if matches.is_empty() {
        // No matches - beep or do nothing
        return len;
    }

    if matches.len() == 1 {
        // Single match - complete it
        let completion = &matches[0];
        let to_add = &completion[word_to_complete.len()..];

        // Add completion to buffer
        let new_len = len + to_add.len();
        if new_len <= buffer.len() {
            for (i, b) in to_add.bytes().enumerate() {
                buffer[len + i] = b;
            }
            uart::write_str(to_add);

            // Add space after command completion (not for paths ending in /)
            if is_command && new_len + 1 <= buffer.len() {
                buffer[new_len] = b' ';
                uart::write_str(" ");
                return new_len + 1;
            }

            return new_len;
        }
        return len;
    }

    // Multiple matches - find common prefix and show options
    let common = find_common_prefix(&matches);

    if common.len() > word_to_complete.len() {
        // Complete up to common prefix
        let to_add = &common[word_to_complete.len()..];
        let new_len = len + to_add.len();
        if new_len <= buffer.len() {
            for (i, b) in to_add.bytes().enumerate() {
                buffer[len + i] = b;
            }
            uart::write_str(to_add);
            return new_len;
        }
        return len;
    }

    // Show all matches
    uart::write_line("");
    let mut col = 0;
    let col_width = 16;
    let num_cols = 4;

    for m in &matches {
        let display_len = m.len();
        uart::write_str(m);

        col += 1;
        if col >= num_cols {
            uart::write_line("");
            col = 0;
        } else {
            // Pad to column width
            for _ in display_len..col_width {
                uart::write_str(" ");
            }
        }
    }
    if col > 0 {
        uart::write_line("");
    }

    // Redraw prompt and current input
    print_prompt();
    uart::write_bytes(&buffer[..len]);

    len
}


/// Shell service entry point
///
/// This is called by the scheduler as a daemon process.
/// It does one iteration of shell work and returns, allowing
/// other processes to run (cooperative multitasking).
pub fn shell_service() {
    shell_tick();
   
    let hart_id = crate::get_hart_id();
    
    // Initialize on first run
    {
        let mut state = SHELL_STATE.lock();
        if !state.initialized {
            state.initialized = true;
            drop(state);
            
            // Initialize shell components
            crate::utils::cwd_init();
             // Print initial prompt
             print_prompt();
            
            // Store our PID
            if let Some(cpu) = crate::cpu::CPU_TABLE.get(hart_id) {
                if let Some(pid) = cpu.running_process() {
                    SHELL_PID.store(pid as usize, Ordering::Relaxed);
                }
            }
            
           
        }
    }
    
    // Do one iteration of shell work
    shell_tick();
}

/// One iteration of shell work
///
/// Polls UART for input and processes any available bytes.
/// Returns quickly to allow other processes to run.
pub fn shell_tick() {
    // Poll for input (non-blocking)
    // Process up to 64 bytes per tick for faster input response
    for _ in 0..64 {
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

    
    // Poll tail follow mode (always)
    poll_tail_follow();
    
    // NOTE: poll_network() removed - it acquires NET_STATE/PING_STATE locks
    // which blocks shell input when other harts hold these locks.
    // Network polling happens in netd::tick() called from hart_loop.
}

/// Process a single input byte
fn process_input_byte(byte: u8) {
    let mut state = SHELL_STATE.lock();
    
    // Handle Ctrl+C
    if byte == 0x03 {
        if state.tail_follow_mode {
            state.tail_follow_mode = false;
            drop(state);
            TAIL_FOLLOW_STATE.lock().stop();
            uart::write_line("");
            uart::write_line("\x1b[2m--- tail -f stopped ---\x1b[0m");
            print_prompt();
            return;
        }
        drop(state);
        if cancel_running_command() {
            print_prompt();
        }
        return;
    }
    
    // In follow mode, 'q' also exits
    if state.tail_follow_mode && (byte == b'q' || byte == b'Q') {
        state.tail_follow_mode = false;
        drop(state);
        TAIL_FOLLOW_STATE.lock().stop();
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
            let new_len = handle_tab_completion(&mut buffer, len);
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


/// Parse a command to see if it's a tail -f command
/// Returns Some((filepath, num_lines)) if it's a follow command, None otherwise
pub fn parse_tail_follow_command(cmd: &[u8]) -> Option<(String, usize)> {
    let cmd_str = core::str::from_utf8(cmd).ok()?;
    let cmd_str = cmd_str.trim();

    // Must start with "tail"
    if !cmd_str.starts_with("tail ") && cmd_str != "tail" {
        return None;
    }

    // Parse arguments
    let parts: Vec<&str> = cmd_str.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }

    let mut has_follow = false;
    let mut num_lines: usize = 10;
    let mut filepath: Option<&str> = None;

    let mut i = 1;
    while i < parts.len() {
        let part = parts[i];

        if part == "-f" || part == "--follow" {
            has_follow = true;
        } else if part.starts_with("-f") && part.len() > 2 {
            // -f is in combined flags like -fn20 or just -f alone
            has_follow = true;
        } else if part.starts_with("--follow=") {
            has_follow = true;
        } else if part == "-n" {
            // Next arg is number of lines
            if i + 1 < parts.len() {
                i += 1;
                if let Ok(n) = parts[i].parse::<usize>() {
                    num_lines = n;
                }
            }
        } else if part.starts_with("-n") {
            // -nNUM format
            if let Ok(n) = part[2..].parse::<usize>() {
                num_lines = n;
            }
        } else if part.starts_with("--lines=") {
            if let Ok(n) = part[8..].parse::<usize>() {
                num_lines = n;
            }
        } else if !part.starts_with("-") {
            // It's the filepath
            filepath = Some(part);
        }

        i += 1;
    }

    // Must have -f flag and a file path
    if has_follow {
        if let Some(path) = filepath {
            return Some((String::from(path), num_lines));
        }
    }

    None
}


/// Start tail follow mode for a file
/// Returns (success, initial_size)
pub fn start_tail_follow(path: &str, num_lines: usize) -> (bool, usize) {
    let mut fs_guard = FS_STATE.write();
    let mut blk_guard = BLK_DEV.write();

    if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
        if let Some(content) = fs.read_file(dev, path) {
            // Show last N lines
            if let Ok(text) = core::str::from_utf8(&content) {
                let lines: Vec<&str> = text.lines().collect();
                let start = if lines.len() > num_lines {
                    lines.len() - num_lines
                } else {
                    0
                };

                for i in start..lines.len() {
                    uart::write_line(lines[i]);
                }
            }

            uart::write_line("");
            uart::write_line("\x1b[2m--- Following (Ctrl+C or 'q' to stop) ---\x1b[0m");

            return (true, content.len());
        } else {
            uart::write_str("\x1b[1;31mtail: cannot open '");
            uart::write_str(path);
            uart::write_line("': No such file\x1b[0m");
        }
    } else {
        uart::write_line("\x1b[1;31mtail: filesystem not available\x1b[0m");
    }

    (false, 0)
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
    if let Some((path, num_lines)) = parse_tail_follow_command(&buffer[..len]) {
        let resolved = crate::resolve_path(&path);
        drop(state);
        
        let (success, initial_size) = start_tail_follow(&resolved, num_lines);
        if success {
            let mut state = SHELL_STATE.lock();
            state.tail_follow_mode = true;
            drop(state);
            crate::lock::utils::TAIL_FOLLOW_STATE.lock().start(&resolved, initial_size);
        } else {
            print_prompt();
        }
    } else {
        // Execute command
        let mut count = 0;
        drop(state);
        uart::handle_line(&buffer, len, &mut count);
        print_prompt();
    }
    
    // Reset state for next command
    let mut state = SHELL_STATE.lock();
    state.len = 0;
    state.browsing_history = false;
    state.history_pos = 0;
}

/// Clear the current input line on the terminal
fn clear_input_line(len: usize) {
    // Move cursor back and clear each character
    for _ in 0..len {
        uart::write_str("\u{8} \u{8}");
    }
}



/// Print ping statistics summary (like Linux ping)
fn print_ping_statistics() {
    let ping_guard = PING_STATE.lock();
    if let Some(ref ping) = *ping_guard {
        let mut ip_buf = [0u8; 16];
        let ip_len = net::format_ipv4(ping.target, &mut ip_buf);

        uart::write_line("");
        uart::write_str("--- ");
        uart::write_bytes(&ip_buf[..ip_len]);
        uart::write_line(" ping statistics ---");

        uart::write_u64(ping.packets_sent as u64);
        uart::write_str(" packets transmitted, ");
        uart::write_u64(ping.packets_received as u64);
        uart::write_str(" received, ");
        uart::write_u64(ping.packet_loss_percent() as u64);
        uart::write_line("% packet loss");

        if ping.packets_received > 0 {
            uart::write_str("rtt min/avg/max = ");
            uart::write_u64(ping.min_rtt as u64);
            uart::write_str("/");
            uart::write_u64(ping.avg_rtt() as u64);
            uart::write_str("/");
            uart::write_u64(ping.max_rtt as u64);
            uart::write_line(" ms");
        }
        uart::write_line("");
    }
}


/// Cancel any running command (called when Ctrl+C is pressed)
pub fn cancel_running_command() -> bool {
    let running = *COMMAND_RUNNING.lock();
    if !running {
        return false;
    }

    // Check if ping is running
    let should_print_stats = {
        let ping_guard = PING_STATE.lock();
        if let Some(ref ping) = *ping_guard {
            ping.continuous
        } else {
            false
        }
    };

    if should_print_stats {
        uart::write_line("^C");
        print_ping_statistics();
        *PING_STATE.lock() = None;
        *COMMAND_RUNNING.lock() = false;
        return true;
    }

    // Generic command cancellation
    *COMMAND_RUNNING.lock() = false;
    uart::write_line("^C");
    true
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
