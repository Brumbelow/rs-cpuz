//! Processor topology: parsing what Windows reports about packages, cores and
//! caches, and combining it with the facts CPUID gives on each logical processor.

use std::collections::{BTreeMap, BTreeSet};
use std::mem::offset_of;

use windows::Win32::System::SystemInformation::{
    CacheData, CacheInstruction, CacheTrace, CacheUnified, RelationCache, RelationProcessorCore,
    RelationProcessorPackage, CACHE_RELATIONSHIP, PROCESSOR_RELATIONSHIP,
};

use super::cache;
use super::cpuid::{bit, bits, Leaves};
use crate::hw::processors::{group_masks, records, u16_at, u32_at, GroupMask, ProcId};
use crate::model::{CacheKind, CoreKind, CoreSet, CpuVendor, LogicalProcessor};

/// Core sets are listed in this order; the first one present is the one whose caches are shown.
const KIND_ORDER: [CoreKind; 4] =
    [CoreKind::Uniform, CoreKind::Performance, CoreKind::Efficient, CoreKind::LowPower];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsCore {
    /// Relative efficiency as rated by Windows; higher means more performance.
    pub efficiency_class: u8,
    /// Indices into [`OsTopology::processors`].
    pub processors: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsCache {
    pub level: u8,
    pub kind: CacheKind,
    /// Size of this instance in KiB.
    pub size_kb: u32,
    /// Indices into [`OsTopology::processors`] of the logical processors sharing this instance.
    pub processors: Vec<u32>,
}

/// The processor topology as Windows reports it, across all processor groups.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OsTopology {
    /// Every active logical processor, ordered by group then number. The position is its index.
    pub processors: Vec<ProcId>,
    /// The logical processors of each physical package.
    pub packages: Vec<Vec<u32>>,
    pub cores: Vec<OsCore>,
    pub caches: Vec<OsCache>,
}

impl OsTopology {
    /// Turns every logical processor into a core of its own, for processors
    /// whose cores Windows reports as the threads of a shared module.
    pub fn split_cores(&mut self) {
        let single = |efficiency_class, processor| OsCore { efficiency_class, processors: vec![processor] };
        self.cores = self
            .cores
            .iter()
            .flat_map(|core| core.processors.iter().map(|&p| single(core.efficiency_class, p)))
            .collect();
    }

    /// How many instances of a cache of this size the given logical processors
    /// see, and the largest number of logical processors sharing one of them.
    /// Instances of another size are a different cache, even at the same level.
    pub fn cache_count(
        &self,
        level: u8,
        kind: CacheKind,
        size_kb: u32,
        members: &[u32],
    ) -> Option<(u32, u32)> {
        let seen = |c: &&OsCache| {
            (c.level, c.kind, c.size_kb) == (level, kind, size_kb)
                && c.processors.iter().any(|p| members.contains(p))
        };
        let sharing = self.caches.iter().filter(seen).map(|c| c.processors.len()).max()?;
        Some((self.caches.iter().filter(seen).count() as u32, sharing as u32))
    }
}

enum Record {
    Core { efficiency_class: u8, masks: Vec<GroupMask> },
    Package(Vec<GroupMask>),
    Cache { level: u8, kind: CacheKind, size_kb: u32, masks: Vec<GroupMask> },
}

/// Parses a `GetLogicalProcessorInformationEx(RelationAll)` buffer, skipping the
/// kinds of record not used here. Malformed or truncated input yields whatever
/// could be read up to that point.
pub fn parse(buffer: &[u8]) -> OsTopology {
    let records: Vec<Record> =
        records(buffer).filter_map(|(relationship, body)| parse_record(relationship, body)).collect();

    // Every active logical processor belongs to exactly one core record.
    let processors: BTreeSet<ProcId> = records
        .iter()
        .filter_map(|r| if let Record::Core { masks, .. } = r { Some(masks) } else { None })
        .flat_map(|masks| masks.iter().flat_map(|m| m.processors()))
        .collect();
    let index: BTreeMap<ProcId, u32> = processors.iter().copied().zip(0..).collect();
    let members = |masks: &[GroupMask]| {
        let mut list: Vec<u32> =
            masks.iter().flat_map(|m| m.processors()).filter_map(|p| index.get(&p).copied()).collect();
        list.sort_unstable();
        list
    };

    let mut topology = OsTopology { processors: processors.iter().copied().collect(), ..Default::default() };
    for record in &records {
        match record {
            Record::Core { efficiency_class, masks } => topology
                .cores
                .push(OsCore { efficiency_class: *efficiency_class, processors: members(masks) }),
            Record::Package(masks) => topology.packages.push(members(masks)),
            Record::Cache { level, kind, size_kb, masks } => topology.caches.push(OsCache {
                level: *level,
                kind: *kind,
                size_kb: *size_kb,
                processors: members(masks),
            }),
        }
    }
    topology.packages.sort_by_key(|p| p.first().copied());
    topology.cores.sort_by_key(|c| c.processors.first().copied());
    topology
}

