//! The instruction-set list of the CPU tab.

use super::cpuid::{bit, Leaves};
use crate::model::{CpuVendor, CpuidLeaf};

/// XCR0 bits the OS must enable before AVX code may run: SSE and AVX state.
const XCR0_AVX: u64 = 0b110;
/// Additionally opmask, ZMM0-15 upper halves and ZMM16-31 for AVX-512.
const XCR0_AVX512: u64 = XCR0_AVX | 0b1110_0000;
/// AMX tile configuration and tile data.
const XCR0_AMX: u64 = 0b11 << 17;

/// The registers of leaf 7 that announce AVX-512 sub-features.
#[derive(Clone, Copy)]
enum Leaf7 {
    Ebx,
    Ecx,
    Edx,
    /// `EAX` of sub-leaf 1.
    Sub1Eax,
}

/// AVX-512 sub-features in display order, with the register and bit announcing each.
const AVX512_SUBSETS: [(&str, Leaf7, u32); 17] = [
    ("DQ", Leaf7::Ebx, 17),
    ("BW", Leaf7::Ebx, 30),
    ("VL", Leaf7::Ebx, 31),
    ("CD", Leaf7::Ebx, 28),
    ("ER", Leaf7::Ebx, 27),
    ("PF", Leaf7::Ebx, 26),
    ("IFMA", Leaf7::Ebx, 21),
    ("VBMI", Leaf7::Ecx, 1),
    ("VBMI2", Leaf7::Ecx, 6),
    ("VNNI", Leaf7::Ecx, 11),
    ("BITALG", Leaf7::Ecx, 12),
    ("VPOPCNTDQ", Leaf7::Ecx, 14),
    ("4VNNIW", Leaf7::Edx, 2),
    ("4FMAPS", Leaf7::Edx, 3),
    ("VP2INTERSECT", Leaf7::Edx, 8),
    ("BF16", Leaf7::Sub1Eax, 5),
    ("FP16", Leaf7::Edx, 23),
];

/// Builds the instruction-set list in display order. `xcr0` is the OS-enabled
/// state mask (`None` when XSAVE is off); extensions whose register state the
/// OS does not save are left out because programs cannot use them.
pub fn instructions(vendor: CpuVendor, leaves: Leaves, xcr0: Option<u64>) -> Vec<String> {
    let (l1, l7, l7s1) = (leaves.regs(1, 0), leaves.regs(7, 0), leaves.regs(7, 1));
    let ext = leaves.regs(0x8000_0001, 0);
    let intel = vendor == CpuVendor::Intel;
    // The AMD-defined bits of the extended leaf mean something else (or nothing) on Intel parts.
    let amd_bit = |register: u32, n: u32| !intel && bit(register, n);
    let saved = |mask: u64| xcr0.is_some_and(|x| x & mask == mask);
    let (avx, avx512, amx) = (saved(XCR0_AVX), saved(XCR0_AVX512), saved(XCR0_AMX));
    let tsx = match (bit(l7.ebx, 4), bit(l7.ebx, 11)) {
        (true, true) => Some("TSX"),
        (false, true) => Some("RTM"),
        (true, false) => Some("HLE"),
        (false, false) => None,
    };

    let mut list: Vec<String> = Vec::new();
    let mut add = |present: bool, name: &str| {
        if present {
            list.push(name.to_string());
        }
    };
    add(bit(l1.edx, 23), if amd_bit(ext.edx, 22) { "MMX (+)" } else { "MMX" });
    add(amd_bit(ext.edx, 31), if amd_bit(ext.edx, 30) { "3DNow! (+)" } else { "3DNow!" });
    add(bit(l1.edx, 25), "SSE");
    add(bit(l1.edx, 26), "SSE2");
    add(bit(l1.ecx, 0), "SSE3");
    add(bit(l1.ecx, 9), "SSSE3");
    add(bit(l1.ecx, 19), "SSE4.1");
    add(bit(l1.ecx, 20), "SSE4.2");
    add(amd_bit(ext.ecx, 6), "SSE4A");
    add(bit(ext.edx, 29), if intel { "EM64T" } else { "x86-64" });
    // Hyper-V hides VMX and SVM from every partition, the host included; on the host they
    // are there all the same, enabled and in use by the hypervisor.
    let host = hyper_v_root(leaves);
    let amd_family = matches!(vendor, CpuVendor::Amd | CpuVendor::Hygon);
    add(intel && (bit(l1.ecx, 5) || host), "VT-x");
    add(amd_bit(ext.ecx, 2) || amd_family && host, "AMD-V");
    add(bit(l1.ecx, 25), "AES");
    add(avx && bit(l1.ecx, 28), "AVX");
    add(avx && bit(l7.ebx, 5), "AVX2");
    add(avx && bit(l7s1.eax, 4), "AVX-VNNI");
    add(avx512 && bit(l7.ebx, 16), &avx512_entry(&l7, &l7s1));
    add(avx512 && bit(l7s1.edx, 19), "AVX10");
    add(avx && amd_bit(ext.ecx, 11), "XOP");
    add(avx && bit(l1.ecx, 12), "FMA3");
    add(avx && amd_bit(ext.ecx, 16), "FMA4");
    add(tsx.is_some(), tsx.unwrap_or_default());
    add(bit(l7.ebx, 29), "SHA");
    add(amx && bit(l7.edx, 24), "AMX");
    list
}

