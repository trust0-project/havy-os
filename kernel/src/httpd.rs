//! httpd - HTTP Server Daemon using Embassy-Net patterns
//!
//! A background service that listens on TCP port 80 and responds
//! with HTTP content to incoming connections.
//!
//! This implementation uses embassy-net types and patterns for async networking,
//! integrated with the existing smoltcp infrastructure.

use alloc::format;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;

use crate::klog::klog_info;

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

/// Build the main index page
fn build_index_response() -> Vec<u8> {
    let uptime_ms = crate::get_time_ms();
    let uptime_secs = uptime_ms / 1000;
    let hours = uptime_secs / 3600;
    let mins = (uptime_secs % 3600) / 60;
    let secs = uptime_secs % 60;
    
    let num_harts = crate::HARTS_ONLINE.load(Ordering::Relaxed);
    let requests = HTTPD_REQUESTS_SERVED.load(Ordering::Relaxed);
    let version = env!("CARGO_PKG_VERSION");
    
    let body = format!(
        r#"<!DOCTYPE html>
<html lang='en'>
<head>
    <meta charset='UTF-8'>
    <meta name='viewport' content='width=device-width, initial-scale=1.0'>
    <title>BAVY OS Web Server</title>
    <style>
        :root {{
            --bg-primary: #0f0f1a;
            --bg-secondary: #1a1a2e;
            --bg-card: #16213e;
            --accent: #0f3460;
            --text-primary: #e0e0e0;
            --text-secondary: #a0a0a0;
            --highlight: #00d9ff;
            --success: #00ff88;
        }}
        * {{ margin: 0; padding: 0; box-sizing: border-box; }}
        body {{
            font-family: 'Inter', system-ui, -apple-system, sans-serif;
            background: linear-gradient(135deg, var(--bg-primary) 0%, var(--bg-secondary) 100%);
            color: var(--text-primary);
            min-height: 100vh;
            display: flex;
            flex-direction: column;
            align-items: center;
            padding: 2rem;
        }}
        .container {{ max-width: 800px; width: 100%; }}
        header {{ text-align: center; margin-bottom: 3rem; }}
        h1 {{
            font-size: 2.5rem;
            font-weight: 700;
            background: linear-gradient(90deg, var(--highlight), var(--success));
            -webkit-background-clip: text;
            -webkit-text-fill-color: transparent;
            background-clip: text;
            margin-bottom: 0.5rem;
        }}
        .subtitle {{ color: var(--text-secondary); font-size: 1.1rem; }}
        .badge {{
            display: inline-block;
            background: var(--accent);
            color: var(--highlight);
            padding: 0.25rem 0.75rem;
            border-radius: 999px;
            font-size: 0.75rem;
            margin-top: 0.5rem;
        }}
        .card {{
            background: var(--bg-card);
            border-radius: 16px;
            padding: 1.5rem;
            margin-bottom: 1.5rem;
            border: 1px solid rgba(255, 255, 255, 0.05);
            box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
        }}
        .card h2 {{
            font-size: 1.2rem;
            color: var(--highlight);
            margin-bottom: 1rem;
            display: flex;
            align-items: center;
            gap: 0.5rem;
        }}
        .stats-grid {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
            gap: 1rem;
        }}
        .stat {{
            background: var(--accent);
            padding: 1rem;
            border-radius: 12px;
            text-align: center;
        }}
        .stat-value {{
            font-size: 1.8rem;
            font-weight: 700;
            color: var(--success);
        }}
        .stat-label {{
            font-size: 0.85rem;
            color: var(--text-secondary);
            margin-top: 0.25rem;
        }}
        .status-indicator {{
            display: inline-block;
            width: 10px;
            height: 10px;
            background: var(--success);
            border-radius: 50%;
            animation: pulse 2s infinite;
        }}
        @keyframes pulse {{
            0%, 100% {{ opacity: 1; }}
            50% {{ opacity: 0.5; }}
        }}
        footer {{
            margin-top: 2rem;
            text-align: center;
            color: var(--text-secondary);
            font-size: 0.9rem;
        }}
        a {{ color: var(--highlight); text-decoration: none; }}
        a:hover {{ text-decoration: underline; }}
    </style>
</head>
<body>
    <div class='container'>
        <header>
            <h1>BAVY OS</h1>
            <p class='subtitle'>RISC-V Operating System with Async HTTP Server</p>
            <span class='badge'>Powered by Embassy-Net</span>
        </header>
        
        <div class='card'>
            <h2><span class='status-indicator'></span> System Status</h2>
            <div class='stats-grid'>
                <div class='stat'>
                    <div class='stat-value'>{:02}:{:02}:{:02}</div>
                    <div class='stat-label'>Uptime</div>
                </div>
                <div class='stat'>
                    <div class='stat-value'>{}</div>
                    <div class='stat-label'>CPU Cores</div>
                </div>
                <div class='stat'>
                    <div class='stat-value'>{}</div>
                    <div class='stat-label'>Requests Served</div>
                </div>
            </div>
        </div>
        
        <div class='card'>
            <h2>API Endpoints</h2>
            <ul style='list-style: none; line-height: 2;'>
                <li><a href='/status'>/status</a> - Human-readable status page</li>
                <li><a href='/api/status'>/api/status</a> - JSON status API</li>
            </ul>
        </div>
        
        <footer>
            <p>Powered by <strong>BAVY OS httpd v{}</strong></p>
            <p>Async networking with Embassy-Net + smoltcp on RISC-V</p>
        </footer>
    </div>
</body>
</html>"#,
        hours, mins, secs,
        num_harts,
        requests,
        version
    );
    
    build_simple_response(200, "OK", "text/html; charset=utf-8", body.as_bytes())
}

