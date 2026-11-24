//! Processor sections: groups, the APIC tree, timers, the socket block and the CPUID dumps.

use std::collections::{BTreeMap, BTreeSet};

use crate::hw::cpu::thread_caches;
use crate::model::{ClockSnapshot, CpuInfo, CpuVendor, CpuidLeaf, LogicalProcessor, TimerFrequency};

use super::doc::{Lines, Section};
use super::fmt;

/// CPUID leaves that describe the APIC ID layout, most detailed first.
const TOPOLOGY_LEAVES: [u32; 2] = [0x1F, 0x0B];
/// Level type of the SMT level in a topology leaf (ECX bits 15-8).
const LEVEL_SMT: u32 = 1;

/// AMD leaf with one sub-leaf per level of the package, from the core up to the socket.
const AMD_TOPOLOGY_LEAF: u32 = 0x8000_0026;
/// AMD leaf whose ECX holds the node of the logical processor in its low byte.
const AMD_NODE_LEAF: u32 = 0x8000_001E;
/// Level types of the AMD topology leaf (ECX bits 15-8): core, complex, die and socket.
const AMD_LEVELS: [u32; 4] = [1, 2, 3, 4];

/// "Processors": the processor groups with their affinity masks, then the socket and thread counts.
pub(super) fn processors(cpu: &CpuInfo) -> Section {
    let mut out = Lines::default();
    let groups = group_masks(&cpu.logical);
    if !groups.is_empty() {
        out.pair(0, "CPU Groups", groups.len().to_string());
        for (group, (threads, mask)) in &groups {
            out.pair(0, &format!("CPU Group {group}"), format!("{threads} threads, mask=0x{mask:X}"));
        }
        out.blank();
    }
    let threads = if cpu.logical.is_empty() { cpu.threads } else { cpu.logical.len() as u32 };
    out.opt(0, "Number of sockets", nonzero(cpu.sockets));
    out.opt(0, "Number of threads", nonzero(threads));
    out.into_section("Processors")
}

/// "APICs": the socket / core / thread tree with the APIC ID of every thread.
/// Core IDs are shown when a CPUID dump describes the APIC ID layout. An AMD
/// processor whose threads all state where they sit gets the node, CCD and CCX
/// levels between the socket and its cores.
pub(super) fn apics(cpu: &CpuInfo) -> Section {
    let layout = ApicLayout::of(cpu);
    let places = amd_places(cpu);
    let nested = cpu.logical.iter().all(|lp| places.contains_key(&lp.index));
    let place = |lp: &LogicalProcessor| places.get(&lp.index).filter(|_| nested);
    let mut threads: Vec<&LogicalProcessor> = cpu.logical.iter().collect();
    threads.sort_by_key(|lp| (lp.package, place(lp).map(|at| (at.node, at.ccd, at.ccx)), lp.core, lp.index));

    let mut out = Lines::default();
    // The labels from the socket down to the core of the thread printed last.
    let mut open: Vec<String> = Vec::new();
    for lp in threads {
        let mut branch = vec![format!("Socket {}", lp.package)];
        if let Some(at) = place(lp) {
            branch.extend([
                format!("Node {}", at.node),
                format!("CCD {}", at.ccd),
                format!("CCX {}", at.ccx),
            ]);
        }
        branch.push(match layout {
            Some(layout) => format!("{} {} (ID {})", lp.kind.label(), lp.core, layout.core_id(lp.apic_id)),
            None => format!("{} {}", lp.kind.label(), lp.core),
        });
        let shared = open.iter().zip(&branch).take_while(|(above, here)| above == here).count();
        for (depth, label) in (0u8..).zip(&branch).skip(shared) {
            out.tree(depth, label.clone(), String::new());
        }
        out.tree(branch.len() as u8, format!("Thread {}", lp.index), lp.apic_id.to_string());
        open = branch;
    }
    out.into_section("APICs")
}

/// "Timers": the frequency of each system time source.
pub(super) fn timers(timers: &[TimerFrequency]) -> Section {
    let mut out = Lines::default();
    for timer in timers {
        out.pair(1, &timer.name, fmt::frequency(timer.hz));
    }
    out.blank();
    out.into_section("Timers")
}

/// "Processors Information": identity, caches and clocks of the first socket,
/// the only one the model describes.
pub(super) fn information(cpu: &CpuInfo, clocks: Option<&ClockSnapshot>) -> Section {
    let details = socket_details(cpu, clocks);
    let mut out = Lines::default();
    if !details.is_empty() {
        let id = cpu.logical.first().map_or(0, |lp| lp.package);
        out.pair(0, "Socket 1", format!("ID = {id}"));
        out.append(details);
    }
    out.blank();
    out.into_section("Processors Information")
}

