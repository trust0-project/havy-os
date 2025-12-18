use alloc::{format, string::String, vec, vec::Vec};
use wasmi::{Caller, Config, Engine, Func, Linker, Module, Store};
use core::ptr;

use crate::{SHELL_CMD_STATE, ShellCmdState, clint::get_time_ms, commands::http, constants::TEST_FINISHER, cpu, lock::{self, utils::BLK_DEV}, services::klogd::{KLOG, klog_info}, uart};

/// State to pass to host functions - includes command arguments
struct WasmContext {
    args: Vec<String>, 
}

/// Get shell command info for ps_list (returns: name, pid, cpu (hart), uptime_ms, is_running)
pub fn get_shell_cmd_info() -> Option<(String, u32, i64, u64, bool)> {
    let state: lock::SpinlockGuard<'_, ShellCmdState> = SHELL_CMD_STATE.lock();
    let current_time = get_time_ms() as u64;
    
    if state.is_running {
        // Currently running command - runs on hart 0 (shell hart)
        let uptime = current_time.saturating_sub(state.start_time);
        Some((
            String::from(state.get_name()),
            state.pid,
            0i64,  // Shell commands run on hart 0
            uptime,
            true,
        ))
    } else if state.name_len > 0 {
        // Last command finished - show shell as not on any hart
        let uptime = current_time.saturating_sub(state.session_start);
        Some((
            String::from("shell"),
            0,
            -1i64, // Not running on any hart
            uptime,
            false,
        ))
    } else {
        None
    }
}



