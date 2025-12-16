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
static TCPD_USING_D1: AtomicBool = AtomicBool::new(false);  // True if using D1 EMAC
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
    // Try VirtIO network first
    {
        let mut net = crate::NET_STATE.lock();
        if let Some(ref mut n) = *net {
            let socket = n.tcp_listen(TCPD_PORT)?;
            unsafe { TCPD_LISTEN_SOCKET = Some(socket); }
            TCPD_INITIALIZED.store(true, Ordering::Release);
            klog_info("tcpd", &format!("Listening on TCP port {} (VirtIO)", TCPD_PORT));
            return Ok(());
        }
    }
    
    // Fallback to D1 EMAC
    {
        let mut net = crate::D1_NET_STATE.lock();
        if let Some(ref mut n) = *net {
            let socket = n.tcp_listen(TCPD_PORT)?;
            unsafe { TCPD_LISTEN_SOCKET = Some(socket); }
            TCPD_USING_D1.store(true, Ordering::Release);
            TCPD_INITIALIZED.store(true, Ordering::Release);
            klog_info("tcpd", &format!("Listening on TCP port {} (D1 EMAC)", TCPD_PORT));
            return Ok(());
        }
    }
    
    Err("Network not available")
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
    
    // Dispatch to correct implementation
    if TCPD_USING_D1.load(Ordering::Acquire) {
        tick_d1(now);
    } else {
        tick_virtio(now);
    }
}

/// VirtIO network tick implementation
fn tick_virtio(now: i64) {
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
    
    // Check listening socket state
    if let Some(listen_id) = unsafe { TCPD_LISTEN_SOCKET } {
        let state = net.tcp_server_state(listen_id);
        
        if state == "Established" {
            for slot in unsafe { TCPD_CONNECTIONS.iter_mut() } {
                if slot.socket_id.is_none() {
                    slot.socket_id = Some(listen_id);
                    slot.sent_hello = false;
                    slot.close_pending = false;
                    unsafe { TCPD_LISTEN_SOCKET = None; }
                    klog_info("tcpd", &format!("Connection accepted (socket {})", listen_id));
                    break;
                }
            }
        } else if let Some((conn_id, remote_ip, remote_port)) = net.tcp_accept(listen_id) {
            for slot in unsafe { TCPD_CONNECTIONS.iter_mut() } {
                if slot.socket_id.is_none() {
                    slot.socket_id = Some(conn_id);
                    slot.sent_hello = false;
                    slot.close_pending = false;
                    unsafe { TCPD_LISTEN_SOCKET = None; }
                    let o = remote_ip.octets();
                    klog_info("tcpd", &format!("Connection from {}.{}.{}.{}:{}", o[0], o[1], o[2], o[3], remote_port));
                    break;
                }
            }
        }
    }
    
    // Service existing connections
    for slot in unsafe { TCPD_CONNECTIONS.iter_mut() } {
        if let Some(sock_id) = slot.socket_id {
            let state = net.tcp_server_state(sock_id);
            
            if slot.close_pending {
                if state == "Closed" || state == "TimeWait" {
                    net.tcp_release_server(sock_id);
                    slot.reset();
                    if unsafe { TCPD_LISTEN_SOCKET.is_none() } {
                        if let Ok(new_listen) = net.tcp_listen(TCPD_PORT) {
                            unsafe { TCPD_LISTEN_SOCKET = Some(new_listen); }
                        }
                    }
                }
            } else if state != "Established" {
                if state == "Closed" || state == "TimeWait" {
                    net.tcp_release_server(sock_id);
                    slot.reset();
                    if unsafe { TCPD_LISTEN_SOCKET.is_none() } {
                        if let Ok(new_listen) = net.tcp_listen(TCPD_PORT) {
                            unsafe { TCPD_LISTEN_SOCKET = Some(new_listen); }
                        }
                    }
                }
            } else if !slot.sent_hello {
                match net.tcp_send_on(sock_id, b"works\n", now) {
                    Ok(sent) if sent > 0 => {
                        slot.sent_hello = true;
                        klog_info("tcpd", &format!("Sent 'works' ({} bytes)", sent));
                    }
                    Ok(_) => {}
                    Err(e) => {
                        klog_info("tcpd", &format!("Send error: {}", e));
                        net.tcp_close_on(sock_id, now);
                        slot.close_pending = true;
                    }
                }
            } else {
                net.tcp_close_on(sock_id, now);
                slot.close_pending = true;
                klog_info("tcpd", "Closing connection");
            }
        }
    }
    
    // Final poll
    net.poll(now);
}

