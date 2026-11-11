//! CPU exception handling: IDT, entry stubs and the common Rust handler.

use core::arch::{asm, global_asm};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::boot::KERNEL_CS;

const VECTORS: usize = 32;
const BREAKPOINT: u64 = 3;
const PAGE_FAULT: u64 = 14;

// One stub per exception vector. Each normalises the stack to
// (vector, error code) and funnels into isr_common, which saves the general
// purpose registers and calls `trap_handler(&mut TrapFrame)`.
global_asm!(
    r#"
    .macro ISR_NOERR n
    .balign 16
isr_\n\():
    pushq $0
    pushq $\n
    jmp isr_common
    .endm

    .macro ISR_ERR n
    .balign 16
isr_\n\():
    pushq $\n
    jmp isr_common
    .endm

    .section .text
    ISR_NOERR 0
    ISR_NOERR 1
    ISR_NOERR 2
    ISR_NOERR 3
    ISR_NOERR 4
    ISR_NOERR 5
    ISR_NOERR 6
    ISR_NOERR 7
    ISR_ERR   8
    ISR_NOERR 9
    ISR_ERR   10
    ISR_ERR   11
    ISR_ERR   12
    ISR_ERR   13
    ISR_ERR   14
    ISR_NOERR 15
    ISR_NOERR 16
    ISR_ERR   17
    ISR_NOERR 18
    ISR_NOERR 19
    ISR_NOERR 20
    ISR_ERR   21
    ISR_NOERR 22
    ISR_NOERR 23
    ISR_NOERR 24
    ISR_NOERR 25
    ISR_NOERR 26
    ISR_NOERR 27
    ISR_NOERR 28
    ISR_ERR   29
    ISR_ERR   30
    ISR_NOERR 31

isr_common:
    pushq %rax
    pushq %rbx
    pushq %rcx
    pushq %rdx
    pushq %rsi
    pushq %rdi
    pushq %rbp
    pushq %r8
    pushq %r9
    pushq %r10
    pushq %r11
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    movq %rsp, %rdi
    cld
    call trap_handler
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %r11
    popq %r10
    popq %r9
    popq %r8
    popq %rbp
    popq %rdi
    popq %rsi
    popq %rdx
    popq %rcx
    popq %rbx
    popq %rax
    addq $16, %rsp
    iretq

    .macro ISR_ADDR n
    .quad isr_\n
    .endm

    .section .rodata
    .balign 8
    .global isr_stub_table
isr_stub_table:
    ISR_ADDR 0
    ISR_ADDR 1
    ISR_ADDR 2
    ISR_ADDR 3
    ISR_ADDR 4
    ISR_ADDR 5
    ISR_ADDR 6
    ISR_ADDR 7
    ISR_ADDR 8
    ISR_ADDR 9
    ISR_ADDR 10
    ISR_ADDR 11
    ISR_ADDR 12
    ISR_ADDR 13
    ISR_ADDR 14
    ISR_ADDR 15
    ISR_ADDR 16
    ISR_ADDR 17
    ISR_ADDR 18
    ISR_ADDR 19
    ISR_ADDR 20
    ISR_ADDR 21
    ISR_ADDR 22
    ISR_ADDR 23
    ISR_ADDR 24
    ISR_ADDR 25
    ISR_ADDR 26
    ISR_ADDR 27
    ISR_ADDR 28
    ISR_ADDR 29
    ISR_ADDR 30
    ISR_ADDR 31
"#,
    options(att_syntax, raw)
);

extern "C" {
    static isr_stub_table: [u64; VECTORS];
}

/// Register state saved by `isr_common`, lowest address first.
#[repr(C)]
#[derive(Debug)]
pub struct TrapFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    const MISSING: IdtEntry = IdtEntry {
        offset_low: 0,
        selector: 0,
        ist: 0,
        type_attr: 0,
        offset_mid: 0,
        offset_high: 0,
        reserved: 0,
    };

    /// Present, DPL 0, 64-bit interrupt gate (interrupts stay off in the
    /// handler).
    fn interrupt_gate(handler: u64) -> IdtEntry {
        IdtEntry {
            offset_low: handler as u16,
            selector: KERNEL_CS,
            ist: 0,
            type_attr: 0x8e,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved: 0,
        }
    }
}

#[repr(C, packed)]
struct IdtPointer {
    limit: u16,
    base: u64,
}

struct Idt(UnsafeCell<[IdtEntry; VECTORS]>);
// SAFETY: written once in `init` on the boot CPU before any trap can be
// delivered, read-only (by the CPU) afterwards.
unsafe impl Sync for Idt {}

static IDT: Idt = Idt(UnsafeCell::new([IdtEntry::MISSING; VECTORS]));
static BREAKPOINTS: AtomicU64 = AtomicU64::new(0);

pub fn init() {
    // SAFETY: single-threaded early boot; see `Idt`. The stub table is
    // emitted by the global_asm block above with exactly VECTORS entries.
    unsafe {
        let idt = &mut *IDT.0.get();
        for (entry, &stub) in idt.iter_mut().zip(isr_stub_table.iter()) {
            *entry = IdtEntry::interrupt_gate(stub);
        }
        let ptr = IdtPointer {
            limit: (core::mem::size_of::<[IdtEntry; VECTORS]>() - 1) as u16,
            base: idt.as_ptr() as u64,
        };
        asm!("lidt [{}]", in(reg) &ptr, options(readonly, nostack, preserves_flags));
    }
}

/// Proves the trap path round-trips: raise #BP, expect the handler to count
/// it and return here.
pub fn self_test() {
    let before = BREAKPOINTS.load(Ordering::Relaxed);
    // SAFETY: int3 is handled by `trap_handler`, which returns.
    unsafe { asm!("int3", options(nomem, nostack)) };
    let after = BREAKPOINTS.load(Ordering::Relaxed);
    assert_eq!(after, before + 1, "breakpoint trap was not delivered");
    kprintln!("traps: IDT live, breakpoint round-trip ok");
}

#[no_mangle]
extern "C" fn trap_handler(frame: &mut TrapFrame) {
    if frame.vector == BREAKPOINT {
        BREAKPOINTS.fetch_add(1, Ordering::Relaxed);
        return;
    }

    let cr2: u64 = if frame.vector == PAGE_FAULT {
        let v: u64;
        // SAFETY: reading CR2 has no side effects.
        unsafe { asm!("mov {}, cr2", out(reg) v, options(nomem, nostack, preserves_flags)) };
        v
    } else {
        0
    };
    crate::console::print_unlocked(format_args!(
        "\nFATAL TRAP {} ({}) error={:#x} rip={:#x} rsp={:#x} cr2={:#x}\n{:#x?}\n",
        frame.vector,
        name(frame.vector),
        frame.error,
        frame.rip,
        frame.rsp,
        cr2,
        frame
    ));
    crate::halt();
}

fn name(vector: u64) -> &'static str {
    match vector {
        0 => "divide error",
        1 => "debug",
        2 => "nmi",
        3 => "breakpoint",
        4 => "overflow",
        5 => "bound range",
        6 => "invalid opcode",
        7 => "device not available",
        8 => "double fault",
        10 => "invalid tss",
        11 => "segment not present",
        12 => "stack fault",
        13 => "general protection",
        14 => "page fault",
        16 => "x87 fp",
        17 => "alignment check",
        18 => "machine check",
        19 => "simd fp",
        20 => "virtualization",
        21 => "control protection",
        _ => "reserved",
    }
}