fn parse_record(relationship: i32, body: &[u8]) -> Option<Record> {
    if relationship == RelationProcessorCore.0 {
        let efficiency_class = *body.get(offset_of!(PROCESSOR_RELATIONSHIP, EfficiencyClass))?;
        Some(Record::Core { efficiency_class, masks: processor_masks(body)? })
    } else if relationship == RelationProcessorPackage.0 {
        Some(Record::Package(processor_masks(body)?))
    } else if relationship == RelationCache.0 {
        let kind = match u32_at(body, offset_of!(CACHE_RELATIONSHIP, Type))? as i32 {
            k if k == CacheUnified.0 => CacheKind::Unified,
            k if k == CacheInstruction.0 => CacheKind::Instruction,
            k if k == CacheData.0 => CacheKind::Data,
            k if k == CacheTrace.0 => CacheKind::Trace,
            _ => return None,
        };
        // Older Windows versions leave the group count zero and supply a single mask.
        let count = u16_at(body, offset_of!(CACHE_RELATIONSHIP, GroupCount))?.max(1);
        let masks = group_masks(body, offset_of!(CACHE_RELATIONSHIP, Anonymous), count)?;
        Some(Record::Cache {
            level: *body.get(offset_of!(CACHE_RELATIONSHIP, Level))?,
            kind,
            size_kb: u32_at(body, offset_of!(CACHE_RELATIONSHIP, CacheSize))? / 1024,
            masks,
        })
    } else {
        None
    }
}

fn processor_masks(body: &[u8]) -> Option<Vec<GroupMask>> {
    let count = u16_at(body, offset_of!(PROCESSOR_RELATIONSHIP, GroupCount))?;
    group_masks(body, offset_of!(PROCESSOR_RELATIONSHIP, GroupMask), count)
}

/// What CPUID says about the logical processor it ran on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadFacts {
    pub apic_id: u32,
    /// The core type on hybrid processors, `None` when CPUID does not distinguish.
    pub core_type: Option<CoreKind>,
    pub has_l3: bool,
}

pub fn thread_facts(vendor: CpuVendor, leaves: Leaves) -> ThreadFacts {
    ThreadFacts {
        apic_id: apic_id(leaves),
        core_type: core_type(vendor, leaves),
        has_l3: cache::describe(leaves).iter().any(|c| c.level == 3),
    }
}

/// The x2APIC id from the topology leaves, else the extended or initial APIC id.
fn apic_id(leaves: Leaves) -> u32 {
    let topology_leaf =
        [0x1F, 0x0B].into_iter().filter_map(|leaf| leaves.get(leaf, 0)).find(|l| l.ebx & 0xFFFF != 0);
    if let Some(leaf) = topology_leaf {
        return leaf.edx;
    }
    let amd_topology = bit(leaves.regs(0x8000_0001, 0).ecx, 22);
    match leaves.get(0x8000_001E, 0) {
        Some(extended) if amd_topology => extended.eax,
        _ => leaves.regs(1, 0).ebx >> 24,
    }
}

fn core_type(vendor: CpuVendor, leaves: Leaves) -> Option<CoreKind> {
    let raw = match vendor {
        // Leaf 1Ah is only meaningful when leaf 7 flags the part as hybrid.
        CpuVendor::Intel if bit(leaves.regs(7, 0).edx, 15) => match leaves.get(0x1A, 0)?.eax >> 24 {
            0x40 => 0,
            0x20 => 1,
            _ => return None,
        },
        // Leaf 80000026h flags heterogeneous cores at the level where they differ.
        CpuVendor::Amd if leaves.subleaves(0x8000_0026).any(|l| bit(l.eax, 30)) => {
            leaves.get(0x8000_0026, 0)?.ebx >> 28
        }
        _ => return None,
    };
    match raw {
        0 => Some(CoreKind::Performance),
        1 => Some(CoreKind::Efficient),
        _ => None,
    }
}

/// First AMD family whose leaf `8000001Eh` counts the threads of a core. On the
/// families before it the field counts the cores of a compute unit, which are
/// cores of their own here.
const ZEN_FAMILY: u16 = 0x17;

