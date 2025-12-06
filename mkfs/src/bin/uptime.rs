// uptime - Show system uptime
//
// Usage:
//   uptime        Show how long the system has been running

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{console_log, get_time, print_int};

    #[no_mangle]
    pub extern "C" fn _start() {
        let ms = get_time();
        let total_sec = ms / 1000;
        let hours = total_sec / 3600;
        let minutes = (total_sec % 3600) / 60;
        let seconds = total_sec % 60;

        console_log("Uptime: ");

        if hours > 0 {
            print_int(hours);
            console_log("h ");
            print_int(minutes);
            console_log("m ");
            print_int(seconds);
            console_log("s\n");
        } else if minutes > 0 {
            print_int(minutes);
            console_log("m ");
            print_int(seconds);
            console_log("s\n");
        } else {
            print_int(seconds);
            console_log("s\n");
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
