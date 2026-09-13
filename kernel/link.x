/* ============================================================================
 * havy_os virt linker script (QEMU virt / Machine::Virt)
 *
 * DRAM: 0x8000_0000, 512 MiB
 *
 * Layout (Phase 1 freeze):
 *   kernel image .............. 0x8000_0000 .. _ebss
 *   FB doorbell page ........... 0x80FF_F000 (4 KiB, reserved)
 *   scanout framebuffer ........ 0x8100_0000 (3 MiB, stride-padded, NOLOAD)
 *   HDL mailbox ................ 0x8140_0000 (132 KiB: 4 KiB control + 2 × 64 KiB)
 *   DTB (VM) .................. 0x8200_0000 .. 0x8201_0000
 *   heap ...................... 0x8201_0000 .. stack_tail
 *   hart stacks ............... RAM top, 8 × 128 KiB
 *
 * ~13 MiB remains between .fb end (0x8130_0000) and _heap_origin. .hdl sits
 * in that hole; the allocator never claims it.
 *
 * SMP cap is 8 harts (ids 0–7). Do not raise _max_hart_id without
 * growing the stack tail — riscv-rt parks sp at
 *   _stack_start - hart_id * _hart_stack_size
 * ============================================================================ */

MEMORY
{
    RAM : ORIGIN = 0x80000000, LENGTH = 512M
}

REGION_ALIAS("REGION_TEXT", RAM);
REGION_ALIAS("REGION_RODATA", RAM);
REGION_ALIAS("REGION_DATA", RAM);
REGION_ALIAS("REGION_BSS", RAM);
REGION_ALIAS("REGION_HEAP", RAM);
REGION_ALIAS("REGION_STACK", RAM);

PROVIDE(_stext = ORIGIN(REGION_TEXT));
PROVIDE(_stack_start = ORIGIN(REGION_STACK) + LENGTH(REGION_STACK));

/* Inclusive max hart id. 8 harts (0–7). D1 uses 0 in d1.ld. */
_max_hart_id = 7;
_hart_stack_size = 128K;

/* 1024×768 XRGB8888, stride 4096 (already a multiple of 256). */
_fb_width = 1024;
_fb_height = 768;
_fb_stride = 4096;
_fb_bytes = _fb_stride * _fb_height;

/* Stack tail: (_max_hart_id + 1) * _hart_stack_size = 1 MiB. */
_stack_reserve = 8 * 128K;

/* Heap begins after the VM DTB (DRAM+32 MiB + 64 KiB). */
_heap_origin = 0x82010000;

/* Two-slot HDL mailbox. 4 KiB control + 2 × 64 KiB slots. Frozen Phase 1. */
_hdl_origin = 0x81400000;
_hdl_control_size = 4096;
_hdl_slot_size = 65536;
_hdl_bytes = _hdl_control_size + 2 * _hdl_slot_size;

PROVIDE(UserSoft = DefaultHandler);
PROVIDE(SupervisorSoft = DefaultHandler);
PROVIDE(MachineSoft = DefaultHandler);
PROVIDE(UserTimer = DefaultHandler);
PROVIDE(SupervisorTimer = DefaultHandler);
PROVIDE(MachineTimer = DefaultHandler);
PROVIDE(UserExternal = DefaultHandler);
PROVIDE(SupervisorExternal = DefaultHandler);
PROVIDE(MachineExternal = DefaultHandler);

PROVIDE(DefaultHandler = DefaultInterruptHandler);
PROVIDE(ExceptionHandler = DefaultExceptionHandler);
PROVIDE(__pre_init = default_pre_init);
PROVIDE(_setup_interrupts = default_setup_interrupts);
PROVIDE(_mp_hook = default_mp_hook);
PROVIDE(_start_trap = default_start_trap);

