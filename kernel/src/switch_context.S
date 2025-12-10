# Context switch assembly for RISC-V 64-bit
#
# void switch_context(Context *old, Context *new);
#
# Saves the current CPU state into `old`, then loads the state from `new`
# and resumes execution there. This is the core of cooperative multitasking.
#
# Context layout (must match process.rs Context struct):
#   0x00: ra    (return address)
#   0x08: sp    (stack pointer)
#   0x10: s0    (frame pointer / callee-saved)
#   0x18: s1
#   0x20: s2
#   0x28: s3
#   0x30: s4
#   0x38: s5
#   0x40: s6
#   0x48: s7
#   0x50: s8
#   0x58: s9
#   0x60: s10
#   0x68: s11
#
# Arguments:
#   a0 = pointer to old Context (to save current state)
#   a1 = pointer to new Context (to load new state)

.section .text
.global switch_context
.align 4
switch_context:
    # Save current context into old (a0)
    sd ra, 0x00(a0)     # Save return address
    sd sp, 0x08(a0)     # Save stack pointer
    sd s0, 0x10(a0)     # Save callee-saved registers
    sd s1, 0x18(a0)
    sd s2, 0x20(a0)
    sd s3, 0x28(a0)
    sd s4, 0x30(a0)
    sd s5, 0x38(a0)
    sd s6, 0x40(a0)
    sd s7, 0x48(a0)
    sd s8, 0x50(a0)
    sd s9, 0x58(a0)
    sd s10, 0x60(a0)
    sd s11, 0x68(a0)

    # Load new context from new (a1)
    ld ra, 0x00(a1)     # Load return address
    ld sp, 0x08(a1)     # Load stack pointer
    ld s0, 0x10(a1)     # Load callee-saved registers
    ld s1, 0x18(a1)
    ld s2, 0x20(a1)
    ld s3, 0x28(a1)
    ld s4, 0x30(a1)
    ld s5, 0x38(a1)
    ld s6, 0x40(a1)
    ld s7, 0x48(a1)
    ld s8, 0x50(a1)
    ld s9, 0x58(a1)
    ld s10, 0x60(a1)
    ld s11, 0x68(a1)

    # Return - this jumps to new ra (either back to caller or to entry point)
    ret
