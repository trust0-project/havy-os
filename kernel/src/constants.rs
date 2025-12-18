pub(crate) const CLINT_MSIP_BASE: usize = 0x0200_0000;
pub(crate) const TEST_FINISHER: usize = 0x0010_0000;
pub(crate) const SYSINFO_BASE: usize = 0x0011_0000;
pub(crate) const SYSINFO_HEAP_USED: usize = SYSINFO_BASE + 0x00;
pub(crate) const SYSINFO_HEAP_TOTAL: usize = SYSINFO_BASE + 0x08;
pub(crate) const SYSINFO_DISK_USED: usize = SYSINFO_BASE + 0x10;
pub(crate) const SYSINFO_DISK_TOTAL: usize = SYSINFO_BASE + 0x18;
pub(crate) const SYSINFO_CPU_COUNT: usize = SYSINFO_BASE + 0x20;
pub(crate) const SYSINFO_UPTIME: usize = SYSINFO_BASE + 0x28;

// Scheduler diagnostics MMIO region
pub(crate) const SCHED_DIAG_BASE: usize = 0x0011_1000;
pub(crate) const SCHED_DIAG_HART_ID: usize = SCHED_DIAG_BASE + 0x00;        // u32
pub(crate) const SCHED_DIAG_PICK_COUNT: usize = SCHED_DIAG_BASE + 0x04;     // u32
pub(crate) const SCHED_DIAG_PICK_RESULT: usize = SCHED_DIAG_BASE + 0x08;    // u32 (0=None, 1=Some)
pub(crate) const SCHED_DIAG_PROCESS_PID: usize = SCHED_DIAG_BASE + 0x0C;    // u32
pub(crate) const SCHED_DIAG_PROCESS_NAME: usize = SCHED_DIAG_BASE + 0x10;   // 32 bytes
pub(crate) const SCHED_DIAG_CAN_SCHEDULE: usize = SCHED_DIAG_BASE + 0x30;   // u32
pub(crate) const SCHED_DIAG_REQUEUE_OK: usize = SCHED_DIAG_BASE + 0x34;     // u32
pub(crate) const SCHED_DIAG_QUEUE_DEPTH: usize = SCHED_DIAG_BASE + 0x38;    // u32

pub(crate) const CLINT_MTIME: usize = 0x0200_BFF8;
