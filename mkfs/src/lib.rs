// mkfs/src/lib.rs
//
// This file serves as the runtime library for native RISC-V userspace binaries.
// The System Call API is provided via RISC-V `ecall` instructions.
//
// For the host tool (mkfs binary), this module is mostly ignored.

// Remove all std when targeting RISC-V
#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

// ═══════════════════════════════════════════════════════════════════════════════
// Native RISC-V Syscall Module
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "riscv64")]
pub mod riscv;

// Re-export everything from riscv module at crate root for convenience
#[cfg(target_arch = "riscv64")]
pub use riscv::*;

// ═══════════════════════════════════════════════════════════════════════════════
// Entry Point for Native RISC-V Binaries
// ═══════════════════════════════════════════════════════════════════════════════

/// Entry point called by kernel after ELF load
/// Calls the binary's main() function and exits cleanly
#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Call the binary's main function (defined in each bin/*.rs)
    extern "Rust" {
        fn main();
    }
    unsafe { main(); }
    
    // Exit cleanly
    riscv::exit(0);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Panic Handler
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "riscv64")]
#[panic_handler]
fn panic_handler(info: &core::panic::PanicInfo) -> ! {
    // Try to print panic message
    let msg = "PANIC: ";
    riscv::print(msg.as_ptr(), msg.len());
    
    // If we can get location info, print it
    if let Some(location) = info.location() {
        let file = location.file();
        riscv::print(file.as_ptr(), file.len());
        riscv::console_log(":");
        riscv::print_int(location.line() as i64);
    }
    
    riscv::console_log("\n");
    riscv::exit(1);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Host/Non-RISC-V Stubs (for cargo check on host)
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(not(target_arch = "riscv64"))]
pub fn console_log(_s: &str) {}
#[cfg(not(target_arch = "riscv64"))]
pub fn print(_ptr: *const u8, _len: usize) {}
#[cfg(not(target_arch = "riscv64"))]
pub fn print_int(_n: i64) {}
#[cfg(not(target_arch = "riscv64"))]
pub fn argc() -> usize { 0 }
#[cfg(not(target_arch = "riscv64"))]
pub fn argv(_index: usize, _buf: &mut [u8]) -> Option<usize> { None }
#[cfg(not(target_arch = "riscv64"))]
pub fn get_cwd(_buf: &mut [u8]) -> Option<usize> { None }
#[cfg(not(target_arch = "riscv64"))]
pub fn get_time() -> i64 { 0 }
#[cfg(not(target_arch = "riscv64"))]
pub fn poweroff() -> ! { loop {} }
#[cfg(not(target_arch = "riscv64"))]
pub fn is_net_available() -> bool { false }
#[cfg(not(target_arch = "riscv64"))]
pub fn env_get(_key_ptr: *const u8, _key_len: i32, _val_ptr: *mut u8, _val_len: i32) -> i32 { -1 }
#[cfg(not(target_arch = "riscv64"))]
pub fn arg_count() -> i32 { 0 }
#[cfg(not(target_arch = "riscv64"))]
pub fn arg_get(_index: i32, _buf_ptr: *mut u8, _buf_len: i32) -> i32 { -1 }
#[cfg(not(target_arch = "riscv64"))]
pub fn cwd_set(_path_ptr: *const u8, _path_len: i32) -> i32 { -1 }
#[cfg(not(target_arch = "riscv64"))]
pub fn fs_read(_path_ptr: *const u8, _path_len: i32, _buf_ptr: *mut u8, _buf_len: i32) -> i32 { -1 }
#[cfg(not(target_arch = "riscv64"))]
pub fn ps_list(_buf_ptr: *mut u8, _buf_len: i32) -> i32 { -1 }
#[cfg(not(target_arch = "riscv64"))]
pub fn get_klog(_count: usize, _buf: &mut [u8]) -> Option<usize> { None }
#[cfg(not(target_arch = "riscv64"))]
pub fn kill_process(_pid: u32) -> KillResult { KillResult::NotFound }

