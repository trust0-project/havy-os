//! VirtIO net (device ID 1).

use alloc::boxed::Box;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

use crate::device::{NetworkDevice, NetworkError};

use super::virtio::{self, LegacyQueueMem, VirtqDesc, VRING_DESC_F_NEXT, VRING_DESC_F_WRITE};

const QUEUE_SIZE: u16 = 16;
const BUF_SIZE: usize = 2048;
const HDR_LEN: usize = 10;

#[repr(C)]
struct NetHdr {
    flags: u8,
    gso_type: u8,
    hdr_len: u16,
    gso_size: u16,
    csum_start: u16,
    csum_offset: u16,
}

pub struct VirtioNet {
    base: usize,
    rxq: Box<LegacyQueueMem>,
    txq: Box<LegacyQueueMem>,
    rx_bufs: Box<[[u8; BUF_SIZE]; QUEUE_SIZE as usize]>,
    tx_buf: Box<[u8; BUF_SIZE]>,
    tx_hdr: Box<NetHdr>,
    rx_last: u16,
    tx_avail: u16,
    mac: [u8; 6],
    modern: bool,
}

impl VirtioNet {
    pub fn probe() -> Option<Self> {
        let base = virtio::probe(virtio::DEVICE_NET)?;
        Some(Self {
            base,
            rxq: LegacyQueueMem::new(),
            txq: LegacyQueueMem::new(),
            rx_bufs: Box::new([[0u8; BUF_SIZE]; QUEUE_SIZE as usize]),
            tx_buf: Box::new([0u8; BUF_SIZE]),
            tx_hdr: Box::new(NetHdr {
                flags: 0,
                gso_type: 0,
                hdr_len: 0,
                gso_size: 0,
                csum_start: 0,
                csum_offset: 0,
            }),
            rx_last: 0,
            tx_avail: 0,
            mac: [0x52, 0x54, 0x00, 0x12, 0x34, 0x56],
            modern: virtio::is_modern(base),
        })
    }

    pub fn init(&mut self) -> Result<(), &'static str> {
        virtio::reset(self.base);
        virtio::features_ok(self.base);

        let mut mac = [0u8; 6];
        for (i, b) in mac.iter_mut().enumerate() {
            *b = unsafe { read_volatile((self.base + virtio::CONFIG + i) as *const u8) };
        }
        if mac.iter().any(|&b| b != 0) {
            self.mac = mac;
        }

        self.setup_rx();
        if self.modern {
            virtio::attach_modern(
                self.base,
                0,
                QUEUE_SIZE,
                self.rxq.data.as_ptr() as u64,
                self.rxq.data.as_ptr() as u64 + (QUEUE_SIZE as u64 * 16),
                self.rxq.data.as_ptr() as u64 + virtio::PAGE_SIZE as u64,
            );
            virtio::attach_modern(
                self.base,
                1,
                QUEUE_SIZE,
                self.txq.data.as_ptr() as u64,
                self.txq.data.as_ptr() as u64 + (QUEUE_SIZE as u64 * 16),
                self.txq.data.as_ptr() as u64 + virtio::PAGE_SIZE as u64,
            );
        } else {
            virtio::attach_legacy(self.base, 0, QUEUE_SIZE, self.rxq.pfn());
            virtio::attach_legacy(self.base, 1, QUEUE_SIZE, self.txq.pfn());
        }
        virtio::driver_ok(self.base);
        virtio::notify(self.base, 0);
        Ok(())
    }

    fn setup_rx(&mut self) {
        unsafe {
            let d = self.rxq.desc();
            let avail = self.rxq.avail(QUEUE_SIZE as usize);
            for i in 0..QUEUE_SIZE as usize {
                *d.add(i) = VirtqDesc {
                    addr: self.rx_bufs[i].as_ptr() as u64,
                    len: BUF_SIZE as u32,
                    flags: VRING_DESC_F_WRITE,
                    next: 0,
                };
                *(avail.add(4 + i * 2) as *mut u16) = i as u16;
            }
            write_volatile(avail.add(2) as *mut u16, QUEUE_SIZE);
        }
    }
}

