//! Simple File System (SFS)
//!
//! This module re-exports the filesystem implementation from `lock::state::fs`.
//! The actual implementation lives in `lock/state/fs.rs` for better state organization.
//!
//! Features:
//! - Block-level write caching (BufferCache)
//! - Dirty block tracking for efficient sync
//! - LRU eviction for cache management

// Re-export everything from the canonical location
pub use crate::lock::state::fs::{
    FileSystemState,
    FileSystem,  // Type alias for backwards compatibility
    FileInfo,
    BufferCache,
    SEC_DIR_START,
    SEC_DIR_COUNT,
};
