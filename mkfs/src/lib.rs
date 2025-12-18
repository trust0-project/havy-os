// mkfs/src/lib.rs
//
// This file serves two purposes:
// 1. It is mostly ignored by the host tool (mkfs binary)
// 2. For WASM targets, it provides the System Call API and Panic Handler

// Use no_std when targeting WASM
#![cfg_attr(target_arch = "wasm32", no_std)]

// Only compile this module logic when targeting WASM
#[cfg(target_arch = "wasm32")]
pub mod syscalls {
    use core::panic::PanicInfo;

    // --- System Calls provided by Kernel ---
    extern "C" {
        /// Print a string to the console
        pub fn print(ptr: *const u8, len: usize);
        /// Get current time in milliseconds
        pub fn time() -> i64;
        /// Get number of command-line arguments
        pub fn arg_count() -> i32;
        /// Get argument at index into buffer, returns actual length or -1 on error
        pub fn arg_get(index: i32, buf_ptr: *mut u8, buf_len: i32) -> i32;
        /// Get current working directory into buffer, returns length or -1
        pub fn cwd_get(buf_ptr: *mut u8, buf_len: i32) -> i32;
        /// Set current working directory, returns 0 on success, -1 on error
        pub fn cwd_set(path_ptr: *const u8, path_len: i32) -> i32;
        /// Check if file exists (1 = yes, 0 = no)
        pub fn fs_exists(path_ptr: *const u8, path_len: i32) -> i32;
        /// Read file into buffer, returns bytes read or -1 on error
        pub fn fs_read(path_ptr: *const u8, path_len: i32, buf_ptr: *mut u8, buf_len: i32) -> i32;
        /// Write data to file, returns bytes written or -1 on error
        pub fn fs_write(path_ptr: *const u8, path_len: i32, data_ptr: *const u8, data_len: i32)
            -> i32;
        /// List files in root directory, returns "name:size\n" format into buffer
        pub fn fs_list(buf_ptr: *mut u8, buf_len: i32) -> i32;
        /// List files in a specific directory, returns "name:size\n" format into buffer
        pub fn fs_list_dir(
            path_ptr: *const u8,
            path_len: i32,
            buf_ptr: *mut u8,
            buf_len: i32,
        ) -> i32;
        /// Get file stats. Writes to out_ptr: u32 size, u8 exists, u8 is_dir
        /// Returns 0 on success, -1 on error
        pub fn fs_stat(path_ptr: *const u8, path_len: i32, out_ptr: *mut u8) -> i32;
        /// Create a directory marker, returns 0 on success, -1 on error
        pub fn fs_mkdir(path_ptr: *const u8, path_len: i32) -> i32;
        /// Get kernel log entries, returns data into buffer
        pub fn klog_get(count: i32, buf_ptr: *mut u8, buf_len: i32) -> i32;
        /// Check if network is available (1 = yes, 0 = no)
        pub fn net_available() -> i32;
        /// HTTP GET request, returns response length or -1 on error
        pub fn http_get(
            url_ptr: *const u8,
            url_len: i32,
            resp_ptr: *mut u8,
            resp_len: i32,
        ) -> i32;
        /// DNS resolve hostname to IP address. Writes 4 bytes (IPv4) to ip_buf.
        /// Returns 4 on success, -1 on error
        pub fn dns_resolve(
            host_ptr: *const u8,
            host_len: i32,
            ip_buf_ptr: *mut u8,
            ip_buf_len: i32,
        ) -> i32;
        /// Get environment variable value, returns length or -1 if not found
        pub fn env_get(
            key_ptr: *const u8,
            key_len: i32,
            val_ptr: *mut u8,
            val_len: i32,
        ) -> i32;
        /// Fill buffer with random bytes, returns bytes written or -1 on error
        pub fn random(buf_ptr: *mut u8, buf_len: i32) -> i32;
        /// Sleep for the specified number of milliseconds
        pub fn sleep_ms(ms: i32);
        /// Get disk usage stats. Writes to out_ptr: u64 used_bytes, u64 total_bytes
        /// Returns 0 on success, -1 on error
        pub fn disk_stats(out_ptr: *mut u8) -> i32;
        /// Get heap usage stats. Writes to out_ptr: u64 used_bytes, u64 total_bytes
        /// Returns 0 on success, -1 on error
        pub fn heap_stats(out_ptr: *mut u8) -> i32;
        /// Power off the system. Does not return.
        pub fn shutdown() -> !;
        
        // ═══════════════════════════════════════════════════════════════════
        // WASM Worker Syscalls - For multi-hart WASM execution
        // ═══════════════════════════════════════════════════════════════════
        
        /// Get number of WASM workers
        pub fn wasm_worker_count() -> i32;
        /// Get worker stats at index. Writes 36 bytes to out_ptr.
        /// Returns 0 on success, -1 on error.
        pub fn wasm_worker_stats(worker_idx: i32, out_ptr: *mut u8) -> i32;
        /// Submit WASM job. Returns job_id (>0) or -1 on error.
        /// target_hart: 0 = auto, 1+ = specific hart
        pub fn wasm_submit_job(
            wasm_ptr: *const u8,
            wasm_len: i32,
            args_ptr: *const u8,
            args_len: i32,
            target_hart: i32,
        ) -> i32;
        /// Get job status: 0=pending, 1=running, 2=completed, 3=failed, -1=not found
        pub fn wasm_job_status(job_id: i32) -> i32;
        /// Get total hart count (including primary)
        pub fn hart_count() -> i32;
        /// Get CPU info for a specific CPU
        /// Writes to out_ptr: u32 state, u32 running_pid, u8 utilization, u64 context_switches = 17 bytes
        /// Returns 0 on success, -1 if CPU not online or error
        pub fn cpu_info(cpu_id: i32, out_ptr: *mut u8) -> i32;
        /// Get list of all processes/tasks
        /// Format: "pid:name:state:priority:cpu_time:uptime\n" per task
        /// Returns bytes written or -1 on error
        pub fn ps_list(buf_ptr: *mut u8, buf_len: i32) -> i32;
        /// Kill a process by PID
        /// Returns 0 on success, -1 if not found, -2 if cannot kill (init)
        pub fn kill(pid: i32) -> i32;
        
        // ═══════════════════════════════════════════════════════════════════
        // Additional Syscalls - For migrated native commands
        // ═══════════════════════════════════════════════════════════════════
        
        /// Get kernel version string
        /// Returns bytes written or -1 on error
        pub fn version(buf_ptr: *mut u8, buf_len: i32) -> i32;
        /// Check if filesystem is available (1 = yes, 0 = no)
        pub fn fs_available() -> i32;
        /// Get network info. Writes 19 bytes: IP(4) + Gateway(4) + DNS(4) + MAC(6) + prefix_len(1)
        /// Returns 0 on success, -1 if network not available
        pub fn net_info(out_ptr: *mut u8) -> i32;
        /// Remove a file. Returns 0 on success, -1 on error
        pub fn fs_remove(path_ptr: *const u8, path_len: i32) -> i32;
        /// Check if path is a directory. Returns 1 if dir, 0 if not, -1 on error
        pub fn fs_is_dir(path_ptr: *const u8, path_len: i32) -> i32;
        /// Get available service definitions. Format: "name:description\n"
        /// Returns bytes written or -1 on error
        pub fn service_list_defs(buf_ptr: *mut u8, buf_len: i32) -> i32;
        /// Get running services. Format: "name:status:pid\n"
        /// Returns bytes written or -1 on error
        pub fn service_list_running(buf_ptr: *mut u8, buf_len: i32) -> i32;
        /// Start a service. Returns 0 on success, -1 on error
        pub fn service_start(name_ptr: *const u8, name_len: i32) -> i32;
        /// Stop a service. Returns 0 on success, -1 on error
        pub fn service_stop(name_ptr: *const u8, name_len: i32) -> i32;
        /// Restart a service. Returns 0 on success, -1 on error
        pub fn service_restart(name_ptr: *const u8, name_len: i32) -> i32;
        /// Get service status. Returns status string length or -1 if not found
        pub fn service_status(
            name_ptr: *const u8,
            name_len: i32,
            out_ptr: *mut u8,
            out_len: i32,
        ) -> i32;
        /// Send ping and wait for reply
        /// ip_ptr: 4 bytes IPv4 address
        /// seq: sequence number
        /// timeout_ms: timeout in milliseconds
        /// out_ptr: receives rtt_ms (4 bytes) on success
        /// Returns 0 on success, -1 on timeout, -2 on network error
        pub fn send_ping(ip_ptr: *const u8, seq: i32, timeout_ms: i32, out_ptr: *mut u8) -> i32;
        
        // ═══════════════════════════════════════════════════════════════════
        // TCP Socket Syscalls
        // ═══════════════════════════════════════════════════════════════════
        
        /// Connect to a TCP server. ip_ptr points to 4 bytes IPv4 address.
        /// Returns 0 on success (connection initiated), -1 on error.
        pub fn tcp_connect(ip_ptr: *const u8, ip_len: i32, port: i32) -> i32;
        /// Send data over TCP connection. Returns bytes sent or -1 on error.
        pub fn tcp_send(data_ptr: *const u8, data_len: i32) -> i32;
        /// Receive data from TCP. Returns bytes received, 0 if no data, -1 on error.
        pub fn tcp_recv(buf_ptr: *mut u8, buf_len: i32, timeout_ms: i32) -> i32;
        /// Close TCP connection. Returns 0 on success.
        pub fn tcp_close() -> i32;
        /// Get TCP connection status.
        /// Returns: 0=closed, 1=connecting, 2=connected, 3=failed
        pub fn tcp_status() -> i32;
        
        // ═══════════════════════════════════════════════════════════════════
        // Console Input Syscalls
        // ═══════════════════════════════════════════════════════════════════
        
        /// Check if console input is available. Returns 1 if available, 0 otherwise.
        pub fn console_available() -> i32;
        /// Read from console (non-blocking). Returns bytes read, 0 if no data.
        pub fn console_read(buf_ptr: *mut u8, buf_len: i32) -> i32;
    }

