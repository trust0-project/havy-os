//! Filesystem Module
//!
//! This module provides the Virtual File System (VFS) abstraction layer
//! that supports multiple filesystem backends:
//!
//! - **SFS**: Simple File System on block devices (default root filesystem)
//! - **P9**: 9P protocol filesystem for host directory mounting
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐
//! │   fs_proxy  │  (Hart-aware filesystem access)
//! └──────┬──────┘
//!        │
//! ┌──────▼──────┐
//! │     VFS     │  (Path-based routing)
//! └──────┬──────┘
//!        │
//!    ┌───┴───┐
//!    │       │
//! ┌──▼──┐ ┌──▼──┐
//! │ SFS │ │ P9  │  (Filesystem implementations)
//! └─────┘ └─────┘
//! ```
//!
//! # Usage
//!
//! Most code should use `cpu::fs_proxy` for filesystem access, which
//! handles Hart-awareness and VFS routing automatically.

pub mod vfs;
pub mod sfs;
pub mod p9;

// Re-export key types
pub use vfs::{FileSystem, Vfs, FileInfo};
pub use sfs::{Sfs, GlobalSfs};
pub use p9::P9FileSystem;


// Re-export legacy types for backwards compatibility
pub use crate::lock::state::fs::{
    FileSystemState,
    BufferCache,
    SEC_DIR_START,
    SEC_DIR_COUNT,
};

// Re-export legacy FileInfo (with different name to avoid conflict)
pub use crate::lock::state::fs::FileInfo as SfsFileInfo;
