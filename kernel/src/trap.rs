//! Trap Handler for RISC-V M-mode
//!
//! This module handles all machine-mode traps including:
//! - Timer interrupts (for preemptive scheduling)
//! - Software interrupts (IPIs for cross-hart communication)
//! - Exceptions (illegal instructions, page faults, etc.)
//!
//! ## Trap Flow
//!
//! ```text
//! Process Running
//!       │
//!       ▼
//! ┌─────────────┐
//! │ Trap Occurs │ (timer, IPI, exception)
//! └─────────────┘
//!       │
//!       ▼
//! ┌─────────────┐
//! │ trap_entry  │ (assembly: save registers)
//! └─────────────┘
//!       │
//!       ▼
//! ┌─────────────┐
//! │trap_handler │ (Rust: dispatch based on cause)
//! └─────────────┘
//!       │
//!       ▼
//! ┌─────────────┐
//! │ trap_exit   │ (assembly: restore registers, mret)
//! └─────────────┘
//! ```

use core::arch::asm;

/// Machine cause register values (mcause)
/// Bit 63 (XLEN-1) is the interrupt bit: 1 = interrupt, 0 = exception
pub mod cause {
    // Interrupts (bit 63 set)
    pub const MACHINE_SOFTWARE_INTERRUPT: usize = 0x8000_0000_0000_0003;
    pub const MACHINE_TIMER_INTERRUPT: usize = 0x8000_0000_0000_0007;
    pub const MACHINE_EXTERNAL_INTERRUPT: usize = 0x8000_0000_0000_000B;

    // Exceptions (bit 63 clear)
    pub const INSTRUCTION_ADDRESS_MISALIGNED: usize = 0;
    pub const INSTRUCTION_ACCESS_FAULT: usize = 1;
    pub const ILLEGAL_INSTRUCTION: usize = 2;
    pub const BREAKPOINT: usize = 3;
    pub const LOAD_ADDRESS_MISALIGNED: usize = 4;
    pub const LOAD_ACCESS_FAULT: usize = 5;
    pub const STORE_ADDRESS_MISALIGNED: usize = 6;
    pub const STORE_ACCESS_FAULT: usize = 7;
    pub const ECALL_FROM_U_MODE: usize = 8;
    pub const ECALL_FROM_S_MODE: usize = 9;
    pub const ECALL_FROM_M_MODE: usize = 11;
    pub const INSTRUCTION_PAGE_FAULT: usize = 12;
    pub const LOAD_PAGE_FAULT: usize = 13;
    pub const STORE_PAGE_FAULT: usize = 15;
}

/// CLINT timer registers
const CLINT_MTIMECMP_BASE: usize = 0x0200_4000;
const CLINT_MTIME: usize = 0x0200_BFF8;

/// Timer interval in cycles (approximately 10ms at 10MHz)
/// Adjust based on actual clock frequency
const TIMER_INTERVAL: u64 = 100_000;

/// Read the current machine time
#[inline]
pub fn read_mtime() -> u64 {
    unsafe { core::ptr::read_volatile(CLINT_MTIME as *const u64) }
}

/// Set the timer compare value for a hart
#[inline]
pub fn set_mtimecmp(hart_id: usize, value: u64) {
    let addr = CLINT_MTIMECMP_BASE + (hart_id * 8);
    unsafe {
        core::ptr::write_volatile(addr as *mut u64, value);
    }
}

/// Schedule the next timer interrupt
pub fn schedule_timer_interrupt(hart_id: usize) {
    let current = read_mtime();
    set_mtimecmp(hart_id, current.wrapping_add(TIMER_INTERVAL));
}

/// Enable machine-mode interrupts
pub fn enable_interrupts() {
    unsafe {
        // Enable MIE (Machine Interrupt Enable) in mstatus
        asm!(
            "csrsi mstatus, 0x8",  // Set MIE bit (bit 3)
            options(nomem, nostack)
        );
        
        // Enable timer and software interrupts in mie
        // MTIE = bit 7, MSIE = bit 3
        asm!(
            "li t0, 0x88",
            "csrs mie, t0",
            out("t0") _,
            options(nomem, nostack)
        );
    }
}