    // --- Helper Wrappers ---

    /// Print a string to the console
    pub fn console_log(s: &str) {
        unsafe { print(s.as_ptr(), s.len()) };
    }

    /// Get current time in milliseconds
    pub fn get_time() -> i64 {
        unsafe { time() }
    }

    /// Get number of arguments
    pub fn argc() -> usize {
        unsafe { arg_count() as usize }
    }

    /// Get argument at index (returns None if out of bounds or buffer too small)
    pub fn argv(index: usize, buf: &mut [u8]) -> Option<usize> {
        let len = unsafe { arg_get(index as i32, buf.as_mut_ptr(), buf.len() as i32) };
        if len >= 0 {
            Some(len as usize)
        } else {
            None
        }
    }

    /// Get current working directory
    pub fn get_cwd(buf: &mut [u8]) -> Option<usize> {
        let len = unsafe { cwd_get(buf.as_mut_ptr(), buf.len() as i32) };
        if len >= 0 {
            Some(len as usize)
        } else {
            None
        }
    }

    /// Check if file exists
    pub fn file_exists(path: &str) -> bool {
        unsafe { fs_exists(path.as_ptr(), path.len() as i32) == 1 }
    }

    /// Read file contents into buffer, returns bytes read
    pub fn read_file(path: &str, buf: &mut [u8]) -> Option<usize> {
        let len =
            unsafe { fs_read(path.as_ptr(), path.len() as i32, buf.as_mut_ptr(), buf.len() as i32) };
        if len >= 0 {
            Some(len as usize)
        } else {
            None
        }
    }