/// "Thread dumps": per dumped logical processor, a `CPU Thread (socket, APIC ID)`
/// heading, its topology, the caches it sees and its raw CPUID leaves.
pub(super) fn thread_dumps(cpu: &CpuInfo) -> Section {
    let layout = ApicLayout::of(cpu);
    let places = amd_places(cpu);
    let mut out = Lines::default();
    for dump in &cpu.thread_dumps {
        match cpu.logical.iter().find(|lp| lp.index == dump.processor) {
            Some(lp) => {
                let core = layout.map_or(lp.core, |layout| layout.core_id(lp.apic_id));
                out.heading(0, format!("CPU Thread ({}, 0x{:X})", lp.package, lp.apic_id), 1);
                out.pair(1, "APIC ID", lp.apic_id.to_string());
                out.pair(
                    1,
                    "Topology",
                    format!("Processor ID {}, Core ID {core}, Thread ID {}", lp.package, lp.smt),
                );
                out.opt(1, "AMD Topology", places.get(&lp.index).map(AmdPlace::text));
            }
            None => out.heading(0, format!("CPU Thread {}", dump.processor), 1),
        }
        for cache in thread_caches(&dump.leaves) {
            out.pair(1, "Cache descriptor", fmt::cache_descriptor(&cache));
        }
        out.blank();
        out.cpuid(&dump.leaves);
        out.blank();
        out.blank();
    }
    out.blank();
    out.into_section("Thread dumps")
}

/// The indented lines of the socket block; empty when nothing is known about the processor.
fn socket_details(cpu: &CpuInfo, clocks: Option<&ClockSnapshot>) -> Lines {
    let identified = !cpu.vendor_string.is_empty();
    let mut out = Lines::default();
    out.opt(1, "Number of cores", nonzero(cpu.cores).map(|cores| count_text(cores, cpu.max_cores)));
    out.opt(1, "Number of threads", nonzero(cpu.threads).map(|threads| count_text(threads, cpu.max_threads)));
    if cpu.is_hybrid() {
        out.pair(1, "Hybrid", format!("yes, {} coresets", cpu.core_sets.len()));
        for (n, set) in cpu.core_sets.iter().enumerate() {
            let text = format!("{}s, {} cores, {} threads", set.kind.label(), set.cores, set.threads);
            out.pair(1, &format!("Core Set {n}"), text);
        }
    }
    out.opt(1, "Number of CCDs", ccd_count(cpu));
    out.pair(1, "Manufacturer", &cpu.vendor_string);
    out.pair(1, "Name", &cpu.name);
    out.opt(1, "Codename", cpu.codename.as_deref());
    out.pair(1, "Specification", &cpu.specification);
    out.opt(1, "Package", cpu.package.as_deref());
    if identified {
        out.pair(1, "CPUID", format!("{:X}.{:X}.{:X}", cpu.family, cpu.model, cpu.stepping));
        out.pair(1, "Extended CPUID", format!("{:X}.{:X}", cpu.ext_family, cpu.ext_model));
    }
    out.opt(1, "Core Stepping", cpu.revision.as_deref());
    out.opt(1, "Technology", cpu.technology_nm.map(|nm| format!("{nm} nm")));
    out.opt(1, "TDP Limit", cpu.max_tdp_w.map(|watts| format!("{watts:.1} Watts")));
    if let Some(clocks) = clocks {
        clock_summary(&mut out, clocks);
    }
    out.opt(1, "Stock frequency", cpu.nominal_mhz.map(|mhz| format!("{mhz:.0} MHz")));
    out.pair(1, "Instructions sets", cpu.instructions.join(", "));
    out.opt(1, "Microcode Revision", cpu.microcode.map(|revision| format!("0x{revision:X}")));
    // Caches of one level that differ in size share a line.
    for caches in cpu.caches.chunk_by(|a, b| (a.level, a.kind) == (b.level, b.kind)) {
        out.pair(1, &fmt::cache_label(&caches[0]), fmt::cache_text(caches));
    }
    out.opt(1, "Preferred cores", preferred_text(&cpu.preferred_cores));
    if identified {
        out.pair(1, "Max CPUID level", format!("{:08X}h", cpu.max_cpuid_leaf));
        out.pair(1, "Max CPUID ext. level", format!("{:08X}h", cpu.max_ext_cpuid_leaf));
    }
    if let Some(clocks) = clocks {
        clock_speeds(&mut out, clocks);
    }
    out
}

/// Speed of the first core, its multiplier and the bus speed, the multiplier range, then the
/// bus clock on its own.
fn clock_summary(out: &mut Lines, clocks: &ClockSnapshot) {
    let megahertz = |mhz: f64| format!("{mhz:.1} MHz");
    if let Some(core) = clocks.cores.first() {
        out.pair(1, "Core Speed", megahertz(core.mhz));
        if let (Some(multiplier), Some(bus)) = (core.multiplier, clocks.bus_mhz) {
            out.pair(1, "Multiplier x Bus Speed", format!("{multiplier:.1} x {bus:.1} MHz"));
        }
    }
    // Only the processor's own registers state the lowest multiplier.
    if let (Some(lowest), Some(highest)) = (clocks.multiplier_min, clocks.multiplier_max) {
        out.pair(1, "Multiplier range", format!("{lowest:.1} - {highest:.1}"));
    }
    out.opt(1, "Base frequency (cores)", clocks.bus_mhz.map(megahertz));
    out.opt(1, "Rated Bus speed", clocks.rated_fsb_mhz.map(megahertz));
}

