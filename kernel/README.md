# Custom RISC-V Kernel

A lightweight, bare-metal operating system kernel written in Rust for the RISC-V architecture. It serves as a demonstration of the VM's capabilities, featuring a command-line interface, memory management, and a TCP/IP networking stack.

## Features

- **Pure Rust**: Built with `#![no_std]` for bare-metal execution.
- **Networking**: TCP/IP via `smoltcp`. Virt: virtio-net + DHCPv4. D1: EMAC + DHCPv4.
- **Memory Management**: Dynamic heap allocation using a linked-list allocator.
- **Interactive Shell**: Built-in UART console with command history and editing.
- **Device Drivers**: UART, timer, storage, and networking (board-dependent).
- **Boards**: Cargo feature `virt` (default, QEMU virt) or `d1` (`--no-default-features --features d1`).

## Commands

The kernel boots into an interactive shell supporting the following commands:

| Command | Description |
|---------|-------------|
| `help` | Show available commands |
| `ip addr` | Display network interface configuration (IP/MAC/Gateway) |
| `ping <addr>` | Send ICMP Echo requests to an IP or hostname |
| `nslookup <host>` | Resolve a hostname to an IP address using DNS |
| `netstat` | Show network device status |
| `alloc <bytes>` | Allocate memory on the heap (debug) |
| `memstats` | Show heap usage statistics |
| `memtest` | Run memory allocation/deallocation stress tests |
| `clear` | Clear the screen |

## Building

To build the kernel, you need the RISC-V target installed:

```bash
rustup target add riscv64gc-unknown-none-elf
```

Build the kernel binary:

```bash
# virt (default)
cargo build --release

# D1 / Lichee RV
cargo build --release --no-default-features --features d1
```

The artifact will be located at `../target/riscv64gc-unknown-none-elf/release/kernel`. Pack an SD card from the repo root with `./build.sh sdcard`.

## Running

Boot the SD card in `riscv-vm` (no `--kernel` flag). Match `--machine` to the Cargo feature:

```bash
# virt kernel (default)
npx virtual-machine --sdcard target/riscv64gc-unknown-none-elf/release/sdcard.img --machine virt --harts 2

# D1 kernel
npx virtual-machine --sdcard target/riscv64gc-unknown-none-elf/release/sdcard.img --machine d1
```

## HDL mailbox DTB (virt VM)

The guest always reserves `.hdl` in DRAM. It publishes `MAILBOX_PRESENT` only
when the VM DTB advertises a matching capability (kill-switch: omit the node).
`riscv-vm` must add this when HDL is enabled (default on virt; omit for
`?hdl=0` / CLI off). D1 never publishes the mailbox to a host GPU.

```
/reserved-memory {
    #address-cells = <2>;
    #size-cells = <2>;
    ranges;
    hdl-mailbox@81400000 {
        compatible = "havy,hdl-mailbox";
        reg = <0x0 0x81400000 0x0 0x21000>;
        havy,abi-major = <1>;
        havy,abi-minor = <0>;
        no-map;
    };
};
```

Optional `/chosen` fallback: `havy,hdl-mailbox = <0x0 0x81400000 0x0 0x21000>`
and `havy,hdl-abi-major = <1>`.



