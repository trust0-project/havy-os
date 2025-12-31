//! ELF Loader for Native RISC-V Binaries
//!
//! This module loads and executes position-independent RISC-V 64-bit ELF
//! executables. Binaries are loaded into heap-allocated memory.
//!
//! ## Supported Features
//! - RISC-V 64-bit little-endian ELF
//! - Position-independent executables (PIE)
//! - PT_LOAD segments

use alloc::vec::Vec;
use alloc::vec;
use alloc::boxed::Box;
use core::slice;

/// ELF Magic: 0x7f 'E' 'L' 'F'
const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

/// ELF Class: 64-bit
const ELFCLASS64: u8 = 2;

/// ELF Data: Little-endian
const ELFDATA2LSB: u8 = 1;

/// ELF Machine: RISC-V
const EM_RISCV: u16 = 0xf3;

/// Program header type: Loadable segment
const PT_LOAD: u32 = 1;

/// Debug flag: set when ELF exits, checked by shell_tick
pub static ELF_JUST_EXITED: core::sync::atomic::AtomicBool = 
    core::sync::atomic::AtomicBool::new(false);


/// ELF64 Header (size: 64 bytes)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
struct Elf64Header {
    e_ident: [u8; 16],
    e_type: u16,
    e_machine: u16,
    e_version: u32,
    e_entry: u64,
    e_phoff: u64,
    e_shoff: u64,
    e_flags: u32,
    e_ehsize: u16,
    e_phentsize: u16,
    e_phnum: u16,
    e_shentsize: u16,
    e_shnum: u16,
    e_shstrndx: u16,
}

/// ELF64 Program Header (size: 56 bytes)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
struct Elf64ProgramHeader {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_paddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
}

/// Result of loading an ELF binary
pub struct LoadedElf {
    /// Entry point address (adjusted for load address)
    pub entry: u64,
    /// Memory holding the loaded binary (must be kept alive during execution)
    pub memory: Box<[u8]>,
    /// Base address where binary was loaded
    pub load_base: u64,
}

/// ELF loading error
#[derive(Debug)]
pub enum ElfError {
    InvalidMagic,
    WrongClass,
    WrongEndian,
    WrongArch,
    TooSmall,
    InvalidProgramHeader,
    NoLoadableSegments,
}

/// Validate an ELF header
fn validate_header(header: &Elf64Header) -> Result<(), ElfError> {
    if header.e_ident[0..4] != ELF_MAGIC {
        return Err(ElfError::InvalidMagic);
    }
    if header.e_ident[4] != ELFCLASS64 {
        return Err(ElfError::WrongClass);
    }
    if header.e_ident[5] != ELFDATA2LSB {
        return Err(ElfError::WrongEndian);
    }
    if header.e_machine != EM_RISCV {
        return Err(ElfError::WrongArch);
    }
    Ok(())
}

/// Load an ELF binary into heap-allocated memory
///
/// For PIE binaries, we allocate memory and load at offset 0.
/// The entry point is adjusted to the allocated address.
pub fn load_elf(bytes: &[u8]) -> Result<LoadedElf, ElfError> {
    if bytes.len() < core::mem::size_of::<Elf64Header>() {
        return Err(ElfError::TooSmall);
    }
    
    // Parse header
    let header: Elf64Header = unsafe {
        core::ptr::read_unaligned(bytes.as_ptr() as *const Elf64Header)
    };
    
    validate_header(&header)?;
    
    // Find the memory range needed
    let phoff = header.e_phoff as usize;
    let phentsize = header.e_phentsize as usize;
    let phnum = header.e_phnum as usize;
    
    let mut min_vaddr: u64 = u64::MAX;
    let mut max_vaddr: u64 = 0;
    
    for i in 0..phnum {
        let ph_offset = phoff + i * phentsize;
        if ph_offset + phentsize > bytes.len() {
            return Err(ElfError::InvalidProgramHeader);
        }
        
        let ph: Elf64ProgramHeader = unsafe {
            core::ptr::read_unaligned(bytes.as_ptr().add(ph_offset) as *const Elf64ProgramHeader)
        };
        
        if ph.p_type != PT_LOAD {
            continue;
        }
        
        let vaddr = ph.p_vaddr;
        let memsz = ph.p_memsz;
        
        if vaddr < min_vaddr {
            min_vaddr = vaddr;
        }
        if vaddr + memsz > max_vaddr {
            max_vaddr = vaddr + memsz;
        }
    }
    
    if min_vaddr == u64::MAX {
        return Err(ElfError::NoLoadableSegments);
    }
    
    // Allocate memory for the entire binary
    let total_size = (max_vaddr - min_vaddr) as usize;
    let mut memory: Box<[u8]> = vec![0u8; total_size].into_boxed_slice();
    let load_base = memory.as_ptr() as u64;
    
    // Load segments into allocated memory
    for i in 0..phnum {
        let ph_offset = phoff + i * phentsize;
        let ph: Elf64ProgramHeader = unsafe {
            core::ptr::read_unaligned(bytes.as_ptr().add(ph_offset) as *const Elf64ProgramHeader)
        };
        
        if ph.p_type != PT_LOAD {
            continue;
        }
        
        let vaddr = ph.p_vaddr;
        let filesz = ph.p_filesz as usize;
        let offset = ph.p_offset as usize;
        
        // Calculate destination in our allocated buffer
        let dest_offset = (vaddr - min_vaddr) as usize;
        
        // Copy file data
        if filesz > 0 && offset + filesz <= bytes.len() {
            memory[dest_offset..dest_offset + filesz]
                .copy_from_slice(&bytes[offset..offset + filesz]);
        }
        // .bss is already zeroed from vec![0u8; ...]
    }
    
    // Calculate entry point adjusted for our load base
    let entry = load_base + (header.e_entry - min_vaddr);
    
    Ok(LoadedElf {
        entry,
        memory,
        load_base,
    })
}

