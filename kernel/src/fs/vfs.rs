//! Virtual File System (VFS) Abstraction
//!
//! This module provides a trait-based abstraction for filesystems, allowing
//! multiple filesystem implementations (SFS, 9P, etc.) to be mounted at
//! different paths.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Information about a file or directory
#[derive(Clone, Debug)]
pub struct FileInfo {
    pub name: String,
    pub size: u32,
    pub is_dir: bool,
}

/// Abstract Filesystem Interface
///
/// Implemented by specific filesystem drivers (SFS, P9, etc.)
pub trait FileSystem: Send + Sync {
    /// Read a file's contents
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>>;
    
    /// Write data to a file (creates if doesn't exist)
    fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), &'static str>;
    
    /// List directory contents
    fn list_dir(&mut self, path: &str) -> Vec<FileInfo>;
    
    /// Check if a path exists
    fn exists(&mut self, path: &str) -> bool;
    
    /// Check if a path is a directory
    fn is_dir(&mut self, path: &str) -> bool;
    
    /// Remove a file or empty directory
    fn remove(&mut self, path: &str) -> Result<(), &'static str>;
    
    /// Sync any cached data to storage
    fn sync(&mut self) -> Result<usize, &'static str>;
    
    /// Create a directory
    fn mkdir(&mut self, path: &str) -> Result<(), &'static str>;
}

/// Mount point entry
struct MountPoint {
    path: String,
    fs: Box<dyn FileSystem>,
}

/// Virtual File System Router
///
/// Routes filesystem operations to the appropriate mounted filesystem
/// based on path prefixes.
pub struct Vfs {
    /// Mount points, sorted by path length descending for longest-prefix matching
    mounts: Vec<MountPoint>,
}

impl Vfs {
    /// Create a new empty VFS
    pub const fn new() -> Self {
        Self { mounts: Vec::new() }
    }

    /// Mount a filesystem at a path
    ///
    /// # Arguments
    /// * `mount_point` - Path where the filesystem is mounted (e.g., "/mnt")
    /// * `fs` - The filesystem implementation
    pub fn mount(&mut self, mount_point: &str, fs: Box<dyn FileSystem>) {
        // Normalize mount point (ensure it starts with / and doesn't end with /)
        let normalized = if mount_point == "/" {
            String::from("/")
        } else {
            let mut s = String::from(mount_point);
            if !s.starts_with('/') {
                s.insert(0, '/');
            }
            while s.ends_with('/') && s.len() > 1 {
                s.pop();
            }
            s
        };

        self.mounts.push(MountPoint {
            path: normalized,
            fs,
        });

        // Sort by length descending for longest-prefix matching
        self.mounts.sort_by(|a, b| b.path.len().cmp(&a.path.len()));
    }

    /// Resolve a path to a filesystem and relative path within that filesystem
    fn resolve_mut(&mut self, path: &str) -> Option<(&mut Box<dyn FileSystem>, String)> {
        for mount in &mut self.mounts {
            if path == mount.path {
                // Exact match - relative path is root
                return Some((&mut mount.fs, String::from("/")));
            } else if mount.path == "/" {
                // Root mount matches everything
                return Some((&mut mount.fs, String::from(path)));
            } else if path.starts_with(&mount.path) {
                // Check for proper path boundary (must be followed by / or end)
                let rest = &path[mount.path.len()..];
                if rest.starts_with('/') || rest.is_empty() {
                    let relative = if rest.is_empty() {
                        String::from("/")
                    } else {
                        String::from(rest)
                    };
                    return Some((&mut mount.fs, relative));
                }
            }
        }
        None
    }

    /// List mount points
    pub fn list_mounts(&self) -> Vec<&str> {
        self.mounts.iter().map(|m| m.path.as_str()).collect()
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // FileSystem trait forwarding methods
    // ═══════════════════════════════════════════════════════════════════════════

    /// Read a file
    pub fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        use crate::device::uart::write_str;
        if let Some((fs, relative)) = self.resolve_mut(path) {
            let result = fs.read_file(&relative);
            result
        } else {
            None
        }
    }

