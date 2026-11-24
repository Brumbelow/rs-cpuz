//! Live clock sampling: per-core speed, multiplier and bus speed.
//!
//! Without a driver the ratio registers are out of reach, so the clocks are
//! derived from what user mode can observe:
//!
//! * core speed: the `% Processor Performance` counter of each logical
//!   processor (the frequency it delivered while busy, relative to nominal)
//!   scaled by the measured time-stamp counter frequency, which is the nominal
//!   frequency that counter refers to;
//! * bus speed: the time-stamp counter frequency divided by the ratio of the
//!   nominal frequency;
//! * multiplier: core speed over bus speed.
//!
//! When the performance counters are unavailable the core speed falls back to
//! what the power manager reports, which is coarse and capped at nominal.

mod perf;
mod power;
mod topology;

use std::collections::{BTreeMap, HashMap};
use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use crate::hw::cpu::cpuid;
use crate::hw::processors::{Pin, ProcId};
use crate::hw::smbios::{kind, Smbios};
use crate::hw::timers::tsc;
use crate::model::{ClockSnapshot, CoreClock, CoreKind, CpuInfo, CpuVendor, LogicalProcessor};

use perf::PerfQuery;
use topology::Layout;

/// Bus clock assumed when the firmware does not state a usable one.
const DEFAULT_REFERENCE_MHZ: f64 = 100.0;

/// External clocks outside this range are firmware placeholders.
const SANE_REFERENCE_MHZ: RangeInclusive<u16> = 50..=500;

/// An advertised frequency is only believed while it puts the bus clock this
/// close to its reference; mobile parts often run their counter at another
/// ratio than the one on the label.
const BUS_TOLERANCE: f64 = 0.05;

/// Time the first sample lets the performance counters run before reading them.
const FIRST_WINDOW: Duration = Duration::from_millis(100);

/// How long the first sample keeps a core busy that was too idle for a reading.
/// The counters need 4 ms of work; the rest is margin for the thread switch.
const WAKE_TIME: Duration = Duration::from_millis(5);

/// Most cores the first sample keeps busy that way, which bounds the time it takes.
const MAX_WAKES: usize = 64;

/// A core of the first package and the logical processors it runs.
#[derive(Debug, Clone, PartialEq)]
struct Core {
    index: u32,
    kind: CoreKind,
    threads: Vec<ProcId>,
    /// Latest measured speed in MHz; kept while the core sits idle.
    mhz: Option<f64>,
}

/// Speeds stated by the firmware for the first populated socket (SMBIOS type 4), in MHz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct FirmwareSpeeds {
    external_clock: Option<u16>,
    max: Option<u16>,
    current: Option<u16>,
}

/// What CPUID leaf 0x16 says, in MHz. A zero field is not reported by the processor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FrequencyLeaf {
    max: u32,
    bus: u32,
}

enum Perf {
    /// Not opened yet; the first sample does that.
    Pending,
    Ready(PerfQuery),
    Unavailable,
}

/// Samples the processor clocks. Create it once and call [`ClockMonitor::sample`] periodically.
pub struct ClockMonitor {
    cores: Vec<Core>,
    layout: Layout,
    perf: Perf,
    /// Whether the time-stamp counter can serve as a clock reference.
    invariant_tsc: bool,
    /// Candidates for the nominal frequency, most trusted first.
    advertised_mhz: Vec<f64>,
    reference_mhz: f64,
    /// Steps per unit of the nominal ratio.
    ratio_parts: f64,
    multiplier_min: Option<f64>,
    multiplier_max: Option<f64>,
}

