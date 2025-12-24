//! Simple File System (SFS) Wrapper
//!
//! This module provides a `FileSystem` trait implementation around the
//! existing `FileSystemState` from `lock/state/fs.rs`.

use alloc::vec::Vec;
use crate::lock::state::fs::FileSystemState;
use crate::platform::d1_mmc::D1Mmc as BlockDev;
use super::vfs::{FileSystem, FileInfo};

/// Simple File System wrapper implementing the VFS FileSystem trait
///
/// This wraps the existing FileSystemState and BlockDev together,
/// providing a self-contained filesystem implementation.
pub struct Sfs {
    state: FileSystemState,
    dev: BlockDev,
}

impl Sfs {
    /// Create a new SFS instance
    ///
    /// # Arguments
    /// * `state` - The initialized FileSystemState
    /// * `dev` - The block device to use for I/O
    pub fn new(state: FileSystemState, dev: BlockDev) -> Self {
        Self { state, dev }
    }

    /// Get a reference to the underlying FileSystemState
    pub fn state(&self) -> &FileSystemState {
        &self.state
    }

    /// Get a mutable reference to the underlying FileSystemState
    pub fn state_mut(&mut self) -> &mut FileSystemState {
        &mut self.state
    }

    /// Get a mutable reference to the block device
    pub fn device_mut(&mut self) -> &mut BlockDev {
        &mut self.dev
    }

    /// Get cache statistics: (hits, misses, writebacks, cached_blocks)
    pub fn cache_stats(&self) -> (u64, u64, u64, usize) {
        self.state.cache_stats()
    }

    /// Get disk usage statistics: (used_blocks, total_blocks)
    pub fn disk_stats(&self) -> (u64, u64) {
        self.state.disk_stats()
    }
}

impl FileSystem for Sfs {
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        // NOTE: Don't strip leading slash - SFS stores files with full paths including /
        self.state.read_file(&mut self.dev, path)
    }

    fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), &'static str> {
        // NOTE: Don't strip leading slash - SFS stores files with full paths including /
        self.state.write_file(&mut self.dev, path, data)
    }

    fn list_dir(&mut self, path: &str) -> Vec<FileInfo> {
        // SFS has flat structure, path is mostly ignored
        self.state
            .list_dir(&mut self.dev, path)
            .into_iter()
            .map(|e| FileInfo {
                name: e.name,
                size: e.size,
                is_dir: e.is_dir,
            })
            .collect()
    }

    fn exists(&mut self, path: &str) -> bool {
        // NOTE: Don't strip leading slash - SFS stores files with full paths including /
        self.state.exists(&mut self.dev, path)
    }

    fn is_dir(&mut self, path: &str) -> bool {
        // NOTE: Don't strip leading slash - SFS stores files with full paths including /
        self.state.is_dir(&mut self.dev, path)
    }

    fn remove(&mut self, path: &str) -> Result<(), &'static str> {
        // NOTE: Don't strip leading slash - SFS stores files with full paths including /
        self.state.remove(&mut self.dev, path)
    }

    fn sync(&mut self) -> Result<usize, &'static str> {
        self.state.sync(&mut self.dev)
    }

    fn mkdir(&mut self, path: &str) -> Result<(), &'static str> {
        // NOTE: Don't strip leading slash - SFS stores files with full paths including /
        self.state.mkdir(&mut self.dev, path)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Global SFS Adapter
// ═══════════════════════════════════════════════════════════════════════════════

use crate::lock::utils::{FS_STATE, BLK_DEV};

/// Global SFS Adapter
/// 
/// Adapts the global `FS_STATE` and `BLK_DEV` locks into a `FileSystem` trait object.
/// This allows mounting the SFS in VFS without taking ownership of the globals,
/// preserving compatibility with legacy code.
pub struct GlobalSfs;

impl FileSystem for GlobalSfs {
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        use crate::device::uart::write_str;
        

        
        let mut fs_guard = FS_STATE.write();
        let mut blk_guard = BLK_DEV.write();
        
        if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
            let result = fs.read_file(dev, path);
            return result;
        }
        None
    }

    fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), &'static str> {
        let mut fs_guard = FS_STATE.write();
        let mut blk_guard = BLK_DEV.write();
        
        if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
            // NOTE: Don't strip leading slash - SFS stores files with full paths including /
            return fs.write_file(dev, path, data);
        }
        Err("Filesystem not initialized")
    }

    fn list_dir(&mut self, path: &str) -> Vec<FileInfo> {
        let mut fs_guard = FS_STATE.write();
        let mut blk_guard = BLK_DEV.write();
        
        if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
            return fs.list_dir(dev, path)
                .into_iter()
                .map(|e| FileInfo {
                    name: e.name,
                    size: e.size,
                    is_dir: e.is_dir,
                })
                .collect();
        }
        Vec::new()
    }

    fn exists(&mut self, path: &str) -> bool {
        let mut fs_guard = FS_STATE.write();
        let mut blk_guard = BLK_DEV.write();
        
        if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
            // NOTE: Don't strip leading slash - SFS stores files with full paths including /
            return fs.exists(dev, path);
        }
        false
    }

    fn is_dir(&mut self, path: &str) -> bool {
        let mut fs_guard = FS_STATE.write();
        let mut blk_guard = BLK_DEV.write();
        
        if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
            // NOTE: Don't strip leading slash - SFS stores files with full paths including /
            return fs.is_dir(dev, path);
        }
        false
    }

    fn remove(&mut self, path: &str) -> Result<(), &'static str> {
        let mut fs_guard = FS_STATE.write();
        let mut blk_guard = BLK_DEV.write();
        
        if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
            // NOTE: Don't strip leading slash - SFS stores files with full paths including /
            return fs.remove(dev, path);
        }
        Err("Filesystem not initialized")
    }

    fn sync(&mut self) -> Result<usize, &'static str> {
        let mut fs_guard = FS_STATE.write();
        let mut blk_guard = BLK_DEV.write();
        
        if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
            return fs.sync(dev);
        }
        Err("Filesystem not initialized")
    }

    fn mkdir(&mut self, path: &str) -> Result<(), &'static str> {
        let mut fs_guard = FS_STATE.write();
        let mut blk_guard = BLK_DEV.write();
        
        if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
            // NOTE: Don't strip leading slash - SFS stores files with full paths including /
            return fs.mkdir(dev, path);
        }
        Err("Filesystem not initialized")
    }
}
