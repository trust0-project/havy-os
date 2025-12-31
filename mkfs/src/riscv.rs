// mkfs/src/riscv.rs
//! Native RISC-V syscall wrappers for userspace binaries.
//!
//! This module provides the same API as the WASM syscalls module but uses
//! RISC-V `ecall` instructions to invoke kernel services.
//!
//! Calling convention:
//! - a7 = syscall number
//! - a0-a5 = arguments  
//! - a0 = return value

use core::arch::asm;

// ═══════════════════════════════════════════════════════════════════════════════
// Syscall Numbers (must match kernel/src/syscall_numbers.rs)
// ═══════════════════════════════════════════════════════════════════════════════

const SYS_PRINT: u64 = 0;
const SYS_TIME: u64 = 1;
const SYS_EXIT: u64 = 2;
const SYS_ARG_COUNT: u64 = 10;
const SYS_ARG_GET: u64 = 11;
const SYS_CWD_GET: u64 = 12;
const SYS_CWD_SET: u64 = 13;
const SYS_FS_EXISTS: u64 = 20;
const SYS_FS_READ: u64 = 21;
const SYS_FS_WRITE: u64 = 22;
const SYS_FS_LIST: u64 = 23;
const SYS_FS_STAT: u64 = 24;
const SYS_FS_REMOVE: u64 = 25;
const SYS_FS_MKDIR: u64 = 26;
const SYS_FS_IS_DIR: u64 = 27;
const SYS_FS_LIST_DIR: u64 = 28;
const SYS_NET_AVAILABLE: u64 = 30;
const SYS_DNS_RESOLVE: u64 = 31;
const SYS_SEND_PING: u64 = 32;
const SYS_TCP_CONNECT: u64 = 33;
const SYS_TCP_SEND: u64 = 34;
const SYS_TCP_RECV: u64 = 35;
const SYS_TCP_CLOSE: u64 = 36;
const SYS_TCP_STATUS: u64 = 37;
const SYS_HTTP_GET: u64 = 38;
const SYS_CONSOLE_AVAILABLE: u64 = 40;
const SYS_CONSOLE_READ: u64 = 41;
const SYS_PS_LIST: u64 = 50;
const SYS_KILL: u64 = 51;
const SYS_CPU_INFO: u64 = 52;
const SYS_SHUTDOWN: u64 = 60;
const SYS_SHOULD_CANCEL: u64 = 61;
const SYS_RANDOM: u64 = 62;
const SYS_ENV_GET: u64 = 63;
const SYS_KLOG_GET: u64 = 64;
const SYS_SERVICE_LIST: u64 = 70;
const SYS_SERVICE_START: u64 = 71;
const SYS_SERVICE_STOP: u64 = 72;
const SYS_SERVICE_RUNNING: u64 = 73;
const SYS_NET_INFO: u64 = 80;
const SYS_HEAP_STATS: u64 = 81;
const SYS_SLEEP: u64 = 82;



// ═══════════════════════════════════════════════════════════════════════════════
// Low-level syscall wrappers
// ═══════════════════════════════════════════════════════════════════════════════

