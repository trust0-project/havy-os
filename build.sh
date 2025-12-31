#!/bin/bash
# Build script for havy_os kernel targeting Lichee RV 86 (Allwinner D1)
#
# Usage:
#   ./build_d1.sh           - Build kernel + filesystem
#   ./build_d1.sh sdcard    - Create complete SD card image
set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
KERNEL_DIR="$SCRIPT_DIR/kernel"
TARGET="riscv64gc-unknown-none-elf"
OUTPUT_DIR="$SCRIPT_DIR/target/$TARGET/release"
# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'
echo -e "${GREEN}═══════════════════════════════════════════════════${NC}"
echo -e "${GREEN}  Building havy_os for Lichee RV 86 (Allwinner D1)  ${NC}"
echo -e "${GREEN}═══════════════════════════════════════════════════${NC}"
# =============================================================================
# Step 1: Build kernel (D1 is now the only target)
# =============================================================================
echo -e "\n${YELLOW}[1/5] Building kernel...${NC}"
cd "$KERNEL_DIR"
cargo build --release --target $TARGET
cd "$SCRIPT_DIR"
# =============================================================================
# Step 2: Create raw binary
# =============================================================================
echo -e "${YELLOW}[2/5] Creating kernel binary...${NC}"
OBJCOPY=""
if command -v riscv64-unknown-elf-objcopy &> /dev/null; then
    OBJCOPY="riscv64-unknown-elf-objcopy"
elif command -v llvm-objcopy &> /dev/null; then
    OBJCOPY="llvm-objcopy"
elif command -v riscv64-linux-gnu-objcopy &> /dev/null; then
    OBJCOPY="riscv64-linux-gnu-objcopy"
fi
if [ -n "$OBJCOPY" ]; then
    $OBJCOPY -O binary "$OUTPUT_DIR/kernel" "$OUTPUT_DIR/kernel.bin"
    echo "  ✓ Created: kernel.bin"
else
    echo "  ⚠ No objcopy found, skipping binary"
fi
# =============================================================================
# Step 3: Build WASM binaries for userspace programs (DISABLED - migrating to native RISC-V)
# =============================================================================
echo -e "${YELLOW}[3/5] Skipping WASM binaries (migrating to native RISC-V)...${NC}"
# WASM build disabled - using native RISC-V binaries only
# RUSTFLAGS="-C link-arg=--initial-memory=2097152" cargo build -p mkfs --release --target wasm32-unknown-unknown --no-default-features
# if command -v wasm-opt &> /dev/null; then
#     echo "  Optimizing WASM binaries..."
#     for wasm in target/wasm32-unknown-unknown/release/*.wasm; do
#         if [[ -f "$wasm" && ! "$wasm" == *"mkfs.wasm"* && ! "$wasm" == *"riscv_vm.wasm"* ]]; then
#             # Use -O2 instead of -O3 to avoid aggressive optimizations that break integer handling
#             wasm-opt -O2 --enable-bulk-memory --enable-sign-ext "$wasm" -o "$wasm.opt" && mv "$wasm.opt" "$wasm"
#         fi
#     done
#     echo "  ✓ WASM binaries optimized"
# else
#     echo "  ⚠ wasm-opt not found, skipping optimization"
# fi