/// The most cores and logical processors one package can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capacity {
    pub cores: u32,
    pub threads: u32,
}

/// Reads the capacity of the package from the counts CPUID states for it. AMD
/// gives the number of threads the package has; the others give the number of
/// core and thread IDs it reserves, which may be more than it has cores and
/// threads. `None` without the basic feature leaf.
pub fn package_capacity(vendor: CpuVendor, ext_family: u16, leaves: Leaves) -> Option<Capacity> {
    let basic = leaves.get(1, 0)?;
    // Leaf 1 only counts logical processors when it flags that there may be several.
    let flagged = if bit(basic.edx, 28) { bits(basic.ebx, 23, 16).max(1) } else { 1 };
    Some(match vendor {
        CpuVendor::Amd | CpuVendor::Hygon => {
            let threads = leaves.get(0x8000_0008, 0).map_or(flagged, |sizes| bits(sizes.ecx, 7, 0) + 1);
            let counts_threads = bit(leaves.regs(0x8000_0001, 0).ecx, 22) && ext_family >= ZEN_FAMILY;
            let per_core = if counts_threads { bits(leaves.regs(0x8000_001E, 0).ebx, 15, 8) + 1 } else { 1 };
            Capacity { cores: threads / per_core, threads }
        }
        _ => {
            let cores = leaves
                .get(4, 0)
                .filter(|cache| bits(cache.eax, 4, 0) != 0)
                .map_or(1, |cache| bits(cache.eax, 31, 26) + 1);
            Capacity { cores, threads: flagged.max(cores) }
        }
    })
}

/// The cores of one kind in `package` that Windows schedules ahead of their
/// peers: those in the highest scheduling class found among them, lowest core
/// first. Empty when Windows ranks them all alike or leaves one of them unranked.
pub fn preferred_cores(
    logical: &[LogicalProcessor],
    classes: &[(ProcId, u8)],
    package: u32,
    kind: CoreKind,
) -> Vec<u32> {
    let class_of = |lp: &LogicalProcessor| {
        let id = ProcId { group: lp.group, number: lp.number };
        classes.iter().find(|(ranked, _)| *ranked == id).map(|&(_, class)| class)
    };
    // A core ranks as its best thread does.
    let mut cores: BTreeMap<u32, Option<u8>> = BTreeMap::new();
    for lp in logical.iter().filter(|lp| lp.package == package && lp.kind == kind) {
        let class = cores.entry(lp.core).or_default();
        *class = (*class).max(class_of(lp));
    }
    let Some(ranked) = cores.values().copied().collect::<Option<Vec<u8>>>() else { return Vec::new() };
    let top = ranked.iter().copied().max();
    if ranked.iter().all(|&class| Some(class) == top) {
        return Vec::new();
    }
    cores.into_iter().filter(|&(_, class)| class == top).map(|(core, _)| core).collect()
}

/// Lays out every logical processor: package, dense core index, thread index
/// and core kind. `facts` holds the CPUID view of the processors that could be
/// sampled, keyed by index; the others keep an APIC id of 0.
pub fn logical_processors(os: &OsTopology, facts: &BTreeMap<u32, ThreadFacts>) -> Vec<LogicalProcessor> {
    let mut logical: Vec<LogicalProcessor> = os
        .processors
        .iter()
        .zip(0u32..)
        .map(|(p, index)| LogicalProcessor {
            index,
            group: p.group,
            number: p.number,
            apic_id: facts.get(&index).map_or(0, |f| f.apic_id),
            ..Default::default()
        })
        .collect();

    for (members, package) in os.packages.iter().zip(0u32..) {
        for &i in members {
            if let Some(lp) = logical.get_mut(i as usize) {
                lp.package = package;
            }
        }
    }

    let mut cores_seen: BTreeMap<u32, u32> = BTreeMap::new();
    for (core, kind) in os.cores.iter().zip(core_kinds(os, facts)) {
        let Some(package) =
            core.processors.first().and_then(|&i| logical.get(i as usize)).map(|lp| lp.package)
        else {
            continue;
        };
        let number = cores_seen.entry(package).or_default();
        for (&i, smt) in core.processors.iter().zip(0u32..) {
            if let Some(lp) = logical.get_mut(i as usize) {
                (lp.core, lp.smt, lp.kind) = (*number, smt, kind);
            }
        }
        *number += 1;
    }
    logical
}

