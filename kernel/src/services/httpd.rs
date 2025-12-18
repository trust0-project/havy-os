//! httpd - HTTP Server Daemon using Embassy-Net patterns
//!
//! A background service that listens on TCP port 80 and responds
//! with HTTP content to incoming connections.
//!
//! This implementation uses embassy-net types and patterns for async networking,
//! integrated with the existing smoltcp infrastructure.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;

use crate::lock::utils::BLK_DEV;
use crate::services::klogd::klog_info;

// ═══════════════════════════════════════════════════════════════════════════════
// Filesystem Access Helpers (Thread-Safe)
// ═══════════════════════════════════════════════════════════════════════════════

/// Read a file from the filesystem with proper locking.
/// 
/// Lock Ordering (per lock.rs protocol):
/// 1. FS_STATE (Level 4) - Read lock allows concurrent readers
/// 2. BLK_DEV (Level 5) - Write lock serializes hardware access
/// 
/// This ordering prevents deadlocks with other services (klogd, sysmond).
fn read_from_fs(path: &str) -> Option<Vec<u8>> {
    crate::uart::write_str("[httpd] read_from_fs: ");
    crate::uart::write_line(path);
    
    // 1. Acquire Read Lock on FS_STATE (Level 4)
    let fs_guard = crate::FS_STATE.read();
    crate::uart::write_line("[httpd] FS_STATE lock acquired");
    
    if let Some(ref fs) = *fs_guard {
        crate::uart::write_line("[httpd] FS is Some, acquiring BLK_DEV...");
        
        // 2. Acquire Write Lock on BLK_DEV (Level 5)
        // VirtioBlock requires &mut self for all operations
        let mut blk_guard = BLK_DEV.write();
        crate::uart::write_line("[httpd] BLK_DEV lock acquired");
        
        if let Some(ref mut dev) = *blk_guard {
            crate::uart::write_line("[httpd] DEV is Some, calling read_file...");
            
            // 3. Perform Read Operation
            let result = fs.read_file(dev, path);
            
            match &result {
                Some(data) => {
                    crate::uart::write_str("[httpd] read_file SUCCESS: ");
                    crate::uart::write_u64(data.len() as u64);
                    crate::uart::write_line(" bytes");
                }
                None => {
                    crate::uart::write_line("[httpd] read_file returned None");
                }
            }
            
            return result;
        } else {
            crate::uart::write_line("[httpd] ERROR: DEV is None");
        }
    } else {
        crate::uart::write_line("[httpd] ERROR: FS is None");
    }
    None
}

/// HTTP daemon listen port (default HTTP port)
pub const HTTPD_PORT: u16 = 80;

/// Maximum request buffer size (4KB)
const MAX_REQUEST_SIZE: usize = 4096;

/// Daemon state
static HTTPD_INITIALIZED: AtomicBool = AtomicBool::new(false);
static HTTPD_LAST_RUN: AtomicI64 = AtomicI64::new(0);
static HTTPD_REQUESTS_SERVED: AtomicUsize = AtomicUsize::new(0);

/// Signal to notify the executor to poll (for future fully async implementation)
#[allow(dead_code)]
static POLL_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

// ═══════════════════════════════════════════════════════════════════════════════
// HTTP Response Builders
// ═══════════════════════════════════════════════════════════════════════════════

/// Build HTTP response based on the request
fn build_http_response(request: &[u8]) -> Vec<u8> {
    // Parse the request line
    let request_str = core::str::from_utf8(request).unwrap_or("");
    let first_line = request_str.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    
    let method = parts.get(0).copied().unwrap_or("GET");
    let path = parts.get(1).copied().unwrap_or("/");
    
    klog_info("httpd", &format!("{} {}", method, path));
    
    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => build_index_response(),
        ("GET", "/status") => build_status_response(),
        ("GET", "/api/status") => build_json_response(),
        ("GET", "/favicon.ico") => build_simple_response(204, "No Content", "image/x-icon", b""),
        ("HEAD", _) => build_simple_response(200, "OK", "text/html", b""),
        _ => build_404_response(path),
    }
}

/// Build simple HTTP response
fn build_simple_response(status: u16, status_text: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
    let headers = format!(
        "HTTP/1.1 {} {}\r\n\
         Content-Type: {}\r\n\
         Content-Length: {}\r\n\
         Server: BAVY-OS/0.1 httpd (embassy-net)\r\n\
         Connection: close\r\n\
         \r\n",
        status, status_text, content_type, body.len()
    );
    
    let mut response = headers.into_bytes();
    response.extend_from_slice(body);
    response
}

