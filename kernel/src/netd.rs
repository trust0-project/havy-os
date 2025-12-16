//! netd - Network Daemon Service
//!
//! Background service that polls for IP assignment from the relay.
//! Runs after boot to provision the network IP address dynamically.

use core::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use smoltcp::wire::{IpAddress, IpCidr, Ipv4Address};

use crate::klog::klog_info;
use crate::net::{set_my_ip, is_ip_assigned, get_my_ip, PREFIX_LEN};

/// Daemon state
static NETD_INITIALIZED: AtomicBool = AtomicBool::new(false);
static NETD_IP_ASSIGNED: AtomicBool = AtomicBool::new(false);
static NETD_LAST_RUN: AtomicI64 = AtomicI64::new(0);

/// Poll interval in milliseconds
const POLL_INTERVAL_MS: i64 = 500;

/// Initialize the netd daemon
pub fn init() -> Result<(), &'static str> {
    if NETD_INITIALIZED.load(Ordering::Acquire) {
        return Ok(());
    }
    
    NETD_INITIALIZED.store(true, Ordering::Release);
    klog_info("netd", "Network daemon initialized, waiting for IP assignment");
    
    Ok(())
}

/// Check if netd is initialized and running
pub fn is_running() -> bool {
    NETD_INITIALIZED.load(Ordering::Acquire)
}

/// Check if IP has been assigned
pub fn has_ip() -> bool {
    NETD_IP_ASSIGNED.load(Ordering::Acquire)
}

/// netd tick - poll for IP assignment from relay
///
/// Called by the scheduler. Polls the D1 EMAC for IP assignment.
pub fn tick() {
    if !NETD_INITIALIZED.load(Ordering::Acquire) {
        return;
    }
    
    // Already have IP assigned
    if NETD_IP_ASSIGNED.load(Ordering::Acquire) {
        return;
    }
    
    let now = crate::get_time_ms();
    let last_run = NETD_LAST_RUN.load(Ordering::Acquire);
    
    // Rate limit polling
    if now - last_run < POLL_INTERVAL_MS {
        return;
    }
    NETD_LAST_RUN.store(now, Ordering::Release);
    
    // Try to get IP from D1 EMAC first
    if try_d1_emac_ip() {
        return;
    }
    
    // Try VirtIO network
    if try_virtio_ip() {
        return;
    }
}

/// Try to get IP from D1 EMAC MMIO register
fn try_d1_emac_ip() -> bool {
    let mut net = match crate::D1_NET_STATE.try_lock() {
        Some(guard) => guard,
        None => return false,
    };
    
    let net = match net.as_mut() {
        Some(n) => n,
        None => return false,
    };
    
    // Poll the network to check for IP assignment
    net.poll(crate::get_time_ms());
    
    // Check if IP was assigned via poll()
    if is_ip_assigned() {
        let ip = get_my_ip();
        let octets = ip.octets();
        klog_info("netd", &alloc::format!(
            "IP assigned from relay: {}.{}.{}.{}/{}",
            octets[0], octets[1], octets[2], octets[3], PREFIX_LEN
        ));
        NETD_IP_ASSIGNED.store(true, Ordering::Release);
        return true;
    }
    
    false
}

/// Try to get IP from VirtIO network config space
fn try_virtio_ip() -> bool {
    let net = match crate::NET_STATE.try_lock() {
        Some(guard) => guard,
        None => return false,
    };
    
    if net.is_none() {
        return false;
    }
    
    // VirtIO network assigns IP during initialization
    // Check if it's been set
    if is_ip_assigned() {
        let ip = get_my_ip();
        let octets = ip.octets();
        klog_info("netd", &alloc::format!(
            "IP assigned (VirtIO): {}.{}.{}.{}/{}",
            octets[0], octets[1], octets[2], octets[3], PREFIX_LEN
        ));
        NETD_IP_ASSIGNED.store(true, Ordering::Release);
        return true;
    }
    
    false
}

/// netd service entry point (for scheduler)
pub fn netd_service() {
    loop {
        tick();
        // Yield to other tasks
        for _ in 0..1000 {
            core::hint::spin_loop();
        }
    }
}