/// Decides the kind of every core. CPUID's core type wins; where it is silent
/// the Windows efficiency classes split the cores into performance (highest
/// class) and efficient ones. Cores that all turn out alike are `Uniform`.
fn core_kinds(os: &OsTopology, facts: &BTreeMap<u32, ThreadFacts>) -> Vec<CoreKind> {
    let top_class = os.cores.iter().map(|c| c.efficiency_class).max().unwrap_or(0);
    let classes_differ = os.cores.iter().any(|c| c.efficiency_class != top_class);
    let core_facts = |core: &OsCore| core.processors.iter().find_map(|i| facts.get(i));
    let typed = |core: &OsCore| core_facts(core).and_then(|f| Some((f.core_type?, f.has_l3)));
    // Low-power efficient cores sit outside the shared L3 that regular efficient cores use.
    let efficient_with_l3 = os.cores.iter().filter_map(typed).any(|t| t == (CoreKind::Efficient, true));

    let mut kinds: Vec<CoreKind> = os
        .cores
        .iter()
        .map(|core| match typed(core) {
            Some((CoreKind::Efficient, false)) if efficient_with_l3 => CoreKind::LowPower,
            Some((kind, _)) => kind,
            None if !classes_differ => CoreKind::Uniform,
            None if core.efficiency_class == top_class => CoreKind::Performance,
            None => CoreKind::Efficient,
        })
        .collect();
    if kinds.windows(2).all(|pair| pair[0] == pair[1]) {
        kinds.fill(CoreKind::Uniform);
    }
    kinds
}

/// Groups the cores of one package by kind, in display order.
pub fn core_sets(logical: &[LogicalProcessor], package: u32) -> Vec<CoreSet> {
    KIND_ORDER
        .into_iter()
        .filter_map(|kind| {
            let threads = || logical.iter().filter(move |lp| lp.package == package && lp.kind == kind);
            let cores: BTreeSet<u32> = threads().map(|lp| lp.core).collect();
            (!cores.is_empty()).then(|| CoreSet {
                kind,
                cores: cores.len() as u32,
                threads: threads().count() as u32,
            })
        })
        .collect()
}

#[cfg(test)]
pub mod fixtures {
    use super::*;

    /// A single-package topology: one core per entry, given as (efficiency class, threads).
    /// Every core gets private L1/L2 caches and the package one L3, sized as on
    /// the Zen 4 CPUID fixture: 32 KB, 32 KB, 1 MB and 32 MB.
    pub fn single_package(cores: &[(u8, u32)]) -> OsTopology {
        let mut topology = OsTopology::default();
        for &(efficiency_class, threads) in cores {
            let first = topology.processors.len() as u32;
            let members: Vec<u32> = (first..first + threads).collect();
            topology.processors.extend(members.iter().map(|&i| ProcId { group: 0, number: i as u8 }));
            for (level, kind, size_kb) in
                [(1, CacheKind::Data, 32), (1, CacheKind::Instruction, 32), (2, CacheKind::Unified, 1024)]
            {
                topology.caches.push(OsCache { level, kind, size_kb, processors: members.clone() });
            }
            topology.cores.push(OsCore { efficiency_class, processors: members });
        }
        let all: Vec<u32> = (0..topology.processors.len() as u32).collect();
        topology.caches.push(OsCache {
            level: 3,
            kind: CacheKind::Unified,
            size_kb: 32 * 1024,
            processors: all.clone(),
        });
        topology.packages.push(all);
        topology
    }
}

#[cfg(test)]
mod tests {
    use super::super::cpuid::fixtures::{hybrid, leaf, zen4};
    use super::*;
    use crate::hw::processors::fixtures::{put_masks, record};
    use windows::Win32::System::SystemInformation::{
        RelationGroup, RelationNumaNode, SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
    };

    const BODY: usize = offset_of!(SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX, Anonymous);

    fn processor_record(relationship: i32, efficiency_class: u8, masks: &[(u16, u64)]) -> Vec<u8> {
        let mut body = Vec::new();
        put_masks(&mut body, offset_of!(PROCESSOR_RELATIONSHIP, GroupMask), masks);
        body[offset_of!(PROCESSOR_RELATIONSHIP, EfficiencyClass)] = efficiency_class;
        let count_at = offset_of!(PROCESSOR_RELATIONSHIP, GroupCount);
        body[count_at..count_at + 2].copy_from_slice(&(masks.len() as u16).to_le_bytes());
        record(relationship, body)
    }

    fn core(efficiency_class: u8, group: u16, mask: u64) -> Vec<u8> {
        processor_record(RelationProcessorCore.0, efficiency_class, &[(group, mask)])
    }

