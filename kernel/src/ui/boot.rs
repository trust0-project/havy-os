//! Boot Console - Terminal-style scrolling text for kernel boot
//!
//! Displays boot messages on the GPU framebuffer with a classic
//! terminal aesthetic (black background, green text scrolling up).
//!
//! Uses u8g2-fonts for Unicode text rendering (box-drawing, symbols).

use core::ptr::addr_of_mut;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use embedded_graphics::{
    pixelcolor::Rgb888,
    prelude::*,
};

use u8g2_fonts::{
    fonts,
    types::{FontColor, HorizontalAlignment, VerticalPosition},
    FontRenderer,
};

use crate::platform::d1_display;

/// Maximum lines in the boot console buffer
const MAX_LINES: usize = 50;

/// Maximum BYTES per line (UTF-8 box-drawing chars are 3 bytes each)
/// A line with 80 Unicode chars could need up to 240 bytes
const MAX_LINE_LEN: usize = 100;

/// Display dimensions (1024×768 display)
const DISPLAY_WIDTH: u32 = 1024;
const DISPLAY_HEIGHT: u32 = 768;

/// Font dimensions (9x15 X11 fixed font)
const FONT_HEIGHT: u32 = 15;
const LINE_SPACING: u32 = 2;
const LINE_HEIGHT: u32 = FONT_HEIGHT + LINE_SPACING;

/// Font renderer for Unicode text
/// Using 9x15 X11 fixed font with symbols - includes box-drawing (U+2500-U+257F)
const FONT: FontRenderer = FontRenderer::new::<fonts::u8g2_font_9x15_t_symbols>();

/// Console margins
const MARGIN_LEFT: i32 = 16;
const MARGIN_TOP: i32 = 16;

/// Colors
const COLOR_BACKGROUND: Rgb888 = Rgb888::new(0, 0, 0);  // Black
const COLOR_TEXT: Rgb888 = Rgb888::new(0, 255, 0);      // Bright green

/// Boot phase tracking
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BootPhase {
    /// Boot console is active, showing scrolling text
    Console = 0,
    /// Full GUI is active after boot completes
    Gui = 1,
}

/// Global boot phase
static BOOT_PHASE: AtomicUsize = AtomicUsize::new(BootPhase::Console as usize);

/// Boot console line buffer
struct LineBuffer {
    /// Line storage [line_index][char_index]
    lines: [[u8; MAX_LINE_LEN]; MAX_LINES],
    /// Length of each line
    lengths: [usize; MAX_LINES],
    /// Current write position (next line to write)
    write_pos: usize,
    /// Number of valid lines
    line_count: usize,
}

impl LineBuffer {
    const fn new() -> Self {
        Self {
            lines: [[0u8; MAX_LINE_LEN]; MAX_LINES],
            lengths: [0; MAX_LINES],
            write_pos: 0,
            line_count: 0,
        }
    }
    
    /// Add a line to the buffer (scrolls if full)
    fn push_line(&mut self, text: &str) {
        let bytes = text.as_bytes();
        let len = bytes.len().min(MAX_LINE_LEN);
        
        // Copy text to current position
        self.lines[self.write_pos][..len].copy_from_slice(&bytes[..len]);
        self.lengths[self.write_pos] = len;
        
        // Advance write position (circular buffer)
        self.write_pos = (self.write_pos + 1) % MAX_LINES;
        
        // Track total lines
        if self.line_count < MAX_LINES {
            self.line_count += 1;
        }
    }
    
    /// Get line at display position (0 = oldest visible line)
    fn get_line(&self, display_idx: usize) -> Option<&str> {
        if display_idx >= self.line_count {
            return None;
        }
        
        // Calculate actual buffer index
        let start = if self.line_count < MAX_LINES {
            0
        } else {
            self.write_pos
        };
        let idx = (start + display_idx) % MAX_LINES;
        let len = self.lengths[idx];
        
        core::str::from_utf8(&self.lines[idx][..len]).ok()
    }
}

/// Global boot console state
static mut CONSOLE: LineBuffer = LineBuffer::new();
static CONSOLE_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Initialize the boot console
/// Should be called early in boot after GPU is available
pub fn init() {
    CONSOLE_INITIALIZED.store(true, Ordering::Release);
    // Clear both framebuffers to black (deferred from d1_display::init for speed)
    d1_display::init_clear_buffers();
}

/// Check if boot console is initialized
pub fn is_initialized() -> bool {
    CONSOLE_INITIALIZED.load(Ordering::Acquire)
}

/// Get current boot phase
pub fn get_phase() -> BootPhase {
    match BOOT_PHASE.load(Ordering::Acquire) {
        0 => BootPhase::Console,
        _ => BootPhase::Gui,
    }
}

