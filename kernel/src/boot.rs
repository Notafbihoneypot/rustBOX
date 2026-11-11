//! PVH entry: 32-bit protected mode, paging off, `ebx` = `HvmStartInfo`.
//!
//! The stub identity-maps the first 4 GiB with 2 MiB pages, switches to long
//! mode and calls `kmain(start_info_paddr)`. Everything it touches lives in
//! the image, below 4 GiB.

use core::arch::global_asm;

global_asm!(
    r#"
    .section .note.xen, "a", @note
    .balign 4
    .long 4                     /* namesz: "Xen\0" */
    .long 4                     /* descsz */
    .long 18                    /* XEN_ELFNOTE_PHYS32_ENTRY */
    .asciz "Xen"
    .balign 4
    .long _start32
    .balign 4

    .section .text.boot, "ax"
    .code32
    .global _start32
_start32:
    cld
    cli
    movl $boot_stack_top, %esp
    movl %ebx, %edi             /* start_info survives until kmain */

    /* PML4[0] -> PDPT, PDPT[0..4] -> PD[0..4], 2048 x 2 MiB leaves. */
    movl $boot_pdpt, %eax
    orl $0x3, %eax
    movl %eax, boot_pml4
    movl $boot_pd, %eax
    orl $0x3, %eax
    xorl %ecx, %ecx
1:  movl %eax, boot_pdpt(,%ecx,8)
    addl $0x1000, %eax
    incl %ecx
    cmpl $4, %ecx
    jne 1b
    xorl %ecx, %ecx
2:  movl %ecx, %eax
    shll $21, %eax
    orl $0x83, %eax             /* present | writable | 2 MiB */
    movl %eax, boot_pd(,%ecx,8)
    incl %ecx
    cmpl $2048, %ecx
    jne 2b

    movl $boot_pml4, %eax
    movl %eax, %cr3
    movl %cr4, %eax
    orl $0x20, %eax             /* CR4.PAE */
    movl %eax, %cr4
    movl $0xC0000080, %ecx      /* EFER */
    rdmsr
    orl $0x100, %eax            /* EFER.LME */
    wrmsr
    movl %cr0, %eax
    orl $0x80010001, %eax       /* CR0.PG | CR0.WP | CR0.PE */
    movl %eax, %cr0
    lgdt boot_gdt_desc
    ljmp $0x08, $_start64

    .code64
_start64:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    xorl %eax, %eax
    movw %ax, %fs
    movw %ax, %gs
    movq $boot_stack_top, %rsp
    xorl %ebp, %ebp
    movl %edi, %edi             /* zero-extend start_info pointer */
    call kmain
3:  hlt
    jmp 3b

    .section .rodata
    .balign 8
boot_gdt:
    .quad 0
    .quad 0x00AF9A000000FFFF    /* 0x08: 64-bit code */
    .quad 0x00CF92000000FFFF    /* 0x10: data */
boot_gdt_desc:
    .word boot_gdt_desc - boot_gdt - 1
    .quad boot_gdt

    .section .bss
    .balign 4096
boot_pml4:
    .skip 4096
boot_pdpt:
    .skip 4096
boot_pd:
    .skip 4096 * 4
    .global hypercall_page
hypercall_page:
    .skip 4096
boot_stack:
    .skip 65536
boot_stack_top:
"#,
    options(att_syntax, raw)
);

/// Kernel code segment selector installed by the boot stub.
pub const KERNEL_CS: u16 = 0x08;