    fn package(masks: &[(u16, u64)]) -> Vec<u8> {
        processor_record(RelationProcessorPackage.0, 0, masks)
    }

    /// A cache record; `size` is in bytes, as Windows reports it.
    fn cache_record(level: u8, kind: i32, size: u32, group_count: u16, masks: &[(u16, u64)]) -> Vec<u8> {
        let mut body = Vec::new();
        put_masks(&mut body, offset_of!(CACHE_RELATIONSHIP, Anonymous), masks);
        body[offset_of!(CACHE_RELATIONSHIP, Level)] = level;
        let size_at = offset_of!(CACHE_RELATIONSHIP, CacheSize);
        body[size_at..size_at + 4].copy_from_slice(&size.to_le_bytes());
        let type_at = offset_of!(CACHE_RELATIONSHIP, Type);
        body[type_at..type_at + 4].copy_from_slice(&kind.to_le_bytes());
        let count_at = offset_of!(CACHE_RELATIONSHIP, GroupCount);
        body[count_at..count_at + 2].copy_from_slice(&group_count.to_le_bytes());
        record(RelationCache.0, body)
    }

    fn facts(apic_id: u32, core_type: Option<CoreKind>, has_l3: bool) -> ThreadFacts {
        ThreadFacts { apic_id, core_type, has_l3 }
    }

    #[test]
    fn parses_cores_packages_and_caches() {
        let mut buffer = Vec::new();
        // Records arrive in no particular order and with kinds this module ignores.
        buffer.extend(record(RelationNumaNode.0, vec![0; 40]));
        buffer.extend(core(0, 0, 0b1100));
        buffer.extend(core(0, 0, 0b0011));
        buffer.extend(package(&[(0, 0b1111)]));
        buffer.extend(cache_record(1, CacheData.0, 32 << 10, 1, &[(0, 0b0011)]));
        buffer.extend(cache_record(1, CacheData.0, 32 << 10, 1, &[(0, 0b1100)]));
        buffer.extend(cache_record(3, CacheUnified.0, 16 << 20, 0, &[(0, 0b1111)]));
        buffer.extend(record(RelationGroup.0, vec![0; 72]));

        let topology = parse(&buffer);
        assert_eq!(topology.processors.len(), 4);
        assert_eq!(topology.processors[3], ProcId { group: 0, number: 3 });
        assert_eq!(topology.packages, [[0, 1, 2, 3]]);
        let cores: Vec<_> = topology.cores.iter().map(|c| c.processors.clone()).collect();
        assert_eq!(cores, [[0, 1], [2, 3]]);
        assert_eq!(topology.caches.len(), 3);
        assert_eq!((topology.caches[0].size_kb, topology.caches[2].size_kb), (32, 16 * 1024));
        assert_eq!(topology.cache_count(1, CacheKind::Data, 32, &[0, 1, 2, 3]), Some((2, 2)));
        assert_eq!(topology.cache_count(1, CacheKind::Data, 32, &[2]), Some((1, 2)));
        // A zero group count (older Windows) still carries one mask.
        assert_eq!(topology.cache_count(3, CacheKind::Unified, 16 * 1024, &[0]), Some((1, 4)));
        assert_eq!(topology.cache_count(2, CacheKind::Unified, 1024, &[0]), None);
    }

    #[test]
    fn counts_the_caches_of_a_level_by_size() {
        // Two core complexes of two cores each, the first under a larger L3.
        let mut buffer = Vec::new();
        for mask in [0b0001, 0b0010, 0b0100, 0b1000] {
            buffer.extend(core(0, 0, mask));
        }
        buffer.extend(cache_record(3, CacheUnified.0, 96 << 20, 1, &[(0, 0b0011)]));
        buffer.extend(cache_record(3, CacheUnified.0, 32 << 20, 1, &[(0, 0b1100)]));

        let topology = parse(&buffer);
        let all = [0, 1, 2, 3];
        assert_eq!(topology.cache_count(3, CacheKind::Unified, 96 * 1024, &all), Some((1, 2)));
        assert_eq!(topology.cache_count(3, CacheKind::Unified, 32 * 1024, &all), Some((1, 2)));
        assert_eq!(topology.cache_count(3, CacheKind::Unified, 32 * 1024, &[0, 1]), None);
        assert_eq!(topology.cache_count(3, CacheKind::Unified, 64 * 1024, &all), None);
    }

