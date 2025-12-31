//! Native RISC-V Syscall Handler
//!
//! This module handles `ecall` instructions from native RISC-V userspace binaries.
//! Unlike WASM execution, native binaries share the kernel's address space, so
//! memory access is direct (no wasmi memory abstraction needed).
//!
//! Calling convention (same as Linux RISC-V):
//! - a7 = syscall number
//! - a0-a5 = arguments
//! - a0 = return value (negative on error)

use alloc::{format, string::String, vec};
use core::slice;

use crate::syscall_numbers::*;
use crate::{
    clint::get_time_ms,
    cpu::fs_proxy,
    lock::utils::BLK_DEV,
    services::klogd::KLOG,
    scripting, uart,
};

// ═══════════════════════════════════════════════════════════════════════════════
// Syscall Context - Passed to syscall handlers
// ═══════════════════════════════════════════════════════════════════════════════

/// Context for the currently executing userspace binary.
/// Contains arguments passed from shell and exit status.
pub struct SyscallContext {
    /// Command-line arguments
    pub args: &'static [&'static str],
    /// Exit code (set by SYS_EXIT)
    pub exit_code: Option<i32>,
}

/// Thread-local syscall context
/// SAFETY: Only accessed from the hart running the binary
static mut SYSCALL_CTX: Option<SyscallContext> = None;

/// Initialize syscall context with arguments
pub fn init_context(args: &'static [&'static str]) {
    unsafe {
        SYSCALL_CTX = Some(SyscallContext {
            args,
            exit_code: None,
        });
    }
}

/// Get the current syscall context
fn get_context() -> Option<&'static SyscallContext> {
    unsafe { SYSCALL_CTX.as_ref() }
}

