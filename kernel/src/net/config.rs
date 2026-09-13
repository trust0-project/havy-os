//! Network configuration constants and IP address management.

use smoltcp::wire::Ipv4Address;

/// Network configuration
/// Default IP address: 0.0.0.0 (unassigned until DHCPv4)
pub const DEFAULT_IP_ADDR: Ipv4Address = Ipv4Address::new(0, 0, 0, 0);
pub const GATEWAY: Ipv4Address = Ipv4Address::new(10, 0, 2, 2);
pub const PREFIX_LEN: u8 = 24;

/// Dynamic IP address assigned by DHCPv4
/// This is set when the DHCP client reaches Configured.
pub static mut MY_IP_ADDR: Ipv4Address = Ipv4Address::new(0, 0, 0, 0);

/// Get the current IP address (safe wrapper)
pub fn get_my_ip() -> Ipv4Address {
    unsafe { MY_IP_ADDR }
}

/// Set the IP address (called when DHCPv4 configures the iface)
pub fn set_my_ip(ip: Ipv4Address) {
    unsafe { MY_IP_ADDR = ip; }
}

/// Check if an IP has been assigned (not 0.0.0.0)
pub fn is_ip_assigned() -> bool {
    let ip = unsafe { MY_IP_ADDR };
    ip.octets() != [0, 0, 0, 0]
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