    #[test]
    fn spans_processor_groups() {
        // Two packages of 64 single-thread cores... abbreviated to the first and last core of each group.
        let mut buffer = Vec::new();
        buffer.extend(core(0, 1, 1 << 63));
        buffer.extend(core(0, 1, 1));
        buffer.extend(core(0, 0, 1 << 63));
        buffer.extend(core(0, 0, 1));
        buffer.extend(package(&[(1, 1 | 1 << 63)]));
        buffer.extend(package(&[(0, 1 | 1 << 63)]));
        // One cache shared across both groups.
        buffer.extend(cache_record(3, CacheUnified.0, 32 << 20, 2, &[(0, 1), (1, 1)]));

        let topology = parse(&buffer);
        let expected = [(0, 0), (0, 63), (1, 0), (1, 63)].map(|(group, number)| ProcId { group, number });
        assert_eq!(topology.processors, expected);
        assert_eq!(topology.packages, [[0, 1], [2, 3]]);
        assert_eq!(topology.caches[0].processors, [0, 2]);

        let logical = logical_processors(&topology, &BTreeMap::new());
        let placed: Vec<_> =
            logical.iter().map(|lp| (lp.index, lp.group, lp.number, lp.package, lp.core)).collect();
        assert_eq!(placed, [(0, 0, 0, 0, 0), (1, 0, 63, 0, 1), (2, 1, 0, 1, 0), (3, 1, 63, 1, 1)]);
        assert_eq!(core_sets(&logical, 1), [CoreSet { kind: CoreKind::Uniform, cores: 2, threads: 2 }]);
    }

    #[test]
    fn tolerates_malformed_buffers() {
        let good = core(0, 0, 0b11);
        assert_eq!(parse(&[]), OsTopology::default());
        assert_eq!(parse(&good[..good.len() - 1]), OsTopology::default());
        // A record that claims more group masks than it holds is dropped, the rest is kept.
        let mut lying = core(0, 0, 0b1100);
        let count_at = BODY + offset_of!(PROCESSOR_RELATIONSHIP, GroupCount);
        lying[count_at..count_at + 2].copy_from_slice(&u16::MAX.to_le_bytes());
        let mut buffer = lying;
        buffer.extend(&good);
        // Followed by a record whose size field is nonsense.
        buffer.extend([0xFF; 8]);
        let topology = parse(&buffer);
        assert_eq!(topology.processors.len(), 2);
        assert_eq!(topology.cores.len(), 1);
    }

    #[test]
    fn reads_thread_facts() {
        assert_eq!(thread_facts(CpuVendor::Amd, Leaves(&zen4(5))), facts(5, None, true));
        assert_eq!(
            thread_facts(CpuVendor::Intel, Leaves(&hybrid(9, true))),
            facts(9, Some(CoreKind::Performance), true)
        );
        assert_eq!(
            thread_facts(CpuVendor::Intel, Leaves(&hybrid(66, false))),
            facts(66, Some(CoreKind::Efficient), true)
        );
        // Without the topology leaves the APIC id comes from the extended leaf, then from leaf 1.
        let extended = [
            leaf(1, 0, 0, 0x0700_0000, 0, 0),
            leaf(0x8000_0001, 0, 0, 0, 1 << 22, 0),
            leaf(0x8000_001E, 0, 0x123, 0, 0, 0),
        ];
        assert_eq!(apic_id(Leaves(&extended)), 0x123);
        assert_eq!(apic_id(Leaves(&extended[..1])), 7);
        // An AMD part that flags heterogeneous cores reports the type per thread.
        let dense = [leaf(0x8000_0026, 0, 1 << 30 | 1, 1 << 28 | 2, 0x100, 0)];
        assert_eq!(core_type(CpuVendor::Amd, Leaves(&dense)), Some(CoreKind::Efficient));
        // The hybrid leaf alone, without the flag in leaf 7, is not trusted.
        assert_eq!(core_type(CpuVendor::Intel, Leaves(&[leaf(0x1A, 0, 0x2000_0001, 0, 0, 0)])), None);
    }

