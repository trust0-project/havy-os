//! VirtIO Input Driver for Guest Kernel
//!
//! This driver interfaces with the VirtIO Input device (Device ID 18) to receive
//! keyboard events from the host.

use core::sync::atomic::{AtomicBool, Ordering};
use alloc::collections::VecDeque;

/// VirtIO Input Device ID
const VIRTIO_INPUT_DEVICE_ID: u32 = 18;

/// MMIO register offsets
const MAGIC_VALUE_OFFSET: usize = 0x000;
const DEVICE_ID_OFFSET: usize = 0x008;
const STATUS_OFFSET: usize = 0x070;
const QUEUE_NOTIFY_OFFSET: usize = 0x050;
const INTERRUPT_STATUS_OFFSET: usize = 0x060;
const INTERRUPT_ACK_OFFSET: usize = 0x064;

// Device status flags
const STATUS_ACKNOWLEDGE: u32 = 1;
const STATUS_DRIVER: u32 = 2;
const STATUS_DRIVER_OK: u32 = 4;
const STATUS_FEATURES_OK: u32 = 8;

// Linux input event types
pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;

// Common key codes (Linux input.h compatible)
pub const KEY_ESC: u16 = 1;
pub const KEY_1: u16 = 2;
pub const KEY_2: u16 = 3;
pub const KEY_3: u16 = 4;
pub const KEY_4: u16 = 5;
pub const KEY_5: u16 = 6;
pub const KEY_6: u16 = 7;
pub const KEY_7: u16 = 8;
pub const KEY_8: u16 = 9;
pub const KEY_9: u16 = 10;
pub const KEY_0: u16 = 11;
pub const KEY_BACKSPACE: u16 = 14;
pub const KEY_TAB: u16 = 15;
pub const KEY_Q: u16 = 16;
pub const KEY_W: u16 = 17;
pub const KEY_E: u16 = 18;
pub const KEY_R: u16 = 19;
pub const KEY_T: u16 = 20;
pub const KEY_Y: u16 = 21;
pub const KEY_U: u16 = 22;
pub const KEY_I: u16 = 23;
pub const KEY_O: u16 = 24;
pub const KEY_P: u16 = 25;
pub const KEY_ENTER: u16 = 28;
pub const KEY_A: u16 = 30;
pub const KEY_S: u16 = 31;
pub const KEY_D: u16 = 32;
pub const KEY_F: u16 = 33;
pub const KEY_G: u16 = 34;
pub const KEY_H: u16 = 35;
pub const KEY_J: u16 = 36;
pub const KEY_K: u16 = 37;
pub const KEY_L: u16 = 38;
pub const KEY_Z: u16 = 44;
pub const KEY_X: u16 = 45;
pub const KEY_C: u16 = 46;
pub const KEY_V: u16 = 47;
pub const KEY_B: u16 = 48;
pub const KEY_N: u16 = 49;
pub const KEY_M: u16 = 50;
pub const KEY_SPACE: u16 = 57;
pub const KEY_UP: u16 = 103;
pub const KEY_LEFT: u16 = 105;
pub const KEY_RIGHT: u16 = 106;
pub const KEY_DOWN: u16 = 108;

/// Input event structure (8 bytes, matches VirtIO input event)
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct InputEvent {
    pub event_type: u16,
    pub code: u16,
    pub value: i32,
}

impl InputEvent {
    /// Check if this is a key press event
    pub fn is_key_press(&self) -> bool {
        self.event_type == EV_KEY && self.value == 1
    }

    /// Check if this is a key release event
    pub fn is_key_release(&self) -> bool {
        self.event_type == EV_KEY && self.value == 0
    }
}

// Queue constants
const PAGE_SIZE: usize = 4096;
const QUEUE_SIZE: u16 = 64;
const QUEUE_MEM_SIZE: usize = PAGE_SIZE * 2;

// VirtIO MMIO register offsets
const GUEST_PAGE_SIZE_OFFSET: usize = 0x028;
const QUEUE_SEL_OFFSET: usize = 0x030;
const QUEUE_NUM_OFFSET: usize = 0x038;
const QUEUE_PFN_OFFSET: usize = 0x040;
const QUEUE_NOTIFY_OFFSET_ALT: usize = 0x050;