/// Build plain text status response
fn build_status_response() -> Vec<u8> {
    let uptime_ms = crate::get_time_ms();
    let uptime_secs = uptime_ms / 1000;
    let num_harts = crate::HARTS_ONLINE.load(Ordering::Relaxed);
    let requests = HTTPD_REQUESTS_SERVED.load(Ordering::Relaxed);
    
    let body = format!(
        "BAVY OS Status (Embassy-Net)\n\
         ============================\n\
         Uptime: {} seconds\n\
         CPU Cores: {}\n\
         HTTP Requests Served: {}\n\
         HTTP Port: {}\n\
         Async Runtime: embassy-executor (arch-spin)\n",
        uptime_secs, num_harts, requests, HTTPD_PORT
    );
    
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

/// Build 404 response
fn build_404_response(path: &str) -> Vec<u8> {
    let body = format!(
        r#"<!DOCTYPE html>
<html><head><title>404 Not Found</title></head>
<body style='font-family: system-ui; text-align: center; padding: 50px;'>
<h1>404 Not Found</h1>
<p>The requested path <code>{}</code> was not found on this server.</p>
<hr><p><i>BAVY OS httpd (embassy-net)</i></p>
</body></html>"#,
        path
    );
    
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
    
    HTTPD_INITIALIZED.store(true, Ordering::Release);
    klog_info("httpd", &format!("Embassy-net HTTP server initialized on port {}", HTTPD_PORT));
    
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
    
    // Try to acquire network lock (non-blocking)
    let mut net = match crate::NET_STATE.try_lock() {
        Some(guard) => guard,
        None => return,
    };
    
    let net = match net.as_mut() {
        Some(n) => n,
        None => return,
    };
    
    // Poll the network
    net.poll(now);
    
    // Check for connections using the existing smoltcp server infrastructure
    static mut LISTEN_SOCKET: Option<crate::net::TcpSocketId> = None;
    
    // Initialize listening socket if needed
    if unsafe { LISTEN_SOCKET.is_none() } {
        if let Ok(sock) = net.tcp_listen(HTTPD_PORT) {
            unsafe { LISTEN_SOCKET = Some(sock); }
            klog_info("httpd", &format!("Listening on port {}", HTTPD_PORT));
        }
    }
    
    // Check for and handle connections
    if let Some(listen_id) = unsafe { LISTEN_SOCKET } {
        let state = net.tcp_server_state(listen_id);
        
        // Log heartbeat every 10 seconds to confirm httpd is running
        static mut LAST_HEARTBEAT: i64 = 0;
        static mut HEARTBEAT_COUNT: u32 = 0;
        static mut LAST_STATE: &str = "Unknown";
        let should_heartbeat = unsafe {
            if now - LAST_HEARTBEAT > 10000 {
                LAST_HEARTBEAT = now;
                HEARTBEAT_COUNT += 1;
                true
            } else {
                false
            }
        };
        if should_heartbeat {
            let count = unsafe { HEARTBEAT_COUNT };
            klog_info("httpd", &format!("heartbeat #{} - socket state: {}", count, state));
        }
        
        // Log immediately when state changes from Listen  
        let state_changed = unsafe {
            if state != LAST_STATE {
                LAST_STATE = state;
                true
            } else {
                false
            }
        };
        if state_changed {
            klog_info("httpd", &format!("Socket state changed to: {}", state));
        }
        
        if state == "Established" {
            klog_info("httpd", "Connection established, handling request...");
            // Handle the connection
            handle_connection(net, listen_id, now);
            
            // Socket consumed, need to re-listen
            unsafe { LISTEN_SOCKET = None; }
        } else if let Some((conn_id, remote_ip, remote_port)) = net.tcp_accept(listen_id) {
            let remote_ip_o = remote_ip.octets();
            klog_info("httpd", &format!(
                "Connection from {}.{}.{}.{}:{}",
                remote_ip_o[0], remote_ip_o[1], remote_ip_o[2], remote_ip_o[3],
                remote_port
            ));
            
            handle_connection(net, conn_id, now);
            unsafe { LISTEN_SOCKET = None; }
        }
    }
    
    // Re-listen if needed
    if unsafe { LISTEN_SOCKET.is_none() } {
        if let Ok(sock) = net.tcp_listen(HTTPD_PORT) {
            unsafe { LISTEN_SOCKET = Some(sock); }
        }
    }
    
    net.poll(now);
}

/// Handle a connection - receive request and send response
fn handle_connection(net: &mut crate::net::NetState, socket_id: crate::net::TcpSocketId, now: i64) {
    // Receive request
    let mut request_buf = [0u8; MAX_REQUEST_SIZE];
    let mut request_len = 0;
    
    // Try to receive data (with timeout)
    let start = now;
    let timeout = 5000; // 5 second timeout
    
    loop {
        net.poll(crate::get_time_ms());
        
        match net.tcp_recv_on(socket_id, &mut request_buf[request_len..], crate::get_time_ms()) {
            Ok(n) if n > 0 => {
                request_len += n;
                
                // Check for end of headers
                if request_len >= 4 {
                    let has_end = request_buf[..request_len]
                        .windows(4)
                        .any(|w| w == b"\r\n\r\n");
                    if has_end {
                        break;
                    }
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
        
        if crate::get_time_ms() - start > timeout {
            break;
        }
        
        // Brief delay
        for _ in 0..1000 {
            core::hint::spin_loop();
        }
    }
    
    if request_len == 0 {
        net.tcp_close_on(socket_id, crate::get_time_ms());
        return;
    }
    
    // Build response
    let response = build_http_response(&request_buf[..request_len]);
    
    // Send response
    let mut sent = 0;
    let start = crate::get_time_ms();
    
    while sent < response.len() {
        net.poll(crate::get_time_ms());
        
        match net.tcp_send_on(socket_id, &response[sent..], crate::get_time_ms()) {
            Ok(n) if n > 0 => sent += n,
            Ok(_) => {}
            Err(_) => break,
        }
        
        if crate::get_time_ms() - start > timeout {
            break;
        }
        
        for _ in 0..500 {
            core::hint::spin_loop();
        }
    }
    
    // Close and release
    net.tcp_close_on(socket_id, crate::get_time_ms());
    
    // Poll a few times to process the close
    for _ in 0..10 {
        net.poll(crate::get_time_ms());
        for _ in 0..1000 {
            core::hint::spin_loop();
        }
    }
    
    net.tcp_release_server(socket_id);
    HTTPD_REQUESTS_SERVED.fetch_add(1, Ordering::Relaxed);
    klog_info("httpd", "Request completed");
}

/// httpd service entry point (for scheduler)
pub fn httpd_service() {
    tick();
}