SECTIONS
{
  .text.dummy (NOLOAD) :
  {
    . = ABSOLUTE(_stext);
  } > REGION_TEXT

  .text _stext :
  {
    KEEP(*(.init));
    KEEP(*(.init.rust));
    . = ALIGN(4);
    *(.trap);
    *(.trap.rust);
    *libriscv_rt*.rlib:*(.text .text.*);
    *libpanic_halt*.rlib:*(.text .text.*);
    *(.text .text.*);
  } > REGION_TEXT

  .rodata : ALIGN(4)
  {
    *(.srodata .srodata.*);
    *(.rodata .rodata.*);
    . = ALIGN(4);
  } > REGION_RODATA

  .data : ALIGN(4)
  {
    _sidata = LOADADDR(.data);
    _sdata = .;
    PROVIDE(__global_pointer$ = . + 0x800);
    *(.sdata .sdata.* .sdata2 .sdata2.*);
    *(.data .data.*);
    . = ALIGN(4);
    _edata = .;
  } > REGION_DATA AT > REGION_RODATA

  .bss (NOLOAD) :
  {
    _sbss = .;
    *(.sbss .sbss.* .bss .bss.*);
    . = ALIGN(4);
    _ebss = .;
  } > REGION_BSS

  /* Reserved doorbell page. Dirty rect / version sit at the historical
     offsets 0x80FF_FFE0 / 0x80FF_FFFC so the host scrape stays compatible. */
  .fb_meta 0x80FFF000 (NOLOAD) :
  {
    _sfb_meta = .;
    . += 4096;
    _efb_meta = .;
  } > RAM

  /* Guest-owned scanout. Outside the heap. */
  .fb 0x81000000 (NOLOAD) :
  {
    _sfb = .;
    . += _fb_bytes;
    _efb = .;
  } > RAM

  /* HDL latest-frame mailbox. Outside .fb, outside the heap, outside DTB.
     Host scrapes this only when the VM DTB advertises havy,hdl-mailbox. */
  .hdl _hdl_origin (NOLOAD) :
  {
    _shdl = .;
    . += _hdl_bytes;
    _ehdl = .;
  } > RAM

  /* Leftover DRAM after DTB, up to the stack tail. */
  .heap _heap_origin (NOLOAD) :
  {
    _sheap = .;
    . = ORIGIN(RAM) + LENGTH(RAM) - _stack_reserve;
    . = ALIGN(4);
    _eheap = .;
  } > REGION_HEAP

  _heap_size = _eheap - _sheap;

  .stack (NOLOAD) :
  {
    _estack = .;
    . = ABSOLUTE(_stack_start);
    _sstack = .;
  } > REGION_STACK

  .got (INFO) :
  {
    KEEP(*(.got .got.*));
  }

  /DISCARD/ :
  {
    *(.eh_frame)
    *(.eh_frame_hdr)
  }
}

ASSERT(ORIGIN(REGION_TEXT) % 4 == 0, "ERROR(riscv-rt): REGION_TEXT must be 4-byte aligned");
ASSERT(ORIGIN(REGION_RODATA) % 4 == 0, "ERROR(riscv-rt): REGION_RODATA must be 4-byte aligned");
ASSERT(ORIGIN(REGION_DATA) % 4 == 0, "ERROR(riscv-rt): REGION_DATA must be 4-byte aligned");
ASSERT(ORIGIN(REGION_HEAP) % 4 == 0, "ERROR(riscv-rt): REGION_HEAP must be 4-byte aligned");
ASSERT(ORIGIN(REGION_STACK) % 4 == 0, "ERROR(riscv-rt): REGION_STACK must be 4-byte aligned");
ASSERT(_stext % 4 == 0, "ERROR(riscv-rt): `_stext` must be 4-byte aligned");
ASSERT(_sdata % 4 == 0 && _edata % 4 == 0, "BUG(riscv-rt): .data is not 4-byte aligned");
ASSERT(_sidata % 4 == 0, "BUG(riscv-rt): the LMA of .data is not 4-byte aligned");
ASSERT(_sbss % 4 == 0 && _ebss % 4 == 0, "BUG(riscv-rt): .bss is not 4-byte aligned");
ASSERT(_sheap % 4 == 0, "BUG(riscv-rt): start of .heap is not 4-byte aligned");
ASSERT(_ebss <= 0x80FFF000, "ERROR: kernel image overlaps the reserved FB doorbell page at 0x80FF_F000");
ASSERT(_efb <= _shdl, "ERROR: framebuffer overlaps the HDL mailbox");
ASSERT(_shdl == 0x81400000, "ERROR: HDL mailbox moved; Phase 1 freeze is 0x8140_0000");
ASSERT(_ehdl - _shdl == 0x21000, "ERROR: HDL mailbox size is not 132 KiB");
ASSERT(_ehdl <= 0x82000000, "ERROR: HDL mailbox overlaps the VM DTB window at 0x8200_0000");
ASSERT(_ehdl <= _heap_origin, "ERROR: HDL mailbox overlaps the heap");
ASSERT(_sheap == _heap_origin, "ERROR: heap origin moved; allocator must stay at 0x8201_0000");
ASSERT(_eheap > _sheap, "ERROR: heap is empty; check _stack_reserve");
ASSERT(_stext + SIZEOF(.text) < ORIGIN(REGION_TEXT) + LENGTH(REGION_TEXT), "
ERROR(riscv-rt): The .text section must be placed inside the REGION_TEXT region.");
ASSERT(SIZEOF(.stack) >= (_max_hart_id + 1) * _hart_stack_size, "
ERROR(riscv-rt): .stack section is too small for allocating stacks for all the harts.
Consider changing `_max_hart_id` or `_hart_stack_size`.");
ASSERT(SIZEOF(.got) == 0, "
.got section detected. Dynamic relocations are not supported.");