#[inline(always)]
fn syscall0(num: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "ecall",
            in("a7") num,
            lateout("a0") ret,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
fn syscall1(num: u64, a0: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "ecall",
            in("a7") num,
            inlateout("a0") a0 as i64 => ret,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
fn syscall2(num: u64, a0: u64, a1: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "ecall",
            in("a7") num,
            inlateout("a0") a0 as i64 => ret,
            in("a1") a1,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
fn syscall3(num: u64, a0: u64, a1: u64, a2: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "ecall",
            in("a7") num,
            inlateout("a0") a0 as i64 => ret,
            in("a1") a1,
            in("a2") a2,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
fn syscall4(num: u64, a0: u64, a1: u64, a2: u64, a3: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "ecall",
            in("a7") num,
            inlateout("a0") a0 as i64 => ret,
            in("a1") a1,
            in("a2") a2,
            in("a3") a3,
            options(nostack)
        );
    }
    ret
}

// ═══════════════════════════════════════════════════════════════════════════════
// Raw Syscall Functions (matching WASM extern "C" declarations)
// ═══════════════════════════════════════════════════════════════════════════════

/// Print a string to the console
#[inline]
pub fn print(ptr: *const u8, len: usize) {
    syscall2(SYS_PRINT, ptr as u64, len as u64);
}

/// Get current time in milliseconds
#[inline]
pub fn time() -> i64 {
    syscall0(SYS_TIME)
}

/// Exit process with code
#[inline]
pub fn exit(code: i32) -> ! {
    syscall1(SYS_EXIT, code as u64);
    loop {
        unsafe { asm!("wfi", options(nomem, nostack)); }
    }
}

/// Get argument count
#[inline]
pub fn arg_count() -> i32 {
    syscall0(SYS_ARG_COUNT) as i32
}

/// Get argument at index
#[inline]
pub fn arg_get(index: i32, buf_ptr: *mut u8, buf_len: i32) -> i32 {
    syscall3(SYS_ARG_GET, index as u64, buf_ptr as u64, buf_len as u64) as i32
}

/// Get current working directory
#[inline]
pub fn cwd_get(buf_ptr: *mut u8, buf_len: i32) -> i32 {
    syscall2(SYS_CWD_GET, buf_ptr as u64, buf_len as u64) as i32
}

/// Set current working directory
#[inline]
pub fn cwd_set(path_ptr: *const u8, path_len: i32) -> i32 {
    syscall2(SYS_CWD_SET, path_ptr as u64, path_len as u64) as i32
}

/// Check if file exists
#[inline]
pub fn fs_exists(path_ptr: *const u8, path_len: i32) -> i32 {
    syscall2(SYS_FS_EXISTS, path_ptr as u64, path_len as u64) as i32
}

/// Read file
#[inline]
pub fn fs_read(path_ptr: *const u8, path_len: i32, buf_ptr: *mut u8, buf_len: i32) -> i32 {
    syscall4(SYS_FS_READ, path_ptr as u64, path_len as u64, buf_ptr as u64, buf_len as u64) as i32
}

/// Write file
#[inline]
pub fn fs_write(path_ptr: *const u8, path_len: i32, data_ptr: *const u8, data_len: i32) -> i32 {
    syscall4(SYS_FS_WRITE, path_ptr as u64, path_len as u64, data_ptr as u64, data_len as u64) as i32
}

/// List files
#[inline]
pub fn fs_list(buf_ptr: *mut u8, buf_len: i32) -> i32 {
    syscall2(SYS_FS_LIST, buf_ptr as u64, buf_len as u64) as i32
}

/// List directory
#[inline]
pub fn fs_list_dir(path_ptr: *const u8, path_len: i32, buf_ptr: *mut u8, buf_len: i32) -> i32 {
    syscall4(SYS_FS_LIST_DIR, path_ptr as u64, path_len as u64, buf_ptr as u64, buf_len as u64) as i32
}

/// Get file stat
#[inline]
pub fn fs_stat(path_ptr: *const u8, path_len: i32, out_ptr: *mut u8) -> i32 {
    syscall3(SYS_FS_STAT, path_ptr as u64, path_len as u64, out_ptr as u64) as i32
}

/// Create directory
#[inline]
pub fn fs_mkdir(path_ptr: *const u8, path_len: i32) -> i32 {
    syscall2(SYS_FS_MKDIR, path_ptr as u64, path_len as u64) as i32
}

/// Remove file or directory
#[inline]
pub fn fs_remove(path_ptr: *const u8, path_len: i32) -> i32 {
    syscall2(SYS_FS_REMOVE, path_ptr as u64, path_len as u64) as i32
}

/// Check if path is a directory
#[inline]
pub fn fs_is_dir(path_ptr: *const u8, path_len: i32) -> i32 {
    syscall2(SYS_FS_IS_DIR, path_ptr as u64, path_len as u64) as i32
}


/// Network available
#[inline]
pub fn net_available() -> i32 {
    syscall0(SYS_NET_AVAILABLE) as i32
}

/// DNS resolve
#[inline]
pub fn dns_resolve(host_ptr: *const u8, host_len: i32, ip_buf_ptr: *mut u8, ip_buf_len: i32) -> i32 {
    syscall4(SYS_DNS_RESOLVE, host_ptr as u64, host_len as u64, ip_buf_ptr as u64, ip_buf_len as u64) as i32
}

/// Send ping
#[inline]
pub fn send_ping(ip_ptr: *const u8, seq: i32, timeout_ms: i32, out_ptr: *mut u8) -> i32 {
    syscall4(SYS_SEND_PING, ip_ptr as u64, seq as u64, timeout_ms as u64, out_ptr as u64) as i32
}

/// TCP connect
#[inline]
pub fn tcp_connect(ip_ptr: *const u8, _ip_len: i32, port: i32) -> i32 {
    syscall2(SYS_TCP_CONNECT, ip_ptr as u64, port as u64) as i32
}

/// TCP send
#[inline]
pub fn tcp_send(data_ptr: *const u8, data_len: i32) -> i32 {
    syscall2(SYS_TCP_SEND, data_ptr as u64, data_len as u64) as i32
}

/// TCP recv
#[inline]
pub fn tcp_recv(buf_ptr: *mut u8, buf_len: i32, _timeout_ms: i32) -> i32 {
    syscall2(SYS_TCP_RECV, buf_ptr as u64, buf_len as u64) as i32
}

/// TCP close
#[inline]
pub fn tcp_close() -> i32 {
    syscall0(SYS_TCP_CLOSE) as i32
}

/// TCP status
#[inline]
pub fn tcp_status() -> i32 {
    syscall0(SYS_TCP_STATUS) as i32
}

/// HTTP get
#[inline]
pub fn http_get(url_ptr: *const u8, url_len: i32, resp_ptr: *mut u8, resp_len: i32) -> i32 {
    syscall4(SYS_HTTP_GET, url_ptr as u64, url_len as u64, resp_ptr as u64, resp_len as u64) as i32
}

/// Console available
#[inline]
pub fn console_available() -> i32 {
    syscall0(SYS_CONSOLE_AVAILABLE) as i32
}

/// Console read
#[inline]
pub fn console_read(buf_ptr: *mut u8, buf_len: i32) -> i32 {
    syscall2(SYS_CONSOLE_READ, buf_ptr as u64, buf_len as u64) as i32
}

/// PS list
#[inline]
pub fn ps_list(buf_ptr: *mut u8, buf_len: i32) -> i32 {
    syscall2(SYS_PS_LIST, buf_ptr as u64, buf_len as u64) as i32
}

/// Kill process
#[inline]
pub fn kill(pid: i32) -> i32 {
    syscall1(SYS_KILL, pid as u64) as i32
}

/// Shutdown
#[inline]
pub fn shutdown() -> ! {
    syscall0(SYS_SHUTDOWN);
    loop {
        unsafe { asm!("wfi", options(nomem, nostack)); }
    }
}

/// Should cancel
#[inline]
pub fn should_cancel() -> i32 {
    syscall0(SYS_SHOULD_CANCEL) as i32
}

/// Random
#[inline]
pub fn random(buf_ptr: *mut u8, buf_len: i32) -> i32 {
    syscall2(SYS_RANDOM, buf_ptr as u64, buf_len as u64) as i32
}

/// Env get
#[inline]
pub fn env_get(key_ptr: *const u8, key_len: i32, val_ptr: *mut u8, val_len: i32) -> i32 {
    syscall4(SYS_ENV_GET, key_ptr as u64, key_len as u64, val_ptr as u64, val_len as u64) as i32
}

/// Klog get
#[inline]
pub fn klog_get(count: i32, buf_ptr: *mut u8, buf_len: i32) -> i32 {
    syscall3(SYS_KLOG_GET, count as u64, buf_ptr as u64, buf_len as u64) as i32
}

/// CPU info
#[inline]
pub fn cpu_info(info_type: i32, out_ptr: *mut u8) -> i32 {
    syscall2(SYS_CPU_INFO, info_type as u64, out_ptr as u64) as i32
}

/// List services
#[inline]
pub fn service_list(buf_ptr: *mut u8, buf_len: i32) -> i32 {
    syscall2(SYS_SERVICE_LIST, buf_ptr as u64, buf_len as u64) as i32
}

/// Start service
#[inline]
pub fn service_start(name_ptr: *const u8, name_len: i32) -> i32 {
    syscall2(SYS_SERVICE_START, name_ptr as u64, name_len as u64) as i32
}

/// Stop service
#[inline]
pub fn service_stop(name_ptr: *const u8, name_len: i32) -> i32 {
    syscall2(SYS_SERVICE_STOP, name_ptr as u64, name_len as u64) as i32
}

/// Get running services
#[inline]
pub fn service_running(buf_ptr: *mut u8, buf_len: i32) -> i32 {
    syscall2(SYS_SERVICE_RUNNING, buf_ptr as u64, buf_len as u64) as i32
}

/// Get network information: IP[4], MAC[6], Gateway[4], DNS[4], prefix_len[1] = 19 bytes
#[inline]
pub fn net_info(out_ptr: *mut u8, out_len: i32) -> i32 {
    syscall2(SYS_NET_INFO, out_ptr as u64, out_len as u64) as i32
}

/// Get heap statistics: used_bytes[8], total_bytes[8] = 16 bytes
#[inline]
pub fn heap_stats(out_ptr: *mut u8) -> i32 {
    syscall1(SYS_HEAP_STATS, out_ptr as u64) as i32
}

/// Sleep for the given number of milliseconds
#[inline]
pub fn sleep_ms(ms: u64) -> i32 {
    syscall1(SYS_SLEEP, ms) as i32
}


// ═══════════════════════════════════════════════════════════════════════════════
// Higher-level helpers (same as WASM module)
// ═══════════════════════════════════════════════════════════════════════════════

/// Print a string to the console
pub fn console_log(s: &str) {
    print(s.as_ptr(), s.len());
}

/// Get current time in milliseconds
pub fn get_time() -> i64 {
    time()
}

/// Get number of arguments
pub fn argc() -> usize {
    arg_count() as usize
}

/// Get argument at index
pub fn argv(index: usize, buf: &mut [u8]) -> Option<usize> {
    let len = arg_get(index as i32, buf.as_mut_ptr(), buf.len() as i32);
    if len >= 0 { Some(len as usize) } else { None }
}

/// Get current working directory
pub fn get_cwd(buf: &mut [u8]) -> Option<usize> {
    let len = cwd_get(buf.as_mut_ptr(), buf.len() as i32);
    if len >= 0 { Some(len as usize) } else { None }
}

/// Set current working directory
pub fn set_cwd(path: &str) -> bool {
    cwd_set(path.as_ptr(), path.len() as i32) == 0
}

/// Check if file exists
pub fn file_exists(path: &str) -> bool {
    fs_exists(path.as_ptr(), path.len() as i32) == 1
}

/// Read file contents
pub fn read_file(path: &str, buf: &mut [u8]) -> Option<usize> {
    let len = fs_read(path.as_ptr(), path.len() as i32, buf.as_mut_ptr(), buf.len() as i32);
    if len >= 0 { Some(len as usize) } else { None }
}

/// Write file
pub fn write_file(path: &str, data: &[u8]) -> bool {
    let written = fs_write(path.as_ptr(), path.len() as i32, data.as_ptr(), data.len() as i32);
    written >= 0
}

/// List files
pub fn list_files(buf: &mut [u8]) -> Option<usize> {
    let len = fs_list(buf.as_mut_ptr(), buf.len() as i32);
    if len >= 0 { Some(len as usize) } else { None }
}

/// List directory
pub fn list_dir(path: &str, buf: &mut [u8]) -> Option<usize> {
    let len = fs_list_dir(path.as_ptr(), path.len() as i32, buf.as_mut_ptr(), buf.len() as i32);
    if len >= 0 { Some(len as usize) } else { None }
}

/// File stat result
pub struct FileStat {
    pub size: u32,
    pub exists: bool,
    pub is_dir: bool,
}

/// Get file stats
pub fn file_stat(path: &str) -> Option<FileStat> {
    let mut out = [0u8; 6];
    let result = fs_stat(path.as_ptr(), path.len() as i32, out.as_mut_ptr());
    if result == 0 {
        Some(FileStat {
            size: u32::from_le_bytes([out[0], out[1], out[2], out[3]]),
            exists: out[4] != 0,
            is_dir: out[5] != 0,
        })
    } else {
        None
    }
}

/// Create directory
pub fn mkdir(path: &str) -> bool {
    fs_mkdir(path.as_ptr(), path.len() as i32) == 0
}

/// Remove file or directory
pub fn remove_file(path: &str) -> bool {
    fs_remove(path.as_ptr(), path.len() as i32) == 0
}

/// Check if path is a directory
pub fn is_dir(path: &str) -> bool {
    fs_is_dir(path.as_ptr(), path.len() as i32) == 1
}


/// Network available
pub fn is_net_available() -> bool {
    net_available() == 1
}

/// Get klog
pub fn get_klog(count: usize, buf: &mut [u8]) -> Option<usize> {
    let len = klog_get(count as i32, buf.as_mut_ptr(), buf.len() as i32);
    if len >= 0 { Some(len as usize) } else { None }
}

/// HTTP fetch
pub fn http_fetch(url: &str, buf: &mut [u8]) -> Option<usize> {
    let len = http_get(url.as_ptr(), url.len() as i32, buf.as_mut_ptr(), buf.len() as i32);
    if len >= 0 { Some(len as usize) } else { None }
}

/// DNS resolve
pub fn resolve_dns(hostname: &str, ip_buf: &mut [u8; 4]) -> bool {
    dns_resolve(hostname.as_ptr(), hostname.len() as i32, ip_buf.as_mut_ptr(), 4) == 4
}

/// Get environment variable
pub fn getenv(key: &str, buf: &mut [u8]) -> Option<usize> {
    let len = env_get(key.as_ptr(), key.len() as i32, buf.as_mut_ptr(), buf.len() as i32);
    if len >= 0 { Some(len as usize) } else { None }
}

/// Get random bytes
pub fn get_random(buf: &mut [u8]) -> bool {
    random(buf.as_mut_ptr(), buf.len() as i32) >= 0
}

/// Network info structure
pub struct NetInfo {
    pub ip: [u8; 4],
    pub mac: [u8; 6],
    pub gateway: [u8; 4],
    pub dns: [u8; 4],
    pub prefix_len: u8,
}

/// Get network information
pub fn get_net_info() -> Option<NetInfo> {
    let mut buf = [0u8; 19];
    let result = net_info(buf.as_mut_ptr(), 19);
    if result >= 19 {
        Some(NetInfo {
            ip: [buf[0], buf[1], buf[2], buf[3]],
            mac: [buf[4], buf[5], buf[6], buf[7], buf[8], buf[9]],
            gateway: [buf[10], buf[11], buf[12], buf[13]],
            dns: [buf[14], buf[15], buf[16], buf[17]],
            prefix_len: buf[18],
        })
    } else {
        None
    }
}

/// Heap statistics structure
pub struct HeapStats {
    pub used_bytes: u64,
    pub total_bytes: u64,
}

/// Get heap statistics
pub fn get_heap_stats() -> HeapStats {
    let mut buf = [0u8; 16];
    heap_stats(buf.as_mut_ptr());
    HeapStats {
        used_bytes: u64::from_le_bytes([buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7]]),
        total_bytes: u64::from_le_bytes([buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15]]),
    }
}

