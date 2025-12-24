//! VirtIO 9P (Plan 9 Filesystem) Driver
//!
//! This driver interfaces with the VirtIO 9P device (Device ID 9) to access
//! host-mounted directories via the 9P2000.L protocol.
//!
//! # Usage
//! ```no_run
//! use crate::device::virtio_p9;
//!
//! if let Some(mut driver) = virtio_p9::probe() {
//!     driver.init()?;
//!     // Now use driver.read_file(), driver.list_dir(), etc.
//! }
//! ```

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use crate::Spinlock;


// ═══════════════════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════════════════

/// VirtIO 9P Device ID
const VIRTIO_9P_DEVICE_ID: u32 = 9;

/// Maximum negotiated message size
const DEFAULT_MSIZE: u32 = 8192;

// 9P2000.L Message Types
const T_VERSION: u8 = 100;
const R_VERSION: u8 = 101;
const T_ATTACH: u8 = 104;
const R_ATTACH: u8 = 105;
const T_WALK: u8 = 110;
const R_WALK: u8 = 111;
const T_LOPEN: u8 = 12;
const R_LOPEN: u8 = 13;
const T_READ: u8 = 116;
const R_READ: u8 = 117;
const T_WRITE: u8 = 118;
const R_WRITE: u8 = 119;
const T_CLUNK: u8 = 120;
const R_CLUNK: u8 = 121;
const T_READDIR: u8 = 40;
const R_READDIR: u8 = 41;
const T_GETATTR: u8 = 24;
const R_GETATTR: u8 = 25;
const T_LCREATE: u8 = 14;
const R_LCREATE: u8 = 15;
const R_LERROR: u8 = 7;

// Linux open flags
const O_RDONLY: u32 = 0;
const O_WRONLY: u32 = 1;
const O_RDWR: u32 = 2;

// MMIO register offsets
const MAGIC_VALUE_OFFSET: usize = 0x000;
const VERSION_OFFSET: usize = 0x004;
const DEVICE_ID_OFFSET: usize = 0x008;
const STATUS_OFFSET: usize = 0x070;
const QUEUE_SEL_OFFSET: usize = 0x030;
const QUEUE_NUM_OFFSET: usize = 0x038;
const QUEUE_PFN_OFFSET: usize = 0x040;
const GUEST_PAGE_SIZE_OFFSET: usize = 0x028;
const QUEUE_NOTIFY_OFFSET: usize = 0x050;
const INTERRUPT_STATUS_OFFSET: usize = 0x060;
const INTERRUPT_ACK_OFFSET: usize = 0x064;
const CONFIG_OFFSET: usize = 0x100;

// Device status flags
const STATUS_ACKNOWLEDGE: u32 = 1;
const STATUS_DRIVER: u32 = 2;
const STATUS_DRIVER_OK: u32 = 4;
const STATUS_FEATURES_OK: u32 = 8;

// Queue constants
const PAGE_SIZE: usize = 4096;
const QUEUE_SIZE: u16 = 16;
const QUEUE_MEM_SIZE: usize = PAGE_SIZE * 2;

// ═══════════════════════════════════════════════════════════════════════════════
// VirtQueue Memory
// ═══════════════════════════════════════════════════════════════════════════════

/// Page-aligned queue memory for VirtIO descriptors
#[repr(C, align(4096))]
struct P9QueueMem {
    data: [u8; QUEUE_MEM_SIZE],
}

impl P9QueueMem {
    fn new() -> Box<Self> {
        Box::new(Self { data: [0; QUEUE_MEM_SIZE] })
    }
}

/// VirtIO descriptor structure
#[repr(C)]
#[derive(Clone, Copy)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

// ═══════════════════════════════════════════════════════════════════════════════
// 9P Driver
// ═══════════════════════════════════════════════════════════════════════════════

/// VirtIO 9P Driver
pub struct VirtioP9Driver {
    base: usize,
    queue_mem: Box<P9QueueMem>,
    /// Request buffer (send to device)
    request_buf: Box<[u8; DEFAULT_MSIZE as usize]>,
    /// Response buffer (receive from device)
    response_buf: Box<[u8; DEFAULT_MSIZE as usize]>,
    /// Negotiated message size
    msize: u32,
    /// Root FID (established during attach)
    root_fid: u32,
    /// Next available FID
    next_fid: AtomicU32,
    /// Last used ring index
    last_used_idx: u16,
    /// Message tag counter
    next_tag: AtomicU32,
    /// Whether the driver is initialized
    initialized: AtomicBool,
}