// Additional stubs for FS commands
#[cfg(not(target_arch = "riscv64"))]
pub fn fs_list(_buf_ptr: *mut u8, _buf_len: i32) -> i32 { -1 }
#[cfg(not(target_arch = "riscv64"))]
pub fn fs_list_dir(_path_ptr: *const u8, _path_len: i32, _buf_ptr: *mut u8, _buf_len: i32) -> i32 { -1 }
#[cfg(not(target_arch = "riscv64"))]
pub fn list_files(_buf: &mut [u8]) -> Option<usize> { None }
#[cfg(not(target_arch = "riscv64"))]
pub fn list_dir(_path: &str, _buf: &mut [u8]) -> Option<usize> { None }
#[cfg(not(target_arch = "riscv64"))]
pub fn mkdir(_path: &str) -> bool { false }
#[cfg(not(target_arch = "riscv64"))]
pub fn fs_mkdir(_path_ptr: *const u8, _path_len: i32) -> i32 { -1 }
#[cfg(not(target_arch = "riscv64"))]
pub fn remove_file(_path: &str) -> bool { false }
#[cfg(not(target_arch = "riscv64"))]
pub fn is_dir(_path: &str) -> bool { false }
#[cfg(not(target_arch = "riscv64"))]
pub fn file_exists(_path: &str) -> bool { false }
#[cfg(not(target_arch = "riscv64"))]
pub fn read_file(_path: &str, _buf: &mut [u8]) -> Option<usize> { None }
#[cfg(not(target_arch = "riscv64"))]
pub fn write_file(_path: &str, _data: &[u8]) -> bool { false }
#[cfg(not(target_arch = "riscv64"))]
pub fn file_stat(_path: &str) -> Option<FileStat> { None }
#[cfg(not(target_arch = "riscv64"))]
pub fn getenv(_key: &str, _buf: &mut [u8]) -> Option<usize> { None }
#[cfg(not(target_arch = "riscv64"))]
pub fn set_cwd(_path: &str) -> bool { false }
#[cfg(not(target_arch = "riscv64"))]
pub fn get_ps_list(_buf: &mut [u8]) -> Option<usize> { None }
#[cfg(not(target_arch = "riscv64"))]
pub fn sleep(_ms: u64) {}

// Network stubs
#[cfg(not(target_arch = "riscv64"))]
pub fn resolve_dns(_hostname: &str, _ip_buf: &mut [u8; 4]) -> bool { false }
#[cfg(not(target_arch = "riscv64"))]
pub fn ping(_ip: &[u8; 4], _seq: i32, _timeout_ms: i32) -> PingResult { PingResult::Timeout }
#[cfg(not(target_arch = "riscv64"))]
pub fn http_fetch(_url: &str, _buf: &mut [u8]) -> Option<usize> { None }
#[cfg(not(target_arch = "riscv64"))]
pub fn tcp_connect(_ip: &[u8; 4], _port: u16) -> bool { false }
#[cfg(not(target_arch = "riscv64"))]
pub fn tcp_send(_data: &[u8]) -> i32 { -1 }
#[cfg(not(target_arch = "riscv64"))]
pub fn tcp_recv(_buf: &mut [u8], _timeout_ms: i32) -> i32 { -1 }
#[cfg(not(target_arch = "riscv64"))]
pub fn tcp_close() {}
#[cfg(not(target_arch = "riscv64"))]
pub fn should_cancel() -> i32 { 0 }

// TCP helper stubs
#[cfg(not(target_arch = "riscv64"))]
pub fn tcp_connect_ip(_ip: &[u8; 4], _port: u16) -> bool { false }
#[cfg(not(target_arch = "riscv64"))]
pub fn tcp_send_data(_data: &[u8]) -> Option<usize> { None }
#[cfg(not(target_arch = "riscv64"))]
pub fn tcp_recv_data(_buf: &mut [u8], _timeout_ms: u32) -> Option<usize> { None }
#[cfg(not(target_arch = "riscv64"))]
pub fn tcp_disconnect() -> bool { false }
#[cfg(not(target_arch = "riscv64"))]
pub fn tcp_get_status() -> TcpStatus { TcpStatus::Closed }
#[cfg(not(target_arch = "riscv64"))]
pub fn console_available() -> i32 { 0 }
#[cfg(not(target_arch = "riscv64"))]
pub fn read_console(_buf: &mut [u8]) -> usize { 0 }

