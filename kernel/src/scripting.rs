// kernel/src/scripting.rs
//! Script discovery for WASM binaries in /usr/bin/
//!
//! This module provides script lookup functionality for the shell.
//! Scripts are WASM binaries located in /usr/bin/ directory.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

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
        let mut blk_guard = match crate::BLK_DEV.try_write() {
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
