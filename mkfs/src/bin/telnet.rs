// telnet - Simple telnet client for BAVY OS
//
// Usage: telnet <host/ip> [port]

#![no_std]
#![no_main]

extern crate mkfs;

use mkfs::{
    console_log, argc, argv, resolve_dns, format_ipv4, sleep,
    tcp_connect_ip, tcp_send_data, tcp_recv_data, tcp_disconnect, tcp_get_status,
    is_console_available, read_console, TcpStatus,
};

#[no_mangle]
pub extern "C" fn _start() {
    // Parse arguments: argv(0) = host, argv(1) = port
    if argc() < 1 {
        console_log("Usage: telnet <host/ip> [port]\n");
        console_log("Example: telnet 10.0.2.253 30\n");
        return;
    }

    let mut host_buf = [0u8; 128];
    let host_len = argv(0, &mut host_buf).unwrap_or(0);
    if host_len == 0 {
        console_log("Error: Invalid host\n");
        return;
    }

    // Parse port (default 23)
    let port: u16 = if argc() >= 2 {
        let mut port_buf = [0u8; 16];
        if let Some(len) = argv(1, &mut port_buf) {
            parse_u16(&port_buf[..len]).unwrap_or(23)
        } else {
            23
        }
    } else {
        23
    };

    // Resolve host to IP
    let ip = if is_ip_address(&host_buf[..host_len]) {
        parse_ip(&host_buf[..host_len])
    } else {
        // DNS lookup
        let mut ip_buf = [0u8; 4];
        let host = unsafe { core::str::from_utf8_unchecked(&host_buf[..host_len]) };
        if resolve_dns(host, &mut ip_buf) {
            ip_buf
        } else {
            console_log("Error: Could not resolve host\n");
            return;
        }
    };

    // Show connection attempt
    console_log("Trying ");
    let mut ip_str = [0u8; 16];
    let ip_len = format_ipv4(&ip, &mut ip_str);
    console_log(unsafe { core::str::from_utf8_unchecked(&ip_str[..ip_len]) });
    console_log(":");
    print_u16(port);
    console_log("...\n");

    // Connect
    if !tcp_connect_ip(&ip, port) {
        console_log("Error: Connection failed\n");
        return;
    }

    // Wait for connection (up to 5 seconds)
    // Poll frequently (every 10ms) to catch SYN-ACK packets quickly
    let mut connected = false;
    for _ in 0..500 {
        match tcp_get_status() {
            TcpStatus::Connected => {
                connected = true;
                break;
            }
            TcpStatus::Failed => {
                console_log("Error: Connection refused\n");
                return;
            }
            TcpStatus::Closed => {
                console_log("Error: Connection closed\n");
                return;
            }
            TcpStatus::Connecting => {
                sleep(10);  // Poll every 10ms during handshake
            }
        }
    }

    if !connected {
        console_log("Error: Connection timeout\n");
        tcp_disconnect();
        return;
    }

    console_log("Connected to ");
    console_log(unsafe { core::str::from_utf8_unchecked(&ip_str[..ip_len]) });
    console_log(".\n");
    console_log("Type your input. Press Ctrl+C to quit.\n\n");

    let mut bytes_sent: u32 = 0;
    let mut bytes_received: u32 = 0;
    let mut recv_buf = [0u8; 512];
    let mut send_buf = [0u8; 256];
    let mut send_len = 0usize;

    // Main loop
    loop {
        // Receive data FIRST (before checking connection status)
        // This ensures we get any pending data before the connection closes
        let mut got_data = false;
        if let Some(len) = tcp_recv_data(&mut recv_buf, 0) {
            if len > 0 {
                bytes_received += len as u32;
                got_data = true;
                // Print received data
                console_log(unsafe { core::str::from_utf8_unchecked(&recv_buf[..len]) });
            }
        }
        
        // Check connection status - but allow one more receive if we just got data
        let status = tcp_get_status();
        if status != TcpStatus::Connected && !got_data {
            // Try one final receive before giving up
            if let Some(len) = tcp_recv_data(&mut recv_buf, 0) {
                if len > 0 {
                    bytes_received += len as u32;
                    console_log(unsafe { core::str::from_utf8_unchecked(&recv_buf[..len]) });
                }
            }
            break;
        }

        // Check for console input
        let mut ch = [0u8; 1];
        if is_console_available() {
            let n = read_console(&mut ch);
            if n > 0 {
                // Check for Ctrl+C
                if ch[0] == 3 {
                    console_log("\n^C\n");
                    break;
                }
                
                // Echo character
                console_log(unsafe { core::str::from_utf8_unchecked(&ch[..1]) });
                
                // Buffer the character
                if send_len < send_buf.len() - 1 {
                    send_buf[send_len] = ch[0];
                    send_len += 1;
                }
                
                // Send on Enter
                if ch[0] == b'\r' || ch[0] == b'\n' {
                    if send_len > 0 {
                        // Add newline if not present
                        if send_buf[send_len - 1] != b'\n' {
                            if send_len < send_buf.len() {
                                send_buf[send_len] = b'\n';
                                send_len += 1;
                            }
                        }
                        
                        if let Some(sent) = tcp_send_data(&send_buf[..send_len]) {
                            bytes_sent += sent as u32;
                        }
                        send_len = 0;
                    }
                }
            }
        }

        // Small delay to avoid busy loop
        sleep(10);
    }

    tcp_disconnect();
}

/// Check if string looks like an IP address
fn is_ip_address(s: &[u8]) -> bool {
    let mut dots = 0;
    let mut has_digit = false;
    for &b in s {
        match b {
            b'.' => dots += 1,
            b'0'..=b'9' => has_digit = true,
            // Ignore trailing whitespace/newlines
            b' ' | b'\n' | b'\r' | b'\t' | 0 => break,
            _ => return false,
        }
    }
    dots == 3 && has_digit
}

/// Parse IP address from string
fn parse_ip(s: &[u8]) -> [u8; 4] {
    let mut ip = [0u8; 4];
    let mut idx = 0;
    let mut num: u16 = 0;
    
    for &b in s {
        if b == b'.' {
            if idx < 4 {
                ip[idx] = num as u8;
                idx += 1;
                num = 0;
            }
        } else if b >= b'0' && b <= b'9' {
            num = num * 10 + (b - b'0') as u16;
        }
    }
    if idx < 4 {
        ip[idx] = num as u8;
    }
    ip
}

/// Parse u16 from string
fn parse_u16(s: &[u8]) -> Option<u16> {
    let mut result: u16 = 0;
    for &b in s {
        if b >= b'0' && b <= b'9' {
            result = result.checked_mul(10)?.checked_add((b - b'0') as u16)?;
        } else {
            break;
        }
    }
    Some(result)
}

/// Print u16
fn print_u16(n: u16) {
    let mut buf = [0u8; 6];
    let mut i = buf.len();
    let mut num = n;
    if num == 0 {
        console_log("0");
        return;
    }
    while num > 0 && i > 0 {
        i -= 1;
        buf[i] = b'0' + (num % 10) as u8;
        num /= 10;
    }
    console_log(unsafe { core::str::from_utf8_unchecked(&buf[i..]) });
}

/// Print u32
fn print_u32(n: u32) {
    let mut buf = [0u8; 11];
    let mut i = buf.len();
    let mut num = n;
    if num == 0 {
        console_log("0");
        return;
    }
    while num > 0 && i > 0 {
        i -= 1;
        buf[i] = b'0' + (num % 10) as u8;
        num /= 10;
    }
    console_log(unsafe { core::str::from_utf8_unchecked(&buf[i..]) });
}
