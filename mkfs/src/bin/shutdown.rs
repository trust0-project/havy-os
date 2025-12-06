// shutdown - Power off the system
//
// Usage:
//   shutdown       Immediately power off the system

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::poweroff;

    #[no_mangle]
    pub extern "C" fn _start() {
        poweroff();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}

