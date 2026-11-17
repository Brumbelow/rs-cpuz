//! Cache decoding: the deterministic cache parameters (Intel leaf 4, AMD leaf
//! `8000001Dh`), the older AMD descriptor leaves, and the folding of per-thread
//! descriptions into the per-package list of the CPU tab.

use std::collections::BTreeMap;

use super::cpuid::{bit, bits, Leaves};
use crate::model::{CacheInfo, CacheKind};

/// One cache as CPUID describes it on one logical processor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheDesc {
    pub level: u8,
    pub kind: CacheKind,
    pub size_kb: u32,
    /// Ways of associativity in the model's convention (`0` unknown, `u32::MAX` fully associative).
    pub ways: u32,
    pub line_size: u32,
    /// Number of low APIC-id bits that differ between the logical processors
    /// sharing one instance. `None` when CPUID does not describe the sharing.
    pub share_shift: Option<u32>,
    /// The most logical processors one instance can be shared by, as CPUID states
    /// it: the size of the APIC-id span, not a count of the threads present.
    pub max_sharing: Option<u32>,
}

/// What one logical processor of the package contributes to the cache list.
#[derive(Debug, Clone)]
pub struct ThreadCaches {
    pub apic_id: u32,
    /// Whether the thread belongs to the core type whose caches are displayed.
    pub displayed: bool,
    pub caches: Vec<CacheDesc>,
}

/// Describes the caches seen by one logical processor, best source first.
pub fn describe(leaves: Leaves) -> Vec<CacheDesc> {
    [0x04, 0x8000_001D]
        .into_iter()
        .map(|leaf| leaves.subleaves(leaf).filter_map(|l| deterministic(l.eax, l.ebx, l.ecx)).collect())
        .find(|caches: &Vec<CacheDesc>| !caches.is_empty())
        .unwrap_or_else(|| legacy(leaves))
}

/// Decodes one sub-leaf of the deterministic cache parameters. `None` for the terminator.
fn deterministic(eax: u32, ebx: u32, ecx: u32) -> Option<CacheDesc> {
    let kind = match bits(eax, 4, 0) {
        1 => CacheKind::Data,
        2 => CacheKind::Instruction,
        3 => CacheKind::Unified,
        _ => return None,
    };
    let line_size = bits(ebx, 11, 0) + 1;
    let partitions = bits(ebx, 21, 12) + 1;
    let ways = bits(ebx, 31, 22) + 1;
    let sets = u128::from(ecx) + 1;
    let bytes = u128::from(line_size) * u128::from(partitions) * u128::from(ways) * sets;
    // The sharing field is the highest APIC-id offset inside one instance, not a thread count.
    let sharing_ids = bits(eax, 25, 14) + 1;
    Some(CacheDesc {
        level: bits(eax, 7, 5) as u8,
        kind,
        size_kb: u32::try_from(bytes / 1024).unwrap_or(u32::MAX),
        ways: if bit(eax, 9) { u32::MAX } else { ways },
        line_size,
        share_shift: Some(sharing_ids.next_power_of_two().trailing_zeros()),
        max_sharing: Some(sharing_ids),
    })
}