/// Build the main index page from filesystem template
fn build_index_response() -> Vec<u8> {
    let uptime_ms = crate::get_time_ms();
    let uptime_secs = uptime_ms / 1000;
    let hours = uptime_secs / 3600;
    let mins = (uptime_secs % 3600) / 60;
    let secs = uptime_secs % 60;
    
    let num_harts = crate::HARTS_ONLINE.load(Ordering::Relaxed);
    let requests = HTTPD_REQUESTS_SERVED.load(Ordering::Relaxed);
    let version = env!("CARGO_PKG_VERSION");
    
    // Try to read template from filesystem, fallback to minimal error page
    let template = read_from_fs("/etc/httpd/html/index.html")
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_else(|| String::from("<html><body><h1>Error: /etc/httpd/html/index.html not found</h1></body></html>"));
    
    // Perform template substitutions
    let body = template
        .replace("{{UPTIME}}", &format!("{:02}:{:02}:{:02}", hours, mins, secs))
        .replace("{{CPU_CORES}}", &num_harts.to_string())
        .replace("{{REQUESTS}}", &requests.to_string())
        .replace("{{VERSION}}", version);
    
    build_simple_response(200, "OK", "text/html; charset=utf-8", body.as_bytes())
}

/// Build plain text status response from filesystem template
fn build_status_response() -> Vec<u8> {
    let uptime_ms = crate::get_time_ms();
    let uptime_secs = uptime_ms / 1000;
    let num_harts = crate::HARTS_ONLINE.load(Ordering::Relaxed);
    let requests = HTTPD_REQUESTS_SERVED.load(Ordering::Relaxed);
    
    // Try to read template from filesystem, fallback to hardcoded
    let template = read_from_fs("/etc/httpd/html/status.html")
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_else(|| String::from(
            "BAVY OS Status\n============================\nUptime: {{UPTIME_SEC}} seconds\nCPU Cores: {{CPU_CORES}}\n"
        ));
    
    // Perform template substitutions
    let body = template
        .replace("{{UPTIME_SEC}}", &uptime_secs.to_string())
        .replace("{{CPU_CORES}}", &num_harts.to_string())
        .replace("{{REQUESTS}}", &requests.to_string());
    
    build_simple_response(200, "OK", "text/plain; charset=utf-8", body.as_bytes())
}

/// Build JSON status response
fn build_json_response() -> Vec<u8> {
    let uptime_ms = crate::get_time_ms();
    let num_harts = crate::HARTS_ONLINE.load(Ordering::Relaxed);
    let requests = HTTPD_REQUESTS_SERVED.load(Ordering::Relaxed);
    let version = env!("CARGO_PKG_VERSION");
    
    let body = format!(
        r#"{{"status":"ok","uptime_ms":{},"cpu_cores":{},"requests_served":{},"http_port":{},"version":"{}","runtime":"embassy-net"}}"#,
        uptime_ms, num_harts, requests, HTTPD_PORT, version
    );
    
    build_simple_response(200, "OK", "application/json", body.as_bytes())
}

/// Build 404 response from filesystem template
fn build_404_response(path: &str) -> Vec<u8> {
    // Try to read template from filesystem, fallback to minimal error page
    let template = read_from_fs("/etc/httpd/html/404.html")
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_else(|| String::from(
            "<html><body><h1>404 Not Found</h1><p>Path: {{PATH}}</p></body></html>"
        ));
    
    // Perform template substitution
    let body = template.replace("{{PATH}}", path);
    
    build_simple_response(404, "Not Found", "text/html; charset=utf-8", body.as_bytes())
}

// ═══════════════════════════════════════════════════════════════════════════════
// Public API - Integration with kernel's init system
// ═══════════════════════════════════════════════════════════════════════════════

/// Initialize the httpd daemon
///
/// Sets up the embassy-net stack and prepares to accept connections.
/// The actual async server runs via tick() being called periodically.
pub fn init() -> Result<(), &'static str> {
    // Check if network is available
    let net_available = crate::NET_STATE.try_lock()
        .map(|g| g.is_some())
        .unwrap_or(false);
    
    if !net_available {
        return Err("Network not available");
    }
    
    // Verify httpd templates exist in filesystem
    {
        let mut fs_guard = crate::FS_STATE.write();
        let mut blk_guard = BLK_DEV.write();
        
        if let (Some(ref mut fs), Some(ref mut dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
            let files = fs.list_dir(dev, "/");
            let httpd_files: usize = files.iter().filter(|f| f.name.contains("httpd")).count();
            if httpd_files > 0 {
                klog_info("httpd", &format!("Found {} template files in /etc/httpd/html/", httpd_files));
            } else {
                klog_info("httpd", "WARNING: No httpd template files found");
            }
        }
    }
    
    HTTPD_INITIALIZED.store(true, Ordering::Release);
    klog_info("httpd", &format!("HTTP server initialized on port {}", HTTPD_PORT));
    
    Ok(())
}

