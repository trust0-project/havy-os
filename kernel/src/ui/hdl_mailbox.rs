//! Two-slot HDL mailbox in reserved DRAM.
//!
//! Control-page init plus seq-last publication ([`publish_frame`]). The
//! producer writes the inactive slot, release-fences, then stores `seq`.
//!
//! Layout (`.hdl`, 132 KiB):
//! ```text
//! control  4 KiB   (64 bytes used)
//! slot 0   64 KiB
//! slot 1   64 KiB
//! ```
//!
//! `MAILBOX_PRESENT` is set only when the host advertised a matching DTB
//! capability (virt). D1 compiles the region but never publishes it.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::constants::{
    HDL_ABI_MAJOR, HDL_ABI_MINOR, HDL_ADDR, HDL_CONTROL_SIZE, HDL_REGION_SIZE, HDL_SLOT_COUNT,
    HDL_SLOT_SIZE,
};
use crate::services::klogd::{klog_info, klog_warning};
use crate::ui::hdl;
use crate::ui::input_queue;

/// Little-endian `'HDLM'` — memory bytes `H D L M`.
const HDL_MAGIC: u32 = 0x4D4C_4448;

pub(crate) const FLAG_MAILBOX_PRESENT: u16 = 1 << 0;
pub(crate) const FLAG_HOST_ACCEPTED: u16 = 1 << 1;

const OFF_MAGIC: usize = 0;
const OFF_ABI_MAJOR: usize = 4;
const OFF_ABI_MINOR: usize = 5;
const OFF_FLAGS: usize = 6;
const OFF_SLOT_BYTES: usize = 8;
const OFF_PACK_VERSION: usize = 12;
const OFF_PACK_HASH: usize = 16;
const OFF_SEQ: usize = 24;

unsafe extern "C" {
    static _shdl: u8;
    static _ehdl: u8;
}

static MAILBOX_PRESENT: AtomicBool = AtomicBool::new(false);

/// Linker origin of `.hdl`.
pub(crate) fn region_base() -> usize {
    &raw const _shdl as usize
}

/// One past the last byte of `.hdl`.
pub(crate) fn region_end() -> usize {
    &raw const _ehdl as usize
}

/// Guest has published the mailbox for host scrape.
pub(crate) fn mailbox_present() -> bool {
    MAILBOX_PRESENT.load(Ordering::Acquire)
}

/// Host has validated a complete frame (`flags` bit1). Dual-draw until this.
pub(crate) fn host_accepted() -> bool {
    mailbox_present() && (control_flags() & FLAG_HOST_ACCEPTED) != 0
}

/// Slot `index` (0 or 1). Does not encode a frame.
pub(crate) fn slot_ptr(index: usize) -> *mut u8 {
    let i = index & (HDL_SLOT_COUNT - 1);
    (region_base() + HDL_CONTROL_SIZE + i * HDL_SLOT_SIZE) as *mut u8
}

/// Host advertised a matching mailbox in the DTB. Always false on D1.
fn host_advertised() -> bool {
    #[cfg(feature = "d1")]
    {
        false
    }
    #[cfg(not(feature = "d1"))]
    {
        crate::platform::virt::hdl_host_advertised()
    }
}

/// Zero the control page, write identity fields, optionally set
/// `MAILBOX_PRESENT`. `seq` stays 0 (unpublished).
pub(crate) fn init() {
    let base = region_base();
    let end = region_end();
    if end.saturating_sub(base) != HDL_REGION_SIZE {
        klog_warning(
            "hdl",
            "linker .hdl size does not match the frozen 132 KiB mailbox",
        );
    }
    if base != HDL_ADDR {
        klog_warning(
            "hdl",
            "linker .hdl origin does not match the frozen HDL_ADDR",
        );
    }

    let present = host_advertised();
    unsafe {
        core::ptr::write_bytes(base as *mut u8, 0, HDL_CONTROL_SIZE);

        write_u32(base, OFF_MAGIC, HDL_MAGIC);
        write_u8(base, OFF_ABI_MAJOR, HDL_ABI_MAJOR);
        write_u8(base, OFF_ABI_MINOR, HDL_ABI_MINOR);
        write_u32(base, OFF_SLOT_BYTES, HDL_SLOT_SIZE as u32);
        write_u32(base, OFF_PACK_VERSION, 0);
        write_u64(base, OFF_PACK_HASH, 0);

        let flags = if present { FLAG_MAILBOX_PRESENT } else { 0 };
        write_u16(base, OFF_FLAGS, flags);

        crate::fence_release();
        write_u32(base, OFF_SEQ, 0);
    }

    MAILBOX_PRESENT.store(present, Ordering::Release);

    if present {
        klog_info(
            "hdl",
            "mailbox published (MAILBOX_PRESENT); host DTB ABI matches",
        );
        crate::device::uart::write_line("[hdl] mailbox present");
    } else {
        klog_info(
            "hdl",
            "mailbox unpublished (no DTB capability or D1); framebuffer path",
        );
        crate::device::uart::write_line("[hdl] mailbox unpublished");
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PublishError {
    NotOwner,
    MailboxAbsent,
    TooLarge,
    Truncated,
}

/// Write a complete HDL snapshot into the inactive slot, release-fence, store
/// `seq` last. Header `seq` is stamped to match the mailbox. `seq` 0 is never
/// a published value (wrap stores 2).
///
/// No-op for a host GPU when [`mailbox_present`] is false (D1 / kill-switch).
pub(crate) fn publish_frame(bytes: &[u8]) -> Result<u32, PublishError> {
    if !input_queue::is_owner() {
        return Err(PublishError::NotOwner);
    }
    if !mailbox_present() {
        return Err(PublishError::MailboxAbsent);
    }
    if bytes.len() < hdl::HEADER_BYTES {
        return Err(PublishError::Truncated);
    }
    if bytes.len() > HDL_SLOT_SIZE {
        return Err(PublishError::TooLarge);
    }

    let base = region_base();
    let old = unsafe { read_u32(base, OFF_SEQ) };
    let new = next_seq(old);
    let slot = slot_ptr(new as usize);

    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), slot, bytes.len());
        write_u32(slot as usize, 8, new);
        crate::fence_release();
        write_u32(base, OFF_SEQ, new);
    }
    Ok(new)
}

fn next_seq(old: u32) -> u32 {
    let n = old.wrapping_add(1);
    if n == 0 {
        2
    } else {
        n
    }
}

fn control_flags() -> u16 {
    unsafe { read_u16(region_base(), OFF_FLAGS) }
}

#[inline]
unsafe fn read_u16(base: usize, off: usize) -> u16 {
    core::ptr::read_volatile((base + off) as *const u16)
}

#[inline]
unsafe fn read_u32(base: usize, off: usize) -> u32 {
    core::ptr::read_volatile((base + off) as *const u32)
}

#[inline]
unsafe fn write_u8(base: usize, off: usize, v: u8) {
    core::ptr::write_volatile((base + off) as *mut u8, v);
}

#[inline]
unsafe fn write_u16(base: usize, off: usize, v: u16) {
    core::ptr::write_volatile((base + off) as *mut u16, v);
}

#[inline]
unsafe fn write_u32(base: usize, off: usize, v: u32) {
    core::ptr::write_volatile((base + off) as *mut u32, v);
}

#[inline]
unsafe fn write_u64(base: usize, off: usize, v: u64) {
    core::ptr::write_volatile((base + off) as *mut u64, v);
}
