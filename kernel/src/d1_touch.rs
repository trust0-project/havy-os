//! D1 GT911 Touchscreen Driver
//!
//! Driver for the Goodix GT911 touchscreen controller on D1 platforms.
//! Uses simplified MMIO interface matching the emulator's d1_touch device.
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

/// Touch state
static mut TOUCH_PRESSED: bool = false;
static mut LAST_X: i32 = 0;
static mut LAST_Y: i32 = 0;
static mut EVENTS: [Option<InputEvent>; 16] = [None; 16];
static mut EVENT_HEAD: usize = 0;
static mut EVENT_TAIL: usize = 0;

/// Read a 32-bit register
fn read_reg(addr: usize) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

/// Write a 32-bit register
fn write_reg(addr: usize, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

/// Push an event to the event queue
fn push_event(event: InputEvent) {
    unsafe {
        let next = (EVENT_HEAD + 1) % 16;
        if next != EVENT_TAIL {
            EVENTS[EVENT_HEAD] = Some(event);
            EVENT_HEAD = next;
        }
    }
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
        
        unsafe {
            if touch_count > 0 {
                // Touch is active
                // IMPORTANT: Send position events FIRST so UI has correct coords when handling button
                if x != LAST_X {
                    LAST_X = x;
                    push_event(InputEvent {
                        event_type: EV_ABS,
                        code: ABS_X,
                        value: x,
                    });
                }
                if y != LAST_Y {
                    LAST_Y = y;
                    push_event(InputEvent {
                        event_type: EV_ABS,
                        code: ABS_Y,
                        value: y,
                    });
                }
                
                // Now send button press after position is set
                if !TOUCH_PRESSED {
                    // New touch - send BTN_TOUCH press
                    TOUCH_PRESSED = true;
                    push_event(InputEvent {
                        event_type: EV_KEY,
                        code: BTN_TOUCH,
                        value: 1,
                    });
                }
                
                // Sync event
                push_event(InputEvent {
                    event_type: EV_SYN,
                    code: 0,
                    value: 0,
                });
            } else {
                // No touch - release if was pressed
                if TOUCH_PRESSED {
                    TOUCH_PRESSED = false;
                    push_event(InputEvent {
                        event_type: EV_KEY,
                        code: BTN_TOUCH,
                        value: 0,
                    });
                    push_event(InputEvent {
                        event_type: EV_SYN,
                        code: 0,
                        value: 0,
                    });
                }
            }
        }
        
        // Clear buffer ready flag
        write_reg(TOUCH_STATUS, 0);
    }
    
    // Clear interrupt
    write_reg(TOUCH_INT_STATUS, 0);
}

/// Get the next event from the queue
pub fn next_event() -> Option<InputEvent> {
    unsafe {
        if EVENT_TAIL == EVENT_HEAD {
            return None;
        }
        
        let event = EVENTS[EVENT_TAIL];
        EVENTS[EVENT_TAIL] = None;
        EVENT_TAIL = (EVENT_TAIL + 1) % 16;
        event
    }
}

/// Check if there are pending events
pub fn has_events() -> bool {
    unsafe { EVENT_TAIL != EVENT_HEAD }
}
