//! ELF notes Xen's domain builder looks for in a guest kernel image.

/// Owner name of every Xen ELF note (NUL-terminated in the image).
pub const NOTE_NAME: &[u8; 4] = b"Xen\0";

/// 32-bit physical entry point of a PVH kernel. This is the only note a PVH
/// image needs: Xen (and QEMU's `-kernel` PVH loader) jump here in 32-bit
/// protected mode with paging off and `ebx` pointing at `HvmStartInfo`.
pub const XEN_ELFNOTE_PHYS32_ENTRY: u32 = 18;
