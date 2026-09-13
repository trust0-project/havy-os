use alloc::boxed::Box;
use alloc::format;

use crate::boot::console::{print_section, print_status, print_info};
use crate::fs::{FileSystemState, Vfs, GlobalSfs, P9FileSystem};
use crate::lock::state::blk::BlockDeviceState;
use crate::lock::utils::{BLK_DEV, FS_STATE, VFS_STATE};
#[cfg(feature = "d1")]
use crate::platform;

fn ensure_directories() {
    let dirs = ["/var", "/var/log", "/var/run", "/etc", "/tmp"];
    let mut blk_guard = crate::lock::utils::BLK_DEV.write();
    let mut fs_guard = crate::lock::utils::FS_STATE.write();
    if let (Some(ref mut blk), Some(ref mut fs)) = (&mut *blk_guard, &mut *fs_guard) {
        for dir in &dirs {
            let _ = fs.mkdir(blk, dir);
        }
    }
}

fn mount_sfs() {
    let mut blk_guard = BLK_DEV.write();
    if let Some(ref mut blk) = *blk_guard {
        if let Some(fs) = FileSystemState::init(blk) {
            print_status("SFS Mounted (R/W)", true);
            *FS_STATE.write() = Some(fs);
        }
    }
}

pub fn init_storage() {
    print_section("STORAGE SUBSYSTEM");

    #[cfg(feature = "d1")]
    {
        let mut blk = platform::d1_mmc::D1Mmc::new();
        if blk.init().is_ok() {
            let capacity_mb = blk.capacity() * 512 / 1024 / 1024;
            print_info("Block Device", &format!("{} MiB (SMHC)", capacity_mb));
            *BLK_DEV.write() = Some(BlockDeviceState::Mmc(blk));
            print_status("D1 MMC driver loaded", true);
        } else {
            print_status("No storage device found", false);
        }
    }

    #[cfg(not(feature = "d1"))]
    {
        if let Some(mut blk) = crate::device::virtio_blk::VirtioBlk::probe() {
            if blk.init().is_ok() {
                let capacity_mb = blk.capacity() * 512 / 1024 / 1024;
                print_info("Block Device", &format!("{} MiB (virtio-blk)", capacity_mb));
                *BLK_DEV.write() = Some(BlockDeviceState::Virtio(blk));
                print_status("virtio-blk driver loaded", true);
            } else {
                print_status("virtio-blk init failed", false);
            }
        } else {
            print_status("No virtio-blk device found", false);
        }
    }

    mount_sfs();
    ensure_directories();
    init_vfs();
}

fn init_vfs() {
    let mut vfs = Vfs::new();

    if FS_STATE.read().is_some() {
        vfs.mount("/", Box::new(GlobalSfs));
    }

    if let Some(p9fs) = P9FileSystem::probe() {
        print_status("VirtIO 9P detected", true);
        vfs.mount("/mnt/disk1", Box::new(p9fs));
        print_info("9P Mount", "/mnt/disk1");
    }

    if !vfs.list_mounts().is_empty() {
        *VFS_STATE.write() = Some(vfs);
        print_status("VFS initialized", true);
    }
}
