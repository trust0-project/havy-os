//! TCP server socket infrastructure.
//!
//! This module provides the server socket management for TCP listen/accept operations.

use smoltcp::iface::SocketHandle;

// =============================================================================
// TCP SERVER SOCKET INFRASTRUCTURE
// =============================================================================

/// Maximum number of server TCP sockets
pub const MAX_SERVER_SOCKETS: usize = 4;

/// TCP socket ID for multi-socket operations
pub type TcpSocketId = u8;

/// Server socket state
#[derive(Clone, Copy, PartialEq)]
pub enum ServerSocketState {
    /// Socket slot is free
    Free,
    /// Socket is listening for connections
    Listening,
    /// Socket has an active connection
    Connected,
    /// Socket is closing
    Closing,
}

/// A server socket entry with per-socket TCP patching state
pub struct ServerSocket {
    pub handle: Option<SocketHandle>,
    pub port: u16,
    pub state: ServerSocketState,
    /// Per-socket TCP patching state (to support multiple concurrent server connections)
    /// Store the last received SYN's sequence number for patching
    pub last_syn_seq: Option<u32>,
    /// Store our SYN-ACK seq number for patching incoming ACKs  
    pub synack_seq: Option<u32>,
    /// Track the expected ACK we should be sending
    pub expected_ack: Option<u32>,
    /// Track our own initial sequence number
    pub our_seq: Option<u32>,
    /// Track what ACK we expect to receive from the peer
    pub peer_ack_expected: Option<u32>,
}

impl ServerSocket {
    pub const fn new() -> Self {
        Self {
            handle: None,
            port: 0,
            state: ServerSocketState::Free,
            last_syn_seq: None,
            synack_seq: None,
            expected_ack: None,
            our_seq: None,
            peer_ack_expected: None,
        }
    }
    
    /// Reset patching state for this socket
    pub fn reset_patching(&mut self) {
        self.last_syn_seq = None;
        self.synack_seq = None;
        self.expected_ack = None;
        self.our_seq = None;
        self.peer_ack_expected = None;
    }
}

/// Manager for server TCP sockets
pub struct TcpServerManager {
    pub sockets: [ServerSocket; MAX_SERVER_SOCKETS],
}

impl TcpServerManager {
    pub const fn new() -> Self {
        Self {
            sockets: [
                ServerSocket::new(),
                ServerSocket::new(),
                ServerSocket::new(),
                ServerSocket::new(),
            ],
        }
    }
    
    /// Allocate a free socket slot, returns socket ID
    pub fn allocate(&mut self) -> Option<TcpSocketId> {
        for (i, slot) in self.sockets.iter_mut().enumerate() {
            if slot.state == ServerSocketState::Free {
                return Some(i as TcpSocketId);
            }
        }
        None
    }
    
    /// Get socket info by ID
    pub fn get(&self, id: TcpSocketId) -> Option<&ServerSocket> {
        self.sockets.get(id as usize)
    }
    
    /// Get mutable socket info by ID    
    pub fn get_mut(&mut self, id: TcpSocketId) -> Option<&mut ServerSocket> {
        self.sockets.get_mut(id as usize)
    }
    
    /// Find socket by port (for TCP patching)
    #[allow(dead_code)]
    pub fn find_by_port(&self, port: u16) -> Option<&ServerSocket> {
        self.sockets.iter().find(|s| s.port == port && s.state != ServerSocketState::Free)
    }
    
    /// Find mutable socket by port (for TCP patching)
    #[allow(dead_code)]
    pub fn find_by_port_mut(&mut self, port: u16) -> Option<&mut ServerSocket> {
        self.sockets.iter_mut().find(|s| s.port == port && s.state != ServerSocketState::Free)
    }
    
    /// Release a socket slot
    pub fn release(&mut self, id: TcpSocketId) {
        if let Some(slot) = self.sockets.get_mut(id as usize) {
            slot.handle = None;
            slot.port = 0;
            slot.state = ServerSocketState::Free;
            slot.reset_patching();  // Reset per-socket patching state
        }
    }
}