impl VirtioP9Driver {
    /// Probe for VirtIO 9P device using DTB discovery or fallback addresses
    pub fn probe() -> Option<Self> {
        // Try DTB discovery first
        let virtio_devices = crate::dtb::find_by_compatible("virtio,mmio");
        
        // Check each VirtIO device for 9P capability
        for device in &virtio_devices {
            let base = device.reg_base as usize;
            if Self::check_device_id(base, VIRTIO_9P_DEVICE_ID) {
                return Self::create_driver(base);
            }
        }
        
        // Fallback to legacy hardcoded addresses if DTB discovery didn't find anything
        if virtio_devices.is_empty() {
            const VIRTIO_BASE: usize = 0x1000_1000;
            const VIRTIO_STRIDE: usize = 0x1000;
            
            for i in 0..8 {
                let base = VIRTIO_BASE + i * VIRTIO_STRIDE;
                if Self::check_device_id(base, VIRTIO_9P_DEVICE_ID) {
                    return Self::create_driver(base);
                }
            }
        }
        
        None
    }
    
    /// Check if device at base address has matching device ID
    fn check_device_id(base: usize, expected_id: u32) -> bool {
        unsafe {
            let magic = core::ptr::read_volatile((base + MAGIC_VALUE_OFFSET) as *const u32);
            let device_id = core::ptr::read_volatile((base + DEVICE_ID_OFFSET) as *const u32);
            magic == 0x7472_6976 && device_id == expected_id
        }
    }
    
    /// Create driver instance for device at base address
    fn create_driver(base: usize) -> Option<Self> {
        let queue_mem = P9QueueMem::new();
        let request_buf = Box::new([0u8; DEFAULT_MSIZE as usize]);
        let response_buf = Box::new([0u8; DEFAULT_MSIZE as usize]);
        
        Some(Self {
            base,
            queue_mem,
            request_buf,
            response_buf,
            msize: DEFAULT_MSIZE,
            root_fid: 0,
            next_fid: AtomicU32::new(1),
            last_used_idx: 0,
            next_tag: AtomicU32::new(1),
            initialized: AtomicBool::new(false),
        })
    }

    /// Read the mount tag from config space
    pub fn read_mount_tag(&self) -> String {
        unsafe {
            // Config space layout: tag_len[2] + tag[...]
            let tag_len = core::ptr::read_volatile((self.base + CONFIG_OFFSET) as *const u16) as usize;
            let mut tag = Vec::with_capacity(tag_len.min(64));
            for i in 0..tag_len.min(64) {
                let byte = core::ptr::read_volatile((self.base + CONFIG_OFFSET + 2 + i) as *const u8);
                if byte == 0 { break; }
                tag.push(byte);
            }
            String::from_utf8(tag).unwrap_or_else(|_| String::from("unknown"))
        }
    }

    /// Initialize the 9P device
    pub fn init(&mut self) -> Result<(), &'static str> {
        unsafe {
            // Reset device
            core::ptr::write_volatile((self.base + STATUS_OFFSET) as *mut u32, 0);
            
            // Brief delay for reset
            for _ in 0..1000 {
                core::hint::spin_loop();
            }
            
            // Acknowledge + Driver
            core::ptr::write_volatile(
                (self.base + STATUS_OFFSET) as *mut u32,
                STATUS_ACKNOWLEDGE | STATUS_DRIVER
            );
            
            // Set guest page size
            core::ptr::write_volatile(
                (self.base + GUEST_PAGE_SIZE_OFFSET) as *mut u32,
                PAGE_SIZE as u32
            );
            
            // Select queue 0
            core::ptr::write_volatile((self.base + QUEUE_SEL_OFFSET) as *mut u32, 0);
            
            // Set queue size
            core::ptr::write_volatile((self.base + QUEUE_NUM_OFFSET) as *mut u32, QUEUE_SIZE as u32);
            
            // Set queue PFN
            let pfn = (self.queue_mem.data.as_ptr() as u64) / PAGE_SIZE as u64;
            core::ptr::write_volatile((self.base + QUEUE_PFN_OFFSET) as *mut u32, pfn as u32);
            
            // Features OK + Driver OK
            core::ptr::write_volatile(
                (self.base + STATUS_OFFSET) as *mut u32,
                STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK
            );
        }