    #[test]
    fn reads_the_package_capacity() {
        let capacity = |cores, threads| Some(Capacity { cores, threads });
        // Zen 4: sixteen threads, two to a core.
        assert_eq!(package_capacity(CpuVendor::Amd, 0x19, Leaves(&zen4(0))), capacity(8, 16));
        // The same package with SMT switched off states eight threads, one to a core.
        let mut smt_off = zen4(0);
        for l in smt_off.iter_mut() {
            match l.leaf {
                0x8000_0008 => l.ecx = 0x0000_4007,
                0x8000_001E => l.ebx &= !0xFF00,
                _ => {}
            }
        }
        assert_eq!(package_capacity(CpuVendor::Amd, 0x19, Leaves(&smt_off)), capacity(8, 8));
        // A hybrid Intel part reserves 64 core ids and 128 thread ids.
        assert_eq!(package_capacity(CpuVendor::Intel, 6, Leaves(&hybrid(0, true))), capacity(64, 128));
        // Family 15h counts cores, and its compute units are not threads of one core.
        let piledriver = [
            leaf(1, 0, 0x0060_0F20, 0x0008_0800, 0, 1 << 28),
            leaf(0x8000_0001, 0, 0, 0, 1 << 22, 0),
            leaf(0x8000_0008, 0, 0x0000_3030, 0, 0x0000_4007, 0),
            leaf(0x8000_001E, 0, 0, 0x0000_0100, 0, 0),
        ];
        assert_eq!(package_capacity(CpuVendor::Amd, 0x15, Leaves(&piledriver)), capacity(8, 8));
        // Without the size leaf the count of leaf 1 stands in.
        assert_eq!(package_capacity(CpuVendor::Amd, 0x15, Leaves(&piledriver[..1])), capacity(8, 8));
    }

    #[test]
    fn capacity_of_processors_without_the_newer_leaves() {
        let capacity = |cores, threads| Some(Capacity { cores, threads });
        // One core with two threads, before leaf 4 existed.
        let hyper_threaded = [leaf(1, 0, 0x0000_0F29, 0x0002_0800, 0, 1 << 28)];
        assert_eq!(package_capacity(CpuVendor::Intel, 0xF, Leaves(&hyper_threaded)), capacity(1, 2));
        // A single-thread part leaves the count of leaf 1 unflagged, whatever it holds.
        let single = [leaf(1, 0, 0x0000_06D8, 0x0004_0800, 0, 0)];
        assert_eq!(package_capacity(CpuVendor::Intel, 6, Leaves(&single)), capacity(1, 1));
        // Two cores without Hyper-Threading: the thread ids cover at least the core ids.
        let dual_core = [leaf(1, 0, 0x0001_067A, 0, 0, 0), leaf(4, 0, 0x0400_0121, 0, 0, 0)];
        assert_eq!(package_capacity(CpuVendor::Intel, 6, Leaves(&dual_core)), capacity(2, 2));
        assert_eq!(package_capacity(CpuVendor::Intel, 6, Leaves(&[])), None);
    }

    #[test]
    fn finds_the_cores_windows_prefers() {
        let os = fixtures::single_package(&[(0, 2); 4]);
        let logical = logical_processors(&os, &BTreeMap::new());
        let ranked = |classes: [u8; 8]| -> Vec<(ProcId, u8)> {
            (0u8..).zip(classes).map(|(number, class)| (ProcId { group: 0, number }, class)).collect()
        };
        let preferred = |classes: &[(ProcId, u8)]| preferred_cores(&logical, classes, 0, CoreKind::Uniform);
        // Cores 1 and 3 share the highest class.
        assert_eq!(preferred(&ranked([2, 2, 5, 5, 0, 0, 5, 5])), [1, 3]);
        // A core ranks as its best thread does.
        assert_eq!(preferred(&ranked([1, 4, 3, 3, 0, 0, 2, 2])), [0]);
        // All alike, unranked altogether, or one core missing from the list: no preference.
        assert!(preferred(&ranked([3; 8])).is_empty());
        assert!(preferred(&[]).is_empty());
        assert!(preferred(&ranked([2, 2, 5, 5, 0, 0, 5, 5])[..6]).is_empty());
        // Another package or another kind of core has no say.
        let classes = ranked([2, 2, 5, 5, 0, 0, 5, 5]);
        assert!(preferred_cores(&logical, &classes, 1, CoreKind::Uniform).is_empty());
        assert!(preferred_cores(&logical, &classes, 0, CoreKind::Efficient).is_empty());
    }

    #[test]
    fn preference_is_judged_among_cores_of_one_kind() {
        // Two P-cores, the second one favoured, and two E-cores Windows ranks below both.
        let os = fixtures::single_package(&[(1, 2), (1, 2), (0, 1), (0, 1)]);
        let logical = logical_processors(&os, &BTreeMap::new());
        let classes: Vec<(ProcId, u8)> = (0u8..)
            .zip([4, 4, 5, 5, 1, 1])
            .map(|(number, class)| (ProcId { group: 0, number }, class))
            .collect();
        assert_eq!(preferred_cores(&logical, &classes, 0, CoreKind::Performance), [1]);
        assert!(preferred_cores(&logical, &classes, 0, CoreKind::Efficient).is_empty());
    }

