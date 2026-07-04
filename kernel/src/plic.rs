//! PLIC (Platform-Level Interrupt Controller) Driver
//!
//! The PLIC aggregates external interrupts and presents them to harts.
//! Each hart has 2 contexts: M-mode (2*N) and S-mode (2*N+1).
//!
//! MMIO Layout (base = 0x0C00_0000):
//! - 0x000000: Priority registers (4 bytes each, sources 0-31)
//! - 0x001000: Pending bits (read-only)
//! - 0x002000 + 0x80*ctx: Enable bits per context
//! - 0x200000 + 0x1000*ctx: Threshold and claim/complete per context

use crate::services::klogd::klog_info;

/// PLIC base address (matches VM's plic.rs)
const PLIC_BASE: usize = 0x0C00_0000;

/// Predefined IRQ numbers (must match VM's definitions)
pub const VIRTIO_INPUT_IRQ: u32 = 1;
pub const D1_TOUCH_IRQ: u32 = 2;
pub const UART_IRQ: u32 = 10;

/// Get the S-mode context ID for a hart.
/// Hart N uses S-mode context 2*N + 1.
#[inline]
fn s_context(hart_id: usize) -> usize {
    hart_id * 2 + 1
}

/// Initialize PLIC for a hart.
///
/// Enables relevant interrupt sources and sets threshold to 0.
/// Must be called during boot for each hart.
pub fn init(hart_id: usize) {
    let ctx = s_context(hart_id);

    unsafe {
        // Set priorities for our interrupt sources
        // Priority 1 is sufficient (threshold 0 means all priorities > 0 fire)
        let prio_base = PLIC_BASE as *mut u32;
        core::ptr::write_volatile(prio_base.add(VIRTIO_INPUT_IRQ as usize), 1);
        core::ptr::write_volatile(prio_base.add(D1_TOUCH_IRQ as usize), 1);
        core::ptr::write_volatile(prio_base.add(UART_IRQ as usize), 1);

        // Enable interrupt sources for this context
        // Enable bits at 0x002000 + 0x80 * context
        let enable_addr = (PLIC_BASE + 0x002000 + 0x80 * ctx) as *mut u32;
        let enable_mask = (1u32 << VIRTIO_INPUT_IRQ)
            | (1u32 << D1_TOUCH_IRQ)
            | (1u32 << UART_IRQ);
        core::ptr::write_volatile(enable_addr, enable_mask);

        // Set threshold to 0 (accept all priorities > 0)
        let threshold_addr = (PLIC_BASE + 0x200000 + 0x1000 * ctx) as *mut u32;
        core::ptr::write_volatile(threshold_addr, 0);
    }

    klog_info(
        "plic",
        &alloc::format!("Initialized for hart {} (ctx {})", hart_id, ctx),
    );
}

/// Claim the highest priority pending interrupt.
///
/// Returns the IRQ number (0 = no interrupt pending).
///
/// IMPORTANT: After claiming, you MUST call `complete()` or the PLIC
/// will block further interrupts from this source.
pub fn claim(hart_id: usize) -> u32 {
    let ctx = s_context(hart_id);
    let claim_addr = (PLIC_BASE + 0x200000 + 0x1000 * ctx + 4) as *const u32;

    unsafe { core::ptr::read_volatile(claim_addr) }
}

/// Complete (acknowledge) an interrupt.
///
/// Call this after handling the interrupt to re-enable that source.
pub fn complete(hart_id: usize, irq: u32) {
    let ctx = s_context(hart_id);
    let complete_addr = (PLIC_BASE + 0x200000 + 0x1000 * ctx + 4) as *mut u32;

    unsafe { core::ptr::write_volatile(complete_addr, irq) }
}
