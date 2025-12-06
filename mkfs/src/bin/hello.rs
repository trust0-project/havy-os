// hello - Simple WASM test program
//
// Usage:
//   hello    Print a greeting

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::console_log;

    #[no_mangle]
    pub extern "C" fn _start() {
        console_log("Hello from WASM!\n");
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
