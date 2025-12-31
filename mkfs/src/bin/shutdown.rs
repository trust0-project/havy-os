// shutdown - Power off the system
//
// Usage:
//   shutdown       Immediately power off the system

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    mkfs::poweroff();
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
