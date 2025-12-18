//! Block device state
//!
//! Re-exports the D1 MMC driver as BlockDeviceState for consistency
//! with the state module structure. The actual hardware driver
//! implementation remains in platform/d1_mmc.rs.

// Re-export from platform module
pub use crate::platform::d1_mmc::D1Mmc as BlockDeviceState;

// Type alias for backwards compatibility
pub type D1Mmc = BlockDeviceState;