/// Page-aligned queue memory for VirtIO descriptors
#[repr(C, align(4096))]
struct InputQueueMem {
    data: [u8; QUEUE_MEM_SIZE],
}

impl InputQueueMem {
    fn new() -> alloc::boxed::Box<Self> {
        alloc::boxed::Box::new(Self {
            data: [0; QUEUE_MEM_SIZE],
        })
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

/// Input driver with proper virtqueue support
pub struct InputDriver {
    base: usize,
    /// Heap-allocated queue memory
    queue_mem: alloc::boxed::Box<InputQueueMem>,
    /// Event buffers (heap-allocated to ensure DRAM address range)
    event_buffers: alloc::boxed::Box<[InputEvent; QUEUE_SIZE as usize]>,
    /// Parsed events ready for consumption
    event_queue: VecDeque<InputEvent>,
    /// Last processed used ring index
    last_used_idx: u16,
    initialized: AtomicBool,
}

impl InputDriver {
    /// Probe for VirtIO Input device at potential base addresses
    pub fn probe() -> Option<Self> {
        const VIRTIO_BASE: usize = 0x1000_1000;
        const VIRTIO_STRIDE: usize = 0x1000;

        for i in 0..8 {
            let base = VIRTIO_BASE + i * VIRTIO_STRIDE;
            unsafe {
                let magic = core::ptr::read_volatile((base + MAGIC_VALUE_OFFSET) as *const u32);
                let device_id = core::ptr::read_volatile((base + DEVICE_ID_OFFSET) as *const u32);
                
                if magic == 0x7472_6976 && device_id == VIRTIO_INPUT_DEVICE_ID {
                    let queue_mem = InputQueueMem::new();
                    let event_buffers = alloc::boxed::Box::new(
                        [InputEvent { event_type: 0, code: 0, value: 0 }; QUEUE_SIZE as usize]
                    );
                    return Some(Self {
                        base,
                        queue_mem,
                        event_buffers,
                        event_queue: VecDeque::with_capacity(32),
                        last_used_idx: 0,
                        initialized: AtomicBool::new(false),
                    });
                }
            }
        }
        None
    }

    /// Initialize the input device with proper virtqueue setup
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
            
            // Select queue 0 (event queue)
            core::ptr::write_volatile(
                (self.base + QUEUE_SEL_OFFSET) as *mut u32,
                0
            );
            
            // Set queue size
            core::ptr::write_volatile(
                (self.base + QUEUE_NUM_OFFSET) as *mut u32,
                QUEUE_SIZE as u32
            );
            
            // Set queue PFN (page frame number)
            let pfn = (self.queue_mem.data.as_ptr() as u64) / PAGE_SIZE as u64;
            core::ptr::write_volatile(
                (self.base + QUEUE_PFN_OFFSET) as *mut u32,
                pfn as u32
            );
            
            // Setup descriptors - point each to an event buffer
            self.setup_descriptors();
            
            // Features OK + Driver OK
            core::ptr::write_volatile(
                (self.base + STATUS_OFFSET) as *mut u32,
                STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK
            );
            
            // Notify device that buffers are available
            core::ptr::write_volatile(
                (self.base + QUEUE_NOTIFY_OFFSET_ALT) as *mut u32,
                0
            );
            
            self.initialized.store(true, Ordering::Release);
        }
        Ok(())
    }
    
    /// Setup descriptors pointing to event buffers
    fn setup_descriptors(&mut self) {
        let queue_mem_ptr = self.queue_mem.data.as_mut_ptr();
        
        // Descriptor table is at the start of queue memory
        let desc_table = queue_mem_ptr as *mut VirtqDesc;
        
        // Available ring is after descriptors (16 bytes each)
        let avail_ring = unsafe { queue_mem_ptr.add(QUEUE_SIZE as usize * 16) };
        

        
        // Setup each descriptor to point to an event buffer
        for i in 0..QUEUE_SIZE as usize {
            unsafe {
                let desc = &mut *desc_table.add(i);
                desc.addr = &self.event_buffers[i] as *const InputEvent as u64;
                desc.len = 8; // sizeof(InputEvent)
                desc.flags = 2; // VRING_DESC_F_WRITE - device can write
                desc.next = 0;
                
                // Add to available ring
                let avail_idx_ptr = avail_ring.add(2) as *mut u16;
                let ring_ptr = avail_ring.add(4 + i * 2) as *mut u16;
                *ring_ptr = i as u16;
                *avail_idx_ptr = (i + 1) as u16;
            }
        }
    }

