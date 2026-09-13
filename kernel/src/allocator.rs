use core::alloc::{GlobalAlloc, Layout};
use linked_list_allocator::LockedHeap;

use crate::perfstat;
use crate::platform::current;

unsafe extern "C" {
    static _stext: u8;
    static mut _sheap: u8;
    static mut _eheap: u8;
    static _sfb: u8;
    static _efb: u8;
}

/// Per-hart stack size (must match link.x / d1.ld: `_hart_stack_size = 128K`).
const HART_STACK_SIZE: usize = 128 * 1024;

/// Global allocator wrapper that maintains perfstat counters around the
/// underlying heap implementation.
struct CountingAllocator {
    inner: LockedHeap,
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        perfstat::inc(perfstat::id::HEAP_ALLOCS);
        perfstat::add(perfstat::id::HEAP_ALLOC_BYTES, layout.size() as u64);
        unsafe { self.inner.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        perfstat::inc(perfstat::id::HEAP_FREES);
        unsafe { self.inner.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator {
    inner: LockedHeap::empty(),
};

/// Initialize the heap allocator from the linker-claimed leftover DRAM.
/// Must be called before any heap allocations occur.
pub fn init() {
    unsafe {
        let heap_start = &raw mut _sheap as *mut u8;
        let heap_end = &raw const _eheap as usize;
        let heap_size = heap_end - (heap_start as usize);
        ALLOCATOR.inner.lock().init(heap_start, heap_size);
    }
}

/// Heap start physical address (`_sheap`).
pub fn heap_base() -> usize {
    &raw const _sheap as usize
}

/// Returns (used, free) bytes in the heap, if the allocator supports introspection.
pub fn heap_stats() -> (usize, usize) {
    let allocator = ALLOCATOR.inner.lock();
    let used = allocator.used();
    let free = allocator.free();
    (used, free)
}

/// Returns the total heap size.
pub fn heap_size() -> usize {
    let heap_start = &raw const _sheap as usize;
    let heap_end = &raw const _eheap as usize;
    heap_end - heap_start
}

/// Scanout bytes reserved by the linker (one buffer, stride-padded).
pub fn framebuffer_reserved() -> usize {
    let start = &raw const _sfb as usize;
    let end = &raw const _efb as usize;
    end.saturating_sub(start)
}

/// Comprehensive memory statistics
pub struct MemoryStats {
    /// Static memory: kernel code + rodata + data + bss (from _stext to _sheap)
    pub static_size: usize,
    /// Heap memory currently allocated
    pub heap_used: usize,
    /// Heap memory available (free)
    pub heap_free: usize,
    /// Total heap size
    pub heap_total: usize,
    /// Per-hart stack memory (HART_STACK_SIZE × active harts)
    pub stack_size: usize,
    /// GPU framebuffer memory (reserved scanout)
    pub framebuffer_size: usize,
    /// Total memory consumed (static + heap_used + stacks + framebuffers)
    pub total_used: usize,
    /// Total RAM available
    pub total_available: usize,
}

/// Get comprehensive memory statistics.
pub fn memory_stats(active_harts: usize, gpu_enabled: bool) -> MemoryStats {
    let static_size = unsafe {
        let text_start = &raw const _stext as usize;
        let heap_start = &raw const _sheap as usize;
        heap_start.saturating_sub(text_start)
    };

    let (heap_used, heap_free) = heap_stats();
    let heap_total = heap_size();
    let stack_size = active_harts * HART_STACK_SIZE;
    let framebuffer_size = if gpu_enabled { framebuffer_reserved() } else { 0 };
    let total_used = static_size + heap_used + stack_size + framebuffer_size;

    MemoryStats {
        static_size,
        heap_used,
        heap_free,
        heap_total,
        stack_size,
        framebuffer_size,
        total_used,
        total_available: current::DRAM_SIZE,
    }
}