/// One `Clock Speed n` line per core, then the bus clock.
fn clock_speeds(out: &mut Lines, clocks: &ClockSnapshot) {
    for (n, core) in clocks.cores.iter().enumerate() {
        let text = format!("{:.2} MHz ({} #{})", core.mhz, core.kind.label(), core.core);
        out.pair(1, &format!("Clock Speed {n}"), text);
    }
    if let Some(bus) = clocks.bus_mhz {
        out.pair(1, &format!("Clock Speed {}", clocks.cores.len()), format!("{bus:.2} MHz (CPU BCLK)"));
    }
}

fn nonzero(count: u32) -> Option<u32> {
    (count > 0).then_some(count)
}

/// `8 (max 8)` - a count and, when it is known, the most the package can hold.
fn count_text(count: u32, max: Option<u32>) -> String {
    match max {
        Some(max) => format!("{count} (max {max})"),
        None => count.to_string(),
    }
}

/// `2 (#0, #1)` - how many cores are preferred, and which. `None` when none is.
fn preferred_text(cores: &[u32]) -> Option<String> {
    let list: Vec<String> = cores.iter().map(|core| format!("#{core}")).collect();
    (!list.is_empty()).then(|| format!("{} ({})", list.len(), list.join(", ")))
}

/// Where a logical processor sits inside an AMD package: its node, its die
/// (CCD), its core complex (CCX), its core inside the complex and its thread
/// inside the core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AmdPlace {
    node: u32,
    ccd: u32,
    ccx: u32,
    core: u32,
    thread: u32,
}

impl AmdPlace {
    /// Splits `apic_id` along the levels the thread's own topology leaf lists. EAX
    /// bits 4-0 of a level hold the shift that turns an APIC ID into the ID of
    /// that level, so what tells its instances apart inside the level above is
    /// the field from its own shift up to the next one. `None` when the
    /// processor does not describe all four levels, or not its node.
    fn from_leaves(leaves: &[CpuidLeaf], apic_id: u32) -> Option<Self> {
        let shift_past = |level: u32| {
            let found = leaves.iter().find(|l| l.leaf == AMD_TOPOLOGY_LEAF && (l.ecx >> 8) & 0xFF == level);
            found.map(|l| l.eax & 0x1F)
        };
        let [core, complex, die, socket] = AMD_LEVELS.map(shift_past);
        let (core, complex, die, socket) = (core?, complex?, die?, socket?);
        if core > complex || complex > die || die > socket {
            return None;
        }
        let node = leaves.iter().find(|l| l.leaf == AMD_NODE_LEAF)?.ecx & 0xFF;
        let field = |low: u32, high: u32| (apic_id >> low) & ((1u32 << (high - low)) - 1);
        Some(AmdPlace {
            node,
            ccd: field(die, socket),
            ccx: field(complex, die),
            core: field(core, complex),
            thread: field(0, core),
        })
    }

    /// `Node 0, CCD 0, CCX 0, core 3, thread 1`.
    fn text(&self) -> String {
        let AmdPlace { node, ccd, ccx, core, thread } = self;
        format!("Node {node}, CCD {ccd}, CCX {ccx}, core {core}, thread {thread}")
    }
}

/// The place of every dumped logical processor of an AMD or Hygon processor, by processor index.
fn amd_places(cpu: &CpuInfo) -> BTreeMap<u32, AmdPlace> {
    if !matches!(cpu.vendor, CpuVendor::Amd | CpuVendor::Hygon) {
        return BTreeMap::new();
    }
    let place = |lp: &LogicalProcessor| {
        let dump = cpu.thread_dumps.iter().find(|dump| dump.processor == lp.index)?;
        Some((lp.index, AmdPlace::from_leaves(&dump.leaves, lp.apic_id)?))
    };
    cpu.logical.iter().filter_map(place).collect()
}

/// Number of dies (CCDs) in the first package. `None` unless every thread of it states its die.
fn ccd_count(cpu: &CpuInfo) -> Option<usize> {
    let places = amd_places(cpu);
    let dies: Option<BTreeSet<u32>> = cpu
        .logical
        .iter()
        .filter(|lp| lp.package == 0)
        .map(|lp| places.get(&lp.index).map(|place| place.ccd))
        .collect();
    dies.map(|dies| dies.len()).filter(|&count| count > 0)
}

