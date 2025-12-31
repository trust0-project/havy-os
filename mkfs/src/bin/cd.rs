// cd - Change directory (minimal implementation)
//
// Usage:
//   cd <dir>    Change to specified directory
//   cd          Change to root directory

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{arg_count, arg_get, cwd_set, print};

    // Static buffer to avoid runtime memory.fill
    static mut BUF: [u8; 128] = [0u8; 128];

    // No arguments: go to root
    let count = arg_count();
    if count < 1 {
        cwd_set(b"/".as_ptr(), 1);
        return;
    }

    // Get the path argument
    let len = unsafe { arg_get(0, (*core::ptr::addr_of_mut!(BUF)).as_mut_ptr(), 128) };
    
    if len <= 0 {
        cwd_set(b"/".as_ptr(), 1);
        return;
    }
    
    let len = len as usize;
    
    // Handle special cases
    unsafe {
        let buf = &*core::ptr::addr_of!(BUF);
        if len == 1 && buf[0] == b'~' {
            cwd_set(b"/".as_ptr(), 1);
            return;
        }
        
        if len == 1 && buf[0] == b'-' {
            let msg = b"cd: OLDPWD not set\n";
            print(msg.as_ptr(), msg.len());
            return;
        }

        // Try to change directory - kernel handles path resolution
        let result = cwd_set(buf.as_ptr(), len as i32);
        
        if result != 0 {
            let err = b"\x1b[1;31mcd:\x1b[0m ";
            print(err.as_ptr(), err.len());
            print(buf.as_ptr(), len);
            let msg = b": No such directory\n";
            print(msg.as_ptr(), msg.len());
        }
    }
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
