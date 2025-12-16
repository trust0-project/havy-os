//! Trap Handler for RISC-V S-mode
//!
//! This module handles all supervisor-mode traps including:
//! - Timer interrupts (for preemptive scheduling) via SBI
//! - Software interrupts (IPIs for cross-hart communication)
//! - Exceptions (illegal instructions, page faults, etc.)
//!
//! ## Trap Flow
//!
//! ```text
//! Process Running (S-mode)
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
//! │ trap_exit   │ (assembly: restore registers, sret)
//! └─────────────┘
//! ```

use core::arch::asm;

/// Supervisor cause register values (scause)
/// Bit 63 (XLEN-1) is the interrupt bit: 1 = interrupt, 0 = exception
pub mod cause {
    // Interrupts (bit 63 set) - Supervisor mode
    pub const SUPERVISOR_SOFTWARE_INTERRUPT: usize = 0x8000_0000_0000_0001;
    pub const SUPERVISOR_TIMER_INTERRUPT: usize = 0x8000_0000_0000_0005;
    pub const SUPERVISOR_EXTERNAL_INTERRUPT: usize = 0x8000_0000_0000_0009;

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
    pub const INSTRUCTION_PAGE_FAULT: usize = 12;
    pub const LOAD_PAGE_FAULT: usize = 13;
    pub const STORE_PAGE_FAULT: usize = 15;
}

/// Timer interval in cycles (approximately 10ms at 10MHz)
const TIMER_INTERVAL: u64 = 100_000;

/// Read the current time via the `time` CSR
#[inline]
pub fn read_mtime() -> u64 {
    let time: u64;
    unsafe {
        asm!(
            "rdtime {}",
            out(reg) time,
            options(nomem, nostack)
        );
    }
    time
}

/// Schedule the next timer interrupt using SBI
pub fn schedule_timer_interrupt(_hart_id: usize) {
    let current = read_mtime();
    crate::sbi::set_timer(current.wrapping_add(TIMER_INTERVAL));
}

/// Enable supervisor-mode interrupts
pub fn enable_interrupts() {
    unsafe {
        // Enable SIE (Supervisor Interrupt Enable) in sstatus
        asm!(
            "csrsi sstatus, 0x2",  // Set SIE bit (bit 1)
            options(nomem, nostack)
        );
        
        // Enable timer, software, and external interrupts in sie
        // STIE = bit 5, SSIE = bit 1, SEIE = bit 9
        asm!(
            "li t0, 0x222",
            "csrs sie, t0",
            out("t0") _,
            options(nomem, nostack)
        );
    }
}

/// Disable supervisor-mode interrupts
#[allow(dead_code)]
pub fn disable_interrupts() {
    unsafe {
        asm!(
            "csrci sstatus, 0x2",  // Clear SIE bit
            options(nomem, nostack)
        );
    }
}

/// Check if interrupts are enabled
#[inline]
#[allow(dead_code)]
pub fn interrupts_enabled() -> bool {
    let sstatus: usize;
    unsafe {
        asm!(
            "csrr {}, sstatus",
            out(reg) sstatus,
            options(nomem, nostack)
        );
    }
    (sstatus & 0x2) != 0
}

/// Set the trap handler vector (stvec)
pub fn set_trap_vector(handler: usize) {
    unsafe {
        asm!(
            "csrw stvec, {}",
            in(reg) handler,
            options(nomem, nostack)
        );
    }
}

/// Read scause register
#[inline]
pub fn read_scause() -> usize {
    let scause: usize;
    unsafe {
        asm!(
            "csrr {}, scause",
            out(reg) scause,
            options(nomem, nostack)
        );
    }
    scause
}

/// Read sepc register (exception PC)
#[inline]
pub fn read_sepc() -> usize {
    let sepc: usize;
    unsafe {
        asm!(
            "csrr {}, sepc",
            out(reg) sepc,
            options(nomem, nostack)
        );
    }
    sepc
}

/// Read stval register (trap value)
#[inline]
pub fn read_stval() -> usize {
    let stval: usize;
    unsafe {
        asm!(
            "csrr {}, stval",
            out(reg) stval,
            options(nomem, nostack)
        );
    }
    stval
}

