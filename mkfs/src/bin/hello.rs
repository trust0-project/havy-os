// hello - Simple test program
//
// Usage:
//   hello    Print a greeting

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    mkfs::console_log("Hello from native RISC-V!\n");
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