/// Check if httpd is initialized and running
pub fn is_running() -> bool {
    HTTPD_INITIALIZED.load(Ordering::Acquire)
}

/// Get the number of requests served
#[allow(dead_code)]
pub fn requests_served() -> usize {
    HTTPD_REQUESTS_SERVED.load(Ordering::Relaxed)
}

/// httpd tick - run the async executor for one iteration
///
/// This integrates with the kernel's cooperative scheduling.
/// Uses embassy-net patterns with the existing smoltcp TCP server.
pub fn tick() {
    if !HTTPD_INITIALIZED.load(Ordering::Acquire) {
        return;
    }
    
    let now = crate::get_time_ms();
    let last = HTTPD_LAST_RUN.load(Ordering::Relaxed);
    
    // Poll every 10ms
    if now - last < 10 {
        return;
    }
    HTTPD_LAST_RUN.store(now, Ordering::Relaxed);
    
    // Use unified NET_STATE implementation
    tick_impl(now);
}

/// Static listen socket (shared between VirtIO and D1 implementations)
static mut LISTEN_SOCKET: Option<crate::net::TcpSocketId> = None;

/// Network tick implementation
fn tick_impl(now: i64) {
    let mut net = match crate::NET_STATE.try_lock() {
        Some(guard) => guard,
        None => return,
    };
    
    let net = match net.as_mut() {
        Some(n) => n,
        None => return,
    };
    
    net.poll(now);
    
    if unsafe { LISTEN_SOCKET.is_none() } {
        if let Ok(sock) = net.tcp_listen(HTTPD_PORT) {
            unsafe { LISTEN_SOCKET = Some(sock); }
            klog_info("httpd", &format!("Listening on port {}", HTTPD_PORT));
        }
    }
    
    if let Some(listen_id) = unsafe { LISTEN_SOCKET } {
        let state = net.tcp_server_state(listen_id);
        
        if state == "Established" {
            klog_info("httpd", "Connection established, handling request...");
            handle_connection(net, listen_id, now);
            unsafe { LISTEN_SOCKET = None; }
        } else if let Some((conn_id, remote_ip, remote_port)) = net.tcp_accept(listen_id) {
            let o = remote_ip.octets();
            klog_info("httpd", &format!("Connection from {}.{}.{}.{}:{}", o[0], o[1], o[2], o[3], remote_port));
            handle_connection(net, conn_id, now);
            unsafe { LISTEN_SOCKET = None; }
        }
    }
    
    if unsafe { LISTEN_SOCKET.is_none() } {
        if let Ok(sock) = net.tcp_listen(HTTPD_PORT) {
            unsafe { LISTEN_SOCKET = Some(sock); }
        }
    }
    
    net.poll(now);
}

/// Handle a connection - receive request and send response
fn handle_connection(net: &mut crate::net::NetState, socket_id: crate::net::TcpSocketId, now: i64) {
    let mut request_buf = [0u8; MAX_REQUEST_SIZE];
    let mut request_len = 0;
    let timeout = 100; // 100ms max - cooperative, let scheduler retry
    let start = now;
    
    loop {
        net.poll(crate::get_time_ms());
        
        match net.tcp_recv_on(socket_id, &mut request_buf[request_len..], crate::get_time_ms()) {
            Ok(n) if n > 0 => {
                request_len += n;
                if request_len >= 4 {
                    let has_end = request_buf[..request_len].windows(4).any(|w| w == b"\r\n\r\n");
                    if has_end { break; }
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
        
        if crate::get_time_ms() - start > timeout { break; }
    }
    
    if request_len == 0 {
        net.tcp_close_on(socket_id, crate::get_time_ms());
        return;
    }
    
    let response = build_http_response(&request_buf[..request_len]);
    let mut sent = 0;
    let start = crate::get_time_ms();
    
    while sent < response.len() {
        net.poll(crate::get_time_ms());
        match net.tcp_send_on(socket_id, &response[sent..], crate::get_time_ms()) {
            Ok(n) if n > 0 => sent += n,
            Ok(_) => {}
            Err(_) => break,
        }
        if crate::get_time_ms() - start > timeout { break; }
    }
    
    net.tcp_close_on(socket_id, crate::get_time_ms());
    net.poll(crate::get_time_ms());
    
    net.tcp_release_server(socket_id);
    HTTPD_REQUESTS_SERVED.fetch_add(1, Ordering::Relaxed);
    klog_info("httpd", "Request completed");
}

/// httpd service entry point (for scheduler)
pub fn httpd_service() {
    tick();
}
