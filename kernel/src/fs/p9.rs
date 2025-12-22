//! 9P Filesystem Implementation
//!
//! Implements the `FileSystem` trait for accessing host-mounted directories
//! via the VirtIO 9P driver.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use super::vfs::{FileSystem, FileInfo};
use crate::device::virtio_p9::{self, VirtioP9Driver, DirEntry};
use crate::Spinlock;

/// 9P Filesystem implementing the VFS FileSystem trait
pub struct P9FileSystem {
    /// The underlying VirtIO 9P driver (wrapped in Spinlock for interior mutability)
    driver: Spinlock<VirtioP9Driver>,
}

impl P9FileSystem {
    /// Create a new P9FileSystem from an initialized driver
    pub fn new(driver: VirtioP9Driver) -> Self {
        Self {
            driver: Spinlock::new(driver),
        }
    }

    /// Try to initialize and return a P9FileSystem
    /// Returns None if no VirtIO 9P device is found
    pub fn probe() -> Option<Self> {
        let mut driver = VirtioP9Driver::probe()?;
        driver.init().ok()?;
        Some(Self::new(driver))
    }
}

impl FileSystem for P9FileSystem {
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        let mut driver = self.driver.lock();
        driver.read_file(path)
    }

    fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), &'static str> {
        let mut driver = self.driver.lock();
        
        // Try to walk to the file
        let fid = match driver.walk(path) {
            Ok(f) => {
                // File exists, open for writing
                if driver.open(f, 1).is_err() { // O_WRONLY
                    let _ = driver.clunk(f);
                    return Err("Failed to open file");
                }
                f
            }
            Err(_) => {
                // File doesn't exist - walk to parent and create
                let (parent, filename) = if let Some(last_slash) = path.rfind('/') {
                    if last_slash == 0 {
                        ("/", &path[1..])
                    } else {
                        (&path[..last_slash], &path[last_slash + 1..])
                    }
                } else {
                    ("/", path)
                };
                
                // Walk to parent directory
                let parent_fid = match driver.walk(parent) {
                    Ok(f) => f,
                    Err(_) => return Err("Parent directory not found"),
                };
                
                // Create the file using lcreate
                match driver.lcreate(parent_fid, filename) {
                    Ok(f) => f,
                    Err(_) => {
                        let _ = driver.clunk(parent_fid);
                        return Err("Failed to create file");
                    }
                }
            }
        };
        
        // Write data
        let result = driver.write(fid, 0, data);
        
        // Close
        let _ = driver.clunk(fid);
        
        result.map(|_| ())
    }

    fn list_dir(&mut self, path: &str) -> Vec<FileInfo> {
        let mut driver = self.driver.lock();
        driver.list_dir(path)
            .into_iter()
            .map(|e| FileInfo {
                name: e.name,
                size: 0, // Would need getattr to get size
                is_dir: e.is_dir,
            })
            .collect()
    }

    fn exists(&mut self, path: &str) -> bool {
        let mut driver = self.driver.lock();
        // Try to walk to the path - if successful, it exists
        match driver.walk(path) {
            Ok(fid) => {
                let _ = driver.clunk(fid);
                true
            }
            Err(_) => false,
        }
    }

    fn is_dir(&mut self, path: &str) -> bool {
        // Root is always a directory
        if path == "/" || path.is_empty() {
            return true;
        }
        
        let mut driver = self.driver.lock();
        // List parent and check entry type
        let entries = driver.list_dir(path);
        // If we got entries, it's a directory
        !entries.is_empty()
    }

    fn remove(&mut self, _path: &str) -> Result<(), &'static str> {
        // 9P remove not implemented in this minimal driver
        Err("Remove not supported on 9P mount")
    }

    fn sync(&mut self) -> Result<usize, &'static str> {
        // 9P sync is handled by host
        Ok(0)
    }

    fn mkdir(&mut self, _path: &str) -> Result<(), &'static str> {
        // 9P mkdir not implemented in this minimal driver
        Err("Mkdir not supported on 9P mount")
    }
}