/// Thread count and affinity mask of each processor group.
fn group_masks(logical: &[LogicalProcessor]) -> BTreeMap<u16, (u32, u64)> {
    let mut groups: BTreeMap<u16, (u32, u64)> = BTreeMap::new();
    for lp in logical {
        let (threads, mask) = groups.entry(lp.group).or_default();
        *threads += 1;
        *mask |= 1u64.checked_shl(u32::from(lp.number)).unwrap_or(0);
    }
    groups
}

/// Bit positions that split an APIC ID into its thread, core and package fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ApicLayout {
    smt_shift: u32,
    package_shift: u32,
}

impl ApicLayout {
    /// The layout reported by the first CPUID dump that carries a topology leaf.
    fn of(cpu: &CpuInfo) -> Option<Self> {
        cpu.thread_dumps.iter().find_map(|dump| Self::from_leaves(&dump.leaves))
    }

    fn from_leaves(leaves: &[CpuidLeaf]) -> Option<Self> {
        TOPOLOGY_LEAVES.into_iter().find_map(|leaf| Self::from_leaf(leaves, leaf))
    }

    /// Reads the shifts from the sub-leaves of one topology leaf: EAX bits 4-0 hold
    /// the shift past a level and ECX bits 15-8 its type (0 marks an unused sub-leaf).
    fn from_leaf(leaves: &[CpuidLeaf], leaf: u32) -> Option<Self> {
        let level_type = |l: &CpuidLeaf| (l.ecx >> 8) & 0xFF;
        let shift = |l: &CpuidLeaf| l.eax & 0x1F;
        let levels = || leaves.iter().filter(|l| l.leaf == leaf && level_type(l) != 0);
        let smt_shift = levels().find(|l| level_type(l) == LEVEL_SMT).map(shift)?;
        let package_shift = levels().map(shift).max()?;
        (package_shift > smt_shift).then_some(ApicLayout { smt_shift, package_shift })
    }

