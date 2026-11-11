/* rustBOX kernel image layout.
 *
 * Virtual == physical: the boot stub identity-maps the first 4 GiB, and both
 * Xen's PVH domain builder and QEMU's PVH loader place segments at p_paddr.
 * The PT_NOTE segment carries XEN_ELFNOTE_PHYS32_ENTRY, which is how either
 * loader finds the 32-bit entry point.
 */
ENTRY(_start32)

PHDRS
{
    text   PT_LOAD FLAGS(5);
    rodata PT_LOAD FLAGS(4);
    data   PT_LOAD FLAGS(6);
    note   PT_NOTE FLAGS(4);
}

SECTIONS
{
    . = 0x200000;
    __kernel_start = .;

    .text : {
        *(.text.boot)
        *(.text .text.*)
    } :text

    . = ALIGN(4K);
    .rodata : {
        *(.rodata .rodata.*)
    } :rodata

    .note.xen : {
        KEEP(*(.note.xen))
    } :rodata :note

    . = ALIGN(4K);
    .data : {
        *(.data .data.*)
        *(.got .got.*)
    } :data

    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    } :data

    . = ALIGN(4K);
    __kernel_end = .;

    /DISCARD/ : {
        *(.eh_frame*)
        *(.note.gnu*)
        *(.comment)
    }
}