impl ClockMonitor {
    /// Prepares the monitor without blocking. `cpu.logical` selects the cores;
    /// when it is empty every logical processor counts as a core.
    pub fn new(cpu: &CpuInfo, smbios: Option<&Smbios>) -> Self {
        let layout = Layout::detect();
        let mut cores = cores_from_logical(&cpu.logical);
        if cores.is_empty() {
            cores = cores_from_layout(&layout);
        }
        let firmware = smbios.map(firmware_speeds).unwrap_or_default();
        let reference_mhz = reference_clock(firmware);
        let advertised = [cpu.nominal_mhz, firmware.current.map(f64::from)];
        ClockMonitor {
            cores,
            layout,
            perf: Perf::Pending,
            invariant_tsc: tsc::is_invariant(),
            advertised_mhz: advertised.into_iter().flatten().filter(|mhz| *mhz > 0.0).collect(),
            reference_mhz,
            ratio_parts: ratio_parts(cpu.vendor),
            // The processor's own registers, read through the driver, state the range exactly.
            multiplier_min: cpu.ratio_range.map(|(lowest, _)| lowest),
            multiplier_max: cpu
                .ratio_range
                .map(|(_, highest)| highest)
                .or_else(|| max_multiplier(cpu.vendor, frequency_leaf(cpu), firmware, reference_mhz)),
        }
    }

    /// Returns the current clocks; one entry per core of the first package, in
    /// core order. A core nothing is known about is left out.
    ///
    /// A core's speed is the average over the time it was busy since its
    /// previous reading; an idle core keeps its last one.
    ///
    /// The first call blocks for about 0.35 s: it opens the performance
    /// counters (0.2 s), lets them run for 0.1 s and then keeps every core that
    /// was too idle for a reading busy for 5 ms. If nothing has measured the
    /// time-stamp counter yet, that adds another 50 ms. Later calls take a
    /// fraction of a millisecond.
    pub fn sample(&mut self) -> ClockSnapshot {
        let tsc_mhz = if self.invariant_tsc { tsc::frequency_hz().map(|hz| hz / 1e6) } else { None };
        // The counters are relative to the frequency the time-stamp counter runs at.
        let nominal_mhz = tsc_mhz.or_else(power::nominal_mhz).or(self.advertised_mhz.first().copied());
        if let Some(nominal_mhz) = nominal_mhz {
            if matches!(self.perf, Perf::Pending) {
                self.perf = PerfQuery::open().map_or(Perf::Unavailable, Perf::Ready);
                if matches!(self.perf, Perf::Ready(_)) {
                    self.warm_up(nominal_mhz);
                }
            } else {
                self.measure(nominal_mhz);
            }
        }
        let bus_mhz = tsc_mhz
            .map(|tsc| tsc / base_ratio(tsc, &self.advertised_mhz, self.reference_mhz, self.ratio_parts));
        ClockSnapshot {
            cores: core_clocks(&self.cores, &self.reported(), bus_mhz),
            bus_mhz,
            multiplier_min: self.multiplier_min,
            multiplier_max: self.multiplier_max,
            rated_fsb_mhz: None,
        }
    }

    /// Takes the first readings. A core that did too little in the first window
    /// is given a moment of work, so that its speed is measured rather than unknown.
    fn warm_up(&mut self, nominal_mhz: f64) {
        std::thread::sleep(FIRST_WINDOW);
        if !self.measure(nominal_mhz) {
            return;
        }
        let idle: Vec<ProcId> = self
            .cores
            .iter()
            .filter(|core| core.mhz.is_none())
            .filter_map(|core| core.threads.first().copied())
            .take(MAX_WAKES)
            .collect();
        if idle.is_empty() {
            return;
        }
        // A helper thread does the hopping, so the caller keeps its affinity.
        std::thread::scope(|scope| {
            let spawned =
                std::thread::Builder::new().spawn_scoped(scope, || idle.iter().copied().for_each(keep_busy));
            if let Ok(helper) = spawned {
                let _ = helper.join();
            }
        });
        self.measure(nominal_mhz);
    }

