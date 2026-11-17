//! Processor identification through the CPUID instruction and the Windows topology APIs.

mod cache;
pub mod cpuid;
mod db;
mod features;
mod name;
mod os;
mod topology;

use std::collections::{BTreeMap, BTreeSet};

use crate::hw::processors::{self, ProcId};
use crate::model::{CacheInfo, CoreKind, CpuInfo, CpuVendor, CpuidLeaf, LogicalProcessor, ThreadDump};
use cache::ThreadCaches;
use cpuid::Leaves;

pub use cache::CacheDesc;
use topology::{OsTopology, ThreadFacts};

/// Display family of the AMD Bulldozer, Piledriver, Steamroller and Excavator cores.
const BULLDOZER_FAMILY: u16 = 0x15;

/// Identifies the processor(s): names, signature, features, caches and topology.
pub fn detect() -> CpuInfo {
    let topology = os::topology();
    let thread_dumps = os::thread_dumps(&topology);
    interpret(Sample {
        topology,
        thread_dumps,
        scheduling_classes: processors::scheduling_classes(),
        xcr0: cpuid::xcr0(),
        microcode: os::update_revision().as_deref().and_then(microcode_revision),
        firmware_mhz: os::firmware_mhz(),
    })
}

/// The caches CPUID describes in the leaves of one logical processor, in the order it lists them.
pub fn thread_caches(leaves: &[CpuidLeaf]) -> Vec<CacheDesc> {
    cache::describe(Leaves(leaves))
}

/// Everything read from the machine, before any interpretation.
struct Sample {
    topology: OsTopology,
    /// One CPUID dump per sampled logical processor, lowest index first.
    thread_dumps: Vec<ThreadDump>,
    /// The scheduling class Windows gives each logical processor.
    scheduling_classes: Vec<(ProcId, u8)>,
    /// The OS-enabled XSAVE state mask, `None` when XSAVE is off.
    xcr0: Option<u64>,
    microcode: Option<u32>,
    /// The processor speed Windows recorded at boot, in MHz.
    firmware_mhz: Option<u32>,
}

/// Turns a sample into the model. Pure, so fixtures of other machines can be run through it.
fn interpret(sample: Sample) -> CpuInfo {
    let Sample { mut topology, thread_dumps, scheduling_classes, xcr0, microcode, firmware_mhz } = sample;
    // Identification comes from the first sampled thread: the boot processor, a P-core on hybrid parts.
    let leaves = Leaves(thread_dumps.first().map_or(&[], |dump| dump.leaves.as_slice()));
    let vendor_string = cpuid::vendor_string(leaves);
    let vendor = cpuid::vendor(&vendor_string);
    let signature = cpuid::signature(leaves.regs(1, 0).eax);
    let specification = cpuid::brand_string(leaves);
    let name = name::short_name(vendor, &specification);
    let known = db::lookup(vendor, signature.ext_family, signature.ext_model, signature.stepping, &name);

    // A Bulldozer-family module holds two integer cores. Windows schedules them as
    // the two threads of one core; AMD and this tool count them as cores.
    if vendor == CpuVendor::Amd && signature.ext_family == BULLDOZER_FAMILY {
        topology.split_cores();
    }

    let facts: BTreeMap<u32, ThreadFacts> = thread_dumps
        .iter()
        .map(|dump| (dump.processor, topology::thread_facts(vendor, Leaves(&dump.leaves))))
        .collect();
    let logical = topology::logical_processors(&topology, &facts);
    let core_sets = topology::core_sets(&logical, 0);
    let displayed = core_sets.first().map_or(CoreKind::Uniform, |set| set.kind);
    let cores: u32 = core_sets.iter().map(|set| set.cores).sum();
    let threads: u32 = core_sets.iter().map(|set| set.threads).sum();
    // A capacity below what the package holds is not one: the count behind it saturated or is not kept up.
    let capacity = topology::package_capacity(vendor, signature.ext_family, leaves);

    CpuInfo {
        vendor,
        codename: known.map(|k| k.codename.to_string()),
        package: known.and_then(|k| k.package).map(str::to_string),
        technology_nm: known.and_then(|k| k.technology_nm),
        max_tdp_w: db::max_tdp(&name),
        core_voltage: None,
        family: signature.family,
        model: signature.model,
        stepping: signature.stepping,
        ext_family: signature.ext_family,
        ext_model: signature.ext_model,
        revision: db::revision(vendor, signature.ext_family, signature.ext_model, signature.stepping)
            .map(str::to_string),
        instructions: features::instructions(vendor, leaves, xcr0),
        caches: package_caches(&topology, &logical, &thread_dumps, displayed),
        cores,
        threads,
        max_cores: capacity.map(|capacity| capacity.cores).filter(|&max| max >= cores),
        max_threads: capacity.map(|capacity| capacity.threads).filter(|&max| max >= threads),
        preferred_cores: topology::preferred_cores(&logical, &scheduling_classes, 0, displayed),
        sockets: logical.iter().map(|lp| lp.package).collect::<BTreeSet<_>>().len() as u32,
        core_sets,
        logical,
        microcode,
        nominal_mhz: nominal_mhz(leaves, &specification, firmware_mhz),
        max_cpuid_leaf: leaves.max_basic(),
        max_ext_cpuid_leaf: leaves.max_extended(),
        vendor_string,
        name,
        specification,
        thread_dumps,
    }
}

