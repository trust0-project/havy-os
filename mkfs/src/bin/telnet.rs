// telnet - Simple telnet client
//
// Usage:
//   telnet <host> [port]      Connect to host

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{
        console_log, is_net_available, argc, argv, print, print_int,
        resolve_dns, format_ipv4, tcp_connect_ip, tcp_send_data, 
        tcp_recv_data, tcp_disconnect, tcp_get_status, TcpStatus,
        should_cancel, get_time, console_available, read_console, sleep
    };

    if argc() < 1 {
        console_log("Usage: telnet <host> [port]\n");
        console_log("Example: telnet towel.blinkenlights.nl 23\n");
        return;
    }

    if !is_net_available() {
        console_log("\x1b[1;31m[X]\x1b[0m Network not available\n");
        return;
    }

    // Parse host
    let mut host_buf = [0u8; 256];
    let host_len = match argv(0, &mut host_buf) {
        Some(len) => len,
        None => {
            console_log("Error: Could not read hostname\n");
            return;
        }
    };
    let hostname = unsafe { core::str::from_utf8_unchecked(&host_buf[..host_len]) };

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

    // Resolve hostname
    console_log("Resolving ");
    print(host_buf.as_ptr(), host_len);
    console_log("... ");
    
    let mut ip = [0u8; 4];
    if !resolve_dns(hostname, &mut ip) {
        console_log("\x1b[1;31mfailed\x1b[0m\n");
        console_log("Could not resolve hostname\n");
        return;
    }
    
    let mut ip_buf = [0u8; 16];
    let ip_len = format_ipv4(&ip, &mut ip_buf);
    console_log("\x1b[1;32m");
    print(ip_buf.as_ptr(), ip_len);
    console_log("\x1b[0m\n");

    // Connect
    console_log("Trying ");
    print(ip_buf.as_ptr(), ip_len);
    console_log(":");
    print_int(port as i64);
    console_log("...\n");

    if !tcp_connect_ip(&ip, port) {
        console_log("\x1b[1;31mConnection failed\x1b[0m\n");
        return;
    }

    // Wait for connection
    let start = get_time();
    let timeout = 10000; // 10 seconds
    
    loop {
        let status = tcp_get_status();
        match status {
            TcpStatus::Connected => {
                console_log("Connected to ");
                print(host_buf.as_ptr(), host_len);
                console_log(".\n");
                console_log("Escape character is '^]'.\n");
                console_log("(Press ESC or q to quit)\n\n");
                break;
            }
            TcpStatus::Failed => {
                console_log("\x1b[1;31mConnection refused\x1b[0m\n");
                tcp_disconnect();
                return;
            }
            TcpStatus::Closed => {
                console_log("\x1b[1;31mConnection closed\x1b[0m\n");
                return;
            }
            TcpStatus::Connecting => {
                if get_time() - start > timeout {
                    console_log("\x1b[1;31mConnection timeout\x1b[0m\n");
                    tcp_disconnect();
                    return;
                }
                sleep(50);
            }
        }
    }

    // Main loop - receive and display data, send user input
    let mut recv_buf = [0u8; 4096];
    let mut input_buf = [0u8; 256];
    let mut input_len = 0usize;
    
    loop {
        // Check for cancel
        if should_cancel() != 0 {
            break;
        }

        // Check connection status
        let status = tcp_get_status();
        if status == TcpStatus::Closed || status == TcpStatus::Failed {
            console_log("\n\x1b[33mConnection closed by remote host.\x1b[0m\n");
            break;
        }

        // Try to receive data
        if let Some(len) = tcp_recv_data(&mut recv_buf, 0) {
            if len > 0 {
                // Filter and print received data (handle telnet control sequences)
                print_telnet_data(&recv_buf[..len]);
            }
        }

        // Check for user input
        if console_available() > 0 {
            let mut ch_buf = [0u8; 1];
            if read_console(&mut ch_buf) > 0 {
                let ch = ch_buf[0];
                
                // ESC or 'q' to quit
                if ch == 0x1B || ch == b'q' {
                    console_log("\n\x1b[33mConnection closed.\x1b[0m\n");
                    break;
                }
                
                // Handle enter
                if ch == b'\r' || ch == b'\n' {
                    input_buf[input_len] = b'\r';
                    input_len += 1;
                    if input_len < input_buf.len() {
                        input_buf[input_len] = b'\n';
                        input_len += 1;
                    }
                    let _ = tcp_send_data(&input_buf[..input_len]);
                    input_len = 0;
                    console_log("\n");
                } else if ch == 0x7F || ch == 0x08 {
                    // Backspace
                    if input_len > 0 {
                        input_len -= 1;
                        console_log("\x08 \x08"); // Move back, erase, move back
                    }
                } else if ch >= 0x20 && ch < 0x7F {
                    // Printable character
                    if input_len < input_buf.len() - 2 {
                        input_buf[input_len] = ch;
                        input_len += 1;
                        print(&ch_buf as *const u8, 1);
                    }
                }
            }
        }
        
        // Small delay to prevent busy-waiting
        sleep(10);
    }

    tcp_disconnect();

    fn parse_u16(buf: &[u8]) -> Option<u16> {
        let mut n: u16 = 0;
        for &c in buf {
            if c >= b'0' && c <= b'9' {
                n = n.checked_mul(10)?.checked_add((c - b'0') as u16)?;
            } else {
                break;
            }
        }
        if n > 0 { Some(n) } else { None }
    }

    fn print_telnet_data(data: &[u8]) {
        let mut i = 0;
        while i < data.len() {
            let c = data[i];
            
            // Handle telnet commands (IAC = 0xFF)
            if c == 0xFF && i + 2 < data.len() {
                // Skip telnet negotiation sequences (IAC + cmd + option)
                i += 3;
                continue;
            }
            
            // Skip other control characters except CR, LF, tab
            if c < 0x20 && c != b'\r' && c != b'\n' && c != b'\t' {
                i += 1;
                continue;
            }
            
            // Print the character
            if c == b'\r' {
                // Skip CR, we'll handle LF
                i += 1;
                continue;
            }
            
            mkfs::print(&data[i] as *const u8, 1);
            i += 1;
        }
    }
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