/// Decodes the AMD descriptor leaves `80000005h` (L1) and `80000006h` (L2, L3).
fn legacy(leaves: Leaves) -> Vec<CacheDesc> {
    let (l1, l23) = (leaves.regs(0x8000_0005, 0), leaves.regs(0x8000_0006, 0));
    let cache = |level, kind, size_kb, ways, line_size| {
        let described = size_kb != 0;
        described.then_some(CacheDesc {
            level,
            kind,
            size_kb,
            ways,
            line_size,
            share_shift: None,
            max_sharing: None,
        })
    };
    let l1_cache = |kind, r: u32| {
        let ways = match bits(r, 23, 16) {
            0xFF => u32::MAX,
            n => n,
        };
        cache(1, kind, bits(r, 31, 24), ways, bits(r, 7, 0))
    };
    let outer = |level, r: u32, size_kb| {
        // An associativity code of zero means the cache is absent or disabled.
        let code = bits(r, 15, 12);
        cache(
            level,
            CacheKind::Unified,
            if code == 0 { 0 } else { size_kb },
            legacy_ways(code),
            bits(r, 7, 0),
        )
    };
    [
        l1_cache(CacheKind::Data, l1.ecx),
        l1_cache(CacheKind::Instruction, l1.edx),
        outer(2, l23.ecx, bits(l23.ecx, 31, 16)),
        // The L3 size is reported in 512 KB units.
        outer(3, l23.edx, bits(l23.edx, 31, 18) * 512),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// The associativity encoding of leaf `80000006h`. Unlisted codes are reserved.
fn legacy_ways(code: u32) -> u32 {
    match code {
        1..=4 => code,
        5 => 6,
        6 => 8,
        8 => 16,
        0xA => 32,
        0xB => 48,
        0xC => 64,
        0xD => 96,
        0xE => 128,
        0xF => u32::MAX,
        _ => 0,
    }
}

/// Folds the caches of every thread of a package into one entry per distinct
/// cache of the displayed core type. A level whose caches differ between those
/// cores (a stacked L3 on one core complex only) gets an entry for each, in the
/// order the threads report them. Instances are told apart by the APIC-id bits
/// above the sharing span, so `count` and `threads_sharing` reflect the real
/// topology. `None` when a cache does not describe its sharing (the descriptor leaves).
pub fn summarize(threads: &[ThreadCaches]) -> Option<Vec<CacheInfo>> {
    let mut list: Vec<CacheInfo> = Vec::new();
    let mut seen: Vec<&CacheDesc> = Vec::new();
    let displayed = threads.iter().filter(|t| t.displayed);
    for desc in displayed.clone().flat_map(|t| &t.caches) {
        if seen.contains(&desc) {
            continue;
        }
        seen.push(desc);
        let shift = desc.share_shift?;
        let instance = |t: &ThreadCaches| t.apic_id.checked_shr(shift).unwrap_or(0);
        // Threads per instance, counting every thread of the package that sees the same cache.
        let mut sharers: BTreeMap<u32, u32> = BTreeMap::new();
        for thread in threads.iter().filter(|t| t.caches.contains(desc)) {
            *sharers.entry(instance(thread)).or_default() += 1;
        }
        // Another core type may own caches of the very same description: only the
        // instances a displayed thread sits on count, and only their sharers.
        let mut instances: Vec<u32> =
            displayed.clone().filter(|t| t.caches.contains(desc)).map(instance).collect();
        instances.sort_unstable();
        instances.dedup();
        let sharing = instances.iter().filter_map(|i| sharers.get(i)).copied().max().unwrap_or(0);
        list.push(entry(desc, instances.len() as u32, sharing));
    }
    sort(&mut list);
    Some(list)
}

/// Builds the list from one thread's description when the per-thread APIC
/// picture is incomplete. `lookup` supplies `(count, threads_sharing)` for a
/// cache from another source; caches it does not know count once.
pub fn with_counts(
    caches: &[CacheDesc],
    lookup: impl Fn(&CacheDesc) -> Option<(u32, u32)>,
) -> Vec<CacheInfo> {
    let mut list: Vec<CacheInfo> = Vec::new();
    for (position, desc) in caches.iter().enumerate() {
        if !caches[..position].contains(desc) {
            let (count, threads_sharing) = lookup(desc).unwrap_or((1, 0));
            list.push(entry(desc, count, threads_sharing));
        }
    }
    sort(&mut list);
    list
}

fn entry(desc: &CacheDesc, count: u32, threads_sharing: u32) -> CacheInfo {
    CacheInfo {
        level: desc.level,
        kind: desc.kind,
        size_kb: desc.size_kb,
        count,
        ways: desc.ways,
        line_size: desc.line_size,
        threads_sharing,
    }
}

/// Display order: by level, data before instruction before unified. Caches of
/// one level and kind keep the order they were found in.
fn sort(list: &mut [CacheInfo]) {
    let rank = |kind| match kind {
        CacheKind::Data => 0,
        CacheKind::Instruction => 1,
        CacheKind::Unified => 2,
        CacheKind::Trace => 3,
    };
    list.sort_by_key(|c| (c.level, rank(c.kind)));
}

#[cfg(test)]
mod tests {
    use super::super::cpuid::fixtures::{hybrid, leaf, zen4, zen4_stacked};
    use super::*;

    fn info(
        level: u8,
        kind: CacheKind,
        size_kb: u32,
        count: u32,
        ways: u32,
        threads_sharing: u32,
    ) -> CacheInfo {
        CacheInfo { level, kind, size_kb, count, ways, line_size: 64, threads_sharing }
    }

    fn thread(apic_id: u32, displayed: bool, table: &[crate::model::CpuidLeaf]) -> ThreadCaches {
        ThreadCaches { apic_id, displayed, caches: describe(Leaves(table)) }
    }

    #[test]
    fn decodes_amd_deterministic_leaf() {
        let caches = describe(Leaves(&zen4(0)));
        let expected = [
            (1, CacheKind::Data, 32, 8, 1),
            (1, CacheKind::Instruction, 32, 8, 1),
            (2, CacheKind::Unified, 1024, 8, 1),
            (3, CacheKind::Unified, 32 * 1024, 16, 4),
        ];
        assert_eq!(caches.len(), expected.len());
        for (cache, (level, kind, size_kb, ways, shift)) in caches.iter().zip(expected) {
            let want = CacheDesc {
                level,
                kind,
                size_kb,
                ways,
                line_size: 64,
                share_shift: Some(shift),
                max_sharing: Some(1 << shift),
            };
            assert_eq!(*cache, want);
        }
    }

    #[test]
    fn decodes_intel_leaf_4() {
        let caches = describe(Leaves(&hybrid(0, true)));
        let sizes: Vec<_> = caches.iter().map(|c| (c.level, c.kind, c.size_kb, c.ways)).collect();
        assert_eq!(
            sizes,
            [
                (1, CacheKind::Data, 48, 12),
                (1, CacheKind::Instruction, 32, 8),
                (2, CacheKind::Unified, 2048, 16),
                (3, CacheKind::Unified, 36 * 1024, 12),
            ]
        );
        assert_eq!(
            caches.iter().map(|c| c.share_shift).collect::<Vec<_>>(),
            [Some(1), Some(1), Some(1), Some(7)]
        );
        assert_eq!(
            caches.iter().map(|c| c.max_sharing).collect::<Vec<_>>(),
            [Some(2), Some(2), Some(2), Some(128)]
        );
    }

    #[test]
    fn decodes_fully_associative_and_odd_geometries() {
        // Fully associative, 4 partitions of 3 ways, 8 sets of 64-byte lines: 6 KB.
        let fully = deterministic(0x0000_0221, 2 << 22 | 3 << 12 | 63, 7).unwrap();
        assert_eq!((fully.level, fully.size_kb, fully.ways), (1, 6, u32::MAX));
        assert_eq!(fully.share_shift, Some(0));
        // Six APIC ids per instance need three bits.
        let six = deterministic(0x0001_4163, 0x03C0_003F, 0x3FFF).unwrap();
        assert_eq!((six.share_shift, six.max_sharing), (Some(3), Some(6)));
        // The largest encodable cache must not overflow.
        let huge = deterministic(0x0000_0163, u32::MAX, u32::MAX).unwrap();
        assert_eq!(huge.size_kb, u32::MAX);
        assert_eq!(deterministic(0, 0, 0), None);
    }

    #[test]
    fn falls_back_to_the_descriptor_leaves() {
        // A K10 without leaf 8000001Dh: 64 KB 2-way L1s, 512 KB 16-way L2, 6 MB 48-way L3.
        let table = [
            leaf(0x8000_0005, 0, 0xFF30_FF10, 0xFF30_4000, 0x4002_0140, 0x4002_0140),
            leaf(0x8000_0006, 0, 0x2000_4200, 0x4200_4200, 0x0200_8140, 0x0030_B140),
        ];
        let caches = describe(Leaves(&table));
        let got: Vec<_> = caches.iter().map(|c| (c.level, c.kind, c.size_kb, c.ways, c.line_size)).collect();
        assert_eq!(
            got,
            [
                (1, CacheKind::Data, 64, 2, 64),
                (1, CacheKind::Instruction, 64, 2, 64),
                (2, CacheKind::Unified, 512, 16, 64),
                (3, CacheKind::Unified, 6 * 1024, 48, 64),
            ]
        );
        assert!(caches.iter().all(|c| c.share_shift.is_none() && c.max_sharing.is_none()));
        assert_eq!(summarize(&[ThreadCaches { apic_id: 0, displayed: true, caches }]), None);
    }

    #[test]
    fn descriptor_leaves_skip_absent_caches() {
        // No L3 (associativity code 0) and a fully associative L1 data cache.
        let table = [
            leaf(0x8000_0005, 0, 0, 0, 0x10FF_0140, 0x4002_0140),
            leaf(0x8000_0006, 0, 0, 0, 0x0400_6140, 0x0100_0140),
        ];
        let caches = describe(Leaves(&table));
        assert_eq!(caches.len(), 3);
        assert_eq!((caches[0].size_kb, caches[0].ways), (16, u32::MAX));
        assert_eq!((caches[2].level, caches[2].ways), (2, 8));
        assert!(describe(Leaves(&[])).is_empty());
    }

    #[test]
    fn counts_instances_on_a_uniform_package() {
        let threads: Vec<_> = (0..16).map(|apic| thread(apic, true, &zen4(apic))).collect();
        assert_eq!(
            summarize(&threads).unwrap(),
            [
                info(1, CacheKind::Data, 32, 8, 8, 2),
                info(1, CacheKind::Instruction, 32, 8, 8, 2),
                info(2, CacheKind::Unified, 1024, 8, 8, 2),
                info(3, CacheKind::Unified, 32 * 1024, 1, 16, 16),
            ]
        );
    }

    #[test]
    fn counts_real_sharers_not_the_apic_span() {
        // Six cores without SMT whose APIC ids skip every other value: the L1
        // span of two ids holds one thread, the L3 span of sixteen holds six.
        let threads: Vec<_> = (0..6).map(|core| thread(core * 2, true, &zen4(core * 2))).collect();
        let list = summarize(&threads).unwrap();
        assert_eq!((list[0].count, list[0].threads_sharing), (6, 1));
        assert_eq!((list[3].count, list[3].threads_sharing), (1, 6));
    }

    #[test]
    fn hybrid_package_shows_the_performance_cores() {
        // 8 P-cores with two threads each (APIC ids 0,1, 8,9, ...) and 16 E-cores (64, 66, ...).
        let p = (0..8)
            .flat_map(|core| [core * 8, core * 8 + 1])
            .map(|apic| thread(apic, true, &hybrid(apic, true)));
        let e = (0..16).map(|core| 64 + core * 2).map(|apic| thread(apic, false, &hybrid(apic, false)));
        let threads: Vec<_> = p.chain(e).collect();
        assert_eq!(
            summarize(&threads).unwrap(),
            [
                info(1, CacheKind::Data, 48, 8, 12, 2),
                info(1, CacheKind::Instruction, 32, 8, 8, 2),
                info(2, CacheKind::Unified, 2048, 8, 16, 2),
                // The L3 is shared with the E-cores: one instance, all 32 threads.
                info(3, CacheKind::Unified, 36 * 1024, 1, 12, 32),
            ]
        );
    }

    #[test]
    fn lists_every_geometry_of_a_level() {
        // Two core complexes, the first under a stacked 96 MB L3, the second under the plain 32 MB one.
        let threads: Vec<_> = (0..32).map(|apic| thread(apic, true, &zen4_stacked(apic))).collect();
        assert_eq!(
            summarize(&threads).unwrap(),
            [
                info(1, CacheKind::Data, 32, 16, 8, 2),
                info(1, CacheKind::Instruction, 32, 16, 8, 2),
                info(2, CacheKind::Unified, 1024, 16, 8, 2),
                info(3, CacheKind::Unified, 96 * 1024, 1, 16, 16),
                info(3, CacheKind::Unified, 32 * 1024, 1, 16, 16),
            ]
        );
        // Complexes of equal geometry still fold into one entry.
        let threads: Vec<_> = (0..32).map(|apic| thread(apic, true, &zen4(apic))).collect();
        let list = summarize(&threads).unwrap();
        assert_eq!(list.len(), 4);
        assert_eq!(list[3], info(3, CacheKind::Unified, 32 * 1024, 2, 16, 16));
    }

    #[test]
    fn sharers_are_counted_on_the_displayed_instances_only() {
        // Six P-cores of two threads and two four-core E clusters whose L2 leaves
        // are identical: 2 MB, 16-way, a sharing span of eight APIC ids.
        let l2 = [leaf(4, 0, 0x0001_C143, 0x03C0_003F, 0x0000_07FF, 0)];
        let p = (0..6).flat_map(|core| [core * 8, core * 8 + 1]).map(|apic| thread(apic, true, &l2));
        let e = (0..8).map(|core| thread(64 + core * 2, false, &l2));
        let threads: Vec<_> = p.chain(e).collect();
        assert_eq!(summarize(&threads).unwrap(), [info(2, CacheKind::Unified, 2048, 6, 16, 2)]);
    }

    #[test]
    fn external_counts_fill_in_for_missing_sharing() {
        let caches = describe(Leaves(&zen4(0)));
        let list = with_counts(&caches, |cache| (cache.level < 3).then_some((8, 2)));
        assert_eq!(list.len(), 4);
        assert_eq!((list[0].count, list[0].threads_sharing), (8, 2));
        assert_eq!((list[3].count, list[3].threads_sharing), (1, 0));
        // The lookup is asked about the cache itself, so it can tell sizes apart.
        let stacked = describe(Leaves(&zen4_stacked(0)));
        let list = with_counts(&stacked, |cache| (cache.size_kb == 96 * 1024).then_some((1, 16)));
        assert_eq!((list[3].size_kb, list[3].count, list[3].threads_sharing), (96 * 1024, 1, 16));
        assert_eq!((list[0].count, list[0].threads_sharing), (1, 0));
        assert!(summarize(&[]).unwrap().is_empty());
    }
}
