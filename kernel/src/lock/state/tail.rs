//! Tail follow state for tail -f functionality
//!
//! State management for following file changes like `tail -f`.

/// State for tail -f follow mode
pub struct TailFollowState {
    pub active: bool,
    pub path: [u8; 128],
    pub path_len: usize,
    pub last_size: usize,
    pub last_check_ms: i64,
}

impl TailFollowState {
    pub const fn new() -> Self {
        Self {
            active: false,
            path: [0u8; 128],
            path_len: 0,
            last_size: 0,
            last_check_ms: 0,
        }
    }
    
    pub fn start(&mut self, path: &str, initial_size: usize) {
        let bytes = path.as_bytes();
        let len = bytes.len().min(128);
        self.path[..len].copy_from_slice(&bytes[..len]);
        self.path_len = len;
        self.last_size = initial_size;
        self.last_check_ms = crate::get_time_ms();
        self.active = true;
    }
    
    pub fn stop(&mut self) {
        self.active = false;
    }
    
    pub fn get_path(&self) -> Option<&str> {
        if self.active && self.path_len > 0 {
            core::str::from_utf8(&self.path[..self.path_len]).ok()
        } else {
            None
        }
    }
    
    pub fn is_active(&self) -> bool {
        self.active
    }
    
    pub fn last_size(&self) -> usize {
        self.last_size
    }
    
    pub fn set_last_size(&mut self, size: usize) {
        self.last_size = size;
    }
    
    pub fn last_check_ms(&self) -> i64 {
        self.last_check_ms
    }
    
    pub fn set_last_check_ms(&mut self, ms: i64) {
        self.last_check_ms = ms;
    }
}

