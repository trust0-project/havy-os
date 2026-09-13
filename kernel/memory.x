MEMORY
{
    /* 
     * RAM starts at 0x80000000.
     * We allocate 512MB to match the VM's DRAM size.
     */
    RAM : ORIGIN = 0x80000000, LENGTH = 512M
}

REGION_ALIAS("REGION_TEXT", RAM);
REGION_ALIAS("REGION_RODATA", RAM);
REGION_ALIAS("REGION_DATA", RAM);
REGION_ALIAS("REGION_BSS", RAM);
REGION_ALIAS("REGION_HEAP", RAM);
REGION_ALIAS("REGION_STACK", RAM);

/* Multi-hart configuration for riscv-rt.
 * _max_hart_id: Inclusive max hart ID. Virt cap is 7 (8 harts); D1 is 0.
 * The live values live in link.x / d1.ld; this file is not the virt script.
 *
 * riscv-rt calculates secondary hart stack pointers as:
 *   sp = _stack_start - hart_id * _hart_stack_size
 */
PROVIDE(_max_hart_id = 7);
PROVIDE(_hart_stack_size = 0x20000);
