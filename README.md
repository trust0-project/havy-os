# Havy OS

Havy OS is a hobby operating system written in Rust, designed for the RISC-V 64-bit (RV64GC) architecture. It is built as a monolithic kernel with support for Symmetric Multiprocessing (SMP), a simple file system, a networking stack, and a WebAssembly (WASM) runtime for user-space applications.

## Features

- **RISC-V 64-bit Architecture:** Targets the `riscv64gc-unknown-none-elf` platform. Two boards: QEMU `virt` (default) and Allwinner D1.
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
- Device drivers (UART; storage and net depend on `virt` vs `d1`).
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

Kernel Cargo features are mutually exclusive:

- **`virt` (default)** — QEMU `virt` / `riscv-vm --machine virt` (DRAM `0x8000_0000`, 10 MHz timebase).
- **`d1`** — Allwinner D1 / Lichee RV. Build with `--no-default-features --features d1`.

```sh
# virt kernel (default features)
sh ./build.sh
sh ./build.sh sdcard    # also writes sdcard.img for the VM

# D1 kernel
cd kernel && cargo build --release --target riscv64gc-unknown-none-elf --no-default-features --features d1
```

`./build.sh` compiles the kernel, user-space programs, and a filesystem image (`fs.img`). Pass `sdcard` to pack `kernel.bin` + `fs.img` into `sdcard.img`.

### Running

Match the kernel feature to the board. The VM boots an SD card image (`--sdcard`); there is no `--kernel` flag.

**QEMU virt** (kernel built with default `virt` features):

```sh
qemu-system-riscv64 -machine virt -m 1G -bios none \
  -kernel target/riscv64gc-unknown-none-elf/release/kernel \
  -drive file=target/riscv64gc-unknown-none-elf/release/fs.img,format=raw,id=hd0,if=none \
  -device virtio-blk-device,drive=hd0 \
  -netdev user,id=net0 -device virtio-net-device,netdev=net0 \
  -device virtio-gpu-device \
  -device virtio-keyboard-device -device virtio-mouse-device \
  -chardev stdio,id=char0,mux=on,signal=off \
  -serial chardev:char0 -display none
```

**riscv-vm** (`npx virtual-machine`, after `./build.sh sdcard`):

```sh
# virt kernel ↔ --machine virt (default, 10 MHz)
npx virtual-machine --sdcard target/riscv64gc-unknown-none-elf/release/sdcard.img --harts 2 --machine virt

# D1 kernel ↔ --machine d1 (24 MHz)
npx virtual-machine --sdcard target/riscv64gc-unknown-none-elf/release/sdcard.img --machine d1
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
