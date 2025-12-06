// ping - Send ICMP echo requests
//
// Usage:
//   ping <ip>           Ping an IP address
//   ping <hostname>     Ping a hostname (DNS resolution)

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{
        console_log, is_net_available, resolve_dns, ping, format_ipv4,
        argc, argv, print_int, PingResult,
    };

    // Static buffers
    static mut ARG_BUF: [u8; 256] = [0u8; 256];
    static mut IP_BUF: [u8; 16] = [0u8; 16];

    #[no_mangle]
    pub extern "C" fn _start() {
        if argc() < 1 {
            console_log("Usage: ping <ip|hostname>\n");
            console_log("\x1b[0;90mExamples:\x1b[0m\n");
            console_log("  ping 10.0.2.2\n");
            console_log("  ping google.com\n");
            return;
        }

        if !is_net_available() {
            console_log("\x1b[1;31m✗\x1b[0m Network not initialized\n");
            return;
        }

        // Get target argument (arg 0 since command name is not passed)
        let arg_len = unsafe { argv(0, &mut ARG_BUF) };
        let Some(arg_len) = arg_len else {
            console_log("Error: Could not read argument\n");
            return;
        };
        
        // Trim whitespace
        let mut trimmed_len = arg_len;
        while trimmed_len > 0 {
            let b = unsafe { ARG_BUF[trimmed_len - 1] };
            if b == b' ' || b == b'\t' {
                trimmed_len -= 1;
            } else {
                break;
            }
        }
        
        let target = unsafe { &ARG_BUF[..trimmed_len] };

        // Try to parse as IP address first
        let ip = if let Some(parsed) = parse_ipv4(target) {
            parsed
        } else {
            // Try DNS resolution
            console_log("\x1b[0;90m[DNS]\x1b[0m Resolving ");
            print_bytes(target);
            console_log("...\n");

            let mut ip_bytes = [0u8; 4];
            let target_str = unsafe { core::str::from_utf8_unchecked(target) };
            if !resolve_dns(target_str, &mut ip_bytes) {
                console_log("\x1b[1;31m[DNS]\x1b[0m Failed to resolve: ");
                print_bytes(target);
                console_log("\n");
                return;
            }

            let ip_len = unsafe { format_ipv4(&ip_bytes, &mut IP_BUF) };
            console_log("\x1b[1;32m[DNS]\x1b[0m Resolved to \x1b[1;97m");
            unsafe { print_bytes(&IP_BUF[..ip_len]) };
            console_log("\x1b[0m\n");

            ip_bytes
        };

        // Display ping header
        let ip_len = unsafe { format_ipv4(&ip, &mut IP_BUF) };
        console_log("PING ");
        unsafe { print_bytes(&IP_BUF[..ip_len]) };
        console_log(" 56(84) bytes of data.\n");

        // Send pings (4 by default)
        let mut sent = 0u32;
        let mut received = 0u32;
        let mut min_rtt = u32::MAX;
        let mut max_rtt = 0u32;
        let mut total_rtt = 0u64;

        for seq in 1..=4u16 {
            match ping(&ip, seq, 5000) {
                PingResult::Success { rtt_ms } => {
                    console_log("64 bytes from ");
                    unsafe { print_bytes(&IP_BUF[..ip_len]) };
                    console_log(": icmp_seq=");
                    print_int(seq as i64);
                    console_log(" time=");
                    print_int(rtt_ms as i64);
                    console_log(" ms\n");

                    received += 1;
                    total_rtt += rtt_ms as u64;
                    if rtt_ms < min_rtt {
                        min_rtt = rtt_ms;
                    }
                    if rtt_ms > max_rtt {
                        max_rtt = rtt_ms;
                    }
                }
                PingResult::Timeout => {
                    console_log("Request timeout for icmp_seq ");
                    print_int(seq as i64);
                    console_log("\n");
                }
                PingResult::NetworkError => {
                    console_log("\x1b[1;31mNetwork error\x1b[0m for icmp_seq ");
                    print_int(seq as i64);
                    console_log("\n");
                }
            }
            sent += 1;
        }

        // Statistics
        console_log("\n--- ");
        unsafe { print_bytes(&IP_BUF[..ip_len]) };
        console_log(" ping statistics ---\n");
        
        console_log(unsafe { &core::str::from_utf8_unchecked(&IP_BUF[..ip_len]) });
        print_int(sent as i64);
        console_log(" packets transmitted, ");
        print_int(received as i64);
        console_log(" received, ");
        
        let loss = if sent > 0 { ((sent - received) * 100) / sent } else { 0 };
        print_int(loss as i64);
        console_log("% packet loss\n");

        if received > 0 {
            let avg_rtt = total_rtt / received as u64;
            console_log("rtt min/avg/max = ");
            print_int(min_rtt as i64);
            console_log("/");
            print_int(avg_rtt as i64);
            console_log("/");
            print_int(max_rtt as i64);
            console_log(" ms\n");
        }
    }

    /// Parse IPv4 address from bytes (e.g., "10.0.2.2")
    fn parse_ipv4(s: &[u8]) -> Option<[u8; 4]> {
        let mut ip = [0u8; 4];
        let mut octet_idx = 0;
        let mut current: u16 = 0;
        let mut has_digit = false;

        for &b in s {
            if b >= b'0' && b <= b'9' {
                current = current * 10 + (b - b'0') as u16;
                if current > 255 {
                    return None;
                }
                has_digit = true;
            } else if b == b'.' {
                if !has_digit || octet_idx >= 3 {
                    return None;
                }
                ip[octet_idx] = current as u8;
                octet_idx += 1;
                current = 0;
                has_digit = false;
            } else {
                return None;
            }
        }

        if has_digit && octet_idx == 3 {
            ip[3] = current as u8;
            Some(ip)
        } else {
            None
        }
    }

    fn print_bytes(bytes: &[u8]) {
        unsafe { mkfs::print(bytes.as_ptr(), bytes.len()) };
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}

