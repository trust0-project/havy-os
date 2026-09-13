//! Virt-only private MMIO symbols, plus the frozen HDL mailbox layout.
//!
//! MMIO addresses below are compiled only without `feature = "d1"` so a D1
//! build cannot accidentally write the QEMU test finisher or CLINT maps that
//! overlap D1 GPIO.
//!
//! SysInfo MMIO (`0x0011_0000`) is gone: heap stats go in the reserved FB
//! doorbell page and `SYS_HEAP_STATS` / `SYS_PERFSTAT`.

#[cfg(not(feature = "d1"))]
pub(crate) const CLINT_MSIP_BASE: usize = 0x0200_0000;
#[cfg(not(feature = "d1"))]
pub(crate) const CLINT_MTIME: usize = 0x0200_BFF8;

#[cfg(not(feature = "d1"))]
#[allow(dead_code)]
pub(crate) const TEST_FINISHER: usize = 0x0010_0000;

// ============================================================================
// HDL mailbox (Phase 1 freeze). Reserved DRAM, not MMIO.
// Virt: 0x8140_0000. D1: 0x4140_0000 (same DRAM-relative offset).
// ============================================================================

/// DRAM-relative offset of `.hdl`. Matches `link.x` / `d1.ld` `_hdl_origin`.
pub(crate) const HDL_OFFSET: usize = 0x0140_0000;

/// Control page (doorbell) at the start of `.hdl`.
pub(crate) const HDL_CONTROL_SIZE: usize = 4096;

/// One command-buffer slot.
pub(crate) const HDL_SLOT_SIZE: usize = 65536;

/// Double-buffered slots. Index = `seq & 1`; `seq == 0` is unpublished.
pub(crate) const HDL_SLOT_COUNT: usize = 2;

/// Total `.hdl` reservation: 4 KiB control + 2 × 64 KiB = 132 KiB.
pub(crate) const HDL_REGION_SIZE: usize =
    HDL_CONTROL_SIZE + HDL_SLOT_COUNT * HDL_SLOT_SIZE;

/// Host/guest ABI major written into the control page and matched against DTB.
pub(crate) const HDL_ABI_MAJOR: u8 = 1;

/// Host/guest ABI minor.
pub(crate) const HDL_ABI_MINOR: u8 = 0;

#[cfg(not(feature = "d1"))]
pub(crate) const HDL_ADDR: usize = 0x8000_0000 + HDL_OFFSET;

#[cfg(feature = "d1")]
pub(crate) const HDL_ADDR: usize = 0x4000_0000 + HDL_OFFSET;