/// Disable machine-mode interrupts
#[allow(dead_code)]
pub fn disable_interrupts() {
    unsafe {
        asm!(
            "csrci mstatus, 0x8",  // Clear MIE bit
            options(nomem, nostack)
        );
    }
}

/// Check if interrupts are enabled
#[inline]
#[allow(dead_code)]
pub fn interrupts_enabled() -> bool {
    let mstatus: usize;
    unsafe {
        asm!(
            "csrr {}, mstatus",
            out(reg) mstatus,
            options(nomem, nostack)
        );
    }
    (mstatus & 0x8) != 0
}

/// Set the trap handler vector
pub fn set_trap_vector(handler: usize) {
    unsafe {
        asm!(
            "csrw mtvec, {}",
            in(reg) handler,
            options(nomem, nostack)
        );
    }
}

/// Read mcause register
#[inline]
pub fn read_mcause() -> usize {
    let mcause: usize;
    unsafe {
        asm!(
            "csrr {}, mcause",
            out(reg) mcause,
            options(nomem, nostack)
        );
    }
    mcause
}

/// Read mepc register (exception PC)
#[inline]
pub fn read_mepc() -> usize {
    let mepc: usize;
    unsafe {
        asm!(
            "csrr {}, mepc",
            out(reg) mepc,
            options(nomem, nostack)
        );
    }
    mepc
}

/// Read mtval register (trap value - faulting address or instruction)
#[inline]
pub fn read_mtval() -> usize {
    let mtval: usize;
    unsafe {
        asm!(
            "csrr {}, mtval",
            out(reg) mtval,
            options(nomem, nostack)
        );
    }
    mtval
}

/// The main trap handler called from assembly
///
/// This function is called with interrupts disabled.
/// It dispatches to the appropriate handler based on mcause.
#[no_mangle]
pub extern "C" fn trap_handler() {
    let mcause = read_mcause();
    let hart_id = crate::get_hart_id();
    
    // Check if this is an interrupt (bit 63 set) or exception
    let is_interrupt = (mcause as isize) < 0;
    let cause_code = mcause & 0x7FFF_FFFF_FFFF_FFFF;
    
    if is_interrupt {
        match mcause {
            cause::MACHINE_TIMER_INTERRUPT => {
                handle_timer_interrupt(hart_id);
            }
            cause::MACHINE_SOFTWARE_INTERRUPT => {
                handle_software_interrupt(hart_id);
            }
            cause::MACHINE_EXTERNAL_INTERRUPT => {
                handle_external_interrupt(hart_id);
            }
            _ => {
                // Unknown interrupt - log and continue
                crate::klog::klog_warning(
                    "trap",
                    &alloc::format!("Unknown interrupt: cause={:#x} hart={}", mcause, hart_id),
                );
            }
        }
    } else {
        handle_exception(hart_id, cause_code);
    }
}

/// Handle timer interrupt - triggers preemptive scheduling
fn handle_timer_interrupt(hart_id: usize) {
    // Update CPU stats
    if let Some(cpu) = crate::cpu::CPU_TABLE.get(hart_id) {
        cpu.enter_interrupt();
    }
    
    // Schedule next timer interrupt
    schedule_timer_interrupt(hart_id);
    
    // Trigger a context switch by calling yield
    // The scheduler will pick the next process
    crate::sched::yield_from_interrupt();
    
    if let Some(cpu) = crate::cpu::CPU_TABLE.get(hart_id) {
        cpu.exit_interrupt();
    }
}

/// Handle software interrupt (IPI)
fn handle_software_interrupt(hart_id: usize) {
    // Clear the software interrupt
    crate::clear_msip(hart_id);
    
    // Software interrupts are used for cross-hart notifications
    // The hart loop will check for new work after returning
    if let Some(cpu) = crate::cpu::CPU_TABLE.get(hart_id) {
        cpu.enter_interrupt();
        cpu.exit_interrupt();
    }
}

/// Handle external interrupt (PLIC)
fn handle_external_interrupt(hart_id: usize) {
    // External interrupts come from PLIC (VirtIO, etc.)
    // For now, just log - actual handling would involve PLIC claim/complete
    crate::klog::klog_trace(
        "trap",
        &alloc::format!("External interrupt on hart {}", hart_id),
    );
}