/// The caches of the first package, as seen by its cores of the `displayed` kind.
fn package_caches(
    topology: &OsTopology,
    logical: &[LogicalProcessor],
    thread_dumps: &[ThreadDump],
    displayed: CoreKind,
) -> Vec<CacheInfo> {
    let described: BTreeMap<u32, Vec<CacheDesc>> =
        thread_dumps.iter().map(|dump| (dump.processor, cache::describe(Leaves(&dump.leaves)))).collect();
    let members = || logical.iter().filter(|lp| lp.package == 0);

    // The exact picture needs CPUID from every thread of the package.
    let threads: Option<Vec<ThreadCaches>> = members()
        .map(|lp| {
            let caches = described.get(&lp.index)?.clone();
            Some(ThreadCaches { apic_id: lp.apic_id, displayed: lp.kind == displayed, caches })
        })
        .collect();
    if let Some(list) = threads.filter(|t| !t.is_empty()).and_then(|t| cache::summarize(&t)) {
        return list;
    }

    // Otherwise describe the caches from one thread and let Windows count the instances
    // among the processors of that thread's core type: the displayed type when one of
    // its threads was sampled, else whichever type the first sampled thread belongs to.
    let of_kind = |kind: CoreKind| members().filter(move |lp| lp.kind == kind).map(|lp| lp.index);
    let reference = of_kind(displayed)
        .find(|index| described.contains_key(index))
        .or_else(|| described.keys().next().copied());
    let Some(caches) = reference.and_then(|index| described.get(&index)) else { return Vec::new() };
    // A thread Windows does not list has no peers to count; its caches then count once each.
    let kind = logical.iter().find(|lp| Some(lp.index) == reference).map(|lp| lp.kind);
    let counted: Vec<u32> = kind.map(|kind| of_kind(kind).collect()).unwrap_or_default();
    cache::with_counts(caches, |cache| topology.cache_count(cache.level, cache.kind, cache.size_kb, &counted))
}

/// The advertised base frequency: CPUID leaf 16h, else the brand string, else the firmware value.
fn nominal_mhz(leaves: Leaves, brand: &str, firmware_mhz: Option<u32>) -> Option<f64> {
    let base = leaves.get(0x16, 0).map(|leaf| leaf.eax & 0xFFFF).filter(|&mhz| mhz != 0);
    base.map(f64::from).or_else(|| name::brand_mhz(brand)).or(firmware_mhz.map(f64::from))
}