/// Clear syscall context after binary exits
pub fn clear_context() -> Option<i32> {
    unsafe {
        let exit_code = SYSCALL_CTX.as_ref().and_then(|c| c.exit_code);
        SYSCALL_CTX = None;
        exit_code
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Main Syscall Dispatcher
// ═══════════════════════════════════════════════════════════════════════════════

/// Handle a syscall from userspace.
///
/// Called from trap handler when an `ecall` is executed.
///
/// # Arguments
/// * `syscall_num` - Syscall number (from a7 register)
/// * `a0..a5` - Syscall arguments
///
/// # Returns
/// * Result value to store in a0 (negative on error)
#[inline(never)]
pub fn handle_syscall(
    syscall_num: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
    _a5: u64,
) -> i64 {
    match syscall_num {
        // Core
        SYS_PRINT => sys_print(a0 as *const u8, a1 as usize),
        SYS_TIME => sys_time(),
        SYS_EXIT => sys_exit(a0 as i32),

        // Arguments
        SYS_ARG_COUNT => sys_arg_count(),
        SYS_ARG_GET => sys_arg_get(a0 as usize, a1 as *mut u8, a2 as usize),
        SYS_CWD_GET => sys_cwd_get(a0 as *mut u8, a1 as usize),
        SYS_CWD_SET => sys_cwd_set(a0 as *const u8, a1 as usize),

        // Filesystem
        SYS_FS_EXISTS => sys_fs_exists(a0 as *const u8, a1 as usize),
        SYS_FS_READ => sys_fs_read(a0 as *const u8, a1 as usize, a2 as *mut u8, a3 as usize),
        SYS_FS_WRITE => sys_fs_write(a0 as *const u8, a1 as usize, a2 as *const u8, a3 as usize),
        SYS_FS_LIST => sys_fs_list(a0 as *mut u8, a1 as usize),
        SYS_FS_LIST_DIR => sys_fs_list_dir(a0 as *const u8, a1 as usize, a2 as *mut u8, a3 as usize),
        SYS_FS_STAT => sys_fs_stat(a0 as *const u8, a1 as usize, a2 as *mut u8),
        SYS_FS_REMOVE => sys_fs_remove(a0 as *const u8, a1 as usize),
        SYS_FS_MKDIR => sys_fs_mkdir(a0 as *const u8, a1 as usize),
        SYS_FS_IS_DIR => sys_fs_is_dir(a0 as *const u8, a1 as usize),

        // Network
        SYS_NET_AVAILABLE => sys_net_available(),
        SYS_DNS_RESOLVE => sys_dns_resolve(a0 as *const u8, a1 as usize, a2 as *mut u8, a3 as usize),
        SYS_SEND_PING => sys_send_ping(a0 as *const u8, a1 as i32, a2 as i32, a3 as *mut u8),
        SYS_TCP_CONNECT => sys_tcp_connect(a0 as *const u8, a1 as u16),
        SYS_TCP_SEND => sys_tcp_send(a0 as *const u8, a1 as usize),
        SYS_TCP_RECV => sys_tcp_recv(a0 as *mut u8, a1 as usize),
        SYS_TCP_CLOSE => sys_tcp_close(),
        SYS_TCP_STATUS => sys_tcp_status(),
        SYS_HTTP_GET => sys_http_get(a0 as *const u8, a1 as usize, a2 as *mut u8, a3 as usize),

        // Console
        SYS_CONSOLE_AVAILABLE => sys_console_available(),
        SYS_CONSOLE_READ => sys_console_read(a0 as *mut u8, a1 as usize),

        // Process
        SYS_PS_LIST => sys_ps_list(a0 as *mut u8, a1 as usize),
        SYS_KILL => sys_kill(a0 as u32),
        SYS_CPU_INFO => sys_cpu_info(a0 as i32, a1 as *mut u8),

        // System
        SYS_SHUTDOWN => sys_shutdown(),
        SYS_SHOULD_CANCEL => sys_should_cancel(),
        SYS_RANDOM => sys_random(a0 as *mut u8, a1 as usize),
        SYS_ENV_GET => sys_env_get(a0 as *const u8, a1 as usize, a2 as *mut u8, a3 as usize),
        SYS_KLOG_GET => sys_klog_get(a0 as usize, a1 as *mut u8, a2 as usize),

        // Services
        SYS_SERVICE_LIST => sys_service_list(a0 as *mut u8, a1 as usize),
        SYS_SERVICE_START => sys_service_start(a0 as *const u8, a1 as usize),
        SYS_SERVICE_STOP => sys_service_stop(a0 as *const u8, a1 as usize),
        SYS_SERVICE_RUNNING => sys_service_running(a0 as *mut u8, a1 as usize),

        // Extended
        SYS_NET_INFO => sys_net_info(a0 as *mut u8, a1 as usize),
        SYS_HEAP_STATS => sys_heap_stats(a0 as *mut u8),
        SYS_SLEEP => sys_sleep(a0 as u64),

        // Unknown syscall
        _ => -1, // ENOSYS
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Helper Functions
// ═══════════════════════════════════════════════════════════════════════════════

/// Read a string from userspace memory
/// SAFETY: Caller must ensure ptr is valid and len is correct
unsafe fn read_str(ptr: *const u8, len: usize) -> Option<&'static str> {
    if ptr.is_null() || len == 0 {
        return None;
    }
    let bytes = slice::from_raw_parts(ptr, len);
    core::str::from_utf8(bytes).ok()
}

/// Write bytes to userspace memory
/// SAFETY: Caller must ensure ptr is valid and has enough capacity
unsafe fn write_bytes(ptr: *mut u8, data: &[u8], max_len: usize) -> i64 {
    if ptr.is_null() {
        return -1;
    }
    let to_copy = data.len().min(max_len);
    core::ptr::copy_nonoverlapping(data.as_ptr(), ptr, to_copy);
    to_copy as i64
}

// ═══════════════════════════════════════════════════════════════════════════════
// Core Syscalls
// ═══════════════════════════════════════════════════════════════════════════════

fn sys_print(ptr: *const u8, len: usize) -> i64 {
    if ptr.is_null() {
        return -1;
    }
    unsafe {
        if let Some(s) = read_str(ptr, len) {
            scripting::out_str(s);
        }
    }
    0
}

fn sys_time() -> i64 {
    get_time_ms()
}

fn sys_exit(code: i32) -> i64 {
    // Signal exit to the ELF loader - trap handler will restore kernel context
    crate::elf_loader::signal_exit(code);
    code as i64
}

// ═══════════════════════════════════════════════════════════════════════════════
// Argument Syscalls
// ═══════════════════════════════════════════════════════════════════════════════

fn sys_arg_count() -> i64 {
    get_context().map(|c| c.args.len() as i64).unwrap_or(0)
}

fn sys_arg_get(index: usize, buf_ptr: *mut u8, buf_len: usize) -> i64 {
    let ctx = match get_context() {
        Some(c) => c,
        None => return -1,
    };
    
    if index >= ctx.args.len() {
        return -1;
    }
    
    let arg = ctx.args[index];
    let bytes = arg.as_bytes();
    
    if bytes.len() > buf_len {
        return -1;
    }
    
    unsafe { write_bytes(buf_ptr, bytes, buf_len) }
}

fn sys_cwd_get(buf_ptr: *mut u8, buf_len: usize) -> i64 {
    let cwd = crate::utils::cwd_get();
    let bytes = cwd.as_bytes();
    if bytes.len() > buf_len {
        return -1;
    }
    unsafe { write_bytes(buf_ptr, bytes, buf_len) }
}

fn sys_cwd_set(path_ptr: *const u8, path_len: usize) -> i64 {
    unsafe {
        if let Some(path) = read_str(path_ptr, path_len) {
            let resolved = crate::resolve_path(path);
            if crate::utils::path_exists(&resolved) {
                crate::utils::cwd_set(&resolved);
                return 0;
            }
        }
    }
    -1
}

// ═══════════════════════════════════════════════════════════════════════════════
// Filesystem Syscalls
// ═══════════════════════════════════════════════════════════════════════════════

fn sys_fs_exists(path_ptr: *const u8, path_len: usize) -> i64 {
    unsafe {
        if let Some(path) = read_str(path_ptr, path_len) {
            return if fs_proxy::fs_exists(path) { 1 } else { 0 };
        }
    }
    0
}

fn sys_fs_read(path_ptr: *const u8, path_len: usize, buf_ptr: *mut u8, buf_len: usize) -> i64 {
    unsafe {
        if let Some(path) = read_str(path_ptr, path_len) {
            if let Some(data) = fs_proxy::fs_read(path) {
                return write_bytes(buf_ptr, &data, buf_len);
            }
        }
    }
    -1
}

fn sys_fs_write(path_ptr: *const u8, path_len: usize, data_ptr: *const u8, data_len: usize) -> i64 {
    use crate::device::uart::{write_str, write_line};
    
    unsafe {
        if let Some(path) = read_str(path_ptr, path_len) {
            write_str("fs_write syscall: ");
            write_str(path);
            write_str(" (");
            write_str(&alloc::format!("{}", data_len));
            write_line(" bytes)");
            
            if !data_ptr.is_null() {
                let data = slice::from_raw_parts(data_ptr, data_len);
                match fs_proxy::fs_write(path, data) {
                    Ok(()) => {
                        write_line("fs_write: OK");
                        return data_len as i64;
                    }
                    Err(e) => {
                        write_str("fs_write: ERROR - ");
                        write_line(e);
                    }
                }
            } else {
                write_line("fs_write: data_ptr is null");
            }
        } else {
            write_line("fs_write: path read failed");
        }
    }
    -1
}

fn sys_fs_list(buf_ptr: *mut u8, buf_len: usize) -> i64 {
    let files = fs_proxy::fs_list("/");
    let mut output = String::new();
    for file in files {
        output.push_str(&file.name);
        output.push(':');
        output.push_str(&format!("{}", file.size));
        output.push('\n');
    }
    unsafe { write_bytes(buf_ptr, output.as_bytes(), buf_len) }
}

fn sys_fs_list_dir(path_ptr: *const u8, path_len: usize, buf_ptr: *mut u8, buf_len: usize) -> i64 {
    unsafe {
        if let Some(path) = read_str(path_ptr, path_len) {
            let files = fs_proxy::fs_list(path);
            let mut output = String::new();
            for file in files {
                output.push_str(&file.name);
                output.push(':');
                output.push_str(&format!("{}", file.size));
                output.push('\n');
            }
            return write_bytes(buf_ptr, output.as_bytes(), buf_len);
        }
    }
    -1
}

fn sys_fs_stat(path_ptr: *const u8, path_len: usize, out_ptr: *mut u8) -> i64 {
    unsafe {
        if let Some(path) = read_str(path_ptr, path_len) {
            let mut fs_guard = crate::FS_STATE.write();
            let mut blk_guard = BLK_DEV.write();
            if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
                let file_data = fs.read_file(dev, path);
                let (size, exists, is_dir): (u32, u8, u8) = match file_data {
                    Some(data) => (data.len() as u32, 1, 0),
                    None => {
                        let files = fs.list_dir(dev, "/");
                        let prefix = if path.ends_with('/') {
                            String::from(path)
                        } else {
                            format!("{}/", path)
                        };
                        let is_directory = files.iter().any(|f| f.name.starts_with(&prefix));
                        if is_directory { (0, 1, 1) } else { (0, 0, 0) }
                    }
                };
                
                let mut out = [0u8; 6];
                out[0..4].copy_from_slice(&size.to_le_bytes());
                out[4] = exists;
                out[5] = is_dir;
                core::ptr::copy_nonoverlapping(out.as_ptr(), out_ptr, 6);
                return 0;
            }
        }
    }
    -1
}

fn sys_fs_remove(_path_ptr: *const u8, _path_len: usize) -> i64 {
    // File removal not yet supported
    -1
}

fn sys_fs_mkdir(path_ptr: *const u8, path_len: usize) -> i64 {
    unsafe {
        if let Some(path) = read_str(path_ptr, path_len) {
            let mut fs_guard = crate::FS_STATE.write();
            let mut blk_guard = BLK_DEV.write();
            if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
                let keep_path = format!("{}/.keep", path.trim_end_matches('/'));
                if fs.write_file(dev, &keep_path, &[]).is_ok() {
                    return 0;
                }
            }
        }
    }
    -1
}

fn sys_fs_is_dir(path_ptr: *const u8, path_len: usize) -> i64 {
    unsafe {
        if let Some(path) = read_str(path_ptr, path_len) {
            let fs_guard = crate::FS_STATE.read();
            let blk_guard = BLK_DEV.read();
            if let (Some(fs), Some(_dev)) = (fs_guard.as_ref(), blk_guard.as_ref()) {
                // Check if any files have this prefix
                let prefix = if path.ends_with('/') {
                    String::from(path)
                } else {
                    format!("{}/", path)
                };
                // Use VFS to check
                let files = fs_proxy::fs_list("/");
                let is_dir = files.iter().any(|f| f.name.starts_with(&prefix));
                return if is_dir { 1 } else { 0 };
            }
        }
    }
    -1
}

// ═══════════════════════════════════════════════════════════════════════════════
// Network Syscalls
// ═══════════════════════════════════════════════════════════════════════════════

fn sys_net_available() -> i64 {
    let net_guard = crate::NET_STATE.lock();
    if net_guard.is_some() { 1 } else { 0 }
}

fn sys_dns_resolve(host_ptr: *const u8, host_len: usize, ip_buf_ptr: *mut u8, ip_buf_len: usize) -> i64 {
    if ip_buf_len < 4 {
        return -1;
    }
    unsafe {
        if host_ptr.is_null() {
            return -1;
        }
        let host_bytes = slice::from_raw_parts(host_ptr, host_len);
        let dns_server = smoltcp::wire::Ipv4Address::new(8, 8, 8, 8);
        
        let mut net_guard = crate::NET_STATE.lock();
        if let Some(ref mut net) = *net_guard {
            if let Some(ip) = crate::dns::resolve(net, host_bytes, dns_server, 5000, get_time_ms) {
                let octets = ip.octets();
                core::ptr::copy_nonoverlapping(octets.as_ptr(), ip_buf_ptr, 4);
                return 4;
            }
        }
    }
    -1
}

fn sys_send_ping(ip_ptr: *const u8, seq: i32, timeout_ms: i32, out_ptr: *mut u8) -> i64 {
    if ip_ptr.is_null() || out_ptr.is_null() {
        return -2;
    }
    
    unsafe {
        let ip_bytes = slice::from_raw_parts(ip_ptr, 4);
        let target = smoltcp::wire::Ipv4Address::new(ip_bytes[0], ip_bytes[1], ip_bytes[2], ip_bytes[3]);
        let seq = seq as u16;
        let timestamp = get_time_ms();
        
        // Send ping using NET_STATE
        let send_result = {
            let mut net_guard = crate::NET_STATE.lock();
            if let Some(ref mut state) = *net_guard {
                state.send_ping(target, seq, timestamp)
            } else {
                return -2; // No network available
            }
        };
        
        if send_result.is_err() {
            return -2;
        }
        
        // Wait for reply
        let deadline = timestamp + timeout_ms as i64;
        loop {
            let now = get_time_ms();
            if now >= deadline {
                return -1; // Timeout
            }
            
            // Poll network and check for reply
            let reply = {
                let mut net_guard = crate::NET_STATE.lock();
                if let Some(ref mut state) = *net_guard {
                    state.poll(now);
                    state.check_ping_reply()
                } else {
                    None
                }
            };
            
            if let Some((reply_ip, _ident, reply_seq)) = reply {
                // Check if this is the reply we're waiting for
                if reply_ip == target && reply_seq == seq {
                    let rtt = (now - timestamp) as u32;
                    // Write result (rtt in ms)
                    let out = rtt.to_le_bytes();
                    core::ptr::copy_nonoverlapping(out.as_ptr(), out_ptr, 4);
                    return 0;
                }
            }
            
            core::hint::spin_loop();
        }
    }
}

fn sys_tcp_connect(ip_ptr: *const u8, port: u16) -> i64 {
    unsafe {
        if ip_ptr.is_null() {
            return -1;
        }
        let ip_bytes = slice::from_raw_parts(ip_ptr, 4);
        
        let mut net_guard = crate::NET_STATE.lock();
        if let Some(ref mut net) = *net_guard {
            let ip = smoltcp::wire::Ipv4Address::new(ip_bytes[0], ip_bytes[1], ip_bytes[2], ip_bytes[3]);
            let now = get_time_ms();
            if net.tcp_connect(ip, port, now).is_ok() {
                net.poll(now);
                return 0;
            }
        }
    }
    -1
}

fn sys_tcp_send(data_ptr: *const u8, data_len: usize) -> i64 {
    unsafe {
        if data_ptr.is_null() {
            return -1;
        }
        let data = slice::from_raw_parts(data_ptr, data_len);
        
        let mut net_guard = crate::NET_STATE.lock();
        if let Some(ref mut net) = *net_guard {
            let now = get_time_ms();
            if let Ok(sent) = net.tcp_send(data, now) {
                net.poll(now);
                return sent as i64;
            }
        }
    }
    -1
}

fn sys_tcp_recv(buf_ptr: *mut u8, buf_len: usize) -> i64 {
    unsafe {
        if buf_ptr.is_null() {
            return -1;
        }
        
        let mut net_guard = crate::NET_STATE.lock();
        if let Some(ref mut net) = *net_guard {
            let now = get_time_ms();
            net.poll(now);
            let mut temp_buf = vec![0u8; buf_len];
            if let Ok(received) = net.tcp_recv(&mut temp_buf, now) {
                if received > 0 {
                    core::ptr::copy_nonoverlapping(temp_buf.as_ptr(), buf_ptr, received);
                }
                return received as i64;
            }
        }
    }
    -1
}

fn sys_tcp_close() -> i64 {
    let mut net_guard = crate::NET_STATE.lock();
    if let Some(ref mut net) = *net_guard {
        let now = get_time_ms();
        net.tcp_close(now);
        return 0;
    }
    -1
}

fn sys_tcp_status() -> i64 {
    let mut net_guard = crate::NET_STATE.lock();
    if let Some(ref mut net) = *net_guard {
        // 0=closed, 1=connecting, 2=connected, 3=failed
        if net.tcp_is_connected() {
            return 2;
        } else if net.tcp_connection_failed() {
            return 3;
        }
        return 1; // connecting
    }
    0 // closed
}

fn sys_http_get(url_ptr: *const u8, url_len: usize, resp_ptr: *mut u8, resp_len: usize) -> i64 {
    unsafe {
        if let Some(url) = read_str(url_ptr, url_len) {
            let mut net_guard = crate::NET_STATE.lock();
            if let Some(ref mut net) = *net_guard {
                match crate::commands::http::get_follow_redirects(net, url, 30000, get_time_ms) {
                    Ok(response) => {
                        return write_bytes(resp_ptr, &response.body, resp_len);
                    }
                    Err(_) => return -1,
                }
            }
        }
    }
    -1
}

// ═══════════════════════════════════════════════════════════════════════════════
// Console Syscalls
// ═══════════════════════════════════════════════════════════════════════════════

fn sys_console_available() -> i64 {
    if uart::has_pending_input() { 1 } else { 0 }
}

fn sys_console_read(buf_ptr: *mut u8, _buf_len: usize) -> i64 {
    if let Some(ch) = uart::read_char_nonblocking() {
        unsafe {
            if !buf_ptr.is_null() {
                *buf_ptr = ch;
                return 1;
            }
        }
    }
    0
}

// ═══════════════════════════════════════════════════════════════════════════════
// Process Syscalls
// ═══════════════════════════════════════════════════════════════════════════════

fn sys_ps_list(buf_ptr: *mut u8, buf_len: usize) -> i64 {
    use crate::cpu::sched::SCHEDULER;
    use crate::cpu::process::ProcessState;
    
    let mut output = String::new();
    
    // Get processes from scheduler
    for proc in SCHEDULER.list_processes() {
        let is_running = proc.state == ProcessState::Running;
        output.push_str(&format!(
            "{}:{}:{}:{}:{}:{}\n",
            proc.pid,
            proc.name,
            if is_running { "R" } else { "S" },
            proc.priority as u8,
            proc.cpu_time_ms,
            proc.uptime_ms
        ));
    }
    
    // Also include shell command if running
    if let Some((name, pid, cpu, uptime, running)) = crate::wasm::get_shell_cmd_info() {
        output.push_str(&format!(
            "{}:{}:{}:0:{}:{}\n",
            pid,
            name,
            if running { "R" } else { "S" },
            uptime,
            cpu
        ));
    }
    
    unsafe { write_bytes(buf_ptr, output.as_bytes(), buf_len) }
}

fn sys_kill(pid: u32) -> i64 {
    use crate::cpu::sched::SCHEDULER;
    
    if pid == 0 {
        return -2; // Cannot kill init
    }
    
    SCHEDULER.exit(pid, 9);
    0
}

fn sys_cpu_info(cpu_id: i32, out_ptr: *mut u8) -> i64 {
    use crate::cpu::CPU_TABLE;
    
    if let Some(cpu) = CPU_TABLE.get(cpu_id as usize) {
        if !cpu.is_online() {
            return -1;
        }
        
        // Format: state (1 byte) + utilization (1 byte) + current_pid (4 bytes)
        let mut out = [0u8; 6];
        out[0] = cpu.state() as u8;
        out[1] = cpu.utilization();
        out[2..6].copy_from_slice(&cpu.current_process.load(core::sync::atomic::Ordering::Relaxed).to_le_bytes());
        
        unsafe {
            if !out_ptr.is_null() {
                core::ptr::copy_nonoverlapping(out.as_ptr(), out_ptr, 6);
                return 0;
            }
        }
    }
    -1
}

// ═══════════════════════════════════════════════════════════════════════════════
// System Syscalls
// ═══════════════════════════════════════════════════════════════════════════════

fn sys_shutdown() -> i64 {
    uart::write_line("");
    uart::write_line("\x1b[1;31m+===================================================================+\x1b[0m");
    uart::write_line("\x1b[1;31m|\x1b[0m                    \x1b[1;97mSystem Shutdown Initiated\x1b[0m                       \x1b[1;31m|\x1b[0m");
    uart::write_line("\x1b[1;31m+===================================================================+\x1b[0m");
    uart::write_line("");
    
    unsafe {
        core::ptr::write_volatile(crate::constants::TEST_FINISHER as *mut u32, 0x5555);
    }
    
    loop {
        core::hint::spin_loop();
    }
}

fn sys_should_cancel() -> i64 {
    // Check shared cancellation flag from SharedArrayBuffer (WASM path)
    let shared_cancel = unsafe {
        core::ptr::read_volatile((0x0250_2000 + 0x130) as *const u32)
    };
    if shared_cancel != 0 {
        return 1;
    }
    
    // Check kernel-side cancellation flag
    if crate::ui::main_screen::should_cancel() {
        return 1;
    }
    
    0
}

fn sys_random(buf_ptr: *mut u8, buf_len: usize) -> i64 {
    // Simple PRNG based on time
    let mut seed = get_time_ms() as u64;
    let mut random_bytes = vec![0u8; buf_len];
    for byte in random_bytes.iter_mut() {
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        *byte = (seed >> 16) as u8;
    }
    unsafe { write_bytes(buf_ptr, &random_bytes, buf_len) }
}

fn sys_env_get(key_ptr: *const u8, key_len: usize, val_ptr: *mut u8, val_len: usize) -> i64 {
    unsafe {
        if let Some(key) = read_str(key_ptr, key_len) {
            let value = match key {
                "HOME" => Some("/home"),
                "PATH" => Some("/usr/bin"),
                "USER" => Some("root"),
                "SHELL" => Some("/usr/bin/sh"),
                "TERM" => Some("xterm-256color"),
                "PWD" => {
                    let cwd = crate::utils::cwd_get();
                    return write_bytes(val_ptr, cwd.as_bytes(), val_len);
                }
                _ => None,
            };
            
            if let Some(val) = value {
                return write_bytes(val_ptr, val.as_bytes(), val_len);
            }
        }
    }
    -1
}

fn sys_klog_get(count: usize, buf_ptr: *mut u8, buf_len: usize) -> i64 {
    let count = count.max(1).min(100);
    let entries = KLOG.recent(count);
    let mut output = String::new();
    for entry in entries.iter().rev() {
        output.push_str(&entry.format_colored());
        output.push('\n');
    }
    unsafe { write_bytes(buf_ptr, output.as_bytes(), buf_len) }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Service Syscalls
// ═══════════════════════════════════════════════════════════════════════════════

fn sys_service_list(buf_ptr: *mut u8, buf_len: usize) -> i64 {
    // Return a static list of known services for now
    let output = "netd:Network daemon\nhttpd:HTTP server\ngpuid:GUI daemon\n";
    unsafe { write_bytes(buf_ptr, output.as_bytes(), buf_len) }
}

fn sys_service_start(name_ptr: *const u8, name_len: usize) -> i64 {
    unsafe {
        if let Some(name) = read_str(name_ptr, name_len) {
            if crate::init::start_service(name).is_ok() {
                return 0;
            }
        }
    }
    -1
}

fn sys_service_stop(name_ptr: *const u8, name_len: usize) -> i64 {
    unsafe {
        if let Some(name) = read_str(name_ptr, name_len) {
            if crate::init::stop_service(name).is_ok() {
                return 0;
            }
        }
    }
    -1
}

fn sys_service_running(buf_ptr: *mut u8, buf_len: usize) -> i64 {
    use crate::cpu::sched::SCHEDULER;
    use crate::cpu::process::ProcessFlags;
    
    let mut output = String::new();
    for proc in SCHEDULER.list_processes() {
        if proc.flags.contains(ProcessFlags::DAEMON) {
            output.push_str(&proc.name);
            output.push(':');
            output.push_str(&format!("{}", proc.pid));
            output.push('\n');
        }
    }
    
    unsafe { write_bytes(buf_ptr, output.as_bytes(), buf_len) }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Extended Syscalls
// ═══════════════════════════════════════════════════════════════════════════════

/// Get network information: IP, MAC, gateway, DNS, prefix length
/// Output format: IP[4], MAC[6], Gateway[4], DNS[4], prefix_len[1] = 19 bytes
fn sys_net_info(out_ptr: *mut u8, out_len: usize) -> i64 {
    use crate::net::config::{get_my_ip, GATEWAY, DNS_SERVER, PREFIX_LEN, is_ip_assigned};
    
    if out_ptr.is_null() || out_len < 19 {
        return -1;
    }

    // Check if network is available (IP assigned)
    if !is_ip_assigned() {
        return -2; // Network not configured
    }
    
    let my_ip = get_my_ip();
    
    let mut buf = [0u8; 19];
    // IP (4 bytes)
    buf[0..4].copy_from_slice(&my_ip.octets());
    // MAC (6 bytes) - use a default/fake MAC for now
    buf[4..10].copy_from_slice(&[0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
    // Gateway (4 bytes)
    buf[10..14].copy_from_slice(&GATEWAY.octets());
    // DNS (4 bytes)
    buf[14..18].copy_from_slice(&DNS_SERVER.octets());
    // Prefix length (1 byte)
    buf[18] = PREFIX_LEN;
    
    unsafe { write_bytes(out_ptr, &buf, out_len) }
}

/// Get heap statistics: used_bytes[8], total_bytes[8] = 16 bytes
fn sys_heap_stats(out_ptr: *mut u8) -> i64 {
    if out_ptr.is_null() {
        return -1;
    }

    let (used, free) = crate::allocator::heap_stats();
    let total = crate::allocator::heap_size();
    
    let mut buf = [0u8; 16];
    buf[0..8].copy_from_slice(&(used as u64).to_le_bytes());
    buf[8..16].copy_from_slice(&(total as u64).to_le_bytes());
    
    unsafe { write_bytes(out_ptr, &buf, 16) }
}

/// Sleep for the given number of milliseconds
fn sys_sleep(ms: u64) -> i64 {
    let start = get_time_ms();
    let target = start + ms as i64;
    
    // Busy-wait loop with WFI for power efficiency
    while get_time_ms() < target {
        // Hint to the processor we're waiting
        unsafe { core::arch::asm!("wfi", options(nomem, nostack)); }
    }
    
    0
}

