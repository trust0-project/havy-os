//! Ping state for continuous ping operations
//!
//! State management for ping command similar to Linux ping.

use smoltcp::wire::Ipv4Address;

/// State for continuous ping (like Linux ping command)
pub struct PingState {
    pub target: Ipv4Address,
    pub seq: u16,
    pub sent_time: i64,      // Time when current ping was sent
    pub last_send_time: i64, // Time when we last sent a ping (for 1s interval)
    pub waiting: bool,       // Waiting for reply to current ping
    pub continuous: bool,    // Whether running in continuous mode
    // Statistics
    pub packets_sent: u32,
    pub packets_received: u32,
    pub min_rtt: i64,
    pub max_rtt: i64,
    pub total_rtt: i64,
}

impl PingState {
    pub fn new(target: Ipv4Address, timestamp: i64) -> Self {
        PingState {
            target,
            seq: 0,
            sent_time: timestamp,
            last_send_time: 0,
            waiting: false,
            continuous: true,
            packets_sent: 0,
            packets_received: 0,
            min_rtt: i64::MAX,
            max_rtt: 0,
            total_rtt: 0,
        }
    }

    pub fn record_reply(&mut self, rtt: i64) {
        self.packets_received += 1;
        self.total_rtt += rtt;
        if rtt < self.min_rtt {
            self.min_rtt = rtt;
        }
        if rtt > self.max_rtt {
            self.max_rtt = rtt;
        }
    }

    pub fn avg_rtt(&self) -> i64 {
        if self.packets_received > 0 {
            self.total_rtt / self.packets_received as i64
        } else {
            0
        }
    }

    pub fn packet_loss_percent(&self) -> u32 {
        if self.packets_sent > 0 {
            ((self.packets_sent - self.packets_received) * 100) / self.packets_sent
        } else {
            0
        }
    }
}


