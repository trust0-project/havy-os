// nslookup - DNS lookup utility
//
// Usage:
//   nslookup <hostname>    Look up hostname via DNS

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{console_log, is_net_available, argc, argv, resolve_dns, format_ipv4, print};

    if argc() < 1 {
        console_log("Usage: nslookup <hostname>\n");
        console_log("\x1b[0;90mExample: nslookup google.com\x1b[0m\n");
        return;
    }

    if !is_net_available() {
        console_log("\x1b[1;31m[X]\x1b[0m Network not initialized\n");
        return;
    }

    let mut arg_buf = [0u8; 256];
    let arg_len = match argv(0, &mut arg_buf) {
        Some(len) => len,
        None => {
            console_log("Error: Could not read argument\n");
            return;
        }
    };

    let hostname = &arg_buf[..arg_len];
    let hostname_str = unsafe { core::str::from_utf8_unchecked(hostname) };

    console_log("\n");
    console_log("\x1b[1;33mServer:\x1b[0m  8.8.8.8\n");
    console_log("\x1b[1;33mPort:\x1b[0m    53\n\n");

    console_log("\x1b[0;90mQuerying ");
    print(hostname.as_ptr(), hostname.len());
    console_log("...\x1b[0m\n");

    let mut ip_bytes = [0u8; 4];
    if resolve_dns(hostname_str, &mut ip_bytes) {
        console_log("\n");
        console_log("\x1b[1;32mName:\x1b[0m    ");
        print(hostname.as_ptr(), hostname.len());
        console_log("\n");

        let mut ip_buf = [0u8; 16];
        let ip_len = format_ipv4(&ip_bytes, &mut ip_buf);
        console_log("\x1b[1;32mAddress:\x1b[0m \x1b[1;97m");
        print(ip_buf.as_ptr(), ip_len);
        console_log("\x1b[0m\n\n");
    } else {
        console_log("\n");
        console_log("\x1b[1;31m*** Can't find ");
        print(hostname.as_ptr(), hostname.len());
        console_log(": No response from server\x1b[0m\n\n");
    }
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
