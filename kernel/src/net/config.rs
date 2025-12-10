//! Network configuration constants and IP address management.

use smoltcp::wire::Ipv4Address;

/// Network configuration
/// Default IP address (used as fallback if no IP is assigned by relay)
pub const DEFAULT_IP_ADDR: Ipv4Address = Ipv4Address::new(10, 0, 2, 15);
pub const GATEWAY: Ipv4Address = Ipv4Address::new(10, 0, 2, 2);
pub const PREFIX_LEN: u8 = 24;

/// Dynamic IP address assigned by the relay/network controller
/// This is set during network initialization
pub static mut MY_IP_ADDR: Ipv4Address = Ipv4Address::new(10, 0, 2, 15);

/// Get the current IP address (safe wrapper)
pub fn get_my_ip() -> Ipv4Address {
    unsafe { MY_IP_ADDR }
}

/// DNS server (Google Public DNS)
pub const DNS_SERVER: Ipv4Address = Ipv4Address::new(8, 8, 8, 8);
/// DNS port
pub const DNS_PORT: u16 = 53;

/// Loopback address
pub const LOOPBACK: Ipv4Address = Ipv4Address::new(127, 0, 0, 1);

/// ICMP identifier for our ping socket
pub const ICMP_IDENT: u16 = 0x1234;

/// Local port for DNS queries
pub const DNS_LOCAL_PORT: u16 = 10053;
