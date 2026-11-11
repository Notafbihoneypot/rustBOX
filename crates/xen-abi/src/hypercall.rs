//! Hypercall numbers and sub-operations (`xen/include/public/xen.h` et al.).

/// Each hypercall owns a 32-byte stub inside the hypercall page.
pub const HYPERCALL_STUB_SIZE: usize = 32;
pub const HYPERCALL_PAGE_SIZE: usize = 4096;

pub const HYPERVISOR_MEMORY_OP: usize = 12;
pub const HYPERVISOR_XEN_VERSION: usize = 17;
pub const HYPERVISOR_CONSOLE_IO: usize = 18;
pub const HYPERVISOR_SCHED_OP: usize = 29;

/// `HYPERVISOR_console_io` commands.
pub const CONSOLEIO_WRITE: usize = 0;
pub const CONSOLEIO_READ: usize = 1;

/// `HYPERVISOR_xen_version` commands.
pub const XENVER_VERSION: usize = 0;

/// `HYPERVISOR_sched_op` commands.
pub const SCHEDOP_YIELD: usize = 0;
pub const SCHEDOP_SHUTDOWN: usize = 2;

/// Argument of `SCHEDOP_SHUTDOWN`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SchedShutdown {
    pub reason: u32,
}

pub const SHUTDOWN_POWEROFF: u32 = 0;
pub const SHUTDOWN_REBOOT: u32 = 1;
pub const SHUTDOWN_CRASH: u32 = 3;

/// `HYPERVISOR_memory_op` commands.
pub const XENMEM_MEMORY_MAP: usize = 9;

/// Argument of `XENMEM_MEMORY_MAP`: in, capacity of `buffer`; out, entries
/// written.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct XenMemoryMap {
    pub nr_entries: u32,
    pub buffer: *mut E820Entry,
}

/// Entry written by `XENMEM_MEMORY_MAP`. Packed, as in the BIOS E820 call.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug)]
pub struct E820Entry {
    pub addr: u64,
    pub size: u64,
    pub kind: u32,
}

/// Splits the value returned by `XENVER_VERSION` into (major, minor).
pub const fn split_version(v: u32) -> (u16, u16) {
    ((v >> 16) as u16, v as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_match_the_public_headers() {
        assert_eq!(core::mem::size_of::<E820Entry>(), 20);
        assert_eq!(core::mem::size_of::<XenMemoryMap>(), 16);
        assert_eq!(core::mem::offset_of!(XenMemoryMap, buffer), 8);
    }

    #[test]
    fn version_split() {
        assert_eq!(split_version(0x0004_0011), (4, 17));
    }
}
