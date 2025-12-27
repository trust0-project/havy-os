use core::sync::atomic::Ordering;

use alloc::{format, string::String};

use crate::{ allocator, clint::get_time_ms, constants::{SYSINFO_CPU_COUNT, SYSINFO_DISK_TOTAL, SYSINFO_DISK_USED, SYSINFO_HEAP_TOTAL, SYSINFO_HEAP_USED, SYSINFO_UPTIME}, cpu::HARTS_ONLINE, lock::utils::{BLK_DEV, CWD_MAX_LEN, CWD_STATE, FS_STATE, TAIL_FOLLOW_STATE}, uart};


/// Initialize CWD to root
pub fn cwd_init() {
    let mut cwd = CWD_STATE.lock();
    cwd.path[0] = b'/';
    cwd.len = 1;
}

/// Get current working directory as String
pub fn cwd_get() -> alloc::string::String {
    let cwd = CWD_STATE.lock();
    core::str::from_utf8(&cwd.path[..cwd.len])
        .unwrap_or("/")
        .into()
}

/// Set current working directory
pub fn cwd_set(path: &str) {
    let mut cwd = CWD_STATE.lock();
    let bytes = path.as_bytes();
    let len = core::cmp::min(bytes.len(), CWD_MAX_LEN);
    cwd.path[..len].copy_from_slice(&bytes[..len]);
    cwd.len = len;
}

/// Alias for cwd_get used by shell module
pub fn get_cwd() -> alloc::string::String {
    cwd_get()
}

/// Resolve a path relative to CWD
pub(crate) fn resolve_path(path: &str) -> alloc::string::String {
    use alloc::string::String;
    use alloc::vec::Vec;

    let mut result = String::new();

    // Start from root or CWD
    let cwd = cwd_get();
    let base: &str = if path.starts_with('/') { "/" } else { &cwd };

    // Combine base and path, then normalize
    let full = if path.starts_with('/') {
        String::from(path)
    } else if base == "/" {
        let mut s = String::from("/");
        s.push_str(path);
        s
    } else {
        let mut s = String::from(base);
        s.push('/');
        s.push_str(path);
        s
    };

    // Split and normalize (handle . and ..)
    let mut parts: Vec<&str> = Vec::new();
    for part in full.split('/') {
        match part {
            "" | "." => continue,
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }

    // Rebuild path
    result.push('/');
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            result.push('/');
        }
        result.push_str(part);
    }

    if result.is_empty() {
        result.push('/');
    }

    result
}

/// Check if a path exists (has files under it or is a file)
pub(crate) fn path_exists(path: &str) -> bool {
    // Root always exists
    if path == "/" {
        return true;
    }
    
    // Use VFS for mount point visibility
    let mut vfs_guard = crate::lock::utils::VFS_STATE.write();
    if let Some(vfs) = vfs_guard.as_mut() {
        return vfs.exists(path);
    }
    drop(vfs_guard);
    
    // Fall back to legacy FS_STATE
    let mut fs_guard = FS_STATE.write();
    let mut blk_guard = BLK_DEV.write();
    if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
        let files = fs.list_dir(dev, "/");
        let path_with_slash = if path.ends_with('/') {
            alloc::string::String::from(path)
        } else {
            let mut s = alloc::string::String::from(path);
            s.push('/');
            s
        };

        for file in files {
            // Check if any file starts with this path (it's a directory)
            if file.name.starts_with(&path_with_slash) {
                return true;
            }
            // Or if it exactly matches (it's a file)
            if file.name == path {
                return true;
            }
        }
    }
    false
}


pub(crate) fn print_prompt() {
    let cwd = cwd_get();
    let prompt_path = if cwd == "/" {
        String::new()
    } else {
        format!(" {}", cwd)
    };

    uart::write_str(&format!(
        "\x1b[1;35mBavy\x1b[0m\x1b[1;34m{}\x1b[0m # ",
        prompt_path
    ));
}

/// Write system statistics to the MMIO SysInfo device
/// This allows the emulator to read kernel stats and display them in the UI
pub(crate) fn update_sysinfo() {
    // Get CPU count first (needed for memory stats calculation)
    let cpu_count = HARTS_ONLINE.load(Ordering::Relaxed);
    
    // Get comprehensive memory stats (includes kernel code, stacks, heap)
    let mem_stats = allocator::memory_stats(cpu_count);
    
    // Get disk stats (if filesystem available)
    let (disk_used, disk_total) = {
        let fs_guard = FS_STATE.read();
        if let Some(ref fs) = *fs_guard {
            fs.disk_usage_bytes()
        } else {
            (0, 0)
        }
    };
    
    // Get uptime
    let uptime_ms = get_time_ms() as u64;
    
    // Write to MMIO registers (volatile writes, all 64-bit writes are 8-byte aligned)
    unsafe {
        core::ptr::write_volatile(SYSINFO_HEAP_USED as *mut u64, mem_stats.total_used as u64);
        core::ptr::write_volatile(SYSINFO_HEAP_TOTAL as *mut u64, mem_stats.total_available as u64);
        core::ptr::write_volatile(SYSINFO_DISK_USED as *mut u64, disk_used);
        core::ptr::write_volatile(SYSINFO_DISK_TOTAL as *mut u64, disk_total);
        core::ptr::write_volatile(SYSINFO_CPU_COUNT as *mut u32, cpu_count as u32);
        core::ptr::write_volatile(SYSINFO_UPTIME as *mut u64, uptime_ms);
    }
}

/// Check for new content in a file being followed by tail -f
/// Returns the new file size if content was found, None otherwise
/// 
/// Multi-hart safe: Uses fs_proxy for hart-aware filesystem access.
pub(crate) fn check_tail_follow(path: &str, last_size: usize) -> Option<usize> {
    // Use fs_proxy for multi-hart safety
    if let Some(content) = crate::cpu::fs_proxy::fs_read(path) {
        let new_size = content.len();

        if new_size > last_size {
            // Print new content with green highlighting
            let new_content = &content[last_size..];
            if let Ok(text) = core::str::from_utf8(new_content) {
                for line in text.lines() {
                    uart::write_str("\x1b[1;32m");
                    uart::write_str(line);
                    uart::write_line("\x1b[0m");
                }
            }
            return Some(new_size);
        } else if new_size < last_size {
            // File was truncated
            uart::write_line("\x1b[1;33mtail: file truncated\x1b[0m");
            return Some(new_size);
        }

        return Some(last_size); // No change
    }
    None
}



/// Poll tail follow for new content (called from uart during blocking reads)
/// Returns true if content was found and printed
pub(crate) fn poll_tail_follow() -> bool {
    let mut state = TAIL_FOLLOW_STATE.lock();
    
    if !state.active {
        return false;
    }
    
    // Only check every 500ms to avoid excessive filesystem access
    let now = get_time_ms();
    if now - state.last_check_ms < 500 {
        return false;
    }
    state.last_check_ms = now;
    
    // Get a copy of path before releasing lock
    let path_copy = if let Some(p) = state.get_path() {
        alloc::string::String::from(p)
    } else {
        return false;
    };
    let last_size = state.last_size;
    
    // Release lock before filesystem access
    drop(state);
    
    // Check for new content
    if let Some(new_size) = check_tail_follow(&path_copy, last_size) {
        let mut state = TAIL_FOLLOW_STATE.lock();
        state.last_size = new_size;
        return new_size > last_size;
    }
    
    false
}
