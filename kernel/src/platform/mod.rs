//! Platform abstraction layer for havy_os
//!
//! This module provides platform-specific constants and initialization
//! for different target hardware:
//!
//! - `virt`: QEMU virt machine / riscv-vm emulator (default)
//! - `d1`: Allwinner D1 / Lichee RV 86

#[cfg(feature = "d1")]
pub mod d1;

#[cfg(not(feature = "d1"))]
pub mod virt;

// Re-export the active platform as `current`
#[cfg(feature = "d1")]
pub use d1 as current;

#[cfg(not(feature = "d1"))]
pub use virt as current;

// Common platform trait (future expansion)
pub trait Platform {
    const DRAM_BASE: usize;
    const UART_BASE: usize;
    const TIMER_FREQ_HZ: u64;
}
