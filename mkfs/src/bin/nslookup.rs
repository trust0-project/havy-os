// nslookup - DNS lookup utility
//
// Usage:
//   nslookup <hostname>    Look up hostname via DNS

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{
        console_log, is_net_available, resolve_dns, get_net_info, format_ipv4,
        argc, argv,
    };

    // Static buffers
    static mut ARG_BUF: [u8; 256] = [0u8; 256];
    static mut IP_BUF: [u8; 16] = [0u8; 16];

    #[no_mangle]
    pub extern "C" fn _start() {
        if argc() < 1 {
            console_log("Usage: nslookup <hostname>\n");
            console_log("\x1b[0;90mExample: nslookup google.com\x1b[0m\n");
            return;
        }

        if !is_net_available() {
            console_log("\x1b[1;31m✗\x1b[0m Network not initialized\n");
            return;
        }

        // Get hostname argument (arg 0 since command name is not passed)
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

        let hostname = unsafe { &ARG_BUF[..trimmed_len] };
        let hostname_str = unsafe { core::str::from_utf8_unchecked(hostname) };

        console_log("\n");

        // Show DNS server
        if let Some(info) = get_net_info() {
            let dns_len = unsafe { format_ipv4(&info.dns, &mut IP_BUF) };
            console_log("\x1b[1;33mServer:\x1b[0m  ");
            unsafe { print_bytes(&IP_BUF[..dns_len]) };
            console_log("\n");
        } else {
            console_log("\x1b[1;33mServer:\x1b[0m  8.8.8.8\n");
        }
        console_log("\x1b[1;33mPort:\x1b[0m    53\n\n");

        console_log("\x1b[0;90mQuerying ");
        print_bytes(hostname);
        console_log("...\x1b[0m\n");

        // Perform DNS lookup
        let mut ip_bytes = [0u8; 4];
        if resolve_dns(hostname_str, &mut ip_bytes) {
            console_log("\n");
            console_log("\x1b[1;32mName:\x1b[0m    ");
            print_bytes(hostname);
            console_log("\n");

            let ip_len = unsafe { format_ipv4(&ip_bytes, &mut IP_BUF) };
            console_log("\x1b[1;32mAddress:\x1b[0m \x1b[1;97m");
            unsafe { print_bytes(&IP_BUF[..ip_len]) };
            console_log("\x1b[0m\n\n");
        } else {
            console_log("\n");
            console_log("\x1b[1;31m*** Can't find ");
            print_bytes(hostname);
            console_log(": No response from server\x1b[0m\n\n");
        }
    }

    fn print_bytes(bytes: &[u8]) {
        unsafe { mkfs::print(bytes.as_ptr(), bytes.len()) };
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}

