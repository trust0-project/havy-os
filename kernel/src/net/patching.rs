//! TCP patching state for workaround of smoltcp bugs.
//!
//! This module handles TCP sequence/acknowledgment number patching for both
//! server and client roles to work around issues with smoltcp's ACK handling.

// ═══════════════════════════════════════════════════════════════════════════════
// PER-PORT SERVER TCP PATCHING STATE
// Supports multiple concurrent server connections on different ports
// ═══════════════════════════════════════════════════════════════════════════════

/// Maximum number of server ports we can track patching for
const MAX_SERVER_PATCHING_PORTS: usize = 4;

/// Per-port TCP patching state for server role
#[derive(Clone, Copy)]
pub struct ServerPatchingState {
    pub port: u16,
    pub active: bool,
    pub last_syn_seq: Option<u32>,
    pub synack_seq: Option<u32>,
    pub expected_ack: Option<u32>,
    pub our_seq: Option<u32>,
    pub peer_ack_expected: Option<u32>,
    /// Sequence offset: correct_seq - smoltcp_seq
    /// Used to translate incoming SEQ values to what smoltcp expects
    /// This is needed because smoltcp's internal state uses the unpatched ack value
    pub seq_offset: Option<u32>,
    /// The smoltcp-generated (buggy) ack value before we patched it
    pub smoltcp_ack: Option<u32>,
}

impl ServerPatchingState {
    pub const fn new() -> Self {
        Self {
            port: 0,
            active: false,
            last_syn_seq: None,
            synack_seq: None,
            expected_ack: None,
            our_seq: None,
            peer_ack_expected: None,
            seq_offset: None,
            smoltcp_ack: None,
        }
    }
    
    pub fn reset(&mut self) {
        self.port = 0;
        self.active = false;
        self.last_syn_seq = None;
        self.synack_seq = None;
        self.expected_ack = None;
        self.our_seq = None;
        self.peer_ack_expected = None;
        self.seq_offset = None;
        self.smoltcp_ack = None;
    }
}

/// Global array of per-port server patching states
pub static mut SERVER_PATCHING: [ServerPatchingState; MAX_SERVER_PATCHING_PORTS] = [
    ServerPatchingState::new(),
    ServerPatchingState::new(),
    ServerPatchingState::new(),
    ServerPatchingState::new(),
];

/// Get or create patching state for a server port
pub fn get_server_patching(port: u16) -> Option<&'static mut ServerPatchingState> {
    unsafe {
        // First, look for existing entry for this port
        for state in SERVER_PATCHING.iter_mut() {
            if state.active && state.port == port {
                return Some(state);
            }
        }
        // If not found, allocate a new slot
        for state in SERVER_PATCHING.iter_mut() {
            if !state.active {
                state.port = port;
                state.active = true;
                return Some(state);
            }
        }
        None
    }
}

/// Find existing patching state for a port (read-only check)
pub fn find_server_patching(port: u16) -> Option<&'static ServerPatchingState> {
    unsafe {
        SERVER_PATCHING.iter().find(|s| s.active && s.port == port)
    }
}

/// Find existing patching state for a port (mutable)
pub fn find_server_patching_mut(port: u16) -> Option<&'static mut ServerPatchingState> {
    unsafe {
        SERVER_PATCHING.iter_mut().find(|s| s.active && s.port == port)
    }
}

/// Reset patching state for a specific server port
pub fn reset_server_patching_for_port(port: u16) {
    unsafe {
        for state in SERVER_PATCHING.iter_mut() {
            if state.active && state.port == port {
                state.reset();
                break;
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// CLIENT ROLE PATCHING STATE
// For outgoing connections like telnet
// ═══════════════════════════════════════════════════════════════════════════════

/// Store the server's seq from SYN-ACK so we can patch outgoing ACKs (CLIENT ROLE)
pub static mut CLIENT_SERVER_SEQ: Option<u32> = None;

/// Store the remote port we're connecting to (CLIENT ROLE)
pub static mut CLIENT_REMOTE_PORT: Option<u16> = None;

/// Track the last correctly computed ACK we should be sending (CLIENT ROLE)
pub static mut CLIENT_EXPECTED_ACK: Option<u32> = None;

/// Track our own initial sequence number (CLIENT ROLE)
pub static mut CLIENT_OUR_SEQ: Option<u32> = None;

/// Track what ACK we expect to RECEIVE from the peer (CLIENT ROLE)
pub static mut CLIENT_PEER_ACK_EXPECTED: Option<u32> = None;

/// Reset CLIENT role TCP patching state (call when closing client connection)
pub fn reset_client_patching_state() {
    unsafe {
        CLIENT_SERVER_SEQ = None;
        CLIENT_REMOTE_PORT = None;
        CLIENT_EXPECTED_ACK = None;
        CLIENT_OUR_SEQ = None;
        CLIENT_PEER_ACK_EXPECTED = None;
    }
}

/// Reset SERVER role TCP patching state for ALL ports (legacy function, avoid using)
pub fn reset_server_patching_state() {
    unsafe {
        for state in SERVER_PATCHING.iter_mut() {
            state.reset();
        }
    }
}

/// Helper function to recalculate TCP checksum after patching
pub fn recalculate_tcp_checksum(buffer: &mut [u8], tcp_offset: usize) {
    // Zero out the checksum field
    buffer[tcp_offset + 16] = 0;
    buffer[tcp_offset + 17] = 0;
    
    // Calculate pseudo-header checksum
    let ip_offset = 14;
    let src_ip = u32::from_be_bytes([buffer[ip_offset + 12], buffer[ip_offset + 13], buffer[ip_offset + 14], buffer[ip_offset + 15]]);
    let dst_ip = u32::from_be_bytes([buffer[ip_offset + 16], buffer[ip_offset + 17], buffer[ip_offset + 18], buffer[ip_offset + 19]]);
    let tcp_len = buffer.len() - tcp_offset;
    
    let mut sum: u32 = 0;
    sum += (src_ip >> 16) as u32;
    sum += (src_ip & 0xFFFF) as u32;
    sum += (dst_ip >> 16) as u32;
    sum += (dst_ip & 0xFFFF) as u32;
    sum += 6u32; // TCP protocol number
    sum += tcp_len as u32;
    
    // Add TCP segment (16-bit words)
    let tcp_data = &buffer[tcp_offset..];
    let mut i = 0;
    while i + 1 < tcp_data.len() {
        sum += u16::from_be_bytes([tcp_data[i], tcp_data[i + 1]]) as u32;
        i += 2;
    }
    if i < tcp_data.len() {
        sum += (tcp_data[i] as u32) << 8;
    }
    
    // Fold 32-bit sum to 16 bits
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    let checksum = !(sum as u16);
    let checksum_bytes = checksum.to_be_bytes();
    buffer[tcp_offset + 16] = checksum_bytes[0];
    buffer[tcp_offset + 17] = checksum_bytes[1];
}
