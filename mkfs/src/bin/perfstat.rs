// perfstat - Show kernel performance counters
//
// Usage:
//   perfstat          Display all counters
//   perfstat reset    Display all counters, then reset them to zero
//
// Counter order must match kernel/src/perfstat.rs (module `id`).
//
// NOTE: the kernel ELF loader rebases segments without applying data
// relocations, so this binary must avoid patterns that put &str fat
// pointers into rodata (static arrays, match-expressions returning &str).
// Every name is printed via a direct console_log(literal) call instead.

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
const NUM_COUNTERS: usize = 19;

/// Print the padded name for counter `i` using only direct literal calls.
#[cfg(target_arch = "riscv64")]
fn print_counter_name(i: usize) {
    use mkfs::console_log;
    // Names pre-padded to 20 columns.
    match i {
        0 => console_log("sched.picks         "),
        1 => console_log("sched.steals        "),
        2 => console_log("sched.steal_misses  "),
        3 => console_log("sched.requeues      "),
        4 => console_log("sched.migrations    "),
        5 => console_log("ipi.sent            "),
        6 => console_log("io.requests         "),
        7 => console_log("io.dispatched       "),
        8 => console_log("heap.allocs         "),
        9 => console_log("heap.frees          "),
        10 => console_log("heap.alloc_bytes    "),
        11 => console_log("log.lines           "),
        12 => console_log("gpuid.ticks         "),
        13 => console_log("gpuid.active_ticks  "),
        14 => console_log("gfx.frames_flushed  "),
        15 => console_log("gfx.dirty_pixels    "),
        16 => console_log("irq.timer           "),
        17 => console_log("sys.syscalls        "),
        18 => console_log("sched.idle_wfi      "),
        _ => console_log("unknown             "),
    }
}

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{arg_count, arg_get, console_log, perfstat, print_int};

    // Check for "reset" argument
    let mut reset = false;
    if arg_count() > 0 {
        let mut buf = [0u8; 16];
        let len = arg_get(0, buf.as_mut_ptr(), buf.len() as i32);
        if len > 0 && &buf[..len as usize] == b"reset" {
            reset = true;
        }
    }

    let mut counters = [0u64; NUM_COUNTERS];
    let n = perfstat(&mut counters, reset);
    if n <= 0 {
        console_log("perfstat: syscall failed (kernel too old?)\n");
        return;
    }
    let n = (n as usize).min(NUM_COUNTERS);

    console_log("\n\x1b[1;97mKernel performance counters\x1b[0m\n");
    console_log("\x1b[0;90m---------------------------------------\x1b[0m\n");

    let mut i = 0;
    while i < n {
        console_log("  \x1b[1;36m");
        print_counter_name(i);
        console_log("\x1b[0m");
        print_int(counters[i] as i64);
        console_log("\n");
        i += 1;
    }

    if reset {
        console_log("\n\x1b[0;90mcounters reset\x1b[0m\n");
    }
    console_log("\n");
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
