//! D1 GT911 Touchscreen Driver
//!
//! Driver for the Goodix GT911 touchscreen controller on D1 platforms.
//! Uses simplified MMIO interface matching the emulator's d1_touch device.
//!
//! Thread-safe: All state is protected by a Spinlock, allowing any hart to poll.
//!
//! # Registers (emulator-specific MMIO at 0x0250_2000)
//! - 0x100: INT status (1 = touch event pending)
//! - 0x104: Touch status (bit 7 = data ready, bits 0-3 = touch count)
//! - 0x108: Touch X coordinate
//! - 0x10C: Touch Y coordinate
//! - 0x110: Touch point count
//! - 0x114: X resolution
//! - 0x118: Y resolution

use core::ptr::{read_volatile, write_volatile};
use crate::Spinlock;

// D1 I2C2 base (where GT911 touch controller is attached)
const D1_I2C2_BASE: usize = 0x0250_2000;

// Emulator-specific touch registers (simplified MMIO)
const TOUCH_INT_STATUS: usize = D1_I2C2_BASE + 0x100;
const TOUCH_STATUS: usize = D1_I2C2_BASE + 0x104;
const TOUCH_X: usize = D1_I2C2_BASE + 0x108;
const TOUCH_Y: usize = D1_I2C2_BASE + 0x10C;
const TOUCH_COUNT: usize = D1_I2C2_BASE + 0x110;
const TOUCH_X_RES: usize = D1_I2C2_BASE + 0x114;
const TOUCH_Y_RES: usize = D1_I2C2_BASE + 0x118;

// Event types (compatible with VirtIO Input / Linux evdev)
pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_ABS: u16 = 0x03;

// Absolute axis codes
pub const ABS_X: u16 = 0x00;
pub const ABS_Y: u16 = 0x01;

// Key codes (for touch buttons)
pub const BTN_TOUCH: u16 = 0x14A;
pub const BTN_LEFT: u16 = 0x110;   // Mouse left button (for compatibility)
pub const BTN_RIGHT: u16 = 0x111;  // Mouse right button (for compatibility)
pub const BTN_MIDDLE: u16 = 0x112; // Mouse middle button (for compatibility)

// Keyboard key codes (for compatibility with VirtIO Input replacement)
pub const KEY_UP: u16 = 103;
pub const KEY_DOWN: u16 = 108;
pub const KEY_LEFT: u16 = 105;
pub const KEY_RIGHT: u16 = 106;
pub const KEY_ENTER: u16 = 28;
pub const KEY_SPACE: u16 = 57;
pub const KEY_BACKSPACE: u16 = 14;
pub const KEY_ESC: u16 = 1;

/// Input event structure (compatible with VirtIO Input / evdev)
#[derive(Clone, Copy, Debug, Default)]
pub struct InputEvent {
    pub event_type: u16,
    pub code: u16,
    pub value: i32,
}

impl InputEvent {
    /// Check if this is a key press event (EV_KEY with value 1)
    pub fn is_key_press(&self) -> bool {
        self.event_type == EV_KEY && self.value == 1
    }
}

/// Touch driver state - protected by Spinlock for thread safety
struct TouchState {
    /// Whether touch is currently pressed
    pressed: bool,
    /// Last X coordinate (-1 = unset)
    last_x: i32,
    /// Last Y coordinate (-1 = unset)
    last_y: i32,
    /// Event queue (circular buffer)
    events: [Option<InputEvent>; 16],
    /// Queue head (next write position)
    head: usize,
    /// Queue tail (next read position)
    tail: usize,
    /// Total events processed (for debugging)
    event_count: u32,
}

impl TouchState {
    const fn new() -> Self {
        Self {
            pressed: false,
            last_x: -1,
            last_y: -1,
            events: [None; 16],
            head: 0,
            tail: 0,
            event_count: 0,
        }
    }

    fn push_event(&mut self, event: InputEvent) {
        let next = (self.head + 1) % 16;
        if next != self.tail {
            self.events[self.head] = Some(event);
            self.head = next;
        }
    }

    fn pop_event(&mut self) -> Option<InputEvent> {
        if self.tail == self.head {
            return None;
        }
        let event = self.events[self.tail];
        self.events[self.tail] = None;
        self.tail = (self.tail + 1) % 16;
        event
    }

    fn has_events(&self) -> bool {
        self.tail != self.head
    }
}

/// Global touch state protected by Spinlock
static TOUCH_STATE: Spinlock<TouchState> = Spinlock::new(TouchState::new());

/// Read a 32-bit register
fn read_reg(addr: usize) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

/// Write a 32-bit register
fn write_reg(addr: usize, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

/// Initialize the GT911 touchscreen driver
pub fn init() -> Result<(), &'static str> {
    // Read resolution from device
    let _x_res = read_reg(TOUCH_X_RES);
    let _y_res = read_reg(TOUCH_Y_RES);
    
    // Clear any pending interrupts
    write_reg(TOUCH_INT_STATUS, 0);
    write_reg(TOUCH_STATUS, 0);
    
    Ok(())
}

/// Poll for touch events and queue them
/// Thread-safe: can be called from any hart
pub fn poll() {
    let int_status = read_reg(TOUCH_INT_STATUS);
    if int_status == 0 {
        return; // No interrupt pending
    }
    
    let status = read_reg(TOUCH_STATUS);
    let data_ready = (status & 0x80) != 0;
    let touch_count = (status & 0x0F) as i32;
    
    if data_ready {
        let x = read_reg(TOUCH_X) as i32;
        let y = read_reg(TOUCH_Y) as i32;
        
        let mut state = TOUCH_STATE.lock();
        state.event_count = state.event_count.wrapping_add(1);
        
        if touch_count > 0 {
            // Touch is active
            // IMPORTANT: Send position events FIRST so UI has correct coords when handling button
            let is_new_touch = !state.pressed;
            
            if x != state.last_x || is_new_touch {
                state.last_x = x;
                state.push_event(InputEvent {
                    event_type: EV_ABS,
                    code: ABS_X,
                    value: x,
                });
            }
            if y != state.last_y || is_new_touch {
                state.last_y = y;
                state.push_event(InputEvent {
                    event_type: EV_ABS,
                    code: ABS_Y,
                    value: y,
                });
            }
            
            // Now send button press after position is set
            if is_new_touch {
                state.pressed = true;
                state.push_event(InputEvent {
                    event_type: EV_KEY,
                    code: BTN_TOUCH,
                    value: 1,
                });
            }
            
            // Sync event
            state.push_event(InputEvent {
                event_type: EV_SYN,
                code: 0,
                value: 0,
            });
        } else {
            // No touch - release if was pressed
            if state.pressed {
                state.pressed = false;
                state.push_event(InputEvent {
                    event_type: EV_KEY,
                    code: BTN_TOUCH,
                    value: 0,
                });
                state.push_event(InputEvent {
                    event_type: EV_SYN,
                    code: 0,
                    value: 0,
                });
            }
        }
        // Lock released here
        
        // Clear buffer ready flag
        write_reg(TOUCH_STATUS, 0);
    }
    
    // Clear interrupt
    write_reg(TOUCH_INT_STATUS, 0);
}

/// Get the number of touch events processed (for debugging)
pub fn get_event_count() -> u32 {
    TOUCH_STATE.lock().event_count
}

/// Get the next event from the queue
/// Thread-safe: can be called from any hart
pub fn next_event() -> Option<InputEvent> {
    TOUCH_STATE.lock().pop_event()
}

/// Check if there are pending events
pub fn has_events() -> bool {
    TOUCH_STATE.lock().has_events()
}
