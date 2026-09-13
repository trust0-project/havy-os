//! Sv39 paging.
//!
//! The guest writes `satp` (the VM already walks Sv39). MMIO is a 1 GiB
//! kernel leaf; DRAM is 2 MiB kernel leaves so ELF/stack ranges can be
//! overlaid as 4 KiB user pages. A wild store in `ls` then faults instead
//! of scribbling kernel `.data`.

use core::arch::asm;
use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::alloc::{alloc_zeroed, Layout};
use alloc::format;

use crate::services::klogd::{klog_info, klog_warning};

const PAGE_SIZE: usize = 4096;
const SATP_MODE_SV39: u64 = 8;
const PTE_V: u64 = 1 << 0;
const PTE_R: u64 = 1 << 1;
const PTE_W: u64 = 1 << 2;
const PTE_X: u64 = 1 << 3;
const PTE_U: u64 = 1 << 4;
const PTE_G: u64 = 1 << 5;
const PTE_A: u64 = 1 << 6;
const PTE_D: u64 = 1 << 7;

const KERNEL_LEAF: u64 = PTE_V | PTE_R | PTE_W | PTE_X | PTE_G | PTE_A | PTE_D;
const USER_LEAF: u64 = PTE_V | PTE_R | PTE_W | PTE_X | PTE_U | PTE_A | PTE_D;

#[repr(C, align(4096))]
struct PageTable {
    entries: [u64; 512],
}

static ROOT_SATP: AtomicUsize = AtomicUsize::new(0);

static mut ROOT: Option<&'static mut PageTable> = None;

fn alloc_table() -> &'static mut PageTable {
    let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
    let ptr = unsafe { alloc_zeroed(layout) };
    if ptr.is_null() {
        panic!("paging: out of memory for page table");
    }
    unsafe { &mut *(ptr as *mut PageTable) }
}

fn pte_ppn(pa: u64) -> u64 {
    (pa >> 12) << 10
}

fn pte_is_leaf(pte: u64) -> bool {
    pte & (PTE_R | PTE_W | PTE_X) != 0
}

fn table_from_pte(pte: u64) -> &'static mut PageTable {
    let pa = (pte >> 10) << 12;
    unsafe { &mut *(pa as *mut PageTable) }
}

/// Identity-map a 1 GiB gigapage (L2 leaf). Used for the MMIO window.
fn map_1g(root: &mut PageTable, va: u64, flags: u64) {
    let vpn2 = ((va >> 30) & 0x1FF) as usize;
    root.entries[vpn2] = pte_ppn(va) | flags;
}

/// Identity-map `size` bytes at `va` as 2 MiB kernel leaves.
fn map_2m_range(root: &mut PageTable, va: u64, size: u64, flags: u64) {
    let mut addr = va;
    let end = va + size;
    while addr < end {
        let vpn2 = ((addr >> 30) & 0x1FF) as usize;
        let vpn1 = ((addr >> 21) & 0x1FF) as usize;
        if root.entries[vpn2] & PTE_V == 0 {
            let l1 = alloc_table();
            root.entries[vpn2] = pte_ppn(l1 as *mut PageTable as u64) | PTE_V;
        }
        let l1 = table_from_pte(root.entries[vpn2]);
        l1.entries[vpn1] = pte_ppn(addr) | flags;
        addr += 0x20_0000;
    }
}

fn split_2m_to_4k(l1: &mut PageTable, vpn1: usize) {
    let pte = l1.entries[vpn1];
    if pte & PTE_V == 0 || !pte_is_leaf(pte) {
        if pte & PTE_V == 0 {
            let l0 = alloc_table();
            l1.entries[vpn1] = pte_ppn(l0 as *mut PageTable as u64) | PTE_V;
        }
        return;
    }
    let base = (pte >> 10) << 12;
    let flags = pte & 0x3FF;
    let l0 = alloc_table();
    for i in 0..512 {
        l0.entries[i] = pte_ppn(base + (i as u64) * PAGE_SIZE as u64) | flags;
    }
    l1.entries[vpn1] = pte_ppn(l0 as *mut PageTable as u64) | PTE_V;
}

