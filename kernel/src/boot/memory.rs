use alloc::format;
use crate::{allocator, boot::console::{print_info, print_section, print_status}};

pub fn init_memory() {
    print_section("MEMORY SUBSYSTEM");
    print_info("Heap Base", &format!("{:#x}", allocator::heap_base()));
    print_info("Heap Size", &format!("{} KiB", allocator::heap_size() / 1024));
    print_status("Heap allocator ready", true);
}
