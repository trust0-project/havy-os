use alloc::{format, string::String, vec, vec::Vec};
use wasmi::{Caller, Config, Engine, Func, Linker, Module, Store};
use core::ptr;

use crate::uart;
use crate::TEST_FINISHER;

/// State to pass to host functions - includes command arguments
struct WasmContext {
    args: Vec<String>,
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
                    let cwd = crate::cwd_get();
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
                                let fs_guard = crate::FS_STATE.lock();
                                let mut blk_guard = crate::BLK_DEV.lock();
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
                                let fs_guard = crate::FS_STATE.lock();
                                let mut blk_guard = crate::BLK_DEV.lock();
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
                                let mut fs_guard = crate::FS_STATE.lock();
                                let mut blk_guard = crate::BLK_DEV.lock();
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
                    let mut fs_guard = crate::FS_STATE.lock();
                    let mut blk_guard = crate::BLK_DEV.lock();
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
                    let entries = crate::klog::KLOG.recent(count);
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
    linker
        .define(
            "env",
            "net_available",
            Func::wrap(&mut store, |_caller: Caller<'_, WasmContext>| -> i32 {
                let net_guard = crate::NET_STATE.lock();
                if net_guard.is_some() {
                    1
                } else {
                    0
                }
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
                                    match crate::http::get_follow_redirects(
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
                                if crate::path_exists(&resolved) {
                                    crate::cwd_set(&resolved);
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
            Func::wrap(&mut store, |_caller: Caller<'_, WasmContext>| {
                uart::write_line("");
                uart::write_line(
                    "\x1b[1;31m╔═══════════════════════════════════════════════════════════════════╗\x1b[0m",
                );
                uart::write_line(
                    "\x1b[1;31m║\x1b[0m                                                                   \x1b[1;31m║\x1b[0m",
                );
                uart::write_line(
                    "\x1b[1;31m║\x1b[0m                    \x1b[1;97mSystem Shutdown Initiated\x1b[0m                       \x1b[1;31m║\x1b[0m",
                );
                uart::write_line(
                    "\x1b[1;31m║\x1b[0m                                                                   \x1b[1;31m║\x1b[0m",
                );
                uart::write_line(
                    "\x1b[1;31m╚═══════════════════════════════════════════════════════════════════╝\x1b[0m",
                );
                uart::write_line("");
                uart::write_line("    \x1b[0;90m[1/3]\x1b[0m Syncing filesystems...");
                uart::write_line("    \x1b[0;90m[2/3]\x1b[0m Stopping network services...");
                uart::write_line("    \x1b[0;90m[3/3]\x1b[0m Powering off CPU...");
                uart::write_line("");
                uart::write_line("    \x1b[1;32m✓ Goodbye!\x1b[0m");
                uart::write_line("");
                unsafe {
                    ptr::write_volatile(TEST_FINISHER as *mut u32, 0x5555);
                }
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
                            let mut net_guard = crate::NET_STATE.lock();
                            if let Some(ref mut net) = *net_guard {
                                let dns_server = smoltcp::wire::Ipv4Address::new(8, 8, 8, 8);
                                if let Some(ip) = crate::dns::resolve(
                                    net,
                                    &host_buf,
                                    dns_server,
                                    5000,
                                    crate::get_time_ms,
                                ) {
                                    if mem.write(&mut caller, ip_buf_ptr as usize, &ip.0).is_ok() {
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
                                let mut fs_guard = crate::FS_STATE.lock();
                                let mut blk_guard = crate::BLK_DEV.lock();
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
                                let mut fs_guard = crate::FS_STATE.lock();
                                let mut blk_guard = crate::BLK_DEV.lock();
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
                                let mut fs_guard = crate::FS_STATE.lock();
                                let mut blk_guard = crate::BLK_DEV.lock();
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
                                        let cwd = crate::cwd_get();
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
                        let fs_guard = crate::FS_STATE.lock();
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

    // Syscall: ps_list(buf_ptr, buf_len) -> i32
    // Gets list of all processes/tasks. Returns bytes written or -1 on error.
    // Format: "pid:name:state:priority:cpu:uptime\n" for each task (cpu = hart number, -1 if not running)
    linker
        .define(
            "env",
            "ps_list",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>, buf_ptr: i32, buf_len: i32| -> i32 {
                    let tasks = crate::scheduler::SCHEDULER.list_tasks();
                    let mut output = String::new();
                    
                    // Include the current shell command (which is the one calling ps_list)
                    // This shows the currently running WASM command with its CPU (hart number)
                    if let Some((name, pid, cpu, uptime, is_running)) = crate::get_shell_cmd_info() {
                        let state = if is_running { "R+" } else { "S" };
                        output.push_str(&format!(
                            "{}:{}:{}:{}:{}:{}\n",
                            pid,
                            name,
                            state,
                            "normal",
                            cpu,
                            uptime
                        ));
                    }
                    
                    for task in tasks {
                        // Format: pid:name:state:priority:cpu:uptime\n (cpu = assigned hart number)
                        output.push_str(&format!(
                            "{}:{}:{}:{}:{}:{}\n",
                            task.pid,
                            task.name,
                            task.state.as_str(),
                            task.priority.as_str(),
                            task.cpu,
                            task.uptime
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
    // Kills a process by PID. Returns 0 on success, -1 if not found or invalid.
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
                    if crate::scheduler::SCHEDULER.kill(pid as u32) {
                        0 // Success
                    } else {
                        -1 // Not found
                    }
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
                let fs_guard = crate::FS_STATE.lock();
                if fs_guard.is_some() { 1 } else { 0 }
            }),
        )
        .map_err(|e| format!("define fs_available: {:?}", e))?;

    // Syscall: net_info(out_ptr) -> i32
    // Gets network info. Writes: IP (4) + Gateway (4) + DNS (4) + MAC (6) + prefix_len (1) = 19 bytes
    // Returns 0 on success, -1 if network not available.
    linker
        .define(
            "env",
            "net_info",
            Func::wrap(
                &mut store,
                |mut caller: Caller<'_, WasmContext>, out_ptr: i32| -> i32 {
                    let net_guard = crate::NET_STATE.lock();
                    if net_guard.is_none() {
                        return -1;
                    }
                    
                    let ip = crate::net::get_my_ip();
                    let mac = if let Some(ref state) = *net_guard {
                        state.mac()
                    } else {
                        [0u8; 6]
                    };
                    drop(net_guard);
                    
                    // Pack: IP (4) + Gateway (4) + DNS (4) + MAC (6) + prefix_len (1) = 19 bytes
                    let mut out = [0u8; 19];
                    out[0..4].copy_from_slice(&ip.0);
                    out[4..8].copy_from_slice(&crate::net::GATEWAY.0);
                    out[8..12].copy_from_slice(&crate::net::DNS_SERVER.0);
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
                                let mut fs_guard = crate::FS_STATE.lock();
                                let mut blk_guard = crate::BLK_DEV.lock();
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
                                let mut fs_guard = crate::FS_STATE.lock();
                                let mut blk_guard = crate::BLK_DEV.lock();
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
                        
                        let target = smoltcp::wire::Ipv4Address(ip_buf);
                        let seq = seq as u16;
                        let timestamp = crate::get_time_ms();
                        
                        // Send ping
                        let send_result = {
                            let mut net_guard = crate::NET_STATE.lock();
                            if let Some(ref mut state) = *net_guard {
                                state.send_ping(target, seq, timestamp)
                            } else {
                                return -2;
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
                                    // check_ping_reply returns (from_ip, ident, seq)
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
    // GENERIC PARALLEL EXECUTION SYSCALLS
    // ═══════════════════════════════════════════════════════════════════════════════
    // 
    // These syscalls enable generic parallel task execution from WASM:
    // 1. Primary WASM submits copies of itself to worker harts with range args
    // 2. Each worker computes its portion and calls parallel_set_result()
    // 3. Primary waits for jobs and calls parallel_sum_results()

    // Syscall: parallel_set_result(slot: i32, value: i64) -> i32
    // Store a result value in a slot (0-31). Workers call this to report results.
    // Returns 0 on success, -1 on invalid slot.
    linker
        .define(
            "env",
            "parallel_set_result",
            Func::wrap(
                &mut store,
                |_caller: Caller<'_, WasmContext>, slot: i32, value: i64| -> i32 {
                    if slot < 0 || slot >= crate::PARALLEL_RESULTS.len() as i32 {
                        return -1;
                    }
                    crate::PARALLEL_RESULTS[slot as usize]
                        .store(value as u64, core::sync::atomic::Ordering::Release);
                    0
                },
            ),
        )
        .map_err(|e| format!("define parallel_set_result: {:?}", e))?;

    // Syscall: parallel_get_result(slot: i32) -> i64
    // Get result value from a slot. Returns 0 if slot invalid.
    linker
        .define(
            "env",
            "parallel_get_result",
            Func::wrap(
                &mut store,
                |_caller: Caller<'_, WasmContext>, slot: i32| -> i64 {
                    if slot < 0 || slot >= crate::PARALLEL_RESULTS.len() as i32 {
                        return 0;
                    }
                    crate::PARALLEL_RESULTS[slot as usize]
                        .load(core::sync::atomic::Ordering::Acquire) as i64
                },
            ),
        )
        .map_err(|e| format!("define parallel_get_result: {:?}", e))?;

    // Syscall: parallel_sum_results(start_slot: i32, count: i32) -> i64
    // Sum result values from slots [start_slot, start_slot + count).
    linker
        .define(
            "env",
            "parallel_sum_results",
            Func::wrap(
                &mut store,
                |_caller: Caller<'_, WasmContext>, start_slot: i32, count: i32| -> i64 {
                    let mut sum = 0u64;
                    for i in 0..count {
                        let slot = start_slot + i;
                        if slot >= 0 && (slot as usize) < crate::PARALLEL_RESULTS.len() {
                            sum += crate::PARALLEL_RESULTS[slot as usize]
                                .load(core::sync::atomic::Ordering::Acquire);
                        }
                    }
                    sum as i64
                },
            ),
        )
        .map_err(|e| format!("define parallel_sum_results: {:?}", e))?;

    // Syscall: parallel_clear_results() -> ()
    // Clear all result slots to 0.
    linker
        .define(
            "env",
            "parallel_clear_results",
            Func::wrap(&mut store, |_caller: Caller<'_, WasmContext>| {
                for slot in crate::PARALLEL_RESULTS.iter() {
                    slot.store(0, core::sync::atomic::Ordering::Release);
                }
            }),
        )
        .map_err(|e| format!("define parallel_clear_results: {:?}", e))?;

    // Syscall: parallel_max_slots() -> i32
    // Returns the maximum number of parallel result slots available.
    linker
        .define(
            "env",
            "parallel_max_slots",
            Func::wrap(&mut store, |_caller: Caller<'_, WasmContext>| -> i32 {
                crate::PARALLEL_RESULTS.len() as i32
            }),
        )
        .map_err(|e| format!("define parallel_max_slots: {:?}", e))?;

    let module = Module::new(&engine, wasm_bytes).map_err(|e| format!("Invalid WASM: {:?}", e))?;

    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .map_err(|e| format!("Link/Start: {:?}", e))?;

    let run = instance
        .get_typed_func::<(), ()>(&store, "_start")
        .map_err(|e| format!("Missing _start: {:?}", e))?;

    run.call(&mut store, ())
        .map_err(|e| format!("Runtime: {:?}", e))?;

    Ok(String::new())
}
