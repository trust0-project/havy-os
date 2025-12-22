use alloc::boxed::Box;
use alloc::format;

use crate::boot::console::{print_section, print_status, print_info};
use crate::fs::{FileSystemState, Vfs, GlobalSfs, P9FileSystem};
use crate::lock::utils::{BLK_DEV, FS_STATE, VFS_STATE};
use crate::platform;


/// Ensure required system directories exist
fn ensure_directories() {
    let dirs = ["/var", "/var/log", "/var/run", "/etc", "/tmp"];

    for dir in &dirs {
        // For our simple FS, we just ensure we can write a marker file
        // A real FS would have proper directory support
        // Directory ensured: dir (no-op in our simple FS)
        let _ = dir;
    }
}


pub fn init_storage() {
    print_section("STORAGE SUBSYSTEM");
    
    // Initialize block device
    let mut blk = platform::d1_mmc::D1Mmc::new();
    if blk.init().is_ok() {
        let capacity_mb = blk.capacity() * 512 / 1024 / 1024;
        print_info("Block Device", &format!("{} MiB", capacity_mb));
        *BLK_DEV.write() = Some(blk);
        print_status("D1 MMC driver loaded", true);
        ensure_directories();
    } else {
        print_status("No storage device found", false);
    }

    // Initialize filesystem on block device
    let mut blk_guard = BLK_DEV.write();
    if let Some(ref mut blk) = *blk_guard {
        if let Some(fs) = FileSystemState::init(blk) {
            print_status("SFS Mounted (R/W)", true);
            *FS_STATE.write() = Some(fs);
        }
    }
    drop(blk_guard);

    // Initialize VFS
    init_vfs();
}

/// Initialize the Virtual File System and mount available filesystems
fn init_vfs() {
    let mut vfs = Vfs::new();

    // Mount SFS as root using the GlobalSfs adapter
    // This allows VFS to access the SFS state stored in globals
    if FS_STATE.read().is_some() {
        vfs.mount("/", Box::new(GlobalSfs));
    }

    // Try to mount 9P filesystem at /mnt/disk1 (incremental volume naming)
    if let Some(p9fs) = P9FileSystem::probe() {
        print_status("VirtIO 9P detected", true);
        vfs.mount("/mnt/disk1", Box::new(p9fs));
        print_info("9P Mount", "/mnt/disk1");
    }

    // Store VFS if we have any mounts
    if !vfs.list_mounts().is_empty() {
        *VFS_STATE.write() = Some(vfs);
        print_status("VFS initialized", true);
    }
}
