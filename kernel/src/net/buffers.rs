//! Static buffer storage for network sockets.
//!
//! This module provides static buffers for ICMP, UDP, and TCP sockets
//! to avoid heap allocations in the kernel.

use smoltcp::iface::SocketStorage;
use smoltcp::socket::{icmp, udp};

use super::server::MAX_SERVER_SOCKETS;

/// Static storage for sockets (expanded for server sockets)
pub static mut SOCKET_STORAGE: [SocketStorage<'static>; 16] = [SocketStorage::EMPTY; 16];

/// Static storage for ICMP buffers - need larger buffers for proper ICMP
pub static mut ICMP_RX_META: [icmp::PacketMetadata; 8] = [icmp::PacketMetadata::EMPTY; 8];
pub static mut ICMP_TX_META: [icmp::PacketMetadata; 8] = [icmp::PacketMetadata::EMPTY; 8];
pub static mut ICMP_RX_DATA: [u8; 512] = [0; 512];
pub static mut ICMP_TX_DATA: [u8; 512] = [0; 512];

/// Static storage for UDP buffers (for DNS queries)
pub static mut UDP_RX_META: [udp::PacketMetadata; 8] = [udp::PacketMetadata::EMPTY; 8];
pub static mut UDP_TX_META: [udp::PacketMetadata; 8] = [udp::PacketMetadata::EMPTY; 8];
pub static mut UDP_RX_DATA: [u8; 1024] = [0; 1024];
pub static mut UDP_TX_DATA: [u8; 1024] = [0; 1024];

/// Static storage for TCP buffers (for HTTP client connections)
pub static mut TCP_RX_DATA: [u8; 8192] = [0; 8192];
pub static mut TCP_TX_DATA: [u8; 4096] = [0; 4096];

/// Static storage for server TCP buffers
pub static mut TCP_SERVER_RX_DATA: [[u8; 2048]; MAX_SERVER_SOCKETS] = [[0; 2048]; MAX_SERVER_SOCKETS];
pub static mut TCP_SERVER_TX_DATA: [[u8; 1024]; MAX_SERVER_SOCKETS] = [[0; 1024]; MAX_SERVER_SOCKETS];