/// Sleep for milliseconds
pub fn sleep(ms: u64) {
    sleep_ms(ms);
}


/// Power off system
pub fn poweroff() -> ! {
    shutdown()
}

/// Get process list
pub fn get_ps_list(buf: &mut [u8]) -> Option<usize> {
    let len = ps_list(buf.as_mut_ptr(), buf.len() as i32);
    if len >= 0 { Some(len as usize) } else { None }
}

/// Kill result
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum KillResult {
    Success,
    NotFound,
    CannotKill,
    InvalidPid,
}

/// Kill process
pub fn kill_process(pid: u32) -> KillResult {
    let result = kill(pid as i32);
    match result {
        0 => KillResult::Success,
        -2 => KillResult::CannotKill,
        _ => if pid == 0 { KillResult::InvalidPid } else { KillResult::NotFound }
    }
}

/// Format integer to string
pub fn int_to_str(mut n: i64, buf: &mut [u8]) -> &str {
    if n == 0 {
        buf[0] = b'0';
        return unsafe { core::str::from_utf8_unchecked(&buf[..1]) };
    }

    let negative = n < 0;
    if negative { n = -n; }

    let mut i = buf.len();
    while n > 0 && i > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }

    if negative && i > 0 {
        i -= 1;
        buf[i] = b'-';
    }

    unsafe { core::str::from_utf8_unchecked(&buf[i..]) }
}

