//! rustBOX kernel.
//!
//! Milestone 0: boot through the Xen PVH protocol (as dom0, as a domU, or
//! directly under QEMU's PVH loader), bring up a console, take over the trap
//! vectors and account for physical memory. See `docs/ROADMAP.md` for what
//! comes next.
#![no_std]
#![no_main]

mod boot;
#[macro_use]
mod console;
mod mm;
mod serial;
mod traps;
mod xen;

use core::ffi::CStr;
use core::panic::PanicInfo;
use xen_abi::start_info::{HvmStartInfo, HVM_START_MAGIC, SIF_INITDOMAIN, SIF_PRIVILEGED};

/// Printed once every boot-time self-check has passed. `cargo xtask` boot
/// tests wait for this line.
const BOOT_OK: &str = "rustbox: boot ok";

/// Rust entry point, called by the boot stub in 64-bit mode on the boot
/// stack with interrupts off and the first 4 GiB identity-mapped.
#[no_mangle]
extern "C" fn kmain(start_info_paddr: u64) -> ! {
    serial::init();
    let xen = xen::init();

    kprintln!("rustBOX kernel {}", env!("CARGO_PKG_VERSION"));
    match &xen {
        Some(x) => kprintln!("hypervisor: Xen {}.{}", x.version.0, x.version.1),
        None => kprintln!("hypervisor: none detected (bare PVH loader)"),
    }

    // SAFETY: the PVH boot protocol guarantees ebx pointed at a start_info
    // structure below 4 GiB, which the boot stub identity-mapped. Builders
    // do not promise any alignment, so copy it out bytewise.
    let si = unsafe { (start_info_paddr as *const HvmStartInfo).read_unaligned() };
    if si.magic != HVM_START_MAGIC {
        panic!(
            "bad start_info magic {:#x} at {:#x}",
            si.magic, start_info_paddr
        );
    }
    let role = match (
        si.flags & SIF_INITDOMAIN != 0,
        si.flags & SIF_PRIVILEGED != 0,
    ) {
        (true, _) => "dom0 (initial control domain)",
        (false, true) => "privileged domU",
        (false, false) => "unprivileged guest",
    };
    kprintln!("role: {}", role);
    if si.cmdline_paddr != 0 {
        // SAFETY: a non-zero cmdline_paddr is a NUL-terminated string
        // provided by the domain builder.
        let cmdline = unsafe { CStr::from_ptr(si.cmdline_paddr as *const core::ffi::c_char) };
        kprintln!("cmdline: {}", cmdline.to_str().unwrap_or("<not utf-8>"));
    }

    traps::init();
    traps::self_test();

    mm::init(&si, start_info_paddr, xen.is_some());

    kprintln!("{}", BOOT_OK);
    halt()
}

fn halt() -> ! {
    loop {
        // SAFETY: hlt has no memory or stack effects.
        unsafe { core::arch::asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    console::print_unlocked(format_args!("\nKERNEL PANIC: {}\n", info));
    halt()
}
