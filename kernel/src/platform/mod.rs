//! Platform abstraction: one kernel, two machines (`virt` vs `d1`).
//!
//! Addresses, timebase, and which drivers are first-class come from
//! [`current`]. CPU, SBI wrappers, scheduler, FS, and net stay shared.
//!
//! Cargo features (mutually exclusive):
//! - `virt` (default): `link.x` at `0x8000_0000`, NS16550, SiFive PLIC
//! - `d1`: `d1.ld` at `0x4020_0000`, DW UART, T-Head PLIC, SBI TIME

#[cfg(all(feature = "virt", feature = "d1"))]
compile_error!(
    "features `virt` and `d1` are mutually exclusive; \
     build D1 with --no-default-features --features d1"
);

pub mod d1;

#[cfg(not(feature = "d1"))]
pub mod virt;

#[cfg(feature = "d1")]
pub use d1 as current;
#[cfg(not(feature = "d1"))]
pub use virt as current;

pub mod d1_display;

#[cfg(feature = "d1")]
pub mod d1_emac;
#[cfg(feature = "d1")]
pub mod d1_mmc;
#[cfg(feature = "d1")]
pub mod d1_touch;
#[cfg(feature = "d1")]
pub mod d1_audio;
#[cfg(feature = "d1")]
pub mod d1_de;

pub trait Platform {
    const DRAM_BASE: usize;
    const UART_BASE: usize;
    const TIMER_FREQ_HZ: u64;
}