        // Negotiate protocol version
        self.negotiate_version()?;
        
        // Attach to root
        self.attach()?;
        
        self.initialized.store(true, Ordering::Release);
        Ok(())
    }

    /// Get a new unique FID
    fn alloc_fid(&self) -> u32 {
        self.next_fid.fetch_add(1, Ordering::Relaxed)
    }

    /// Get a new unique message tag
    fn alloc_tag(&self) -> u16 {
        (self.next_tag.fetch_add(1, Ordering::Relaxed) & 0xFFFF) as u16
    }

    /// Send a message and receive response
    fn transact(&mut self, request: &[u8]) -> Result<&[u8], &'static str> {
        // Copy request to buffer
        let req_len = request.len().min(self.msize as usize);
        self.request_buf[..req_len].copy_from_slice(&request[..req_len]);
        
        // Setup descriptors
        let queue_mem_ptr = self.queue_mem.data.as_mut_ptr();
        let desc_table = queue_mem_ptr as *mut VirtqDesc;
        let avail_ring = unsafe { queue_mem_ptr.add(QUEUE_SIZE as usize * 16) };
        
        unsafe {
            // Descriptor 0: request (device reads)
            let desc0 = &mut *desc_table;
            desc0.addr = self.request_buf.as_ptr() as u64;
            desc0.len = req_len as u32;
            desc0.flags = 1; // VRING_DESC_F_NEXT
            desc0.next = 1;
            
            // Descriptor 1: response (device writes)
            let desc1 = &mut *desc_table.add(1);
            desc1.addr = self.response_buf.as_ptr() as u64;
            desc1.len = self.msize;
            desc1.flags = 2; // VRING_DESC_F_WRITE
            desc1.next = 0;
            
            // Add to available ring
            let avail_idx_ptr = avail_ring.add(2) as *mut u16;
            let avail_idx = core::ptr::read_volatile(avail_idx_ptr);
            let ring_slot = (avail_idx % QUEUE_SIZE) as usize;
            let ring_ptr = avail_ring.add(4 + ring_slot * 2) as *mut u16;
            *ring_ptr = 0; // First descriptor index
            core::sync::atomic::fence(Ordering::SeqCst);
            core::ptr::write_volatile(avail_idx_ptr, avail_idx.wrapping_add(1));
            
            // Notify device
            core::ptr::write_volatile((self.base + QUEUE_NOTIFY_OFFSET) as *mut u32, 0);
        }
        
        // Wait for response
        let used_ring = unsafe {
            let avail_ring_end = queue_mem_ptr.add(QUEUE_SIZE as usize * 16 + 6 + QUEUE_SIZE as usize * 2) as usize;
            let aligned = ((avail_ring_end + PAGE_SIZE - 1) / PAGE_SIZE) * PAGE_SIZE;
            aligned as *const u8
        };
        
        // Poll for completion
        for _ in 0..100_000 {
            let used_idx_ptr = unsafe { used_ring.add(2) as *const u16 };
            let current_used_idx = unsafe { core::ptr::read_volatile(used_idx_ptr) };
            
            if current_used_idx != self.last_used_idx {
                self.last_used_idx = current_used_idx;
                
                // Acknowledge interrupt
                unsafe {
                    let status = core::ptr::read_volatile((self.base + INTERRUPT_STATUS_OFFSET) as *const u32);
                    if status != 0 {
                        core::ptr::write_volatile((self.base + INTERRUPT_ACK_OFFSET) as *mut u32, status);
                    }
                }
                
                // Parse response
                if self.response_buf.len() >= 7 {
                    let resp_size = u32::from_le_bytes(self.response_buf[0..4].try_into().unwrap()) as usize;
                    let resp_type = self.response_buf[4];
                    
                    if resp_type == R_LERROR {
                        return Err("9P error response");
                    }
                    
                    return Ok(&self.response_buf[..resp_size.min(self.msize as usize)]);
                }
            }
            
            core::hint::spin_loop();
        }
        
        Err("9P transaction timeout")
    }

    /// Build a 9P message header
    fn build_header(&self, buf: &mut Vec<u8>, msg_type: u8, tag: u16) {
        buf.extend_from_slice(&0u32.to_le_bytes()); // Placeholder for size
        buf.push(msg_type);
        buf.extend_from_slice(&tag.to_le_bytes());
    }

    /// Finalize message by setting the size field
    fn finalize_message(&self, buf: &mut Vec<u8>) {
        let size = buf.len() as u32;
        buf[0..4].copy_from_slice(&size.to_le_bytes());
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // 9P Protocol Operations
    // ═══════════════════════════════════════════════════════════════════════════

    /// Negotiate protocol version (Tversion/Rversion)
    fn negotiate_version(&mut self) -> Result<(), &'static str> {
        let tag = self.alloc_tag();
        let version = b"9P2000.L";
        
        let mut req = Vec::with_capacity(32);
        self.build_header(&mut req, T_VERSION, tag);
        req.extend_from_slice(&self.msize.to_le_bytes());
        req.extend_from_slice(&(version.len() as u16).to_le_bytes());
        req.extend_from_slice(version);
        self.finalize_message(&mut req);
        
        let resp = self.transact(&req)?;
        
        if resp.len() >= 11 {
            let negotiated_msize = u32::from_le_bytes(resp[7..11].try_into().unwrap());
            self.msize = negotiated_msize.min(DEFAULT_MSIZE);
        }
        
        Ok(())
    }

    /// Attach to filesystem root (Tattach/Rattach)
    fn attach(&mut self) -> Result<(), &'static str> {
        let tag = self.alloc_tag();
        let fid = 0u32; // Root FID
        let afid = 0xFFFFFFFFu32; // No auth
        
        let mut req = Vec::with_capacity(64);
        self.build_header(&mut req, T_ATTACH, tag);
        req.extend_from_slice(&fid.to_le_bytes());
        req.extend_from_slice(&afid.to_le_bytes());
        req.extend_from_slice(&0u16.to_le_bytes()); // uname len
        req.extend_from_slice(&0u16.to_le_bytes()); // aname len
        req.extend_from_slice(&0u32.to_le_bytes()); // n_uname
        self.finalize_message(&mut req);
        
        let _ = self.transact(&req)?;
        self.root_fid = fid;
        
        Ok(())
    }

    /// Walk to a path (Twalk/Rwalk)
    pub fn walk(&mut self, path: &str) -> Result<u32, &'static str> {
        let tag = self.alloc_tag();
        let fid = self.root_fid;
        let new_fid = self.alloc_fid();
        
        // Split path into components
        let components: Vec<&str> = path.trim_start_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();
        
        let mut req = Vec::with_capacity(256);
        self.build_header(&mut req, T_WALK, tag);
        req.extend_from_slice(&fid.to_le_bytes());
        req.extend_from_slice(&new_fid.to_le_bytes());
        req.extend_from_slice(&(components.len() as u16).to_le_bytes());
        
        for name in &components {
            let name_bytes = name.as_bytes();
            req.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            req.extend_from_slice(name_bytes);
        }
        
        self.finalize_message(&mut req);
        let resp = self.transact(&req)?;
        
        // Check if walk succeeded: response contains nwqid[2] + qid*nwqid
        // nwqid must equal components.len() for a successful walk
        if resp.len() >= 9 {
            let nwqid = u16::from_le_bytes(resp[7..9].try_into().unwrap()) as usize;
            if nwqid != components.len() {
                // Walk failed - file or directory doesn't exist
                return Err("Path not found");
            }
        }
        
        Ok(new_fid)
    }

    /// Open a file (Tlopen/Rlopen)
    pub fn open(&mut self, fid: u32, flags: u32) -> Result<(), &'static str> {
        let tag = self.alloc_tag();
        
        let mut req = Vec::with_capacity(32);
        self.build_header(&mut req, T_LOPEN, tag);
        req.extend_from_slice(&fid.to_le_bytes());
        req.extend_from_slice(&flags.to_le_bytes());
        self.finalize_message(&mut req);
        
        let _ = self.transact(&req)?;
        Ok(())
    }

    /// Create a new file in a directory (Tlcreate/Rlcreate)
    /// 
    /// Takes the parent directory fid and creates a file with the given name.
    /// Returns the fid of the newly created file (reuses parent fid).
    pub fn lcreate(&mut self, dir_fid: u32, name: &str) -> Result<u32, &'static str> {
        let tag = self.alloc_tag();
        
        // Tlcreate: fid[4] + name[s] + flags[4] + mode[4] + gid[4]
        let name_bytes = name.as_bytes();
        let mut req = Vec::with_capacity(32 + name_bytes.len());
        self.build_header(&mut req, T_LCREATE, tag);
        req.extend_from_slice(&dir_fid.to_le_bytes());
        // String format: len[2] + data
        req.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        req.extend_from_slice(name_bytes);
        // flags: O_WRONLY | O_CREAT | O_TRUNC = 0x01 | 0x40 | 0x200 = 0x241
        let flags: u32 = 0x241;
        req.extend_from_slice(&flags.to_le_bytes());
        // mode: 0644 (rw-r--r--)
        let mode: u32 = 0o644;
        req.extend_from_slice(&mode.to_le_bytes());
        // gid: 0
        let gid: u32 = 0;
        req.extend_from_slice(&gid.to_le_bytes());
        self.finalize_message(&mut req);
        
        let _ = self.transact(&req)?;
        
        // Rlcreate reuses the same fid and returns qid + iounit
        Ok(dir_fid)
    }

    /// Read data from file (Tread/Rread)
    pub fn read(&mut self, fid: u32, offset: u64, count: u32) -> Result<Vec<u8>, &'static str> {
        let tag = self.alloc_tag();
        
        let mut req = Vec::with_capacity(32);
        self.build_header(&mut req, T_READ, tag);
        req.extend_from_slice(&fid.to_le_bytes());
        req.extend_from_slice(&offset.to_le_bytes());
        req.extend_from_slice(&count.to_le_bytes());
        self.finalize_message(&mut req);
        
        let resp = self.transact(&req)?;
        
        // Response: size[4] + type[1] + tag[2] + count[4] + data[count]
        if resp.len() >= 11 {
            let data_len = u32::from_le_bytes(resp[7..11].try_into().unwrap()) as usize;
            let data_end = (11 + data_len).min(resp.len());
            return Ok(resp[11..data_end].to_vec());
        }
        
        Ok(Vec::new())
    }

    /// Write data to file (Twrite/Rwrite)
    pub fn write(&mut self, fid: u32, offset: u64, data: &[u8]) -> Result<u32, &'static str> {
        let tag = self.alloc_tag();
        
        let mut req = Vec::with_capacity(32 + data.len());
        self.build_header(&mut req, T_WRITE, tag);
        req.extend_from_slice(&fid.to_le_bytes());
        req.extend_from_slice(&offset.to_le_bytes());
        req.extend_from_slice(&(data.len() as u32).to_le_bytes());
        req.extend_from_slice(data);
        self.finalize_message(&mut req);
        
        let resp = self.transact(&req)?;
        
        // Response: size[4] + type[1] + tag[2] + count[4]
        if resp.len() >= 11 {
            let written = u32::from_le_bytes(resp[7..11].try_into().unwrap());
            return Ok(written);
        }
        
        Ok(0)
    }

    /// Close a FID (Tclunk/Rclunk)
    pub fn clunk(&mut self, fid: u32) -> Result<(), &'static str> {
        let tag = self.alloc_tag();
        
        let mut req = Vec::with_capacity(16);
        self.build_header(&mut req, T_CLUNK, tag);
        req.extend_from_slice(&fid.to_le_bytes());
        self.finalize_message(&mut req);
        
        let _ = self.transact(&req)?;
        Ok(())
    }

    /// Read directory entries (Treaddir/Rreaddir)
    pub fn readdir(&mut self, fid: u32, offset: u64, count: u32) -> Result<Vec<DirEntry>, &'static str> {
        let tag = self.alloc_tag();
        
        let mut req = Vec::with_capacity(32);
        self.build_header(&mut req, T_READDIR, tag);
        req.extend_from_slice(&fid.to_le_bytes());
        req.extend_from_slice(&offset.to_le_bytes());
        req.extend_from_slice(&count.to_le_bytes());
        self.finalize_message(&mut req);
        
        let resp = self.transact(&req)?;
        
        // Parse directory entries
        let mut entries = Vec::new();
        
        if resp.len() >= 11 {
            let data_len = u32::from_le_bytes(resp[7..11].try_into().unwrap()) as usize;
            let data = &resp[11..(11 + data_len).min(resp.len())];
            
            let mut i = 0;
            while i + 24 <= data.len() {
                // dirent format: qid[13] + offset[8] + type[1] + name[s]
                let qtype = data[i]; // First byte of QID
                i += 13; // Skip QID
                i += 8; // Skip offset
                i += 1; // Skip type
                
                if i + 2 > data.len() { break; }
                let name_len = u16::from_le_bytes(data[i..i+2].try_into().unwrap()) as usize;
                i += 2;
                
                if i + name_len > data.len() { break; }
                let name = core::str::from_utf8(&data[i..i+name_len])
                    .unwrap_or("")
                    .to_string();
                i += name_len;
                
                entries.push(DirEntry {
                    name,
                    is_dir: qtype & 0x80 != 0,
                });
            }
        }
        
        Ok(entries)
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // High-Level File Operations
    // ═══════════════════════════════════════════════════════════════════════════

    /// Read an entire file by path
    pub fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        // Walk to file
        let fid = self.walk(path).ok()?;
        
        // Open for reading
        if self.open(fid, O_RDONLY).is_err() {
            let _ = self.clunk(fid);
            return None;
        }
        
        // Read file contents
        let mut data = Vec::new();
        let mut offset = 0u64;
        let chunk_size = (self.msize - 100).min(4096);
        
        loop {
            match self.read(fid, offset, chunk_size) {
                Ok(chunk) if !chunk.is_empty() => {
                    offset += chunk.len() as u64;
                    data.extend_from_slice(&chunk);
                }
                _ => break,
            }
        }
        
        // Close
        let _ = self.clunk(fid);
        
        Some(data)
    }

    /// List directory contents by path
    pub fn list_dir(&mut self, path: &str) -> Vec<DirEntry> {
        // Walk to directory
        let fid = match self.walk(path) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };
        
        // Open directory
        if self.open(fid, O_RDONLY).is_err() {
            let _ = self.clunk(fid);
            return Vec::new();
        }
        
        // Read directory entries
        let mut entries = Vec::new();
        let mut offset = 0u64;
        
        loop {
            match self.readdir(fid, offset, 4096) {
                Ok(batch) if !batch.is_empty() => {
                    offset += batch.len() as u64;
                    entries.extend(batch);
                }
                _ => break,
            }
        }
        
        // Close
        let _ = self.clunk(fid);
        
        entries
    }
}

/// Directory entry from readdir
#[derive(Clone, Debug)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

// ═══════════════════════════════════════════════════════════════════════════════
// Global Driver Instance
// ═══════════════════════════════════════════════════════════════════════════════

static mut P9_DRIVER: Option<VirtioP9Driver> = None;

/// Initialize the 9P driver
pub fn init() -> Result<(), &'static str> {
    if let Some(mut driver) = VirtioP9Driver::probe() {
        let tag = driver.read_mount_tag();
        driver.init()?;
        unsafe {
            P9_DRIVER = Some(driver);
        }
        Ok(())
    } else {
        Err("VirtIO 9P device not found")
    }
}

/// Check if 9P driver is available
pub fn is_available() -> bool {
    unsafe { P9_DRIVER.is_some() }
}

/// Read a file from host mount
pub fn read_file(path: &str) -> Option<Vec<u8>> {
    unsafe {
        P9_DRIVER.as_mut().and_then(|d| d.read_file(path))
    }
}

/// List directory from host mount
pub fn list_dir(path: &str) -> Vec<DirEntry> {
    unsafe {
        P9_DRIVER.as_mut().map(|d| d.list_dir(path)).unwrap_or_default()
    }
}
