// cd - Change directory (minimal implementation)
//
// Usage:
//   cd <dir>    Change to specified directory
//   cd          Change to root directory

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::syscalls::{arg_count, arg_get, cwd_set, print};

    // Static buffer to avoid runtime memory.fill
    static mut BUF: [u8; 128] = [0u8; 128];

    #[no_mangle]
    pub extern "C" fn _start() {
        // No arguments: go to root
        let count = unsafe { arg_count() };
        if count < 1 {
            unsafe { cwd_set(b"/".as_ptr(), 1) };
            return;
        }

        // Get the path argument
        let len = unsafe { arg_get(0, BUF.as_mut_ptr(), 128) };
        
        if len <= 0 {
            unsafe { cwd_set(b"/".as_ptr(), 1) };
            return;
        }
        
        let len = len as usize;
        
        // Handle special cases
        unsafe {
            if len == 1 && BUF[0] == b'~' {
                cwd_set(b"/".as_ptr(), 1);
                return;
            }
            
            if len == 1 && BUF[0] == b'-' {
                let msg = b"cd: OLDPWD not set\n";
                print(msg.as_ptr(), msg.len());
                return;
            }

            // Try to change directory - kernel handles path resolution
            let result = cwd_set(BUF.as_ptr(), len as i32);
            
            if result != 0 {
                let err = b"\x1b[1;31mcd:\x1b[0m ";
                print(err.as_ptr(), err.len());
                print(BUF.as_ptr(), len);
                let msg = b": No such directory\n";
                print(msg.as_ptr(), msg.len());
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
