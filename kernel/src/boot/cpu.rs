
use core::sync::atomic::{Ordering, fence};

use alloc::format;

use crate::{boot::{console::{print_info, print_section, print_status}}, cpu::{self, HARTS_ONLINE, get_expected_harts, get_hart_id, sched, send_ipi}, fence_memory, init, services::shelld::shell_service, trap, ui::boot::print_line};

pub fn init_cpu() {
    print_line("\n");
    print_section("CPU & ARCHITECTURE");
    print_info("Architecture", "RISC-V 64-bit (RV64GC)");
    print_info("Mode", "Supervisor Mode (S-Mode via SBI)");
    print_info("Timer Source", "CLINT @ 0x02000000");
    print_status("CPU initialized", true);

    let expected_harts = get_expected_harts();
    print_info("Expected harts", &format!("{}", expected_harts));

    fence(Ordering::SeqCst);

    HARTS_ONLINE.fetch_add(1, Ordering::SeqCst);

    print_info("Primary hart", "online");

    if expected_harts > 1 {
        print_info("Starting secondary harts", &format!("{} via SBI HSM", expected_harts - 1));
        for hart in 1..expected_harts {
            // Use SBI HSM hart_start for OpenSBI-compliant hart lifecycle
            // Pass 0 as address to use PRESERVE_BOOT_PC (harts start at same entry as hart 0)
            // Pass 0 as opaque (we don't use it - DTB is in global)
            let ret = crate::sbi::hart_start(hart, 0, 0);
            if !ret.is_ok() {
                // Fallback to IPI if HSM not supported (won't happen in our VM)
                send_ipi(hart);
            }
        }
    }
    
    print_status(&format!("SMP ready for {} harts", expected_harts), true);
    print_section("PROCESS MANAGER");
    
    // Initialize CPU table for ALL expected harts (not just currently online)
    // Secondary harts will join when they wake up from IPI
    cpu::init(get_hart_id, expected_harts);
    print_status("CPU table initialized", true);
    init::INIT_COMPLETE.store(true, Ordering::Release);

    // Initialize process scheduler with ALL expected harts (not just currently online)
    // This creates run queues for each hart - critical for multi-hart operation
    sched::init(expected_harts);  // Use expected_harts, not HARTS_ONLINE!
    print_status("Process scheduler initialized", true);
   
    trap::init(0);
    fence_memory();


}