/// The main trap handler called from assembly
#[no_mangle]
pub extern "C" fn trap_handler() {
    let scause = read_scause();
    let hart_id = crate::get_hart_id();
    
    let is_interrupt = (scause as isize) < 0;
    let cause_code = scause & 0x7FFF_FFFF_FFFF_FFFF;
    
    if is_interrupt {
        match scause {
            cause::SUPERVISOR_TIMER_INTERRUPT => {
                handle_timer_interrupt(hart_id);
            }
            cause::SUPERVISOR_SOFTWARE_INTERRUPT => {
                handle_software_interrupt(hart_id);
            }
            cause::SUPERVISOR_EXTERNAL_INTERRUPT => {
                handle_external_interrupt(hart_id);
            }
            _ => {
                crate::klog::klog_warning(
                    "trap",
                    &alloc::format!("Unknown interrupt: cause={:#x} hart={}", scause, hart_id),
                );
            }
        }
    } else {
        handle_exception(hart_id, cause_code);
    }
}

/// Handle timer interrupt - triggers preemptive scheduling
fn handle_timer_interrupt(hart_id: usize) {
    if let Some(cpu) = crate::cpu::CPU_TABLE.get(hart_id) {
        cpu.enter_interrupt();
    }
    
    // Schedule next timer interrupt via SBI
    schedule_timer_interrupt(hart_id);
    
    // Set yield pending flag - actual context switch happens in hart_loop
    // NOTE: We cannot call switch_context() from here because we're inside
    // a trap handler that already saved registers to the stack. The actual
    // preemption happens when the trap returns and hart_loop checks the flag.
    crate::sched::yield_from_interrupt();
    
    if let Some(cpu) = crate::cpu::CPU_TABLE.get(hart_id) {
        cpu.exit_interrupt();
    }
}

/// Handle software interrupt (IPI) via SBI
fn handle_software_interrupt(hart_id: usize) {
    crate::sbi::clear_ipi();
    
    if let Some(cpu) = crate::cpu::CPU_TABLE.get(hart_id) {
        cpu.enter_interrupt();
        cpu.exit_interrupt();
    }
}

/// Handle external interrupt (PLIC)
fn handle_external_interrupt(hart_id: usize) {
    crate::klog::klog_trace(
        "trap",
        &alloc::format!("External interrupt on hart {}", hart_id),
    );
}

/// Handle exception (synchronous trap)
fn handle_exception(hart_id: usize, cause: usize) {
    let sepc = read_sepc();
    let stval = read_stval();
    
    match cause {
        cause::ECALL_FROM_U_MODE | cause::ECALL_FROM_S_MODE => {
            // Advance PC past ecall
            unsafe {
                asm!(
                    "csrr t0, sepc",
                    "addi t0, t0, 4",
                    "csrw sepc, t0",
                    out("t0") _,
                    options(nomem, nostack)
                );
            }
        }
        cause::BREAKPOINT => {
            crate::klog::klog_debug(
                "trap",
                &alloc::format!("Breakpoint at {:#x} on hart {}", sepc, hart_id),
            );
            unsafe {
                asm!(
                    "csrr t0, sepc",
                    "addi t0, t0, 2",
                    "csrw sepc, t0",
                    out("t0") _,
                    options(nomem, nostack)
                );
            }
        }
        _ => {
            panic!(
                "EXCEPTION on hart {}: cause={} sepc={:#x} stval={:#x}",
                hart_id, cause, sepc, stval
            );
        }
    }
}

// S-mode trap vector assembly
core::arch::global_asm!(r#"
.section .text
.global trap_vector_entry
.align 4
trap_vector_entry:
    addi sp, sp, -256
    
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
    
    call trap_handler
    
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
    
    sret
"#);

extern "C" {
    fn trap_vector_entry();
}

/// Initialize trap handling for a hart
pub fn init(hart_id: usize) {
    let handler_addr = trap_vector_entry as usize;
    set_trap_vector(handler_addr);
    
    schedule_timer_interrupt(hart_id);
    enable_interrupts();
    
    crate::klog::klog_info(
        "trap",
        &alloc::format!("S-mode trap handler initialized on hart {}", hart_id),
    );
}
