//! Per-process file descriptors, `brk`, and anonymous `mmap`.
//!
//! Path-based FS syscalls stay; these add fds and a user heap so ELF
//! binaries can stream and grow without whole-file `SYS_FS_READ`.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::cpu::fs_proxy;

const MAX_FDS: usize = 16;
const BRK_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
enum FdKind {
    Console,
    File {
        path: String,
        offset: u64,
        writable: bool,
    },
}

struct ProcessAbi {
    fds: [Option<FdKind>; MAX_FDS],
    brk_mem: Option<Box<[u8]>>,
    brk_cur: usize,
    mmaps: Vec<(usize, Box<[u8]>)>,
}

static mut PROC: ProcessAbi = ProcessAbi {
    fds: [None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None],
    brk_mem: None,
    brk_cur: 0,
    mmaps: Vec::new(),
};

fn proc() -> &'static mut ProcessAbi {
    unsafe { &mut PROC }
}

/// Reset fd table and user brk for a new ELF.
pub fn reset() {
    let p = proc();
    p.fds = [None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None];
    p.fds[0] = Some(FdKind::Console);
    p.fds[1] = Some(FdKind::Console);
    p.fds[2] = Some(FdKind::Console);
    p.mmaps.clear();
    let mut heap = alloc::vec![0u8; BRK_BYTES].into_boxed_slice();
    crate::paging::map_user(heap.as_ptr() as usize, heap.len());
    p.brk_cur = 0;
    p.brk_mem = Some(heap);
}

pub fn clear() {
    let p = proc();
    p.fds = [None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None];
    p.brk_mem = None;
    p.brk_cur = 0;
    p.mmaps.clear();
}

fn alloc_fd() -> Option<usize> {
    let p = proc();
    (3..MAX_FDS).find(|&i| p.fds[i].is_none())
}

pub fn sys_open(path: &str, flags: i32) -> i64 {
    let writable = flags & 0x3 != 0;
    if !writable && !fs_proxy::fs_exists(path) {
        return -1;
    }
    let fd = match alloc_fd() {
        Some(f) => f,
        None => return -1,
    };
    proc().fds[fd] = Some(FdKind::File {
        path: String::from(path),
        offset: 0,
        writable,
    });
    fd as i64
}

pub fn sys_close(fd: i32) -> i64 {
    if fd < 3 || fd as usize >= MAX_FDS {
        return if fd >= 0 && (fd as usize) < 3 { 0 } else { -1 };
    }
    let p = proc();
    if p.fds[fd as usize].take().is_some() {
        0
    } else {
        -1
    }
}

pub fn sys_read(fd: i32, buf: *mut u8, len: usize) -> i64 {
    if buf.is_null() || len == 0 {
        return -1;
    }
    match proc().fds.get_mut(fd as usize).and_then(|s| s.as_mut()) {
        Some(FdKind::Console) => {
            if let Some(ch) = crate::device::uart::read_char_nonblocking() {
                unsafe {
                    *buf = ch;
                }
                1
            } else {
                0
            }
        }
        Some(FdKind::File { path, offset, .. }) => {
            let data = match fs_proxy::fs_read(path) {
                Some(d) => d,
                None => return -1,
            };
            let start = (*offset as usize).min(data.len());
            let n = (data.len() - start).min(len);
            unsafe {
                core::ptr::copy_nonoverlapping(data.as_ptr().add(start), buf, n);
            }
            *offset += n as u64;
            n as i64
        }
        None => -1,
    }
}

pub fn sys_write(fd: i32, buf: *const u8, len: usize) -> i64 {
    if buf.is_null() {
        return -1;
    }
    let bytes = unsafe { core::slice::from_raw_parts(buf, len) };
    match proc().fds.get_mut(fd as usize).and_then(|s| s.as_mut()) {
        Some(FdKind::Console) => {
            if let Ok(s) = core::str::from_utf8(bytes) {
                crate::scripting::out_str(s);
            }
            len as i64
        }
        Some(FdKind::File {
            path,
            offset,
            writable,
        }) => {
            if !*writable {
                return -1;
            }
            let mut data = fs_proxy::fs_read(path).unwrap_or_default();
            let start = *offset as usize;
            if start + len > data.len() {
                data.resize(start + len, 0);
            }
            data[start..start + len].copy_from_slice(bytes);
            if fs_proxy::fs_write(path, &data).is_err() {
                return -1;
            }
            *offset += len as u64;
            len as i64
        }
        None => -1,
    }
}

pub fn sys_lseek(fd: i32, off: i64, whence: i32) -> i64 {
    match proc().fds.get_mut(fd as usize).and_then(|s| s.as_mut()) {
        Some(FdKind::File { path, offset, .. }) => {
            let size = fs_proxy::fs_read(path).map(|d| d.len() as i64).unwrap_or(0);
            let new = match whence {
                0 => off,
                1 => *offset as i64 + off,
                2 => size + off,
                _ => return -1,
            };
            if new < 0 {
                return -1;
            }
            *offset = new as u64;
            new
        }
        _ => -1,
    }
}

pub fn sys_brk(addr: u64) -> i64 {
    let p = proc();
    let heap = match p.brk_mem.as_mut() {
        Some(h) => h,
        None => return 0,
    };
    let base = heap.as_ptr() as u64;
    if addr == 0 {
        return (base + p.brk_cur as u64) as i64;
    }
    if addr < base {
        return -1;
    }
    let off = (addr - base) as usize;
    if off > heap.len() {
        return -1;
    }
    p.brk_cur = off;
    addr as i64
}

pub fn sys_mmap(len: usize, prot: i32) -> i64 {
    let _ = prot;
    if len == 0 {
        return -1;
    }
    let aligned = (len + 4095) & !4095;
    let mut mem = alloc::vec![0u8; aligned].into_boxed_slice();
    let ptr = mem.as_ptr() as usize;
    crate::paging::map_user(ptr, mem.len());
    proc().mmaps.push((ptr, mem));
    ptr as i64
}

pub fn sys_munmap(addr: u64, _len: usize) -> i64 {
    let p = proc();
    if let Some(i) = p.mmaps.iter().position(|(a, _)| *a == addr as usize) {
        let (ptr, mem) = p.mmaps.remove(i);
        crate::paging::unmap_user(ptr, mem.len());
        drop(mem);
        0
    } else {
        -1
    }
}
