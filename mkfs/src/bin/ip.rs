// ip - Show network configuration
//
// Usage:
//   ip           Show network interface configuration
//   ip addr      Show network addresses

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use core::ptr::{addr_of, addr_of_mut};
    use mkfs::{console_log, get_net_info, format_ipv4, format_mac, is_net_available, argc, argv};

    // Static buffers
    static mut ARG_BUF: [u8; 64] = [0u8; 64];
    static mut IP_BUF: [u8; 16] = [0u8; 16];
    static mut MAC_BUF: [u8; 18] = [0u8; 18];

    #[no_mangle]
    pub extern "C" fn _start() {
        // Check for "addr" argument (or no args = show addr)
        // Note: arg 0 is the first arg since command name is not passed
        let show_addr = if argc() > 0 {
            let arg_len = unsafe { argv(0, &mut *addr_of_mut!(ARG_BUF)) };
            match arg_len {
                Some(len) if len <= 4 => {
                    let arg = unsafe { &(*addr_of!(ARG_BUF))[..len] };
                    arg == b"addr"
                }
                _ => false,
            }
        } else {
            true // Default to showing addr
        };

        if !show_addr {
            console_log("Usage: ip addr\n");
            return;
        }

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
        let mac_len = unsafe { format_mac(&info.mac, &mut *addr_of_mut!(MAC_BUF)) };
        console_log("\x1b[1;34m|\x1b[0m  \x1b[1;33mlink/ether\x1b[0m  ");
        unsafe { print_bytes(&(*addr_of!(MAC_BUF))[..mac_len]) };
        pad_spaces(47 - mac_len.min(47));
        console_log("\x1b[1;34m|\x1b[0m\n");

        // IP address
        let ip_len = unsafe { format_ipv4(&info.ip, &mut *addr_of_mut!(IP_BUF)) };
        console_log("\x1b[1;34m|\x1b[0m  \x1b[1;33minet\x1b[0m        ");
        unsafe { print_bytes(&(*addr_of!(IP_BUF))[..ip_len]) };
        console_log("/");
        print_u8(info.prefix_len);
        let inet_len = ip_len + 1 + digit_count_u8(info.prefix_len);
        pad_spaces(47 - inet_len.min(47));
        console_log("\x1b[1;34m|\x1b[0m\n");

        // Gateway
        let gw_len = unsafe { format_ipv4(&info.gateway, &mut *addr_of_mut!(IP_BUF)) };
        console_log("\x1b[1;34m|\x1b[0m  \x1b[1;33mgateway\x1b[0m     ");
        unsafe { print_bytes(&(*addr_of!(IP_BUF))[..gw_len]) };
        pad_spaces(47 - gw_len.min(47));
        console_log("\x1b[1;34m|\x1b[0m\n");

        console_log("\x1b[1;34m|\x1b[0m                                                             \x1b[1;34m|\x1b[0m\n");
        console_log("\x1b[1;34m|\x1b[0m  \x1b[1;32mState: UP\x1b[0m    \x1b[0;90mMTU: 1500    Type: VirtIO-Net\x1b[0m              \x1b[1;34m|\x1b[0m\n");
        console_log("\x1b[1;34m+-------------------------------------------------------------+\x1b[0m\n");
        console_log("\n");
    }

    fn pad_spaces(count: usize) {
        for _ in 0..count {
            console_log(" ");
        }
    }

    fn print_bytes(bytes: &[u8]) {
        unsafe { mkfs::print(bytes.as_ptr(), bytes.len()) };
    }

    fn print_u8(n: u8) {
        let mut buf = [0u8; 3];
        let len = u8_to_str(n, &mut buf);
        print_bytes(&buf[..len]);
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

    fn digit_count_u8(mut n: u8) -> usize {
        if n == 0 {
            return 1;
        }
        let mut count = 0;
        while n > 0 {
            count += 1;
            n /= 10;
        }
        count
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}

