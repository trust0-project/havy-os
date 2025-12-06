// ls - List directory contents
//
// Usage:
//   ls              List current directory
//   ls <dir>        List specified directory
//   ls -l           Long format with sizes
//   ls -l <dir>     Long format for directory

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{console_log, argc, argv, get_cwd, list_files, print_int};
    use mkfs::syscalls::print;

    #[derive(Copy, Clone)]
    struct FileEntry {
        name_start: usize,
        name_len: usize,
        size: u32,
        is_dir: bool,
    }

    #[no_mangle]
    pub extern "C" fn _start() {
        let arg_count = argc();

        let mut show_long = false;
        let mut target_path = [0u8; 256];
        let mut target_len = 0usize;

        // Get current working directory as default
        if let Some(len) = get_cwd(&mut target_path) {
            target_len = len;
        } else {
            target_path[0] = b'/';
            target_len = 1;
        }

        // Parse arguments
        for i in 0..arg_count {
            let mut arg_buf = [0u8; 256];
            if let Some(arg_len) = argv(i, &mut arg_buf) {
                let arg = &arg_buf[..arg_len];

                if arg.starts_with(b"-") {
                    for &c in &arg[1..] {
                        if c == b'l' {
                            show_long = true;
                        }
                    }
                } else {
                    // Path argument
                    if arg.starts_with(b"/") {
                        let len = arg.len().min(target_path.len());
                        target_path[..len].copy_from_slice(&arg[..len]);
                        target_len = len;
                    } else {
                        // Relative path
                        let mut cwd = [0u8; 256];
                        if let Some(cwd_len) = get_cwd(&mut cwd) {
                            target_path[..cwd_len].copy_from_slice(&cwd[..cwd_len]);
                            let mut pos = cwd_len;
                            if pos < target_path.len() && target_path[pos - 1] != b'/' {
                                target_path[pos] = b'/';
                                pos += 1;
                            }
                            let remaining = target_path.len() - pos;
                            let copy_len = arg.len().min(remaining);
                            target_path[pos..pos + copy_len].copy_from_slice(&arg[..copy_len]);
                            target_len = pos + copy_len;
                        }
                    }
                }
            }
        }

        // Normalize: remove trailing slash (except for root)
        if target_len > 1 && target_path[target_len - 1] == b'/' {
            target_len -= 1;
        }

        // Get file list from kernel (format: "path:size\n" per line)
        let mut list_buf = [0u8; 32768];
        let list_len = match list_files(&mut list_buf) {
            Some(len) => len,
            None => {
                console_log("\x1b[1;31mError:\x1b[0m Filesystem not available\n");
                return;
            }
        };

        let list_data = &list_buf[..list_len];

        // Build prefix for filtering
        let prefix_buf: [u8; 258] = {
            let mut buf = [0u8; 258];
            if target_len == 1 && target_path[0] == b'/' {
                buf[0] = b'/';
            } else {
                buf[..target_len].copy_from_slice(&target_path[..target_len]);
                buf[target_len] = b'/';
            }
            buf
        };
        let prefix_len = if target_len == 1 && target_path[0] == b'/' { 1 } else { target_len + 1 };
        let prefix = &prefix_buf[..prefix_len];

        // Parse entries and filter
        let mut entries: [FileEntry; 256] = [FileEntry { name_start: 0, name_len: 0, size: 0, is_dir: false }; 256];
        let mut entry_count = 0usize;
        let mut names_buf = [0u8; 8192];
        let mut names_pos = 0usize;
        let mut seen_dirs: [usize; 64] = [0; 64];
        let mut seen_dir_lens: [usize; 64] = [0; 64];
        let mut seen_dir_count = 0usize;

        // Parse the list data (format: "path:size\n" per line)
        let mut pos = 0usize;
        while pos < list_data.len() && entry_count < 256 {
            // Find end of line
            let line_start = pos;
            while pos < list_data.len() && list_data[pos] != b'\n' {
                pos += 1;
            }
            let line_end = pos;
            pos += 1; // Skip newline

            if line_start >= line_end {
                continue;
            }

            let line = &list_data[line_start..line_end];

            // Find colon separator between path and size (search from end)
            let mut colon_pos = None;
            for (i, &c) in line.iter().enumerate().rev() {
                if c == b':' {
                    colon_pos = Some(i);
                    break;
                }
            }

            let (file_path, size_str) = match colon_pos {
                Some(cp) => (&line[..cp], &line[cp + 1..]),
                None => continue,
            };

            // Parse size
            let mut size = 0u32;
            for &c in size_str {
                if c >= b'0' && c <= b'9' {
                    size = size.saturating_mul(10).saturating_add((c - b'0') as u32);
                }
            }

            // Check if file is under target directory
            let is_under_target = if target_len == 1 && target_path[0] == b'/' {
                file_path.starts_with(b"/")
            } else {
                file_path.starts_with(prefix)
            };

            if !is_under_target {
                continue;
            }

            // Get relative path
            let relative_start = if target_len == 1 && target_path[0] == b'/' {
                1 // Skip leading /
            } else {
                prefix_len
            };

            if relative_start >= file_path.len() {
                continue;
            }

            let relative = &file_path[relative_start..];

            if relative.is_empty() {
                continue;
            }

            // Check if there's a subdirectory
            let mut slash_pos = None;
            for (i, &c) in relative.iter().enumerate() {
                if c == b'/' {
                    slash_pos = Some(i);
                    break;
                }
            }

            if let Some(sp) = slash_pos {
                // This is a subdirectory
                let dir_name = &relative[..sp];

                // Check if we've seen this directory
                let mut already_seen = false;
                for d in 0..seen_dir_count {
                    let existing = &names_buf[seen_dirs[d]..seen_dirs[d] + seen_dir_lens[d]];
                    if existing == dir_name {
                        already_seen = true;
                        break;
                    }
                }

                if !already_seen && seen_dir_count < 64 && entry_count < 256 {
                    let copy_len = dir_name.len().min(names_buf.len() - names_pos);
                    if copy_len > 0 {
                        names_buf[names_pos..names_pos + copy_len].copy_from_slice(&dir_name[..copy_len]);
                        seen_dirs[seen_dir_count] = names_pos;
                        seen_dir_lens[seen_dir_count] = copy_len;
                        seen_dir_count += 1;

                        entries[entry_count] = FileEntry {
                            name_start: names_pos,
                            name_len: copy_len,
                            size: 0,
                            is_dir: true,
                        };
                        entry_count += 1;
                        names_pos += copy_len;
                    }
                }
            } else {
                // Direct file
                let copy_len = relative.len().min(names_buf.len() - names_pos);
                if copy_len > 0 && entry_count < 256 {
                    names_buf[names_pos..names_pos + copy_len].copy_from_slice(&relative[..copy_len]);
                    entries[entry_count] = FileEntry {
                        name_start: names_pos,
                        name_len: copy_len,
                        size,
                        is_dir: false,
                    };
                    entry_count += 1;
                    names_pos += copy_len;
                }
            }
        }

        if entry_count == 0 {
            console_log("\x1b[0;90m(empty)\x1b[0m\n");
            return;
        }

        // Simple bubble sort (dirs first, then alphabetical)
        for i in 0..entry_count {
            for j in i + 1..entry_count {
                let swap = if entries[i].is_dir != entries[j].is_dir {
                    !entries[i].is_dir && entries[j].is_dir
                } else {
                    let name_i = &names_buf[entries[i].name_start..entries[i].name_start + entries[i].name_len];
                    let name_j = &names_buf[entries[j].name_start..entries[j].name_start + entries[j].name_len];
                    name_i > name_j
                };
                if swap {
                    let tmp = entries[i];
                    entries[i] = entries[j];
                    entries[j] = tmp;
                }
            }
        }

        let is_usr_bin = (target_len == 8 && &target_path[..8] == b"/usr/bin") ||
                         (target_len > 8 && target_path[..9] == *b"/usr/bin/");

        if show_long {
            // Long format
            for i in 0..entry_count {
                let e = &entries[i];
                let name = &names_buf[e.name_start..e.name_start + e.name_len];

                if e.is_dir {
                    console_log(" \x1b[0;90m<dir>\x1b[0m  \x1b[1;34m");
                    unsafe { print(name.as_ptr(), name.len()) };
                    console_log("/\x1b[0m\n");
                } else {
                    // Right-align size in 6 chars
                    let s = e.size;
                    if s < 10 { console_log("     "); }
                    else if s < 100 { console_log("    "); }
                    else if s < 1000 { console_log("   "); }
                    else if s < 10000 { console_log("  "); }
                    else if s < 100000 { console_log(" "); }
                    print_int(e.size as i64);
                    console_log("  ");
                    if is_usr_bin {
                        console_log("\x1b[1;32m");
                    }
                    unsafe { print(name.as_ptr(), name.len()) };
                    if is_usr_bin {
                        console_log("\x1b[0m");
                    }
                    console_log("\n");
                }
            }

            // Summary
            let mut dir_count = 0usize;
            for i in 0..entry_count {
                if entries[i].is_dir {
                    dir_count += 1;
                }
            }
            let file_count = entry_count - dir_count;
            console_log("\n\x1b[0;90m");
            print_int(dir_count as i64);
            console_log(" dir(s), ");
            print_int(file_count as i64);
            console_log(" file(s)\x1b[0m\n");
        } else {
            // Compact columnar format
            let mut max_len = 4usize;
            for i in 0..entry_count {
                let len = entries[i].name_len + if entries[i].is_dir { 1 } else { 0 };
                if len > max_len {
                    max_len = len;
                }
            }

            let col_width = (max_len + 2).max(4);
            let num_cols = (60 / col_width).max(1);
            let mut col = 0;

            for i in 0..entry_count {
                let e = &entries[i];
                let name = &names_buf[e.name_start..e.name_start + e.name_len];
                let display_len = e.name_len + if e.is_dir { 1 } else { 0 };

                if e.is_dir {
                    console_log("\x1b[1;34m");
                    unsafe { print(name.as_ptr(), name.len()) };
                    console_log("/\x1b[0m");
                } else if is_usr_bin {
                    console_log("\x1b[1;32m");
                    unsafe { print(name.as_ptr(), name.len()) };
                    console_log("\x1b[0m");
                } else {
                    unsafe { print(name.as_ptr(), name.len()) };
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
            if col > 0 {
                console_log("\n");
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