/// Set boot phase to GUI (called when boot completes)
pub fn set_phase_gui() {
    BOOT_PHASE.store(BootPhase::Gui as usize, Ordering::Release);
}

/// Print a line to the boot console
/// This adds the text to the buffer and triggers a render
pub fn print_line(text: &str) {
    if !is_initialized() {
        return;
    }
    
    unsafe {
        (*addr_of_mut!(CONSOLE)).push_line(text);
    }
    
    // Auto-render after each line
    render();
}

/// Print a line with a prefix (for boot stages)
pub fn print_boot_msg(prefix: &str, msg: &str) {
    if !is_initialized() {
        return;
    }
    
    // Format: "[prefix] msg" using static buffer
    let mut buf = [0u8; MAX_LINE_LEN];
    let mut pos = 0;
    
    // Add prefix
    if !prefix.is_empty() {
        buf[pos] = b'[';
        pos += 1;
        let prefix_bytes = prefix.as_bytes();
        let len = prefix_bytes.len().min(20);
        buf[pos..pos+len].copy_from_slice(&prefix_bytes[..len]);
        pos += len;
        buf[pos] = b']';
        pos += 1;
        buf[pos] = b' ';
        pos += 1;
    }
    
    // Add message
    let msg_bytes = msg.as_bytes();
    let len = msg_bytes.len().min(MAX_LINE_LEN - pos);
    buf[pos..pos+len].copy_from_slice(&msg_bytes[..len]);
    pos += len;
    
    if let Ok(line) = core::str::from_utf8(&buf[..pos]) {
        print_line(line);
    }
}





/// Batch depth counter: when > 0, render() skips flushing
/// Uses reference counting so nested batch_begin/batch_end pairs work correctly
/// Example: outer batch_begin, inner batch_begin, inner batch_end, outer batch_end
///          Only the outer batch_end triggers the actual flush
static mut BATCH_DEPTH: usize = 0;

/// Enable batch mode (defer flushes until all batch_end calls complete)
pub fn batch_begin() {
    unsafe { BATCH_DEPTH += 1; }
}

/// End batch mode - only flushes when all nested batches have ended
pub fn batch_end() {
    unsafe {
        if BATCH_DEPTH > 0 {
            BATCH_DEPTH -= 1;
        }
        // Only flush when we've exited all nested batches
        if BATCH_DEPTH == 0 {
            d1_display::flush();
        }
    }
}

/// Render the boot console to the framebuffer using embedded-graphics
/// Uses FULL REDRAW approach - always redraws all visible lines
/// This is simpler and guarantees correct display regardless of timing
pub fn render() {
    if !is_initialized() || get_phase() != BootPhase::Console {
        return;
    }
    
    // Calculate visible lines based on display area
    let visible_lines = ((DISPLAY_HEIGHT as i32 - MARGIN_TOP * 2) / LINE_HEIGHT as i32) as usize;
    let visible_lines = visible_lines.min(MAX_LINES);
    
    unsafe {
        let line_count = CONSOLE.line_count;
        if line_count == 0 {
            if BATCH_DEPTH == 0 {
                d1_display::flush();
            }
            return;
        }
        
        // Calculate scroll offset (how many lines scrolled off the top)
        let scroll_offset = line_count.saturating_sub(visible_lines);
        
        // ALWAYS do full redraw - simpler and more reliable
        d1_display::with_gpu(|gpu| {
            // Clear with bulk 64-bit writes
            let _ = gpu.clear(COLOR_BACKGROUND.r(), COLOR_BACKGROUND.g(), COLOR_BACKGROUND.b());
            
            // Draw ALL visible lines with pixel batching for speed
            d1_display::begin_pixel_batch();
            let num_to_show = line_count.min(visible_lines);
            for i in 0..num_to_show {
                let buffer_idx = scroll_offset + i;
                let y = MARGIN_TOP + (i as i32 * LINE_HEIGHT as i32) + FONT_HEIGHT as i32;
                if let Some(text) = CONSOLE.get_line(buffer_idx) {
                    let _ = FONT.render_aligned(
                        text,
                        Point::new(MARGIN_LEFT, y),
                        VerticalPosition::Baseline,
                        HorizontalAlignment::Left,
                        FontColor::Transparent(COLOR_TEXT),
                        gpu,
                    );
                }
            }
            d1_display::end_pixel_batch();
            // Dirty region already marked by mark_all_dirty() from clear()
        });
        
        // Only flush when not in any batch (BATCH_DEPTH == 0)
        // When batching, batch_end() handles the flush
        if BATCH_DEPTH == 0 {
            d1_display::flush();
        }
    }
}
