#[cfg(not(feature = "d1"))]
pub(crate) fn get_time_ms() -> i64 {
    let mtime = unsafe { core::ptr::read_volatile(crate::constants::CLINT_MTIME as *const u64) };
    (mtime / 10_000) as i64
}

#[cfg(feature = "d1")]
pub(crate) fn get_time_ms() -> i64 {
    let time: u64;
    unsafe {
        core::arch::asm!("rdtime {}", out(reg) time, options(nomem, nostack));
    }
    (time / 24_000) as i64
}