/// Print integer
pub fn print_int(n: i64) {
    let mut buf = [0u8; 20];
    let s = int_to_str(n, &mut buf);
    console_log(s);
}

/// Format IPv4
pub fn format_ipv4(ip: &[u8; 4], buf: &mut [u8]) -> usize {
    let mut pos = 0;
    for (i, &octet) in ip.iter().enumerate() {
        if i > 0 && pos < buf.len() {
            buf[pos] = b'.';
            pos += 1;
        }
        let mut num = octet;
        let mut digits = [0u8; 3];
        let mut digit_count = 0;
        if num == 0 {
            digits[0] = b'0';
            digit_count = 1;
        } else {
            while num > 0 {
                digits[digit_count] = b'0' + (num % 10);
                num /= 10;
                digit_count += 1;
            }
        }
        for j in (0..digit_count).rev() {
            if pos < buf.len() {
                buf[pos] = digits[j];
                pos += 1;
            }
        }
    }
    pos
}

/// Format MAC address as XX:XX:XX:XX:XX:XX
pub fn format_mac(mac: &[u8; 6], buf: &mut [u8]) -> usize {
    const HEX: [u8; 16] = *b"0123456789abcdef";
    
    let mut pos = 0;
    for (i, &byte) in mac.iter().enumerate() {
        if i > 0 && pos < buf.len() {
            buf[pos] = b':';
            pos += 1;
        }
        if pos + 1 < buf.len() {
            buf[pos] = HEX[(byte >> 4) as usize];
            buf[pos + 1] = HEX[(byte & 0xf) as usize];
            pos += 2;
        }
    }
    pos
}


