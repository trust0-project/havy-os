//! tcpd - TCP Daemon Service
//!
//! A background service that listens on TCP port 30 and responds
//! with "works" to any incoming connection.
//!
//! This is a kernel service similar to klogd/sysmond, managed by init.

use alloc::format;
use core::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use crate::klog::klog_info;
use crate::net::TcpSocketId;

/// TCP daemon listen port
pub const TCPD_PORT: u16 = 30;

/// Daemon state
static TCPD_INITIALIZED: AtomicBool = AtomicBool::new(false);
static TCPD_LAST_RUN: AtomicI64 = AtomicI64::new(0);

/// Active connection state
struct TcpdConnection {
    socket_id: Option<TcpSocketId>,
    sent_hello: bool,
    close_pending: bool,
}

impl TcpdConnection {
    const fn new() -> Self {
        Self {
            socket_id: None,
            sent_hello: false,
            close_pending: false,
        }
    }
    
    fn reset(&mut self) {
        self.socket_id = None;
        self.sent_hello = false;
        self.close_pending = false;
    }
}

/// Maximum concurrent connections
const MAX_CONNECTIONS: usize = 4;

/// Active connections
static mut TCPD_CONNECTIONS: [TcpdConnection; MAX_CONNECTIONS] = [
    TcpdConnection::new(),
    TcpdConnection::new(),
    TcpdConnection::new(),
    TcpdConnection::new(),
];

/// Listening socket ID
static mut TCPD_LISTEN_SOCKET: Option<TcpSocketId> = None;

/// Initialize the tcpd daemon
///
/// Binds to port 30 and prepares to accept connections.
/// Must be called before `tick()`.
pub fn init() -> Result<(), &'static str> {
    let mut net = crate::NET_STATE.lock();
    if let Some(ref mut n) = *net {
        let socket = n.tcp_listen(TCPD_PORT)?;
        unsafe { TCPD_LISTEN_SOCKET = Some(socket); }
        TCPD_INITIALIZED.store(true, Ordering::Release);
        klog_info("tcpd", &format!("Listening on TCP port {}", TCPD_PORT));
        Ok(())
    } else {
        Err("Network not available")
    }
}

/// Check if tcpd is initialized and running
pub fn is_running() -> bool {
    TCPD_INITIALIZED.load(Ordering::Acquire)
}