    /// Collects the performance counters and updates the cores that have a
    /// reading due. Returns whether the counters could be read and matched to processors.
    fn measure(&mut self, nominal_mhz: f64) -> bool {
        let Perf::Ready(query) = &mut self.perf else { return false };
        let Some(readings) = query.collect() else { return false };
        // The scheme is judged on every instance, not only on the ones that were busy.
        let Some(naming) = self.layout.naming(&query.instances()) else { return false };
        let percent: HashMap<ProcId, f64> = readings
            .into_iter()
            .filter_map(|(instance, percent)| Some((self.layout.processor(naming, instance)?, percent)))
            .collect();
        update_cores(&mut self.cores, &percent, nominal_mhz);
        true
    }

    /// The power manager's current frequency of each logical processor; only
    /// asked for while a core has no measurement.
    fn reported(&self) -> HashMap<ProcId, f64> {
        if self.cores.iter().all(|core| core.mhz.is_some()) {
            return HashMap::new();
        }
        power::processors()
            .into_iter()
            .filter(|p| p.current_mhz > 0)
            .map(|p| (p.id, f64::from(p.current_mhz)))
            .collect()
    }
}

/// Spins the calling thread on the given logical processor for [`WAKE_TIME`];
/// does nothing if the thread is not allowed to run there.
fn keep_busy(id: ProcId) {
    let pin = Pin::to(id);
    let start = Instant::now();
    while pin.is_pinned() && start.elapsed() < WAKE_TIME {
        std::hint::spin_loop();
    }
}

/// Groups the logical processors of the lowest-numbered package into cores.
fn cores_from_logical(logical: &[LogicalProcessor]) -> Vec<Core> {
    let Some(package) = logical.iter().map(|l| l.package).min() else { return Vec::new() };
    let mut cores: BTreeMap<u32, Core> = BTreeMap::new();
    for l in logical.iter().filter(|l| l.package == package) {
        cores
            .entry(l.core)
            .or_insert_with(|| Core { index: l.core, kind: l.kind, threads: Vec::new(), mhz: None })
            .threads
            .push(ProcId { group: l.group, number: l.number });
    }
    cores.into_values().collect()
}

/// Every logical processor as a core of its own, for a processor model that
/// does not say how they share cores. Their kind is not known this way either.
fn cores_from_layout(layout: &Layout) -> Vec<Core> {
    (0u32..)
        .zip(layout.processors())
        .map(|(index, id)| Core { index, kind: CoreKind::Uniform, threads: vec![id], mhz: None })
        .collect()
}

/// The highest value among the given logical processors.
fn fastest(threads: &[ProcId], values: &HashMap<ProcId, f64>) -> Option<f64> {
    threads.iter().filter_map(|id| values.get(id)).copied().reduce(f64::max)
}

/// Applies fresh readings (percent of `nominal_mhz` per logical processor). A
/// core runs its threads off one clock, so it takes the fastest of them: the
/// slower reading is the one more diluted by wake-ups. Cores without a fresh
/// reading keep their speed.
fn update_cores(cores: &mut [Core], percent: &HashMap<ProcId, f64>, nominal_mhz: f64) {
    for core in cores {
        if let Some(percent) = fastest(&core.threads, percent) {
            core.mhz = Some(percent / 100.0 * nominal_mhz);
        }
    }
}

/// One clock per core that has a speed: its measurement, or failing that the
/// fastest speed the power manager reports for its threads.
fn core_clocks(cores: &[Core], reported: &HashMap<ProcId, f64>, bus_mhz: Option<f64>) -> Vec<CoreClock> {
    cores
        .iter()
        .filter_map(|core| {
            let mhz = core.mhz.or_else(|| fastest(&core.threads, reported))?;
            let multiplier = bus_mhz.filter(|bus| *bus > 0.0).map(|bus| multiplier(mhz, bus));
            Some(CoreClock { core: core.index, kind: core.kind, mhz, multiplier })
        })
        .collect()
}

/// Core speed over bus speed, to one decimal. The speed is an average over the
/// sampling interval rather than a latched ratio, so it need not sit on the
/// processor's ratio steps and anything finer would be noise.
fn multiplier(mhz: f64, bus_mhz: f64) -> f64 {
    round_to(mhz / bus_mhz, 10.0)
}

