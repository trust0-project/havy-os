//! VirtIO MMIO probe and queue setup (legacy QueuePFN + modern split).
//!
//! Device IDs: 1 net, 2 blk, 9 9p, 16 gpu, 18 input.

use alloc::boxed::Box;
use core::ptr::{read_volatile, write_volatile};

pub const MAGIC: u32 = 0x7472_6976;
pub const PAGE_SIZE: usize = 4096;

pub const MAGIC_VALUE: usize = 0x000;
pub const VERSION: usize = 0x004;
pub const DEVICE_ID: usize = 0x008;
pub const GUEST_PAGE_SIZE: usize = 0x028;
pub const QUEUE_SEL: usize = 0x030;
pub const QUEUE_NUM_MAX: usize = 0x034;
pub const QUEUE_NUM: usize = 0x038;
pub const QUEUE_ALIGN: usize = 0x03c;
pub const QUEUE_PFN: usize = 0x040;
pub const QUEUE_READY: usize = 0x044;
pub const QUEUE_NOTIFY: usize = 0x050;
pub const INTERRUPT_STATUS: usize = 0x060;
pub const INTERRUPT_ACK: usize = 0x064;
pub const STATUS: usize = 0x070;
pub const QUEUE_DESC_LOW: usize = 0x080;
pub const QUEUE_DESC_HIGH: usize = 0x084;
pub const QUEUE_DRIVER_LOW: usize = 0x090;
pub const QUEUE_DRIVER_HIGH: usize = 0x094;
pub const QUEUE_DEVICE_LOW: usize = 0x0a0;
pub const QUEUE_DEVICE_HIGH: usize = 0x0a4;
pub const CONFIG: usize = 0x100;

pub const STATUS_ACKNOWLEDGE: u32 = 1;
pub const STATUS_DRIVER: u32 = 2;
pub const STATUS_DRIVER_OK: u32 = 4;
pub const STATUS_FEATURES_OK: u32 = 8;

pub const VRING_DESC_F_NEXT: u16 = 1;
pub const VRING_DESC_F_WRITE: u16 = 2;

pub const DEVICE_NET: u32 = 1;
pub const DEVICE_BLK: u32 = 2;
pub const DEVICE_GPU: u32 = 16;
pub const DEVICE_INPUT: u32 = 18;

const VIRTIO_BASE: usize = 0x1000_1000;
const VIRTIO_STRIDE: usize = 0x1000;
const VIRTIO_SLOTS: usize = 8;

/// The VM exposes emulated devices only on the bootstrap hart's bus.
#[inline]
pub fn is_io_hart() -> bool {
    crate::get_hart_id() == 0
}

#[inline]
pub fn r32(base: usize, off: usize) -> u32 {
    unsafe { read_volatile((base + off) as *const u32) }
}

#[inline]
pub fn w32(base: usize, off: usize, val: u32) {
    unsafe { write_volatile((base + off) as *mut u32, val) }
}

pub fn is_device(base: usize, id: u32) -> bool {
    r32(base, MAGIC_VALUE) == MAGIC && r32(base, DEVICE_ID) == id
}

