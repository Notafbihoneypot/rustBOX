//! Xen CPUID leaves (`xen/include/public/arch-x86/cpuid.h`).

/// First leaf at which a hypervisor signature may appear.
pub const HYPERVISOR_LEAF_BASE: u32 = 0x4000_0000;
/// Xen may be shifted up in steps of this when it emulates Viridian.
pub const HYPERVISOR_LEAF_STEP: u32 = 0x100;
/// Last base worth probing.
pub const HYPERVISOR_LEAF_LIMIT: u32 = 0x4001_0000;

/// "XenVMMXenVMM" as returned in ebx, ecx, edx of the signature leaf.
pub const XEN_SIGNATURE: [u32; 3] = [0x566e_6558, 0x65584d4d, 0x4d4d_566e];

/// Offset (from the Xen base leaf) of the leaf reporting `major << 16 | minor`
/// in eax.
pub const LEAF_VERSION: u32 = 1;
/// Offset of the leaf reporting the hypercall page count (eax) and the MSR
/// used to install hypercall pages (ebx).
pub const LEAF_HYPERCALL: u32 = 2;

/// True if the (ebx, ecx, edx) triple of a signature leaf says "Xen".
pub const fn is_xen_signature(ebx: u32, ecx: u32, edx: u32) -> bool {
    ebx == XEN_SIGNATURE[0] && ecx == XEN_SIGNATURE[1] && edx == XEN_SIGNATURE[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_spells_xenvmm() {
        let mut bytes = [0u8; 12];
        for (i, word) in XEN_SIGNATURE.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        assert_eq!(&bytes, b"XenVMMXenVMM");
        assert!(is_xen_signature(
            XEN_SIGNATURE[0],
            XEN_SIGNATURE[1],
            XEN_SIGNATURE[2]
        ));
        assert!(!is_xen_signature(0, 0, 0));
    }
}
