//! Cursor and Mouse Handling
//!
//! Manages cursor position, visibility, and rendering.

use core::ptr::addr_of_mut;

use crate::platform::d1_display;
use crate::platform::d1_touch::{BTN_LEFT, BTN_MIDDLE, BTN_RIGHT};

use super::{SCREEN_HEIGHT, SCREEN_WIDTH};

/// Mouse/cursor state
pub static mut CURSOR_X: i32 = 512;  // Start at center of 1024x768
pub static mut CURSOR_Y: i32 = 384;
static mut CURSOR_VISIBLE: bool = false;
static mut MOUSE_BUTTONS: u8 = 0;  // Bitmask: bit 0 = left, bit 1 = right, bit 2 = middle

/// Get current cursor position
pub fn get_cursor_pos() -> (i32, i32) {
    unsafe { (CURSOR_X, CURSOR_Y) }
}

/// Set cursor position (called when EV_ABS events received)
pub fn set_cursor_pos(x: i32, y: i32) {
    unsafe {
        CURSOR_X = x.clamp(0, SCREEN_WIDTH - 1);
        CURSOR_Y = y.clamp(0, SCREEN_HEIGHT - 1);
        CURSOR_VISIBLE = true;
    }
}

/// Set mouse button state
pub fn set_mouse_button(button: u16, pressed: bool) {
    use crate::platform::d1_touch::BTN_TOUCH;
    unsafe {
        let bit = match button {
            BTN_LEFT | BTN_TOUCH => 0,  // BTN_TOUCH acts like left mouse button
            BTN_RIGHT => 1,
            BTN_MIDDLE => 2,
            _ => return,
        };
        if pressed {
            MOUSE_BUTTONS |= 1 << bit;
        } else {
            MOUSE_BUTTONS &= !(1 << bit);
        }
    }
}

/// Get mouse button state
pub fn get_mouse_buttons() -> u8 {
    unsafe { MOUSE_BUTTONS }
}

/// Check if left mouse button is pressed
pub fn is_left_button_pressed() -> bool {
    unsafe { (MOUSE_BUTTONS & 1) != 0 }
}

/// Cursor dimensions
const CURSOR_W: usize = 12;
const CURSOR_H: usize = 16;

/// Previous cursor position for restore
static mut CURSOR_PREV_X: i32 = -100;
static mut CURSOR_PREV_Y: i32 = -100;

/// Saved pixels under cursor (12x16 = 192 pixels)
static mut CURSOR_BACKUP: [u32; CURSOR_W * CURSOR_H] = [0; CURSOR_W * CURSOR_H];
static mut CURSOR_BACKUP_VALID: bool = false;

/// Cursor bitmap (1 = white, 2 = black border, 0 = transparent)
/// Arrow cursor pointing top-left
const CURSOR_BITMAP: [u8; CURSOR_W * CURSOR_H] = [
    1,0,0,0,0,0,0,0,0,0,0,0,
    1,1,0,0,0,0,0,0,0,0,0,0,
    1,2,1,0,0,0,0,0,0,0,0,0,
    1,2,2,1,0,0,0,0,0,0,0,0,
    1,2,2,2,1,0,0,0,0,0,0,0,
    1,2,2,2,2,1,0,0,0,0,0,0,
    1,2,2,2,2,2,1,0,0,0,0,0,
    1,2,2,2,2,2,2,1,0,0,0,0,
    1,2,2,2,2,2,2,2,1,0,0,0,
    1,2,2,2,2,2,2,2,2,1,0,0,
    1,2,2,2,2,1,1,1,1,1,1,0,
    1,2,2,1,2,1,0,0,0,0,0,0,
    1,2,1,0,1,2,1,0,0,0,0,0,
    1,1,0,0,1,2,1,0,0,0,0,0,
    1,0,0,0,0,1,2,1,0,0,0,0,
    0,0,0,0,0,1,1,0,0,0,0,0,
];

/// Restore pixels under cursor (call before moving cursor)
pub fn restore_cursor_backup() {
    let (px, py) = unsafe { (CURSOR_PREV_X, CURSOR_PREV_Y) };
    if !unsafe { CURSOR_BACKUP_VALID } || px < 0 || py < 0 {
        return;
    }
    
    // Use batch write for faster restore
    d1_display::with_gpu(|gpu| {
        gpu.write_rect(px as u32, py as u32, CURSOR_W, CURSOR_H, 
            unsafe { &CURSOR_BACKUP }, &CURSOR_BITMAP);
    });
    
    unsafe { CURSOR_BACKUP_VALID = false; }
}

/// Save pixels under cursor location
fn save_cursor_backup(x: i32, y: i32) {
    if x < 0 || y < 0 {
        return;
    }
    
    // Use batch read for faster save
    d1_display::with_gpu(|gpu| {
        gpu.read_rect(x as u32, y as u32, CURSOR_W, CURSOR_H, 
            unsafe { &mut *addr_of_mut!(CURSOR_BACKUP) });
    });
    unsafe { CURSOR_BACKUP_VALID = true; }
}

/// Draw cursor at current position - proper arrow pointer with bitmap
pub fn draw_cursor() {
    let (x, y) = unsafe { (CURSOR_X, CURSOR_Y) };
    let (px, py) = unsafe { (CURSOR_PREV_X, CURSOR_PREV_Y) };
    
    if !unsafe { CURSOR_VISIBLE } {
        return;
    }
    
    // Check if backup was invalidated (UI was redrawn)
    let needs_refresh = !unsafe { CURSOR_BACKUP_VALID };
    
    // Skip if position hasn't changed AND backup is valid
    if x == px && y == py && !needs_refresh {
        return;
    }
    
    // Restore previous cursor location (only if backup is valid)
    if unsafe { CURSOR_BACKUP_VALID } {
        restore_cursor_backup();
    }
    
    // Save pixels at new location
    save_cursor_backup(x, y);
    
    // Update previous position
    unsafe {
        CURSOR_PREV_X = x;
        CURSOR_PREV_Y = y;
    }
    
    // Draw cursor using batched bitmap write
    d1_display::with_gpu(|gpu| {
        gpu.draw_cursor_bitmap(x, y, CURSOR_W, CURSOR_H, &CURSOR_BITMAP);
    });
}

/// Hide cursor (restore background and mark invisible)
pub fn hide_cursor() {
    restore_cursor_backup();
    unsafe {
        CURSOR_VISIBLE = false;
        CURSOR_PREV_X = -100;
        CURSOR_PREV_Y = -100;
    }
}

/// Invalidate cursor backup (call after UI elements are redrawn to prevent ghost cursor)
/// This forces the cursor to re-save the background on next draw
pub fn invalidate_cursor_backup() {
    unsafe {
        CURSOR_BACKUP_VALID = false;
    }
}