impl NetworkDevice for VirtioNet {
    fn mac_address(&self) -> [u8; 6] {
        self.mac
    }

    fn link_up(&self) -> bool {
        true
    }

    fn transmit(&mut self, packet: &[u8]) -> Result<(), NetworkError> {
        if !virtio::is_io_hart() {
            return Err(NetworkError::NotReady);
        }
        let n = packet.len().min(BUF_SIZE - HDR_LEN);
        self.tx_buf[..HDR_LEN].fill(0);
        self.tx_buf[HDR_LEN..HDR_LEN + n].copy_from_slice(&packet[..n]);
        *self.tx_hdr = NetHdr {
            flags: 0,
            gso_type: 0,
            hdr_len: 0,
            gso_size: 0,
            csum_start: 0,
            csum_offset: 0,
        };

        unsafe {
            let d = self.txq.desc();
            *d = VirtqDesc {
                addr: &*self.tx_hdr as *const NetHdr as u64,
                len: HDR_LEN as u32,
                flags: VRING_DESC_F_NEXT,
                next: 1,
            };
            *d.add(1) = VirtqDesc {
                addr: self.tx_buf.as_ptr() as u64 + HDR_LEN as u64,
                len: n as u32,
                flags: 0,
                next: 0,
            };
            let avail = self.txq.avail(QUEUE_SIZE as usize);
            let slot = (self.tx_avail % QUEUE_SIZE) as usize;
            *(avail.add(4 + slot * 2) as *mut u16) = 0;
            fence(Ordering::SeqCst);
            self.tx_avail = self.tx_avail.wrapping_add(1);
            write_volatile(avail.add(2) as *mut u16, self.tx_avail);
        }
        virtio::notify(self.base, 1);
        Ok(())
    }

    fn receive(&mut self, buf: &mut [u8]) -> Result<usize, NetworkError> {
        if !virtio::is_io_hart() {
            return Err(NetworkError::NotReady);
        }
        let used = self.rxq.used();
        let idx = unsafe { read_volatile(used.add(2) as *const u16) };
        if idx == self.rx_last {
            return Err(NetworkError::NoPacket);
        }
        let slot = (self.rx_last % QUEUE_SIZE) as usize;
        let elem = unsafe { read_volatile(used.add(4 + slot * 8) as *const virtio::UsedElem) };
        let desc_id = elem.id as usize;
        let total = elem.len as usize;
        self.rx_last = self.rx_last.wrapping_add(1);

        if desc_id >= QUEUE_SIZE as usize || total <= HDR_LEN {
            self.repost_rx(desc_id);
            return Err(NetworkError::NoPacket);
        }
        let payload = total - HDR_LEN;
        let n = payload.min(buf.len());
        buf[..n].copy_from_slice(&self.rx_bufs[desc_id][HDR_LEN..HDR_LEN + n]);
        self.repost_rx(desc_id);
        virtio::ack_irq(self.base);
        Ok(n)
    }

    fn has_packet(&self) -> bool {
        if !virtio::is_io_hart() {
            return false;
        }
        let used = self.rxq.used();
        let idx = unsafe { read_volatile(used.add(2) as *const u16) };
        idx != self.rx_last
    }
}

impl VirtioNet {
    fn repost_rx(&mut self, desc_id: usize) {
        if desc_id >= QUEUE_SIZE as usize {
            return;
        }
        unsafe {
            let avail = self.rxq.avail(QUEUE_SIZE as usize);
            let idx = read_volatile(avail.add(2) as *const u16);
            let slot = (idx % QUEUE_SIZE) as usize;
            *(avail.add(4 + slot * 2) as *mut u16) = desc_id as u16;
            fence(Ordering::SeqCst);
            write_volatile(avail.add(2) as *mut u16, idx.wrapping_add(1));
        }
        virtio::notify(self.base, 0);
    }
}

#[allow(dead_code)]
pub fn create_device() -> Result<VirtioNet, NetworkError> {
    let mut n = VirtioNet::probe().ok_or(NetworkError::NotReady)?;
    n.init().map_err(|_| NetworkError::NotReady)?;
    Ok(n)
}
