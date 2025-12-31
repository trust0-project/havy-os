// ip - Show network configuration
//
// Usage:
//   ip           Show network interface configuration
//   ip addr      Show network addresses

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
    console_log("\x1b[1;34m+-------------------------------------------------------------+\x1b[0m\n");
    console_log("\x1b[1;34m|\x1b[0m            \x1b[1;97mNetwork Interface: virtio0\x1b[0m                       \x1b[1;34m|\x1b[0m\n");
    console_log("\x1b[1;34m+-------------------------------------------------------------+\x1b[0m\n");

    // MAC address
    let mut mac_buf = [0u8; 18];
    let mac_len = format_mac(&info.mac, &mut mac_buf);
    console_log("\x1b[1;34m|\x1b[0m  \x1b[1;33mlink/ether\x1b[0m  ");
    print(mac_buf.as_ptr(), mac_len);
    pad_spaces(47 - mac_len.min(47));
    console_log("\x1b[1;34m|\x1b[0m\n");

    // IP address
    let mut ip_buf = [0u8; 16];
    let ip_len = format_ipv4(&info.ip, &mut ip_buf);
    console_log("\x1b[1;34m|\x1b[0m  \x1b[1;33minet\x1b[0m        ");
    print(ip_buf.as_ptr(), ip_len);
    console_log("/");
    print_u8(info.prefix_len);
    let inet_len = ip_len + 1 + digit_count(info.prefix_len);
    pad_spaces(47 - inet_len.min(47));
    console_log("\x1b[1;34m|\x1b[0m\n");

    // Gateway
    let gw_len = format_ipv4(&info.gateway, &mut ip_buf);
    console_log("\x1b[1;34m|\x1b[0m  \x1b[1;33mgateway\x1b[0m     ");
    print(ip_buf.as_ptr(), gw_len);
    pad_spaces(47 - gw_len.min(47));
    console_log("\x1b[1;34m|\x1b[0m\n");

    // DNS
    let dns_len = format_ipv4(&info.dns, &mut ip_buf);
    console_log("\x1b[1;34m|\x1b[0m  \x1b[1;33mdns\x1b[0m         ");
    print(ip_buf.as_ptr(), dns_len);
    pad_spaces(47 - dns_len.min(47));
    console_log("\x1b[1;34m|\x1b[0m\n");

    console_log("\x1b[1;34m|\x1b[0m                                                             \x1b[1;34m|\x1b[0m\n");
    console_log("\x1b[1;34m|\x1b[0m  \x1b[1;32mState: UP\x1b[0m    \x1b[0;90mMTU: 1500    Type: VirtIO-Net\x1b[0m              \x1b[1;34m|\x1b[0m\n");
    console_log("\x1b[1;34m+-------------------------------------------------------------+\x1b[0m\n");
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
