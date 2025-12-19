//! Display/Touch Proxy - Hart-aware display and touch access
//!
//! This module provides transparent display and touch access that works on any hart.
//! On Hart 0: Direct MMIO access via d1_display/d1_touch
//! On secondary harts: Delegates to Hart 0 via io_router
//!
//! # Example
//! ```
//! use crate::cpu::display_proxy;
//!
//! // Works on any hart!
//! display_proxy::touch_poll();
//! while let Some(event) = display_proxy::touch_next_event() {
//!     // Process event...
//! }
//! display_proxy::flush();
//! ```

use alloc::vec::Vec;

use crate::cpu::io_router::{DeviceType, IoOp, IoRequest, IoResult, request_io};
use crate::platform::d1_touch::InputEvent;
use crate::platform::{d1_display, d1_touch};

// Timeout for I/O requests (5 seconds - display/touch are fast operations)
const IO_TIMEOUT_MS: u64 = 5000;

// ═══════════════════════════════════════════════════════════════════════════════
// Helper: Submit I/O request to Hart 0
// ═══════════════════════════════════════════════════════════════════════════════

/// Submit an I/O request and wait for the result (blocking).
fn request_io_blocking(operation: IoOp) -> IoResult {
    let request = IoRequest::new(DeviceType::Display, operation);
    request_io(request, IO_TIMEOUT_MS)
}

// ═══════════════════════════════════════════════════════════════════════════════
// Public API: Display Operations
// ═══════════════════════════════════════════════════════════════════════════════

/// Flush the display (copy dirty region from back buffer to front buffer).
///
/// On Hart 0: Direct access via d1_display
/// On secondary harts: Delegates to Hart 0 via io_router
#[inline]
pub fn flush() {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        d1_display::flush();
    } else {
        let _ = request_io_blocking(IoOp::DisplayFlush);
    }
}

/// Clear the display to black.
///
/// On Hart 0: Direct access via d1_display
/// On secondary harts: Delegates to Hart 0 via io_router
#[inline]
pub fn clear_display() {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        d1_display::clear_display();
    } else {
        let _ = request_io_blocking(IoOp::DisplayClear);
    }
}

/// Mark entire screen as dirty.
///
/// On Hart 0: Direct access via d1_display
/// On secondary harts: Delegates to Hart 0 via io_router
#[inline]
pub fn mark_all_dirty() {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        d1_display::mark_all_dirty();
    } else {
        let _ = request_io_blocking(IoOp::DisplayMarkAllDirty);
    }
}

/// Check if display is available.
///
/// On Hart 0: Direct access via d1_display
/// On secondary harts: Delegates to Hart 0 via io_router
#[inline]
pub fn is_available() -> bool {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        d1_display::is_available()
    } else {
        match request_io_blocking(IoOp::DisplayIsAvailable) {
            IoResult::Ok(data) => data.first() == Some(&1),
            IoResult::Err(_) => false,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Public API: Touch Operations
// ═══════════════════════════════════════════════════════════════════════════════

/// Poll for touch events.
///
/// On Hart 0: Direct access via d1_touch
/// On secondary harts: Delegates to Hart 0 via io_router
#[inline]
pub fn touch_poll() {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        d1_touch::poll();
    } else {
        let _ = request_io_blocking(IoOp::TouchPoll);
    }
}

/// Get the next touch event from the queue.
///
/// On Hart 0: Direct access via d1_touch
/// On secondary harts: Delegates to Hart 0 via io_router
///
/// Returns `None` if no events are pending.
#[inline]
pub fn touch_next_event() -> Option<InputEvent> {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        d1_touch::next_event()
    } else {
        match request_io_blocking(IoOp::TouchNextEvent) {
            IoResult::Ok(data) if data.len() == 8 => {
                // Deserialize: [type:2][code:2][value:4]
                Some(InputEvent {
                    event_type: u16::from_le_bytes([data[0], data[1]]),
                    code: u16::from_le_bytes([data[2], data[3]]),
                    value: i32::from_le_bytes([data[4], data[5], data[6], data[7]]),
                })
            }
            _ => None, // Empty response or error = no event
        }
    }
}

/// Check if there are pending touch events.
///
/// On Hart 0: Direct access via d1_touch
/// On secondary harts: Delegates to Hart 0 via io_router
#[inline]
pub fn touch_has_events() -> bool {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        d1_touch::has_events()
    } else {
        match request_io_blocking(IoOp::TouchHasEvents) {
            IoResult::Ok(data) => data.first() == Some(&1),
            IoResult::Err(_) => false,
        }
    }
}