    /// The core's ID inside its package.
    fn core_id(self, apic_id: u32) -> u32 {
        let core_bits = self.package_shift - self.smt_shift;
        (apic_id >> self.smt_shift) & ((1u32 << core_bits) - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use crate::model::{CacheInfo, CoreKind, CoreSet, ThreadDump};
    use crate::report::txt::lines_of;

    fn leaf(leaf: u32, subleaf: u32, eax: u32, ecx: u32) -> CpuidLeaf {
        CpuidLeaf { leaf, subleaf, eax, ebx: 0, ecx, edx: 0 }
    }

    fn logical(index: u32, group: u16, number: u8) -> LogicalProcessor {
        LogicalProcessor { index, group, number, apic_id: index, core: index, ..Default::default() }
    }

    /// The demo processor with the given CPUID leaves missing from every dump.
    fn demo_without(leaves: &[u32]) -> CpuInfo {
        let mut cpu = demo::system().cpu;
        for dump in &mut cpu.thread_dumps {
            dump.leaves.retain(|l| !leaves.contains(&l.leaf));
        }
        cpu
    }

    /// An AMD package of two dies with two single-thread cores each, at the given APIC IDs.
    fn two_dies(apic_ids: [u32; 4]) -> CpuInfo {
        let dump = |(processor, apic_id): (u32, u32)| ThreadDump {
            processor,
            leaves: vec![
                leaf(AMD_NODE_LEAF, 0, apic_id, 0),
                leaf(AMD_TOPOLOGY_LEAF, 0, 0, 0x0100),
                leaf(AMD_TOPOLOGY_LEAF, 1, 4, 0x0201),
                leaf(AMD_TOPOLOGY_LEAF, 2, 4, 0x0302),
                leaf(AMD_TOPOLOGY_LEAF, 3, 5, 0x0403),
            ],
        };
        let lp = |(index, apic_id): (u32, u32)| LogicalProcessor {
            index,
            number: index as u8,
            apic_id,
            core: index,
            ..Default::default()
        };
        CpuInfo {
            vendor: CpuVendor::Amd,
            cores: 4,
            threads: 4,
            logical: (0..).zip(apic_ids).map(lp).collect(),
            thread_dumps: (0..).zip(apic_ids).map(dump).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn counts_groups_and_masks() {
        let cpu = demo::system().cpu;
        assert_eq!(
            lines_of(&processors(&cpu)),
            [
                "CPU Groups\t\t\t1",
                "CPU Group 0\t\t\t16 threads, mask=0xFFFF",
                "",
                "Number of sockets\t\t1",
                "Number of threads\t\t16",
            ]
        );
    }

    #[test]
    fn handles_more_than_one_processor_group() {
        let logical: Vec<_> = (0..64)
            .map(|i| logical(i, 0, i as u8))
            .chain((0..8).map(|i| logical(64 + i, 1, i as u8)))
            .collect();
        let groups = group_masks(&logical);
        assert_eq!(groups[&0], (64, u64::MAX));
        assert_eq!(groups[&1], (8, 0xFF));

        let cpu = CpuInfo { sockets: 2, threads: 36, logical, ..Default::default() };
        let lines = lines_of(&processors(&cpu));
        assert_eq!(lines[0], "CPU Groups\t\t\t2");
        assert_eq!(lines[1], "CPU Group 0\t\t\t64 threads, mask=0xFFFFFFFFFFFFFFFF");
        assert_eq!(lines[2], "CPU Group 1\t\t\t8 threads, mask=0xFF");
        assert_eq!(lines[5], "Number of threads\t\t72");
    }

    #[test]
    fn ignores_out_of_range_processor_numbers() {
        assert_eq!(group_masks(&[logical(0, 0, 200)])[&0], (1, 0));
    }

    #[test]
    fn reads_the_apic_layout() {
        let amd = [leaf(0x0B, 0, 1, 0x0100), leaf(0x0B, 1, 4, 0x0201), leaf(0x0B, 2, 0, 0x0002)];
        let layout = ApicLayout::from_leaves(&amd).unwrap();
        assert_eq!(layout, ApicLayout { smt_shift: 1, package_shift: 4 });
        assert_eq!(layout.core_id(0x0), 0);
        assert_eq!(layout.core_id(0x3), 1);
        assert_eq!(layout.core_id(0xF), 7);
        // The package bits above the core field are masked off.
        assert_eq!(layout.core_id(0x1F), 7);
    }

    #[test]
    fn prefers_the_v2_topology_leaf() {
        let hybrid = [
            leaf(0x0B, 0, 1, 0x0100),
            leaf(0x0B, 1, 6, 0x0201),
            leaf(0x1F, 0, 1, 0x0100),
            leaf(0x1F, 1, 7, 0x0201),
        ];
        let layout = ApicLayout::from_leaves(&hybrid).unwrap();
        assert_eq!(layout, ApicLayout { smt_shift: 1, package_shift: 7 });
        assert_eq!(layout.core_id(0x48), 0x24);
    }

    #[test]
    fn rejects_dumps_without_topology() {
        assert_eq!(ApicLayout::from_leaves(&[leaf(0, 0, 0x0D, 0), leaf(1, 0, 0, 0)]), None);
        // All-zero sub-leaves, as returned by processors without the leaf.
        assert_eq!(ApicLayout::from_leaves(&[leaf(0x0B, 0, 0, 0), leaf(0x0B, 1, 0, 1)]), None);
        // An SMT level alone leaves no core field to extract.
        assert_eq!(ApicLayout::from_leaves(&[leaf(0x0B, 0, 1, 0x0100)]), None);
    }

    #[test]
    fn draws_the_apic_tree() {
        let lines = lines_of(&apics(&demo_without(&[AMD_TOPOLOGY_LEAF])));
        assert_eq!(lines.len(), 1 + 8 * 3);
        assert_eq!(lines[0], "Socket 0\t");
        assert_eq!(lines[1], "\t-- Core 0 (ID 0)\t");
        assert_eq!(lines[2], "\t\t-- Thread 0\t0");
        assert_eq!(lines[3], "\t\t-- Thread 1\t1");
        assert_eq!(lines[22], "\t-- Core 7 (ID 7)\t");
        assert_eq!(lines[24], "\t\t-- Thread 15\t15");
    }

    #[test]
    fn omits_core_ids_without_a_topology_leaf() {
        let lines = lines_of(&apics(&demo_without(&[0x0B, AMD_TOPOLOGY_LEAF])));
        assert_eq!(lines[1], "\t-- Core 0\t");
        assert_eq!(lines[4], "\t-- Core 1\t");
    }

    #[test]
    fn nests_the_levels_of_an_amd_package() {
        let lines = lines_of(&apics(&demo::system().cpu));
        assert_eq!(lines.len(), 4 + 8 * 3);
        assert_eq!(
            lines[..7],
            [
                "Socket 0\t",
                "\t-- Node 0\t",
                "\t\t-- CCD 0\t",
                "\t\t\t-- CCX 0\t",
                "\t\t\t\t-- Core 0 (ID 0)\t",
                "\t\t\t\t\t-- Thread 0\t0",
                "\t\t\t\t\t-- Thread 1\t1",
            ]
        );
        assert_eq!(lines[25], "\t\t\t\t-- Core 7 (ID 7)\t");
        assert_eq!(lines[27], "\t\t\t\t\t-- Thread 15\t15");
    }

    #[test]
    fn opens_a_branch_per_die() {
        assert_eq!(
            lines_of(&apics(&two_dies([0, 1, 16, 17]))),
            [
                "Socket 0\t",
                "\t-- Node 0\t",
                "\t\t-- CCD 0\t",
                "\t\t\t-- CCX 0\t",
                "\t\t\t\t-- Core 0\t",
                "\t\t\t\t\t-- Thread 0\t0",
                "\t\t\t\t-- Core 1\t",
                "\t\t\t\t\t-- Thread 1\t1",
                "\t\t-- CCD 1\t",
                "\t\t\t-- CCX 0\t",
                "\t\t\t\t-- Core 2\t",
                "\t\t\t\t\t-- Thread 2\t16",
                "\t\t\t\t-- Core 3\t",
                "\t\t\t\t\t-- Thread 3\t17",
            ]
        );
    }

    #[test]
    fn keeps_the_tree_flat_unless_every_thread_is_placed() {
        // The last thread could not be sampled.
        let mut cpu = two_dies([0, 1, 16, 17]);
        cpu.thread_dumps.pop();
        let lines = lines_of(&apics(&cpu));
        assert_eq!(lines.len(), 1 + 4 * 2);
        assert_eq!(lines[..3], ["Socket 0\t", "\t-- Core 0\t", "\t\t-- Thread 0\t0"]);
        assert_eq!(ccd_count(&cpu), None);
        // The levels are an AMD notion: the same leaves on another vendor change nothing.
        let mut cpu = two_dies([0, 1, 16, 17]);
        cpu.vendor = CpuVendor::Intel;
        assert_eq!(lines_of(&apics(&cpu)).len(), 1 + 4 * 2);
        assert_eq!(ccd_count(&cpu), None);
    }

    #[test]
    fn places_threads_inside_an_amd_package() {
        let cpu = demo::system().cpu;
        let place = |index: usize| AmdPlace::from_leaves(&cpu.thread_dumps[index].leaves, index as u32);
        assert_eq!(place(0), Some(AmdPlace { node: 0, ccd: 0, ccx: 0, core: 0, thread: 0 }));
        assert_eq!(place(11), Some(AmdPlace { node: 0, ccd: 0, ccx: 0, core: 5, thread: 1 }));
        assert_eq!(place(11).unwrap().text(), "Node 0, CCD 0, CCX 0, core 5, thread 1");
        assert_eq!(ccd_count(&cpu), Some(1));

        let cpu = two_dies([0, 1, 16, 17]);
        let places = amd_places(&cpu);
        assert_eq!(places[&3], AmdPlace { node: 0, ccd: 1, ccx: 0, core: 1, thread: 0 });
        assert_eq!(ccd_count(&cpu), Some(2));
    }

    #[test]
    fn needs_every_level_to_place_a_thread() {
        let levels = [
            leaf(AMD_NODE_LEAF, 0, 0, 3),
            leaf(AMD_TOPOLOGY_LEAF, 0, 1, 0x0100),
            leaf(AMD_TOPOLOGY_LEAF, 1, 4, 0x0201),
            leaf(AMD_TOPOLOGY_LEAF, 2, 5, 0x0302),
            leaf(AMD_TOPOLOGY_LEAF, 3, 7, 0x0403),
        ];
        // Two complexes of eight two-thread cores per die: APIC ID 0b10_1_011_1.
        assert_eq!(
            AmdPlace::from_leaves(&levels, 0x57),
            Some(AmdPlace { node: 3, ccd: 2, ccx: 1, core: 3, thread: 1 })
        );
        // Without the node leaf, without the die level, and with shifts that run backwards.
        assert_eq!(AmdPlace::from_leaves(&levels[1..], 0x57), None);
        let no_die: Vec<_> = levels.iter().copied().filter(|l| l.ecx != 0x0302).collect();
        assert_eq!(AmdPlace::from_leaves(&no_die, 0x57), None);
        let mut backwards = levels;
        backwards[3].eax = 2;
        assert_eq!(AmdPlace::from_leaves(&backwards, 0x57), None);
        assert_eq!(AmdPlace::from_leaves(&[], 0), None);
    }

    #[test]
    fn labels_hybrid_cores() {
        let lp = |index, apic_id, core, kind| LogicalProcessor {
            index,
            apic_id,
            core,
            kind,
            ..Default::default()
        };
        let cpu = CpuInfo {
            logical: vec![
                lp(0, 0, 0, CoreKind::Performance),
                lp(1, 1, 0, CoreKind::Performance),
                lp(2, 16, 1, CoreKind::Efficient),
            ],
            ..Default::default()
        };
        assert_eq!(
            lines_of(&apics(&cpu)),
            [
                "Socket 0\t",
                "\t-- P-Core 0\t",
                "\t\t-- Thread 0\t0",
                "\t\t-- Thread 1\t1",
                "\t-- E-Core 1\t",
                "\t\t-- Thread 2\t16",
            ]
        );
    }

    #[test]
    fn formats_timers() {
        assert_eq!(
            lines_of(&timers(&demo::system().timers)),
            ["\tACPI timer\t\t3.580 MHz", "\tPerf timer\t\t10.000 MHz", "\tSys timer\t\t1.000 KHz", ""]
        );
    }

    #[test]
    fn describes_the_socket() {
        let lines = lines_of(&information(&demo::system().cpu, Some(&demo::clocks())));
        let expected = [
            "Socket 1\t\t\tID = 0",
            "\tNumber of cores\t\t8 (max 8)",
            "\tNumber of threads\t16 (max 16)",
            "\tNumber of CCDs\t\t1",
            "\tManufacturer\t\tAuthenticAMD",
            "\tName\t\t\tAMD Ryzen 7 7700",
            "\tCodename\t\tRaphael",
            "\tSpecification\t\tAMD Ryzen 7 7700 8-Core Processor",
            "\tPackage\t\t\tSocket AM5 (LGA1718)",
            "\tCPUID\t\t\tF.1.2",
            "\tExtended CPUID\t\t19.61",
            "\tCore Stepping\t\tRPL-B2",
            "\tTechnology\t\t5 nm",
            "\tTDP Limit\t\t65.0 Watts",
            "\tCore Speed\t\t5136.6 MHz",
            "\tMultiplier x Bus Speed\t51.5 x 99.7 MHz",
            "\tMultiplier range\t4.0 - 53.5",
            "\tBase frequency (cores)\t99.7 MHz",
            "\tStock frequency\t\t3800 MHz",
        ];
        assert_eq!(lines[..expected.len()], expected);
        assert!(lines[19].starts_with("\tInstructions sets\tMMX (+), SSE, SSE2, "));
        assert!(lines[19].ends_with(", FMA3, SHA"));
        let rest = [
            "\tMicrocode Revision\t0xA601203",
            "\tL1 Data cache\t\t8 x 32 KB (8-way, 64-byte line)",
            "\tL1 Instruction cache\t8 x 32 KB (8-way, 64-byte line)",
            "\tL2 cache\t\t8 x 1024 KB (8-way, 64-byte line)",
            "\tL3 cache\t\t32 MB (16-way, 64-byte line)",
            "\tPreferred cores\t\t2 (#0, #1)",
            "\tMax CPUID level\t\t0000000Dh",
            "\tMax CPUID ext. level\t80000027h",
            "\tClock Speed 0\t\t5136.63 MHz (Core #0)",
        ];
        assert_eq!(lines[20..20 + rest.len()], rest);
        assert_eq!(lines[lines.len() - 3], "\tClock Speed 7\t\t5147.97 MHz (Core #7)");
        assert_eq!(lines[lines.len() - 2], "\tClock Speed 8\t\t99.74 MHz (CPU BCLK)");
        assert_eq!(lines[lines.len() - 1], "");
    }

    #[test]
    fn caches_of_different_size_share_a_line() {
        // A second core complex under a larger, stacked L3.
        let mut cpu = demo::system().cpu;
        let plain = cpu.caches[3];
        cpu.caches.insert(3, CacheInfo { size_kb: 96 * 1024, ..plain });
        let lines = lines_of(&information(&cpu, None));
        let level3: Vec<_> = lines.iter().filter(|line| line.contains("L3 cache")).collect();
        assert_eq!(level3, ["\tL3 cache\t\t96 MB + 32 MB (16-way, 64-byte line)"]);
        assert!(lines.contains(&"\tL2 cache\t\t8 x 1024 KB (8-way, 64-byte line)".to_string()));
    }

    #[test]
    fn leaves_clocks_out_without_a_snapshot() {
        let lines = lines_of(&information(&demo::system().cpu, None));
        assert!(!lines.iter().any(|l| l.contains("Core Speed") || l.contains("Clock Speed")));
        assert!(lines.contains(&"\tStock frequency\t\t3800 MHz".to_string()));
    }

    #[test]
    fn lists_core_sets_on_hybrid_parts() {
        let cpu = CpuInfo {
            cores: 14,
            threads: 20,
            core_sets: vec![
                CoreSet { kind: CoreKind::Performance, cores: 6, threads: 12 },
                CoreSet { kind: CoreKind::Efficient, cores: 8, threads: 8 },
            ],
            ..Default::default()
        };
        assert_eq!(
            lines_of(&information(&cpu, None)),
            [
                "Socket 1\t\t\tID = 0",
                "\tNumber of cores\t\t14",
                "\tNumber of threads\t20",
                "\tHybrid\t\t\tyes, 2 coresets",
                "\tCore Set 0\t\tP-Cores, 6 cores, 12 threads",
                "\tCore Set 1\t\tE-Cores, 8 cores, 8 threads",
                "",
            ]
        );
    }

    #[test]
    fn states_the_capacity_of_the_package_when_known() {
        assert_eq!(count_text(4, Some(8)), "4 (max 8)");
        assert_eq!(count_text(6, None), "6");
        let cpu =
            CpuInfo { cores: 4, threads: 8, max_cores: Some(8), max_threads: None, ..Default::default() };
        assert_eq!(
            lines_of(&information(&cpu, None)),
            ["Socket 1\t\t\tID = 0", "\tNumber of cores\t\t4 (max 8)", "\tNumber of threads\t8", ""]
        );
    }

    #[test]
    fn lists_the_preferred_cores() {
        assert_eq!(preferred_text(&[0, 1]).as_deref(), Some("2 (#0, #1)"));
        assert_eq!(preferred_text(&[5]).as_deref(), Some("1 (#5)"));
        assert_eq!(preferred_text(&[]), None);
        let cpu = CpuInfo { preferred_cores: Vec::new(), ..demo::system().cpu };
        assert!(!lines_of(&information(&cpu, None)).iter().any(|line| line.contains("Preferred cores")));
    }

    #[test]
    fn states_the_bus_clock_without_a_multiplier() {
        let clocks = ClockSnapshot { bus_mhz: Some(100.02), ..Default::default() };
        let cpu = CpuInfo { cores: 2, ..Default::default() };
        assert_eq!(
            lines_of(&information(&cpu, Some(&clocks)))[1..],
            [
                "\tNumber of cores\t\t2",
                "\tBase frequency (cores)\t100.0 MHz",
                "\tClock Speed 0\t\t100.02 MHz (CPU BCLK)",
                ""
            ]
        );
    }

    #[test]
    fn states_the_multiplier_range_only_with_both_ends() {
        let range = |clocks: &ClockSnapshot| {
            let lines = lines_of(&information(&CpuInfo { cores: 2, ..Default::default() }, Some(clocks)));
            lines.into_iter().find(|line| line.contains("Multiplier range"))
        };
        let clocks =
            ClockSnapshot { multiplier_min: Some(8.0), multiplier_max: Some(52.0), ..Default::default() };
        assert_eq!(range(&clocks).as_deref(), Some("\tMultiplier range\t8.0 - 52.0"));
        // User mode knows at most the highest.
        assert_eq!(range(&ClockSnapshot { multiplier_min: None, ..clocks }), None);
    }

    #[test]
    fn unknown_processor_prints_no_socket() {
        assert_eq!(lines_of(&information(&CpuInfo::default(), None)), [""]);
        assert!(lines_of(&processors(&CpuInfo::default())).is_empty());
    }

    #[test]
    fn dumps_cpuid_per_thread() {
        let lines = lines_of(&thread_dumps(&demo::system().cpu));
        assert_eq!(
            lines[..12],
            [
                "CPU Thread (0, 0x0)\t",
                "\tAPIC ID\t\t\t0",
                "\tTopology\t\tProcessor ID 0, Core ID 0, Thread ID 0",
                "\tAMD Topology\t\tNode 0, CCD 0, CCX 0, core 0, thread 0",
                "\tCache descriptor\tLevel 1, D, 32 KB, 2 thread(s)",
                "\tCache descriptor\tLevel 1, I, 32 KB, 2 thread(s)",
                "\tCache descriptor\tLevel 2, U, 1 MB, 2 thread(s)",
                "\tCache descriptor\tLevel 3, U, 32 MB, 16 thread(s)",
                "",
                "\tCPUID\t\t ",
                "\t0x00000000\t\t0x0000000D\t0x68747541\t0x444D4163\t0x69746E65",
                "\t0x00000001\t\t0x00A60F12\t0x00100800\t0xFED8320B\t0x178BFBFF",
            ]
        );
        assert_eq!(lines[lines.len() - 3..], ["", "", ""]);
        assert_eq!(lines.iter().filter(|line| line.starts_with("CPU Thread (0, 0x")).count(), 16);
    }

    #[test]
    fn dumps_without_cache_or_placement_leaves_keep_to_the_topology() {
        let cpu = demo_without(&[0x8000_001D, AMD_TOPOLOGY_LEAF]);
        let lines = lines_of(&thread_dumps(&cpu));
        assert_eq!(
            lines[..5],
            [
                "CPU Thread (0, 0x0)\t",
                "\tAPIC ID\t\t\t0",
                "\tTopology\t\tProcessor ID 0, Core ID 0, Thread ID 0",
                "",
                "\tCPUID\t\t ",
            ]
        );
    }

    #[test]
    fn dumps_threads_missing_from_the_topology() {
        let mut cpu = demo::system().cpu;
        cpu.thread_dumps = vec![ThreadDump { processor: 99, leaves: vec![leaf(0, 0, 1, 0)] }];
        let lines = lines_of(&thread_dumps(&cpu));
        assert_eq!(lines[..3], ["CPU Thread 99\t", "", "\tCPUID\t\t "]);
    }

    #[test]
    fn hex_ids_use_the_apic_id() {
        let lines = lines_of(&thread_dumps(&demo::system().cpu));
        let at =
            lines.iter().position(|line| line == "CPU Thread (0, 0xB)\t").expect("the dump of thread 11");
        assert_eq!(
            lines[at + 1..at + 4],
            [
                "\tAPIC ID\t\t\t11",
                "\tTopology\t\tProcessor ID 0, Core ID 5, Thread ID 1",
                "\tAMD Topology\t\tNode 0, CCD 0, CCX 0, core 5, thread 1",
            ]
        );
    }
}
