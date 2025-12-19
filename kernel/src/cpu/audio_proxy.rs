//! Audio Proxy - Hart-aware audio device access
//!
//! This module provides transparent audio access that works on any hart.
//! On Hart 0: Direct MMIO access via d1_audio
//! On secondary harts: Delegates to Hart 0 via io_router
//!
//! # Example
//! ```
//! use crate::cpu::audio_proxy;
//!
//! // Works on any hart!
//! audio_proxy::set_enabled(true);
//! audio_proxy::set_sample_rate(48000);
//! 
//! // Write samples
//! while !audio_proxy::is_buffer_full() {
//!     audio_proxy::write_sample(sample);
//! }
//! ```

use alloc::vec::Vec;

use crate::cpu::io_router::{DeviceType, IoOp, IoRequest, IoResult, request_io};
use crate::platform::d1_audio;

// Timeout for I/O requests (5 seconds - audio operations are fast)
const IO_TIMEOUT_MS: u64 = 5000;

// ═══════════════════════════════════════════════════════════════════════════════
// Helper: Submit I/O request to Hart 0
// ═══════════════════════════════════════════════════════════════════════════════

/// Submit an I/O request and wait for the result (blocking).
fn request_io_blocking(operation: IoOp) -> IoResult {
    let request = IoRequest::new(DeviceType::Audio, operation);
    request_io(request, IO_TIMEOUT_MS)
}

// ═══════════════════════════════════════════════════════════════════════════════
// Public API: Audio Operations
// ═══════════════════════════════════════════════════════════════════════════════

/// Write an audio sample to the FIFO.
///
/// Sample format: 32-bit word with:
/// - bits [15:0]: Left channel (i16)
/// - bits [31:16]: Right channel (i16)
///
/// On Hart 0: Direct access via d1_audio
/// On secondary harts: Delegates to Hart 0 via io_router
///
/// Returns `true` if the sample was written, `false` if the buffer was full
#[inline]
pub fn write_sample(sample: u32) -> bool {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        d1_audio::write_sample(sample)
    } else {
        match request_io_blocking(IoOp::AudioWriteSample { sample }) {
            IoResult::Ok(data) => data.first() == Some(&1),
            IoResult::Err(_) => false,
        }
    }
}

/// Write stereo samples from separate left/right i16 channels.
///
/// Returns `true` if the sample was written, `false` if the buffer was full
#[inline]
pub fn write_stereo(left: i16, right: i16) -> bool {
    let sample = ((right as u32) << 16) | ((left as u16) as u32);
    write_sample(sample)
}

/// Enable or disable audio playback.
///
/// On Hart 0: Direct access via d1_audio
/// On secondary harts: Delegates to Hart 0 via io_router
#[inline]
pub fn set_enabled(enabled: bool) {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        d1_audio::set_enabled(enabled);
    } else {
        let _ = request_io_blocking(IoOp::AudioSetEnabled { enabled });
    }
}

/// Set the sample rate in Hz (e.g., 44100, 48000).
///
/// On Hart 0: Direct access via d1_audio
/// On secondary harts: Delegates to Hart 0 via io_router
#[inline]
pub fn set_sample_rate(rate: u32) {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        d1_audio::set_sample_rate(rate);
    } else {
        let _ = request_io_blocking(IoOp::AudioSetSampleRate { rate });
    }
}

/// Get the current buffer fill level (number of samples in buffer).
///
/// On Hart 0: Direct access via d1_audio
/// On secondary harts: Delegates to Hart 0 via io_router
#[inline]
pub fn buffer_level() -> u32 {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        d1_audio::buffer_level()
    } else {
        match request_io_blocking(IoOp::AudioGetBufferLevel) {
            IoResult::Ok(data) if data.len() >= 4 => {
                u32::from_le_bytes([data[0], data[1], data[2], data[3]])
            }
            _ => 0,
        }
    }
}

/// Check if the audio buffer is full.
///
/// On Hart 0: Direct access via d1_audio
/// On secondary harts: Delegates to Hart 0 via io_router
#[inline]
pub fn is_buffer_full() -> bool {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        d1_audio::is_buffer_full()
    } else {
        match request_io_blocking(IoOp::AudioIsBufferFull) {
            IoResult::Ok(data) => data.first() == Some(&1),
            IoResult::Err(_) => true, // Assume full on error (safer)
        }
    }
}

/// Check if the audio buffer is empty.
///
/// On Hart 0: Direct access via d1_audio
/// On secondary harts: Delegates to Hart 0 via io_router
#[inline]
pub fn is_buffer_empty() -> bool {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        d1_audio::is_buffer_empty()
    } else {
        match request_io_blocking(IoOp::AudioIsBufferEmpty) {
            IoResult::Ok(data) => data.first() == Some(&1),
            IoResult::Err(_) => true, // Assume empty on error
        }
    }
}

/// Check if audio is initialized.
///
/// On Hart 0: Direct access via d1_audio
/// On secondary harts: Delegates to Hart 0 via io_router
#[inline]
pub fn is_initialized() -> bool {
    let hart_id = crate::get_hart_id();
    
    if hart_id == 0 {
        d1_audio::is_initialized()
    } else {
        match request_io_blocking(IoOp::Status) {
            IoResult::Ok(data) => data == b"online",
            IoResult::Err(_) => false,
        }
    }
}
