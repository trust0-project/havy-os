//! Network Proxy - Hart-aware network access
//!
//! This module provides transparent network access that works on any hart.
//! On Hart 0: Direct MMIO access via NET_STATE
//! On secondary harts: Delegates to Hart 0 via io_router
//!
//! # Example
//! ```
//! use crate::cpu::net_proxy;
//!
//! // Works on any hart!
//! net_proxy::poll(timestamp_ms);
//! if net_proxy::is_ip_assigned() {
//!     let ip = net_proxy::get_ip();
//! }
//! ```

use alloc::vec::Vec;
use smoltcp::wire::Ipv4Address;

use crate::cpu::io_router::{DeviceType, IoOp, IoRequest, IoResult, request_io};
use crate::lock::utils::NET_STATE;

// Timeout for I/O requests (5 seconds)
const IO_TIMEOUT_MS: u64 = 5000;

// ═══════════════════════════════════════════════════════════════════════════════
// Helper: Submit I/O request to Hart 0
// ═══════════════════════════════════════════════════════════════════════════════

/// Submit an I/O request and wait for the result (blocking).
fn request_io_blocking(operation: IoOp) -> IoResult {
    let request = IoRequest::new(DeviceType::Network, operation);
    request_io(request, IO_TIMEOUT_MS)
}

// ═══════════════════════════════════════════════════════════════════════════════
// Public API: Network Operations
// ═══════════════════════════════════════════════════════════════════════════════

/// Poll the network stack.
///
/// On Hart 0: Direct access via NET_STATE
/// On secondary harts: Delegates to Hart 0 via io_router
#[inline]
pub fn poll(timestamp_ms: i64) {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        let mut net = NET_STATE.lock();
        if let Some(state) = net.as_mut() {
            state.poll(timestamp_ms);
        }
    } else {
        let _ = request_io_blocking(IoOp::NetPoll { timestamp_ms });
    }
}

/// Check if IP has been assigned.
///
/// On Hart 0: Direct access via net module
/// On secondary harts: Delegates to Hart 0 via io_router
#[inline]
pub fn is_ip_assigned() -> bool {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        crate::net::is_ip_assigned()
    } else {
        match request_io_blocking(IoOp::NetIsIpAssigned) {
            IoResult::Ok(data) => data.first() == Some(&1),
            IoResult::Err(_) => false,
        }
    }
}

/// Get the assigned IP address.
///
/// On Hart 0: Direct access via net module
/// On secondary harts: Delegates to Hart 0 via io_router
#[inline]
pub fn get_ip() -> Ipv4Address {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        crate::net::get_my_ip()
    } else {
        match request_io_blocking(IoOp::NetGetIp) {
            IoResult::Ok(data) if data.len() == 4 => {
                Ipv4Address::new(data[0], data[1], data[2], data[3])
            }
            _ => Ipv4Address::new(0, 0, 0, 0),
        }
    }
}
