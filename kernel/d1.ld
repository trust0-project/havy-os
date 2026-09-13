/* ============================================================================
 * havy_os D1 linker script (Allwinner D1 / Lichee RV)
 *
 * DRAM: 0x4000_0000, 512 MiB. Kernel after OpenSBI at 0x4020_0000.
 *
 * Layout (Phase 1 freeze):
 *   kernel image .............. 0x4020_0000 .. _ebss
 *   FB doorbell page ........... 0x40FF_F000 (4 KiB, reserved, in D1 DRAM)
 *   scanout framebuffer ........ 0x4100_0000 (480×480, stride 2048)
 *   HDL mailbox ................ 0x4140_0000 (132 KiB, in-kernel only)
 *   heap ...................... _ehdl .. stack_tail
 *   hart stack ................ RAM top, 1 × 128 KiB (C906 is UP)
 *
 * .hdl uses the same DRAM-relative offset as virt (DRAM+0x0140_0000).
 * D1 never publishes this mailbox to a host GPU; MAILBOX_PRESENT stays 0.
 * Never use virt PAs 0x8100_0000 / 0x80FF_FFE0 on this map.
 * ============================================================================ */

MEMORY
{
    RAM : ORIGIN = 0x40200000, LENGTH = 510M
}

REGION_ALIAS("REGION_TEXT", RAM);
REGION_ALIAS("REGION_RODATA", RAM);
REGION_ALIAS("REGION_DATA", RAM);
REGION_ALIAS("REGION_BSS", RAM);
REGION_ALIAS("REGION_HEAP", RAM);
REGION_ALIAS("REGION_STACK", RAM);

PROVIDE(_stext = ORIGIN(REGION_TEXT));
PROVIDE(_stack_start = ORIGIN(REGION_STACK) + LENGTH(REGION_STACK));

_max_hart_id = 0;              /* Uniprocessor C906 */
_hart_stack_size = 128K;

/* ST7701 480×480. 480×4 = 1920 is not a multiple of 256; pad stride to 2048. */
_fb_width = 480;
_fb_height = 480;
_fb_stride = 2048;
_fb_bytes = _fb_stride * _fb_height;

_stack_reserve = 128K;

/* Two-slot HDL mailbox. In-kernel only; not advertised to a host GPU. */
_hdl_origin = 0x41400000;
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

  .fb_meta 0x40FFF000 (NOLOAD) :
  {
    _sfb_meta = .;
    . += 4096;
    _efb_meta = .;
  } > RAM

  .fb 0x41000000 (NOLOAD) :
  {
    _sfb = .;
    . += _fb_bytes;
    _efb = .;
  } > RAM

  /* In-kernel HDL mailbox. Carves the region out of the heap. */
  .hdl _hdl_origin (NOLOAD) :
  {
    _shdl = .;
    . += _hdl_bytes;
    _ehdl = .;
  } > RAM

  .heap (NOLOAD) :
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
ASSERT(_ebss <= 0x40FFF000, "ERROR: kernel image overlaps the reserved FB doorbell page at 0x40FF_F000");
ASSERT(_sfb >= 0x40000000 && _efb <= 0x60000000, "ERROR: D1 framebuffer is not inside D1 DRAM");
ASSERT(_efb <= _shdl, "ERROR: framebuffer overlaps the HDL mailbox");
ASSERT(_shdl == 0x41400000, "ERROR: HDL mailbox moved; Phase 1 freeze is 0x4140_0000");
ASSERT(_ehdl - _shdl == 0x21000, "ERROR: HDL mailbox size is not 132 KiB");
ASSERT(_ehdl <= _sheap, "ERROR: HDL mailbox overlaps the heap");
ASSERT(_shdl >= 0x40000000 && _ehdl <= 0x60000000, "ERROR: D1 HDL mailbox is not inside D1 DRAM");
ASSERT(_eheap > _sheap, "ERROR: heap is empty; check _stack_reserve");
ASSERT(_stext + SIZEOF(.text) < ORIGIN(REGION_TEXT) + LENGTH(REGION_TEXT), "
ERROR(riscv-rt): The .text section must be placed inside the REGION_TEXT region.");
ASSERT(SIZEOF(.stack) >= (_max_hart_id + 1) * _hart_stack_size, "
ERROR(riscv-rt): .stack section is too small for allocating stacks for all the harts.
Consider changing `_max_hart_id` or `_hart_stack_size`.");
ASSERT(SIZEOF(.got) == 0, "
.got section detected. Dynamic relocations are not supported.");
