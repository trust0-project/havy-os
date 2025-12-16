//! QEMU virt / riscv-vm emulator platform configuration
//!
//! Memory map compatible with QEMU virt machine and riscv-vm emulator.

use super::Platform;

/// QEMU virt platform
pub struct Virt;

impl Platform for Virt {
    const DRAM_BASE: usize = 0x8000_0000;
    const UART_BASE: usize = 0x1000_0000;
    const TIMER_FREQ_HZ: u64 = 10_000_000; // 10 MHz
}

// ============================================================================
// Memory Map Constants
// ============================================================================

/// DRAM base address
pub const DRAM_BASE: usize = 0x8000_0000;

/// Kernel load address (same as DRAM base for virt)
pub const KERNEL_START: usize = 0x8000_0000;

/// Heap start address (after kernel, approximately 8MB into RAM)
pub const HEAP_START: usize = 0x8080_0000;

/// Heap size (64 MB)
pub const HEAP_SIZE: usize = 64 * 1024 * 1024;

// ============================================================================
// Peripheral Addresses
// ============================================================================

/// NS16550A UART base address
pub const UART_BASE: usize = 0x1000_0000;

/// VirtIO MMIO region start
pub const VIRTIO_BASE: usize = 0x1000_1000;

/// VirtIO MMIO slots (8 devices)
pub const VIRTIO_SLOTS: usize = 8;

/// VirtIO slot size
pub const VIRTIO_SLOT_SIZE: usize = 0x1000;

/// CLINT base address (used by SBI, not directly accessed in S-mode)
pub const CLINT_BASE: usize = 0x0200_0000;

/// PLIC base address
pub const PLIC_BASE: usize = 0x0C00_0000;

// ============================================================================
// Timer Configuration
// ============================================================================

/// Timer frequency in Hz (10 MHz for QEMU virt/riscv-vm)
pub const TIMER_FREQ_HZ: u64 = 10_000_000;

/// Timer interval for 10ms tick
pub const TIMER_INTERVAL: u64 = TIMER_FREQ_HZ / 100;

// ============================================================================
// UART Type
// ============================================================================

/// UART type for this platform
pub const UART_TYPE: &str = "ns16550a";

// ============================================================================
// VirtIO Configuration
// ============================================================================

/// VirtIO is available on this platform
pub const HAS_VIRTIO: bool = true;

/// Get VirtIO slot address
pub const fn virtio_slot_addr(slot: usize) -> usize {
    VIRTIO_BASE + slot * VIRTIO_SLOT_SIZE
}
