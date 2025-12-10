//! Network utility functions.
//!
//! This module provides helper functions for IP address parsing and formatting.

use smoltcp::wire::Ipv4Address;

/// Parse an IPv4 address from bytes
pub fn parse_ipv4(s: &[u8]) -> Option<Ipv4Address> {
    let mut octets = [0u8; 4];
    let mut octet_idx = 0;
    let mut current = 0u16;
    let mut has_digit = false;

    for &b in s {
        if b >= b'0' && b <= b'9' {
            current = current * 10 + (b - b'0') as u16;
            has_digit = true;
            if current > 255 {
                return None;
            }
        } else if b == b'.' {
            if !has_digit || octet_idx >= 3 {
                return None;
            }
            octets[octet_idx] = current as u8;
            octet_idx += 1;
            current = 0;
            has_digit = false;
        } else {
            return None;
        }
    }

    if !has_digit || octet_idx != 3 {
        return None;
    }
    octets[3] = current as u8;

    Some(Ipv4Address::new(octets[0], octets[1], octets[2], octets[3]))
}

/// Format an IPv4 address to a buffer
pub fn format_ipv4(addr: Ipv4Address, buf: &mut [u8]) -> usize {
    let octets = addr.octets();
    let mut pos = 0;

    for (i, &octet) in octets.iter().enumerate() {
        // Write octet
        if octet >= 100 {
            buf[pos] = b'0' + (octet / 100);
            pos += 1;
        }
        if octet >= 10 {
            buf[pos] = b'0' + ((octet / 10) % 10);
            pos += 1;
        }
        buf[pos] = b'0' + (octet % 10);
        pos += 1;

        // Write dot (except after last octet)
        if i < 3 {
            buf[pos] = b'.';
            pos += 1;
        }
    }

    pos
}