/// Handle exception (synchronous trap)
fn handle_exception(hart_id: usize, cause: usize) {
    let mepc = read_mepc();
    let mtval = read_mtval();
    
    match cause {
        cause::ECALL_FROM_M_MODE => {
            // System call from kernel - not typically used in M-mode only kernels
            // For now, just advance PC past the ecall instruction
            unsafe {
                asm!(
                    "csrr t0, mepc",
                    "addi t0, t0, 4",
                    "csrw mepc, t0",
                    out("t0") _,
                    options(nomem, nostack)
                );
            }
        }
        cause::BREAKPOINT => {
            // Breakpoint instruction - useful for debugging
            crate::klog::klog_debug(
                "trap",
                &alloc::format!("Breakpoint at {:#x} on hart {}", mepc, hart_id),
            );
            // Advance past ebreak (compressed = 2 bytes, regular = 4 bytes)
            // We assume compressed ebreak for now
            unsafe {
                asm!(
                    "csrr t0, mepc",
                    "addi t0, t0, 2",
                    "csrw mepc, t0",
                    out("t0") _,
                    options(nomem, nostack)
                );
            }
        }
        _ => {
            // Fatal exception - panic
            panic!(
                "EXCEPTION on hart {}: cause={} mepc={:#x} mtval={:#x}",
                hart_id, cause, mepc, mtval
            );
        }
    }
}

// Include the trap vector assembly
core::arch::global_asm!(r#"
.section .text
.global trap_vector_entry
.align 4
trap_vector_entry:
    # Save all caller-saved registers to stack
    addi sp, sp, -256
    
    # Save all general purpose registers
    sd ra, 0(sp)
    sd t0, 8(sp)
    sd t1, 16(sp)
    sd t2, 24(sp)
    sd t3, 32(sp)
    sd t4, 40(sp)
    sd t5, 48(sp)
    sd t6, 56(sp)
    sd a0, 64(sp)
    sd a1, 72(sp)
    sd a2, 80(sp)
    sd a3, 88(sp)
    sd a4, 96(sp)
    sd a5, 104(sp)
    sd a6, 112(sp)
    sd a7, 120(sp)
    sd s0, 128(sp)
    sd s1, 136(sp)
    sd s2, 144(sp)
    sd s3, 152(sp)
    sd s4, 160(sp)
    sd s5, 168(sp)
    sd s6, 176(sp)
    sd s7, 184(sp)
    sd s8, 192(sp)
    sd s9, 200(sp)
    sd s10, 208(sp)
    sd s11, 216(sp)
    sd gp, 224(sp)
    sd tp, 232(sp)
    
    # Call the Rust trap handler
    call trap_handler
    
    # Restore all registers
    ld ra, 0(sp)
    ld t0, 8(sp)
    ld t1, 16(sp)
    ld t2, 24(sp)
    ld t3, 32(sp)
    ld t4, 40(sp)
    ld t5, 48(sp)
    ld t6, 56(sp)
    ld a0, 64(sp)
    ld a1, 72(sp)
    ld a2, 80(sp)
    ld a3, 88(sp)
    ld a4, 96(sp)
    ld a5, 104(sp)
    ld a6, 112(sp)
    ld a7, 120(sp)
    ld s0, 128(sp)
    ld s1, 136(sp)
    ld s2, 144(sp)
    ld s3, 152(sp)
    ld s4, 160(sp)
    ld s5, 168(sp)
    ld s6, 176(sp)
    ld s7, 184(sp)
    ld s8, 192(sp)
    ld s9, 200(sp)
    ld s10, 208(sp)
    ld s11, 216(sp)
    ld gp, 224(sp)
    ld tp, 232(sp)
    
    addi sp, sp, 256
    
    # Return from machine-mode trap
    mret
"#);

// External declaration for the trap vector entry point
extern "C" {
    fn trap_vector_entry();
}

/// Initialize trap handling for a hart
pub fn init(hart_id: usize) {
    // Set trap vector to direct mode (all traps go to same handler)
    // The handler address must be 4-byte aligned
    let handler_addr = trap_vector_entry as usize;
    set_trap_vector(handler_addr);
    
    // Schedule first timer interrupt
    schedule_timer_interrupt(hart_id);
    
    // Enable interrupts
    enable_interrupts();
    
    crate::klog::klog_info(
        "trap",
        &alloc::format!("Trap handler initialized on hart {}", hart_id),
    );
}
