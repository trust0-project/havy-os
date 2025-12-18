// ls - List directory contents
//
// Usage:
//   ls              List current directory
//   ls <dir>        List specified directory
//   ls -l           Long format with sizes

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use core::ptr::addr_of_mut;
    use mkfs::{console_log, argc, argv, get_cwd};
    use mkfs::syscalls::{print, fs_list};

    // Static buffers
    static mut LIST_BUF: [u8; 4096] = [0u8; 4096];
    static mut NAMES: [u8; 2048] = [0u8; 2048];
    
    // Entry arrays
    static mut E_START: [u16; 64] = [0; 64];
    static mut E_LEN: [u8; 64] = [0; 64];
    static mut E_SIZE: [u32; 64] = [0; 64];
    static mut E_DIR: [u8; 64] = [0; 64];
    
    // Seen directories
    static mut SEEN_START: [u16; 32] = [0; 32];
    static mut SEEN_LEN: [u8; 32] = [0; 32];

    // Manual byte comparison to avoid optimized SWAR code
    fn bytes_eq(a: &[u8], b: &[u8]) -> bool {
        if a.len() != b.len() { return false; }
        for i in 0..a.len() {
            if a[i] != b[i] { return false; }
        }
        true
    }

    fn bytes_starts_with(haystack: &[u8], needle: &[u8]) -> bool {
        if needle.len() > haystack.len() { return false; }
        for i in 0..needle.len() {
            if haystack[i] != needle[i] { return false; }
        }
        true
    }

    fn bytes_cmp(a: &[u8], b: &[u8]) -> i32 {
        let min_len = if a.len() < b.len() { a.len() } else { b.len() };
        for i in 0..min_len {
            if a[i] < b[i] { return -1; }
            if a[i] > b[i] { return 1; }
        }
        if a.len() < b.len() { -1 }
        else if a.len() > b.len() { 1 }
        else { 0 }
    }

    fn bytes_contains(haystack: &[u8], needle: u8) -> bool {
        for &b in haystack {
            if b == needle { return true; }
        }
        false
    }

    #[no_mangle]
    pub extern "C" fn _start() {
        let arg_count = argc();
        let mut show_long = false;
        let mut target = [0u8; 128];
        let mut target_len: usize = 1;
        target[0] = b'/';

        if let Some(len) = get_cwd(&mut target) {
            target_len = len;
        }

        for i in 0..arg_count {
            let mut arg = [0u8; 64];
            if let Some(len) = argv(i, &mut arg) {
                if len > 0 && arg[0] == b'-' {
                    for j in 1..len {
                        if arg[j] == b'l' { show_long = true; }
                    }
                } else if len > 0 && arg[0] == b'/' {
                    let copy = len.min(128);
                    target[..copy].copy_from_slice(&arg[..copy]);
                    target_len = copy;
                }
            }
        }

        if target_len > 1 && target[target_len - 1] == b'/' {
            target_len -= 1;
        }

        let list_len = unsafe {
            let result = fs_list((*addr_of_mut!(LIST_BUF)).as_mut_ptr(), 4096);
            if result < 0 {
                console_log("\x1b[31mError: filesystem not available\x1b[0m\n");
                return;
            }
            result as usize
        };

        if list_len == 0 {
            console_log("\x1b[90m(empty)\x1b[0m\n");
            return;
        }

        let data = unsafe { &LIST_BUF[..list_len] };
        let is_root = target_len == 1 && target[0] == b'/';

        let mut prefix = [0u8; 130];
        let prefix_len = if is_root {
            prefix[0] = b'/';
            1
        } else {
            prefix[..target_len].copy_from_slice(&target[..target_len]);
            prefix[target_len] = b'/';
            target_len + 1
        };

        let mut entry_count: usize = 0;
        let mut names_pos: usize = 0;
        let mut seen_count: usize = 0;
        let mut pos: usize = 0;

        while pos < list_len && entry_count < 64 {
            let line_start = pos;
            while pos < list_len && data[pos] != b'\n' { pos += 1; }
            let line_end = pos;
            pos += 1;
            if line_start >= line_end { continue; }

            let line = &data[line_start..line_end];

            let mut colon = line.len();
            for (i, &c) in line.iter().enumerate().rev() {
                if c == b':' { colon = i; break; }
            }
            if colon >= line.len() { continue; }

            let path = &line[..colon];
            let size_str = &line[colon + 1..];

            let mut size: u32 = 0;
            for &c in size_str {
                if c >= b'0' && c <= b'9' {
                    size = size.saturating_mul(10).saturating_add((c - b'0') as u32);
                }
            }

            // Check if under target (use manual comparison)
            let is_under = if is_root {
                (path.len() > 0 && path[0] == b'/') || !bytes_contains(path, b'/')
            } else {
                bytes_starts_with(path, &prefix[..prefix_len])
            };
            if !is_under { continue; }

            let relative = if is_root {
                if path.len() > 0 && path[0] == b'/' && path.len() > 1 {
                    &path[1..]
                } else if !bytes_contains(path, b'/') {
                    path
                } else { continue; }
            } else {
                if prefix_len >= path.len() { continue; }
                &path[prefix_len..]
            };
            if relative.is_empty() { continue; }

            let mut has_slash = false;
            let mut slash_pos = 0;
            for (i, &c) in relative.iter().enumerate() {
                if c == b'/' { has_slash = true; slash_pos = i; break; }
            }

            unsafe {
                if has_slash {
                    let dir_name = &relative[..slash_pos];
                    let mut already_seen = false;
                    for d in 0..seen_count {
                        let start = SEEN_START[d] as usize;
                        let len = SEEN_LEN[d] as usize;
                        if bytes_eq(&NAMES[start..start+len], dir_name) {
                            already_seen = true;
                            break;
                        }
                    }
                    if !already_seen && seen_count < 32 && entry_count < 64 && names_pos + dir_name.len() <= NAMES.len() {
                        let copy_len = dir_name.len().min(255);
                        NAMES[names_pos..names_pos + copy_len].copy_from_slice(&dir_name[..copy_len]);
                        SEEN_START[seen_count] = names_pos as u16;
                        SEEN_LEN[seen_count] = copy_len as u8;
                        seen_count += 1;
                        E_START[entry_count] = names_pos as u16;
                        E_LEN[entry_count] = copy_len as u8;
                        E_SIZE[entry_count] = 0;
                        E_DIR[entry_count] = 1;
                        entry_count += 1;
                        names_pos += copy_len;
                    }
                } else {
                    let copy_len = relative.len().min(255);
                    if names_pos + copy_len <= NAMES.len() && entry_count < 64 {
                        NAMES[names_pos..names_pos + copy_len].copy_from_slice(&relative[..copy_len]);
                        E_START[entry_count] = names_pos as u16;
                        E_LEN[entry_count] = copy_len as u8;
                        E_SIZE[entry_count] = size;
                        E_DIR[entry_count] = 0;
                        entry_count += 1;
                        names_pos += copy_len;
                    }
                }
            }
        }

        if entry_count == 0 {
            console_log("\x1b[90m(empty)\x1b[0m\n");
            return;
        }

        // Sort (use manual comparison)
        unsafe {
            for i in 0..entry_count {
                for j in i + 1..entry_count {
                    let swap = if E_DIR[i] != E_DIR[j] {
                        E_DIR[i] == 0 && E_DIR[j] == 1
                    } else {
                        let a_start = E_START[i] as usize;
                        let a_len = E_LEN[i] as usize;
                        let b_start = E_START[j] as usize;
                        let b_len = E_LEN[j] as usize;
                        bytes_cmp(&NAMES[a_start..a_start+a_len], &NAMES[b_start..b_start+b_len]) > 0
                    };
                    if swap {
                        let ts = E_START[i]; E_START[i] = E_START[j]; E_START[j] = ts;
                        let tl = E_LEN[i]; E_LEN[i] = E_LEN[j]; E_LEN[j] = tl;
                        let tz = E_SIZE[i]; E_SIZE[i] = E_SIZE[j]; E_SIZE[j] = tz;
                        let td = E_DIR[i]; E_DIR[i] = E_DIR[j]; E_DIR[j] = td;
                    }
                }
            }
        }

        // Check if /usr/bin (manual)
        let is_usr_bin = target_len >= 8 && bytes_eq(&target[..8], b"/usr/bin");

        if show_long {
            for i in 0..entry_count {
                unsafe {
                    let start = E_START[i] as usize;
                    let len = E_LEN[i] as usize;
                    let name = &NAMES[start..start + len];

                    if E_DIR[i] == 1 {
                        console_log(" \x1b[90m<dir>\x1b[0m  \x1b[1;34m");
                        print(name.as_ptr(), name.len());
                        console_log("/\x1b[0m\n");
                    } else {
                        let s = E_SIZE[i];
                        if s < 10 { console_log("     "); }
                        else if s < 100 { console_log("    "); }
                        else if s < 1000 { console_log("   "); }
                        else if s < 10000 { console_log("  "); }
                        else if s < 100000 { console_log(" "); }
                        print_u32(s);
                        console_log("  ");
                        if is_usr_bin { console_log("\x1b[1;32m"); }
                        print(name.as_ptr(), name.len());
                        if is_usr_bin { console_log("\x1b[0m"); }
                        console_log("\n");
                    }
                }
            }
            let mut dir_count: u32 = 0;
            for i in 0..entry_count {
                unsafe { if E_DIR[i] == 1 { dir_count += 1; } }
            }
            console_log("\n\x1b[90m");
            print_u32(dir_count);
            console_log(" dir(s), ");
            print_u32(entry_count as u32 - dir_count);
            console_log(" file(s)\x1b[0m\n");
        } else {
            let mut max_len: usize = 4;
            for i in 0..entry_count {
                unsafe {
                    let len = E_LEN[i] as usize + if E_DIR[i] == 1 { 1 } else { 0 };
                    if len > max_len { max_len = len; }
                }
            }
            let col_width = (max_len + 2).max(4);
            let num_cols = (60 / col_width).max(1);
            let mut col = 0;

            for i in 0..entry_count {
                unsafe {
                    let start = E_START[i] as usize;
                    let len = E_LEN[i] as usize;
                    let name = &NAMES[start..start + len];
                    let display_len = len + if E_DIR[i] == 1 { 1 } else { 0 };

                    if E_DIR[i] == 1 {
                        console_log("\x1b[1;34m");
                        print(name.as_ptr(), name.len());
                        console_log("/\x1b[0m");
                    } else if is_usr_bin {
                        console_log("\x1b[1;32m");
                        print(name.as_ptr(), name.len());
                        console_log("\x1b[0m");
                    } else {
                        print(name.as_ptr(), name.len());
                    }

                    col += 1;
                    if col >= num_cols {
                        console_log("\n");
                        col = 0;
                    } else {
                        for _ in 0..(col_width - display_len) {
                            console_log(" ");
                        }
                    }
                }
            }
            if col > 0 { console_log("\n"); }
        }
    }

    fn print_u32(mut n: u32) {
        if n == 0 {
            console_log("0");
            return;
        }
        let mut digits = [0u8; 10];
        let mut i = 0;
        while n > 0 && i < 10 {
            digits[i] = b'0' + (n % 10) as u8;
            n /= 10;
            i += 1;
        }
        while i > 0 {
            i -= 1;
            let s = [digits[i]];
            unsafe { print(s.as_ptr(), 1); }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
