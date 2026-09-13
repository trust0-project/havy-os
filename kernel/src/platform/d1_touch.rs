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
use core::sync::atomic::{AtomicBool, Ordering};
use crate::Spinlock;
use crate::input::InputEvent;

pub use crate::input::{
    EV_SYN, EV_KEY, EV_ABS, EV_CHAR, ABS_X, ABS_Y, BTN_TOUCH, BTN_LEFT, BTN_RIGHT, BTN_MIDDLE,
    KEY_UP, KEY_DOWN, KEY_LEFT, KEY_RIGHT, KEY_ENTER, KEY_SPACE, KEY_BACKSPACE, KEY_ESC,
};

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

// Keyboard MMIO registers (emulator-specific)
const KEY_COUNT: usize = D1_I2C2_BASE + 0x11C;   // Number of pending keyboard events
const KEY_CODE: usize = D1_I2C2_BASE + 0x120;    // Key code (Linux evdev)
const KEY_STATE: usize = D1_I2C2_BASE + 0x124;   // Key state (1=pressed, 0=released)

// Character queue MMIO registers (for typed characters respecting keyboard layout)
const CHAR_COUNT: usize = D1_I2C2_BASE + 0x128;  // Number of pending characters
const CHAR_CODE: usize = D1_I2C2_BASE + 0x12C;   // Character ASCII code

// Event types come from crate::input.

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

/// Whether FT6336U answered on TWI (hardware). Emulator uses MMIO.
static USE_TWI: AtomicBool = AtomicBool::new(false);

/// Initialize the GT911/FT6336U touchscreen driver
pub fn init() -> Result<(), &'static str> {
    if twi_probe_ft6336() {
        USE_TWI.store(true, Ordering::Release);
        return Ok(());
    }
    let _x_res = read_reg(TOUCH_X_RES);
    let _y_res = read_reg(TOUCH_Y_RES);
    write_reg(TOUCH_INT_STATUS, 0);
    write_reg(TOUCH_STATUS, 0);
    Ok(())
}