fn map_4k(root: &mut PageTable, va: u64, flags: u64) {
    let vpn2 = ((va >> 30) & 0x1FF) as usize;
    let vpn1 = ((va >> 21) & 0x1FF) as usize;
    let vpn0 = ((va >> 12) & 0x1FF) as usize;

    if root.entries[vpn2] & PTE_V == 0 {
        let l1 = alloc_table();
        root.entries[vpn2] = pte_ppn(l1 as *mut PageTable as u64) | PTE_V;
    } else if pte_is_leaf(root.entries[vpn2]) {
        // 1 GiB leaf — do not split MMIO.
        return;
    }
    let l1 = table_from_pte(root.entries[vpn2]);
    split_2m_to_4k(l1, vpn1);
    let l0 = table_from_pte(l1.entries[vpn1]);
    l0.entries[vpn0] = pte_ppn(va) | flags;
}

/// Overlay 4 KiB user pages over identity-mapped DRAM (ELF + stack).
pub fn map_user(pa: usize, size: usize) {
    let root = match unsafe { ROOT.as_mut() } {
        Some(r) => r,
        None => return,
    };
    let start = pa & !0xFFF;
    let end = (pa + size + PAGE_SIZE - 1) & !0xFFF;
    let mut addr = start;
    while addr < end {
        map_4k(root, addr as u64, USER_LEAF);
        addr += PAGE_SIZE;
    }
    sfence_vma();
}

/// Write `satp` and `sfence.vma`.
pub fn init() {
    let root = alloc_table();
    map_1g(root, 0x0000_0000, KERNEL_LEAF);
    map_2m_range(root, 0x4000_0000, 512 * 1024 * 1024, KERNEL_LEAF);
    map_2m_range(root, 0x8000_0000, 512 * 1024 * 1024, KERNEL_LEAF);

    let ppn = (root as *mut PageTable as u64) >> 12;
    let satp = (SATP_MODE_SV39 << 60) | ppn;
    unsafe {
        ROOT = Some(root);
        asm!(
            "csrw satp, {satp}",
            "sfence.vma",
            satp = in(reg) satp,
            options(nostack)
        );
    }
    ROOT_SATP.store(satp as usize, Ordering::Release);
    // SUM: S-mode may touch user pages (syscalls copy from ELF memory).
    unsafe {
        asm!(
            "li t0, {sum}",
            "csrs sstatus, t0",
            out("x5") _,
            sum = const 1usize << 18,
            options(nomem, nostack)
        );
    }
    klog_info("paging", "Sv39 on: MMIO 1GiB + DRAM 2MiB identity");
}

/// Restore kernel (non-U) leaves over a range previously mapped for a process.
pub fn unmap_user(pa: usize, size: usize) {
    let root = match unsafe { ROOT.as_mut() } {
        Some(r) => r,
        None => return,
    };
    let start = pa & !0xFFF;
    let end = (pa + size + PAGE_SIZE - 1) & !0xFFF;
    let mut addr = start;
    while addr < end {
        map_4k(root, addr as u64, KERNEL_LEAF);
        addr += PAGE_SIZE;
    }
    sfence_vma();
}

pub fn sfence_vma() {
    unsafe {
        asm!("sfence.vma", options(nostack));
    }
}

pub fn satp() -> u64 {
    ROOT_SATP.load(Ordering::Acquire) as u64
}

pub fn enabled() -> bool {
    ROOT_SATP.load(Ordering::Acquire) != 0
}

/// Handle a page fault: kill the current ELF if one is running, otherwise
/// skip the faulting instruction. Never panic.
pub fn handle_fault(_cause: usize, sepc: usize, stval: usize) {
    klog_warning(
        "paging",
        &format!("page fault sepc={:#x} stval={:#x}", sepc, stval),
    );
    if crate::elf_loader::is_running() {
        crate::elf_loader::signal_exit(-11);
        crate::elf_loader::restore_kernel_context();
    }
    unsafe {
        asm!(
            "csrr t0, sepc",
            "addi t0, t0, 4",
            "csrw sepc, t0",
            out("x5") _,
            options(nomem, nostack)
        );
    }
}