/// Check if bytes appear to be an ELF file
#[inline]
pub fn is_elf(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[0..4] == ELF_MAGIC
}

/// Execute a loaded ELF binary in S-mode (no user mode switch)
/// 
/// This is a simpler execution model that runs the binary as a function call
/// in supervisor mode. Used by GUI terminal where we need normal return flow.
/// The binary's ecalls will trap and be handled, but we stay in S-mode.
pub fn execute_elf_smode(loaded: &LoadedElf, args: &[&str]) -> i32 {
    use core::arch::asm;
    
    // Convert args to static refs for syscall context
    let static_args: &'static [&'static str] = unsafe {
        core::mem::transmute(args)
    };
    
    // Initialize syscall context with args
    crate::syscall::init_context(static_args);
    
    let entry = loaded.entry;
    
    // Allocate a stack for the binary (8KB)
    let stack: alloc::boxed::Box<[u8]> = alloc::vec![0u8; 8192].into_boxed_slice();
    let stack_top = stack.as_ptr() as u64 + stack.len() as u64;
    
    // Run the binary as a function call in S-mode
    // Save callee-saved registers, switch stack, call entry, restore
    let exit_code: i64;
    unsafe {
        asm!(
            // Save callee-saved registers on current stack
            "addi sp, sp, -112",
            "sd ra, 0(sp)",
            "sd s0, 8(sp)",
            "sd s1, 16(sp)",
            "sd s2, 24(sp)",
            "sd s3, 32(sp)",
            "sd s4, 40(sp)",
            "sd s5, 48(sp)",
            "sd s6, 56(sp)",
            "sd s7, 64(sp)",
            "sd s8, 72(sp)",
            "sd s9, 80(sp)",
            "sd s10, 88(sp)",
            "sd s11, 96(sp)",
            "sd gp, 104(sp)",  // Save gp too
            
            // Save current sp to s0 (will be preserved)
            "mv s0, sp",
            
            // Switch to binary's stack
            "mv sp, {user_sp}",
            
            // Call the entry point as a function
            "jalr {entry}",
            
            // Restore kernel stack
            "mv sp, s0",
            
            // Return value is in a0
            "mv {ret}, a0",
            
            // Restore callee-saved registers
            "ld gp, 104(sp)",
            "ld s11, 96(sp)",
            "ld s10, 88(sp)",
            "ld s9, 80(sp)",
            "ld s8, 72(sp)",
            "ld s7, 64(sp)",
            "ld s6, 56(sp)",
            "ld s5, 48(sp)",
            "ld s4, 40(sp)",
            "ld s3, 32(sp)",
            "ld s2, 24(sp)",
            "ld s1, 16(sp)",
            "ld s0, 8(sp)",
            "ld ra, 0(sp)",
            "addi sp, sp, 112",
            
            entry = in(reg) entry,
            user_sp = in(reg) stack_top,
            ret = out(reg) exit_code,
            // Clobbers - all caller-saved registers
            out("t0") _,
            out("t1") _,
            out("t2") _,
            out("t3") _,
            out("t4") _,
            out("t5") _,
            out("t6") _,
            out("a1") _,
            out("a2") _,
            out("a3") _,
            out("a4") _,
            out("a5") _,
            out("a6") _,
            out("a7") _,
        );
    }
    
    // Clear syscall context
    crate::syscall::clear_context();
    
    // Keep memory alive until here
    drop(stack);
    drop(loaded.memory.as_ref());
    
    exit_code as i32
}

