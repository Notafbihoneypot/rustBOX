//! 16550 UART on COM1, used when no hypervisor console is available.
//!
//! Under Xen the UART belongs to the hypervisor: a PVH dom0 reads 0xff from
//! its ports, which `init` treats as "no UART here".

use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

const COM1: u16 = 0x3f8;
const LSR: u16 = COM1 + 5;
const LSR_THR_EMPTY: u8 = 0x20;

static PRESENT: AtomicBool = AtomicBool::new(false);

unsafe fn outb(port: u16, val: u8) {
    asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack, preserves_flags));
}

unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    asm!("in al, dx", in("dx") port, out("al") val, options(nomem, nostack, preserves_flags));
    val
}

pub fn init() {
    // SAFETY: COM1 is a fixed legacy port range; touching it has no memory
    // safety implications.
    unsafe {
        if inb(LSR) == 0xff {
            return;
        }
        outb(COM1 + 1, 0x00); // no interrupts
        outb(COM1 + 3, 0x80); // DLAB
        outb(COM1, 0x01); // 115200 baud
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03); // 8n1
        outb(COM1 + 2, 0xc7); // FIFO on, cleared
        outb(COM1 + 4, 0x03); // DTR | RTS
    }
    PRESENT.store(true, Ordering::Relaxed);
}

pub fn write(bytes: &[u8]) {
    if !PRESENT.load(Ordering::Relaxed) {
        return;
    }
    for &b in bytes {
        if b == b'\n' {
            put(b'\r');
        }
        put(b);
    }
}

fn put(b: u8) {
    // SAFETY: see `init`. The wait is bounded so a wedged UART cannot hang
    // the kernel.
    unsafe {
        for _ in 0..100_000 {
            if inb(LSR) & LSR_THR_EMPTY != 0 {
                break;
            }
            core::hint::spin_loop();
        }
        outb(COM1, b);
    }
}
