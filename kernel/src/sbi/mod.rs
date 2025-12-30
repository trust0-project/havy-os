//! SBI (Supervisor Binary Interface) call wrappers
//!
//! This module provides Rust wrappers for SBI calls, allowing the kernel
//! to interact with the SBI firmware (M-mode) from S-mode.
//!
//! ## Calling Convention
//!
//! - a7 = Extension ID (EID)
//! - a6 = Function ID (FID)
//! - a0-a5 = Arguments
//! - Returns: a0 = error code, a1 = value

use core::arch::asm;

// ============================================================================
// Extension IDs
// ============================================================================

/// Legacy Set Timer (0x00)
const EID_LEGACY_SET_TIMER: u64 = 0x00;
/// Legacy Console Putchar (0x01)
const EID_LEGACY_PUTCHAR: u64 = 0x01;
/// Legacy Console Getchar (0x02)
const EID_LEGACY_GETCHAR: u64 = 0x02;
/// Legacy Clear IPI (0x03)
const EID_LEGACY_CLEAR_IPI: u64 = 0x03;
/// Legacy Send IPI (0x04)
const EID_LEGACY_SEND_IPI: u64 = 0x04;
/// Legacy Shutdown (0x08)
const EID_LEGACY_SHUTDOWN: u64 = 0x08;

/// Timer Extension ("TIME" = 0x54494D45)
const EID_TIMER: u64 = 0x54494D45;
/// IPI Extension ("sPI" = 0x735049)
const EID_IPI: u64 = 0x735049;
/// System Reset Extension ("SRST" = 0x53525354)
const EID_SRST: u64 = 0x53525354;
/// Hart State Management Extension ("HSM" = 0x48534D)
const EID_HSM: u64 = 0x48534D;

// ============================================================================
// SBI Return Value
// ============================================================================

/// SBI call result
pub struct SbiRet {
    pub error: i64,
    pub value: i64,
}

impl SbiRet {
    /// Check if the call was successful
    pub fn is_ok(&self) -> bool {
        self.error == 0
    }
}

// ============================================================================
// Low-level SBI Call
// ============================================================================

/// Execute an SBI call
#[inline(always)]
fn sbi_call(eid: u64, fid: u64, a0: u64, a1: u64, a2: u64) -> SbiRet {
    let error: i64;
    let value: i64;
    unsafe {
        asm!(
            "ecall",
            in("a7") eid,
            in("a6") fid,
            inlateout("a0") a0 as i64 => error,
            inlateout("a1") a1 as i64 => value,
            in("a2") a2,
            options(nostack)
        );
    }
    SbiRet { error, value }
}

/// Execute an SBI call with no arguments
#[inline(always)]
fn sbi_call_0(eid: u64, fid: u64) -> SbiRet {
    sbi_call(eid, fid, 0, 0, 0)
}

/// Execute an SBI call with one argument
#[inline(always)]
fn sbi_call_1(eid: u64, fid: u64, a0: u64) -> SbiRet {
    sbi_call(eid, fid, a0, 0, 0)
}

/// Execute an SBI call with two arguments
#[inline(always)]
fn sbi_call_2(eid: u64, fid: u64, a0: u64, a1: u64) -> SbiRet {
    sbi_call(eid, fid, a0, a1, 0)
}

// ============================================================================
// Timer Extension
// ============================================================================

/// Set the timer for the next timer interrupt.
///
/// Programs mtimecmp[hart] to the given value and clears pending STIP.
#[inline]
pub fn set_timer(stime_value: u64) {
    // Use the standard Timer extension (preferred)
    sbi_call_1(EID_TIMER, 0, stime_value);
}

/// Set timer using legacy extension (for compatibility)
#[inline]
#[allow(dead_code)]
pub fn legacy_set_timer(stime_value: u64) {
    sbi_call_1(EID_LEGACY_SET_TIMER, 0, stime_value);
}

// ============================================================================
// IPI Extension
// ============================================================================

/// Send an IPI to the specified harts.
///
/// # Arguments
/// * `hart_mask` - Bit mask of target harts (bit N = hart hart_mask_base + N)
/// * `hart_mask_base` - Starting hart ID for the mask (-1 for all harts)
#[inline]
pub fn send_ipi(hart_mask: u64, hart_mask_base: i64) {
    sbi_call_2(EID_IPI, 0, hart_mask, hart_mask_base as u64);
}

/// Clear the pending IPI for the current hart (legacy)
#[inline]
pub fn clear_ipi() {
    sbi_call_0(EID_LEGACY_CLEAR_IPI, 0);
}

// ============================================================================
// Console Extension (Legacy)
// ============================================================================

/// Write a single character to the debug console.
#[inline]
pub fn console_putchar(c: u8) {
    sbi_call_1(EID_LEGACY_PUTCHAR, 0, c as u64);
}

/// Read a single character from the debug console.
///
/// Returns `Some(char)` if a character is available, `None` otherwise.
#[inline]
pub fn console_getchar() -> Option<u8> {
    let ret = sbi_call_0(EID_LEGACY_GETCHAR, 0);
    if ret.error >= 0 {
        Some(ret.error as u8)
    } else {
        None
    }
}

// ============================================================================
// System Reset Extension
// ============================================================================

/// Shutdown the system.
#[inline]
pub fn shutdown() -> ! {
    // Try SRST extension first
    sbi_call_2(EID_SRST, 0, 0, 0); // SHUTDOWN type, no reason
    
    // Fallback to legacy shutdown
    sbi_call_0(EID_LEGACY_SHUTDOWN, 0);
    
    // If SBI doesn't halt, loop forever
    loop {
        unsafe {
            asm!("wfi", options(nomem, nostack));
        }
    }
}

/// Reboot the system.
#[inline]
#[allow(dead_code)]
pub fn reboot() -> ! {
    // SRST with COLD_REBOOT type
    sbi_call_2(EID_SRST, 0, 1, 0);
    
    // If SBI doesn't reboot, loop forever
    loop {
        unsafe {
            asm!("wfi", options(nomem, nostack));
        }
    }
}

// ============================================================================
// Hart State Management Extension (HSM)
// ============================================================================

/// Start a hart (secondary CPU).
///
/// This is the OpenSBI-compliant way to start secondary harts during boot.
/// The started hart will begin executing at `start_addr` with:
/// - a0 = hart_id
/// - a1 = opaque value (typically DTB address)
///
/// # Arguments
/// * `hartid` - ID of the hart to start
/// * `start_addr` - Physical address where the hart should start executing
/// * `opaque` - Opaque value to pass in a1 register (typically DTB pointer)
///
/// # Returns
/// * SbiRet with error code (0 = success)
#[inline]
pub fn hart_start(hartid: usize, start_addr: u64, opaque: u64) -> SbiRet {
    sbi_call(EID_HSM, 0, hartid as u64, start_addr, opaque)
}

/// Get hart status.
///
/// # Arguments
/// * `hartid` - ID of the hart to query
///
/// # Returns
/// * SbiRet with status in value field:
///   - 0 = STARTED
///   - 1 = STOPPED
///   - 2 = START_PENDING
///   - 3 = STOP_PENDING
///   - 4 = SUSPENDED
///   - 5 = SUSPEND_PENDING
///   - 6 = RESUME_PENDING
#[inline]
#[allow(dead_code)]
pub fn hart_get_status(hartid: usize) -> SbiRet {
    sbi_call_1(EID_HSM, 2, hartid as u64)
}
