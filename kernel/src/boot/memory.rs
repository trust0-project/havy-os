
use alloc::format;
use crate::{allocator, boot::console::{print_info, print_section, print_status}};



pub fn init_memory() {
    print_section("MEMORY SUBSYSTEM");
    let total_heap = allocator::heap_size();
    print_info("Heap Base", "0x80800000");
    print_info("Heap Size", &format!("{} KiB", total_heap / 1024));
    print_status("Heap allocator ready", true);
}
