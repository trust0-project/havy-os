# Havy OS

Havy OS is a hobby operating system written in Rust, designed for the RISC-V 64-bit (RV64GC) architecture. It is built as a monolithic kernel with support for Symmetric Multiprocessing (SMP), a simple file system, a networking stack, and a WebAssembly (WASM) runtime for user-space applications.

## Features

- **RISC-V 64-bit Architecture:** Targets the `riscv64gc-unknown-none-elf` platform.
- **Multi-hart (SMP) Support:** Includes a multi-hart boot process and a scheduler that can distribute tasks across multiple cores.
- **Preemptive Scheduler:** A priority-based, preemptive scheduler with per-hart run queues and work-stealing capabilities.
- **Simple File System (SFS):** A custom block-based file system with write-caching for performance.
- **Networking Stack:** Utilizes `smoltcp` to provide a TCP/IP stack with support for:
  - TCP, UDP, and ICMP protocols.
  - DNS for hostname resolution.
  - HTTP/HTTPS client for web requests (with TLS 1.2 and 1.3 support).
- **WASM Runtime:** Executes user-space applications compiled to WebAssembly using the `wasmi` interpreter. This provides a sandboxed environment for user programs.
- **User-space Utilities:** A collection of standard command-line tools (e.g., `ls`, `cat`, `wget`, `ping`, `ps`, `htop`) compiled to WASM.
- **Inter-Process Communication (IPC):** Provides channels and pipes for communication between tasks.

## Architecture

Havy OS consists of two main parts: the kernel and the user-space applications.

### Kernel

The kernel (`kernel/`) is monolithic and handles all core system functionality, including:
- Memory management (heap allocator).
- Process and task management (scheduler, task control blocks).
- Device drivers (VirtIO for block storage and networking, UART for serial console).
- Filesystem, networking, and IPC services.
- A system call interface for user-space applications.

### User Space (WASM)

User-space applications are written in Rust and compiled to WebAssembly. They are located in `mkfs/src/bin/`. These applications are then included in the filesystem image by the `mkfs` utility. When the shell runs a command, the kernel loads the corresponding WASM binary, and executes it using the `wasmi` interpreter.

This design provides a clear separation between the kernel and user applications, with WASM offering a secure and isolated environment for user code.

## Getting Started

Follow these instructions to build and run Havy OS on your local machine.

### Prerequisites

- **Rust Toolchain:** Make sure you have a recent Rust toolchain installed. You'll need the `riscv64gc-unknown-none-elf` and `wasm32-unknown-unknown` targets.
  ```sh
  rustup target add riscv64gc-unknown-none-elf
  rustup target add wasm32-unknown-unknown
  ```
- **QEMU:** You'll need QEMU for RISC-V to run the OS. Make sure you have `qemu-system-riscv64`.
- **`wasm-opt`:** This tool from the `binaryen` package is used to optimize the WASM binaries. It is recommended for smaller and faster user-space programs.

### Building

A build script is provided to automate the process. Run it from the root of the project:

```sh
sh ./build.sh
```

This script will:
1.  Compile the kernel for RISC-V.
2.  Compile the user-space applications and `mkfs` utility to WASM.
3.  Optimize the WASM binaries using `wasm-opt` if it's installed.
4.  Run the `mkfs` utility to create a 2MB filesystem image (`fs.img`) containing the user-space applications.

### Running

After a successful build, you can run Havy OS in QEMU with the following command:

with our npm package virtual-machine
```sh
npx virtual-machine --kernel target/riscv64gc-unknown-none-elf/release/kernel --disk target/riscv64gc-unknown-none-elf/release/fs.img --harts 2
```

```sh
qemu-system-riscv64 -machine virt -m 1G -bios none \
  -kernel target/riscv64gc-unknown-none-elf/release/kernel \
  -drive file=target/riscv64gc-unknown-none-elf/release/fs.img,format=raw,id=hd0,if=none \
  -device virtio-blk-device,drive=hd0 \
  -chardev stdio,id=char0,mux=on,signal=off \
  -serial chardev:char0 -display none
```

This will start the OS, and you should see the boot process in your terminal, ending with a shell prompt.

## Shell and Commands

Havy OS includes a simple shell that allows you to run commands and interact with the system. Here are some of the available commands:

| Command      | Description                                     |
|--------------|-------------------------------------------------|
| `ls`         | List files and directories.                     |
| `cat`        | Display the contents of a file.                 |
| `echo`       | Print text to the console.                      |
| `pwd`        | Print the current working directory.            |
| `cd`         | Change the current directory.                   |
| `mkdir`      | Create a new directory.                         |
| `rm`         | Remove a file.                                  |
| `write`      | Write text to a file.                           |
| `ps`         | List running processes.                         |
| `kill`       | Terminate a process by its PID.                 |
| `htop`       | Display an interactive process viewer.          |
| `top`        | Display system status and process list.         |
| `dmesg`      | Show messages from the kernel ring buffer.      |
| `sysinfo`    | Display system information.                     |
| `memstats`   | Show memory usage statistics.                   |
| `uptime`     | Show how long the system has been running.      |
| `ping`       | Send ICMP ECHO_REQUEST packets to network hosts. |
| `nslookup`   | Query DNS servers.                              |
| `wget`       | Download a file from the web.                   |
| `ip`         | Show network interface configuration.           |
| `netstat`    | Show network statistics.                        |
| `cowsay`     | An ASCII cow will say your message.             |
| `cputest`    | A CPU benchmark that counts prime numbers.      |
| `memtest`    | A simple memory test.                           |
| `wasmrun`    | Run a WASM binary on a worker hart.             |
| `service`    | Manage system services.                         |
| `shutdown`   | Power off the system.                           |
| `help`       | Show a list of available commands.              |
