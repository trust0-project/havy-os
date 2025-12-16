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
 * _max_hart_id: Maximum hart ID (127 = support for harts 0-127)
 * _hart_stack_size: Stack size per hart (64KB each)
 * 
 * riscv-rt calculates secondary hart stack pointers as:
 *   sp = _stack_start - hart_id * _hart_stack_size
 */
PROVIDE(_max_hart_id = 127);
PROVIDE(_hart_stack_size = 0x10000);
