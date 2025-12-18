//! Network stack using smoltcp.
//!
//! This module provides the TCP/IP stack for the kernel using the smoltcp crate.
//! Uses D1 EMAC for both real D1 hardware and VM D1 emulation.
//! 
//! ## Module Structure
//! 
//! - `config` - Network configuration constants and IP address management
//! - `patching` - TCP patching state for smoltcp bug workarounds
//! - `buffers` - Static buffer storage for sockets
//! - `server` - TCP server socket infrastructure
//! - `utils` - Utility functions for IP parsing/formatting
//!
//! Note: NetState is now defined in `lock::state::net` and re-exported here for compatibility.

pub(crate) mod config;
mod patching;
mod buffers;
pub(crate) mod server;
mod utils;

// Re-export public items from config
pub use config::{
    GATEWAY,
    PREFIX_LEN,
    get_my_ip,
    is_ip_assigned,
    DNS_SERVER,
    DNS_PORT,
};

// Re-export public items from server
pub use server::{
    TcpSocketId,
};

// Re-export NetState from lock::state::net (the new canonical location)
pub use crate::lock::state::net::NetState;

// Type aliases for backwards compatibility
pub type D1NetState = NetState;

// Re-export utility functions
pub use utils::{parse_ipv4, format_ipv4};
