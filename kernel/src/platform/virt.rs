//! QEMU virt platform constants (portable VM / `qemu-system-riscv64 -machine virt`).

use super::Platform;

pub struct Virt;

impl Platform for Virt {
    const DRAM_BASE: usize = 0x8000_0000;
    const UART_BASE: usize = 0x1000_0000;
    const TIMER_FREQ_HZ: u64 = 10_000_000;
}

pub const DRAM_BASE: usize = 0x8000_0000;
pub const KERNEL_START: usize = 0x8000_0000;
pub const DRAM_SIZE: usize = 512 * 1024 * 1024;

pub const UART_BASE: usize = 0x1000_0000;
pub const UART_STRIDE: usize = 1;

pub const PLIC_BASE: usize = 0x0C00_0000;
pub const CLINT_BASE: usize = 0x0200_0000;
pub const CLINT_MTIME: usize = 0x0200_BFF8;
pub const CLINT_MSIP_BASE: usize = 0x0200_0000;

pub const VIRTIO_BASE: usize = 0x1000_1000;
pub const VIRTIO_STRIDE: usize = 0x1000;

pub const TIMER_FREQ_HZ: u64 = 10_000_000;
pub const HAS_VIRTIO: bool = true;
/// Inclusive max hart id. Matches `link.x` `_max_hart_id` (8 harts, 0–7).
pub const MAX_HART_ID: usize = 7;

/// Scanout: 1024×768 XRGB8888, stride already a multiple of 256.
pub const DISPLAY_WIDTH: u32 = 1024;
pub const DISPLAY_HEIGHT: u32 = 768;
pub const FB_STRIDE: usize = 4096;
/// DRAM-relative offset of the reserved doorbell page (matches host scrape).
pub const FB_META_OFFSET: usize = 0x00FF_F000;
/// DRAM-relative offset of the scanout buffer.
pub const FB_OFFSET: usize = 0x0100_0000;
pub const FB_META_ADDR: usize = DRAM_BASE + FB_META_OFFSET;
pub const FB_ADDR: usize = DRAM_BASE + FB_OFFSET;

/// Host advertised the HDL mailbox in the DTB with a matching ABI and address.
///
/// The VM must add this node only when HDL is enabled (`?hdl=0` / CLI off
/// omits it). Guest cannot read `HAVY_HDL` as an environment variable.
///
/// ```dts
/// reserved-memory {
///     #address-cells = <2>;
///     #size-cells = <2>;
///     ranges;
///     hdl-mailbox@81400000 {
///         compatible = "havy,hdl-mailbox";
///         reg = <0x0 0x81400000 0x0 0x21000>;
///         havy,abi-major = <1>;
///         havy,abi-minor = <0>;
///         no-map;
///     };
/// };
/// ```
///
/// Optional `/chosen` fallback: `havy,hdl-mailbox` (same `reg` cells) and
/// `havy,hdl-abi-major = <1>`.
pub fn hdl_host_advertised() -> bool {
    crate::dtb::hdl_mailbox().is_some_and(|m| {
        m.abi_major == crate::constants::HDL_ABI_MAJOR
            && m.base == crate::constants::HDL_ADDR as u64
            && m.size >= crate::constants::HDL_REGION_SIZE as u64
    })
}
