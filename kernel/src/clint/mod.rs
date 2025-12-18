use crate::constants::CLINT_MTIME;

pub(crate) fn get_time_ms() -> i64 {
    let mtime = unsafe { core::ptr::read_volatile(CLINT_MTIME as *const u64) };
    (mtime / 10_000) as i64
}