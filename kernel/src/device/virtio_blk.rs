//! VirtIO block (device ID 2). Legacy QueuePFN and modern split queues.

use alloc::boxed::Box;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

use super::virtio::{self, LegacyQueueMem, VirtqDesc, VRING_DESC_F_NEXT, VRING_DESC_F_WRITE};

const QUEUE_SIZE: u16 = 16;
const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;

#[repr(C)]
struct BlkReq {
    type_: u32,
    reserved: u32,
    sector: u64,
}

pub struct VirtioBlk {
    base: usize,
    queue: Box<LegacyQueueMem>,
    req: Box<BlkReq>,
    data: Box<[u8; 4096]>,
    status: Box<u8>,
    last_used: u16,
    avail_idx: u16,
    capacity: u64,
    modern: bool,
}

impl VirtioBlk {
    pub fn probe() -> Option<Self> {
        let base = virtio::probe(virtio::DEVICE_BLK)?;
        Some(Self {
            base,
            queue: LegacyQueueMem::new(),
            req: Box::new(BlkReq {
                type_: 0,
                reserved: 0,
                sector: 0,
            }),
            data: Box::new([0u8; 4096]),
            status: Box::new(0xff),
            last_used: 0,
            avail_idx: 0,
            capacity: 0,
            modern: virtio::is_modern(base),
        })
    }

    pub fn init(&mut self) -> Result<(), &'static str> {
        virtio::reset(self.base);
        virtio::features_ok(self.base);

        // capacity is the first u64 in config space
        self.capacity = unsafe { read_volatile((self.base + virtio::CONFIG) as *const u64) };
        if self.capacity == 0 {
            self.capacity = 1 << 20; // unknown: 512 MiB of 512 B sectors
        }

        if self.modern {
            virtio::attach_modern(
                self.base,
                0,
                QUEUE_SIZE,
                self.queue.data.as_ptr() as u64,
                self.queue.data.as_ptr() as u64 + (QUEUE_SIZE as u64 * 16),
                self.queue.data.as_ptr() as u64 + virtio::PAGE_SIZE as u64,
            );
        } else {
            virtio::attach_legacy(self.base, 0, QUEUE_SIZE, self.queue.pfn());
        }
        virtio::driver_ok(self.base);
        Ok(())
    }

    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    pub fn read_sector(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), &'static str> {
        if !virtio::is_io_hart() {
            return Err("virtio-blk access requires hart 0");
        }
        if buf.len() < 512 {
            return Err("short buffer");
        }
        self.xfer(VIRTIO_BLK_T_IN, sector, 512)?;
        buf[..512].copy_from_slice(&self.data[..512]);
        Ok(())
    }

    pub fn write_sector(&mut self, sector: u64, buf: &[u8]) -> Result<(), &'static str> {
        if !virtio::is_io_hart() {
            return Err("virtio-blk access requires hart 0");
        }
        if buf.len() < 512 {
            return Err("short buffer");
        }
        self.data[..512].copy_from_slice(&buf[..512]);
        self.xfer(VIRTIO_BLK_T_OUT, sector, 512)
    }

    fn xfer(&mut self, ty: u32, sector: u64, len: u32) -> Result<(), &'static str> {
        self.req.type_ = ty;
        self.req.reserved = 0;
        self.req.sector = sector;
        *self.status = 0xff;

        let data_flags = if ty == VIRTIO_BLK_T_IN {
            VRING_DESC_F_NEXT | VRING_DESC_F_WRITE
        } else {
            VRING_DESC_F_NEXT
        };

        unsafe {
            let d = self.queue.desc();
            *d = VirtqDesc {
                addr: &*self.req as *const BlkReq as u64,
                len: 16,
                flags: VRING_DESC_F_NEXT,
                next: 1,
            };
            *d.add(1) = VirtqDesc {
                addr: self.data.as_ptr() as u64,
                len,
                flags: data_flags,
                next: 2,
            };
            *d.add(2) = VirtqDesc {
                addr: &mut *self.status as *mut u8 as u64,
                len: 1,
                flags: VRING_DESC_F_WRITE,
                next: 0,
            };

            let avail = self.queue.avail(QUEUE_SIZE as usize);
            let ring = avail.add(4) as *mut u16;
            let slot = (self.avail_idx % QUEUE_SIZE) as usize;
            *ring.add(slot) = 0;
            fence(Ordering::SeqCst);
            self.avail_idx = self.avail_idx.wrapping_add(1);
            write_volatile(avail.add(2) as *mut u16, self.avail_idx);
        }

        virtio::notify(self.base, 0);

        for _ in 0..1_000_000 {
            let used = self.queue.used();
            let idx = unsafe { read_volatile(used.add(2) as *const u16) };
            if idx != self.last_used {
                self.last_used = idx;
                virtio::ack_irq(self.base);
                if *self.status == 0 {
                    return Ok(());
                }
                return Err("virtio-blk I/O error");
            }
            core::hint::spin_loop();
        }
        Err("virtio-blk timeout")
    }
}