/// Kernel context saved before entering user mode
/// Used to return from SYS_EXIT
#[repr(C)]
pub struct KernelContext {
    pub ra: u64,
    pub sp: u64,
    pub s0: u64,
    pub s1: u64,
    pub s2: u64,
    pub s3: u64,
    pub s4: u64,
    pub s5: u64,
    pub s6: u64,
    pub s7: u64,
    pub s8: u64,
    pub s9: u64,
    pub s10: u64,
    pub s11: u64,
    pub exit_code: i32,
    pub exited: bool,
    /// True if executed from GUI context - restore_kernel_context should return, not jump to hart_loop
    pub gui_mode: bool,
}

/// Global kernel context for returning from user mode
/// SAFETY: Only accessed from the hart running the binary
static mut KERNEL_CTX: Option<KernelContext> = None;

/// Flag set when GUI execution completes - checked by caller
static GUI_EXECUTION_DONE: core::sync::atomic::AtomicBool = 
    core::sync::atomic::AtomicBool::new(false);

/// Check if GUI execution is done (and clear the flag)
pub fn check_gui_done() -> bool {
    GUI_EXECUTION_DONE.swap(false, core::sync::atomic::Ordering::SeqCst)
}

/// Get exit code from last execution
pub fn get_last_exit_code() -> i32 {
    unsafe {
        KERNEL_CTX.as_ref().map(|ctx| ctx.exit_code).unwrap_or(0)
    }
}

/// Called by SYS_EXIT to signal binary termination
pub fn signal_exit(code: i32) {
    unsafe {
        if let Some(ctx) = KERNEL_CTX.as_mut() {
            ctx.exit_code = code;
            ctx.exited = true;
        }
    }
}

/// Check if binary has exited
pub fn has_exited() -> Option<i32> {
    unsafe {
        KERNEL_CTX.as_ref().and_then(|ctx| {
            if ctx.exited { 
                Some(ctx.exit_code) 
            } else { 
                None 
            }
        })
    }
}

/// Execute a loaded ELF binary
///
/// This function sets up the syscall context and uses `sret` to switch to U-mode
/// for proper ecall handling. The binary's ecalls will trap to S-mode.
/// 
/// caller_ra and caller_sp are the return frame from the caller (run_script_bytes),
/// captured BEFORE calling this function to avoid Rust prologue clobbering them.
pub fn execute_elf(loaded: &LoadedElf, args: &[&str], caller_ra: u64, caller_sp: u64) -> i32 {
    use core::arch::asm;
    
    // Convert args to static refs
    let static_args: &'static [&'static str] = unsafe {
        core::mem::transmute(args)
    };
    
    // Initialize syscall context
    crate::syscall::init_context(static_args);

    
    // Initialize kernel context for return
    // Check if we're in GUI mode - execution should return normally
    let in_gui_mode = crate::scripting::is_gui_context();
    unsafe {
        KERNEL_CTX = Some(KernelContext {
            ra: 0, sp: 0,
            s0: 0, s1: 0, s2: 0, s3: 0, s4: 0, s5: 0,
            s6: 0, s7: 0, s8: 0, s9: 0, s10: 0, s11: 0,
            exit_code: 0,
            exited: false,
            gui_mode: in_gui_mode,
        });
    }
    
    let entry = loaded.entry;
    
    // Allocate a stack for the binary (8KB)  
    let stack: alloc::boxed::Box<[u8]> = alloc::vec![0u8; 8192].into_boxed_slice();
    let stack_top = stack.as_ptr() as u64 + stack.len() as u64;
    
    // Get pointer to kernel context
    let ctx_ptr = unsafe { KERNEL_CTX.as_mut().unwrap() as *mut KernelContext };
    
    // Save kernel context using the caller's ra/sp passed as parameters
    // (captured in run_script_bytes before calling this function)
    unsafe {
        asm!(
            // Save caller's ra (return address to run_script_bytes' caller)
            "sd {caller_ra}, 0({ctx})",
            // Save caller's sp  
            "sd {caller_sp}, 8({ctx})",
            // Save callee-saved registers (these are still valid)
            "sd s0, 16({ctx})",
            "sd s1, 24({ctx})",
            "sd s2, 32({ctx})",
            "sd s3, 40({ctx})",
            "sd s4, 48({ctx})",
            "sd s5, 56({ctx})",
            "sd s6, 64({ctx})",
            "sd s7, 72({ctx})",
            "sd s8, 80({ctx})",
            "sd s9, 88({ctx})",
            "sd s10, 96({ctx})",
            "sd s11, 104({ctx})",
            
            // Set sepc to entry point
            "csrw sepc, {entry}",
            
            // Clear SPP (bit 8) to return to U-mode, set SPIE (bit 5)
            "li t0, 0xFFFFFFFFFFFFFEFF",
            "csrr t1, sstatus",
            "and t1, t1, t0",
            "ori t1, t1, 0x20",
            "csrw sstatus, t1",
            
            // Set up user stack pointer
            "mv sp, {user_sp}",
            
            // sret to user mode
            "sret",
            
            ctx = in(reg) ctx_ptr,
            caller_ra = in(reg) caller_ra,
            caller_sp = in(reg) caller_sp,
            entry = in(reg) entry,
            user_sp = in(reg) stack_top,
            options(noreturn)
        );
    }
}