/// Extracts the microcode revision from the registry's `Update Revision` value.
/// Windows stores the 64-bit signature MSR: Intel reports the revision in its
/// high half, AMD in the low half (and some systems store only four bytes).
fn microcode_revision(raw: &[u8]) -> Option<u32> {
    let dword = |at: usize| raw.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    dword(4).filter(|&high| high != 0).or(dword(0)).filter(|&revision| revision != 0)
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use super::cpuid::fixtures::{brand_leaves, hybrid, leaf, zen4, zen4_stacked};
    use super::topology::fixtures::single_package;
    use super::topology::OsCache;
    use super::*;
    use crate::model::{CacheKind, CoreSet};

    fn dumps(tables: impl IntoIterator<Item = Vec<CpuidLeaf>>) -> Vec<ThreadDump> {
        tables.into_iter().zip(0..).map(|(leaves, processor)| ThreadDump { processor, leaves }).collect()
    }

    /// One core of a four-module, eight-core Piledriver desktop part.
    fn piledriver(apic_id: u32) -> Vec<CpuidLeaf> {
        let mut leaves = vec![
            leaf(0, 0, 0x0000_000D, 0x6874_7541, 0x444D_4163, 0x6974_6E65),
            leaf(1, 0, 0x0060_0F20, apic_id << 24 | 0x0008_0800, 0x3E98_320B, 0x178B_FBFF),
            leaf(0x8000_0000, 0, 0x8000_001E, 0x6874_7541, 0x444D_4163, 0x6974_6E65),
            leaf(0x8000_0001, 0, 0x0060_0F20, 0, 0x01EB_BFFF, 0x2FD3_FBFF),
        ];
        leaves.extend(brand_leaves("AMD FX(tm)-8350 Eight-Core Processor"));
        leaves.extend([
            // 16 KB L1D per core; 64 KB L1I and 2 MB L2 per module; 8 MB L3 for the package.
            leaf(0x8000_001D, 0, 0x0000_0121, 0x00C0_003F, 0x0000_003F, 0),
            leaf(0x8000_001D, 1, 0x0000_4122, 0x0040_003F, 0x0000_01FF, 0),
            leaf(0x8000_001D, 2, 0x0000_4143, 0x03C0_003F, 0x0000_07FF, 0),
            leaf(0x8000_001D, 3, 0x0001_C163, 0x0FC0_003F, 0x0000_07FF, 1),
            leaf(0x8000_001E, 0, apic_id, 0x0000_0100 | apic_id >> 1, 0, 0),
        ]);
        leaves
    }

    fn zen4_sample() -> Sample {
        Sample {
            topology: single_package(&[(0, 2); 8]),
            thread_dumps: dumps((0..16).map(zen4)),
            // Windows rates the first two cores above the rest.
            scheduling_classes: (0u8..16)
                .map(|number| (ProcId { group: 0, number }, if number < 4 { 6 } else { number / 4 }))
                .collect(),
            xcr0: Some(0xE7),
            microcode: microcode_revision(&[0x03, 0x12, 0x60, 0x0A]),
            firmware_mhz: Some(3800),
        }
    }

    /// One cache instance as Windows reports it.
    fn os_cache(level: u8, kind: CacheKind, size_kb: u32, processors: Range<u32>) -> OsCache {
        OsCache { level, kind, size_kb, processors: processors.collect() }
    }

    fn hybrid_sample() -> Sample {
        let p = (0..8u32).flat_map(|core| [core * 8, core * 8 + 1]).map(|apic| hybrid(apic, true));
        let e = (0..16u32).map(|core| hybrid(64 + core * 2, false));
        let cores: Vec<(u8, u32)> = [(1, 2); 8].into_iter().chain([(0, 1); 16]).collect();
        let mut topology = single_package(&cores);
        // Windows sees the caches CPUID describes: per P-core, per E-core, and an L2 per E-cluster.
        let (data, instruction, unified) = (CacheKind::Data, CacheKind::Instruction, CacheKind::Unified);
        topology.caches = vec![os_cache(3, unified, 36 * 1024, 0..32)];
        for first in (0..16).step_by(2) {
            let private = [(1, data, 48), (1, instruction, 32), (2, unified, 2048)];
            let core = first..first + 2;
            topology.caches.extend(private.map(|(level, kind, kb)| os_cache(level, kind, kb, core.clone())));
        }
        for thread in 16..32 {
            let core = thread..thread + 1;
            topology.caches.extend([os_cache(1, data, 32, core.clone()), os_cache(1, instruction, 64, core)]);
        }
        topology.caches.extend((16..32).step_by(4).map(|first| os_cache(2, unified, 4096, first..first + 4)));
        Sample {
            topology,
            thread_dumps: dumps(p.chain(e)),
            scheduling_classes: Vec::new(),
            xcr0: Some(0x7),
            microcode: microcode_revision(&[0, 0, 0, 0, 0x2B, 0x01, 0, 0]),
            firmware_mhz: Some(2995),
        }
    }

    /// Sixteen cores in two core complexes, the first under a stacked 96 MB L3.
    fn stacked_sample() -> Sample {
        let mut topology = single_package(&[(0, 2); 16]);
        topology.caches.retain(|cache| cache.level != 3);
        topology.caches.push(os_cache(3, CacheKind::Unified, 96 * 1024, 0..16));
        topology.caches.push(os_cache(3, CacheKind::Unified, 32 * 1024, 16..32));
        Sample { topology, thread_dumps: dumps((0..32).map(zen4_stacked)), ..zen4_sample() }
    }

    /// Size text, associativity and sharers of the one cache expected at a level.
    fn cache_line(cpu: &CpuInfo, level: u8, kind: CacheKind) -> (String, String, u32) {
        let [cache] = cpu.caches_of(level, kind)[..] else { panic!("one L{level} {kind:?} cache expected") };
        (cache.size_text(), cache.ways_text(), cache.threads_sharing)
    }

    /// Size in MB, instance count and sharers of every L3 entry.
    fn level3(cpu: &CpuInfo) -> Vec<(u32, u32, u32)> {
        let caches = cpu.caches_of(3, CacheKind::Unified);
        caches.iter().map(|c| (c.size_kb / 1024, c.count, c.threads_sharing)).collect()
    }

    #[test]
    fn interprets_a_zen4_desktop() {
        let cpu = interpret(zen4_sample());
        assert_eq!(cpu.vendor, CpuVendor::Amd);
        assert_eq!(cpu.vendor_string, "AuthenticAMD");
        assert_eq!(cpu.name, "AMD Ryzen 7 7700");
        assert_eq!(cpu.specification, "AMD Ryzen 7 7700 8-Core Processor");
        assert_eq!(cpu.codename.as_deref(), Some("Raphael"));
        assert_eq!(cpu.package.as_deref(), Some("Socket AM5 (LGA1718)"));
        assert_eq!(cpu.technology_nm, Some(5));
        assert_eq!(cpu.max_tdp_w, Some(65.0));
        assert_eq!(cpu.core_voltage, None);
        assert_eq!((cpu.family, cpu.model, cpu.stepping), (0xF, 0x1, 0x2));
        assert_eq!((cpu.ext_family, cpu.ext_model), (0x19, 0x61));
        assert_eq!(cpu.revision.as_deref(), Some("RPL-B2"));
        assert_eq!(
            cpu.instructions.join(", "),
            "MMX (+), SSE, SSE2, SSE3, SSSE3, SSE4.1, SSE4.2, SSE4A, x86-64, AES, AVX, AVX2, \
             AVX512 (DQ, BW, VL, CD, IFMA, VBMI, VBMI2, VNNI, BITALG, VPOPCNTDQ, BF16), FMA3, SHA"
        );
        assert_eq!(cache_line(&cpu, 1, CacheKind::Data), ("8 x 32 KBytes".into(), "8-way".into(), 2));
        assert_eq!(cache_line(&cpu, 1, CacheKind::Instruction), ("8 x 32 KBytes".into(), "8-way".into(), 2));
        assert_eq!(cache_line(&cpu, 2, CacheKind::Unified), ("8 x 1 MBytes".into(), "8-way".into(), 2));
        assert_eq!(cache_line(&cpu, 3, CacheKind::Unified), ("32 MBytes".into(), "16-way".into(), 16));
        assert!(cpu.caches.iter().all(|c| c.line_size == 64));
        assert_eq!((cpu.cores, cpu.threads, cpu.sockets), (8, 16, 1));
        assert_eq!((cpu.max_cores, cpu.max_threads), (Some(8), Some(16)));
        assert_eq!(cpu.preferred_cores, [0, 1]);
        assert_eq!(cpu.core_sets, [CoreSet { kind: CoreKind::Uniform, cores: 8, threads: 16 }]);
        assert!(!cpu.is_hybrid());
        assert_eq!(cpu.logical.len(), 16);
        assert_eq!((cpu.logical[15].apic_id, cpu.logical[15].core, cpu.logical[15].smt), (15, 7, 1));
        assert_eq!(cpu.microcode, Some(0x0A60_1203));
        assert_eq!(cpu.nominal_mhz, Some(3800.0));
        assert_eq!((cpu.max_cpuid_leaf, cpu.max_ext_cpuid_leaf), (0xD, 0x8000_0027));
        assert_eq!(cpu.thread_dumps.len(), 16);
    }

    #[test]
    fn interprets_a_hybrid_desktop() {
        let cpu = interpret(hybrid_sample());
        assert_eq!(cpu.name, "Intel Core i9 13900K");
        assert_eq!(cpu.codename.as_deref(), Some("Raptor Lake"));
        assert_eq!(cpu.package.as_deref(), Some("Socket 1700 LGA"));
        assert_eq!((cpu.technology_nm, cpu.max_tdp_w), (Some(10), Some(125.0)));
        assert_eq!((cpu.family, cpu.model, cpu.stepping, cpu.ext_family, cpu.ext_model), (6, 7, 1, 6, 0xB7));
        assert_eq!(cpu.revision.as_deref(), Some("B0"));
        assert_eq!(
            cpu.core_sets,
            [
                CoreSet { kind: CoreKind::Performance, cores: 8, threads: 16 },
                CoreSet { kind: CoreKind::Efficient, cores: 16, threads: 16 },
            ]
        );
        assert!(cpu.is_hybrid());
        assert_eq!((cpu.cores, cpu.threads, cpu.sockets), (24, 32, 1));
        // The id space of the package, not the number of cores built into it.
        assert_eq!((cpu.max_cores, cpu.max_threads), (Some(64), Some(128)));
        assert!(cpu.preferred_cores.is_empty());
        assert_eq!(
            (cpu.logical[16].apic_id, cpu.logical[16].core, cpu.logical[16].kind),
            (64, 8, CoreKind::Efficient)
        );
        // The cache list is that of the P-cores; the L3 they share with the E-cores counts all 32 threads.
        assert_eq!(cache_line(&cpu, 1, CacheKind::Data), ("8 x 48 KBytes".into(), "12-way".into(), 2));
        assert_eq!(cache_line(&cpu, 2, CacheKind::Unified), ("8 x 2 MBytes".into(), "16-way".into(), 2));
        assert_eq!(cache_line(&cpu, 3, CacheKind::Unified), ("36 MBytes".into(), "12-way".into(), 32));
        assert_eq!(cpu.caches.len(), 4);
        assert_eq!(cpu.microcode, Some(0x12B));
        // Leaf 16h wins over the measured value.
        assert_eq!(cpu.nominal_mhz, Some(3000.0));
        assert_eq!(cpu.max_cpuid_leaf, 0x20);
    }

    #[test]
    fn counts_bulldozer_modules_as_two_cores() {
        // Windows reports four cores of two threads; the part is an eight-core.
        let cpu = interpret(Sample {
            topology: single_package(&[(0, 2); 4]),
            thread_dumps: dumps((0..8).map(piledriver)),
            scheduling_classes: Vec::new(),
            xcr0: Some(0x7),
            microcode: None,
            firmware_mhz: Some(4013),
        });
        assert_eq!(cpu.name, "AMD FX-8350");
        assert_eq!((cpu.codename.as_deref(), cpu.technology_nm), (Some("Vishera"), Some(32)));
        assert_eq!(cpu.package.as_deref(), Some("Socket AM3+ (942)"));
        assert_eq!((cpu.ext_family, cpu.ext_model, cpu.stepping), (0x15, 0x02, 0));
        assert_eq!((cpu.revision.as_deref(), cpu.max_tdp_w), (Some("OR-C0"), Some(125.0)));
        assert_eq!((cpu.cores, cpu.threads), (8, 8));
        assert_eq!((cpu.max_cores, cpu.max_threads), (Some(8), Some(8)));
        assert_eq!((cpu.logical[7].apic_id, cpu.logical[7].core, cpu.logical[7].smt), (7, 7, 0));
        assert_eq!(cache_line(&cpu, 1, CacheKind::Data), ("8 x 16 KBytes".into(), "4-way".into(), 1));
        assert_eq!(cache_line(&cpu, 1, CacheKind::Instruction), ("4 x 64 KBytes".into(), "2-way".into(), 2));
        assert_eq!(cache_line(&cpu, 2, CacheKind::Unified), ("4 x 2 MBytes".into(), "16-way".into(), 2));
        assert_eq!(cache_line(&cpu, 3, CacheKind::Unified), ("8 MBytes".into(), "64-way".into(), 8));
        assert!(cpu.instructions.iter().any(|i| i == "XOP") && cpu.instructions.iter().any(|i| i == "AMD-V"));
        assert_eq!(cpu.nominal_mhz, Some(4013.0));
    }

    #[test]
    fn partial_sampling_falls_back_to_the_windows_cache_counts() {
        // Only the first two threads could be pinned (restricted process affinity).
        let mut sample = zen4_sample();
        sample.thread_dumps.truncate(2);
        let cpu = interpret(sample);
        assert_eq!((cpu.cores, cpu.threads, cpu.sockets), (8, 16, 1));
        assert_eq!(cache_line(&cpu, 1, CacheKind::Data), ("8 x 32 KBytes".into(), "8-way".into(), 2));
        assert_eq!(cache_line(&cpu, 3, CacheKind::Unified), ("32 MBytes".into(), "16-way".into(), 16));
        assert_eq!((cpu.logical[1].apic_id, cpu.logical[2].apic_id), (1, 0));
    }

    #[test]
    fn lists_both_l3s_of_a_part_with_one_stacked_complex() {
        let cpu = interpret(stacked_sample());
        assert_eq!((cpu.cores, cpu.threads, cpu.sockets), (16, 32, 1));
        // The fixture keeps the counts of the eight-core part, which cannot bound sixteen cores.
        assert_eq!((cpu.max_cores, cpu.max_threads), (None, None));
        assert_eq!(cache_line(&cpu, 1, CacheKind::Data), ("16 x 32 KBytes".into(), "8-way".into(), 2));
        assert_eq!(cache_line(&cpu, 2, CacheKind::Unified), ("16 x 1 MBytes".into(), "8-way".into(), 2));
        assert_eq!(level3(&cpu), [(96, 1, 16), (32, 1, 16)]);
        assert_eq!(cpu.caches.len(), 5);
    }

    #[test]
    fn partial_sampling_counts_only_caches_of_the_sampled_size() {
        // Windows knows of two L3s; only one of them is the cache the sampled complex describes.
        let mut sample = stacked_sample();
        sample.thread_dumps.truncate(8);
        let cpu = interpret(sample);
        assert_eq!(cache_line(&cpu, 1, CacheKind::Data), ("16 x 32 KBytes".into(), "8-way".into(), 2));
        assert_eq!(level3(&cpu), [(96, 1, 16)]);

        let mut sample = stacked_sample();
        sample.thread_dumps.drain(..24);
        let cpu = interpret(sample);
        assert_eq!(cache_line(&cpu, 2, CacheKind::Unified), ("16 x 1 MBytes".into(), "8-way".into(), 2));
        assert_eq!(level3(&cpu), [(32, 1, 16)]);
    }

    #[test]
    fn partial_sampling_counts_caches_among_the_sampled_core_type() {
        // Only the E-cores could be pinned: the list is theirs, with their own counts.
        let mut sample = hybrid_sample();
        sample.thread_dumps.drain(..16);
        let cpu = interpret(sample);
        assert_eq!(cpu.core_sets[0], CoreSet { kind: CoreKind::Performance, cores: 8, threads: 16 });
        assert_eq!(cache_line(&cpu, 1, CacheKind::Data), ("16 x 32 KBytes".into(), "8-way".into(), 1));
        assert_eq!(cache_line(&cpu, 1, CacheKind::Instruction), ("16 x 64 KBytes".into(), "8-way".into(), 1));
        assert_eq!(cache_line(&cpu, 2, CacheKind::Unified), ("4 x 4 MBytes".into(), "16-way".into(), 4));
        assert_eq!(cache_line(&cpu, 3, CacheKind::Unified), ("36 MBytes".into(), "12-way".into(), 32));

        // With the P-cores sampled the list is that of the displayed type, as usual.
        let mut sample = hybrid_sample();
        sample.thread_dumps.truncate(16);
        let cpu = interpret(sample);
        assert_eq!(cache_line(&cpu, 1, CacheKind::Data), ("8 x 48 KBytes".into(), "12-way".into(), 2));
        assert_eq!(cache_line(&cpu, 1, CacheKind::Instruction), ("8 x 32 KBytes".into(), "8-way".into(), 2));
        assert_eq!(cache_line(&cpu, 2, CacheKind::Unified), ("8 x 2 MBytes".into(), "16-way".into(), 2));
        assert_eq!(cache_line(&cpu, 3, CacheKind::Unified), ("36 MBytes".into(), "12-way".into(), 32));
    }

    #[test]
    fn degrades_without_a_topology() {
        let mut sample = zen4_sample();
        sample.topology = OsTopology::default();
        sample.thread_dumps.truncate(1);
        sample.firmware_mhz = None;
        let cpu = interpret(sample);
        assert_eq!(cpu.name, "AMD Ryzen 7 7700");
        assert_eq!((cpu.cores, cpu.threads, cpu.sockets), (0, 0, 0));
        assert!(cpu.logical.is_empty() && cpu.core_sets.is_empty());
        // The caches are still described, each counted once.
        assert_eq!(cache_line(&cpu, 2, CacheKind::Unified), ("1 MBytes".into(), "8-way".into(), 0));
        assert_eq!(cpu.nominal_mhz, None);
    }

    #[test]
    fn an_empty_sample_is_an_empty_model() {
        let sample = Sample {
            topology: OsTopology::default(),
            thread_dumps: Vec::new(),
            scheduling_classes: Vec::new(),
            xcr0: None,
            microcode: None,
            firmware_mhz: None,
        };
        let cpu = interpret(sample);
        assert_eq!(cpu.vendor, CpuVendor::Other);
        assert!(cpu.name.is_empty() && cpu.instructions.is_empty() && cpu.caches.is_empty());
        assert_eq!((cpu.codename, cpu.package, cpu.revision, cpu.max_tdp_w), (None, None, None, None));
    }

    #[test]
    fn nominal_frequency_sources_are_ranked() {
        let leaf_16 = hybrid(0, true);
        assert_eq!(nominal_mhz(Leaves(&leaf_16), "CPU @ 3.40GHz", Some(3408)), Some(3000.0));
        assert_eq!(nominal_mhz(Leaves(&zen4(0)), "CPU @ 3.40GHz", Some(3408)), Some(3400.0));
        assert_eq!(nominal_mhz(Leaves(&zen4(0)), "CPU", Some(3408)), Some(3408.0));
        assert_eq!(nominal_mhz(Leaves(&[]), "CPU", None), None);
    }

    #[test]
    fn reads_microcode_revisions() {
        // AMD, four bytes; AMD, eight bytes; Intel, revision in the high half.
        assert_eq!(microcode_revision(&[0x03, 0x12, 0x60, 0x0A]), Some(0x0A60_1203));
        assert_eq!(microcode_revision(&[0x03, 0x12, 0x60, 0x0A, 0, 0, 0, 0]), Some(0x0A60_1203));
        assert_eq!(microcode_revision(&[0, 0, 0, 0, 0xF0, 0, 0, 0]), Some(0xF0));
        assert_eq!(microcode_revision(&[0; 8]), None);
        assert_eq!(microcode_revision(&[1, 2]), None);
        assert_eq!(microcode_revision(&[]), None);
    }

    #[test]
    fn detects_the_running_machine_consistently() {
        // Hardware-independent invariants of a live detection.
        let cpu = detect();
        assert_eq!(cpu.vendor_string.len(), 12);
        assert!(!cpu.thread_dumps.is_empty());
        assert!(cpu.thread_dumps.len() <= cpu.logical.len().max(1));
        assert_eq!(cpu.threads, cpu.core_sets.iter().map(|s| s.threads).sum::<u32>());
        assert!(cpu.cores <= cpu.threads);
        assert!(cpu.logical.iter().zip(0..).all(|(lp, index)| lp.index == index));
        assert!(cpu.instructions.iter().all(|i| !i.is_empty()));
        assert!(cpu.caches.iter().all(|c| c.count >= 1 && c.size_kb > 0));
    }
}
