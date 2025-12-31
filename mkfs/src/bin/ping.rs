// ping - Send ICMP echo requests
//
// Usage:
//   ping <host>          Ping hostname or IP address

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{console_log, is_net_available, argc, argv, resolve_dns, format_ipv4, print, print_int, ping, PingResult};

    if argc() < 1 {
        console_log("Usage: ping <hostname>\n");
        return;
    }

    if !is_net_available() {
        console_log("\x1b[1;31m[X]\x1b[0m Network not available\n");
        return;
    }

    let mut arg_buf = [0u8; 256];
    let arg_len = match argv(0, &mut arg_buf) {
        Some(len) => len,
        None => {
            console_log("Error: Could not read hostname\n");
            return;
        }
    };

    let hostname = &arg_buf[..arg_len];
    let hostname_str = unsafe { core::str::from_utf8_unchecked(hostname) };

    // Resolve hostname
    let mut ip = [0u8; 4];
    if !resolve_dns(hostname_str, &mut ip) {
        console_log("\x1b[1;31mError:\x1b[0m Could not resolve ");
        print(hostname.as_ptr(), hostname.len());
        console_log("\n");
        return;
    }

    let mut ip_buf = [0u8; 16];
    let ip_len = format_ipv4(&ip, &mut ip_buf);

    console_log("PING ");
    print(hostname.as_ptr(), hostname.len());
    console_log(" (");
    print(ip_buf.as_ptr(), ip_len);
    console_log("): 56 data bytes\n");

    // Send 4 pings
    for seq in 0..4u16 {
        match ping(&ip, seq, 1000) {
            PingResult::Success { rtt_ms } => {
                console_log("64 bytes from ");
                print(ip_buf.as_ptr(), ip_len);
                console_log(": icmp_seq=");
                print_int(seq as i64);
                console_log(" time=");
                print_int(rtt_ms as i64);
                console_log(" ms\n");
            }
            PingResult::Timeout => {
                console_log("Request timeout for icmp_seq ");
                print_int(seq as i64);
                console_log("\n");
            }
            PingResult::NetworkError => {
                console_log("Network error for icmp_seq ");
                print_int(seq as i64);
                console_log("\n");
            }
        }
    }

    console_log("\n--- ");
    print(hostname.as_ptr(), hostname.len());
    console_log(" ping statistics ---\n");
    console_log("4 packets transmitted\n");
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
