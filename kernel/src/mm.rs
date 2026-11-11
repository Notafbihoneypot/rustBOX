//! Physical memory discovery.
//!
//! Builds the frame allocator from the boot memory map and removes
//! everything the kernel must not hand out: low memory, its own image, the
//! boot structures and modules, and anything above the 4 GiB the boot stub
//! mapped.

use core::cell::UnsafeCell;
use core::ffi::CStr;

use kmem::{FrameAllocator, Range, PAGE_SIZE};
use xen_abi::hypercall::E820Entry;
use xen_abi::start_info::*;

use crate::xen;

const MAX_REGIONS: usize = 64;
const LOW_MEMORY_END: u64 = 1 << 20;
const IDENTITY_MAP_END: u64 = 4 << 30;

extern "C" {
    static __kernel_start: u8;
    static __kernel_end: u8;
}

struct Frames(UnsafeCell<FrameAllocator<MAX_REGIONS>>);
// SAFETY: only touched from the boot CPU during single-threaded init; a lock
// goes around this when secondary CPUs and interrupts arrive.
unsafe impl Sync for Frames {}

static FRAMES: Frames = Frames(UnsafeCell::new(FrameAllocator::new()));

pub fn init(si: &HvmStartInfo, si_paddr: u64, on_xen: bool) {
    // SAFETY: see `Frames`.
    let frames = unsafe { &mut *FRAMES.0.get() };

    let mut ram_bytes = 0u64;
    let mut add = |addr: u64, size: u64, kind: u32| {
        let r = Range::from_len(addr, size);
        kprintln!(
            "  [{:#012x}-{:#012x}) {}",
            r.start,
            r.end,
            memmap_type_name(kind)
        );
        if kind == MEMMAP_TYPE_RAM {
            ram_bytes += r.len();
            frames
                .add_ram(r)
                .expect("memory map has too many RAM regions");
        }
    };

    if si.version >= 1 && si.memmap_entries != 0 {
        kprintln!("memory map (start_info, {} entries):", si.memmap_entries);
        for i in 0..si.memmap_entries as usize {
            // SAFETY: version >= 1 start_info carries `memmap_entries`
            // entries at `memmap_paddr`, below 4 GiB and so identity-mapped;
            // alignment is not guaranteed.
            let e = unsafe {
                (si.memmap_paddr as *const HvmMemmapEntry)
                    .add(i)
                    .read_unaligned()
            };
            add(e.addr, e.size, e.kind);
        }
    } else if on_xen {
        let mut buf = [E820Entry {
            addr: 0,
            size: 0,
            kind: 0,
        }; MAX_REGIONS];
        let n = xen::memory_map(&mut buf).expect("XENMEM_memory_map failed");
        kprintln!("memory map (XENMEM_memory_map, {} entries):", n);
        for e in &buf[..n] {
            add(e.addr, e.size, e.kind);
        }
    } else {
        panic!(
            "no memory map: start_info v{} without entries and no hypervisor",
            si.version
        );
    }

    let mut reserve = |what: &str, r: Range| {
        frames
            .reserve(r)
            .unwrap_or_else(|_| panic!("frame table full reserving {}", what));
    };
    reserve("low memory", Range::new(0, LOW_MEMORY_END));
    reserve(
        "unmapped high memory",
        Range::new(IDENTITY_MAP_END, u64::MAX),
    );
    // Linker-script symbols: only their addresses mean anything.
    let image = Range::new(
        core::ptr::addr_of!(__kernel_start) as u64,
        core::ptr::addr_of!(__kernel_end) as u64,
    );
    reserve("kernel image", image);
    reserve(
        "start_info",
        Range::from_len(si_paddr, core::mem::size_of::<HvmStartInfo>() as u64),
    );
    reserve("command line", cstr_range(si.cmdline_paddr));
    if si.version >= 1 {
        let bytes = si.memmap_entries as u64 * core::mem::size_of::<HvmMemmapEntry>() as u64;
        reserve("memory map", Range::from_len(si.memmap_paddr, bytes));
    }
    if si.nr_modules != 0 {
        let bytes = si.nr_modules as u64 * core::mem::size_of::<HvmModlistEntry>() as u64;
        reserve("module list", Range::from_len(si.modlist_paddr, bytes));
        for i in 0..si.nr_modules as usize {
            // SAFETY: `nr_modules` entries at `modlist_paddr`, per the PVH
            // ABI; alignment is not guaranteed.
            let m = unsafe {
                (si.modlist_paddr as *const HvmModlistEntry)
                    .add(i)
                    .read_unaligned()
            };
            kprintln!(
                "module {}: [{:#x}-{:#x})",
                i,
                m.paddr,
                m.paddr.saturating_add(m.size)
            );
            reserve("module", Range::from_len(m.paddr, m.size));
            reserve("module command line", cstr_range(m.cmdline_paddr));
        }
    }

    kprintln!(
        "memory: {} MiB RAM, kernel image {} KiB at {:#x}, {} frames free below 4 GiB",
        ram_bytes >> 20,
        image.len() >> 10,
        image.start,
        frames.free_frames()
    );

    self_test(frames);
}

/// Range of a NUL-terminated boot string, or empty for a null pointer.
fn cstr_range(paddr: u64) -> Range {
    if paddr == 0 {
        return Range::EMPTY;
    }
    // SAFETY: non-null boot strings are NUL-terminated per the PVH ABI.
    let len = unsafe { CStr::from_ptr(paddr as *const core::ffi::c_char) }
        .to_bytes_with_nul()
        .len();
    Range::from_len(paddr, len as u64)
}

/// Allocates a few frames and checks they are distinct, aligned and really
/// backed by writable memory.
fn self_test(frames: &mut FrameAllocator<MAX_REGIONS>) {
    let mut got = [0u64; 4];
    for i in 0..got.len() {
        let frame = frames.alloc().expect("out of memory in frame self-test");
        assert_eq!(frame % PAGE_SIZE, 0);
        assert!(got[..i].iter().all(|&f| f != frame));
        let pattern = 0x5ca1_ab1e_0000_0000u64 | i as u64;
        let p = frame as *mut u64;
        // SAFETY: the frame was just allocated from RAM below 4 GiB, which is
        // identity-mapped and owned by nobody else.
        unsafe {
            p.write_volatile(pattern);
            assert_eq!(
                p.read_volatile(),
                pattern,
                "frame {:#x} is not backed by RAM",
                frame
            );
        }
        got[i] = frame;
    }
    kprintln!("frames: allocator ok (first frame {:#x})", got[0]);
}
