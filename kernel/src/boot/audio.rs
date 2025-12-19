//! Audio initialization
//!
//! Initializes the D1 audio codec driver during boot and plays a boot sound.

use crate::platform;

/// Sample rate for boot sound
const SAMPLE_RATE: u32 = 48000;

/// Boot beep frequency in Hz
const BEEP_FREQ: u32 = 440;

/// Boot beep duration in milliseconds
const BEEP_DURATION_MS: u32 = 200;

pub fn init_audio() {
    if platform::d1_audio::init().is_ok() {
        // Configure audio device
        platform::d1_audio::set_sample_rate(SAMPLE_RATE);
        platform::d1_audio::set_enabled(true);
        
        // Play boot beep - simple sine wave at 440Hz
        play_boot_beep();
        
        // Disable after beep (kernel will re-enable when needed)
        platform::d1_audio::set_enabled(false);
    }
}

/// Play a beep sound (can be called anytime after boot)
pub fn play_beep() {
    // Configure and enable
    platform::d1_audio::set_sample_rate(SAMPLE_RATE);
    platform::d1_audio::set_enabled(true);
    
    // Play the beep
    play_boot_beep();
    
    // Disable after beep
    platform::d1_audio::set_enabled(false);
}

/// Play a simple sine wave beep for testing
fn play_boot_beep() {
    let samples_to_play = (SAMPLE_RATE * BEEP_DURATION_MS / 1000) as usize;
    let samples_per_cycle = SAMPLE_RATE / BEEP_FREQ;
    
    // Pre-compute sine table for one cycle (256 entries)
    // Using fixed-point math since we're in no_std
    const SINE_TABLE: [i16; 256] = generate_sine_table();
    
    for i in 0..samples_to_play {
        // Calculate phase position in sine table (0-255)
        let phase = ((i as u32 * 256) / samples_per_cycle as u32) % 256;
        let sample = SINE_TABLE[phase as usize];
        
        // Apply volume envelope (fade in/out)
        let envelope = calculate_envelope(i, samples_to_play);
        let sample = ((sample as i32 * envelope as i32) / 100) as i16;
        
        // Write stereo sample (same on both channels)
        // Retry a few times if buffer is full
        for _ in 0..10 {
            if platform::d1_audio::write_stereo(sample, sample) {
                break;
            }
            // Brief spin wait if buffer full
            for _ in 0..100 {
                core::hint::spin_loop();
            }
        }
    }
}

/// Calculate volume envelope (fade in first 10%, fade out last 10%)
fn calculate_envelope(sample_idx: usize, total_samples: usize) -> u8 {
    let fade_samples = total_samples / 10;
    
    if sample_idx < fade_samples {
        // Fade in
        ((sample_idx * 100) / fade_samples) as u8
    } else if sample_idx > total_samples - fade_samples {
        // Fade out
        (((total_samples - sample_idx) * 100) / fade_samples) as u8
    } else {
        100
    }
}

/// Generate a sine table at compile time
const fn generate_sine_table() -> [i16; 256] {
    let mut table = [0i16; 256];
    let mut i = 0;
    while i < 256 {
        // Approximate sine using Taylor series (good enough for audio)
        // sin(x) ≈ x - x³/6 + x⁵/120 for x in radians
        // x = 2π * i / 256
        // Using scaled integer math (scale = 10000)
        
        let x_scaled = (i as i32 * 62832) / 256; // 2π * 10000 * i / 256
        
        // Normalize to -π to π range
        let x = if x_scaled > 31416 {
            x_scaled - 62832
        } else {
            x_scaled
        };
        
        // Simple sine approximation using parabola (faster, good for audio)
        // sin(x) ≈ 4x(π-x) / π² for 0 ≤ x ≤ π
        let pi = 31416; // π * 10000
        let abs_x = if x < 0 { -x } else { x };
        let sign = if x < 0 { -1 } else { 1 };
        
        // y = 4 * x * (π - x) / π²
        let y = (4 * abs_x * (pi - abs_x)) / ((pi * pi) / 10000);
        
        // Scale to i16 range (-32767 to 32767), but use ~50% volume
        table[i] = ((sign * y * 16000) / 10000) as i16;
        
        i += 1;
    }
    table
}