    /// Write a file
    pub fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), &'static str> {
        let (fs, relative) = self.resolve_mut(path).ok_or("No filesystem mounted")?;
        fs.write_file(&relative, data)
    }

    /// List directory contents
    pub fn list_dir(&mut self, path: &str) -> Vec<FileInfo> {
        // Normalize path
        let normalized = if path.is_empty() { "/" } else { path };
        
        // FIRST: Collect mount points that should appear in this directory
        // We do this before resolve_mut to avoid borrow issues
        let mut mount_entries: Vec<FileInfo> = Vec::new();
        
        for mount in &self.mounts {
            // Skip root mount
            if mount.path == "/" {
                continue;
            }

            // Normalize listing path with trailing slash for matching
            let listing_prefix = if normalized == "/" {
                String::from("/")
            } else {
                let mut p = String::from(normalized);
                if !p.ends_with('/') {
                    p.push('/');
                }
                p
            };

            // Check if mount path is under the listing directory
            if mount.path.starts_with(&listing_prefix) || 
               (normalized == "/" && mount.path.starts_with('/')) {
                // Get the portion of mount path after the listing directory
                let relative = if normalized == "/" {
                    &mount.path[1..] // Skip leading /
                } else {
                    &mount.path[listing_prefix.len()..]
                };
                
                if relative.is_empty() {
                    continue;
                }
                
                // Get the first component (immediate child directory)
                let first_component = if let Some(slash_pos) = relative.find('/') {
                    &relative[..slash_pos]
                } else {
                    relative // Direct mount point (like "disk1" from "/mnt/disk1" when listing "/mnt")
                };
                
                if first_component.is_empty() {
                    continue;
                }
                
                // Build entry name - use full path for root, relative for non-root
                // Root: /mnt/ (so ls parses as directory)
                // Non-root: disk1/ (relative to current directory)
                let entry_name = if normalized == "/" {
                    format!("/{}/", first_component)
                } else {
                    format!("{}/", first_component)
                };
                
                mount_entries.push(FileInfo {
                    name: entry_name,
                    size: 0,
                    is_dir: true, // Mount paths are always directories
                });
            }
        }
        
        // SECOND: Get entries from the actual filesystem for this path
        let mut entries = if let Some((fs, relative)) = self.resolve_mut(normalized) {
            fs.list_dir(&relative)
        } else {
            Vec::new()
        };

        // THIRD: Add mount entries that don't already exist
        for mount_entry in mount_entries {
            let already_exists = entries.iter().any(|e| e.name == mount_entry.name);
            if !already_exists {
                entries.push(mount_entry);
            }
        }

        entries
    }



    /// Check if path exists
    pub fn exists(&mut self, path: &str) -> bool {
        // Normalize path
        let normalized = path.trim_end_matches('/');
        let path_with_slash = format!("{}/", normalized);
        
        // Check if path is a mount point or an intermediate directory
        for mount in &self.mounts {
            // Exact mount point match
            if mount.path == normalized {
                return true;
            }
            // Check if any mount path starts with this path (intermediate directory)
            // e.g., /mnt exists because /mnt/disk1 is a mount point
            if mount.path.starts_with(&path_with_slash) {
                return true;
            }
        }

        if let Some((fs, relative)) = self.resolve_mut(path) {
            fs.exists(&relative)
        } else {
            false
        }
    }

    /// Check if path is a directory
    pub fn is_dir(&mut self, path: &str) -> bool {
        // Normalize path
        let normalized = path.trim_end_matches('/');
        let path_with_slash = format!("{}/", normalized);
        
        // Mount points and intermediate directories are always directories
        for mount in &self.mounts {
            // Exact mount point match
            if mount.path == normalized {
                return true;
            }
            // Check if any mount path starts with this path (intermediate directory)
            if mount.path.starts_with(&path_with_slash) {
                return true;
            }
        }

        if let Some((fs, relative)) = self.resolve_mut(path) {
            fs.is_dir(&relative)
        } else {
            false
        }
    }

    /// Remove a file or directory
    pub fn remove(&mut self, path: &str) -> Result<(), &'static str> {
        // Cannot remove mount points
        for mount in &self.mounts {
            if path == mount.path {
                return Err("Cannot remove mount point");
            }
        }

        let (fs, relative) = self.resolve_mut(path).ok_or("No filesystem mounted")?;
        fs.remove(&relative)
    }

    /// Sync all mounted filesystems
    pub fn sync(&mut self) -> Result<usize, &'static str> {
        let mut total = 0;
        for mount in &mut self.mounts {
            total += mount.fs.sync()?;
        }
        Ok(total)
    }

    /// Create a directory
    pub fn mkdir(&mut self, path: &str) -> Result<(), &'static str> {
        let (fs, relative) = self.resolve_mut(path).ok_or("No filesystem mounted")?;
        fs.mkdir(&relative)
    }
}
