//! Platform abstraction layer for havy_os
//!
//! This module provides platform-specific constants and initialization
//! for D1 hardware (real hardware and VM emulation).

pub mod d1;

// D1 device drivers
pub mod d1_display;     // D1 Display Engine driver (for D1 hardware and VM D1 emulation)
pub mod d1_emac;        // D1 EMAC Ethernet driver (for D1 hardware and VM D1 emulation)
pub mod d1_mmc;         // D1 MMC/SD card driver
pub mod d1_touch;       // D1 Touch (GT911) driver
pub mod d1_audio;       // D1 Audio codec driver

// Re-export D1 as the active platform
pub use d1 as current;

// Common platform trait (future expansion)
pub trait Platform {
    const DRAM_BASE: usize;
    const UART_BASE: usize;
    const TIMER_FREQ_HZ: u64;
}
