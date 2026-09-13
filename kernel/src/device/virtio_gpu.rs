//! VirtIO GPU 2D: one scanout resource backed by the reserved framebuffer.
//!
//! Host present still scrapes guest DRAM. This driver is the portable
//! doorbell (RESOURCE_FLUSH) when a virtio-gpu device is attached.

use alloc::boxed::Box;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicBool, fence, Ordering};

use super::virtio::{self, LegacyQueueMem, VirtqDesc, VRING_DESC_F_NEXT, VRING_DESC_F_WRITE};

const QUEUE_SIZE: u16 = 8;
const RES_ID: u32 = 1;

const CMD_RESOURCE_CREATE_2D: u32 = 0x0101;
const CMD_SET_SCANOUT: u32 = 0x0103;
const CMD_RESOURCE_FLUSH: u32 = 0x0104;
const CMD_TRANSFER_TO_HOST_2D: u32 = 0x0105;
const CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;

const FORMAT_X8R8G8B8: u32 = 4;

#[repr(C)]
struct CtrlHdr {
    type_: u32,
    flags: u32,
    fence_id: u64,
    ctx_id: u32,
    padding: u32,
}

#[repr(C)]
struct Resp {
    hdr: CtrlHdr,
}

static AVAILABLE: AtomicBool = AtomicBool::new(false);
static mut GPU: Option<VirtioGpu> = None;

struct VirtioGpu {
    base: usize,
    queue: Box<LegacyQueueMem>,
    cmd: Box<[u8; 256]>,
    resp: Box<Resp>,
    avail_idx: u16,
    last_used: u16,
}

pub fn init() {
    if !virtio::is_io_hart() {
        return;
    }
    if let Some(base) = virtio::probe(virtio::DEVICE_GPU) {
        let mut g = VirtioGpu {
            base,
            queue: LegacyQueueMem::new(),
            cmd: Box::new([0u8; 256]),
            resp: Box::new(Resp {
                hdr: CtrlHdr {
                    type_: 0,
                    flags: 0,
                    fence_id: 0,
                    ctx_id: 0,
                    padding: 0,
                },
            }),
            avail_idx: 0,
            last_used: 0,
        };
        if g.setup().is_ok() && g.attach_scanout().is_ok() {
            unsafe {
                GPU = Some(g);
            }
            AVAILABLE.store(true, Ordering::Release);
        }
    }
}

pub fn is_available() -> bool {
    AVAILABLE.load(Ordering::Acquire)
}

/// Flush a dirty rectangle of the reserved FB through virtio-gpu.
pub fn flush_rect(x: u32, y: u32, w: u32, h: u32) {
    if !virtio::is_io_hart() || !is_available() {
        return;
    }
    unsafe {
        if let Some(ref mut g) = GPU {
            let _ = g.transfer_flush(x, y, w, h);
        }
    }
}

impl VirtioGpu {
    fn setup(&mut self) -> Result<(), &'static str> {
        virtio::reset(self.base);
        virtio::features_ok(self.base);
        if virtio::is_modern(self.base) {
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

    fn attach_scanout(&mut self) -> Result<(), &'static str> {
        let w = crate::platform::current::DISPLAY_WIDTH;
        let h = crate::platform::current::DISPLAY_HEIGHT;
        let stride = crate::platform::current::FB_STRIDE as u32;
        let fb = crate::platform::current::FB_ADDR as u64;
        let bytes = stride as u64 * h as u64;

        // RESOURCE_CREATE_2D
        self.write_hdr(CMD_RESOURCE_CREATE_2D);
        self.cmd[24..28].copy_from_slice(&RES_ID.to_le_bytes());
        self.cmd[28..32].copy_from_slice(&FORMAT_X8R8G8B8.to_le_bytes());
        self.cmd[32..36].copy_from_slice(&w.to_le_bytes());
        self.cmd[36..40].copy_from_slice(&h.to_le_bytes());
        self.submit(40)?;

        // ATTACH_BACKING
        self.write_hdr(CMD_RESOURCE_ATTACH_BACKING);
        self.cmd[24..28].copy_from_slice(&RES_ID.to_le_bytes());
        self.cmd[28..32].copy_from_slice(&1u32.to_le_bytes()); // nr_entries
        self.cmd[32..40].copy_from_slice(&fb.to_le_bytes());
        self.cmd[40..48].copy_from_slice(&bytes.to_le_bytes());
        self.cmd[48..56].copy_from_slice(&0u64.to_le_bytes());
        self.submit(56)?;

        // SET_SCANOUT
        self.write_hdr(CMD_SET_SCANOUT);
        // rect
        self.cmd[24..28].copy_from_slice(&0u32.to_le_bytes());
        self.cmd[28..32].copy_from_slice(&0u32.to_le_bytes());
        self.cmd[32..36].copy_from_slice(&w.to_le_bytes());
        self.cmd[36..40].copy_from_slice(&h.to_le_bytes());
        self.cmd[40..44].copy_from_slice(&0u32.to_le_bytes()); // scanout_id
        self.cmd[44..48].copy_from_slice(&RES_ID.to_le_bytes());
        self.submit(48)?;
        Ok(())
    }

