// help - Show available commands and system information
//
// Usage:
//   help              Show all commands
//   help <command>    Show help for specific command

#![cfg_attr(target_arch = "riscv64", no_std)]
#![cfg_attr(target_arch = "riscv64", no_main)]

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn main() {
    use mkfs::{console_log, argc, argv, is_net_available, print};

    fn show_command_help(cmd: &[u8]) {
        match cmd {
            b"cd" => {
                console_log("\x1b[1mcd\x1b[0m - Change directory\n\n");
                console_log("Usage: cd <directory>\n\n");
                console_log("Examples:\n");
                console_log("  cd /home        Go to /home\n");
                console_log("  cd ..           Go up one level\n");
                console_log("  cd /            Go to root\n");
            }
            b"ls" => {
                console_log("\x1b[1mls\x1b[0m - List directory contents\n\n");
                console_log("Usage: ls [-l] [directory]\n\n");
                console_log("Options:\n");
                console_log("  -l  Long format with sizes\n\n");
                console_log("Examples:\n");
                console_log("  ls              List current directory\n");
                console_log("  ls -l /usr/bin  List /usr/bin in long format\n");
            }
            b"cat" => {
                console_log("\x1b[1mcat\x1b[0m - Display file contents\n\n");
                console_log("Usage: cat [-n] <file>\n\n");
                console_log("Options:\n");
                console_log("  -n  Show line numbers\n\n");
                console_log("Examples:\n");
                console_log("  cat /etc/init.d/startup\n");
                console_log("  cat -n README.md\n");
            }
            b"echo" => {
                console_log("\x1b[1mecho\x1b[0m - Print text to stdout\n\n");
                console_log("Usage: echo [-n] <text>\n\n");
                console_log("Options:\n");
                console_log("  -n  No trailing newline\n\n");
                console_log("Examples:\n");
                console_log("  echo Hello World\n");
                console_log("  echo -n 'no newline'\n");
            }
            b"uptime" => {
                console_log("\x1b[1muptime\x1b[0m - Show system uptime\n\n");
                console_log("Usage: uptime\n\n");
                console_log("Shows how long the system has been running.\n");
            }
            b"dmesg" => {
                console_log("\x1b[1mdmesg\x1b[0m - Display kernel log\n\n");
                console_log("Usage: dmesg [-n <count>]\n\n");
                console_log("Options:\n");
                console_log("  -n <count>  Show last N messages\n");
            }
            b"cowsay" => {
                console_log("\x1b[1mcowsay\x1b[0m - ASCII art cow\n\n");
                console_log("Usage: cowsay [message]\n\n");
                console_log("Examples:\n");
                console_log("  cowsay             Say 'Moo!'\n");
                console_log("  cowsay Hello!      Say 'Hello!'\n");
            }
            _ => {
                console_log("\x1b[31mNo help available for: \x1b[0m");
                print(cmd.as_ptr(), cmd.len());
                console_log("\n");
            }
        }
    }

    let arg_count = argc();

    // Check if asking for specific command help
    if arg_count >= 1 {
        let mut cmd_buf = [0u8; 32];
        if let Some(len) = argv(0, &mut cmd_buf) {
            show_command_help(&cmd_buf[..len]);
            return;
        }
    }

    // Show full help
    console_log("\n");
    console_log("\x1b[1;36m+------------------------------------------------------------+\x1b[0m\n");
    console_log("\x1b[1;36m|\x1b[0m          \x1b[1;37mBAVY OS - Command Reference\x1b[0m                    \x1b[1;36m|\x1b[0m\n");
    console_log("\x1b[1;36m+------------------------------------------------------------+\x1b[0m\n\n");

    // Built-in Shell Commands
    console_log("\x1b[1;33m+-- Built-in Shell Commands ---------------------------------+\x1b[0m\n");
    console_log("\x1b[33m|\x1b[0m  \x1b[1mcd\x1b[0m <dir>       Change directory                       \x1b[33m|\x1b[0m\n");
    console_log("\x1b[33m|\x1b[0m  \x1b[1mpwd\x1b[0m            Print working directory                 \x1b[33m|\x1b[0m\n");
    console_log("\x1b[33m|\x1b[0m  \x1b[1mclear\x1b[0m          Clear the screen                        \x1b[33m|\x1b[0m\n");
    console_log("\x1b[33m|\x1b[0m  \x1b[1mshutdown\x1b[0m       Power off the system                    \x1b[33m|\x1b[0m\n");
    console_log("\x1b[33m+------------------------------------------------------------+\x1b[0m\n\n");

    // Native Programs
    console_log("\x1b[1;32m+-- Native RISC-V Programs (in /usr/bin/) -------------------+\x1b[0m\n");
    console_log("\x1b[32m|\x1b[0m  \x1b[1mcat\x1b[0m [-n] file  Display file contents                  \x1b[32m|\x1b[0m\n");
    console_log("\x1b[32m|\x1b[0m  \x1b[1mecho\x1b[0m [-n] txt  Print text to stdout                   \x1b[32m|\x1b[0m\n");
    console_log("\x1b[32m|\x1b[0m  \x1b[1muptime\x1b[0m         Show system uptime                     \x1b[32m|\x1b[0m\n");
    console_log("\x1b[32m|\x1b[0m  \x1b[1mhelp\x1b[0m [cmd]     Show help (this screen)                \x1b[32m|\x1b[0m\n");
    console_log("\x1b[32m|\x1b[0m  \x1b[1mdmesg\x1b[0m [-n N]   Display kernel log messages             \x1b[32m|\x1b[0m\n");
    console_log("\x1b[32m|\x1b[0m  \x1b[1mcowsay\x1b[0m [msg]   ASCII art cow says something           \x1b[32m|\x1b[0m\n");
    console_log("\x1b[32m+------------------------------------------------------------+\x1b[0m\n\n");

    // System Status
    console_log("\x1b[1;35m+-- System Status -------------------------------------------+\x1b[0m\n");

    // Network status
    if is_net_available() {
        console_log("\x1b[35m|\x1b[0m  Network:       \x1b[32m* Online\x1b[0m                              \x1b[35m|\x1b[0m\n");
    } else {
        console_log("\x1b[35m|\x1b[0m  Network:       \x1b[31mo Offline\x1b[0m                             \x1b[35m|\x1b[0m\n");
    }

    console_log("\x1b[35m|\x1b[0m  Kernel:        BAVY RISC-V                           \x1b[35m|\x1b[0m\n");
    console_log("\x1b[35m|\x1b[0m  Shell:         Built-in                              \x1b[35m|\x1b[0m\n");
    console_log("\x1b[35m+------------------------------------------------------------+\x1b[0m\n\n");

    console_log("\x1b[90mTip: Run 'help <command>' for detailed help on a command.\x1b[0m\n\n");
}

#[cfg(not(target_arch = "riscv64"))]
fn main() {}
