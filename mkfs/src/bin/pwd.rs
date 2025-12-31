// pwd - Print working directory
//
// Usage:
//   pwd         Print the current working directory

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{env_get, print};

    let mut buf = [0u8; 256];
    
    // Get PWD environment variable (which contains the current working directory)
    let len = env_get(b"PWD".as_ptr(), 3, buf.as_mut_ptr(), 256);

    if len >= 0 {
        print(buf.as_ptr(), len as usize);
        print(b"\n".as_ptr(), 1);
    } else {
        // Fallback to root if PWD is not set
        print(b"/\n".as_ptr(), 2);
    }
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