/// TCP status
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TcpStatus {
    Closed = 0,
    Connecting = 1,
    Connected = 2,
    Failed = 3,
}

/// TCP connect by IP
pub fn tcp_connect_ip(ip: &[u8; 4], port: u16) -> bool {
    tcp_connect(ip.as_ptr(), 4, port as i32) == 0
}

/// TCP send data
pub fn tcp_send_data(data: &[u8]) -> Option<usize> {
    let result = tcp_send(data.as_ptr(), data.len() as i32);
    if result >= 0 { Some(result as usize) } else { None }
}

/// TCP receive data
pub fn tcp_recv_data(buf: &mut [u8], timeout_ms: u32) -> Option<usize> {
    let result = tcp_recv(buf.as_mut_ptr(), buf.len() as i32, timeout_ms as i32);
    if result >= 0 { Some(result as usize) } else { None }
}

/// TCP disconnect
pub fn tcp_disconnect() -> bool {
    tcp_close() == 0
}

/// TCP get status
pub fn tcp_get_status() -> TcpStatus {
    match tcp_status() {
        1 => TcpStatus::Connecting,
        2 => TcpStatus::Connected,
        3 => TcpStatus::Failed,
        _ => TcpStatus::Closed,
    }
}

/// Console available check
pub fn is_console_available() -> bool {
    console_available() == 1
}

/// Read from console
pub fn read_console(buf: &mut [u8]) -> usize {
    let result = console_read(buf.as_mut_ptr(), buf.len() as i32);
    if result > 0 { result as usize } else { 0 }
}

/// Ping result
pub enum PingResult {
    Success { rtt_ms: u32 },
    Timeout,
    NetworkError,
}

/// Send ping
pub fn ping(ip: &[u8; 4], seq: u16, timeout_ms: u32) -> PingResult {
    let mut out = [0u8; 4];
    let result = send_ping(ip.as_ptr(), seq as i32, timeout_ms as i32, out.as_mut_ptr());
    match result {
        0 => PingResult::Success { rtt_ms: u32::from_le_bytes(out) },
        -1 => PingResult::Timeout,
        _ => PingResult::NetworkError,
    }
}