    fn transfer_flush(&mut self, x: u32, y: u32, w: u32, h: u32) -> Result<(), &'static str> {
        self.write_hdr(CMD_TRANSFER_TO_HOST_2D);
        self.cmd[24..28].copy_from_slice(&x.to_le_bytes());
        self.cmd[28..32].copy_from_slice(&y.to_le_bytes());
        self.cmd[32..36].copy_from_slice(&w.to_le_bytes());
        self.cmd[36..40].copy_from_slice(&h.to_le_bytes());
        self.cmd[40..48].copy_from_slice(&0u64.to_le_bytes()); // offset
        self.cmd[48..52].copy_from_slice(&RES_ID.to_le_bytes());
        self.cmd[52..56].copy_from_slice(&0u32.to_le_bytes());
        self.submit(56)?;

        self.write_hdr(CMD_RESOURCE_FLUSH);
        self.cmd[24..28].copy_from_slice(&x.to_le_bytes());
        self.cmd[28..32].copy_from_slice(&y.to_le_bytes());
        self.cmd[32..36].copy_from_slice(&w.to_le_bytes());
        self.cmd[36..40].copy_from_slice(&h.to_le_bytes());
        self.cmd[40..44].copy_from_slice(&RES_ID.to_le_bytes());
        self.cmd[44..48].copy_from_slice(&0u32.to_le_bytes());
        self.submit(48)
    }

    fn write_hdr(&mut self, ty: u32) {
        self.cmd[..24].fill(0);
        self.cmd[0..4].copy_from_slice(&ty.to_le_bytes());
        self.resp.hdr.type_ = 0;
    }

    fn submit(&mut self, cmd_len: u32) -> Result<(), &'static str> {
        unsafe {
            let d = self.queue.desc();
            *d = VirtqDesc {
                addr: self.cmd.as_ptr() as u64,
                len: cmd_len,
                flags: VRING_DESC_F_NEXT,
                next: 1,
            };
            *d.add(1) = VirtqDesc {
                addr: &mut *self.resp as *mut Resp as u64,
                len: core::mem::size_of::<Resp>() as u32,
                flags: VRING_DESC_F_WRITE,
                next: 0,
            };
            let avail = self.queue.avail(QUEUE_SIZE as usize);
            let slot = (self.avail_idx % QUEUE_SIZE) as usize;
            *(avail.add(4 + slot * 2) as *mut u16) = 0;
            fence(Ordering::SeqCst);
            self.avail_idx = self.avail_idx.wrapping_add(1);
            write_volatile(avail.add(2) as *mut u16, self.avail_idx);
        }
        virtio::notify(self.base, 0);
        for _ in 0..100_000 {
            let used = self.queue.used();
            let idx = unsafe { read_volatile(used.add(2) as *const u16) };
            if idx != self.last_used {
                self.last_used = idx;
                virtio::ack_irq(self.base);
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err("virtio-gpu timeout")
    }
}