    #[test]
    fn lays_out_a_uniform_package() {
        let os = fixtures::single_package(&[(0, 2); 8]);
        let facts: BTreeMap<u32, ThreadFacts> = (0..16).map(|i| (i, facts(i, None, true))).collect();
        let logical = logical_processors(&os, &facts);
        assert_eq!(logical.len(), 16);
        let lp = logical[13];
        assert_eq!((lp.index, lp.number, lp.apic_id, lp.package, lp.core, lp.smt), (13, 13, 13, 0, 6, 1));
        assert!(logical.iter().all(|lp| lp.kind == CoreKind::Uniform));
        assert_eq!(core_sets(&logical, 0), [CoreSet { kind: CoreKind::Uniform, cores: 8, threads: 16 }]);
        assert!(core_sets(&logical, 1).is_empty());
    }

    #[test]
    fn splits_hybrid_cores_by_cpuid_type() {
        // 2 P-cores with SMT, 4 E-cores on the shared L3 and 2 low-power E-cores off it.
        let os = fixtures::single_package(&[(1, 2), (1, 2), (0, 1), (0, 1), (0, 1), (0, 1), (0, 1), (0, 1)]);
        let mut facts: BTreeMap<u32, ThreadFacts> = BTreeMap::new();
        facts.extend((0..4).map(|i| (i, self::facts(i, Some(CoreKind::Performance), true))));
        facts.extend((4..8).map(|i| (i, self::facts(i, Some(CoreKind::Efficient), true))));
        facts.extend((8..10).map(|i| (i, self::facts(i, Some(CoreKind::Efficient), false))));
        let logical = logical_processors(&os, &facts);
        assert_eq!(
            core_sets(&logical, 0),
            [
                CoreSet { kind: CoreKind::Performance, cores: 2, threads: 4 },
                CoreSet { kind: CoreKind::Efficient, cores: 4, threads: 4 },
                CoreSet { kind: CoreKind::LowPower, cores: 2, threads: 2 },
            ]
        );
        assert_eq!((logical[9].core, logical[9].kind), (7, CoreKind::LowPower));
    }

    #[test]
    fn efficient_cores_without_l3_stay_efficient_when_none_has_one() {
        let os = fixtures::single_package(&[(1, 1), (1, 1), (0, 1), (0, 1)]);
        let mut facts: BTreeMap<u32, ThreadFacts> = BTreeMap::new();
        facts.extend((0..2).map(|i| (i, self::facts(i, Some(CoreKind::Performance), true))));
        facts.extend((2..4).map(|i| (i, self::facts(i, Some(CoreKind::Efficient), false))));
        let kinds: Vec<_> = logical_processors(&os, &facts).iter().map(|lp| lp.kind).collect();
        assert_eq!(
            kinds,
            [CoreKind::Performance, CoreKind::Performance, CoreKind::Efficient, CoreKind::Efficient]
        );
    }

    #[test]
    fn falls_back_to_efficiency_classes() {
        // CPUID is silent (or the threads could not be sampled) but Windows rates the cores differently.
        let os = fixtures::single_package(&[(1, 2), (0, 1), (0, 1)]);
        let logical = logical_processors(&os, &BTreeMap::new());
        assert_eq!(
            core_sets(&logical, 0),
            [
                CoreSet { kind: CoreKind::Performance, cores: 1, threads: 2 },
                CoreSet { kind: CoreKind::Efficient, cores: 2, threads: 2 },
            ]
        );
        assert!(logical.iter().all(|lp| lp.apic_id == 0));
    }

    #[test]
    fn splits_module_threads_into_cores() {
        let mut os = fixtures::single_package(&[(0, 2); 4]);
        os.split_cores();
        let logical = logical_processors(&os, &BTreeMap::new());
        assert_eq!(core_sets(&logical, 0), [CoreSet { kind: CoreKind::Uniform, cores: 8, threads: 8 }]);
        assert_eq!((logical[5].core, logical[5].smt), (5, 0));
    }

    #[test]
    fn cores_of_one_type_are_uniform() {
        // A hybrid-capable part with only its P-cores enabled is not shown as hybrid.
        let os = fixtures::single_package(&[(0, 2); 4]);
        let facts: BTreeMap<u32, ThreadFacts> =
            (0..8).map(|i| (i, self::facts(i, Some(CoreKind::Performance), true))).collect();
        assert!(logical_processors(&os, &facts).iter().all(|lp| lp.kind == CoreKind::Uniform));
        assert!(logical_processors(&OsTopology::default(), &facts).is_empty());
    }
}