    /// Write data to file
    pub fn write_file(path: &str, data: &[u8]) -> bool {
        let written = unsafe {
            fs_write(
                path.as_ptr(),
                path.len() as i32,
                data.as_ptr(),
                data.len() as i32,
            )
        };
        written >= 0
    }

    /// List files (returns raw data into buffer)
    pub fn list_files(buf: &mut [u8]) -> Option<usize> {
        let len = unsafe { fs_list(buf.as_mut_ptr(), buf.len() as i32) };
        if len >= 0 {
            Some(len as usize)
        } else {
            None
        }
    }

    /// Get kernel log entries
    pub fn get_klog(count: usize, buf: &mut [u8]) -> Option<usize> {
        let len = unsafe { klog_get(count as i32, buf.as_mut_ptr(), buf.len() as i32) };
        if len >= 0 {
            Some(len as usize)
        } else {
            None
        }
    }

    /// Check if network is available
    pub fn is_net_available() -> bool {
        unsafe { net_available() == 1 }
    }

    /// HTTP GET request
    pub fn http_fetch(url: &str, buf: &mut [u8]) -> Option<usize> {
        let len = unsafe {
            http_get(
                url.as_ptr(),
                url.len() as i32,
                buf.as_mut_ptr(),
                buf.len() as i32,
            )
        };
        if len >= 0 {
            Some(len as usize)
        } else {
            None
        }
    }

