//! Raw CPUID access: reading leaves, the bounded enumeration behind the thread
//! dumps, and decoding of the vendor, brand and signature leaves.

#[cfg(target_arch = "x86")]
use core::arch::x86::{__cpuid_count, _xgetbv};
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::{__cpuid_count, _xgetbv};

use crate::model::{CpuVendor, CpuidLeaf};

/// Highest leaf offset walked inside each CPUID range (standard, hypervisor, extended, Centaur).
const MAX_LEAVES_PER_RANGE: u32 = 0xFF;
/// Upper bound for sub-leaves that are counted or walked until a terminator.
const MAX_SUBLEAVES: u32 = 32;

const HYPERVISOR_BASE: u32 = 0x4000_0000;
const EXTENDED_BASE: u32 = 0x8000_0000;
const CENTAUR_BASE: u32 = 0xC000_0000;

/// `true` when bit `n` of `value` is set.
pub fn bit(value: u32, n: u32) -> bool {
    value >> n & 1 != 0
}

/// Bits `hi..=lo` of `value`, shifted down.
pub fn bits(value: u32, hi: u32, lo: u32) -> u32 {
    (value >> lo) & (u32::MAX >> (31 - (hi - lo)))
}

/// Executes CPUID on whichever processor the calling thread is running on.
// The intrinsic is an `unsafe fn` on the crate's minimum toolchain and a safe one on current ones.
#[allow(unused_unsafe)]
pub fn read(leaf: u32, subleaf: u32) -> CpuidLeaf {
    // SAFETY: CPUID is unprivileged, has no side effects and exists on every processor this crate targets.
    let r = unsafe { __cpuid_count(leaf, subleaf) };
    CpuidLeaf { leaf, subleaf, eax: r.eax, ebx: r.ebx, ecx: r.ecx, edx: r.edx }
}

/// Executes CPUID for sub-leaf 0 of `leaf` on the calling thread's processor.
/// `None` when the processor does not implement the leaf, that is when it lies
/// beyond the highest leaf of its range.
pub fn read_supported(leaf: u32) -> Option<CpuidLeaf> {
    let highest = read(leaf & EXTENDED_BASE, 0).eax;
    (leaf <= highest).then(|| read(leaf, 0))
}

/// Reads XCR0, the mask of register states the OS saves on a context switch.
/// `None` when the OS has not enabled XSAVE, in which case no AVX-class state is usable.
pub fn xcr0() -> Option<u64> {
    if !bit(read(1, 0).ecx, 27) {
        return None;
    }
    // SAFETY: CPUID.1:ECX.OSXSAVE is set, so XGETBV is enabled in user mode and XCR0 exists.
    Some(unsafe { _xgetbv(0) })
}

/// Read-only view over the leaves gathered on one logical processor.
#[derive(Debug, Clone, Copy)]
pub struct Leaves<'a>(pub &'a [CpuidLeaf]);

impl<'a> Leaves<'a> {
    pub fn get(self, leaf: u32, subleaf: u32) -> Option<&'a CpuidLeaf> {
        self.0.iter().find(|l| l.leaf == leaf && l.subleaf == subleaf)
    }

    /// The registers of a leaf, all zero when the processor does not implement it.
    pub fn regs(self, leaf: u32, subleaf: u32) -> CpuidLeaf {
        self.get(leaf, subleaf).copied().unwrap_or(CpuidLeaf { leaf, subleaf, ..Default::default() })
    }

    pub fn subleaves(self, leaf: u32) -> impl Iterator<Item = &'a CpuidLeaf> {
        self.0.iter().filter(move |l| l.leaf == leaf)
    }

    /// Highest standard leaf (`CPUID.0:EAX`).
    pub fn max_basic(self) -> u32 {
        self.regs(0, 0).eax
    }

    /// Highest extended leaf (`CPUID.80000000h:EAX`), `0` when there are none.
    pub fn max_extended(self) -> u32 {
        self.regs(EXTENDED_BASE, 0).eax
    }
}

