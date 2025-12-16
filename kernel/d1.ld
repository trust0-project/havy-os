/* Linker script for Allwinner D1 / Lichee RV 86 */
/* Kernel runs after OpenSBI (first 2MB reserved) */

MEMORY
{
    RAM : ORIGIN = 0x40200000, LENGTH = 510M
}

ENTRY(_start)

SECTIONS
{
    . = 0x40200000;
    
    .text : ALIGN(4K) {
        *(.text.entry)
        *(.text .text.*)
    } > RAM
    
    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    } > RAM
    
    .data : ALIGN(4K) {
        *(.data .data.*)
        *(.sdata .sdata.*)
    } > RAM
    
    .bss : ALIGN(4K) {
        _sbss = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
        _ebss = .;
    } > RAM
    
    . = ALIGN(4K);
    _end = .;
    
    /* Heap starts after kernel */
    _heap_start = .;
    _heap_size = 56M;
    
    /DISCARD/ : {
        *(.eh_frame)
        *(.eh_frame_hdr)
    }
}