#[cfg(not(target_arch = "riscv64"))]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TcpStatus {
    Closed = 0,
    Connecting = 1,
    Connected = 2,
    Failed = 3,
}


// System info stubs
#[cfg(not(target_arch = "riscv64"))]
pub fn get_heap_stats() -> HeapStats { HeapStats { used_bytes: 0, total_bytes: 0 } }
#[cfg(not(target_arch = "riscv64"))]
pub fn get_hart_count() -> usize { 0 }
#[cfg(not(target_arch = "riscv64"))]
pub fn get_version() -> &'static str { "" }
#[cfg(not(target_arch = "riscv64"))]
pub fn get_net_info() -> Option<NetInfo> { None }
#[cfg(not(target_arch = "riscv64"))]
pub fn net_info(_out_ptr: *mut u8, _out_len: i32) -> i32 { -1 }
#[cfg(not(target_arch = "riscv64"))]
pub fn heap_stats(_out_ptr: *mut u8) -> i32 { -1 }
#[cfg(not(target_arch = "riscv64"))]
pub fn sleep_ms(_ms: u64) -> i32 { 0 }


// Format helpers
#[cfg(not(target_arch = "riscv64"))]
pub fn format_ipv4(_ip: [u8; 4], _buf: &mut [u8]) -> usize { 0 }
#[cfg(not(target_arch = "riscv64"))]
pub fn format_mac(_mac: [u8; 6], _buf: &mut [u8]) -> usize { 0 }

// Service stubs
#[cfg(not(target_arch = "riscv64"))]
pub fn get_service_defs() -> &'static [&'static str] { &[] }
#[cfg(not(target_arch = "riscv64"))]
pub fn get_running_services() -> &'static [&'static str] { &[] }
#[cfg(not(target_arch = "riscv64"))]
pub fn start_service(_name: &str) -> bool { false }
#[cfg(not(target_arch = "riscv64"))]
pub fn stop_service(_name: &str) -> bool { false }
#[cfg(not(target_arch = "riscv64"))]
pub fn restart_service(_name: &str) -> bool { false }
#[cfg(not(target_arch = "riscv64"))]
pub fn get_service_status(_name: &str) -> ServiceStatus { ServiceStatus::Stopped }
#[cfg(not(target_arch = "riscv64"))]
pub fn service_list(_buf_ptr: *mut u8, _buf_len: i32) -> i32 { -1 }
#[cfg(not(target_arch = "riscv64"))]
pub fn service_start(_name_ptr: *const u8, _name_len: i32) -> i32 { -1 }
#[cfg(not(target_arch = "riscv64"))]
pub fn service_stop(_name_ptr: *const u8, _name_len: i32) -> i32 { -1 }
#[cfg(not(target_arch = "riscv64"))]
pub fn service_running(_buf_ptr: *mut u8, _buf_len: i32) -> i32 { -1 }


// Types
#[cfg(not(target_arch = "riscv64"))]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum KillResult {
    Success,
    NotFound,
    CannotKill,
    InvalidPid,
}

#[cfg(not(target_arch = "riscv64"))]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PingResult {
    Success { rtt_ms: u32 },
    Timeout,
    Error,
}

#[cfg(not(target_arch = "riscv64"))]
pub struct FileStat {
    pub size: u32,
    pub exists: bool,
    pub is_dir: bool,
}

#[cfg(not(target_arch = "riscv64"))]
pub struct NetInfo {
    pub ip: [u8; 4],
    pub mac: [u8; 6],
    pub gateway: [u8; 4],
    pub dns: [u8; 4],
    pub prefix_len: u8,
}

#[cfg(not(target_arch = "riscv64"))]
pub struct HeapStats {
    pub used_bytes: u64,
    pub total_bytes: u64,
}

#[cfg(not(target_arch = "riscv64"))]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatus {
    Running,
    Stopped,
    Unknown,
}

