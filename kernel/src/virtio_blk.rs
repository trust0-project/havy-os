//! VirtIO Block Driver for the kernel.
//!
//! This driver communicates with the VirtIO MMIO block device
//! to read and write disk sectors.
//!
//! ## Thread Safety
//!
//! Queue memory is heap-allocated per-device (not static), eliminating
//! race conditions when multiple harts access the block device.
//! The device is protected by RwLock at a higher level (BLK_DEV in main.rs).

use alloc::boxed::Box;
use crate::virtio_net::{VIRTIO_BASE, VIRTIO_STRIDE, VirtQueue, QUEUE_SIZE};
use core::ptr::{read_volatile, write_volatile};

const VIRTIO_BLK_DEVICE_ID: u32 = 2;

/// Page size for queue alignment
const PAGE_SIZE: usize = 4096;

/// Queue memory size (2 pages for descriptors + avail + used rings)
const QUEUE_MEM_SIZE: usize = PAGE_SIZE * 2;

#[repr(C)]
struct VirtioBlkReqHeader {
    req_type: u32,
    reserved: u32,
    sector: u64,
}

/// Heap-allocated, page-aligned queue memory.
///
/// Using a Box ensures the memory is owned by this device instance
/// and automatically freed when the device is dropped.
#[repr(C, align(4096))]
struct BlkQueueMem {
    data: [u8; QUEUE_MEM_SIZE],
}

impl BlkQueueMem {
    /// Allocate zeroed queue memory on the heap
    fn new() -> Box<Self> {
        Box::new(Self {
            data: [0; QUEUE_MEM_SIZE],
        })
    }
}

pub struct VirtioBlock {
    base: usize,
    /// Heap-allocated queue memory (must stay alive as long as device is in use)
    queue_mem: Box<BlkQueueMem>,
    queue: VirtQueue,
    capacity: u64,
}

/// Type alias for block device abstraction
/// This allows fs.rs to use a platform-independent BlockDev type
pub type BlockDev = VirtioBlock;

impl VirtioBlock {
    pub fn probe() -> Option<Self> {
        for i in 0..8 {
            let addr = VIRTIO_BASE + i * VIRTIO_STRIDE;
            let magic = unsafe { read_volatile((addr + 0x00) as *const u32) };
            let device_id = unsafe { read_volatile((addr + 0x08) as *const u32) };

            if magic == 0x7472_6976 && device_id == VIRTIO_BLK_DEVICE_ID {
                return Self::new(addr);
            }
        }
        None
    }

    fn new(base: usize) -> Option<Self> {
        // Allocate queue memory on heap
        let mut queue_mem = BlkQueueMem::new();
        
        // Create VirtQueue using the heap-allocated memory
        let queue = unsafe { VirtQueue::new(queue_mem.data.as_mut_ptr(), 0) };
        
        let mut dev = VirtioBlock {
            base,
            queue_mem,
            queue,
            capacity: 0,
        };
        
        dev.init();
        Some(dev)
    }

    fn init(&mut self) {
        
        // Check device magic number (should be 0x74726976 = "virt")
        let magic = self.read32(0x000);
        
        // Check version
        let version = self.read32(0x004);
        
        if magic != 0x74726976 {
            crate::uart::write_line("[VIRTIO_BLK] ERROR: Invalid magic - device not present!");
            return;
        }
        
        // Reset device
        self.write32(0x070, 0);
        
        // Wait briefly for reset
        for _ in 0..10000 {
            core::hint::spin_loop();
        }
        
        // Acknowledge + Driver
        self.write32(0x070, 1 | 2);

        // Read capacity from config space
        let cap_low = self.read32(0x100);
        let cap_high = self.read32(0x104);
        self.capacity = (cap_low as u64) | ((cap_high as u64) << 32);

        // Set guest page size
        self.write32(0x028, PAGE_SIZE as u32);
        // Select queue 0
        self.write32(0x030, 0);
        // Set queue size
        self.write32(0x038, QUEUE_SIZE as u32);
        
        // Set queue PFN (page frame number of our heap-allocated memory)
        let pfn = (self.queue_mem.data.as_ptr() as u64) / PAGE_SIZE as u64;
        self.write32(0x040, pfn as u32);
        
        // Driver OK
        self.write32(0x070, 1 | 2 | 4 | 8);
    }

    fn op_sector(
        &mut self,
        sector: u64,
        buf: &mut [u8],
        is_write: bool,
    ) -> Result<(), &'static str> {
        if buf.len() != 512 {
            return Err("Buffer must be 512 bytes");
        }

