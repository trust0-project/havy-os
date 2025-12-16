//! Boot Console - Terminal-style scrolling text for kernel boot
//!
//! Displays boot messages on the GPU framebuffer with a classic
//! terminal aesthetic (black background, green/white text scrolling up).

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Maximum lines in the boot console buffer
const MAX_LINES: usize = 40;

/// Maximum characters per line
const MAX_LINE_LEN: usize = 100;

/// Display dimensions (1024×768 display)
const DISPLAY_WIDTH: u32 = 1024;
const DISPLAY_HEIGHT: u32 = 768;

/// Font dimensions (6x10 pixel font)
const FONT_WIDTH: u32 = 6;
const FONT_HEIGHT: u32 = 12;

/// Console margins
const MARGIN_LEFT: u32 = 20;
const MARGIN_TOP: u32 = 20;

/// Colors (RGBA format: 0xAABBGGRR in little-endian)
const COLOR_BACKGROUND: u32 = 0xFF000000;  // Opaque black
const COLOR_TEXT: u32 = 0xFF00FF00;         // Bright green
const COLOR_TEXT_HIGHLIGHT: u32 = 0xFFFFFFFF; // White for highlights

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
    // Clear framebuffer to black
    clear_screen();
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
        CONSOLE.push_line(text);
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

/// Clear the screen to background color
fn clear_screen() {
    use crate::d1_display::BACK_BUFFER_ADDR;
    
    let fb_size = (DISPLAY_WIDTH * DISPLAY_HEIGHT) as usize;
    unsafe {
        let fb_ptr = BACK_BUFFER_ADDR as *mut u32;
        for i in 0..fb_size {
            core::ptr::write_volatile(fb_ptr.add(i), COLOR_BACKGROUND);
        }
    }
}

/// Clear a single line area (for incremental updates)
fn clear_line_area(y: u32) {
    use crate::d1_display::BACK_BUFFER_ADDR;
    
    unsafe {
        let fb_ptr = BACK_BUFFER_ADDR as *mut u32;
        let line_width = DISPLAY_WIDTH - MARGIN_LEFT * 2;
        
        for row in 0..FONT_HEIGHT {
            let py = y + row;
            if py < DISPLAY_HEIGHT {
                for col in 0..line_width {
                    let px = MARGIN_LEFT + col;
                    let idx = (py * DISPLAY_WIDTH + px) as usize;
                    core::ptr::write_volatile(fb_ptr.add(idx), COLOR_BACKGROUND);
                }
            }
        }
    }
}

/// Track if we've done initial clear
static mut INITIAL_CLEAR_DONE: bool = false;

/// Track how many lines we've drawn (for incremental/typewriter effect)
static mut LINES_DRAWN: usize = 0;

/// Track the scroll offset last time we rendered
static mut LAST_SCROLL_OFFSET: usize = 0;

