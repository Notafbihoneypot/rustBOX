//! Xen detection and hypercalls for an HVM-container (PVH) guest.

use core::arch::asm;
use core::arch::x86_64::__cpuid;
use core::sync::atomic::{AtomicBool, Ordering};

use xen_abi::cpuid::*;
use xen_abi::hypercall::*;

extern "C" {
    /// One zeroed page in .bss; Xen fills it with hypercall stubs.
    static hypercall_page: [u8; HYPERCALL_PAGE_SIZE];
}

static HYPERCALLS_READY: AtomicBool = AtomicBool::new(false);
static CONSOLE_USABLE: AtomicBool = AtomicBool::new(false);

pub struct Xen {
    pub version: (u16, u16),
}

/// Detects Xen and installs the hypercall page. Returns `None` when not
/// running on Xen.
pub fn init() -> Option<Xen> {
    let base = find_base_leaf()?;
    let leaf = __cpuid(base + LEAF_HYPERCALL);
    let msr = leaf.ebx;
    let page = core::ptr::addr_of!(hypercall_page) as u64;
    // SAFETY: Xen documents this MSR (from its CPUID leaf) as the way to
    // install hypercall page 0 at the given guest-physical address; the page
    // is ours, page-aligned and unused until now.
    unsafe {
        asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") page as u32,
            in("edx") (page >> 32) as u32,
            options(nomem, nostack, preserves_flags)
        );
    }
    HYPERCALLS_READY.store(true, Ordering::Release);
    CONSOLE_USABLE.store(true, Ordering::Release);

    let v = hypercall(HYPERVISOR_XEN_VERSION, XENVER_VERSION, 0, 0);
    Some(Xen {
        version: split_version(v as u32),
    })
}

fn find_base_leaf() -> Option<u32> {
    // CPUID.1:ECX[31] is the "running under a hypervisor" bit; without it
    // the 0x4000_0000 range is not defined.
    if __cpuid(1).ecx & (1 << 31) == 0 {
        return None;
    }
    let mut base = HYPERVISOR_LEAF_BASE;
    while base < HYPERVISOR_LEAF_LIMIT {
        let r = __cpuid(base);
        if is_xen_signature(r.ebx, r.ecx, r.edx) && r.eax >= base + LEAF_HYPERCALL {
            return Some(base);
        }
        base += HYPERVISOR_LEAF_STEP;
    }
    None
}

/// Issues a hypercall with up to three arguments. Returns Xen's result
/// (negative errno on failure), or `-ENOSYS` when not running on Xen.
pub fn hypercall(nr: usize, a1: usize, a2: usize, a3: usize) -> isize {
    const ENOSYS: isize = 38;
    if !HYPERCALLS_READY.load(Ordering::Acquire) || nr >= HYPERCALL_PAGE_SIZE / HYPERCALL_STUB_SIZE
    {
        return -ENOSYS;
    }
    let entry = core::ptr::addr_of!(hypercall_page) as usize + nr * HYPERCALL_STUB_SIZE;
    let ret: isize;
    // SAFETY: the page was populated by Xen in `init`; each 32-byte stub
    // loads the hypercall number, traps to Xen and returns. Per the x86_64
    // hypercall ABI arguments travel in rdi/rsi/rdx and Xen may clobber the
    // argument registers; rcx/r11 are listed for PV-style stubs.
    unsafe {
        asm!(
            "call {entry}",
            entry = in(reg) entry,
            inlateout("rdi") a1 => _,
            inlateout("rsi") a2 => _,
            inlateout("rdx") a3 => _,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r8") _,
            lateout("r9") _,
            lateout("r10") _,
            lateout("r11") _,
        );
    }
    ret
}

/// Writes to Xen's console via `CONSOLEIO_write`. Always permitted for dom0;
/// for a domU only on debug hypervisors, so the first refusal turns this
/// path off (a domU needs the PV console ring instead: see the roadmap).
pub fn console_write(bytes: &[u8]) {
    if bytes.is_empty() || !CONSOLE_USABLE.load(Ordering::Acquire) {
        return;
    }
    let rc = hypercall(
        HYPERVISOR_CONSOLE_IO,
        CONSOLEIO_WRITE,
        bytes.len(),
        bytes.as_ptr() as usize,
    );
    if rc < 0 {
        CONSOLE_USABLE.store(false, Ordering::Release);
    }
}

/// Fetches the guest memory map from Xen. Returns the number of entries
/// written to `buf`.
pub fn memory_map(buf: &mut [E820Entry]) -> Result<usize, isize> {
    let mut arg = XenMemoryMap {
        nr_entries: buf.len() as u32,
        buffer: buf.as_mut_ptr(),
    };
    let rc = hypercall(
        HYPERVISOR_MEMORY_OP,
        XENMEM_MEMORY_MAP,
        core::ptr::addr_of_mut!(arg) as usize,
        0,
    );
    if rc < 0 {
        return Err(rc);
    }
    Ok((arg.nr_entries as usize).min(buf.len()))
}