    /// Set current working directory
    pub fn set_cwd(path: &str) -> bool {
        unsafe { cwd_set(path.as_ptr(), path.len() as i32) == 0 }
    }

    /// List files in a specific directory
    pub fn list_dir(path: &str, buf: &mut [u8]) -> Option<usize> {
        let len = unsafe {
            fs_list_dir(
                path.as_ptr(),
                path.len() as i32,
                buf.as_mut_ptr(),
                buf.len() as i32,
            )
        };
        if len >= 0 {
            Some(len as usize)
        } else {
            None
        }
    }

    /// File stat result
    pub struct FileStat {
        pub size: u32,
        pub exists: bool,
        pub is_dir: bool,
    }

    /// Get file stats (size, exists, is_dir)
    pub fn file_stat(path: &str) -> Option<FileStat> {
        let mut out = [0u8; 6];
        let result = unsafe { fs_stat(path.as_ptr(), path.len() as i32, out.as_mut_ptr()) };
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

    /// Create a directory
    pub fn mkdir(path: &str) -> bool {
        unsafe { fs_mkdir(path.as_ptr(), path.len() as i32) == 0 }
    }

    /// Resolve hostname to IPv4 address
    pub fn resolve_dns(hostname: &str, ip_buf: &mut [u8; 4]) -> bool {
        let result = unsafe {
            dns_resolve(
                hostname.as_ptr(),
                hostname.len() as i32,
                ip_buf.as_mut_ptr(),
                4,
            )
        };
        result == 4
    }

    /// Get environment variable
    pub fn getenv(key: &str, buf: &mut [u8]) -> Option<usize> {
        let len = unsafe {
            env_get(
                key.as_ptr(),
                key.len() as i32,
                buf.as_mut_ptr(),
                buf.len() as i32,
            )
        };
        if len >= 0 {
            Some(len as usize)
        } else {
            None
        }
    }

    /// Get random bytes
    pub fn get_random(buf: &mut [u8]) -> bool {
        let result = unsafe { random(buf.as_mut_ptr(), buf.len() as i32) };
        result >= 0
    }

    /// Sleep for milliseconds
    pub fn sleep(ms: u32) {
        unsafe { sleep_ms(ms as i32) };
    }

    /// Disk stats result
    pub struct DiskStats {
        pub used_bytes: u64,
        pub total_bytes: u64,
    }

    /// Get disk usage statistics
    pub fn get_disk_stats() -> Option<DiskStats> {
        let mut out = [0u8; 16];
        let result = unsafe { disk_stats(out.as_mut_ptr()) };
        if result == 0 {
            Some(DiskStats {
                used_bytes: u64::from_le_bytes([
                    out[0], out[1], out[2], out[3], out[4], out[5], out[6], out[7],
                ]),
                total_bytes: u64::from_le_bytes([
                    out[8], out[9], out[10], out[11], out[12], out[13], out[14], out[15],
                ]),
            })
        } else {
            None
        }
    }

    /// Heap stats result
    pub struct HeapStats {
        pub used_bytes: u64,
        pub total_bytes: u64,
    }

    /// Get heap usage statistics
    pub fn get_heap_stats() -> Option<HeapStats> {
        let mut out = [0u8; 16];
        let result = unsafe { heap_stats(out.as_mut_ptr()) };
        if result == 0 {
            Some(HeapStats {
                used_bytes: u64::from_le_bytes([
                    out[0], out[1], out[2], out[3], out[4], out[5], out[6], out[7],
                ]),
                total_bytes: u64::from_le_bytes([
                    out[8], out[9], out[10], out[11], out[12], out[13], out[14], out[15],
                ]),
            })
        } else {
            None
        }
    }

    /// Power off the system
    pub fn poweroff() -> ! {
        unsafe { shutdown() }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // WASM Worker Helpers
    // ═══════════════════════════════════════════════════════════════════════

    /// Get number of WASM workers
    pub fn get_worker_count() -> usize {
        unsafe { wasm_worker_count() as usize }
    }

    /// Get total hart count (including primary)
    pub fn get_hart_count() -> usize {
        unsafe { hart_count() as usize }
    }

    /// CPU information
    pub struct CpuInfo {
        pub state: u32,           // 0=Offline, 1=Online, 2=Idle, 3=Running, 4=Halted
        pub running_pid: u32,     // 0 if idle
        pub utilization: u8,      // 0-100%
        pub context_switches: u64,
    }

    /// Get CPU info by ID
    pub fn get_cpu_info(cpu_id: usize) -> Option<CpuInfo> {
        let mut out = [0u8; 17];
        let result = unsafe { cpu_info(cpu_id as i32, out.as_mut_ptr()) };
        if result == 0 {
            Some(CpuInfo {
                state: u32::from_le_bytes([out[0], out[1], out[2], out[3]]),
                running_pid: u32::from_le_bytes([out[4], out[5], out[6], out[7]]),
                utilization: out[8],
                context_switches: u64::from_le_bytes([
                    out[9], out[10], out[11], out[12], out[13], out[14], out[15], out[16],
                ]),
            })
        } else {
            None
        }
    }

    /// WASM worker statistics
    pub struct WorkerStats {
        pub hart_id: u32,
        pub jobs_completed: u64,
        pub jobs_failed: u64,
        pub total_exec_ms: u64,
        pub current_job: u32,
        pub queue_depth: u32,
    }

    /// Get worker stats by index
    pub fn get_worker_stats(worker_idx: usize) -> Option<WorkerStats> {
        let mut out = [0u8; 36];
        let result = unsafe { wasm_worker_stats(worker_idx as i32, out.as_mut_ptr()) };
        if result == 0 {
            Some(WorkerStats {
                hart_id: u32::from_le_bytes([out[0], out[1], out[2], out[3]]),
                jobs_completed: u64::from_le_bytes([
                    out[4], out[5], out[6], out[7], out[8], out[9], out[10], out[11],
                ]),
                jobs_failed: u64::from_le_bytes([
                    out[12], out[13], out[14], out[15], out[16], out[17], out[18], out[19],
                ]),
                total_exec_ms: u64::from_le_bytes([
                    out[20], out[21], out[22], out[23], out[24], out[25], out[26], out[27],
                ]),
                current_job: u32::from_le_bytes([out[28], out[29], out[30], out[31]]),
                queue_depth: u32::from_le_bytes([out[32], out[33], out[34], out[35]]),
            })
        } else {
            None
        }
    }

    /// Job status
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum JobStatus {
        Pending = 0,
        Running = 1,
        Completed = 2,
        Failed = 3,
    }

    /// Submit a WASM job for execution on a worker hart
    /// 
    /// # Arguments
    /// * `wasm_bytes` - The WASM binary
    /// * `args` - Newline-separated arguments string
    /// * `target_hart` - None for auto-selection, Some(hart_id) for specific hart
    /// 
    /// # Returns
    /// Job ID on success, None on error
    pub fn submit_wasm_job(wasm_bytes: &[u8], args: &str, target_hart: Option<usize>) -> Option<u32> {
        let target = match target_hart {
            Some(h) => h as i32,
            None => 0,
        };
        let result = unsafe {
            wasm_submit_job(
                wasm_bytes.as_ptr(),
                wasm_bytes.len() as i32,
                args.as_ptr(),
                args.len() as i32,
                target,
            )
        };
        if result > 0 {
            Some(result as u32)
        } else {
            None
        }
    }

    /// Get job status
    pub fn get_job_status(job_id: u32) -> Option<JobStatus> {
        let result = unsafe { wasm_job_status(job_id as i32) };
        match result {
            0 => Some(JobStatus::Pending),
            1 => Some(JobStatus::Running),
            2 => Some(JobStatus::Completed),
            3 => Some(JobStatus::Failed),
            _ => None,
        }
    }

    /// Get list of all processes/tasks (raw data into buffer)
    /// Returns "pid:name:state:priority:cpu_time:uptime\n" per task
    pub fn get_ps_list(buf: &mut [u8]) -> Option<usize> {
        let len = unsafe { ps_list(buf.as_mut_ptr(), buf.len() as i32) };
        if len >= 0 {
            Some(len as usize)
        } else {
            None
        }
    }

    /// Kill result
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum KillResult {
        Success,
        NotFound,
        CannotKill, // e.g., init process
        InvalidPid,
    }

    /// Kill a process by PID
    pub fn kill_process(pid: u32) -> KillResult {
        let result = unsafe { kill(pid as i32) };
        match result {
            0 => KillResult::Success,
            -2 => KillResult::CannotKill,
            _ => {
                if pid == 0 {
                    KillResult::InvalidPid
                } else {
                    KillResult::NotFound
                }
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Additional Helper Wrappers - For migrated native commands
    // ═══════════════════════════════════════════════════════════════════════

    /// Get kernel version string
    pub fn get_version(buf: &mut [u8]) -> Option<usize> {
        let len = unsafe { version(buf.as_mut_ptr(), buf.len() as i32) };
        if len >= 0 {
            Some(len as usize)
        } else {
            None
        }
    }

    /// Check if filesystem is available
    pub fn is_fs_available() -> bool {
        unsafe { fs_available() == 1 }
    }

    /// Network info result
    pub struct NetInfo {
        pub ip: [u8; 4],
        pub gateway: [u8; 4],
        pub dns: [u8; 4],
        pub mac: [u8; 6],
        pub prefix_len: u8,
    }

    /// Get network info
    pub fn get_net_info() -> Option<NetInfo> {
        let mut out = [0u8; 19];
        let result = unsafe { net_info(out.as_mut_ptr()) };
        if result == 0 {
            Some(NetInfo {
                ip: [out[0], out[1], out[2], out[3]],
                gateway: [out[4], out[5], out[6], out[7]],
                dns: [out[8], out[9], out[10], out[11]],
                mac: [out[12], out[13], out[14], out[15], out[16], out[17]],
                prefix_len: out[18],
            })
        } else {
            None
        }
    }

    /// Remove a file
    pub fn remove_file(path: &str) -> bool {
        unsafe { fs_remove(path.as_ptr(), path.len() as i32) == 0 }
    }

    /// Check if path is a directory
    pub fn is_dir(path: &str) -> bool {
        unsafe { fs_is_dir(path.as_ptr(), path.len() as i32) == 1 }
    }

    /// Get available service definitions
    pub fn get_service_defs(buf: &mut [u8]) -> Option<usize> {
        let len = unsafe { service_list_defs(buf.as_mut_ptr(), buf.len() as i32) };
        if len >= 0 {
            Some(len as usize)
        } else {
            None
        }
    }

    /// Get running services
    pub fn get_running_services(buf: &mut [u8]) -> Option<usize> {
        let len = unsafe { service_list_running(buf.as_mut_ptr(), buf.len() as i32) };
        if len >= 0 {
            Some(len as usize)
        } else {
            None
        }
    }

    /// Start a service
    pub fn start_service(name: &str) -> bool {
        unsafe { service_start(name.as_ptr(), name.len() as i32) == 0 }
    }

    /// Stop a service
    pub fn stop_service(name: &str) -> bool {
        unsafe { service_stop(name.as_ptr(), name.len() as i32) == 0 }
    }

    /// Restart a service
    pub fn restart_service(name: &str) -> bool {
        unsafe { service_restart(name.as_ptr(), name.len() as i32) == 0 }
    }

    /// Get service status
    pub fn get_service_status(name: &str, buf: &mut [u8]) -> Option<usize> {
        let len = unsafe {
            service_status(
                name.as_ptr(),
                name.len() as i32,
                buf.as_mut_ptr(),
                buf.len() as i32,
            )
        };
        if len >= 0 {
            Some(len as usize)
        } else {
            None
        }
    }

    /// Ping result
    pub enum PingResult {
        Success { rtt_ms: u32 },
        Timeout,
        NetworkError,
    }

    /// Send a ping and wait for reply
    pub fn ping(ip: &[u8; 4], seq: u16, timeout_ms: u32) -> PingResult {
        let mut out = [0u8; 4];
        let result = unsafe {
            send_ping(
                ip.as_ptr(),
                seq as i32,
                timeout_ms as i32,
                out.as_mut_ptr(),
            )
        };
        match result {
            0 => PingResult::Success {
                rtt_ms: u32::from_le_bytes(out),
            },
            -1 => PingResult::Timeout,
            _ => PingResult::NetworkError,
        }
    }

    /// Format MAC address
    pub fn format_mac(mac: &[u8; 6], buf: &mut [u8]) -> usize {
        if buf.len() < 17 {
            return 0;
        }
        let hex = b"0123456789abcdef";
        for i in 0..6 {
            buf[i * 3] = hex[(mac[i] >> 4) as usize];
            buf[i * 3 + 1] = hex[(mac[i] & 0x0f) as usize];
            if i < 5 {
                buf[i * 3 + 2] = b':';
            }
        }
        17
    }

    /// Print an integer
    pub fn print_int(n: i64) {
        let mut buf = [0u8; 20];
        let s = int_to_str(n, &mut buf);
        console_log(s);
    }

    /// Convert integer to string (helper)
    pub fn int_to_str(mut n: i64, buf: &mut [u8]) -> &str {
        if n == 0 {
            buf[0] = b'0';
            return unsafe { core::str::from_utf8_unchecked(&buf[..1]) };
        }

        let negative = n < 0;
        if negative {
            n = -n;
        }

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

    /// Format an IPv4 address as a string
    pub fn format_ipv4(ip: &[u8; 4], buf: &mut [u8]) -> usize {
        let mut pos = 0;
        for (i, &octet) in ip.iter().enumerate() {
            if i > 0 && pos < buf.len() {
                buf[pos] = b'.';
                pos += 1;
            }
            // Convert octet to string
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
            // Copy digits in reverse order
            for j in (0..digit_count).rev() {
                if pos < buf.len() {
                    buf[pos] = digits[j];
                    pos += 1;
                }
            }
        }
        pos
    }

    // ═══════════════════════════════════════════════════════════════════════
    // TCP Socket Helpers
    // ═══════════════════════════════════════════════════════════════════════
    
    /// TCP connection status
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum TcpStatus {
        Closed = 0,
        Connecting = 1,
        Connected = 2,
        Failed = 3,
    }
    
    /// Connect to a TCP server by IP address
    pub fn tcp_connect_ip(ip: &[u8; 4], port: u16) -> bool {
        unsafe { tcp_connect(ip.as_ptr(), 4, port as i32) == 0 }
    }
    
    /// Send data over TCP connection
    pub fn tcp_send_data(data: &[u8]) -> Option<usize> {
        let result = unsafe { tcp_send(data.as_ptr(), data.len() as i32) };
        if result >= 0 {
            Some(result as usize)
        } else {
            None
        }
    }
    
    /// Receive data from TCP connection
    pub fn tcp_recv_data(buf: &mut [u8], timeout_ms: u32) -> Option<usize> {
        let result = unsafe { tcp_recv(buf.as_mut_ptr(), buf.len() as i32, timeout_ms as i32) };
        if result > 0 {
            Some(result as usize)
        } else if result == 0 {
            Some(0) // No data yet
        } else {
            None // Error
        }
    }
    
    /// Close TCP connection
    pub fn tcp_disconnect() -> bool {
        unsafe { tcp_close() == 0 }
    }
    
    /// Get TCP connection status
    pub fn tcp_get_status() -> TcpStatus {
        match unsafe { tcp_status() } {
            1 => TcpStatus::Connecting,
            2 => TcpStatus::Connected,
            3 => TcpStatus::Failed,
            _ => TcpStatus::Closed,
        }
    }
    
    /// Check if console input is available
    pub fn is_console_available() -> bool {
        unsafe { console_available() == 1 }
    }
    
    /// Read character from console (non-blocking)
    pub fn read_console(buf: &mut [u8]) -> usize {
        let result = unsafe { console_read(buf.as_mut_ptr(), buf.len() as i32) };
        if result > 0 { result as usize } else { 0 }
    }

    // --- Mandatory Panic Handler for no_std WASM ---
    #[panic_handler]
    fn panic(_info: &PanicInfo) -> ! {
        let msg = "WASM Panic!\n";
        unsafe { print(msg.as_ptr(), msg.len()) };
        loop {}
    }
}

// Re-export for easier access in scripts
#[cfg(target_arch = "wasm32")]
pub use syscalls::{
    // Raw syscalls
    print, time, arg_count, arg_get, cwd_get, cwd_set, fs_exists, fs_read, fs_write,
    fs_list, fs_list_dir, fs_stat, fs_mkdir, klog_get, net_available, http_get,
    dns_resolve, env_get, random, sleep_ms, disk_stats, heap_stats, shutdown,
    wasm_worker_count, wasm_worker_stats, wasm_submit_job, wasm_job_status, hart_count,
    cpu_info, ps_list, kill,
    // Additional raw syscalls
    version, fs_available, net_info, fs_remove, fs_is_dir,
    service_list_defs, service_list_running, service_start, service_stop,
    service_restart, service_status, send_ping,
    // TCP and console syscalls
    tcp_connect, tcp_send, tcp_recv, tcp_close, tcp_status,
    console_available, console_read,
    // Helper wrappers
    console_log, get_time, argc, argv, get_cwd, set_cwd, file_exists, read_file,
    write_file, list_files, list_dir, file_stat, mkdir, get_klog, is_net_available,
    http_fetch, resolve_dns, getenv, get_random, sleep, get_disk_stats, get_heap_stats,
    poweroff, print_int, int_to_str, format_ipv4,
    get_worker_count, get_hart_count, get_cpu_info, get_worker_stats, submit_wasm_job, get_job_status,
    get_ps_list, kill_process,
    // Additional helper wrappers
    get_version, is_fs_available, get_net_info, remove_file, is_dir,
    get_service_defs, get_running_services, start_service, stop_service,
    restart_service, get_service_status, ping, format_mac,
    // TCP and console helpers
    tcp_connect_ip, tcp_send_data, tcp_recv_data, tcp_disconnect, tcp_get_status,
    is_console_available, read_console,
    // Types
    FileStat, DiskStats, HeapStats, CpuInfo, WorkerStats, JobStatus, KillResult,
    NetInfo, PingResult, TcpStatus,
};