/// Find a virtio-mmio device by ID (DTB, then QEMU virt fallback slots).
pub fn probe(device_id: u32) -> Option<usize> {
    if !is_io_hart() {
        return None;
    }
    let nodes = crate::dtb::find_by_compatible("virtio,mmio");
    for n in &nodes {
        let base = n.reg_base as usize;
        if is_device(base, device_id) {
            return Some(base);
        }
    }
    if nodes.is_empty() {
        for i in 0..VIRTIO_SLOTS {
            let base = VIRTIO_BASE + i * VIRTIO_STRIDE;
            if is_device(base, device_id) {
                return Some(base);
            }
        }
    }
    None
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VirtqDesc {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct UsedElem {
    pub id: u32,
    pub len: u32,
}

/// Split virtqueue: desc / avail / used each live in their own aligned page
/// so both legacy PFN and modern 64-bit addresses work.
#[repr(C, align(4096))]
pub struct QueuePage {
    pub data: [u8; PAGE_SIZE],
}

impl QueuePage {
    pub fn new() -> Box<Self> {
        Box::new(Self {
            data: [0; PAGE_SIZE],
        })
    }
}

pub struct VirtQueue {
    pub desc: Box<QueuePage>,
    pub avail: Box<QueuePage>,
    pub used: Box<QueuePage>,
    pub size: u16,
    pub last_used: u16,
}

impl VirtQueue {
    pub fn new(size: u16) -> Self {
        Self {
            desc: QueuePage::new(),
            avail: QueuePage::new(),
            used: QueuePage::new(),
            size,
            last_used: 0,
        }
    }

    pub fn desc_ptr(&mut self) -> *mut VirtqDesc {
        self.desc.data.as_mut_ptr() as *mut VirtqDesc
    }

    pub fn avail_idx(&self) -> u16 {
        unsafe { read_volatile(self.avail.data.as_ptr().add(2) as *const u16) }
    }

    pub fn set_avail_idx(&mut self, v: u16) {
        unsafe { write_volatile(self.avail.data.as_mut_ptr().add(2) as *mut u16, v) }
    }

    pub fn avail_ring_slot(&mut self, slot: usize) -> *mut u16 {
        unsafe { self.avail.data.as_mut_ptr().add(4 + slot * 2) as *mut u16 }
    }

    pub fn used_idx(&self) -> u16 {
        unsafe { read_volatile(self.used.data.as_ptr().add(2) as *const u16) }
    }

    pub fn used_elem(&self, slot: usize) -> UsedElem {
        unsafe { read_volatile(self.used.data.as_ptr().add(4 + slot * 8) as *const UsedElem) }
    }

    pub fn write_desc(&mut self, i: usize, d: VirtqDesc) {
        unsafe { *self.desc_ptr().add(i) = d }
    }

    /// Attach this queue at `queue_sel` on a virtio-mmio device.
    pub fn attach(&mut self, base: usize, queue_sel: u32) {
        w32(base, QUEUE_SEL, queue_sel);
        let max = r32(base, QUEUE_NUM_MAX);
        let n = if max == 0 {
            self.size as u32
        } else {
            (self.size as u32).min(max)
        };
        self.size = n as u16;
        w32(base, QUEUE_NUM, n);

        let ver = r32(base, VERSION);
        if ver >= 2 {
            let desc = self.desc.data.as_ptr() as u64;
            let avail = self.avail.data.as_ptr() as u64;
            let used = self.used.data.as_ptr() as u64;
            w32(base, QUEUE_DESC_LOW, desc as u32);
            w32(base, QUEUE_DESC_HIGH, (desc >> 32) as u32);
            w32(base, QUEUE_DRIVER_LOW, avail as u32);
            w32(base, QUEUE_DRIVER_HIGH, (avail >> 32) as u32);
            w32(base, QUEUE_DEVICE_LOW, used as u32);
            w32(base, QUEUE_DEVICE_HIGH, (used >> 32) as u32);
            w32(base, QUEUE_READY, 1);
        } else {
            w32(base, GUEST_PAGE_SIZE, PAGE_SIZE as u32);
            w32(base, QUEUE_ALIGN, PAGE_SIZE as u32);
            // Legacy wants desc+avail in the first page, used in the next.
            // We already split pages; QueuePFN of the desc page is the
            // historical layout only if used follows. Copy used into
            // desc+PAGE via a 2-page region instead.
        }
    }
}

/// Two-page legacy queue (desc+avail, then used) for QueuePFN devices.
#[repr(C, align(4096))]
pub struct LegacyQueueMem {
    pub data: [u8; PAGE_SIZE * 2],
}

impl LegacyQueueMem {
    pub fn new() -> Box<Self> {
        Box::new(Self {
            data: [0; PAGE_SIZE * 2],
        })
    }

    pub fn pfn(&self) -> u32 {
        (self.data.as_ptr() as u64 / PAGE_SIZE as u64) as u32
    }

    pub fn desc(&mut self) -> *mut VirtqDesc {
        self.data.as_mut_ptr() as *mut VirtqDesc
    }

    pub fn avail(&mut self, queue_size: usize) -> *mut u8 {
        unsafe { self.data.as_mut_ptr().add(queue_size * 16) }
    }

    pub fn used(&self) -> *const u8 {
        unsafe { self.data.as_ptr().add(PAGE_SIZE) }
    }

    pub fn used_mut(&mut self) -> *mut u8 {
        unsafe { self.data.as_mut_ptr().add(PAGE_SIZE) }
    }
}

pub fn reset(base: usize) {
    w32(base, STATUS, 0);
    for _ in 0..1000 {
        core::hint::spin_loop();
    }
    w32(base, STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER);
}

pub fn features_ok(base: usize) {
    w32(
        base,
        STATUS,
        STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
    );
}

pub fn driver_ok(base: usize) {
    w32(
        base,
        STATUS,
        STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
    );
}

pub fn ack_irq(base: usize) {
    let s = r32(base, INTERRUPT_STATUS);
    if s != 0 {
        w32(base, INTERRUPT_ACK, s);
    }
}

pub fn notify(base: usize, queue: u32) {
    w32(base, QUEUE_NOTIFY, queue);
}

pub fn attach_legacy(base: usize, queue_sel: u32, queue_size: u16, pfn: u32) {
    w32(base, GUEST_PAGE_SIZE, PAGE_SIZE as u32);
    w32(base, QUEUE_SEL, queue_sel);
    w32(base, QUEUE_NUM, queue_size as u32);
    w32(base, QUEUE_ALIGN, PAGE_SIZE as u32);
    w32(base, QUEUE_PFN, pfn);
}

pub fn attach_modern(
    base: usize,
    queue_sel: u32,
    queue_size: u16,
    desc: u64,
    avail: u64,
    used: u64,
) {
    w32(base, QUEUE_SEL, queue_sel);
    w32(base, QUEUE_NUM, queue_size as u32);
    w32(base, QUEUE_DESC_LOW, desc as u32);
    w32(base, QUEUE_DESC_HIGH, (desc >> 32) as u32);
    w32(base, QUEUE_DRIVER_LOW, avail as u32);
    w32(base, QUEUE_DRIVER_HIGH, (avail >> 32) as u32);
    w32(base, QUEUE_DEVICE_LOW, used as u32);
    w32(base, QUEUE_DEVICE_HIGH, (used >> 32) as u32);
    w32(base, QUEUE_READY, 1);
}

pub fn is_modern(base: usize) -> bool {
    r32(base, VERSION) >= 2
}