/// Gathers every implemented leaf through `read`: the standard range, the
/// hypervisor range when a hypervisor is present, the extended range and the
/// Centaur range. Each range and each sub-leaf walk is capped, so the result
/// stays small whatever the processor answers.
pub fn enumerate(read: &mut dyn FnMut(u32, u32) -> CpuidLeaf) -> Vec<CpuidLeaf> {
    let mut out = Vec::with_capacity(128);

    let base = read(0, 0);
    for leaf in 0..=base.eax.min(MAX_LEAVES_PER_RANGE) {
        read_leaf(read, leaf, &mut out);
    }

    let hypervisor_present = Leaves(&out).get(1, 0).is_some_and(|l| bit(l.ecx, 31));
    if hypervisor_present {
        read_range(read, HYPERVISOR_BASE, true, &mut out);
    }
    read_range(read, EXTENDED_BASE, false, &mut out);
    if matches!(vendor(&vendor_string(Leaves(&out))), CpuVendor::Via | CpuVendor::Zhaoxin) {
        read_range(read, CENTAUR_BASE, false, &mut out);
    }
    out
}

/// Walks one of the ranges whose first leaf reports the highest leaf in `EAX`.
/// A processor without the range answers with something outside of it. The
/// hypervisor range is announced by a feature bit, so its first leaf is kept
/// even then (`always_first`).
fn read_range(
    read: &mut dyn FnMut(u32, u32) -> CpuidLeaf,
    base: u32,
    always_first: bool,
    out: &mut Vec<CpuidLeaf>,
) {
    let max = read(base, 0).eax;
    let valid = (base..=base + MAX_LEAVES_PER_RANGE).contains(&max);
    if !valid && !always_first {
        return;
    }
    for leaf in base..=if valid { max } else { base } {
        read_leaf(read, leaf, out);
    }
}

/// Appends sub-leaf 0 of `leaf` and every further sub-leaf the leaf defines.
fn read_leaf(read: &mut dyn FnMut(u32, u32) -> CpuidLeaf, leaf: u32, out: &mut Vec<CpuidLeaf>) {
    let first = read(leaf, 0);
    out.push(first);
    match leaf {
        // Cache parameters: one sub-leaf per cache until the type field reads "no more caches".
        0x04 | 0x8000_001D => read_while(read, &first, 1, |l| l.eax & 0x1F != 0, out),
        // Topology levels until the level type reads "invalid".
        0x0B | 0x1F | 0x8000_0026 => read_while(read, &first, 1, |l| l.ecx & 0xFF00 != 0, out),
        // PCONFIG targets until the sub-leaf type reads "invalid".
        0x1B => read_while(read, &first, 1, |l| l.eax & 0xFFF != 0, out),
        // Sub-leaf 0 reports the highest sub-leaf in EAX.
        0x07 | 0x14 | 0x17 | 0x18 | 0x1D | 0x20 | 0x24 => {
            (1..=first.eax.min(MAX_SUBLEAVES)).for_each(|n| out.push(read(leaf, n)));
        }
        // XSAVE: sub-leaf 1, then one sub-leaf per state component in XCR0 or IA32_XSS.
        0x0D => {
            let extended = read(leaf, 1);
            out.push(extended);
            let user = u64::from(first.eax) | u64::from(first.edx) << 32;
            let supervisor = u64::from(extended.ecx) | u64::from(extended.edx) << 32;
            read_set(read, leaf, (user | supervisor) & !0b11, out);
        }
        // Resource monitoring / allocation and PMU extensions: a bitmap names the valid sub-leaves.
        0x0F => read_set(read, leaf, u64::from(first.edx & !1), out),
        0x10 | 0x8000_0020 => read_set(read, leaf, u64::from(first.ebx & !1), out),
        0x23 => read_set(read, leaf, u64::from(first.eax & !1), out),
        // SGX: capabilities, attributes, then EPC sections until an invalid one.
        0x12 if first.eax & 0b11 != 0 => {
            out.push(read(leaf, 1));
            let section = read(leaf, 2);
            if section.eax & 0xF != 0 {
                out.push(section);
                read_while(read, &section, 3, |l| l.eax & 0xF != 0, out);
            }
        }
        _ => {}
    }
}

/// Appends sub-leaves `from..` for as long as they are `valid`, provided `previous` is.
fn read_while(
    read: &mut dyn FnMut(u32, u32) -> CpuidLeaf,
    previous: &CpuidLeaf,
    from: u32,
    valid: fn(&CpuidLeaf) -> bool,
    out: &mut Vec<CpuidLeaf>,
) {
    if !valid(previous) {
        return;
    }
    for subleaf in from..MAX_SUBLEAVES {
        let next = read(previous.leaf, subleaf);
        if !valid(&next) {
            break;
        }
        out.push(next);
    }
}

/// Appends the sub-leaf of every bit set in `set`.
fn read_set(read: &mut dyn FnMut(u32, u32) -> CpuidLeaf, leaf: u32, set: u64, out: &mut Vec<CpuidLeaf>) {
    (0..u64::BITS).filter(|n| set >> n & 1 != 0).for_each(|n| out.push(read(leaf, n)));
}

