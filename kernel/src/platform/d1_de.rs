//! Allwinner D1 DE2 + TCON scanout of the Phase A reserved framebuffer.
//!
//! Programs mixer 0 UI layer 0 to the guest FB at `current::FB_ADDR`
//! (480×480, stride padded). The emulator still scrapes that buffer;
//! on silicon this is the real display pipeline.

use crate::platform::current;
use crate::platform::d1::{CCU_BASE, DE_BASE, TCON_LCD0};

const CCU_DE_BGR: usize = CCU_BASE + 0x060C;
const CCU_TCONLCD_BGR: usize = CCU_BASE + 0x064C;
const CCU_DE_CLK: usize = CCU_BASE + 0x0600;
const CCU_TCONLCD_CLK: usize = CCU_BASE + 0x0600; // shared PLL video path; gate via BGR

fn w32(addr: usize, v: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, v) }
}

fn r32(addr: usize) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

/// Ungate DE/TCON and point UI0 at the reserved scanout buffer.
pub fn init() {
    // De-assert reset (bit 16) and enable clock (bit 0).
    w32(CCU_DE_BGR, (1 << 16) | 1);
    w32(CCU_TCONLCD_BGR, (1 << 16) | 1);
    let _ = r32(CCU_DE_CLK);
    let _ = r32(CCU_TCONLCD_CLK);

    let w = current::DISPLAY_WIDTH;
    let h = current::DISPLAY_HEIGHT;
    let stride = current::FB_STRIDE as u32;
    let fb = current::FB_ADDR as u32;
    let size = ((h.saturating_sub(1)) << 16) | (w.saturating_sub(1));

    // Mixer 0 global
    w32(DE_BASE + 0x00, 1); // GLB_CTL enable
    w32(DE_BASE + 0x0C, size); // GLB_SIZE

    // Blender: pipe 0 from UI0, fill black
    w32(DE_BASE + 0x1000, 0x0000_0101);
    w32(DE_BASE + 0x1008, 0xFF00_0000);
    w32(DE_BASE + 0x1018, 0); // route ch0
    w32(DE_BASE + 0x1088, size); // output size

    // UI channel 0 (offset 0x2000 on sun8i mixer)
    const UI0: usize = DE_BASE + 0x2000;
    // ATTR: enable + XRGB8888 (fmt 4 << 8)
    w32(UI0 + 0x00, 1 | (4 << 8));
    w32(UI0 + 0x04, size); // SIZE
    w32(UI0 + 0x08, 0); // COORD
    w32(UI0 + 0x0C, stride); // PITCH
    w32(UI0 + 0x10, fb); // TOP_LADDR

    // TCON LCD0: basic enable, timing for 480×480 (ht/vt in character clocks).
    // GCTL bit 31 = enable.
    w32(TCON_LCD0 + 0x00, 1u32 << 31);
    w32(TCON_LCD0 + 0x60, size); // basic size
}

/// Re-publish the scanout address after a flush (double-buffer not used).
pub fn kick() {
    let fb = current::FB_ADDR as u32;
    w32(DE_BASE + 0x2010, fb);
}
