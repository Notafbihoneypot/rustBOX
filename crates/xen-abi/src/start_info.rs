//! PVH boot information (`xen/include/public/arch-x86/hvm/start_info.h`).
//!
//! On entry to a PVH kernel `ebx` holds the physical address of an
//! [`HvmStartInfo`]. All pointers inside it are physical addresses.

/// Value of [`HvmStartInfo::magic`] ("xEn3" with the 0x80 bit of 'E' set).
pub const HVM_START_MAGIC: u32 = 0x336e_c578;

/// `flags`: the domain is privileged (may issue control hypercalls).
pub const SIF_PRIVILEGED: u32 = 1 << 0;
/// `flags`: the domain is the initial control domain (dom0).
pub const SIF_INITDOMAIN: u32 = 1 << 1;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct HvmStartInfo {
    pub magic: u32,
    /// 0: no memory map. 1: `memmap_*` fields are valid.
    pub version: u32,
    pub flags: u32,
    pub nr_modules: u32,
    pub modlist_paddr: u64,
    pub cmdline_paddr: u64,
    pub rsdp_paddr: u64,
    pub memmap_paddr: u64,
    pub memmap_entries: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct HvmModlistEntry {
    pub paddr: u64,
    pub size: u64,
    pub cmdline_paddr: u64,
    pub reserved: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct HvmMemmapEntry {
    pub addr: u64,
    pub size: u64,
    /// One of the `MEMMAP_TYPE_*` constants (E820 numbering).
    pub kind: u32,
    pub reserved: u32,
}

pub const MEMMAP_TYPE_RAM: u32 = 1;
pub const MEMMAP_TYPE_RESERVED: u32 = 2;
pub const MEMMAP_TYPE_ACPI: u32 = 3;
pub const MEMMAP_TYPE_NVS: u32 = 4;
pub const MEMMAP_TYPE_UNUSABLE: u32 = 5;
pub const MEMMAP_TYPE_DISABLED: u32 = 6;
pub const MEMMAP_TYPE_PMEM: u32 = 7;

/// Human-readable name of a memory map entry type.
pub const fn memmap_type_name(kind: u32) -> &'static str {
    match kind {
        MEMMAP_TYPE_RAM => "ram",
        MEMMAP_TYPE_RESERVED => "reserved",
        MEMMAP_TYPE_ACPI => "acpi",
        MEMMAP_TYPE_NVS => "acpi-nvs",
        MEMMAP_TYPE_UNUSABLE => "unusable",
        MEMMAP_TYPE_DISABLED => "disabled",
        MEMMAP_TYPE_PMEM => "pmem",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};

    // These layouts are ABI: Xen and QEMU write them, we only read.
    #[test]
    fn layouts_match_the_public_headers() {
        assert_eq!(size_of::<HvmStartInfo>(), 56);
        assert_eq!(offset_of!(HvmStartInfo, modlist_paddr), 16);
        assert_eq!(offset_of!(HvmStartInfo, memmap_paddr), 40);
        assert_eq!(offset_of!(HvmStartInfo, memmap_entries), 48);
        assert_eq!(size_of::<HvmModlistEntry>(), 32);
        assert_eq!(size_of::<HvmMemmapEntry>(), 24);
        assert_eq!(offset_of!(HvmMemmapEntry, kind), 16);
    }
}