/// D1 EMAC network tick implementation  
fn tick_d1(now: i64) {
    let mut net = match crate::D1_NET_STATE.try_lock() {
        Some(guard) => guard,
        None => return,
    };
    
    let net = match net.as_mut() {
        Some(n) => n,
        None => return,
    };
    
    // Poll the network
    net.poll(now);
    
    // Check listening socket state
    if let Some(listen_id) = unsafe { TCPD_LISTEN_SOCKET } {
        let state = net.tcp_server_state(listen_id);
        
        if state == "Established" {
            for slot in unsafe { TCPD_CONNECTIONS.iter_mut() } {
                if slot.socket_id.is_none() {
                    slot.socket_id = Some(listen_id);
                    slot.sent_hello = false;
                    slot.close_pending = false;
                    unsafe { TCPD_LISTEN_SOCKET = None; }
                    klog_info("tcpd", &format!("Connection accepted (socket {})", listen_id));
                    break;
                }
            }
        } else if let Some((conn_id, remote_ip, remote_port)) = net.tcp_accept(listen_id) {
            for slot in unsafe { TCPD_CONNECTIONS.iter_mut() } {
                if slot.socket_id.is_none() {
                    slot.socket_id = Some(conn_id);
                    slot.sent_hello = false;
                    slot.close_pending = false;
                    unsafe { TCPD_LISTEN_SOCKET = None; }
                    let o = remote_ip.octets();
                    klog_info("tcpd", &format!("Connection from {}.{}.{}.{}:{}", o[0], o[1], o[2], o[3], remote_port));
                    break;
                }
            }
        }
    }
    
    // Service existing connections
    for slot in unsafe { TCPD_CONNECTIONS.iter_mut() } {
        if let Some(sock_id) = slot.socket_id {
            let state = net.tcp_server_state(sock_id);
            
            if slot.close_pending {
                if state == "Closed" || state == "TimeWait" {
                    net.tcp_release_server(sock_id);
                    slot.reset();
                    if unsafe { TCPD_LISTEN_SOCKET.is_none() } {
                        if let Ok(new_listen) = net.tcp_listen(TCPD_PORT) {
                            unsafe { TCPD_LISTEN_SOCKET = Some(new_listen); }
                        }
                    }
                }
            } else if state != "Established" {
                if state == "Closed" || state == "TimeWait" {
                    net.tcp_release_server(sock_id);
                    slot.reset();
                    if unsafe { TCPD_LISTEN_SOCKET.is_none() } {
                        if let Ok(new_listen) = net.tcp_listen(TCPD_PORT) {
                            unsafe { TCPD_LISTEN_SOCKET = Some(new_listen); }
                        }
                    }
                }
            } else if !slot.sent_hello {
                match net.tcp_send_on(sock_id, b"works\n", now) {
                    Ok(sent) if sent > 0 => {
                        slot.sent_hello = true;
                        klog_info("tcpd", &format!("Sent 'works' ({} bytes)", sent));
                    }
                    Ok(_) => {}
                    Err(e) => {
                        klog_info("tcpd", &format!("Send error: {}", e));
                        net.tcp_close_on(sock_id, now);
                        slot.close_pending = true;
                    }
                }
            } else {
                net.tcp_close_on(sock_id, now);
                slot.close_pending = true;
                klog_info("tcpd", "Closing connection");
            }
        }
    }
    
    // Final poll
    net.poll(now);
}

/// Common tick logic (unused for now, but kept for future refactoring)
#[allow(dead_code)]
fn tick_common<S, A, SE, C, R, L, P>(
    _now: i64,
    _tcp_server_state: S,
    _tcp_accept: A,
    _tcp_send_on: SE,
    _tcp_close_on: C,
    _tcp_release_server: R,
    _tcp_listen: L,
    _poll: P,
)
where
    S: Fn(TcpSocketId) -> &'static str,
    A: Fn(TcpSocketId) -> Option<(TcpSocketId, smoltcp::wire::Ipv4Address, u16)>,
    SE: Fn(TcpSocketId, &[u8]) -> Result<usize, &'static str>,
    C: Fn(TcpSocketId),
    R: Fn(TcpSocketId),
    L: Fn() -> Result<TcpSocketId, &'static str>,
    P: Fn(),
{
    // Generic implementation - not used
}

/// tcpd service entry point (for scheduler)
pub fn tcpd_service() {
    tick();
}

