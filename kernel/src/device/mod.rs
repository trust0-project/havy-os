//! Device abstraction layer
//!
//! This module provides trait-based abstractions for hardware devices,
//! allowing the same kernel code to work with both:
//! - Real D1 hardware (Lichee RV 86)
//! - Emulated D1 devices (riscv-vm)
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────┐
//! │                Kernel Code                       │
//! │  (fs.rs, net/, ui/, etc.)                       │
//! └───────────────────┬─────────────────────────────┘
//!                     │
//! ┌───────────────────┴─────────────────────────────┐
//! │              Device Traits                       │
//! │  (BlockDevice, NetworkDevice)                   │
//! └───────────────────┬─────────────────────────────┘
//!                     │
//!         ┌───────────┴───────────┐
//!         │                       │
//!    ┌────┴────┐            ┌────┴────┐
//!    │ d1_mmc  │            │ d1_emac │
//!    │ d1_disp │            │  etc.   │
//!    └─────────┘            └─────────┘
//! ```

pub mod block;
pub mod network;
pub mod display;
pub mod rtc;
pub mod uart;
pub mod virtio;
pub mod virtio_p9;

#[cfg(not(feature = "d1"))]
pub mod virtio_blk;
#[cfg(not(feature = "d1"))]
pub mod virtio_net;
#[cfg(not(feature = "d1"))]
pub mod virtio_gpu;

pub use block::{BlockDevice, BlockError};
pub use network::{NetworkDevice, NetworkError};