/// Restore kernel context and return from user mode
/// Called by trap handler when SYS_EXIT is detected
#[inline(never)]
pub fn restore_kernel_context() -> ! {
    use core::arch::asm;
    
    // Check if we're in GUI mode BEFORE clearing context
    let gui_mode = unsafe {
        KERNEL_CTX.as_ref().map(|ctx| ctx.gui_mode).unwrap_or(false)
    };
    
    if gui_mode {
        // GUI MODE: Signal completion to GUI subsystem and return to hart_loop
        // We cannot jump back to the caller because we're in trap context
        
        // Get exit code
        let exit_code = unsafe {
            KERNEL_CTX.as_ref().map(|ctx| ctx.exit_code).unwrap_or(0)
        };
        
        // Signal completion to GUI command service
        crate::services::gui_cmd::signal_completion(exit_code);
        
        // Clear syscall context
        crate::syscall::clear_context();
        
        // Clear the kernel context
        unsafe {
            KERNEL_CTX = None;
        }
        
        // Get current hart ID
        let hart_id = crate::get_hart_id();
        
        // CRITICAL: Enable interrupts before entering hart_loop
        unsafe {
            asm!(
                "csrsi sstatus, 0x2",  // Set SIE (bit 1) to enable interrupts
                options(nomem, nostack)
            );
        }
        
        // Find and re-queue the gpuid process so it can pick up the result
        // Also re-queue gui_cmd for handling the next command
        for process in crate::cpu::process::PROCESS_TABLE.list() {
            if process.name == "gpuid" || process.name == "gui_cmd" {
                process.mark_ready();
                crate::cpu::sched::requeue(process, hart_id);
            }
        }
        
        // Re-enter the main hart loop - this never returns
        crate::cpu::hart_loop(hart_id);
    } else {
        // SHELL MODE: Original behavior - jump to hart_loop
        // Get exit code before clearing context  
        let _exit_code = unsafe {
            KERNEL_CTX.as_ref().map(|ctx| ctx.exit_code).unwrap_or(-1)
        };
        
        // Clear the kernel context
        unsafe {
            KERNEL_CTX = None;
        }
        
        // Set debug flag - shell_tick will check this
        ELF_JUST_EXITED.store(true, core::sync::atomic::Ordering::Release);
        
        // Print a new shell prompt
        crate::utils::print_prompt();
        
        // Get current hart ID
        let hart_id = crate::get_hart_id();
        
        // CRITICAL: Enable interrupts before entering hart_loop
        unsafe {
            asm!(
                "csrsi sstatus, 0x2",  // Set SIE (bit 1) to enable interrupts
                options(nomem, nostack)
            );
        }
        
        // CRITICAL: Re-queue the shell process
        crate::services::shelld::clear_buffer();
        
        for process in crate::cpu::process::PROCESS_TABLE.list() {
            if process.name == "shelld" {
                process.mark_ready();
                crate::cpu::sched::requeue(process, hart_id);
                break;
            }
        }
        
        // Re-enter the main hart loop - this never returns
        crate::cpu::hart_loop(hart_id);
    }
}