/// Execute a WASM binary with the given arguments
pub fn execute(wasm_bytes: &[u8], args: &[&str]) -> Result<String, String> {
    // Configure engine with relaxed limits for scripts
    let mut config = Config::default();
    config.consume_fuel(false);  // Don't limit execution
    let engine = Engine::new(&config);
    
    let ctx = WasmContext {
        args: args.iter().map(|s| String::from(*s)).collect(),
    };
    let mut store = Store::new(&engine, ctx);
    let mut linker = Linker::new(&engine);

    // Syscall: print(ptr, len)
    linker
        .define(
            "env",
            "print",
            Func::wrap(
                &mut store,
                |caller: Caller<'_, WasmContext>, ptr: i32, len: i32| {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut buffer = vec![0u8; len as usize];
                        if mem.read(&caller, ptr as usize, &mut buffer).is_ok() {
                            uart::write_str(&String::from_utf8_lossy(&buffer));
                        }
                    }
                },
            ),
        )
        .map_err(|e| format!("define print: {:?}", e))?;

    // Syscall: time() -> i64
    linker
        .define(
            "env",
            "time",
            Func::wrap(&mut store, |_caller: Caller<'_, WasmContext>| -> i64 {
                crate::get_time_ms()
            }),
        )
        .map_err(|e| format!("define time: {:?}", e))?;

    // Syscall: console_available() -> i32
    // Check if console input is available (non-blocking check)
    // Returns 1 if input is available, 0 otherwise
    linker
        .define(
            "env",
            "console_available",
            Func::wrap(&mut store, |_caller: Caller<'_, WasmContext>| -> i32 {
                // Check if UART has pending input
                if crate::uart::has_pending_input() {
                    1
                } else {
                    0
                }
            }),
        )
        .map_err(|e| format!("define console_available: {:?}", e))?;

    // Syscall: console_read(buf_ptr, buf_len) -> i32
    // Read from console (non-blocking). Returns bytes read, 0 if no data.
    linker
        .define(
            "env",
            "console_read",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>, buf_ptr: i32, buf_len: i32| -> i32 {
                    if buf_len <= 0 {
                        return 0;
                    }
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        // Try to read one character (non-blocking)
                        if let Some(ch) = crate::uart::read_char_nonblocking() {
                            let buf = [ch];
                            if mem.write(&mut caller, buf_ptr as usize, &buf).is_ok() {
                                return 1;
                            }
                        }
                    }
                    0
                },
            ),
        )
        .map_err(|e| format!("define console_read: {:?}", e))?;

    // Syscall: arg_count() -> i32
    linker
        .define(
            "env",
            "arg_count",
            Func::wrap(&mut store, |caller: Caller<'_, WasmContext>| -> i32 {
                caller.data().args.len() as i32
            }),
        )
        .map_err(|e| format!("define arg_count: {:?}", e))?;

    // Syscall: arg_get(index, buf_ptr, buf_len) -> i32
    linker
        .define(
            "env",
            "arg_get",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>,
                 index: i32,
                 buf_ptr: i32,
                 buf_len: i32|
                 -> i32 {
                    // Clone the arg to avoid borrow issues
                    let arg_opt = {
                        let args = &caller.data().args;
                        if index < 0 || (index as usize) >= args.len() {
                            None
                        } else {
                            Some(args[index as usize].clone())
                        }
                    };

                    if let Some(arg) = arg_opt {
                        let bytes = arg.as_bytes();
                        if bytes.len() > buf_len as usize {
                            return -1;
                        }
                        if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory())
                        {
                            if mem.write(&mut caller, buf_ptr as usize, bytes).is_ok() {
                                return bytes.len() as i32;
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define arg_get: {:?}", e))?;

    // Syscall: cwd_get(buf_ptr, buf_len) -> i32
    linker
        .define(
            "env",
            "cwd_get",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>, buf_ptr: i32, buf_len: i32| -> i32 {
                    let cwd = crate::utils::cwd_get();
                    let bytes = cwd.as_bytes();
                    if bytes.len() > buf_len as usize {
                        return -1;
                    }
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        if mem.write(&mut caller, buf_ptr as usize, bytes).is_ok() {
                            return bytes.len() as i32;
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define cwd_get: {:?}", e))?;

    // Syscall: fs_exists(path_ptr, path_len) -> i32
    linker
        .define(
            "env",
            "fs_exists",
            Func::wrap(
                &mut store,
                |caller: Caller<'_, WasmContext>, path_ptr: i32, path_len: i32| -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut path_buf = vec![0u8; path_len as usize];
                        if mem.read(&caller, path_ptr as usize, &mut path_buf).is_ok() {
                            if let Ok(path) = core::str::from_utf8(&path_buf) {
                                let fs_guard = crate::FS_STATE.read();
                                let mut blk_guard = BLK_DEV.write();
                                if let (Some(fs), Some(dev)) =
                                    (fs_guard.as_ref(), blk_guard.as_mut())
                                {
                                    return if fs.read_file(dev, path).is_some() {
                                        1
                                    } else {
                                        0
                                    };
                                }
                            }
                        }
                    }
                    0
                },
            ),
        )
        .map_err(|e| format!("define fs_exists: {:?}", e))?;

    // Syscall: fs_read(path_ptr, path_len, buf_ptr, buf_len) -> i32
    linker
        .define(
            "env",
            "fs_read",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>,
                 path_ptr: i32,
                 path_len: i32,
                 buf_ptr: i32,
                 buf_len: i32|
                 -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut path_buf = vec![0u8; path_len as usize];
                        if mem.read(&caller, path_ptr as usize, &mut path_buf).is_ok() {
                            if let Ok(path) = core::str::from_utf8(&path_buf) {
                                let fs_guard = crate::FS_STATE.read();
                                let mut blk_guard = BLK_DEV.write();
                                if let (Some(fs), Some(dev)) =
                                    (fs_guard.as_ref(), blk_guard.as_mut())
                                {
                                    if let Some(data) = fs.read_file(dev, path) {
                                        let to_copy = data.len().min(buf_len as usize);
                                        if mem
                                            .write(&mut caller, buf_ptr as usize, &data[..to_copy])
                                            .is_ok()
                                        {
                                            return to_copy as i32;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define fs_read: {:?}", e))?;

    // Syscall: fs_write(path_ptr, path_len, data_ptr, data_len) -> i32
    linker
        .define(
            "env",
            "fs_write",
            Func::wrap(
                &mut store,
                |caller: Caller<'_, WasmContext>,
                 path_ptr: i32,
                 path_len: i32,
                 data_ptr: i32,
                 data_len: i32|
                 -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut path_buf = vec![0u8; path_len as usize];
                        let mut data_buf = vec![0u8; data_len as usize];
                        if mem.read(&caller, path_ptr as usize, &mut path_buf).is_ok()
                            && mem.read(&caller, data_ptr as usize, &mut data_buf).is_ok()
                        {
                            if let Ok(path) = core::str::from_utf8(&path_buf) {
                                let mut fs_guard = crate::FS_STATE.write();
                                let mut blk_guard = BLK_DEV.write();
                                if let (Some(fs), Some(dev)) =
                                    (fs_guard.as_mut(), blk_guard.as_mut())
                                {
                                    if fs.write_file(dev, path, &data_buf).is_ok() {
                                        return data_len;
                                    }
                                }
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define fs_write: {:?}", e))?;

    // Syscall: fs_list(buf_ptr, buf_len) -> i32
    linker
        .define(
            "env",
            "fs_list",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>, buf_ptr: i32, buf_len: i32| -> i32 {
                    let mut fs_guard = crate::FS_STATE.write();
                    let mut blk_guard = BLK_DEV.write();
                    if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
                        let files = fs.list_dir(dev, "/");
                        // Format as simple newline-separated list: "name:size\n"
                        let mut output = String::new();
                        for file in files {
                            output.push_str(&file.name);
                            output.push(':');
                            output.push_str(&format!("{}", file.size));
                            output.push('\n');
                        }
                        let bytes = output.as_bytes();
                        if bytes.len() > buf_len as usize {
                            return -1;
                        }
                        if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory())
                        {
                            if mem.write(&mut caller, buf_ptr as usize, bytes).is_ok() {
                                return bytes.len() as i32;
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define fs_list: {:?}", e))?;

    // Syscall: klog_get(count, buf_ptr, buf_len) -> i32
    linker
        .define(
            "env",
            "klog_get",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>,
                 count: i32,
                 buf_ptr: i32,
                 buf_len: i32|
                 -> i32 {
                    let count = (count as usize).max(1).min(100);
                    let entries = KLOG.recent(count);
                    let mut output = String::new();
                    for entry in entries.iter().rev() {
                        output.push_str(&entry.format_colored());
                        output.push('\n');
                    }
                    let bytes = output.as_bytes();
                    if bytes.len() > buf_len as usize {
                        return -1;
                    }
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        if mem.write(&mut caller, buf_ptr as usize, bytes).is_ok() {
                            return bytes.len() as i32;
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define klog_get: {:?}", e))?;

    // Syscall: net_available() -> i32
    // Check network state
    linker
        .define(
            "env",
            "net_available",
            Func::wrap(&mut store, |_caller: Caller<'_, WasmContext>| -> i32 {
                let net_guard = crate::NET_STATE.lock();
                if net_guard.is_some() {
                    return 1;
                }
                0
            }),
        )
        .map_err(|e| format!("define net_available: {:?}", e))?;

    // Syscall: http_get(url_ptr, url_len, resp_ptr, resp_len) -> i32
    linker
        .define(
            "env",
            "http_get",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>,
                 url_ptr: i32,
                 url_len: i32,
                 resp_ptr: i32,
                 resp_len: i32|
                 -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut url_buf = vec![0u8; url_len as usize];
                        if mem.read(&caller, url_ptr as usize, &mut url_buf).is_ok() {
                            if let Ok(url) = core::str::from_utf8(&url_buf) {
                                let mut net_guard = crate::NET_STATE.lock();
                                if let Some(ref mut net) = *net_guard {
                                    match http::get_follow_redirects(
                                        net,
                                        url,
                                        30000,
                                        crate::get_time_ms,
                                    ) {
                                        Ok(response) => {
                                            // Return just the body (already Vec<u8>)
                                            let bytes = &response.body;
                                            let to_copy = bytes.len().min(resp_len as usize);
                                            if mem
                                                .write(
                                                    &mut caller,
                                                    resp_ptr as usize,
                                                    &bytes[..to_copy],
                                                )
                                                .is_ok()
                                            {
                                                return to_copy as i32;
                                            }
                                        }
                                        Err(_) => return -1,
                                    }
                                }
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define http_get: {:?}", e))?;

    // Syscall: cwd_set(path_ptr, path_len) -> i32
    // Sets the current working directory. Returns 0 on success, -1 on error.
    linker
        .define(
            "env",
            "cwd_set",
            Func::wrap(
                &mut store,
                |caller: Caller<'_, WasmContext>, path_ptr: i32, path_len: i32| -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut path_buf = vec![0u8; path_len as usize];
                        if mem.read(&caller, path_ptr as usize, &mut path_buf).is_ok() {
                            if let Ok(path) = core::str::from_utf8(&path_buf) {
                                // Resolve and validate path before setting
                                let resolved = crate::resolve_path(path);
                                if crate::utils::path_exists(&resolved) {
                                    crate::utils::cwd_set(&resolved);
                                    return 0;
                                }
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define cwd_set: {:?}", e))?;

    // Syscall: shutdown() -> !
    // Powers off the system. Does not return.
    linker
        .define(
            "env",
            "shutdown",
            Func::wrap(&mut store, |_caller: Caller<'_, WasmContext>| -> () {
                uart::write_line("");
                uart::write_line(
                    "\x1b[1;31m+===================================================================+\x1b[0m",
                );
                uart::write_line(
                    "\x1b[1;31m|\x1b[0m                                                                   \x1b[1;31m|\x1b[0m",
                );
                uart::write_line(
                    "\x1b[1;31m|\x1b[0m                    \x1b[1;97mSystem Shutdown Initiated\x1b[0m                       \x1b[1;31m|\x1b[0m",
                );
                uart::write_line(
                    "\x1b[1;31m|\x1b[0m                                                                   \x1b[1;31m|\x1b[0m",
                );
                uart::write_line(
                    "\x1b[1;31m+===================================================================+\x1b[0m",
                );
                uart::write_line("");
                uart::write_line("    \x1b[0;90m[1/3]\x1b[0m Syncing filesystems...");
                uart::write_line("    \x1b[0;90m[2/3]\x1b[0m Stopping network services...");
                uart::write_line("    \x1b[0;90m[3/3]\x1b[0m Powering off CPU...");
                uart::write_line("");
                uart::write_line("    \x1b[1;32m[OK] Goodbye!\x1b[0m");
                uart::write_line("");
                unsafe {
                    ptr::write_volatile(TEST_FINISHER as *mut u32, 0x5555);
                }
                #[allow(clippy::empty_loop)]
                loop {}
            }),
        )
        .map_err(|e| format!("define shutdown: {:?}", e))?;

    // Syscall: dns_resolve(host_ptr, host_len, ip_buf_ptr, ip_buf_len) -> i32
    // Resolves hostname to IP address. Returns bytes written (4 for IPv4) or -1 on error.
    linker
        .define(
            "env",
            "dns_resolve",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>,
                 host_ptr: i32,
                 host_len: i32,
                 ip_buf_ptr: i32,
                 ip_buf_len: i32|
                 -> i32 {
                    if ip_buf_len < 4 {
                        return -1;
                    }
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut host_buf = vec![0u8; host_len as usize];
                        if mem.read(&caller, host_ptr as usize, &mut host_buf).is_ok() {
                            let dns_server = smoltcp::wire::Ipv4Address::new(8, 8, 8, 8);
                            
                            // Use unified NET_STATE (D1)
                            let mut net_guard = crate::NET_STATE.lock();
                            if let Some(ref mut net) = *net_guard {
                                if let Some(ip) = crate::dns::resolve(
                                    net,
                                    &host_buf,
                                    dns_server,
                                    5000,
                                    crate::get_time_ms,
                                ) {
                                    if mem.write(&mut caller, ip_buf_ptr as usize, &ip.octets()).is_ok() {
                                        return 4;
                                    }
                                }
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define dns_resolve: {:?}", e))?;

    // Syscall: fs_stat(path_ptr, path_len, out_ptr) -> i32
    // Gets file stats. Writes to out_ptr: u32 size, u8 exists (1/0), u8 is_dir (1/0)
    // Returns 0 on success, -1 on error (path issues, not file-not-found)
    linker
        .define(
            "env",
            "fs_stat",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>,
                 path_ptr: i32,
                 path_len: i32,
                 out_ptr: i32|
                 -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut path_buf = vec![0u8; path_len as usize];
                        if mem.read(&caller, path_ptr as usize, &mut path_buf).is_ok() {
                            if let Ok(path) = core::str::from_utf8(&path_buf) {
                                let mut fs_guard = crate::FS_STATE.write();
                                let mut blk_guard = BLK_DEV.write();
                                if let (Some(fs), Some(dev)) =
                                    (fs_guard.as_mut(), blk_guard.as_mut())
                                {
                                    // Check if file exists and get its size
                                    let file_data = fs.read_file(dev, path);
                                    let (size, exists, is_dir): (u32, u8, u8) = match file_data {
                                        Some(data) => (data.len() as u32, 1, 0),
                                        None => {
                                            // Check if it's a directory by looking for files with this prefix
                                            let files = fs.list_dir(dev, "/");
                                            let prefix = if path.ends_with('/') {
                                                String::from(path)
                                            } else {
                                                format!("{}/", path)
                                            };
                                            let is_directory = files.iter().any(|f| f.name.starts_with(&prefix));
                                            if is_directory {
                                                (0, 1, 1)
                                            } else {
                                                (0, 0, 0)
                                            }
                                        }
                                    };

                                    // Write output: 4 bytes size + 1 byte exists + 1 byte is_dir
                                    let mut out = [0u8; 6];
                                    out[0..4].copy_from_slice(&size.to_le_bytes());
                                    out[4] = exists;
                                    out[5] = is_dir;
                                    if mem.write(&mut caller, out_ptr as usize, &out).is_ok() {
                                        return 0;
                                    }
                                }
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define fs_stat: {:?}", e))?;

    // Syscall: fs_list_dir(path_ptr, path_len, buf_ptr, buf_len) -> i32
    // Lists files in a specific directory (not just root).
    // Returns bytes written or -1 on error.
    linker
        .define(
            "env",
            "fs_list_dir",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>,
                 path_ptr: i32,
                 path_len: i32,
                 buf_ptr: i32,
                 buf_len: i32|
                 -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut path_buf = vec![0u8; path_len as usize];
                        if mem.read(&caller, path_ptr as usize, &mut path_buf).is_ok() {
                            if let Ok(dir_path) = core::str::from_utf8(&path_buf) {
                                let mut fs_guard = crate::FS_STATE.write();
                                let mut blk_guard = BLK_DEV.write();
                                if let (Some(fs), Some(dev)) =
                                    (fs_guard.as_mut(), blk_guard.as_mut())
                                {
                                    let files = fs.list_dir(dev, dir_path);

                                    // Build prefix for filtering
                                    let prefix = if dir_path == "/" {
                                        String::from("/")
                                    } else {
                                        let p = dir_path.trim_end_matches('/');
                                        format!("{}/", p)
                                    };

                                    // Format filtered entries as "name:size\n"
                                    let mut output = String::new();
                                    for file in files {
                                        // Only include files that match the directory
                                        if file.name.starts_with(&prefix) || (dir_path == "/" && file.name.starts_with('/')) {
                                            output.push_str(&file.name);
                                            output.push(':');
                                            output.push_str(&format!("{}", file.size));
                                            output.push('\n');
                                        }
                                    }

                                    let bytes = output.as_bytes();
                                    if bytes.len() > buf_len as usize {
                                        return -1;
                                    }
                                    if mem.write(&mut caller, buf_ptr as usize, bytes).is_ok() {
                                        return bytes.len() as i32;
                                    }
                                }
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define fs_list_dir: {:?}", e))?;

    // Syscall: fs_mkdir(path_ptr, path_len) -> i32
    // Creates a directory marker. Returns 0 on success, -1 on error.
    linker
        .define(
            "env",
            "fs_mkdir",
            Func::wrap(
                &mut store,
                |caller: Caller<'_, WasmContext>, path_ptr: i32, path_len: i32| -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut path_buf = vec![0u8; path_len as usize];
                        if mem.read(&caller, path_ptr as usize, &mut path_buf).is_ok() {
                            if let Ok(path) = core::str::from_utf8(&path_buf) {
                                let mut fs_guard = crate::FS_STATE.write();
                                let mut blk_guard = BLK_DEV.write();
                                if let (Some(fs), Some(dev)) =
                                    (fs_guard.as_mut(), blk_guard.as_mut())
                                {
                                    // Create an empty .keep file as directory marker
                                    let keep_path = format!("{}/.keep", path.trim_end_matches('/'));
                                    if fs.write_file(dev, &keep_path, &[]).is_ok() {
                                        return 0;
                                    }
                                }
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define fs_mkdir: {:?}", e))?;

    // Syscall: env_get(key_ptr, key_len, val_ptr, val_len) -> i32
    // Gets an environment variable. Returns length or -1 if not found.
    linker
        .define(
            "env",
            "env_get",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>,
                 key_ptr: i32,
                 key_len: i32,
                 val_ptr: i32,
                 val_len: i32|
                 -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut key_buf = vec![0u8; key_len as usize];
                        if mem.read(&caller, key_ptr as usize, &mut key_buf).is_ok() {
                            if let Ok(key) = core::str::from_utf8(&key_buf) {
                                // Built-in environment variables
                                let value = match key {
                                    "HOME" => Some("/home"),
                                    "PATH" => Some("/usr/bin"),
                                    "PWD" => {
                                        let cwd = crate::utils::cwd_get();
                                        let bytes = cwd.as_bytes();
                                        if bytes.len() <= val_len as usize {
                                            if mem.write(&mut caller, val_ptr as usize, bytes).is_ok() {
                                                return bytes.len() as i32;
                                            }
                                        }
                                        return -1;
                                    }
                                    "USER" => Some("root"),
                                    "SHELL" => Some("/usr/bin/sh"),
                                    "TERM" => Some("xterm-256color"),
                                    _ => None,
                                };

                                if let Some(val) = value {
                                    let bytes = val.as_bytes();
                                    if bytes.len() <= val_len as usize {
                                        if mem.write(&mut caller, val_ptr as usize, bytes).is_ok() {
                                            return bytes.len() as i32;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define env_get: {:?}", e))?;

    // Syscall: random(buf_ptr, buf_len) -> i32
    // Fills buffer with random bytes. Returns bytes written or -1 on error.
    linker
        .define(
            "env",
            "random",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>, buf_ptr: i32, buf_len: i32| -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        // Simple PRNG based on time
                        let mut seed = crate::get_time_ms() as u64;
                        let mut random_bytes = vec![0u8; buf_len as usize];
                        for byte in random_bytes.iter_mut() {
                            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                            *byte = (seed >> 16) as u8;
                        }
                        if mem.write(&mut caller, buf_ptr as usize, &random_bytes).is_ok() {
                            return buf_len;
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define random: {:?}", e))?;

    // Syscall: sleep_ms(ms) -> ()
    // Sleeps for the specified number of milliseconds.
    linker
        .define(
            "env",
            "sleep_ms",
            Func::wrap(&mut store, |_caller: Caller<'_, WasmContext>, ms: i32| {
                let start = crate::get_time_ms();
                let target = start + ms as i64;
                while crate::get_time_ms() < target {
                    core::hint::spin_loop();
                }
            }),
        )
        .map_err(|e| format!("define sleep_ms: {:?}", e))?;

    // Syscall: disk_stats(out_ptr) -> i32
    // Gets disk usage statistics. Writes to out_ptr: u64 used_bytes, u64 total_bytes
    // Returns 0 on success, -1 on error.
    linker
        .define(
            "env",
            "disk_stats",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>, out_ptr: i32| -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let fs_guard = crate::FS_STATE.read();
                        if let Some(ref fs) = *fs_guard {
                            let (used, total) = fs.disk_usage_bytes();
                            let mut out = [0u8; 16];
                            out[0..8].copy_from_slice(&used.to_le_bytes());
                            out[8..16].copy_from_slice(&total.to_le_bytes());
                            if mem.write(&mut caller, out_ptr as usize, &out).is_ok() {
                                return 0;
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define disk_stats: {:?}", e))?;

    // Syscall: heap_stats(out_ptr) -> i32
    // Gets heap usage statistics. Writes to out_ptr: u64 used_bytes, u64 total_bytes
    // Returns 0 on success, -1 on error.
    linker
        .define(
            "env",
            "heap_stats",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>, out_ptr: i32| -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let (used, _free) = crate::allocator::heap_stats();
                        let total = crate::allocator::heap_size();
                        let mut out = [0u8; 16];
                        out[0..8].copy_from_slice(&(used as u64).to_le_bytes());
                        out[8..16].copy_from_slice(&(total as u64).to_le_bytes());
                        if mem.write(&mut caller, out_ptr as usize, &out).is_ok() {
                            return 0;
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define heap_stats: {:?}", e))?;

    // ═══════════════════════════════════════════════════════════════════════════════
    // WASM WORKER SYSCALLS - For multi-hart WASM execution
    // ═══════════════════════════════════════════════════════════════════════════════

    // Syscall: wasm_worker_count() -> i32
    // Returns number of WASM workers (harts - 1, since hart 0 is primary)
    linker
        .define(
            "env",
            "wasm_worker_count",
            Func::wrap(&mut store, |_caller: Caller<'_, WasmContext>| -> i32 {
                let workers = crate::wasm_service::list_workers();
                workers.len() as i32
            }),
        )
        .map_err(|e| format!("define wasm_worker_count: {:?}", e))?;

    // Syscall: wasm_worker_stats(worker_idx, out_ptr) -> i32
    // Gets worker stats for worker at index.
    // Writes to out_ptr: u32 hart_id, u64 jobs_completed, u64 jobs_failed, 
    //                    u64 total_exec_ms, u32 current_job, u32 queue_depth
    // Returns 0 on success, -1 if invalid worker index or error
    linker
        .define(
            "env",
            "wasm_worker_stats",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>, worker_idx: i32, out_ptr: i32| -> i32 {
                    let workers = crate::wasm_service::list_workers();
                    if worker_idx < 0 || worker_idx as usize >= workers.len() {
                        return -1;
                    }
                    
                    let (hart_id, completed, failed, exec_ms, current_job, queue_depth) = 
                        workers[worker_idx as usize];
                    
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        // Pack: hart_id(4) + completed(8) + failed(8) + exec_ms(8) + current(4) + queue(4) = 36 bytes
                        let mut out = [0u8; 36];
                        out[0..4].copy_from_slice(&(hart_id as u32).to_le_bytes());
                        out[4..12].copy_from_slice(&completed.to_le_bytes());
                        out[12..20].copy_from_slice(&failed.to_le_bytes());
                        out[20..28].copy_from_slice(&exec_ms.to_le_bytes());
                        out[28..32].copy_from_slice(&current_job.to_le_bytes());
                        out[32..36].copy_from_slice(&(queue_depth as u32).to_le_bytes());
                        
                        if mem.write(&mut caller, out_ptr as usize, &out).is_ok() {
                            return 0;
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define wasm_worker_stats: {:?}", e))?;

    // Syscall: wasm_submit_job(wasm_ptr, wasm_len, args_ptr, args_len, target_hart) -> i32
    // Submits a WASM job to be executed on a worker.
    // args_ptr points to newline-separated arguments.
    // target_hart: 0 = auto-select, 1+ = specific hart
    // Returns job_id on success (>0), -1 on error
    linker
        .define(
            "env",
            "wasm_submit_job",
            Func::wrap(
                &mut store,
                |caller: Caller<'_, WasmContext>,
                 wasm_ptr: i32,
                 wasm_len: i32,
                 args_ptr: i32,
                 args_len: i32,
                 target_hart: i32|
                 -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        // Read WASM bytes
                        let mut wasm_bytes = vec![0u8; wasm_len as usize];
                        if mem.read(&caller, wasm_ptr as usize, &mut wasm_bytes).is_err() {
                            return -1;
                        }
                        
                        // Read args (newline-separated)
                        let args_vec = if args_len > 0 {
                            let mut args_buf = vec![0u8; args_len as usize];
                            if mem.read(&caller, args_ptr as usize, &mut args_buf).is_err() {
                                return -1;
                            }
                            if let Ok(args_str) = core::str::from_utf8(&args_buf) {
                                args_str.lines().map(String::from).collect()
                            } else {
                                vec![]
                            }
                        } else {
                            vec![]
                        };
                        
                        let target = if target_hart <= 0 { None } else { Some(target_hart as usize) };
                        
                        match crate::wasm_service::submit_job(wasm_bytes, args_vec, target) {
                            Ok(job_id) => job_id as i32,
                            Err(_) => -1,
                        }
                    } else {
                        -1
                    }
                },
            ),
        )
        .map_err(|e| format!("define wasm_submit_job: {:?}", e))?;

    // Syscall: wasm_job_status(job_id) -> i32
    // Returns job status: 0=pending, 1=running, 2=completed, 3=failed, -1=not found
    linker
        .define(
            "env",
            "wasm_job_status",
            Func::wrap(
                &mut store,
                |_caller: Caller<'_, WasmContext>, job_id: i32| -> i32 {
                    if job_id <= 0 {
                        return -1;
                    }
                    match crate::wasm_service::job_status(job_id as u32) {
                        Some(status) => status as i32,
                        None => -1,
                    }
                },
            ),
        )
        .map_err(|e| format!("define wasm_job_status: {:?}", e))?;

    // Syscall: hart_count() -> i32
    // Returns total number of harts (including primary)
    linker
        .define(
            "env",
            "hart_count",
            Func::wrap(&mut store, |_caller: Caller<'_, WasmContext>| -> i32 {
                crate::HARTS_ONLINE.load(core::sync::atomic::Ordering::Relaxed) as i32
            }),
        )
        .map_err(|e| format!("define hart_count: {:?}", e))?;

    // Syscall: cpu_info(cpu_id, out_ptr) -> i32
    // Gets information about a specific CPU.
    // Writes to out_ptr: u32 state, u32 running_pid, u8 utilization, u64 context_switches = 17 bytes
    // Returns 0 on success, -1 if CPU not online or error.
    linker
        .define(
            "env",
            "cpu_info",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>, cpu_id: i32, out_ptr: i32| -> i32 {
                    if cpu_id < 0 {
                        return -1;
                    }
                    
                    if let Some(cpu) = crate::cpu::CPU_TABLE.get(cpu_id as usize) {
                        if !cpu.is_online() {
                            return -1;
                        }
                        
                        let info = cpu.info();
                        
                        if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                            // Pack: state(4) + running_pid(4) + utilization(1) + context_switches(8) = 17 bytes
                            let mut out = [0u8; 17];
                            out[0..4].copy_from_slice(&(info.state as u32).to_le_bytes());
                            out[4..8].copy_from_slice(&info.running_process.unwrap_or(0).to_le_bytes());
                            out[8] = info.utilization;
                            out[9..17].copy_from_slice(&info.context_switches.to_le_bytes());
                            
                            if mem.write(&mut caller, out_ptr as usize, &out).is_ok() {
                                return 0;
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define cpu_info: {:?}", e))?;

    // Syscall: ps_list(buf_ptr, buf_len) -> i32
    // Gets list of all running processes. Returns bytes written or -1 on error.
    // Format: "pid:name:state:priority:cpu:uptime\n" for each process (cpu = hart number, -1 if not assigned)
    linker
        .define(
            "env",
            "ps_list",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>, buf_ptr: i32, buf_len: i32| -> i32 {
                    let mut output = String::new();
                    let mut seen_pids: alloc::collections::BTreeSet<u32> = alloc::collections::BTreeSet::new();
                    
                    // Include the current shell command (which is the one calling ps_list)
                    if let Some((name, pid, cpu, uptime, is_running)) = get_shell_cmd_info() {
                        let state = if is_running { "R+" } else { "S" };
                        seen_pids.insert(pid);
                        output.push_str(&format!(
                            "{}:{}:{}:{}:{}:{}\n",
                            pid, name, state, "normal", cpu, uptime
                        ));
                    }
                    
                    // Include processes from new scheduler
                    let processes = crate::sched::list_processes();
                    for proc in &processes {
                        if seen_pids.contains(&proc.pid) {
                            continue;
                        }
                        seen_pids.insert(proc.pid);
                        let cpu = proc.cpu.map(|c| c as i64).unwrap_or(-1);
                        output.push_str(&format!(
                            "{}:{}:{}:{}:{}:{}\n",
                            proc.pid,
                            proc.name,
                            proc.state.code(),
                            match proc.priority {
                                cpu::process::Priority::Idle => "idle",
                                cpu::process::Priority::Low => "low",
                                cpu::process::Priority::Normal => "normal",
                                cpu::process::Priority::High => "high",
                                cpu::process::Priority::Realtime => "rt",
                            },
                            cpu,
                            proc.uptime_ms
                        ));
                    }
                    
                    // Include kernel services (klogd, sysmond)
                    // Only show services that aren't wasmworkerd (those are CPUs, not processes)
                    let services = crate::init::list_services();
                    for svc in services {
                        if svc.status != crate::init::ServiceStatus::Running {
                            continue;
                        }
                        if seen_pids.contains(&svc.pid) {
                            continue;
                        }
                        // Skip wasmworkerd services - they are internal and not user-visible processes
                        if svc.name.starts_with("wasmworkerd") {
                            continue;
                        }
                        let hart = svc.hart.unwrap_or(0);
                        let uptime = crate::get_time_ms() as u64 - svc.started_at;
                        output.push_str(&format!(
                            "{}:{}:{}:{}:{}:{}\n",
                            svc.pid, svc.name, "R+", "normal", hart, uptime
                        ));
                    }
                    
                    let bytes = output.as_bytes();
                    if bytes.len() > buf_len as usize {
                        return -1;
                    }
                    
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        if mem.write(&mut caller, buf_ptr as usize, bytes).is_ok() {
                            return bytes.len() as i32;
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define ps_list: {:?}", e))?;

    // Syscall: kill(pid) -> i32
    // Kills a process by PID. Returns 0 on success, -1 if not found, -2 if cannot kill.
    linker
        .define(
            "env",
            "kill",
            Func::wrap(
                &mut store,
                |_caller: Caller<'_, WasmContext>, pid: i32| -> i32 {
                    if pid <= 0 {
                        return -1; // Invalid PID
                    }
                    if pid == 1 {
                        return -2; // Cannot kill init
                    }
                    
                    // Try to kill as a process
                    if crate::sched::kill(pid as u32) {
                        return 0; // Success
                    }
                    
                    // Try to stop as a service
                    if crate::init::stop_service_by_pid(pid as u32) {
                        return 0; // Success
                    }
                    
                    -1 // Not found
                },
            ),
        )
        .map_err(|e| format!("define kill: {:?}", e))?;

    // ═══════════════════════════════════════════════════════════════════════════════
    // ADDITIONAL SYSCALLS - For migrated native commands
    // ═══════════════════════════════════════════════════════════════════════════════

    // Syscall: version(buf_ptr, buf_len) -> i32
    // Gets kernel version string. Returns bytes written or -1 on error.
    linker
        .define(
            "env",
            "version",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>, buf_ptr: i32, buf_len: i32| -> i32 {
                    let version = env!("CARGO_PKG_VERSION");
                    let bytes = version.as_bytes();
                    if bytes.len() > buf_len as usize {
                        return -1;
                    }
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        if mem.write(&mut caller, buf_ptr as usize, bytes).is_ok() {
                            return bytes.len() as i32;
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define version: {:?}", e))?;

    // Syscall: fs_available() -> i32
    // Returns 1 if filesystem is mounted, 0 otherwise.
    linker
        .define(
            "env",
            "fs_available",
            Func::wrap(&mut store, |_caller: Caller<'_, WasmContext>| -> i32 {
                let fs_guard = crate::FS_STATE.read();
                if fs_guard.is_some() { 1 } else { 0 }
            }),
        )
        .map_err(|e| format!("define fs_available: {:?}", e))?;

    // Syscall: net_info(out_ptr) -> i32
    // Gets network info. Writes: IP (4) + Gateway (4) + DNS (4) + MAC (6) + prefix_len (1) = 19 bytes
    // Returns 0 on success, -1 if network not available.
    // Supports both VirtIO and D1 EMAC network backends.
    linker
        .define(
            "env",
            "net_info",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>, out_ptr: i32| -> i32 {
                    // Use unified NET_STATE
                    let net_guard = crate::NET_STATE.lock();
                    let (ip, mac) = if let Some(ref state) = *net_guard {
                        (crate::net::get_my_ip(), state.mac())
                    } else {
                        return -1;
                    };
                    
                    // Pack: IP (4) + Gateway (4) + DNS (4) + MAC (6) + prefix_len (1) = 19 bytes
                    let mut out = [0u8; 19];
                    out[0..4].copy_from_slice(&ip.octets());
                    out[4..8].copy_from_slice(&crate::net::GATEWAY.octets());
                    out[8..12].copy_from_slice(&crate::net::DNS_SERVER.octets());
                    out[12..18].copy_from_slice(&mac);
                    out[18] = crate::net::PREFIX_LEN;
                    
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        if mem.write(&mut caller, out_ptr as usize, &out).is_ok() {
                            return 0;
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define net_info: {:?}", e))?;

    // Syscall: fs_remove(path_ptr, path_len) -> i32
    // Removes a file. Returns 0 on success, -1 on error.
    linker
        .define(
            "env",
            "fs_remove",
            Func::wrap(
                &mut store,
                |caller: Caller<'_, WasmContext>, path_ptr: i32, path_len: i32| -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut path_buf = vec![0u8; path_len as usize];
                        if mem.read(&caller, path_ptr as usize, &mut path_buf).is_ok() {
                            if let Ok(path) = core::str::from_utf8(&path_buf) {
                                let mut fs_guard = crate::FS_STATE.write();
                                let mut blk_guard = BLK_DEV.write();
                                if let (Some(fs), Some(dev)) =
                                    (fs_guard.as_mut(), blk_guard.as_mut())
                                {
                                    if fs.remove(dev, path).is_ok() {
                                        return 0;
                                    }
                                }
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define fs_remove: {:?}", e))?;

    // Syscall: fs_is_dir(path_ptr, path_len) -> i32
    // Checks if path is a directory. Returns 1 if dir, 0 if not, -1 on error.
    linker
        .define(
            "env",
            "fs_is_dir",
            Func::wrap(
                &mut store,
                |caller: Caller<'_, WasmContext>, path_ptr: i32, path_len: i32| -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut path_buf = vec![0u8; path_len as usize];
                        if mem.read(&caller, path_ptr as usize, &mut path_buf).is_ok() {
                            if let Ok(path) = core::str::from_utf8(&path_buf) {
                                let mut fs_guard = crate::FS_STATE.write();
                                let mut blk_guard = BLK_DEV.write();
                                if let (Some(fs), Some(dev)) =
                                    (fs_guard.as_mut(), blk_guard.as_mut())
                                {
                                    return if fs.is_dir(dev, path) { 1 } else { 0 };
                                }
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define fs_is_dir: {:?}", e))?;

    // Syscall: service_list_defs(buf_ptr, buf_len) -> i32
    // Gets available service definitions. Format: "name:description\n"
    // Returns bytes written or -1 on error.
    linker
        .define(
            "env",
            "service_list_defs",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>, buf_ptr: i32, buf_len: i32| -> i32 {
                    let defs = crate::init::list_service_defs();
                    let mut output = String::new();
                    for (name, desc) in defs {
                        output.push_str(&name);
                        output.push(':');
                        output.push_str(&desc);
                        output.push('\n');
                    }
                    let bytes = output.as_bytes();
                    if bytes.len() > buf_len as usize {
                        return -1;
                    }
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        if mem.write(&mut caller, buf_ptr as usize, bytes).is_ok() {
                            return bytes.len() as i32;
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define service_list_defs: {:?}", e))?;

    // Syscall: service_list_running(buf_ptr, buf_len) -> i32
    // Gets running services. Format: "name:status:pid\n"
    // Returns bytes written or -1 on error.
    linker
        .define(
            "env",
            "service_list_running",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>, buf_ptr: i32, buf_len: i32| -> i32 {
                    let svcs = crate::init::list_services();
                    let mut output = String::new();
                    for svc in svcs {
                        output.push_str(&svc.name);
                        output.push(':');
                        output.push_str(svc.status.as_str());
                        output.push(':');
                        output.push_str(&format!("{}", svc.pid));
                        output.push('\n');
                    }
                    let bytes = output.as_bytes();
                    if bytes.len() > buf_len as usize {
                        return -1;
                    }
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        if mem.write(&mut caller, buf_ptr as usize, bytes).is_ok() {
                            return bytes.len() as i32;
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define service_list_running: {:?}", e))?;

    // Syscall: service_start(name_ptr, name_len) -> i32
    // Starts a service. Returns 0 on success, -1 on error.
    linker
        .define(
            "env",
            "service_start",
            Func::wrap(
                &mut store,
                |caller: Caller<'_, WasmContext>, name_ptr: i32, name_len: i32| -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut name_buf = vec![0u8; name_len as usize];
                        if mem.read(&caller, name_ptr as usize, &mut name_buf).is_ok() {
                            if let Ok(name) = core::str::from_utf8(&name_buf) {
                                if crate::init::start_service(name).is_ok() {
                                    return 0;
                                }
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define service_start: {:?}", e))?;

    // Syscall: service_stop(name_ptr, name_len) -> i32
    // Stops a service. Returns 0 on success, -1 on error.
    linker
        .define(
            "env",
            "service_stop",
            Func::wrap(
                &mut store,
                |caller: Caller<'_, WasmContext>, name_ptr: i32, name_len: i32| -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut name_buf = vec![0u8; name_len as usize];
                        if mem.read(&caller, name_ptr as usize, &mut name_buf).is_ok() {
                            if let Ok(name) = core::str::from_utf8(&name_buf) {
                                if crate::init::stop_service(name).is_ok() {
                                    return 0;
                                }
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define service_stop: {:?}", e))?;

    // Syscall: service_restart(name_ptr, name_len) -> i32
    // Restarts a service. Returns 0 on success, -1 on error.
    linker
        .define(
            "env",
            "service_restart",
            Func::wrap(
                &mut store,
                |caller: Caller<'_, WasmContext>, name_ptr: i32, name_len: i32| -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut name_buf = vec![0u8; name_len as usize];
                        if mem.read(&caller, name_ptr as usize, &mut name_buf).is_ok() {
                            if let Ok(name) = core::str::from_utf8(&name_buf) {
                                if crate::init::restart_service(name).is_ok() {
                                    return 0;
                                }
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define service_restart: {:?}", e))?;

    // Syscall: service_status(name_ptr, name_len, out_ptr, out_len) -> i32
    // Gets service status. Returns status string length or -1 if not found.
    linker
        .define(
            "env",
            "service_status",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>,
                 name_ptr: i32,
                 name_len: i32,
                 out_ptr: i32,
                 out_len: i32|
                 -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut name_buf = vec![0u8; name_len as usize];
                        if mem.read(&caller, name_ptr as usize, &mut name_buf).is_ok() {
                            if let Ok(name) = core::str::from_utf8(&name_buf) {
                                if let Some(status) = crate::init::service_status(name) {
                                    let status_str = status.as_str();
                                    let bytes = status_str.as_bytes();
                                    if bytes.len() <= out_len as usize {
                                        if mem.write(&mut caller, out_ptr as usize, bytes).is_ok() {
                                            return bytes.len() as i32;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define service_status: {:?}", e))?;

    // Syscall: send_ping(ip_ptr, seq, timeout_ms, out_ptr) -> i32
    // Sends a single ICMP ping and waits for reply.
    // ip_ptr points to 4 bytes (IPv4 address)
    // seq is the sequence number to use
    // out_ptr receives: rtt_ms (4 bytes) on success
    // Returns 0 on success, -1 on timeout, -2 on network error.
    linker
        .define(
            "env",
            "send_ping",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>,
                 ip_ptr: i32,
                 seq: i32,
                 timeout_ms: i32,
                 out_ptr: i32|
                 -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut ip_buf = [0u8; 4];
                        if mem.read(&caller, ip_ptr as usize, &mut ip_buf).is_err() {
                            return -2;
                        }
                        
                        let target = smoltcp::wire::Ipv4Address::new(ip_buf[0], ip_buf[1], ip_buf[2], ip_buf[3]);
                        let seq = seq as u16;
                        let timestamp = crate::get_time_ms();
                        
                        // Send ping using unified NET_STATE
                        let send_result = {
                            let mut net_guard = crate::NET_STATE.lock();
                            if let Some(ref mut state) = *net_guard {
                                state.send_ping(target, seq, timestamp)
                            } else {
                                return -2; // No network available
                            }
                        };
                        
                        if send_result.is_err() {
                            return -2;
                        }
                        
                        // Wait for reply
                        let deadline = timestamp + timeout_ms as i64;
                        loop {
                            let now = crate::get_time_ms();
                            if now >= deadline {
                                return -1; // Timeout
                            }
                            
                            // Poll network and check for reply
                            let reply = {
                                let mut net_guard = crate::NET_STATE.lock();
                                if let Some(ref mut state) = *net_guard {
                                    state.poll(now);
                                    state.check_ping_reply()
                                } else {
                                    None
                                }
                            };
                            
                            if let Some((reply_ip, _ident, reply_seq)) = reply {
                                // Check if this is the reply we're waiting for
                                if reply_ip == target && reply_seq == seq {
                                    let rtt = (now - timestamp) as u32;
                                    // Write result (rtt in ms)
                                    let out = rtt.to_le_bytes();
                                    if mem.write(&mut caller, out_ptr as usize, &out).is_ok() {
                                        return 0;
                                    }
                                    return -2;
                                }
                            }
                            
                            core::hint::spin_loop();
                        }
                    }
                    -2
                },
            ),
        )
        .map_err(|e| format!("define send_ping: {:?}", e))?;

    // ═══════════════════════════════════════════════════════════════════════════════
    // TCP SOCKET SYSCALLS - For user-space TCP clients
    // ═══════════════════════════════════════════════════════════════════════════════

    // Syscall: tcp_connect(ip_ptr, ip_len, port) -> i32
    // Connect to a TCP server. ip_ptr points to IP address bytes (4 bytes for IPv4).
    // Returns 0 on success (connection initiated), -1 on error.
    linker
        .define(
            "env",
            "tcp_connect",
            Func::wrap(
                &mut store,
                |caller: Caller<'_, WasmContext>, ip_ptr: i32, _ip_len: i32, port: i32| -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut ip_buf = [0u8; 4];
                        if mem.read(&caller, ip_ptr as usize, &mut ip_buf).is_ok() {
                            klog_info("telnet", &alloc::format!(
                                "tcp_connect to {}.{}.{}.{}:{}",
                                ip_buf[0], ip_buf[1], ip_buf[2], ip_buf[3], port
                            ));
                            let mut net_guard = crate::NET_STATE.lock();
                            if let Some(ref mut net) = *net_guard {
                                let ip = smoltcp::wire::Ipv4Address::new(ip_buf[0], ip_buf[1], ip_buf[2], ip_buf[3]);
                                let now = crate::get_time_ms();
                                match net.tcp_connect(ip, port as u16, now) {
                                    Ok(()) => {
                                        // Poll to actually send the SYN packet
                                        net.poll(now);
                                        klog_info("telnet", "SYN sent");
                                        return 0;
                                    }
                                    Err(e) => {
                                        klog_info("telnet", &alloc::format!("connect error: {}", e));
                                    }
                                }
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define tcp_connect: {:?}", e))?;

    // Syscall: tcp_send(data_ptr, data_len) -> i32
    // Send data over TCP connection. Returns bytes sent or -1 on error.
    linker
        .define(
            "env",
            "tcp_send",
            Func::wrap(
                &mut store,
                |caller: Caller<'_, WasmContext>, data_ptr: i32, data_len: i32| -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut data_buf = vec![0u8; data_len as usize];
                        if mem.read(&caller, data_ptr as usize, &mut data_buf).is_ok() {
                            let mut net_guard = crate::NET_STATE.lock();
                            if let Some(ref mut net) = *net_guard {
                                let now = crate::get_time_ms();
                                match net.tcp_send(&data_buf, now) {
                                    Ok(sent) => {
                                        // Poll network to actually transmit
                                        net.poll(now);
                                        return sent as i32;
                                    }
                                    Err(_) => return -1,
                                }
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define tcp_send: {:?}", e))?;

    // Syscall: tcp_recv(buf_ptr, buf_len, timeout_ms) -> i32
    // Receive data from TCP connection. Returns bytes received, 0 if no data, -1 on error/closed.
    linker
        .define(
            "env",
            "tcp_recv",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>, buf_ptr: i32, buf_len: i32, timeout_ms: i32| -> i32 {
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let mut net_guard = crate::NET_STATE.lock();
                        if let Some(ref mut net) = *net_guard {
                            let start = crate::get_time_ms();
                            let deadline = if timeout_ms > 0 {
                                start + timeout_ms as i64
                            } else {
                                start // No timeout, just try once
                            };
                            
                            let mut recv_buf = vec![0u8; buf_len as usize];
                            
                            loop {
                                let now = crate::get_time_ms();
                                
                                // Poll network to process incoming packets and recycle TX buffers
                                net.poll(now);
                                
                                match net.tcp_recv(&mut recv_buf, now) {
                                    Ok(len) if len > 0 => {
                                        if mem.write(&mut caller, buf_ptr as usize, &recv_buf[..len]).is_ok() {
                                            return len as i32;
                                        }
                                        return -1;
                                    }
                                    Ok(_) => {
                                        // No data yet
                                        if timeout_ms <= 0 || now >= deadline {
                                            return 0;
                                        }
                                        // Brief delay before retry
                                        for _ in 0..1000 { core::hint::spin_loop(); }
                                    }
                                    Err(_) => return -1,
                                }
                            }
                        }
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define tcp_recv: {:?}", e))?;

    // Syscall: tcp_close() -> i32
    // Close TCP connection. Returns 0 on success.
    linker
        .define(
            "env",
            "tcp_close",
            Func::wrap(
                &mut store,
                |_caller: Caller<'_, WasmContext>| -> i32 {
                    let mut net_guard = crate::NET_STATE.lock();
                    if let Some(ref mut net) = *net_guard {
                        let now = crate::get_time_ms();
                        net.tcp_close(now);
                        return 0;
                    }
                    -1
                },
            ),
        )
        .map_err(|e| format!("define tcp_close: {:?}", e))?;

    // Syscall: tcp_status() -> i32
    // Get TCP connection status.
    // Returns: 0=closed, 1=connecting, 2=connected, 3=failed
    linker
        .define(
            "env",
            "tcp_status",
            Func::wrap(
                &mut store,
                |_caller: Caller<'_, WasmContext>| -> i32 {
                    let mut net_guard = crate::NET_STATE.lock();
                    if let Some(ref mut net) = *net_guard {
                        // Poll multiple times to ensure we catch pending handshake packets
                        // This is critical for TCP connections as SYN-ACK may arrive between polls
                        let now = crate::get_time_ms();
                        for _ in 0..5 {
                            net.poll(now);
                        }
                        
                        // Get the actual socket state for debugging
                        let socket_state = net.tcp_client_state();
                        
                        // Log socket state periodically (every ~500ms)
                        static mut LAST_LOG: i64 = 0;
                        static mut LAST_STATE: &str = "";
                        unsafe {
                            if socket_state != LAST_STATE || now - LAST_LOG > 500 {
                                klog_info("telnet", &alloc::format!(
                                    "client socket state: {}", socket_state
                                ));
                                LAST_LOG = now;
                                LAST_STATE = socket_state;
                            }
                        }
                        
                        if net.tcp_is_connected() {
                            return 2; // Connected
                        } else if net.tcp_is_connecting() {
                            return 1; // Connecting
                        } else if net.tcp_connection_failed() {
                            return 3; // Failed
                        } else {
                            return 0; // Closed
                        }
                    }
                    0
                },
            ),
        )
        .map_err(|e| format!("define tcp_status: {:?}", e))?;

    let module = Module::new(&engine, wasm_bytes).map_err(|e| format!("Invalid WASM: {:?}", e))?;

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| format!("Instantiate: {:?}", e))?
        .ensure_no_start(&mut store)
        .map_err(|e| format!("Start: {:?}", e))?;

    let run = instance
        .get_typed_func::<(), ()>(&store, "_start")
        .map_err(|e| format!("Missing _start: {:?}", e))?;

    run.call(&mut store, ())
        .map_err(|e| format!("Runtime: {:?}", e))?;

    Ok(String::new())
}
