//! Display/Touch Proxy - Hart-aware display and touch access

use crate::cpu::io_router::{DeviceType, IoOp, IoRequest, IoResult, request_io};
use crate::input::InputEvent;
use crate::platform::d1_display;

const IO_TIMEOUT_MS: u64 = 5000;

fn request_io_blocking(operation: IoOp) -> IoResult {
    let request = IoRequest::new(DeviceType::Display, operation);
    request_io(request, IO_TIMEOUT_MS)
}

#[inline]
pub fn flush() {
    if crate::get_hart_id() == 0 {
        d1_display::flush();
    } else {
        let _ = request_io_blocking(IoOp::DisplayFlush);
    }
}

#[inline]
pub fn clear_display() {
    if crate::get_hart_id() == 0 {
        d1_display::clear_display();
    } else {
        let _ = request_io_blocking(IoOp::DisplayClear);
    }
}

#[inline]
pub fn mark_all_dirty() {
    if crate::get_hart_id() == 0 {
        d1_display::mark_all_dirty();
    } else {
        let _ = request_io_blocking(IoOp::DisplayMarkAllDirty);
    }
}

#[inline]
pub fn is_available() -> bool {
    if crate::get_hart_id() == 0 {
        d1_display::is_available()
    } else {
        match request_io_blocking(IoOp::DisplayIsAvailable) {
            IoResult::Ok(data) => data.first() == Some(&1),
            IoResult::Err(_) => false,
        }
    }
}

#[inline]
pub fn touch_poll() {
    if crate::get_hart_id() == 0 {
        crate::input::poll();
    } else {
        let _ = request_io_blocking(IoOp::TouchPoll);
    }
}

#[inline]
pub fn touch_next_event() -> Option<InputEvent> {
    if crate::get_hart_id() == 0 {
        crate::input::next_event()
    } else {
        match request_io_blocking(IoOp::TouchNextEvent) {
            IoResult::Ok(data) if data.len() == 8 => Some(InputEvent {
                event_type: u16::from_le_bytes([data[0], data[1]]),
                code: u16::from_le_bytes([data[2], data[3]]),
                value: i32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            }),
            _ => None,
        }
    }
}

#[inline]
pub fn touch_has_events() -> bool {
    if crate::get_hart_id() == 0 {
        crate::input::has_events()
    } else {
        match request_io_blocking(IoOp::TouchHasEvents) {
            IoResult::Ok(data) => data.first() == Some(&1),
            IoResult::Err(_) => false,
        }
    }
}
