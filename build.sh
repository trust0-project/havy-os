#!/bin/bash

set -e

cargo build -p kernel --target riscv64gc-unknown-none-elf --release
RUSTFLAGS="-C link-arg=--initial-memory=2097152" cargo build -p mkfs --release --target wasm32-unknown-unknown --no-default-features
if command -v wasm-opt &> /dev/null; then
    echo "Optimizing WASM binaries..."
    for wasm in target/wasm32-unknown-unknown/release/*.wasm; do
        if [[ -f "$wasm" && ! "$wasm" == *"mkfs.wasm"* && ! "$wasm" == *"riscv_vm.wasm"* ]]; then
            # Use -O2 instead of -O3 to avoid aggressive optimizations that break integer handling
            wasm-opt -O2 --enable-bulk-memory --enable-sign-ext "$wasm" -o "$wasm.opt" && mv "$wasm.opt" "$wasm"
        fi
    done
fi
cargo run -p mkfs -- --output target/riscv64gc-unknown-none-elf/release/fs.img --dir mkfs/root --size 2

