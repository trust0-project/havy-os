use linked_list_allocator::LockedHeap;

unsafe extern "C" {
    // Linker symbols for section boundaries
    static _stext: u8;      // Start of .text section (kernel code)
    static mut _sheap: u8;  // Start of heap (end of static sections)
    static mut _eheap: u8;  // End of heap
}

/// Per-hart stack size (must match link.x: _hart_stack_size = 128K)
const HART_STACK_SIZE: usize = 128 * 1024;

/// RAM base address (must match link.x: ORIGIN = 0x80000000)
const RAM_BASE: usize = 0x8000_0000;

/// Total RAM size (must match link.x: LENGTH = 512M)
const RAM_SIZE: usize = 512 * 1024 * 1024;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// Initialize the heap allocator.
/// Must be called before any heap allocations occur.
pub fn init() {
    unsafe {
        let heap_start = &raw mut _sheap as *mut u8;
        let heap_end = &raw const _eheap as usize;
        let heap_size = heap_end - (heap_start as usize);
        ALLOCATOR.lock().init(heap_start, heap_size);
    }
}

/// Returns (used, free) bytes in the heap, if the allocator supports introspection.
pub fn heap_stats() -> (usize, usize) {
    let allocator = ALLOCATOR.lock();
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
    /// GPU framebuffer memory (front + back buffers, 0 if GPU disabled)
    pub framebuffer_size: usize,
    /// Total memory consumed (static + heap_used + stacks + framebuffers)
    pub total_used: usize,
    /// Total RAM available
    pub total_available: usize,
}

/// Framebuffer size: 1024 × 768 × 4 bytes × 2 buffers (front + back)
const FRAMEBUFFER_TOTAL: usize = 1024 * 768 * 4 * 2;

/// Get comprehensive memory statistics.
/// 
/// # Arguments
/// * `active_harts` - Number of harts currently online (for stack calculation)
/// * `gpu_enabled` - Whether GPU display is active (for framebuffer calculation)
pub fn memory_stats(active_harts: usize, gpu_enabled: bool) -> MemoryStats {
    // Calculate static section size (from kernel start to heap start)
    let static_size = unsafe {
        let text_start = &raw const _stext as usize;
        let heap_start = &raw const _sheap as usize;
        heap_start.saturating_sub(text_start)
    };
    
    // Get heap stats
    let (heap_used, heap_free) = heap_stats();
    let heap_total = heap_size();
    
    // Calculate stack memory for all active harts
    let stack_size = active_harts * HART_STACK_SIZE;
    
    // Framebuffer memory (only when GPU is enabled)
    let framebuffer_size = if gpu_enabled { FRAMEBUFFER_TOTAL } else { 0 };
    
    // Total memory used
    let total_used = static_size + heap_used + stack_size + framebuffer_size;
    
    MemoryStats {
        static_size,
        heap_used,
        heap_free,
        heap_total,
        stack_size,
        framebuffer_size,
        total_used,
        total_available: RAM_SIZE,
    }
}