    /// Poll for new input events from the device
    pub fn poll(&mut self) {
        if !self.initialized.load(Ordering::Acquire) {
            return;
        }
        
        let queue_mem_ptr = self.queue_mem.data.as_ptr();
        
        // Used ring is after available ring (page-aligned in legacy mode)
        // For legacy: used ring at aligned boundary after avail ring
        let used_ring = unsafe { 
            let avail_ring_end = queue_mem_ptr.add(QUEUE_SIZE as usize * 16 + 6 + QUEUE_SIZE as usize * 2);
            // Align to page boundary for used ring
            let aligned = ((avail_ring_end as usize + PAGE_SIZE - 1) / PAGE_SIZE) * PAGE_SIZE;
            aligned as *const u8
        };
        
        // Read current used index from device
        let used_idx_ptr = unsafe { used_ring.add(2) as *const u16 };
        let current_used_idx = unsafe { core::ptr::read_volatile(used_idx_ptr) };

        
        // Process new used entries
        while self.last_used_idx != current_used_idx {
            let ring_idx = (self.last_used_idx % QUEUE_SIZE) as usize;
            
            // Read used ring element (8 bytes: id + len)
            let used_elem_ptr = unsafe { used_ring.add(4 + ring_idx * 8) as *const u32 };
            let desc_id = unsafe { core::ptr::read_volatile(used_elem_ptr) } as usize;
            
            if desc_id < QUEUE_SIZE as usize {
                // Debug: dump raw bytes from buffer to see layout
                let buf_ptr = &self.event_buffers[desc_id] as *const InputEvent as *const u8;
                let mut bytes = [0u8; 8];
                for i in 0..8 {
                    bytes[i] = unsafe { core::ptr::read_volatile(buf_ptr.add(i)) };
                }

                
                // Read event directly from the buffer address using volatile
                // (read from DRAM where host wrote, not from Rust array)
                let event = unsafe { core::ptr::read_volatile(&self.event_buffers[desc_id] as *const InputEvent) };
                
                // Only queue key events (filter out SYN etc)
                if event.event_type == EV_KEY {
                    self.event_queue.push_back(event);
                }
                
                // Re-add descriptor to available ring
                self.readd_descriptor(desc_id as u16);
            }
            
            self.last_used_idx = self.last_used_idx.wrapping_add(1);
        }
        
        // Acknowledge any interrupt
        unsafe {
            let status = core::ptr::read_volatile((self.base + INTERRUPT_STATUS_OFFSET) as *const u32);
            if status != 0 {
                core::ptr::write_volatile((self.base + INTERRUPT_ACK_OFFSET) as *mut u32, status);
            }
        }
    }
    
    /// Re-add a descriptor to the available ring
    fn readd_descriptor(&mut self, desc_id: u16) {
        let queue_mem_ptr = self.queue_mem.data.as_mut_ptr();
        let avail_ring = unsafe { queue_mem_ptr.add(QUEUE_SIZE as usize * 16) };
        
        unsafe {
            let avail_idx_ptr = avail_ring.add(2) as *mut u16;
            let avail_idx = core::ptr::read_volatile(avail_idx_ptr);
            let ring_slot = (avail_idx % QUEUE_SIZE) as usize;
            let ring_ptr = avail_ring.add(4 + ring_slot * 2) as *mut u16;
            *ring_ptr = desc_id;
            core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
            core::ptr::write_volatile(avail_idx_ptr, avail_idx.wrapping_add(1));
            
            // Notify device
            core::ptr::write_volatile((self.base + QUEUE_NOTIFY_OFFSET_ALT) as *mut u32, 0);
        }
    }

