//! D1 Audio Codec Driver
//!
//! Driver for the Allwinner D1-style audio codec on D1 platforms.
//! Uses simplified MMIO interface matching the emulator's d1_audio device.
//!
//! Thread-safe: All state is protected by a Spinlock, allowing any hart to access.
//!
//! # Registers (emulator-specific MMIO at 0x0203_0000)
//! - 0x00: CODEC_CTL - Control register (enable, reset)
//! - 0x04: CODEC_STS - Status register (buffer flags)
//! - 0x08: CODEC_DATA - Sample FIFO write port
//! - 0x0C: CODEC_BUF_LEVEL - Buffer fill level (read-only)
//! - 0x10: CODEC_SAMPLE_RATE - Sample rate in Hz

use core::ptr::{read_volatile, write_volatile};
use crate::Spinlock;

// =============================================================================
// Register Definitions
// =============================================================================

/// D1 Audio Codec base address
const D1_AUDIO_BASE: usize = 0x0203_0000;

// Register offsets
const CODEC_CTL: usize = D1_AUDIO_BASE + 0x00;
const CODEC_STS: usize = D1_AUDIO_BASE + 0x04;
const CODEC_DATA: usize = D1_AUDIO_BASE + 0x08;
const CODEC_BUF_LEVEL: usize = D1_AUDIO_BASE + 0x0C;
const CODEC_SAMPLE_RATE: usize = D1_AUDIO_BASE + 0x10;

// Control register bits
const CTL_ENABLE: u32 = 0x1;
const CTL_RESET: u32 = 0x2;

// Status register bits
const STS_BUFFER_FULL: u32 = 0x1;
const STS_BUFFER_EMPTY: u32 = 0x2;
const STS_UNDERRUN: u32 = 0x4;

// =============================================================================
// Driver State
// =============================================================================

/// Audio driver state - protected by Spinlock for thread safety
struct AudioState {
    /// Whether the driver has been initialized
    initialized: bool,
    /// Current sample rate
    sample_rate: u32,
    /// Whether playback is enabled
    enabled: bool,
}

impl AudioState {
    const fn new() -> Self {
        Self {
            initialized: false,
            sample_rate: 48000, // Default to 48kHz
            enabled: false,
        }
    }
}

/// Global audio state protected by Spinlock
static AUDIO_STATE: Spinlock<AudioState> = Spinlock::new(AudioState::new());

// =============================================================================
// Register Access
// =============================================================================

/// Read a 32-bit register
#[inline]
fn read_reg(addr: usize) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

/// Write a 32-bit register
#[inline]
fn write_reg(addr: usize, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

// =============================================================================
// Public API
// =============================================================================

/// Initialize the audio codec driver
pub fn init() -> Result<(), &'static str> {
    let mut state = AUDIO_STATE.lock();
    
    // Reset the codec
    write_reg(CODEC_CTL, CTL_RESET);
    
    // Wait a bit for reset (simple delay)
    for _ in 0..100 {
        core::hint::spin_loop();
    }
    
    // Clear reset, leave disabled
    write_reg(CODEC_CTL, 0);
    
    // Set default sample rate
    write_reg(CODEC_SAMPLE_RATE, state.sample_rate);
    
    state.initialized = true;
    
    Ok(())
}

/// Check if the audio driver is initialized
#[inline]
pub fn is_initialized() -> bool {
    AUDIO_STATE.lock().initialized
}

/// Enable or disable audio playback
pub fn set_enabled(enabled: bool) {
    let mut state = AUDIO_STATE.lock();
    
    if enabled {
        write_reg(CODEC_CTL, CTL_ENABLE);
    } else {
        write_reg(CODEC_CTL, 0);
    }
    
    state.enabled = enabled;
}

/// Check if playback is enabled
#[inline]
pub fn is_enabled() -> bool {
    AUDIO_STATE.lock().enabled
}

/// Set the sample rate in Hz (e.g., 44100, 48000)
pub fn set_sample_rate(rate: u32) {
    let mut state = AUDIO_STATE.lock();
    write_reg(CODEC_SAMPLE_RATE, rate);
    state.sample_rate = rate;
}

/// Get the current sample rate
#[inline]
pub fn get_sample_rate() -> u32 {
    AUDIO_STATE.lock().sample_rate
}

/// Write a stereo audio sample to the FIFO
///
/// Sample format: 32-bit word with:
/// - bits [15:0]: Left channel (i16)
/// - bits [31:16]: Right channel (i16)
///
/// Returns `true` if the sample was written, `false` if the buffer was full
pub fn write_sample(sample: u32) -> bool {
    // Check if buffer is full before writing
    if is_buffer_full() {
        return false;
    }
    
    write_reg(CODEC_DATA, sample);
    true
}

/// Write stereo samples from separate left/right i16 channels
///
/// Returns `true` if the sample was written, `false` if the buffer was full
pub fn write_stereo(left: i16, right: i16) -> bool {
    let sample = ((right as u32) << 16) | ((left as u16) as u32);
    write_sample(sample)
}

/// Get the current buffer fill level (number of samples in buffer)
#[inline]
pub fn buffer_level() -> u32 {
    read_reg(CODEC_BUF_LEVEL)
}

/// Check if the audio buffer is full
#[inline]
pub fn is_buffer_full() -> bool {
    (read_reg(CODEC_STS) & STS_BUFFER_FULL) != 0
}

/// Check if the audio buffer is empty
#[inline]
pub fn is_buffer_empty() -> bool {
    (read_reg(CODEC_STS) & STS_BUFFER_EMPTY) != 0
}

/// Check if an underrun has occurred (buffer emptied during playback)
#[inline]
pub fn has_underrun() -> bool {
    (read_reg(CODEC_STS) & STS_UNDERRUN) != 0
}

/// Clear the underrun flag
pub fn clear_underrun() {
    // Read-modify-write to clear only the underrun bit
    let status = read_reg(CODEC_STS);
    write_reg(CODEC_STS, status & !STS_UNDERRUN);
}

/// Reset the audio codec (clears buffer, stops playback)
pub fn reset() {
    let mut state = AUDIO_STATE.lock();
    
    // Assert reset
    write_reg(CODEC_CTL, CTL_RESET);
    
    // Brief delay
    for _ in 0..100 {
        core::hint::spin_loop();
    }
    
    // Clear reset
    write_reg(CODEC_CTL, 0);
    
    // Restore sample rate
    write_reg(CODEC_SAMPLE_RATE, state.sample_rate);
    
    state.enabled = false;
}