/// The 12-character vendor string of leaf 0 (`EBX`, `EDX`, `ECX`).
pub fn vendor_string(leaves: Leaves) -> String {
    let l = leaves.regs(0, 0);
    ascii(&[l.ebx, l.edx, l.ecx])
}

pub fn vendor(vendor_string: &str) -> CpuVendor {
    match vendor_string {
        "GenuineIntel" => CpuVendor::Intel,
        "AuthenticAMD" => CpuVendor::Amd,
        "HygonGenuine" => CpuVendor::Hygon,
        "  Shanghai  " => CpuVendor::Zhaoxin,
        "CentaurHauls" | "VIA VIA VIA " => CpuVendor::Via,
        _ => CpuVendor::Other,
    }
}

/// The brand string of leaves `80000002h..=80000004h` with runs of whitespace collapsed.
/// Empty when the processor has no brand string.
pub fn brand_string(leaves: Leaves) -> String {
    let words: Vec<u32> = (EXTENDED_BASE + 2..=EXTENDED_BASE + 4)
        .filter_map(|leaf| leaves.get(leaf, 0))
        .flat_map(|l| [l.eax, l.ebx, l.ecx, l.edx])
        .collect();
    ascii(&words).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Text stored little-endian in register values, up to the first NUL. Non-printable bytes are dropped.
fn ascii(words: &[u32]) -> String {
    words
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .take_while(|&b| b != 0)
        .filter(|b| b.is_ascii_graphic() || *b == b' ')
        .map(char::from)
        .collect()
}

/// Family, model and stepping as the CPU tab shows them: the raw nibbles of
/// `CPUID.1:EAX` plus the effective ("extended") family and model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Signature {
    pub family: u8,
    pub model: u8,
    pub stepping: u8,
    pub ext_family: u16,
    pub ext_model: u16,
}

pub fn signature(eax: u32) -> Signature {
    let family = bits(eax, 11, 8);
    let model = bits(eax, 7, 4);
    // The extended fields only count for the families that ran out of nibble space.
    let ext_family = if family == 0xF { family + bits(eax, 27, 20) } else { family };
    let ext_model = if family >= 6 { bits(eax, 19, 16) << 4 | model } else { model };
    Signature {
        family: family as u8,
        model: model as u8,
        stepping: bits(eax, 3, 0) as u8,
        ext_family: ext_family as u16,
        ext_model: ext_model as u16,
    }
}

#[cfg(test)]
pub mod fixtures {
    //! CPUID answers of a few published processors, shared by the tests of the `cpu` modules.

    use super::*;

    pub const fn leaf(leaf: u32, subleaf: u32, eax: u32, ebx: u32, ecx: u32, edx: u32) -> CpuidLeaf {
        CpuidLeaf { leaf, subleaf, eax, ebx, ecx, edx }
    }

    /// Packs a brand string into the three brand leaves.
    pub fn brand_leaves(brand: &str) -> Vec<CpuidLeaf> {
        let mut bytes = brand.as_bytes().to_vec();
        bytes.resize(48, 0);
        let word = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
        (0..3)
            .map(|n| {
                let at = n * 16;
                leaf(EXTENDED_BASE + 2 + n as u32, 0, word(at), word(at + 4), word(at + 8), word(at + 12))
            })
            .collect()
    }