        // Check device status and re-initialize if needed
        let device_status = self.read32(0x070);
        if device_status != (1 | 2 | 4 | 8) {
            crate::uart::write_str("[VIRTIO_BLK] Device needs re-init, status=");
            crate::uart::write_u64(device_status as u64);
            crate::uart::write_line("");
            
            // Re-initialize the device
            self.init();
            
            // Verify initialization succeeded
            let new_status = self.read32(0x070);
            if new_status != (1 | 2 | 4 | 8) {
                crate::uart::write_str("[VIRTIO_BLK] Re-init FAILED, status=");
                crate::uart::write_u64(new_status as u64);
                crate::uart::write_line("");
                return Err("Device init failed");
            }
            crate::uart::write_line("[VIRTIO_BLK] Device re-initialized successfully");
        }

        // Spin-wait for descriptor allocation
        let start_time = crate::get_time_ms();
        let timeout_ms = 5000;
        
        let (head_idx, data_idx, status_idx) = loop {
            // Process completed operations first
            while self.queue.has_used() {
                self.queue.pop_used();
            }
            
            // Try to allocate all 3 descriptors
            match (
                self.queue.alloc_desc(),
                self.queue.alloc_desc(),
                self.queue.alloc_desc(),
            ) {
                (Some(head), Some(data), Some(status)) => {
                    break (head, data, status);
                }
                (Some(head), Some(data), None) => {
                    self.queue.free_desc(head);
                    self.queue.free_desc(data);
                }
                (Some(head), None, _) => {
                    self.queue.free_desc(head);
                }
                _ => {}
            }
            
            if crate::get_time_ms() - start_time > timeout_ms {
                return Err("Descriptor allocation timeout");
            }
            
            core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
            for _ in 0..1000 {
                core::hint::spin_loop();
            }
        };

        let mut req_hdr = VirtioBlkReqHeader {
            req_type: if is_write { 1 } else { 0 },
            reserved: 0,
            sector,
        };
        let mut req_status: u8 = 0xFF;

        unsafe {
            self.queue.desc[head_idx as usize].addr = &raw const req_hdr as u64;
            self.queue.desc[head_idx as usize].len = 16;
            self.queue.desc[head_idx as usize].flags = 1;
            self.queue.desc[head_idx as usize].next = data_idx;

            self.queue.desc[data_idx as usize].addr = buf.as_ptr() as u64;
            self.queue.desc[data_idx as usize].len = 512;
            self.queue.desc[data_idx as usize].flags = 1 | (if is_write { 0 } else { 2 });
            self.queue.desc[data_idx as usize].next = status_idx;

            self.queue.desc[status_idx as usize].addr = &raw mut req_status as u64;
            self.queue.desc[status_idx as usize].len = 1;
            self.queue.desc[status_idx as usize].flags = 2;

            self.queue.push_avail(head_idx);
            core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
            self.write32(0x050, 0); // Notify device

            // Spin-wait for completion
            let mut timeout = 10_000_000u32;
            while !self.queue.has_used() {
                core::hint::spin_loop();
                timeout = timeout.saturating_sub(1);
                if timeout == 0 {
                    self.queue.free_desc(head_idx);
                    self.queue.free_desc(data_idx);
                    self.queue.free_desc(status_idx);
                    return Err("IO timeout");
                }
            }
            self.queue.pop_used();

            self.queue.free_desc(head_idx);
            self.queue.free_desc(data_idx);
            self.queue.free_desc(status_idx);

            if req_status == 0 {
                Ok(())
            } else {
                Err("IO Error")
            }
        }
    }

    pub fn read_sector(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), &'static str> {
        let result = self.op_sector(sector, buf, false);
        if let Err(e) = &result {
            crate::uart::write_str("[VIRTIO_BLK] read_sector FAILED: sector=");
            crate::uart::write_u64(sector);
            crate::uart::write_str(" error='");
            crate::uart::write_str(e);
            crate::uart::write_line("'");
        }
        result
    }

    pub fn write_sector(&mut self, sector: u64, buf: &[u8]) -> Result<(), &'static str> {
        // Cast const slice to mut slice because op_sector signature expects mut,
        // but for write op the device won't actually modify it.
        let ptr = buf.as_ptr() as *mut u8;
        let mut_slice = unsafe { core::slice::from_raw_parts_mut(ptr, 512) };
        self.op_sector(sector, mut_slice, true)
    }

    fn read32(&self, offset: usize) -> u32 {
        unsafe { read_volatile((self.base + offset) as *const u32) }
    }
    
    fn write32(&self, offset: usize, val: u32) {
        unsafe { write_volatile((self.base + offset) as *mut u32, val) }
    }
    
    pub fn capacity(&self) -> u64 {
        self.capacity
    }
}
