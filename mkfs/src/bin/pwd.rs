// pwd - Print working directory
//
// Usage:
//   pwd         Print the current working directory

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::syscalls::{env_get, print};

    // Static buffer to avoid runtime memory.fill
    static mut BUF: [u8; 256] = [0u8; 256];

    #[no_mangle]
    pub extern "C" fn _start() {
        unsafe {
            // Get PWD environment variable (which contains the current working directory)
            let len = env_get(b"PWD".as_ptr(), 3, BUF.as_mut_ptr(), 256);

            if len > 0 {
                print(BUF.as_ptr(), len as usize);
                print(b"\n".as_ptr(), 1);
            } else {
                // Fallback to root if PWD is not set
                print(b"/\n".as_ptr(), 2);
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