    /// An 8-core / 16-thread Zen 4 desktop part running under a hypervisor,
    /// as seen from the thread with the given x2APIC id.
    pub fn zen4(apic_id: u32) -> Vec<CpuidLeaf> {
        let mut leaves = vec![
            leaf(0, 0, 0x0000_000D, 0x6874_7541, 0x444D_4163, 0x6974_6E65),
            leaf(1, 0, 0x00A6_0F12, apic_id << 24 | 0x0010_0800, 0xFED8_320B, 0x178B_FBFF),
            leaf(4, 0, 0, 0, 0, 0),
            leaf(7, 0, 0x0000_0001, 0xF1BF_97A9, 0x0040_5FC6, 0x0000_0010),
            leaf(7, 1, 0x0000_0020, 0, 0, 0),
            leaf(0xB, 0, 0x0000_0001, 0x0000_0002, 0x0000_0100, apic_id),
            leaf(0xB, 1, 0x0000_0004, 0x0000_0010, 0x0000_0201, apic_id),
            leaf(0xD, 0, 0x0000_00E7, 0x0000_0980, 0x0000_0980, 0),
            leaf(0xD, 1, 0x0000_000F, 0x0000_0990, 0x0000_1800, 0),
            leaf(0x8000_0000, 0, 0x8000_0027, 0x6874_7541, 0x444D_4163, 0x6974_6E65),
            leaf(0x8000_0001, 0, 0x00A6_0F12, 0, 0x20C0_23F3, 0x2FD3_FBFF),
        ];
        leaves.extend(brand_leaves("AMD Ryzen 7 7700 8-Core Processor            "));
        leaves.extend([
            leaf(0x8000_0005, 0, 0xFF48_FF40, 0xFF48_FF40, 0x2008_0140, 0x2008_0140),
            leaf(0x8000_0006, 0, 0x5C00_2200, 0x6C00_4200, 0x0400_6140, 0x0100_9140),
            leaf(0x8000_0008, 0, 0x0000_3030, 0x7912_D215, 0x0000_400F, 0x0001_0000),
            leaf(0x8000_001D, 0, 0x0000_4121, 0x01C0_003F, 0x0000_003F, 0),
            leaf(0x8000_001D, 1, 0x0000_4122, 0x01C0_003F, 0x0000_003F, 0),
            leaf(0x8000_001D, 2, 0x0000_4143, 0x01C0_003F, 0x0000_07FF, 2),
            leaf(0x8000_001D, 3, 0x0003_C163, 0x03C0_003F, 0x0000_7FFF, 1),
            leaf(0x8000_001E, 0, apic_id, 0x0000_0100 | apic_id >> 1, 0, 0),
            leaf(0x8000_0026, 0, 0x0000_0001, 0x0000_0002, 0x0000_0100, apic_id),
            leaf(0x8000_0026, 1, 0x0000_0004, 0x0000_0010, 0x0000_0201, apic_id),
        ]);
        leaves
    }

    /// One thread of a 16-core Zen 4 part built from two core complexes: the
    /// first (x2APIC ids 0-15) sits under a stacked 96 MB L3, the second keeps
    /// the plain 32 MB one. Everything else is the `zen4` thread.
    pub fn zen4_stacked(apic_id: u32) -> Vec<CpuidLeaf> {
        let mut leaves = zen4(apic_id);
        if apic_id < 16 {
            for l3 in leaves.iter_mut().filter(|l| (l.leaf, l.subleaf) == (0x8000_001D, 3)) {
                l3.ecx = 0x0001_7FFF;
            }
        }
        leaves
    }

    /// One thread of a hybrid desktop part with 8 P-cores (two threads each) and
    /// 16 E-cores in clusters of four. `p_core` selects which kind answers.
    pub fn hybrid(apic_id: u32, p_core: bool) -> Vec<CpuidLeaf> {
        let (l1d, l1i, l2) = if p_core {
            // 48 KB 12-way, 32 KB 8-way, 2 MB 16-way; two threads share each.
            (
                leaf(4, 0, 0xFC00_4121, 0x02C0_003F, 0x0000_003F, 0),
                leaf(4, 1, 0xFC00_4122, 0x01C0_003F, 0x0000_003F, 0),
                leaf(4, 2, 0xFC00_4143, 0x03C0_003F, 0x0000_07FF, 0),
            )
        } else {
            // 32 KB 8-way, 64 KB 8-way, one thread each; 4 MB 16-way shared by a four-core cluster.
            (
                leaf(4, 0, 0xFC00_0121, 0x01C0_003F, 0x0000_003F, 0),
                leaf(4, 1, 0xFC00_0122, 0x01C0_003F, 0x0000_007F, 0),
                leaf(4, 2, 0xFC01_C143, 0x03C0_003F, 0x0000_0FFF, 0),
            )
        };
        let mut leaves = vec![
            leaf(0, 0, 0x0000_0020, 0x756E_6547, 0x6C65_746E, 0x4965_6E69),
            leaf(1, 0, 0x000B_0671, apic_id << 24 | 0x0080_0800, 0x7FFA_FBFF, 0xBFEB_FBFF),
            l1d,
            l1i,
            l2,
            // 36 MB 12-way L3 shared by the whole package.
            leaf(4, 3, 0xFC1F_C163, 0x02C0_003F, 0x0000_BFFF, 4),
            leaf(7, 0, 0x0000_0002, 0x239C_27EB, 0x9840_0FAC, 0xFC18_C410),
            leaf(7, 1, 0x0040_0810, 0, 0, 0),
            leaf(0xB, 0, 0x0000_0001, if p_core { 2 } else { 1 }, 0x0000_0100, apic_id),
            leaf(0xB, 1, 0x0000_0007, 0x0000_0020, 0x0000_0201, apic_id),
            leaf(0x16, 0, 3000, 5800, 100, 0),
            leaf(0x1A, 0, if p_core { 0x4000_0001 } else { 0x2000_0001 }, 0, 0, 0),
            leaf(0x8000_0000, 0, 0x8000_0008, 0, 0, 0),
            leaf(0x8000_0001, 0, 0, 0, 0x0000_0121, 0x2C10_0800),
        ];
        leaves.extend(brand_leaves("13th Gen Intel(R) Core(TM) i9-13900K"));
        leaves
    }