/// Rounds to the nearest multiple of `1 / parts`.
fn round_to(value: f64, parts: f64) -> f64 {
    (value * parts).round() / parts
}

/// Ratio steps per unit for the nominal frequency: quarters on AMD designs,
/// halves elsewhere (which also covers the whole steps of current Intel parts).
fn ratio_parts(vendor: CpuVendor) -> f64 {
    match vendor {
        CpuVendor::Amd | CpuVendor::Hygon => 4.0,
        _ => 2.0,
    }
}

/// Ratio between the time-stamp counter and the bus clock, i.e. the multiplier
/// of the nominal frequency, in steps of `1 / parts`.
///
/// The first advertised frequency that puts the bus clock near its reference
/// decides. Failing that the bus clock is taken to be near its reference and
/// the ratio is the closest step to what the counter suggests.
fn base_ratio(tsc_mhz: f64, advertised_mhz: &[f64], reference_mhz: f64, parts: f64) -> f64 {
    let plausible =
        |ratio: &f64| *ratio > 0.0 && (tsc_mhz / ratio / reference_mhz - 1.0).abs() <= BUS_TOLERANCE;
    advertised_mhz
        .iter()
        .map(|mhz| round_to(mhz / reference_mhz, parts))
        .find(plausible)
        .unwrap_or_else(|| round_to(tsc_mhz / reference_mhz, parts).max(1.0 / parts))
}

/// Reads the speed fields of the first populated processor socket.
fn firmware_speeds(smbios: &Smbios) -> FirmwareSpeeds {
    const EXTERNAL_CLOCK: usize = 0x12;
    const MAX_SPEED: usize = 0x14;
    const CURRENT_SPEED: usize = 0x16;
    const STATUS: usize = 0x18;
    const SOCKET_POPULATED: u8 = 1 << 6;

    let populated = smbios
        .of_kind(kind::PROCESSOR)
        .find(|s| s.byte(STATUS).map_or(true, |status| status & SOCKET_POPULATED != 0));
    let Some(socket) = populated else { return FirmwareSpeeds::default() };
    // Zero means unknown in all three fields.
    let speed = |offset| socket.word(offset).filter(|mhz| *mhz != 0);
    FirmwareSpeeds {
        external_clock: speed(EXTERNAL_CLOCK),
        max: speed(MAX_SPEED),
        current: speed(CURRENT_SPEED),
    }
}

/// The firmware's external clock in MHz, unless it is a placeholder.
fn stated_reference(firmware: FirmwareSpeeds) -> Option<u16> {
    firmware.external_clock.filter(|mhz| SANE_REFERENCE_MHZ.contains(mhz))
}

/// The bus reference clock in MHz: the firmware's external clock when it is sane.
fn reference_clock(firmware: FirmwareSpeeds) -> f64 {
    stated_reference(firmware).map_or(DEFAULT_REFERENCE_MHZ, f64::from)
}

/// The bus clock in MHz that the firmware states for the first populated processor
/// socket, which is what the clocks here are measured against. `None` when it
/// states none or a placeholder.
pub fn firmware_reference_mhz(smbios: &Smbios) -> Option<u16> {
    stated_reference(firmware_speeds(smbios))
}

/// CPUID leaf 0x16: the maximum (EBX) and bus (ECX) frequency in MHz.
const FREQUENCY_LEAF: u32 = 0x16;

impl FrequencyLeaf {
    fn new(ebx: u32, ecx: u32) -> Self {
        FrequencyLeaf { max: ebx & 0xFFFF, bus: ecx & 0xFFFF }
    }
}

/// The frequency leaf from the CPUID dumps in `cpu`, or failing that from the
/// processor the calling thread is on.
fn frequency_leaf(cpu: &CpuInfo) -> Option<FrequencyLeaf> {
    dumped_frequency_leaf(cpu)
        .or_else(|| cpuid::read_supported(FREQUENCY_LEAF).map(|leaf| FrequencyLeaf::new(leaf.ebx, leaf.ecx)))
}