/// Render the boot console to the framebuffer
/// Uses true typewriter effect: new lines appear without clearing old ones
/// When scrolling is needed, pixels are shifted up and only the new line is drawn
pub fn render() {
    if !is_initialized() || get_phase() != BootPhase::Console {
        return;
    }
    
    // Calculate visible lines
    let visible_lines = ((DISPLAY_HEIGHT - MARGIN_TOP * 2) / FONT_HEIGHT) as usize;
    let visible_lines = visible_lines.min(MAX_LINES);
    
    unsafe {
        // Initial full clear on first render only
        if !INITIAL_CLEAR_DONE {
            clear_screen();
            INITIAL_CLEAR_DONE = true;
            LINES_DRAWN = 0;
            LAST_SCROLL_OFFSET = 0;
        }
        
        let line_count = CONSOLE.line_count;
        if line_count == 0 {
            crate::d1_display::flush();
            return;
        }
        
        // Calculate scroll offset (how many lines scrolled off the top)
        let scroll_offset = line_count.saturating_sub(visible_lines);
        let prev_scroll = LAST_SCROLL_OFFSET;
        
        // Check if scrolling happened (content shifted up)
        if scroll_offset > prev_scroll {
            // Scrolling occurred - shift existing content up by copying pixels
            // This preserves existing lines visually and only draws the new line
            let lines_to_scroll = scroll_offset - prev_scroll;
            scroll_framebuffer_up(lines_to_scroll as u32);
            
            // Clear bottom line area and draw just the new line(s)
            let num_to_show = line_count.min(visible_lines);
            let new_lines_start = num_to_show.saturating_sub(lines_to_scroll);
            for i in new_lines_start..num_to_show {
                let buffer_idx = scroll_offset + i;
                let y = MARGIN_TOP + (i as u32 * FONT_HEIGHT);
                clear_line_area(y);
                if let Some(text) = CONSOLE.get_line(buffer_idx) {
                    draw_text(text, MARGIN_LEFT, y, COLOR_TEXT);
                }
            }
            LAST_SCROLL_OFFSET = scroll_offset;
            LINES_DRAWN = line_count;
        } else if line_count > LINES_DRAWN {
            // No scrolling - just draw NEW lines (typewriter effect)
            // Don't clear - background is already black from initial clear
            for line_idx in LINES_DRAWN..line_count {
                let display_pos = line_idx - scroll_offset;
                if display_pos < visible_lines {
                    let y = MARGIN_TOP + (display_pos as u32 * FONT_HEIGHT);
                    // No clear needed - just draw on black background
                    if let Some(text) = CONSOLE.get_line(line_idx) {
                        draw_text(text, MARGIN_LEFT, y, COLOR_TEXT);
                    }
                }
            }
            LINES_DRAWN = line_count;
        }
    }
    
    // Mark screen as dirty (boot_console draws directly to framebuffer without tracking)
    crate::d1_display::mark_all_dirty();
    // Flush to display
    crate::d1_display::flush();
}

/// Scroll the framebuffer content up by the specified number of lines
/// This copies pixels from lower rows to upper rows, preserving content visually
fn scroll_framebuffer_up(lines: u32) {
    use crate::d1_display::BACK_BUFFER_ADDR;
    
    let scroll_pixels = lines * FONT_HEIGHT;
    let text_area_height = DISPLAY_HEIGHT - MARGIN_TOP * 2;
    
    // Only scroll if there's content to preserve
    if scroll_pixels >= text_area_height {
        return;
    }
    
    unsafe {
        let fb_ptr = BACK_BUFFER_ADDR as *mut u32;
        
        // Copy each row from (y + scroll_pixels) to y
        // Start from top and work down to avoid overwriting source data
        for y in MARGIN_TOP..(DISPLAY_HEIGHT - MARGIN_TOP - scroll_pixels) {
            let src_y = y + scroll_pixels;
            for x in MARGIN_LEFT..(DISPLAY_WIDTH - MARGIN_LEFT) {
                let src_idx = (src_y * DISPLAY_WIDTH + x) as usize;
                let dst_idx = (y * DISPLAY_WIDTH + x) as usize;
                let pixel = core::ptr::read_volatile(fb_ptr.add(src_idx));
                core::ptr::write_volatile(fb_ptr.add(dst_idx), pixel);
            }
        }
    }
}

/// Draw text at position using simple pixel font
/// This is a minimal implementation - draws basic ASCII chars
fn draw_text(text: &str, x: u32, y: u32, color: u32) {
    use crate::d1_display::BACK_BUFFER_ADDR;
    
    let mut cx = x;
    for ch in text.bytes() {
        if ch >= 32 && ch < 127 {
            draw_char(ch, cx, y, color);
        }
        cx += FONT_WIDTH;
        
        // Stop at edge of screen
        if cx + FONT_WIDTH > DISPLAY_WIDTH - MARGIN_LEFT {
            break;
        }
    }
}

/// Draw a single character (simple 6x10 bitmap font)
fn draw_char(ch: u8, x: u32, y: u32, color: u32) {
    use crate::d1_display::BACK_BUFFER_ADDR;
    
    let glyph = get_glyph(ch);
    
    unsafe {
        let fb_ptr = BACK_BUFFER_ADDR as *mut u32;
        
        for row in 0..FONT_HEIGHT.min(10) {
            let bits = glyph[row as usize];
            for col in 0..FONT_WIDTH.min(6) {
                if (bits >> (5 - col)) & 1 != 0 {
                    let px = x + col;
                    let py = y + row;
                    if px < DISPLAY_WIDTH && py < DISPLAY_HEIGHT {
                        let idx = (py * DISPLAY_WIDTH + px) as usize;
                        core::ptr::write_volatile(fb_ptr.add(idx), color);
                    }
                }
            }
        }
    }
}

