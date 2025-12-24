//! Shell command state for tracking currently running shell commands
//!
//! Tracks CPU time and other metrics for shell commands.

/// State for tracking the currently running shell command's CPU time
pub struct ShellCmdState {
    /// Command name (limited to 32 chars)
    pub name: [u8; 32],
    pub name_len: usize,
    /// Virtual PID for the current command (starts at 1000)
    pub pid: u32,
    /// Start time of current command (ms since boot)
    pub start_time: u64,
    /// Whether a command is currently running
    pub is_running: bool,
    /// Accumulated CPU time for tracking (ms)
    pub accumulated_cpu_time: u64,
    /// Time when this shell session started
    pub session_start: u64,
}

impl ShellCmdState {
    pub const fn new() -> Self {
        Self {
            name: [0u8; 32],
            name_len: 0,
            pid: 0,
            start_time: 0,
            is_running: false,
            accumulated_cpu_time: 0,
            session_start: 0,
        }
    }

    pub fn start_command(&mut self, cmd_name: &str, current_time: u64) {
        // Copy command name (truncate if too long)
        let bytes = cmd_name.as_bytes();
        let copy_len = bytes.len().min(31);
        self.name[..copy_len].copy_from_slice(&bytes[..copy_len]);
        self.name_len = copy_len;
        self.start_time = current_time;
        self.is_running = true;
        // Reset CPU time for this command (don't accumulate from previous commands)
        self.accumulated_cpu_time = 0;
        // Allocate a real PID from the process module
        self.pid = crate::cpu::process::allocate_pid();
    }

    pub fn end_command(&mut self, current_time: u64) {
        if self.is_running {
            let elapsed = current_time.saturating_sub(self.start_time);
            self.accumulated_cpu_time = self.accumulated_cpu_time.saturating_add(elapsed);
            self.is_running = false;
        }
    }

    pub fn get_name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("unknown")
    }
}