/// Whether the code runs in the root partition of Hyper-V: the hypervisor identifies
/// itself as Microsoft's and grants the CreatePartitions privilege, which only the root
/// partition holds. A guest never has it.
fn hyper_v_root(leaves: Leaves) -> bool {
    let vendor = leaves.regs(0x4000_0000, 0);
    let microsoft = (vendor.ebx, vendor.ecx, vendor.edx) == (0x7263_694D, 0x666F_736F, 0x7648_2074);
    microsoft && bit(leaves.regs(0x4000_0003, 0).ebx, 0)
}

/// `AVX512 (DQ, BW, ...)`: the foundation is implied, the sub-features are listed.
fn avx512_entry(l7: &CpuidLeaf, l7s1: &CpuidLeaf) -> String {
    let register = |source| match source {
        Leaf7::Ebx => l7.ebx,
        Leaf7::Ecx => l7.ecx,
        Leaf7::Edx => l7.edx,
        Leaf7::Sub1Eax => l7s1.eax,
    };
    let subsets: Vec<&str> = AVX512_SUBSETS
        .iter()
        .filter(|(_, source, n)| bit(register(*source), *n))
        .map(|(name, ..)| *name)
        .collect();
    if subsets.is_empty() {
        "AVX512".to_string()
    } else {
        format!("AVX512 ({})", subsets.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::super::cpuid::fixtures::{hybrid, leaf, zen4};
    use super::*;

    const ALL_STATE: Option<u64> = Some(0x0006_00E7);

    fn joined(vendor: CpuVendor, table: &[CpuidLeaf], xcr0: Option<u64>) -> String {
        instructions(vendor, Leaves(table), xcr0).join(", ")
    }

    #[test]
    fn zen4_under_a_hypervisor() {
        // The hypervisor hides AMD-V, and without its own leaves nothing says the processor has it.
        assert_eq!(
            joined(CpuVendor::Amd, &zen4(0), Some(0xE7)),
            "MMX (+), SSE, SSE2, SSE3, SSSE3, SSE4.1, SSE4.2, SSE4A, x86-64, AES, AVX, AVX2, \
             AVX512 (DQ, BW, VL, CD, IFMA, VBMI, VBMI2, VNNI, BITALG, VPOPCNTDQ, BF16), FMA3, SHA"
        );
    }

    #[test]
    fn hybrid_intel() {
        assert_eq!(
            joined(CpuVendor::Intel, &hybrid(0, true), Some(0x7)),
            "MMX, SSE, SSE2, SSE3, SSSE3, SSE4.1, SSE4.2, EM64T, VT-x, AES, AVX, AVX2, AVX-VNNI, FMA3, SHA"
        );
    }

    /// `table` as Hyper-V shows it to a partition: VMX and SVM hidden, the hypervisor's
    /// vendor leaf, and the partition privileges with CreatePartitions as given.
    fn under_hyper_v(mut table: Vec<CpuidLeaf>, root: bool) -> Vec<CpuidLeaf> {
        for l1 in table.iter_mut().filter(|l| l.leaf == 1) {
            l1.ecx &= !(1 << 5);
        }
        for ext in table.iter_mut().filter(|l| l.leaf == 0x8000_0001) {
            ext.ecx &= !(1 << 2);
        }
        table.push(leaf(0x4000_0000, 0, 0x4000_000C, 0x7263_694D, 0x666F_736F, 0x7648_2074));
        table.push(leaf(0x4000_0003, 0, 0x0000_3FFF, if root { 0x002B_B9FF } else { 0x002B_B9FE }, 0, 0));
        table
    }

    #[test]
    fn the_hyper_v_root_partition_keeps_vt_x() {
        // Hyper-V uses VMX itself and hides it from CPUID; the root partition is the host.
        let list = instructions(CpuVendor::Intel, Leaves(&under_hyper_v(hybrid(0, true), true)), Some(0x7));
        assert!(list.iter().any(|name| name == "VT-x"), "{list:?}");
        let list = instructions(CpuVendor::Amd, Leaves(&under_hyper_v(zen4(0), true)), Some(0xE7));
        assert!(list.iter().any(|name| name == "AMD-V"), "{list:?}");
    }

    #[test]
    fn a_guest_partition_lists_no_virtualization() {
        let list = instructions(CpuVendor::Intel, Leaves(&under_hyper_v(hybrid(0, true), false)), Some(0x7));
        assert!(!list.iter().any(|name| name == "VT-x"), "{list:?}");
        let list = instructions(CpuVendor::Amd, Leaves(&under_hyper_v(zen4(0), false)), Some(0xE7));
        assert!(!list.iter().any(|name| name == "AMD-V"), "{list:?}");
    }

    #[test]
    fn os_support_gates_the_vector_extensions() {
        let table = zen4(0);
        // AVX state saved, AVX-512 state not: only the 256-bit extensions remain.
        let avx_only = joined(CpuVendor::Amd, &table, Some(0x7));
        assert!(avx_only.contains("AVX2") && avx_only.contains("FMA3"));
        assert!(!avx_only.contains("AVX512"));
        // XSAVE off altogether: nothing that needs YMM state.
        for xcr0 in [None, Some(0x3)] {
            let sse_only = joined(CpuVendor::Amd, &table, xcr0);
            assert!(!sse_only.contains("AVX") && !sse_only.contains("FMA"), "{sse_only}");
            assert!(sse_only.contains("SSE4.2") && sse_only.contains("AES") && sse_only.contains("SHA"));
        }
    }

    #[test]
    fn bulldozer_era_amd() {
        let table = [
            leaf(1, 0, 0x0060_0F20, 0, 0x3E98_320B, 0x178B_FBFF),
            leaf(0x8000_0001, 0, 0x0060_0F20, 0, 0x01EB_BFFF, 0x2FD3_FBFF),
        ];
        assert_eq!(
            joined(CpuVendor::Amd, &table, Some(0x7)),
            "MMX (+), SSE, SSE2, SSE3, SSSE3, SSE4.1, SSE4.2, SSE4A, x86-64, AMD-V, AES, AVX, XOP, FMA3, FMA4"
        );
    }

    #[test]
    fn k8_has_3dnow() {
        let table = [
            leaf(1, 0, 0x0002_0FF2, 0, 0x0000_0001, 0x078B_FBFF),
            leaf(0x8000_0001, 0, 0x0002_0FF2, 0, 0x0000_0001, 0xE3D3_FBFF),
        ];
        assert_eq!(joined(CpuVendor::Amd, &table, None), "MMX (+), 3DNow! (+), SSE, SSE2, SSE3, x86-64");
    }

    #[test]
    fn server_intel_with_avx512_and_amx() {
        let table = [
            leaf(1, 0, 0x0008_06F8, 0, 0x7FFE_FBFF, 0xBFEB_FBFF),
            leaf(7, 0, 1, 0xF1BF_2FFB, 0x0000_5F42, 0x0380_0000),
            leaf(7, 1, 0x0000_0030, 0, 0, 0),
            leaf(0x8000_0001, 0, 0, 0, 0x0000_0121, 0x2C10_0800),
        ];
        assert_eq!(
            joined(CpuVendor::Intel, &table, ALL_STATE),
            "MMX, SSE, SSE2, SSE3, SSSE3, SSE4.1, SSE4.2, EM64T, VT-x, AES, AVX, AVX2, AVX-VNNI, \
             AVX512 (DQ, BW, VL, CD, IFMA, VBMI, VBMI2, VNNI, BITALG, VPOPCNTDQ, BF16, FP16), \
             FMA3, TSX, SHA, AMX"
        );
        // Without the tile state the processor still has AMX, but nothing can run it.
        assert!(!joined(CpuVendor::Intel, &table, Some(0xE7)).contains("AMX"));
    }

    #[test]
    fn foundation_only_avx512_and_partial_tsx() {
        let table = [leaf(1, 0, 0, 0, 0x1800_0000, 0), leaf(7, 0, 0, 0x0001_0800, 0, 0)];
        assert_eq!(joined(CpuVendor::Intel, &table, Some(0xE7)), "AVX, AVX512, RTM");
        let table = [leaf(7, 0, 0, 0x0000_0010, 0, 0)];
        assert_eq!(joined(CpuVendor::Intel, &table, None), "HLE");
    }

    #[test]
    fn amd_only_bits_are_ignored_on_intel() {
        // Intel leaves the AMD-defined extended bits reserved; a stray one must not surface.
        let table = [leaf(1, 0, 0, 0, 0, 0x0080_0000), leaf(0x8000_0001, 0, 0, 0, 0x0001_0844, 0xC040_0000)];
        assert_eq!(joined(CpuVendor::Intel, &table, ALL_STATE), "MMX");
        assert_eq!(joined(CpuVendor::Amd, &[], ALL_STATE), "");
    }
}