/// tcpd tick - poll for connections and handle them
///
/// Called by the scheduler. Does one unit of work and returns.
pub fn tick() {
    if !TCPD_INITIALIZED.load(Ordering::Acquire) {
        return;
    }
    
    let now = crate::get_time_ms();
    let last = TCPD_LAST_RUN.load(Ordering::Relaxed);
    
    // Poll every 10ms (more responsive than 50ms)
    if now - last < 10 {
        return;
    }
    TCPD_LAST_RUN.store(now, Ordering::Relaxed);
    
    // Try to acquire network lock (non-blocking)
    let mut net = match crate::NET_STATE.try_lock() {
        Some(guard) => guard,
        None => {
            // Log if we're frequently failing to get the lock
            static mut LOCK_FAILS: u32 = 0;
            static mut LAST_FAIL_LOG: i64 = 0;
            unsafe {
                LOCK_FAILS += 1;
                if now - LAST_FAIL_LOG > 5000 {  // Log every 5 seconds
                    if LOCK_FAILS > 10 {
                        klog_info("tcpd", &format!("Lock contention: {} fails in 5s", LOCK_FAILS));
                    }
                    LOCK_FAILS = 0;
                    LAST_FAIL_LOG = now;
                }
            }
            return;
        }
    };
    
    let net = match net.as_mut() {
        Some(n) => n,
        None => return,
    };
    
    // Poll the network to process any pending packets
    net.poll(now);
    
    // Check listening socket state
    if let Some(listen_id) = unsafe { TCPD_LISTEN_SOCKET } {
        let state = net.tcp_server_state(listen_id);
        
        // Log heartbeat every 10 seconds to confirm tcpd is running
        static mut LAST_HEARTBEAT: i64 = 0;
        static mut HEARTBEAT_COUNT: u32 = 0;
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
            klog_info("tcpd", &format!("heartbeat #{} - socket state: {}", count, state));
        }
        
        // Log immediately when state changes from Listen
        if state != "Listen" {
            klog_info("tcpd", &format!("Socket state changed to: {}", state));
        }
        if state == "Established" {
            // Socket transitioned to connected, find remote endpoint
            // This is our connection
            for slot in unsafe { TCPD_CONNECTIONS.iter_mut() } {
                if slot.socket_id.is_none() {
                    slot.socket_id = Some(listen_id);
                    slot.sent_hello = false;
                    slot.close_pending = false;
                    
                    // Clear the listen socket since it's now a data socket
                    unsafe { TCPD_LISTEN_SOCKET = None; }
                    
                    klog_info("tcpd", &format!("Connection accepted (socket {})", listen_id));
                    break;
                }
            }
        } else if let Some((conn_id, remote_ip, remote_port)) = net.tcp_accept(listen_id) {
            // Store in connections array
            for slot in unsafe { TCPD_CONNECTIONS.iter_mut() } {
                if slot.socket_id.is_none() {
                    slot.socket_id = Some(conn_id);
                    slot.sent_hello = false;
                    slot.close_pending = false;
                    
                    // Clear the listen socket since it's now a data socket
                    unsafe { TCPD_LISTEN_SOCKET = None; }
                    
                    let remote_ip_o = remote_ip.octets();
                    klog_info("tcpd", &format!(
                        "Connection from {}.{}.{}.{}:{}",
                        remote_ip_o[0], remote_ip_o[1], remote_ip_o[2], remote_ip_o[3],
                        remote_port
                    ));
                    break;
                }
            }
        }
    }
    
    // Service existing connections
    for slot in unsafe { TCPD_CONNECTIONS.iter_mut() } {
        if let Some(sock_id) = slot.socket_id {
            // Check socket state
            let state = net.tcp_server_state(sock_id);
            
            if slot.close_pending {
                // Check if socket is closed, then release
                if state == "Closed" || state == "TimeWait" {
                    net.tcp_release_server(sock_id);
                    slot.reset();
                    
                    // Re-listen on the port since we released a socket
                    if unsafe { TCPD_LISTEN_SOCKET.is_none() } {
                        if let Ok(new_listen) = net.tcp_listen(TCPD_PORT) {
                            unsafe { TCPD_LISTEN_SOCKET = Some(new_listen); }
                            klog_info("tcpd", "Re-listening on port 30");
                        }
                    }
                }
            } else if state != "Established" {
                // Socket not connected yet or already closing
                if state == "Closed" || state == "TimeWait" {
                    net.tcp_release_server(sock_id);
                    slot.reset();
                    
                    // Re-listen
                    if unsafe { TCPD_LISTEN_SOCKET.is_none() } {
                        if let Ok(new_listen) = net.tcp_listen(TCPD_PORT) {
                            unsafe { TCPD_LISTEN_SOCKET = Some(new_listen); }
                            klog_info("tcpd", "Re-listening on port 30");
                        }
                    }
                }
            } else if !slot.sent_hello {
                // Socket is established, send response
                match net.tcp_send_on(sock_id, b"works\n", now) {
                    Ok(sent) if sent > 0 => {
                        slot.sent_hello = true;
                        klog_info("tcpd", &format!("Sent 'works' ({} bytes)", sent));
                    }
                    Ok(_) => {
                        // Couldn't send yet, will retry
                    }
                    Err(e) => {
                        klog_info("tcpd", &format!("Send error: {}", e));
                        net.tcp_close_on(sock_id, now);
                        slot.close_pending = true;
                    }
                }
            } else {
                // Already sent, close the connection
                net.tcp_close_on(sock_id, now);
                slot.close_pending = true;
                klog_info("tcpd", "Closing connection");
            }
        }
    }
    
    // Final poll to transmit any queued responses (important for SYN-ACK!)
    net.poll(now);
}

/// tcpd service entry point (for scheduler)
pub fn tcpd_service() {
    tick();
}
