// netstat - Show network statistics
//
// Usage:
//   netstat      Display network configuration and statistics

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{console_log, is_net_available, get_net_info, format_ipv4, format_mac, print};

    if !is_net_available() {
        console_log("\x1b[1;31m[X]\x1b[0m Network not initialized\n");
        return;
    }

    let Some(info) = get_net_info() else {
        console_log("\x1b[1;31m[X]\x1b[0m Could not get network info\n");
        return;
    };

    console_log("\n");
    console_log("\x1b[1;35m+-------------------------------------------------------------+\x1b[0m\n");
    console_log("\x1b[1;35m|\x1b[0m                   \x1b[1;97mNetwork Statistics\x1b[0m                        \x1b[1;35m|\x1b[0m\n");
    console_log("\x1b[1;35m+-------------------------------------------------------------+\x1b[0m\n");
    console_log("\x1b[1;35m|\x1b[0m  \x1b[1;33mDevice:\x1b[0m                                                    \x1b[1;35m|\x1b[0m\n");
    console_log("\x1b[1;35m|\x1b[0m    Type:     \x1b[1;97mVirtIO Network Device\x1b[0m                          \x1b[1;35m|\x1b[0m\n");
    console_log("\x1b[1;35m|\x1b[0m    Address:  \x1b[1;97m0x10001000\x1b[0m                                     \x1b[1;35m|\x1b[0m\n");
    console_log("\x1b[1;35m|\x1b[0m    Status:   \x1b[1;32m* ONLINE\x1b[0m                                       \x1b[1;35m|\x1b[0m\n");
    console_log("\x1b[1;35m|\x1b[0m                                                             \x1b[1;35m|\x1b[0m\n");
    console_log("\x1b[1;35m|\x1b[0m  \x1b[1;33mConfiguration:\x1b[0m                                             \x1b[1;35m|\x1b[0m\n");

    // MAC address
    let mut mac_buf = [0u8; 18];
    let mac_len = format_mac(&info.mac, &mut mac_buf);
    console_log("\x1b[1;35m|\x1b[0m    MAC:      \x1b[1;97m");
    print(mac_buf.as_ptr(), mac_len);
    console_log("\x1b[0m");
    pad_spaces(45 - mac_len.min(45));
    console_log("\x1b[1;35m|\x1b[0m\n");

    // IP address
    let mut ip_buf = [0u8; 16];
    let ip_len = format_ipv4(&info.ip, &mut ip_buf);
    console_log("\x1b[1;35m|\x1b[0m    IP:       \x1b[1;97m");
    print(ip_buf.as_ptr(), ip_len);
    console_log("/");
    print_u8(info.prefix_len);
    console_log("\x1b[0m");
    let ip_full_len = ip_len + 1 + digit_count(info.prefix_len);
    pad_spaces(45 - ip_full_len.min(45));
    console_log("\x1b[1;35m|\x1b[0m\n");

    // Gateway
    let gw_len = format_ipv4(&info.gateway, &mut ip_buf);
    console_log("\x1b[1;35m|\x1b[0m    Gateway:  \x1b[1;97m");
    print(ip_buf.as_ptr(), gw_len);
    console_log("\x1b[0m");
    pad_spaces(45 - gw_len.min(45));
    console_log("\x1b[1;35m|\x1b[0m\n");

    // DNS
    let dns_len = format_ipv4(&info.dns, &mut ip_buf);
    console_log("\x1b[1;35m|\x1b[0m    DNS:      \x1b[1;97m");
    print(ip_buf.as_ptr(), dns_len);
    console_log("\x1b[0m");
    pad_spaces(45 - dns_len.min(45));
    console_log("\x1b[1;35m|\x1b[0m\n");

    console_log("\x1b[1;35m|\x1b[0m                                                             \x1b[1;35m|\x1b[0m\n");
    console_log("\x1b[1;35m|\x1b[0m  \x1b[1;33mProtocol Stack:\x1b[0m                                            \x1b[1;35m|\x1b[0m\n");
    console_log("\x1b[1;35m|\x1b[0m    \x1b[1;97msmoltcp\x1b[0m - Lightweight TCP/IP stack                       \x1b[1;35m|\x1b[0m\n");
    console_log("\x1b[1;35m|\x1b[0m    Protocols: ICMP, UDP, TCP, ARP                           \x1b[1;35m|\x1b[0m\n");
    console_log("\x1b[1;35m+-------------------------------------------------------------+\x1b[0m\n");
    console_log("\n");

    fn pad_spaces(count: usize) {
        for _ in 0..count {
            mkfs::console_log(" ");
        }
    }

    fn print_u8(n: u8) {
        let mut buf = [0u8; 3];
        let len = u8_to_str(n, &mut buf);
        mkfs::print(buf.as_ptr(), len);
    }

    fn u8_to_str(mut n: u8, buf: &mut [u8]) -> usize {
        if n == 0 {
            buf[0] = b'0';
            return 1;
        }
        let mut i = 0;
        let mut temp = [0u8; 3];
        while n > 0 {
            temp[i] = b'0' + (n % 10);
            n /= 10;
            i += 1;
        }
        for j in 0..i {
            buf[j] = temp[i - 1 - j];
        }
        i
    }

    fn digit_count(mut n: u8) -> usize {
        if n == 0 { return 1; }
        let mut count = 0;
        while n > 0 {
            count += 1;
            n /= 10;
        }
        count
    }
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