/// Poll for touch and keyboard events and queue them
/// Thread-safe: can be called from any hart
pub fn poll() {
    if USE_TWI.load(Ordering::Acquire) {
        ft6336_poll();
        return;
    }
    // First, poll for typed characters (respects keyboard layout)
    // These are used for Terminal window text input
    loop {
        let char_count = read_reg(CHAR_COUNT);
        if char_count == 0 {
            break;
        }
        
        // Read the character
        let char_code = read_reg(CHAR_CODE) as u8;
        
        let mut state = TOUCH_STATE.lock();
        state.event_count = state.event_count.wrapping_add(1);
        // Use a special event type (0x10) to indicate this is a typed character
        // This is a custom extension - UI code should check for this
        state.push_event(InputEvent {
            event_type: EV_CHAR,  // Custom event type for characters
            code: char_code as u16,
            value: 1,  // Always "pressed" for characters
        });
        state.push_event(InputEvent {
            event_type: EV_SYN,
            code: 0,
            value: 0,
        });
        drop(state);
        
        // Acknowledge the character (consume from queue)
        write_reg(CHAR_COUNT, 0);
    }
    
    // Second, poll for keyboard events (raw keycodes - for Enter, Backspace, etc.)
    loop {
        let key_count = read_reg(KEY_COUNT);
        if key_count == 0 {
            break;
        }
        
        // Read the key event
        let key_code = read_reg(KEY_CODE) as u16;
        let key_state = read_reg(KEY_STATE) as i32;
        
        let mut state = TOUCH_STATE.lock();
        state.event_count = state.event_count.wrapping_add(1);
        state.push_event(InputEvent {
            event_type: EV_KEY,
            code: key_code,
            value: key_state,
        });
        state.push_event(InputEvent {
            event_type: EV_SYN,
            code: 0,
            value: 0,
        });
        drop(state);
        
        // Clear the key event (by reading - or write 0 to acknowledge)
        write_reg(KEY_COUNT, 0);
    }
    
    // Then poll for touch events
    let int_status = read_reg(TOUCH_INT_STATUS);
    if int_status == 0 {
        return; // No touch interrupt pending
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

/// Check if there are pending character inputs
pub fn has_char_input() -> bool {
    read_reg(CHAR_COUNT) > 0
}

/// Peek at the next character without consuming it
pub fn peek_char() -> Option<u8> {
    if read_reg(CHAR_COUNT) > 0 {
        Some(read_reg(CHAR_CODE) as u8)
    } else {
        None
    }
}

/// Consume the current character from the queue
pub fn consume_char() {
    write_reg(CHAR_COUNT, 0);
}

// ── TWI2 + FT6336U (hardware). Emulator probe fails and stays on MMIO. ──

const TWI_BASE: usize = 0x0250_2000;
const TWI_DATA: usize = 0x08;
const TWI_CNTR: usize = 0x0C;
const TWI_STAT: usize = 0x10;
const TWI_CCR: usize = 0x14;
const TWI_SRST: usize = 0x18;
const FT_ADDR: u8 = 0x38;
const FT_ID_REG: u8 = 0xA3;
const FT_TD_STATUS: u8 = 0x02;

const CNTR_BUS_EN: u32 = 1 << 6;
const CNTR_M_STA: u32 = 1 << 5;
const CNTR_M_STP: u32 = 1 << 4;
const CNTR_INT_FLAG: u32 = 1 << 3;
const CNTR_A_ACK: u32 = 1 << 2;

fn twi_wait_flag() -> Option<u32> {
    for _ in 0..50_000 {
        let c = read_reg(TWI_BASE + TWI_CNTR);
        if (c & CNTR_INT_FLAG) != 0 {
            return Some(read_reg(TWI_BASE + TWI_STAT) & 0xFF);
        }
        core::hint::spin_loop();
    }
    None
}

fn twi_clear_flag(ack: bool, extra: u32) {
    let mut c = read_reg(TWI_BASE + TWI_CNTR);
    c &= !CNTR_INT_FLAG;
    if ack {
        c |= CNTR_A_ACK;
    } else {
        c &= !CNTR_A_ACK;
    }
    write_reg(TWI_BASE + TWI_CNTR, c | extra);
}

fn twi_stop() {
    let mut c = read_reg(TWI_BASE + TWI_CNTR);
    c |= CNTR_M_STP;
    c &= !CNTR_INT_FLAG;
    write_reg(TWI_BASE + TWI_CNTR, c);
}

fn twi_read_reg(reg: u8) -> Option<u8> {
    write_reg(TWI_BASE + TWI_SRST, 1);
    for _ in 0..1000 {
        core::hint::spin_loop();
    }
    write_reg(TWI_BASE + TWI_SRST, 0);
    write_reg(TWI_BASE + TWI_CCR, 0x43);
    write_reg(TWI_BASE + TWI_CNTR, CNTR_BUS_EN);

    write_reg(TWI_BASE + TWI_CNTR, CNTR_BUS_EN | CNTR_M_STA);
    let st = twi_wait_flag()?;
    if st != 0x08 && st != 0x10 {
        twi_stop();
        return None;
    }
    write_reg(TWI_BASE + TWI_DATA, (FT_ADDR << 1) as u32);
    twi_clear_flag(false, 0);
    let st = twi_wait_flag()?;
    if st != 0x18 {
        twi_stop();
        return None;
    }
    write_reg(TWI_BASE + TWI_DATA, reg as u32);
    twi_clear_flag(false, 0);
    if twi_wait_flag()? != 0x28 {
        twi_stop();
        return None;
    }
    write_reg(TWI_BASE + TWI_CNTR, CNTR_BUS_EN | CNTR_M_STA);
    let st = twi_wait_flag()?;
    if st != 0x10 && st != 0x08 {
        twi_stop();
        return None;
    }
    write_reg(TWI_BASE + TWI_DATA, ((FT_ADDR << 1) | 1) as u32);
    twi_clear_flag(false, 0);
    if twi_wait_flag()? != 0x40 {
        twi_stop();
        return None;
    }
    twi_clear_flag(false, 0); // NACK last byte
    let st = twi_wait_flag()?;
    let data = read_reg(TWI_BASE + TWI_DATA) as u8;
    let _ = st;
    twi_stop();
    Some(data)
}

fn twi_probe_ft6336() -> bool {
    matches!(twi_read_reg(FT_ID_REG), Some(0x64) | Some(0x11) | Some(0x36) | Some(0x06))
}

fn ft6336_poll() {
    let n = match twi_read_reg(FT_TD_STATUS) {
        Some(v) => (v & 0x0F) as i32,
        None => return,
    };
    let mut state = TOUCH_STATE.lock();
    if n > 0 {
        let xh = twi_read_reg(0x03).unwrap_or(0);
        let xl = twi_read_reg(0x04).unwrap_or(0);
        let yh = twi_read_reg(0x05).unwrap_or(0);
        let yl = twi_read_reg(0x06).unwrap_or(0);
        let x = ((((xh as u16) & 0x0F) << 8) | xl as u16) as i32;
        let y = ((((yh as u16) & 0x0F) << 8) | yl as u16) as i32;
        let is_new = !state.pressed;
        if x != state.last_x || is_new {
            state.last_x = x;
            state.push_event(InputEvent { event_type: EV_ABS, code: ABS_X, value: x });
        }
        if y != state.last_y || is_new {
            state.last_y = y;
            state.push_event(InputEvent { event_type: EV_ABS, code: ABS_Y, value: y });
        }
        if is_new {
            state.pressed = true;
            state.push_event(InputEvent { event_type: EV_KEY, code: BTN_TOUCH, value: 1 });
        }
        state.push_event(InputEvent { event_type: EV_SYN, code: 0, value: 0 });
    } else if state.pressed {
        state.pressed = false;
        state.push_event(InputEvent { event_type: EV_KEY, code: BTN_TOUCH, value: 0 });
        state.push_event(InputEvent { event_type: EV_SYN, code: 0, value: 0 });
    }
}
