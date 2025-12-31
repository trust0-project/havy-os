//! Syscall numbers for native RISC-V userspace binaries
//!
//! These syscall numbers are used by userspace programs to invoke kernel
//! services via the RISC-V `ecall` instruction.
//!
//! Calling convention:
//! - a7 = syscall number
//! - a0-a5 = arguments
//! - a0 = return value (or error code if negative)

// ═══════════════════════════════════════════════════════════════════════════════
// Core System Calls
// ═══════════════════════════════════════════════════════════════════════════════

/// Print string to console: print(ptr, len)
pub const SYS_PRINT: u64 = 0;
/// Get current time in milliseconds: time() -> i64
pub const SYS_TIME: u64 = 1;
/// Exit process: exit(code) -> !
pub const SYS_EXIT: u64 = 2;

// ═══════════════════════════════════════════════════════════════════════════════
// Argument Handling
// ═══════════════════════════════════════════════════════════════════════════════

/// Get argument count: arg_count() -> i32
pub const SYS_ARG_COUNT: u64 = 10;
/// Get argument by index: arg_get(index, buf_ptr, buf_len) -> i32 (length or -1)
pub const SYS_ARG_GET: u64 = 11;
/// Get current working directory: cwd_get(buf_ptr, buf_len) -> i32
pub const SYS_CWD_GET: u64 = 12;
/// Set current working directory: cwd_set(path_ptr, path_len) -> i32
pub const SYS_CWD_SET: u64 = 13;

// ═══════════════════════════════════════════════════════════════════════════════
// Filesystem Operations
// ═══════════════════════════════════════════════════════════════════════════════

/// Check if file exists: fs_exists(path_ptr, path_len) -> i32 (1 or 0)
pub const SYS_FS_EXISTS: u64 = 20;
/// Read file: fs_read(path_ptr, path_len, buf_ptr, buf_len) -> i32
pub const SYS_FS_READ: u64 = 21;
/// Write file: fs_write(path_ptr, path_len, data_ptr, data_len) -> i32
pub const SYS_FS_WRITE: u64 = 22;
/// List files in root: fs_list(buf_ptr, buf_len) -> i32
pub const SYS_FS_LIST: u64 = 23;
/// Get file stats: fs_stat(path_ptr, path_len, out_ptr) -> i32
pub const SYS_FS_STAT: u64 = 24;
/// Remove file: fs_remove(path_ptr, path_len) -> i32
pub const SYS_FS_REMOVE: u64 = 25;
/// Create directory: fs_mkdir(path_ptr, path_len) -> i32
pub const SYS_FS_MKDIR: u64 = 26;
/// Check if path is directory: fs_is_dir(path_ptr, path_len) -> i32
pub const SYS_FS_IS_DIR: u64 = 27;
/// List files in directory: fs_list_dir(path_ptr, path_len, buf_ptr, buf_len) -> i32
pub const SYS_FS_LIST_DIR: u64 = 28;

// ═══════════════════════════════════════════════════════════════════════════════
// Network Operations
// ═══════════════════════════════════════════════════════════════════════════════

/// Check network availability: net_available() -> i32
pub const SYS_NET_AVAILABLE: u64 = 30;
/// DNS resolve: dns_resolve(host_ptr, host_len, ip_buf_ptr, ip_buf_len) -> i32
pub const SYS_DNS_RESOLVE: u64 = 31;
/// Send ICMP ping: send_ping(ip_ptr, seq, timeout_ms, out_ptr) -> i32
pub const SYS_SEND_PING: u64 = 32;
/// TCP connect: tcp_connect(ip_ptr, port) -> i32
pub const SYS_TCP_CONNECT: u64 = 33;
/// TCP send: tcp_send(data_ptr, data_len) -> i32
pub const SYS_TCP_SEND: u64 = 34;
/// TCP receive: tcp_recv(buf_ptr, buf_len) -> i32
pub const SYS_TCP_RECV: u64 = 35;
/// TCP close: tcp_close() -> i32
pub const SYS_TCP_CLOSE: u64 = 36;
/// TCP status: tcp_status() -> i32
pub const SYS_TCP_STATUS: u64 = 37;
/// HTTP GET: http_get(url_ptr, url_len, resp_ptr, resp_len) -> i32
pub const SYS_HTTP_GET: u64 = 38;

// ═══════════════════════════════════════════════════════════════════════════════
// Console I/O
// ═══════════════════════════════════════════════════════════════════════════════

/// Check console input available: console_available() -> i32
pub const SYS_CONSOLE_AVAILABLE: u64 = 40;
/// Read from console: console_read(buf_ptr, buf_len) -> i32
pub const SYS_CONSOLE_READ: u64 = 41;

// ═══════════════════════════════════════════════════════════════════════════════
// Process Management
// ═══════════════════════════════════════════════════════════════════════════════

/// List processes: ps_list(buf_ptr, buf_len) -> i32
pub const SYS_PS_LIST: u64 = 50;
/// Kill process: kill(pid) -> i32
pub const SYS_KILL: u64 = 51;
/// Get CPU info: cpu_info(cpu_id, out_ptr) -> i32
pub const SYS_CPU_INFO: u64 = 52;

// ═══════════════════════════════════════════════════════════════════════════════
// System Operations
// ═══════════════════════════════════════════════════════════════════════════════

/// Shutdown system: shutdown() -> !
pub const SYS_SHUTDOWN: u64 = 60;
/// Check if cancel requested: should_cancel() -> i32
pub const SYS_SHOULD_CANCEL: u64 = 61;
/// Get random bytes: random(buf_ptr, buf_len) -> i32
pub const SYS_RANDOM: u64 = 62;
/// Get environment variable: env_get(key_ptr, key_len, val_ptr, val_len) -> i32
pub const SYS_ENV_GET: u64 = 63;
/// Get kernel log: klog_get(count, buf_ptr, buf_len) -> i32
pub const SYS_KLOG_GET: u64 = 64;

// ═══════════════════════════════════════════════════════════════════════════════
// Service Management
// ═══════════════════════════════════════════════════════════════════════════════

/// List services: service_list(buf_ptr, buf_len) -> i32
pub const SYS_SERVICE_LIST: u64 = 70;
/// Start service: service_start(name_ptr, name_len) -> i32
pub const SYS_SERVICE_START: u64 = 71;
/// Stop service: service_stop(name_ptr, name_len) -> i32
pub const SYS_SERVICE_STOP: u64 = 72;
/// Get running services: service_running(buf_ptr, buf_len) -> i32
pub const SYS_SERVICE_RUNNING: u64 = 73;

// ═══════════════════════════════════════════════════════════════════════════════
// Extended System Calls
// ═══════════════════════════════════════════════════════════════════════════════

/// Get network info: net_info(out_ptr, out_len) -> i32
/// Returns: IP[4], MAC[6], Gateway[4], DNS[4], prefix_len[1] = 19 bytes
pub const SYS_NET_INFO: u64 = 80;

/// Get heap statistics: heap_stats(out_ptr) -> i32
/// Returns: used_bytes[8], total_bytes[8] = 16 bytes
pub const SYS_HEAP_STATS: u64 = 81;

/// Sleep: sleep_ms(milliseconds) -> i32
pub const SYS_SLEEP: u64 = 82;