/// Get glyph bitmap for ASCII character (6x10 font, 10 rows of 6 bits)
/// Returns array of 10 bytes, each representing a row (MSB = leftmost pixel)
fn get_glyph(ch: u8) -> [u8; 10] {
    // Minimal font - just enough for boot messages
    match ch {
        // Space
        b' ' => [0; 10],
        
        // Common characters for boot messages
        b'[' => [0b011000, 0b010000, 0b010000, 0b010000, 0b010000, 0b010000, 0b010000, 0b010000, 0b011000, 0],
        b']' => [0b011000, 0b001000, 0b001000, 0b001000, 0b001000, 0b001000, 0b001000, 0b001000, 0b011000, 0],
        b':' => [0b000000, 0b000000, 0b011000, 0b011000, 0b000000, 0b000000, 0b011000, 0b011000, 0b000000, 0],
        b'.' => [0b000000, 0b000000, 0b000000, 0b000000, 0b000000, 0b000000, 0b000000, 0b011000, 0b011000, 0],
        b',' => [0b000000, 0b000000, 0b000000, 0b000000, 0b000000, 0b000000, 0b011000, 0b011000, 0b010000, 0b100000],
        b'-' => [0b000000, 0b000000, 0b000000, 0b000000, 0b111110, 0b000000, 0b000000, 0b000000, 0b000000, 0],
        b'_' => [0b000000, 0b000000, 0b000000, 0b000000, 0b000000, 0b000000, 0b000000, 0b000000, 0b111111, 0],
        b'/' => [0b000010, 0b000010, 0b000100, 0b000100, 0b001000, 0b001000, 0b010000, 0b010000, 0b100000, 0],
        b'=' => [0b000000, 0b000000, 0b111110, 0b000000, 0b000000, 0b111110, 0b000000, 0b000000, 0b000000, 0],
        b'+' => [0b000000, 0b001000, 0b001000, 0b001000, 0b111110, 0b001000, 0b001000, 0b001000, 0b000000, 0],
        b'(' => [0b000100, 0b001000, 0b010000, 0b010000, 0b010000, 0b010000, 0b010000, 0b001000, 0b000100, 0],
        b')' => [0b100000, 0b010000, 0b001000, 0b001000, 0b001000, 0b001000, 0b001000, 0b010000, 0b100000, 0],
        b'!' => [0b001000, 0b001000, 0b001000, 0b001000, 0b001000, 0b001000, 0b000000, 0b001000, 0b001000, 0],
        
        // Digits 0-9
        b'0' => [0b011100, 0b100010, 0b100110, 0b101010, 0b110010, 0b100010, 0b100010, 0b011100, 0b000000, 0],
        b'1' => [0b001000, 0b011000, 0b001000, 0b001000, 0b001000, 0b001000, 0b001000, 0b011100, 0b000000, 0],
        b'2' => [0b011100, 0b100010, 0b000010, 0b000100, 0b001000, 0b010000, 0b100000, 0b111110, 0b000000, 0],
        b'3' => [0b011100, 0b100010, 0b000010, 0b001100, 0b000010, 0b000010, 0b100010, 0b011100, 0b000000, 0],
        b'4' => [0b000100, 0b001100, 0b010100, 0b100100, 0b111110, 0b000100, 0b000100, 0b000100, 0b000000, 0],
        b'5' => [0b111110, 0b100000, 0b111100, 0b000010, 0b000010, 0b000010, 0b100010, 0b011100, 0b000000, 0],
        b'6' => [0b011100, 0b100000, 0b100000, 0b111100, 0b100010, 0b100010, 0b100010, 0b011100, 0b000000, 0],
        b'7' => [0b111110, 0b000010, 0b000100, 0b001000, 0b010000, 0b010000, 0b010000, 0b010000, 0b000000, 0],
        b'8' => [0b011100, 0b100010, 0b100010, 0b011100, 0b100010, 0b100010, 0b100010, 0b011100, 0b000000, 0],
        b'9' => [0b011100, 0b100010, 0b100010, 0b011110, 0b000010, 0b000010, 0b100010, 0b011100, 0b000000, 0],
        
        // Uppercase A-Z
        b'A' => [0b001000, 0b010100, 0b100010, 0b100010, 0b111110, 0b100010, 0b100010, 0b100010, 0b000000, 0],
        b'B' => [0b111100, 0b100010, 0b100010, 0b111100, 0b100010, 0b100010, 0b100010, 0b111100, 0b000000, 0],
        b'C' => [0b011100, 0b100010, 0b100000, 0b100000, 0b100000, 0b100000, 0b100010, 0b011100, 0b000000, 0],
        b'D' => [0b111100, 0b100010, 0b100010, 0b100010, 0b100010, 0b100010, 0b100010, 0b111100, 0b000000, 0],
        b'E' => [0b111110, 0b100000, 0b100000, 0b111100, 0b100000, 0b100000, 0b100000, 0b111110, 0b000000, 0],
        b'F' => [0b111110, 0b100000, 0b100000, 0b111100, 0b100000, 0b100000, 0b100000, 0b100000, 0b000000, 0],
        b'G' => [0b011100, 0b100010, 0b100000, 0b100000, 0b100110, 0b100010, 0b100010, 0b011100, 0b000000, 0],
        b'H' => [0b100010, 0b100010, 0b100010, 0b111110, 0b100010, 0b100010, 0b100010, 0b100010, 0b000000, 0],
        b'I' => [0b011100, 0b001000, 0b001000, 0b001000, 0b001000, 0b001000, 0b001000, 0b011100, 0b000000, 0],
        b'J' => [0b001110, 0b000100, 0b000100, 0b000100, 0b000100, 0b100100, 0b100100, 0b011000, 0b000000, 0],
        b'K' => [0b100010, 0b100100, 0b101000, 0b110000, 0b101000, 0b100100, 0b100010, 0b100010, 0b000000, 0],
        b'L' => [0b100000, 0b100000, 0b100000, 0b100000, 0b100000, 0b100000, 0b100000, 0b111110, 0b000000, 0],
        b'M' => [0b100010, 0b110110, 0b101010, 0b101010, 0b100010, 0b100010, 0b100010, 0b100010, 0b000000, 0],
        b'N' => [0b100010, 0b110010, 0b101010, 0b100110, 0b100010, 0b100010, 0b100010, 0b100010, 0b000000, 0],
        b'O' => [0b011100, 0b100010, 0b100010, 0b100010, 0b100010, 0b100010, 0b100010, 0b011100, 0b000000, 0],
        b'P' => [0b111100, 0b100010, 0b100010, 0b111100, 0b100000, 0b100000, 0b100000, 0b100000, 0b000000, 0],
        b'Q' => [0b011100, 0b100010, 0b100010, 0b100010, 0b100010, 0b101010, 0b100100, 0b011010, 0b000000, 0],
        b'R' => [0b111100, 0b100010, 0b100010, 0b111100, 0b101000, 0b100100, 0b100010, 0b100010, 0b000000, 0],
        b'S' => [0b011100, 0b100010, 0b100000, 0b011100, 0b000010, 0b000010, 0b100010, 0b011100, 0b000000, 0],
        b'T' => [0b111110, 0b001000, 0b001000, 0b001000, 0b001000, 0b001000, 0b001000, 0b001000, 0b000000, 0],
        b'U' => [0b100010, 0b100010, 0b100010, 0b100010, 0b100010, 0b100010, 0b100010, 0b011100, 0b000000, 0],
        b'V' => [0b100010, 0b100010, 0b100010, 0b100010, 0b100010, 0b010100, 0b010100, 0b001000, 0b000000, 0],
        b'W' => [0b100010, 0b100010, 0b100010, 0b100010, 0b101010, 0b101010, 0b110110, 0b100010, 0b000000, 0],
        b'X' => [0b100010, 0b100010, 0b010100, 0b001000, 0b010100, 0b100010, 0b100010, 0b100010, 0b000000, 0],
        b'Y' => [0b100010, 0b100010, 0b010100, 0b001000, 0b001000, 0b001000, 0b001000, 0b001000, 0b000000, 0],
        b'Z' => [0b111110, 0b000010, 0b000100, 0b001000, 0b010000, 0b100000, 0b100000, 0b111110, 0b000000, 0],
        
        // Lowercase a-z
        b'a' => [0b000000, 0b000000, 0b011100, 0b000010, 0b011110, 0b100010, 0b100010, 0b011110, 0b000000, 0],
        b'b' => [0b100000, 0b100000, 0b111100, 0b100010, 0b100010, 0b100010, 0b100010, 0b111100, 0b000000, 0],
        b'c' => [0b000000, 0b000000, 0b011100, 0b100010, 0b100000, 0b100000, 0b100010, 0b011100, 0b000000, 0],
        b'd' => [0b000010, 0b000010, 0b011110, 0b100010, 0b100010, 0b100010, 0b100010, 0b011110, 0b000000, 0],
        b'e' => [0b000000, 0b000000, 0b011100, 0b100010, 0b111110, 0b100000, 0b100010, 0b011100, 0b000000, 0],
        b'f' => [0b001100, 0b010010, 0b010000, 0b111000, 0b010000, 0b010000, 0b010000, 0b010000, 0b000000, 0],
        b'g' => [0b000000, 0b000000, 0b011110, 0b100010, 0b100010, 0b011110, 0b000010, 0b011100, 0b000000, 0],
        b'h' => [0b100000, 0b100000, 0b111100, 0b100010, 0b100010, 0b100010, 0b100010, 0b100010, 0b000000, 0],
        b'i' => [0b001000, 0b000000, 0b011000, 0b001000, 0b001000, 0b001000, 0b001000, 0b011100, 0b000000, 0],
        b'j' => [0b000100, 0b000000, 0b001100, 0b000100, 0b000100, 0b000100, 0b100100, 0b011000, 0b000000, 0],
        b'k' => [0b100000, 0b100000, 0b100100, 0b101000, 0b110000, 0b101000, 0b100100, 0b100010, 0b000000, 0],
        b'l' => [0b011000, 0b001000, 0b001000, 0b001000, 0b001000, 0b001000, 0b001000, 0b011100, 0b000000, 0],
        b'm' => [0b000000, 0b000000, 0b110100, 0b101010, 0b101010, 0b101010, 0b101010, 0b100010, 0b000000, 0],
        b'n' => [0b000000, 0b000000, 0b111100, 0b100010, 0b100010, 0b100010, 0b100010, 0b100010, 0b000000, 0],
        b'o' => [0b000000, 0b000000, 0b011100, 0b100010, 0b100010, 0b100010, 0b100010, 0b011100, 0b000000, 0],
        b'p' => [0b000000, 0b000000, 0b111100, 0b100010, 0b100010, 0b111100, 0b100000, 0b100000, 0b000000, 0],
        b'q' => [0b000000, 0b000000, 0b011110, 0b100010, 0b100010, 0b011110, 0b000010, 0b000010, 0b000000, 0],
        b'r' => [0b000000, 0b000000, 0b101100, 0b110010, 0b100000, 0b100000, 0b100000, 0b100000, 0b000000, 0],
        b's' => [0b000000, 0b000000, 0b011110, 0b100000, 0b011100, 0b000010, 0b000010, 0b111100, 0b000000, 0],
        b't' => [0b010000, 0b010000, 0b111000, 0b010000, 0b010000, 0b010000, 0b010010, 0b001100, 0b000000, 0],
        b'u' => [0b000000, 0b000000, 0b100010, 0b100010, 0b100010, 0b100010, 0b100110, 0b011010, 0b000000, 0],
        b'v' => [0b000000, 0b000000, 0b100010, 0b100010, 0b100010, 0b010100, 0b010100, 0b001000, 0b000000, 0],
        b'w' => [0b000000, 0b000000, 0b100010, 0b100010, 0b101010, 0b101010, 0b101010, 0b010100, 0b000000, 0],
        b'x' => [0b000000, 0b000000, 0b100010, 0b010100, 0b001000, 0b010100, 0b100010, 0b100010, 0b000000, 0],
        b'y' => [0b000000, 0b000000, 0b100010, 0b100010, 0b011110, 0b000010, 0b000010, 0b011100, 0b000000, 0],
        b'z' => [0b000000, 0b000000, 0b111110, 0b000100, 0b001000, 0b010000, 0b100000, 0b111110, 0b000000, 0],
        
        // Unknown character - draw a box
        _ => [0b111110, 0b100010, 0b100010, 0b100010, 0b100010, 0b100010, 0b100010, 0b111110, 0b000000, 0],
    }
}