/// The dumped frequency leaf of the thread with the highest maximum, which on a
/// hybrid processor is a performance core.
fn dumped_frequency_leaf(cpu: &CpuInfo) -> Option<FrequencyLeaf> {
    cpu.thread_dumps
        .iter()
        .flat_map(|dump| &dump.leaves)
        .filter(|l| l.leaf == FREQUENCY_LEAF)
        .map(|l| FrequencyLeaf::new(l.ebx, l.ecx))
        .max_by_key(|leaf| leaf.max)
}

/// Highest multiplier of the processor: its maximum frequency over the bus reference.
///
/// The processor's own statement (CPUID) is preferred. The firmware's "max
/// speed" is used for the other vendors only: Intel boards commonly fill it
/// with what the socket could take.
fn max_multiplier(
    vendor: CpuVendor,
    leaf: Option<FrequencyLeaf>,
    firmware: FirmwareSpeeds,
    reference_mhz: f64,
) -> Option<f64> {
    let from_cpu = leaf.filter(|leaf| leaf.max != 0).map(|leaf| {
        let bus = if leaf.bus != 0 { f64::from(leaf.bus) } else { reference_mhz };
        f64::from(leaf.max) / bus
    });
    let from_firmware = || {
        let below_current = |max: &u16| firmware.current.is_some_and(|current| *max < current);
        let max = firmware.max.filter(|max| vendor != CpuVendor::Intel && !below_current(max))?;
        Some(f64::from(max) / reference_mhz)
    };
    from_cpu.or_else(from_firmware).map(|ratio| round_to(ratio, 100.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::smbios::SmbiosStructure;
    use crate::model::{CpuidLeaf, ThreadDump};

    fn id(group: u16, number: u8) -> ProcId {
        ProcId { group, number }
    }

    fn logical(package: u32, core: u32, group: u16, number: u8, kind: CoreKind) -> LogicalProcessor {
        LogicalProcessor { package, core, group, number, kind, ..Default::default() }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    /// A type 4 structure with the given external clock, max speed, current speed and status.
    fn socket(external: u16, max: u16, current: u16, status: u8) -> SmbiosStructure {
        let mut data = vec![0u8; 0x1A];
        data[0] = kind::PROCESSOR;
        data[1] = 0x1A;
        data[0x12..0x14].copy_from_slice(&external.to_le_bytes());
        data[0x14..0x16].copy_from_slice(&max.to_le_bytes());
        data[0x16..0x18].copy_from_slice(&current.to_le_bytes());
        data[0x18] = status;
        SmbiosStructure { kind: kind::PROCESSOR, handle: 4, data, strings: Vec::new() }
    }

    fn speeds(external: u16, max: u16, current: u16) -> FirmwareSpeeds {
        firmware_speeds(&Smbios {
            major: 3,
            minor: 5,
            structures: vec![socket(external, max, current, 0x41)],
        })
    }

    #[test]
    fn monitor_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<ClockMonitor>();
    }

    #[test]
    fn groups_threads_of_the_first_package_into_cores() {
        let list = [
            logical(1, 0, 1, 0, CoreKind::Uniform),
            logical(0, 1, 0, 2, CoreKind::Efficient),
            logical(0, 0, 0, 0, CoreKind::Performance),
            logical(0, 0, 0, 1, CoreKind::Performance),
        ];
        assert_eq!(
            cores_from_logical(&list),
            vec![
                Core { index: 0, kind: CoreKind::Performance, threads: vec![id(0, 0), id(0, 1)], mhz: None },
                Core { index: 1, kind: CoreKind::Efficient, threads: vec![id(0, 2)], mhz: None },
            ]
        );
        assert!(cores_from_logical(&[]).is_empty());
    }

    #[test]
    fn layout_cores_are_the_logical_processors_in_order() {
        let layout = Layout { groups: vec![2, 1], ..Default::default() };
        let cores = cores_from_layout(&layout);
        assert_eq!(cores.iter().map(|c| c.index).collect::<Vec<_>>(), [0, 1, 2]);
        assert_eq!(cores[2], Core { index: 2, kind: CoreKind::Uniform, threads: vec![id(1, 0)], mhz: None });
    }

    /// Three cores: two with two threads each and one with a single thread.
    fn three_cores() -> Vec<Core> {
        cores_from_logical(&[
            logical(0, 0, 0, 0, CoreKind::Uniform),
            logical(0, 0, 0, 1, CoreKind::Uniform),
            logical(0, 1, 0, 2, CoreKind::Uniform),
            logical(0, 1, 0, 3, CoreKind::Uniform),
            logical(0, 2, 0, 4, CoreKind::Uniform),
        ])
    }

    fn speeds_of(cores: &[Core]) -> Vec<Option<f64>> {
        cores.iter().map(|core| core.mhz).collect()
    }

    #[test]
    fn core_speed_is_its_fastest_thread() {
        let mut cores = three_cores();
        let percent = HashMap::from([(id(0, 0), 110.0), (id(0, 1), 140.0), (id(0, 3), 95.0)]);
        update_cores(&mut cores, &percent, 3800.0);
        assert_eq!(speeds_of(&cores), [Some(5320.0), Some(3610.0), None]);
    }

    #[test]
    fn idle_cores_keep_their_speed_and_fresh_readings_replace_it() {
        let mut cores = three_cores();
        update_cores(&mut cores, &HashMap::from([(id(0, 0), 140.0), (id(0, 2), 120.0)]), 3800.0);
        // Core 0 now only has a reading from its other, slower thread; core 1 has none.
        update_cores(&mut cores, &HashMap::from([(id(0, 1), 100.0), (id(1, 0), 150.0)]), 3800.0);
        assert_eq!(speeds_of(&cores), [Some(3800.0), Some(4560.0), None]);
    }

    #[test]
    fn clocks_follow_the_core_order() {
        let mut cores = three_cores();
        update_cores(
            &mut cores,
            &HashMap::from([(id(0, 1), 140.0), (id(0, 2), 100.0), (id(0, 4), 50.0)]),
            3800.0,
        );
        assert_eq!(
            core_clocks(&cores, &HashMap::new(), Some(100.0)),
            vec![
                CoreClock { core: 0, kind: CoreKind::Uniform, mhz: 5320.0, multiplier: Some(53.2) },
                CoreClock { core: 1, kind: CoreKind::Uniform, mhz: 3800.0, multiplier: Some(38.0) },
                CoreClock { core: 2, kind: CoreKind::Uniform, mhz: 1900.0, multiplier: Some(19.0) },
            ]
        );
    }

    #[test]
    fn measurement_wins_over_reported_speed() {
        let mut cores = three_cores();
        update_cores(&mut cores, &HashMap::from([(id(0, 1), 90.0)]), 4000.0);
        let reported = HashMap::from([(id(0, 0), 3800.0), (id(0, 2), 1900.0), (id(0, 3), 3800.0)]);
        // Core 0 has a measurement, core 1 only reported speeds and core 2 nothing at all.
        assert_eq!(
            core_clocks(&cores, &reported, None),
            vec![
                CoreClock { core: 0, kind: CoreKind::Uniform, mhz: 3600.0, multiplier: None },
                CoreClock { core: 1, kind: CoreKind::Uniform, mhz: 3800.0, multiplier: None },
            ]
        );
    }

    #[test]
    fn multiplier_has_one_decimal() {
        assert!(close(multiplier(4258.32, 99.96), 42.6));
        assert!(close(multiplier(5322.90, 99.96), 53.3));
        assert!(close(multiplier(800.0, 100.0), 8.0));
    }

    #[test]
    fn rounds_to_steps() {
        assert_eq!(round_to(38.009, 4.0), 38.0);
        assert_eq!(round_to(23.71, 4.0), 23.75);
        assert_eq!(round_to(31.87, 2.0), 32.0);
        assert_eq!(round_to(53.2503, 10.0), 53.3);
        assert_eq!(round_to(53.5, 100.0), 53.5);
    }

    #[test]
    fn advertised_frequency_sets_the_base_ratio() {
        // 38 x 99.96 MHz.
        let ratio = base_ratio(3798.48, &[3800.0], 100.0, 4.0);
        assert!(close(ratio, 38.0));
        assert!(close(3798.48 / ratio, 99.96));
        // A raised bus clock is seen as such while it stays within the tolerance.
        assert!(close(base_ratio(3876.0, &[3800.0], 100.0, 4.0), 38.0));
    }

    #[test]
    fn advertised_frequency_off_the_ratio_grid_is_snapped() {
        assert!(close(base_ratio(3793.0, &[3792.9], 100.0, 4.0), 38.0));
        assert!(close(base_ratio(2999.7, &[3000.0], 333.0, 2.0), 9.0));
    }

    #[test]
    fn implausible_advertised_frequency_is_skipped() {
        // Label says 1.8 GHz, the counter runs at 20 x 99.6 MHz.
        assert!(close(base_ratio(1992.0, &[1800.0], 100.0, 2.0), 20.0));
        // The second candidate is tried before guessing.
        assert!(close(base_ratio(2371.0, &[1800.0, 2375.0], 100.0, 4.0), 23.75));
        assert!(close(base_ratio(1992.0, &[0.0], 100.0, 2.0), 20.0));
    }

    #[test]
    fn base_ratio_without_advertised_frequency() {
        assert!(close(base_ratio(3800.9, &[], 100.0, 2.0), 38.0));
        assert!(close(base_ratio(3187.2, &[], 100.0, 2.0), 32.0));
        assert!(close(base_ratio(2370.5, &[], 100.0, 4.0), 23.75));
        // Never zero, so the bus speed stays finite.
        assert!(close(base_ratio(10.0, &[], 100.0, 2.0), 0.5));
    }

    #[test]
    fn reads_firmware_speeds() {
        assert_eq!(
            speeds(100, 5350, 3800),
            FirmwareSpeeds { external_clock: Some(100), max: Some(5350), current: Some(3800) }
        );
        assert_eq!(speeds(0, 0, 0), FirmwareSpeeds::default());
    }

    #[test]
    fn skips_empty_sockets_and_short_structures() {
        let mut short = socket(100, 4000, 3000, 0x41);
        short.data.truncate(0x16);
        let smbios = Smbios {
            major: 2,
            minor: 0,
            structures: vec![socket(133, 3000, 0, 0x00), socket(200, 4200, 3600, 0x41), short.clone()],
        };
        assert_eq!(
            firmware_speeds(&smbios),
            FirmwareSpeeds { external_clock: Some(200), max: Some(4200), current: Some(3600) }
        );
        // Without a status byte the structure is taken as populated; missing fields stay unknown.
        let smbios = Smbios { major: 2, minor: 0, structures: vec![short] };
        assert_eq!(
            firmware_speeds(&smbios),
            FirmwareSpeeds { external_clock: Some(100), max: Some(4000), current: None }
        );
        assert_eq!(firmware_speeds(&Smbios::default()), FirmwareSpeeds::default());
    }

    #[test]
    fn reference_clock_needs_a_sane_value() {
        assert!(close(reference_clock(speeds(133, 0, 0)), 133.0));
        assert!(close(reference_clock(speeds(100, 0, 0)), 100.0));
        assert!(close(reference_clock(speeds(0, 0, 0)), 100.0));
        assert!(close(reference_clock(speeds(25, 0, 0)), 100.0));
        assert!(close(reference_clock(speeds(8000, 0, 0)), 100.0));
    }

    #[test]
    fn firmware_reference_is_the_sane_external_clock_of_the_populated_socket() {
        let stated = |sockets: Vec<SmbiosStructure>| {
            firmware_reference_mhz(&Smbios { major: 3, minor: 5, structures: sockets })
        };
        assert_eq!(stated(vec![socket(125, 0, 0, 0x41)]), Some(125));
        assert_eq!(stated(vec![socket(133, 0, 0, 0x00), socket(100, 0, 0, 0x41)]), Some(100));
        assert_eq!(stated(vec![socket(0, 4000, 3600, 0x41)]), None);
        assert_eq!(stated(vec![socket(25, 0, 0, 0x41)]), None);
        assert_eq!(stated(vec![socket(8000, 0, 0, 0x41)]), None);
        assert_eq!(stated(Vec::new()), None);
    }

    #[test]
    fn max_multiplier_from_firmware() {
        let amd = max_multiplier(CpuVendor::Amd, None, speeds(100, 5350, 3800), 100.0);
        assert!(close(amd.unwrap(), 53.5));
        let quarter = max_multiplier(CpuVendor::Amd, None, speeds(100, 4725, 3700), 100.0);
        assert!(close(quarter.unwrap(), 47.25));
        assert_eq!(max_multiplier(CpuVendor::Amd, None, speeds(100, 0, 3800), 100.0), None);
        // A maximum below the current speed is not a maximum.
        assert_eq!(max_multiplier(CpuVendor::Amd, None, speeds(100, 3000, 3800), 100.0), None);
        assert_eq!(max_multiplier(CpuVendor::Intel, None, speeds(100, 8300, 3600), 100.0), None);
    }

    #[test]
    fn max_multiplier_prefers_the_processor() {
        let leaf = Some(FrequencyLeaf { max: 5200, bus: 100 });
        let intel = max_multiplier(CpuVendor::Intel, leaf, speeds(100, 8300, 3200), 100.0);
        assert!(close(intel.unwrap(), 52.0));
        // A leaf without a bus frequency falls back to the reference clock.
        let no_bus = max_multiplier(
            CpuVendor::Intel,
            Some(FrequencyLeaf { max: 4000, bus: 0 }),
            speeds(0, 0, 0),
            133.0,
        );
        assert!(close(no_bus.unwrap(), 30.08));
        // An empty leaf says nothing.
        let empty = Some(FrequencyLeaf { max: 0, bus: 0 });
        assert!(close(max_multiplier(CpuVendor::Amd, empty, speeds(100, 5350, 3800), 100.0).unwrap(), 53.5));
    }

    #[test]
    fn frequency_leaf_takes_the_fastest_dumped_thread() {
        let dump = |processor, leaf, ebx| ThreadDump {
            processor,
            leaves: vec![
                CpuidLeaf { leaf: 0, eax: 0x20, ..Default::default() },
                CpuidLeaf { leaf, eax: 3200, ebx, ecx: 100, ..Default::default() },
            ],
        };
        let mut cpu = CpuInfo {
            thread_dumps: vec![dump(0, 0x16, 3900), dump(1, 0x16, 0xABCD_0000 | 5200)],
            ..Default::default()
        };
        assert_eq!(dumped_frequency_leaf(&cpu), Some(FrequencyLeaf { max: 5200, bus: 100 }));
        assert_eq!(frequency_leaf(&cpu), Some(FrequencyLeaf { max: 5200, bus: 100 }));
        cpu.thread_dumps = vec![dump(0, 0x15, 5200)];
        assert_eq!(dumped_frequency_leaf(&cpu), None);
    }

    #[test]
    fn sampling_gives_consistent_clocks() {
        let mut monitor = ClockMonitor::new(&CpuInfo::default(), None);
        for _ in 0..2 {
            let snapshot = monitor.sample();
            assert!(snapshot.cores.windows(2).all(|pair| pair[0].core < pair[1].core));
            assert!(snapshot.cores.iter().all(|core| core.mhz.is_finite() && core.mhz > 0.0));
            assert!(snapshot.bus_mhz.map_or(true, |bus| bus.is_finite() && bus > 0.0));
            assert!(snapshot
                .cores
                .iter()
                .all(|core| core.multiplier.is_some() == snapshot.bus_mhz.is_some()));
            assert_eq!((snapshot.multiplier_min, snapshot.rated_fsb_mhz), (None, None));
        }
    }
}