    /// Get the next pending input event
    pub fn next_event(&mut self) -> Option<InputEvent> {
        self.event_queue.pop_front()
    }

    /// Check if there are pending events
    pub fn has_events(&self) -> bool {
        !self.event_queue.is_empty()
    }
}

/// Global Input driver instance
static mut INPUT_DRIVER: Option<InputDriver> = None;

/// Initialize the global input driver
pub fn init() -> Result<(), &'static str> {
    if let Some(mut input) = InputDriver::probe() {
        input.init()?;
        unsafe {
            INPUT_DRIVER = Some(input);
        }
        Ok(())
    } else {
        Err("VirtIO Input device not found")
    }
}

/// Poll for input events
pub fn poll() {
    unsafe {
        if let Some(ref mut input) = INPUT_DRIVER {
            input.poll();
        }
    }
}

/// Get the next input event
pub fn next_event() -> Option<InputEvent> {
    unsafe {
        INPUT_DRIVER.as_mut().and_then(|i| i.next_event())
    }
}

/// Check if input is available
pub fn is_available() -> bool {
    unsafe { INPUT_DRIVER.is_some() }
}

/// Convert Linux key code to ASCII character (for simple text input)
pub fn key_to_char(code: u16, shift: bool) -> Option<char> {
    match code {
        KEY_1 => Some(if shift { '!' } else { '1' }),
        KEY_2 => Some(if shift { '@' } else { '2' }),
        KEY_3 => Some(if shift { '#' } else { '3' }),
        KEY_4 => Some(if shift { '$' } else { '4' }),
        KEY_5 => Some(if shift { '%' } else { '5' }),
        KEY_6 => Some(if shift { '^' } else { '6' }),
        KEY_7 => Some(if shift { '&' } else { '7' }),
        KEY_8 => Some(if shift { '*' } else { '8' }),
        KEY_9 => Some(if shift { '(' } else { '9' }),
        KEY_0 => Some(if shift { ')' } else { '0' }),
        KEY_Q => Some(if shift { 'Q' } else { 'q' }),
        KEY_W => Some(if shift { 'W' } else { 'w' }),
        KEY_E => Some(if shift { 'E' } else { 'e' }),
        KEY_R => Some(if shift { 'R' } else { 'r' }),
        KEY_T => Some(if shift { 'T' } else { 't' }),
        KEY_Y => Some(if shift { 'Y' } else { 'y' }),
        KEY_U => Some(if shift { 'U' } else { 'u' }),
        KEY_I => Some(if shift { 'I' } else { 'i' }),
        KEY_O => Some(if shift { 'O' } else { 'o' }),
        KEY_P => Some(if shift { 'P' } else { 'p' }),
        KEY_A => Some(if shift { 'A' } else { 'a' }),
        KEY_S => Some(if shift { 'S' } else { 's' }),
        KEY_D => Some(if shift { 'D' } else { 'd' }),
        KEY_F => Some(if shift { 'F' } else { 'f' }),
        KEY_G => Some(if shift { 'G' } else { 'g' }),
        KEY_H => Some(if shift { 'H' } else { 'h' }),
        KEY_J => Some(if shift { 'J' } else { 'j' }),
        KEY_K => Some(if shift { 'K' } else { 'k' }),
        KEY_L => Some(if shift { 'L' } else { 'l' }),
        KEY_Z => Some(if shift { 'Z' } else { 'z' }),
        KEY_X => Some(if shift { 'X' } else { 'x' }),
        KEY_C => Some(if shift { 'C' } else { 'c' }),
        KEY_V => Some(if shift { 'V' } else { 'v' }),
        KEY_B => Some(if shift { 'B' } else { 'b' }),
        KEY_N => Some(if shift { 'N' } else { 'n' }),
        KEY_M => Some(if shift { 'M' } else { 'm' }),
        KEY_SPACE => Some(' '),
        KEY_ENTER => Some('\n'),
        KEY_TAB => Some('\t'),
        _ => None,
    }
}
