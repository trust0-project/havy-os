#![no_std]
#![no_main]

#[cfg(not(feature = "d1"))]
core::arch::global_asm!(
    ".global _max_hart_id",
    "_max_hart_id = 7",
    ".global _hart_stack_size",
    "_hart_stack_size = 0x20000"
);

#[cfg(feature = "d1")]
core::arch::global_asm!(
    ".global _max_hart_id",
    "_max_hart_id = 0",
    ".global _hart_stack_size",
    "_hart_stack_size = 0x20000"
);

mod allocator;
mod device;      
mod dns;
mod lock;
mod perfstat;
mod platform;   
mod wasm;
mod wasm_service;
mod utils;
mod dtb;
mod boot;
mod commands;

pub use lock::{
    Spinlock, 
    RwLock, 
    TicketLock, 
    fence_memory, 
    fence_acquire, 
    fence_release
};

mod fs;
mod net;
mod scripting;
mod tls;
mod tls12;
mod ui;
mod constants;
mod services;
mod init;
mod task;
mod clint;
mod cpu;
mod trap;
mod sbi;
mod plic;
mod syscall_numbers;
mod syscall;
mod elf_loader;
mod paging;
#[cfg(not(feature = "d1"))]
mod virtio_input;
mod input;
mod fd;

pub use cpu::CPU_TABLE;
pub use cpu::process::PROCESS_TABLE;
pub use sched::SCHEDULER as PROC_SCHEDULER;

extern crate alloc;
use panic_halt as _;
use riscv_rt::entry;
use crate::boot::init_boot;
use crate::clint::get_time_ms;
use crate::cpu::{HARTS_ONLINE, get_hart_id, hart_loop, sched, send_ipi};
use crate::device::uart;
use crate::lock::utils::{FS_STATE, NET_STATE, PING_STATE};
use crate::utils::{ resolve_path};
use lock::state::shell::ShellCmdState;
use crate::lock::utils::SHELL_CMD_STATE;

#[entry]
fn main(hart_id: usize, dtb_addr: usize) -> ! {
    debug_assert_eq!(hart_id, 0);
    if dtb_addr != 0 {
        crate::dtb::DTB_ADDR.store(dtb_addr, core::sync::atomic::Ordering::Release);
    }
    uart::Console::init();
    allocator::init();
    init_boot();
    hart_loop(0);
}


