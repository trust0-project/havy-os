//! netd - Network Daemon Service
//!
//! Background service that polls for IP assignment from the relay.
//! Runs after boot to provision the network IP address dynamically.

use core::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use crate::{clint::get_time_ms, device::uart, lock::utils::{NET_STATE, PING_STATE}, net::{self, PREFIX_LEN, get_my_ip, is_ip_assigned}, services::klogd::klog_info};

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


/// Poll the network stack
pub(crate) fn poll_network() {
    let timestamp = get_time_ms();

    // Poll the unified network state
    {
        let mut net_guard = NET_STATE.lock();
        if let Some(ref mut state) = *net_guard {
            state.poll(timestamp);
        }
    }

    // Then handle ping state separately to avoid holding both locks
    let mut ping_guard = PING_STATE.lock();
    if let Some(ref mut ping) = *ping_guard {
        // Check for ping reply
        if ping.waiting {
            let reply = {
                let mut net_guard = NET_STATE.lock();
                if let Some(ref mut state) = *net_guard {
                    state.check_ping_reply()
                } else {
                    None
                }
            };

            if let Some((from, _ident, seq)) = reply {
                if seq == ping.seq {
                    let rtt = timestamp - ping.sent_time;
                    ping.record_reply(rtt);

                    let mut ip_buf = [0u8; 16];
                    let ip_len = net::format_ipv4(from, &mut ip_buf);
                    uart::write_str("64 bytes from ");
                    uart::write_bytes(&ip_buf[..ip_len]);
                    uart::write_str(": icmp_seq=");
                    uart::write_u64(seq as u64);
                    uart::write_str(" time=");
                    uart::write_u64(rtt as u64);
                    uart::write_line(" ms");
                    ping.waiting = false;
                }
            }

            // Timeout after 5 seconds for current ping
            if timestamp - ping.sent_time > 5000 {
                uart::write_str("Request timeout for icmp_seq ");
                uart::write_u64(ping.seq as u64);
                uart::write_line("");
                ping.waiting = false;
            }
        }

        // In continuous mode, send next ping after 1 second interval
        if ping.continuous && !ping.waiting {
            if timestamp - ping.last_send_time >= 1000 {
                ping.seq = ping.seq.wrapping_add(1);
                ping.sent_time = timestamp;
                ping.last_send_time = timestamp;
                ping.packets_sent += 1;

                let send_result = {
                    let mut net_guard = NET_STATE.lock();
                    if let Some(ref mut state) = *net_guard {
                        state.send_ping(ping.target, ping.seq, timestamp)
                    } else {
                        Err("Network not available")
                    }
                };

                match send_result {
                    Ok(()) => {
                        ping.waiting = true;
                    }
                    Err(_e) => {
                        // Failed to send, will retry next interval
                    }
                }
            }
        }
    }
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
    
    // Try to get IP from D1 EMAC
    try_get_ip();
}

/// Try to get IP from D1 EMAC MMIO register
fn try_get_ip() -> bool {
    let mut net = match crate::NET_STATE.try_lock() {
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

/// netd service entry point (for scheduler)
pub fn netd_service() {
    // Ensure netd is initialized on first run
    if !NETD_INITIALIZED.load(Ordering::Acquire) {
        let _ = init();
    }
    
    // Poll network stack for traffic (packets, etc.)
    poll_network();
    
    // Check for IP assignment from relay
    tick();
}
