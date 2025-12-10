//! Network stack using smoltcp.
//!
//! This module provides the TCP/IP stack for the kernel using the smoltcp crate.
//! 
//! ## Module Structure
//! 
//! - `config` - Network configuration constants and IP address management
//! - `patching` - TCP patching state for smoltcp bug workarounds
//! - `buffers` - Static buffer storage for sockets
//! - `server` - TCP server socket infrastructure
//! - `state` - Main NetState struct and implementation
//! - `device` - VirtIO device wrapper and token implementations
//! - `utils` - Utility functions for IP parsing/formatting

mod config;
mod patching;
mod buffers;
mod server;
mod state;
mod device;
mod utils;

// Re-export public items from config
pub use config::{
    DEFAULT_IP_ADDR,
    GATEWAY,
    PREFIX_LEN,
    MY_IP_ADDR,
    get_my_ip,
    DNS_SERVER,
    DNS_PORT,
    LOOPBACK,
};

// Re-export public items from patching
pub use patching::{
    reset_client_patching_state,
    reset_server_patching_state,
    reset_server_patching_for_port,
};

// Re-export public items from server
pub use server::{
    MAX_SERVER_SOCKETS,
    TcpSocketId,
    ServerSocketState,
};

// Re-export NetState from state
pub use state::NetState;

// Re-export utility functions
pub use utils::{parse_ipv4, format_ipv4};