# =============================================================================
# Step 3b: Build native RISC-V ELF binaries for userspace programs
# =============================================================================
echo -e "${YELLOW}[3b/5] Building native RISC-V binaries...${NC}"
# Build all binaries in mkfs package for riscv64
# Only build binaries that have RISC-V support (won't fail silently)
cd mkfs
NATIVE_COUNT=0
for bin_file in src/bin/*.rs; do
    bin_name=$(basename "$bin_file" .rs)
    if cargo build --bin "$bin_name" --release --target riscv64gc-unknown-none-elf --no-default-features 2>/dev/null; then
        echo "  ✓ $bin_name"
        NATIVE_COUNT=$((NATIVE_COUNT + 1))
    fi
done
cd "$SCRIPT_DIR"
echo "  ✓ Built $NATIVE_COUNT native RISC-V binaries"

# =============================================================================
# Step 4: Create filesystem image
# =============================================================================
echo -e "${YELLOW}[4/5] Creating filesystem image...${NC}"
cargo run -p mkfs --release -- \
    --output "$OUTPUT_DIR/fs.img" \
    --dir mkfs/root \
    --size 20
echo "  ✓ Created: fs.img (20MB)"

# =============================================================================
# Step 5: Create SD card image (if requested)
# =============================================================================
if [ "$1" = "sdcard" ]; then
    echo -e "${YELLOW}[5/5] Creating SD card image...${NC}"
    
    SDIMG="$OUTPUT_DIR/sdcard.img"
    BOOT_SIZE_MB=2
    FS_SIZE_MB=20
    TOTAL_SIZE_MB=$((BOOT_SIZE_MB + FS_SIZE_MB + 1))
    
    # Create empty image
    dd if=/dev/zero of="$SDIMG" bs=1M count=$TOTAL_SIZE_MB status=none
    
    # Create MBR partition table manually (portable - works on macOS and Linux)
    # macOS fdisk has different syntax than Linux, so we write MBR directly
    
    # Partition 1: FAT32 (type 0x0C = FAT32 LBA) at sector 2048, size BOOT_SIZE_MB
    # Partition 2: Linux (type 0x83) starting after partition 1
    BOOT_SECTORS=$((BOOT_SIZE_MB * 2048))  # 2048 sectors per MB
    FS_SECTORS=$((FS_SIZE_MB * 2048))
    
    PART1_START=2048
    PART1_SIZE=$BOOT_SECTORS
    PART2_START=$((PART1_START + PART1_SIZE))
    PART2_SIZE=$FS_SECTORS
    
    # Create MBR with partition table
    # MBR structure: 446 bytes bootloader, 64 bytes partition table (4x16), 2 bytes signature
    python3 - "$SDIMG" $PART1_START $PART1_SIZE $PART2_START $PART2_SIZE << 'PYTHON_EOF'
import sys
import struct

img_path = sys.argv[1]
p1_start = int(sys.argv[2])
p1_size = int(sys.argv[3])
p2_start = int(sys.argv[4])
p2_size = int(sys.argv[5])

# Read existing image
with open(img_path, 'rb') as f:
    data = bytearray(f.read())

# Build partition entries (16 bytes each)
def make_partition(bootable, part_type, start_lba, size_sectors):
    entry = bytearray(16)
    entry[0] = 0x80 if bootable else 0x00  # Boot flag
    # CHS addresses (not used for LBA, but required)
    entry[1:4] = bytes([0xFE, 0xFF, 0xFF])  # Start CHS
    entry[4] = part_type  # Partition type
    entry[5:8] = bytes([0xFE, 0xFF, 0xFF])  # End CHS
    # LBA start and size
    entry[8:12] = struct.pack('<I', start_lba)
    entry[12:16] = struct.pack('<I', size_sectors)
    return entry

# Partition 1: FAT32 LBA (0x0C), bootable
part1 = make_partition(True, 0x0C, p1_start, p1_size)
# Partition 2: Linux (0x83)
part2 = make_partition(False, 0x83, p2_start, p2_size)
# Partitions 3 & 4: empty
part3 = bytearray(16)
part4 = bytearray(16)

# Write partition table at offset 446
data[446:462] = part1
data[462:478] = part2
data[478:494] = part3
data[494:510] = part4

# Write MBR signature
data[510] = 0x55
data[511] = 0xAA

# Write back
with open(img_path, 'wb') as f:
    f.write(data)

print(f"  Partition 1: FAT32 at sector {p1_start}, {p1_size} sectors ({p1_size * 512 // 1024 // 1024}MB)")
print(f"  Partition 2: Linux at sector {p2_start}, {p2_size} sectors ({p2_size * 512 // 1024 // 1024}MB)")
PYTHON_EOF
    
    echo "  ✓ Created MBR partition table"
    
    # Format boot partition (requires root or fuse)
    if command -v mformat &> /dev/null; then
        # Use mtools for FAT32 (no root needed)
        BOOT_OFFSET=$((2048 * 512))
        BOOT_SECTORS=$((BOOT_SIZE_MB * 2048))
        
        mformat -i "$SDIMG@@$BOOT_OFFSET" -F -v BOOT ::
        
        # Copy kernel to boot partition
        if [ -f "$OUTPUT_DIR/kernel.bin" ]; then
            mcopy -i "$SDIMG@@$BOOT_OFFSET" "$OUTPUT_DIR/kernel.bin" ::
            echo "  ✓ Copied kernel.bin to boot partition"
        fi
    else
        echo "  ⚠ mtools not found, boot partition not formatted"
        echo "    Install: brew install mtools (macOS) or apt install mtools (Linux)"
    fi
    
    # Write filesystem to partition 2
    FS_OFFSET=$(((2048 + BOOT_SIZE_MB * 2048) * 512))
    dd if="$OUTPUT_DIR/fs.img" of="$SDIMG" bs=512 seek=$((2048 + BOOT_SIZE_MB * 2048)) conv=notrunc status=none
    echo "  ✓ Wrote fs.img to partition 2"
    
    echo -e "\n${CYAN}SD Card Image Ready:${NC} $SDIMG"
    echo ""
    echo "To write to real SD card:"
    echo "  sudo dd if=$SDIMG of=/dev/sdX bs=1M status=progress"
    echo ""
else
    echo -e "${YELLOW}[5/5] Skipping SD card image${NC}"
    echo "  Run with 'sdcard' argument to create bootable image"
fi

# =============================================================================
# Summary
# =============================================================================
echo -e "\n${GREEN}═══════════════════════════════════════════════════${NC}"
echo -e "${GREEN}  Build Complete!${NC}"
echo -e "${GREEN}═══════════════════════════════════════════════════${NC}"
echo ""
echo "Output files in: $OUTPUT_DIR/"
echo "  • kernel      - ELF executable"
[ -f "$OUTPUT_DIR/kernel.bin" ] && echo "  • kernel.bin - Raw binary ($(stat -f%z "$OUTPUT_DIR/kernel.bin" 2>/dev/null || stat -c%s "$OUTPUT_DIR/kernel.bin") bytes)"
[ -f "$OUTPUT_DIR/fs.img" ] && echo "  • fs.img      - Filesystem image"
[ -f "$OUTPUT_DIR/sdcard.img" ] && echo "  • sdcard.img  - Complete SD card image"
echo ""
echo -e "${CYAN}U-Boot boot commands:${NC}"
echo "  load mmc 0:1 0x40200000 kernel.bin"
echo "  go 0x40200000"
