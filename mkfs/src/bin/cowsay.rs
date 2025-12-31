// cowsay - Make a cow say something!
//
// Usage:
//   cowsay              Say "Moo!"
//   cowsay <message>    Say a custom message

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{console_log, argc, argv, print};

    fn print_char(c: u8, count: usize) {
        for _ in 0..count {
            print(&c as *const u8, 1);
        }
    }

    let arg_count = argc();

    // Collect message from arguments or use default
    let mut msg_buf = [0u8; 256];
    let mut msg_len: usize = 0;

    if arg_count > 0 {
        // Concatenate all arguments with spaces
        for i in 0..arg_count {
            if i > 0 && msg_len < 255 {
                msg_buf[msg_len] = b' ';
                msg_len += 1;
            }
            if let Some(len) = argv(i, &mut msg_buf[msg_len..]) {
                msg_len += len;
            }
        }
    }

    // Default message
    if msg_len == 0 {
        let default = b"Moo!";
        msg_buf[..default.len()].copy_from_slice(default);
        msg_len = default.len();
    }

    // Draw the speech bubble
    let bubble_width = msg_len + 2;

    // Top border
    console_log(" ");
    print_char(b'_', bubble_width);
    console_log("\n");

    // Message line
    console_log("< ");
    print(msg_buf.as_ptr(), msg_len);
    console_log(" >\n");

    // Bottom border
    console_log(" ");
    print_char(b'-', bubble_width);
    console_log("\n");

    // The cow
    console_log("        \\   ^__^\n");
    console_log("         \\  (oo)\\_______\n");
    console_log("            (__)\\       )\\/\\\n");
    console_log("                ||----w |\n");
    console_log("                ||     ||\n");
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
