// wget - Download files from web
//
// Usage:
//   wget <url>           Download file from URL
//   wget -O <file> <url> Download and save to file

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{console_log, is_net_available, argc, argv, print, http_fetch, print_int, write_file};

    let arg_count = argc();
    
    if arg_count < 1 {
        console_log("Usage: wget <url>\n");
        console_log("       wget -O <filename> <url>\n");
        console_log("Example: wget http://example.com/file.txt\n");
        console_log("         wget -O myfile.html http://example.com/\n");
        console_log("\n\x1b[33mNote:\x1b[0m HTTPS is supported but may be slow.\n");
        return;
    }

    if !is_net_available() {
        console_log("\x1b[1;31m[X]\x1b[0m Network not available\n");
        return;
    }

    // Parse arguments: check for -O flag
    let mut output_file: Option<&str> = None;
    let mut url_arg_idx: usize = 0;
    
    let mut arg0_buf = [0u8; 512];
    let arg0_len = argv(0, &mut arg0_buf).unwrap_or(0);
    let arg0 = unsafe { core::str::from_utf8_unchecked(&arg0_buf[..arg0_len]) };
    
    if arg0 == "-O" || arg0 == "-o" {
        // -O <filename> <url>
        if arg_count < 3 {
            console_log("\x1b[1;31m[X]\x1b[0m -O requires a filename and URL\n");
            console_log("Usage: wget -O <filename> <url>\n");
            return;
        }
        // Get filename from arg1
        static mut FNAME_BUF: [u8; 256] = [0u8; 256];
        let fname_buf = unsafe { &mut *core::ptr::addr_of_mut!(FNAME_BUF) };
        let fname_len = argv(1, fname_buf).unwrap_or(0);
        output_file = Some(unsafe { core::str::from_utf8_unchecked(&fname_buf[..fname_len]) });
        url_arg_idx = 2;
    } else {
        // No -O, first arg is URL
        url_arg_idx = 0;
    }

    let mut url_buf = [0u8; 512];
    let url_len = match argv(url_arg_idx, &mut url_buf) {
        Some(len) => len,
        None => {
            console_log("Error: Could not read URL\n");
            return;
        }
    };

    let url = unsafe { core::str::from_utf8_unchecked(&url_buf[..url_len]) };
    
    // Check for HTTPS
    if url.starts_with("https://") {
        console_log("\x1b[1;33m[!]\x1b[0m HTTPS detected - using TLS (may be slow)\n");
    }

    console_log("--");
    print_time();
    console_log("--  ");
    print(url_buf.as_ptr(), url_len);
    console_log("\n");
    
    console_log("Connecting... ");

    // Buffer for response (64KB max)
    static mut RESP_BUF: [u8; 65536] = [0u8; 65536];
    
    let resp_buf = unsafe { &mut *core::ptr::addr_of_mut!(RESP_BUF) };
    
    match http_fetch(url, resp_buf) {
        Some(len) => {
            console_log("\x1b[1;32mconnected\x1b[0m\n");
            console_log("HTTP request sent, awaiting response... ");
            console_log("\x1b[1;32m200 OK\x1b[0m\n");
            
            console_log("Length: ");
            print_int(len as i64);
            console_log(" bytes\n\n");
            
            // Determine filename
            let filename = if let Some(f) = output_file {
                f
            } else {
                extract_filename(url)
            };
            
            console_log("Saving to: '");
            print(filename.as_ptr(), filename.len());
            console_log("'\n\n");
            
            // If -O was specified, save to file
            if output_file.is_some() {
                let content = &resp_buf[..len];
                
                // Build full path - if no leading /, prepend /home/
                static mut PATH_BUF: [u8; 320] = [0u8; 320];
                let path_buf = unsafe { &mut *core::ptr::addr_of_mut!(PATH_BUF) };
                
                // Always build the path explicitly
                let path_len: usize;
                if filename.starts_with('/') {
                    // Use filename as-is
                    let fname_bytes = filename.as_bytes();
                    path_buf[..fname_bytes.len()].copy_from_slice(fname_bytes);
                    path_len = fname_bytes.len();
                } else {
                    // Prepend /home/
                    let prefix = b"/home/";
                    let fname_bytes = filename.as_bytes();
                    path_buf[..prefix.len()].copy_from_slice(prefix);
                    path_buf[prefix.len()..prefix.len() + fname_bytes.len()].copy_from_slice(fname_bytes);
                    path_len = prefix.len() + fname_bytes.len();
                }
                
                let full_path = unsafe { core::str::from_utf8_unchecked(&path_buf[..path_len]) };
                
                
                if write_file(full_path, content) {
                    // Kernel handles sync internally after write_file
                    console_log("\x1b[1;32m✓\x1b[0m Saved to '");
                    print(full_path.as_ptr(), full_path.len());
                    console_log("' (");
                    print_int(len as i64);
                    console_log(" bytes)\n");
                } else {
                    console_log("\x1b[1;31m[X]\x1b[0m Failed to write file (");
                    print(full_path.as_ptr(), full_path.len());
                    console_log(")\n");
                }
            } else {
                // Display content (for text files up to 32KB)
                if len > 0 && len <= 32768 {
                    console_log("--- Content ---\n");
                    let content = &resp_buf[..len];
                    // Print the whole content directly
                    print(content.as_ptr(), len);
                    if content[len - 1] != b'\n' {
                        console_log("\n");
                    }
                    console_log("--- End ---\n\n");
                } else if len > 32768 {
                    console_log("\x1b[90m(Content too large to display, ");
                    print_int(len as i64);
                    console_log(" bytes received)\x1b[0m\n\n");
                }
                
                console_log("\x1b[1;32m✓\x1b[0m Downloaded ");
                print_int(len as i64);
                console_log(" bytes\n");
            }
        }
        None => {
            console_log("\x1b[1;31mfailed\x1b[0m\n");
            console_log("\x1b[1;31m[X]\x1b[0m Could not fetch URL\n");
            console_log("\n\x1b[33mPossible causes:\x1b[0m\n");
            console_log("  - Server unreachable or timeout\n");
            console_log("  - HTTPS redirect (TLS handshake timeout)\n");
            console_log("  - DNS resolution failed\n");
            console_log("\n\x1b[90mTip: Try a plain HTTP URL like http://example.com\x1b[0m\n");
        }
    }

    fn extract_filename(url: &str) -> &str {
        // Find last '/' and get everything after it
        let bytes = url.as_bytes();
        let mut last_slash = 0;
        for (i, &c) in bytes.iter().enumerate() {
            if c == b'/' {
                last_slash = i;
            }
        }
        if last_slash > 0 && last_slash + 1 < bytes.len() {
            // Get the part after the last slash
            let name = &url[last_slash + 1..];
            // Remove query string if any
            if let Some(q) = name.find('?') {
                return &name[..q];
            }
            if name.is_empty() {
                return "index.html";
            }
            return name;
        }
        "index.html"
    }

    fn print_time() {
        let time_ms = mkfs::get_time();
        let hours = (time_ms / 3600000) % 24;
        let mins = (time_ms / 60000) % 60;
        let secs = (time_ms / 1000) % 60;
        mkfs::print_int(hours);
        mkfs::console_log(":");
        if mins < 10 { mkfs::console_log("0"); }
        mkfs::print_int(mins);
        mkfs::console_log(":");
        if secs < 10 { mkfs::console_log("0"); }
        mkfs::print_int(secs);
    }
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