    /// A reader backed by a fixed table; anything missing reads as zero.
    pub fn reader(table: Vec<CpuidLeaf>) -> impl FnMut(u32, u32) -> CpuidLeaf {
        move |l, s| Leaves(&table).regs(l, s)
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn extracts_bit_fields() {
        assert!(bit(0x8000_0000, 31));
        assert!(!bit(0x8000_0000, 30));
        assert_eq!(bits(0x00A6_0F12, 11, 8), 0xF);
        assert_eq!(bits(0xFFFF_FFFF, 31, 0), 0xFFFF_FFFF);
        assert_eq!(bits(0x0003_C163, 25, 14), 0xF);
    }

    #[test]
    fn decodes_signatures() {
        let zen4 = signature(0x00A6_0F12);
        assert_eq!((zen4.family, zen4.model, zen4.stepping), (0xF, 0x1, 0x2));
        assert_eq!((zen4.ext_family, zen4.ext_model), (0x19, 0x61));

        let raptor_lake = signature(0x000B_0671);
        assert_eq!((raptor_lake.family, raptor_lake.model, raptor_lake.stepping), (6, 7, 1));
        assert_eq!((raptor_lake.ext_family, raptor_lake.ext_model), (6, 0xB7));

        let k8 = signature(0x0002_0FF2);
        assert_eq!((k8.ext_family, k8.ext_model, k8.stepping), (0xF, 0x2F, 2));

        // Family 5 has no extended model, whatever the reserved bits hold.
        let p5 = signature(0x000F_0543);
        assert_eq!((p5.ext_family, p5.ext_model), (5, 4));
    }

    #[test]
    fn decodes_vendor_and_brand() {
        let table = zen4(0);
        let leaves = Leaves(&table);
        assert_eq!(vendor_string(leaves), "AuthenticAMD");
        assert_eq!(vendor("AuthenticAMD"), CpuVendor::Amd);
        assert_eq!(vendor("GenuineIntel"), CpuVendor::Intel);
        assert_eq!(vendor("HygonGenuine"), CpuVendor::Hygon);
        assert_eq!(vendor("  Shanghai  "), CpuVendor::Zhaoxin);
        assert_eq!(vendor("CentaurHauls"), CpuVendor::Via);
        assert_eq!(vendor("Microsoft Hv"), CpuVendor::Other);
        assert_eq!(brand_string(leaves), "AMD Ryzen 7 7700 8-Core Processor");
        assert_eq!(leaves.max_basic(), 0xD);
        assert_eq!(leaves.max_extended(), 0x8000_0027);
    }

    #[test]
    fn brand_string_is_normalised_or_empty() {
        let table = brand_leaves("  Intel(R)  Core(TM) i7 CPU     920  @ 2.67GHz");
        assert_eq!(brand_string(Leaves(&table)), "Intel(R) Core(TM) i7 CPU 920 @ 2.67GHz");
        assert_eq!(brand_string(Leaves(&[])), "");
        assert_eq!(vendor_string(Leaves(&[])), "");
    }

    #[test]
    fn enumerates_ranges_and_subleaves() {
        let mut table = zen4(3);
        table.extend([
            leaf(0x4000_0000, 0, 0x4000_0002, 0x7263_694D, 0x666F_736F, 0x7648_2074),
            leaf(0x4000_0001, 0, 0x3123_7648, 0, 0, 0),
        ]);
        let dump = enumerate(&mut reader(table));
        let leaves = Leaves(&dump);
        let count = |leaf| leaves.subleaves(leaf).count();

        // Standard range 0..=0xD, each leaf at least once and in ascending order.
        assert!((0..=0xD).all(|l| count(l) >= 1));
        assert_eq!(count(0xE), 0);
        assert!(dump.windows(2).all(|w| (w[0].leaf, w[0].subleaf) < (w[1].leaf, w[1].subleaf)));

        // Counted, terminated and bitmap-driven sub-leaf walks.
        assert_eq!(count(0x07), 2);
        assert_eq!(count(0x04), 1);
        assert_eq!(count(0x0B), 2);
        let xsave: Vec<u32> = leaves.subleaves(0x0D).map(|l| l.subleaf).collect();
        assert_eq!(xsave, [0, 1, 2, 5, 6, 7, 11, 12]);
        assert_eq!(count(0x8000_001D), 4);
        assert_eq!(count(0x8000_0026), 2);

        // The hypervisor bit is set in the fixture, so its range is walked too.
        assert_eq!(count(0x4000_0000), 1);
        assert_eq!(count(0x4000_0002), 1);
        assert_eq!(count(0x4000_0003), 0);
        assert_eq!(dump.last().map(|l| l.leaf), Some(0x8000_0027));
    }

    #[test]
    fn skips_absent_ranges() {
        // No hypervisor bit and an extended range that echoes the last standard leaf.
        let table =
            vec![leaf(0, 0, 1, 0x756E_6547, 0x6C65_746E, 0x4965_6E69), leaf(1, 0, 0x0000_0F29, 0, 0, 0)];
        let dump = enumerate(&mut reader(table));
        assert_eq!(dump.iter().map(|l| l.leaf).collect::<Vec<_>>(), [0, 1]);
    }

    #[test]
    fn walks_the_centaur_range_for_via() {
        let table = vec![
            leaf(0, 0, 1, 0x746E_6543, 0x736C_7561, 0x4872_7561),
            leaf(0xC000_0000, 0, 0xC000_0001, 0, 0, 0),
            leaf(0xC000_0001, 0, 0, 0, 0, 0x0000_3FCC),
        ];
        let dump = enumerate(&mut reader(table));
        assert_eq!(dump.iter().map(|l| l.leaf).collect::<Vec<_>>(), [0, 1, 0xC000_0000, 0xC000_0001]);
    }

    #[test]
    fn stays_bounded_on_a_processor_that_answers_garbage() {
        // Every register of every leaf reads all-ones: every range claims the
        // largest possible size and no sub-leaf walk ever sees a terminator.
        let mut calls = 0u32;
        let dump = enumerate(&mut |l, s| {
            calls += 1;
            leaf(l, s, u32::MAX, u32::MAX, u32::MAX, u32::MAX)
        });
        assert!(calls < 20_000, "{calls} CPUID calls");
        assert!(dump.len() < 20_000);
        assert!(Leaves(&dump).subleaves(0x04).count() <= MAX_SUBLEAVES as usize);
        assert!(Leaves(&dump).subleaves(0x0D).count() <= 64);
    }

    #[test]
    fn stays_bounded_when_every_range_claims_its_maximum() {
        let dump = enumerate(&mut |l, s| match l {
            0 => leaf(l, s, 0xFFFF, 0x756E_6547, 0x6C65_746E, 0x4965_6E69),
            1 => leaf(l, s, 0, 0, 0x8000_0000, 0),
            0x4000_0000 => leaf(l, s, 0x4000_00FF, 0, 0, 0),
            0x8000_0000 => leaf(l, s, 0x8000_00FF, 0, 0, 0),
            _ => leaf(l, s, 0, 0, 0, 0),
        });
        // 0x100 leaves per range, plus the unconditional second XSAVE sub-leaf.
        assert_eq!(dump.len(), 3 * 0x100 + 1);
    }

    #[test]
    fn live_read_rejects_leaves_out_of_range() {
        assert_eq!(read_supported(0), Some(read(0, 0)));
        assert_eq!(read_supported(0x7FFF_FFFF), None);
        assert_eq!(read_supported(0xFFFF_FFFF), None);
    }

    #[test]
    fn live_cpuid_is_consistent() {
        let dump = enumerate(&mut read);
        let leaves = Leaves(&dump);
        assert_eq!(leaves.get(0, 0), Some(&read(0, 0)));
        assert_eq!(vendor_string(leaves).len(), 12);
        assert_eq!(leaves.subleaves(1).count(), 1);
        // OSXSAVE and XCR0 go together, and XCR0 always includes the x87 state.
        assert_eq!(xcr0().is_some(), bit(leaves.regs(1, 0).ecx, 27));
        assert!(xcr0().map_or(true, |x| x & 1 == 1));
    }
}
