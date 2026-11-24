//! The figures only ring 0 can read, through the PawnIO driver: temperatures, core
//! voltage and multiplier range from the processor's model-specific registers, and the
//! memory timings and channel layout from the memory controller. Intel client processors
//! only; everything else, and every register a generation encodes differently, is left
//! as user mode leaves it.

use windows::core::{w, Owned, PCWSTR};
use windows::Win32::Foundation::{HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

use super::memory::spd::{self, Spd};
use super::pawnio::{Mchbar, Module, Msr, Smbus, Transport};
use super::processors::{Pin, ProcId};
use crate::model::{CoreKind, CpuInfo, CpuVendor, MemoryTimings, Sensor, SensorKind};

/// Registers read.
const PERF_STATUS: u32 = 0x198;
const THERM_STATUS: u32 = 0x19C;
const TEMPERATURE_TARGET: u32 = 0x1A2;
const PACKAGE_THERM_STATUS: u32 = 0x1B1;
const PLATFORM_INFO: u32 = 0xCE;
const TURBO_RATIO_LIMIT: u32 = 0x1AD;
const UNCORE_PERF_STATUS: u32 = 0x621;

/// The memory controllers' windows inside MCHBAR, and the registers read in each: the DIMM
/// layout of its two channels and the timings of its first.
const CONTROLLERS: [u32; 2] = [0x0, 0x1_0000];
const MAD_DIMM_CH0: u32 = 0xD80C;
const MAD_DIMM_CH1: u32 = 0xD810;
const TC_PRE: u32 = 0xE000;
const TC_PRE_HIGH: u32 = 0xE004;
const TC_ODT: u32 = 0xE070;

/// The lock monitoring tools agree on before they use the SMBus, and how long to wait for it.
const SMBUS_LOCK: PCWSTR = w!(r"Global\Access_SMBUS.HTP.Method");
const LOCK_WAIT_MS: u32 = 500;

/// Intel client processors (family 6) whose thermal, voltage and ratio registers have
/// the layout read here: Sandy Bridge through Arrow Lake, desktop and mobile.
const CLIENT_MODELS: &[u16] = &[
    0x2A, 0x3A, 0x3C, 0x45, 0x46, 0x3D, 0x47, 0x4E, 0x5E, 0x8E, 0x9E, 0xA5, 0xA6, 0x7D, 0x7E, 0xA7, 0x8C,
    0x8D, 0x97, 0x9A, 0xB7, 0xBA, 0xBF, 0xAA, 0xAC, 0xBD, 0xB5, 0xC5, 0xC6,
];
/// Those with the ring's ratio in the uncore performance status: Skylake to Raptor Lake.
const UNCORE_MODELS: &[u16] =
    &[0x4E, 0x5E, 0x8E, 0x9E, 0xA5, 0xA6, 0x7D, 0x7E, 0xA7, 0x8C, 0x8D, 0x97, 0x9A, 0xB7, 0xBA, 0xBF];
/// The desktop (S and HX) dies of Alder and Raptor Lake, whose memory controller
/// registers are decoded here.
const CONTROLLER_MODELS: &[u16] = &[0x97, 0xB7, 0xBF];

/// Everything one pass read.
#[derive(Debug, Clone, Default)]
pub struct Readings {
    pub core_voltage: Option<f64>,
    pub ratio_range: Option<(f64, f64)>,
    pub uncore_mhz: Option<f64>,
    pub sensors: Vec<Sensor>,
    pub timings: Option<MemoryTimings>,
    /// Populated 32-bit DDR5 sub-channels.
    pub sub_channels: Option<u32>,
    /// The SPD data of each module that answered, by SMBus address.
    pub spd: Vec<(u8, Spd)>,
    /// Whether one of the driver's modules served this machine.
    pub used: bool,
}

/// Reads what the driver can tell about `cpu` and its memory, of type `memory_kind`.
pub fn read(cpu: &CpuInfo, memory_kind: Option<&str>) -> Readings {
    let mut readings = Readings::default();
    if let Some(live) = Live::open(cpu) {
        readings.used = true;
        readings.ratio_range = live.ratio_range;
        readings.uncore_mhz = live.uncore_mhz();
        readings.core_voltage = live.core_voltage();
        readings.sensors = live.sample();
    }
    if reads_memory_controller(cpu) {
        if let Some(mchbar) = Mchbar::open() {
            readings.used = true;
            read_controller(&mchbar, memory_kind, &mut readings);
        }
    }
    if cpu.vendor == CpuVendor::Intel {
        // Another tool may be in the middle of a transfer. The lock keeps the two apart while
        // the module starts up and while each memory module is read; without it nothing is read.
        if let Some(smbus) = BusLock::acquire(SMBUS_LOCK).and_then(|_lock| Smbus::open()) {
            readings.used = true;
            readings.spd = spd::read(&smbus, || BusLock::acquire(SMBUS_LOCK));
        }
    }
    readings
}

/// A named mutex shared with other tools, held until dropped.
struct BusLock(Owned<HANDLE>);

impl BusLock {
    /// Takes the lock, waiting a little for its owner. An owner that died holding it
    /// leaves it abandoned, which counts as taken.
    fn acquire(name: PCWSTR) -> Option<BusLock> {
        // SAFETY: the name is a NUL-terminated literal; the handle is owned by `Owned`.
        let handle = unsafe { CreateMutexW(None, false, name) }.ok()?;
        // SAFETY: the call above returned the handle, closed once by `Owned`.
        let handle = unsafe { Owned::new(handle) };
        // SAFETY: the handle is a live mutex handle.
        let waited = unsafe { WaitForSingleObject(*handle, LOCK_WAIT_MS) };
        // Only a mutex this thread took is released; a timed-out wait just closes the handle.
        (waited == WAIT_OBJECT_0 || waited == WAIT_ABANDONED).then(|| BusLock(handle))
    }
}

impl Drop for BusLock {
    fn drop(&mut self) {
        // SAFETY: the mutex is owned by this thread, which took it in `acquire`.
        let _ = unsafe { ReleaseMutex(*self.0) };
    }
}

/// The registers that change while the program runs, read again for the Sensors window.
pub struct Live {
    msr: Msr<Module>,
    /// The first processor of the package, where package-wide registers are read.
    first: ProcId,
    /// One processor of each core, with the core's name.
    cores: Vec<(String, ProcId)>,
    tjmax: Option<u32>,
    ratio_range: Option<(f64, f64)>,
    uncore: bool,
}

impl Live {
    /// Loads the register module for `cpu`; `None` when the driver is off or the
    /// processor is not one whose registers are read.
    pub fn open(cpu: &CpuInfo) -> Option<Live> {
        if !reads_msrs(cpu) {
            return None;
        }
        let msr = Msr::open()?;
        let package = || cpu.logical.iter().filter(|lp| lp.package == 0);
        let first = package().next().map(|lp| ProcId { group: lp.group, number: lp.number })?;
        let cores = package()
            .filter(|lp| lp.smt == 0)
            .map(|lp| (core_name(lp.kind, lp.core), ProcId { group: lp.group, number: lp.number }))
            .collect();
        // These are read on the first processor or not at all: on an E-core of a hybrid part
        // the turbo ratio limits are the E-cores' own.
        let pin = Pin::to(first);
        let (tjmax, ratio_range) = if pin.is_pinned() {
            let turbo = msr.read(TURBO_RATIO_LIMIT).map(|limits| limits & 0xFF).filter(|&ratio| ratio != 0);
            (
                msr.read(TEMPERATURE_TARGET).and_then(tjmax),
                msr.read(PLATFORM_INFO).and_then(|info| ratio_range(info, turbo)),
            )
        } else {
            (None, None)
        };
        drop(pin);
        Some(Live { msr, first, cores, tjmax, ratio_range, uncore: has_uncore_ratio(cpu) })
    }

    fn uncore_mhz(&self) -> Option<f64> {
        let pin = Pin::to(self.first);
        // The ring runs off the 100 MHz base clock.
        (self.uncore && pin.is_pinned())
            .then(|| self.msr.read(UNCORE_PERF_STATUS))
            .flatten()
            .and_then(uncore_ratio)
            .map(|ratio| f64::from(ratio) * 100.0)
    }

    fn core_voltage(&self) -> Option<f64> {
        let pin = Pin::to(self.first);
        pin.is_pinned().then(|| self.msr.read(PERF_STATUS)).flatten().and_then(core_voltage)
    }

    /// Package and core temperatures, and the voltage the first core runs at.
    pub fn sample(&self) -> Vec<Sensor> {
        let sensor = |name: &str, kind, value| Sensor { group: "CPU".into(), name: name.into(), kind, value };
        let mut sensors = Vec::new();
        let mut pin = Pin::to(self.first);
        if let Some(tjmax) = self.tjmax {
            if let Some(celsius) =
                self.msr.read(PACKAGE_THERM_STATUS).and_then(|status| package_temperature(status, tjmax))
            {
                sensors.push(sensor("Package", SensorKind::Temperature, celsius));
            }
            for (name, processor) in &self.cores {
                if !pin.move_to(*processor) {
                    continue;
                }
                if let Some(celsius) =
                    self.msr.read(THERM_STATUS).and_then(|status| core_temperature(status, tjmax))
                {
                    sensors.push(sensor(name, SensorKind::Temperature, celsius));
                }
            }
        }
        if pin.move_to(self.first) {
            if let Some(volts) = self.msr.read(PERF_STATUS).and_then(core_voltage) {
                sensors.push(sensor("Core VID", SensorKind::Voltage, volts));
            }
        }
        sensors
    }
}

/// Reads the DIMM layout and the timings of the first populated controller.
fn read_controller<T: Transport>(mchbar: &Mchbar<T>, memory_kind: Option<&str>, readings: &mut Readings) {
    let layout: Vec<[u32; 2]> = CONTROLLERS
        .iter()
        .map(|&base| [MAD_DIMM_CH0, MAD_DIMM_CH1].map(|register| mchbar.read(base + register).unwrap_or(0)))
        .collect();
    let channels = populated_channels(&layout.concat());
    // The two channels of a controller are the 32-bit halves of a DDR5 module; other
    // memory types pair channels and modules differently and keep the firmware's count.
    if memory_kind == Some("DDR5") && channels > 0 {
        readings.sub_channels = Some(channels);
    }
    readings.timings =
        CONTROLLERS.iter().zip(&layout).find(|(_, dimms)| populated_channels(&dimms[..]) > 0).and_then(
            |(&base, _)| {
                memory_timings(
                    mchbar.read(base + TC_PRE)?,
                    mchbar.read(base + TC_PRE_HIGH)?,
                    mchbar.read(base + TC_ODT)?,
                )
            },
        );
}

/// `P-Core #0`, `E-Core #8`, `Core #3`.
fn core_name(kind: CoreKind, core: u32) -> String {
    format!("{} #{core}", kind.label())
}

fn bits(value: u64, high: u32, low: u32) -> u64 {
    (value >> low) & ((1 << (high - low + 1)) - 1)
}

/// Whether the processor's thermal, voltage and ratio registers are read.
fn reads_msrs(cpu: &CpuInfo) -> bool {
    cpu.vendor == CpuVendor::Intel && cpu.ext_family == 6 && CLIENT_MODELS.contains(&cpu.ext_model)
}

fn has_uncore_ratio(cpu: &CpuInfo) -> bool {
    reads_msrs(cpu) && UNCORE_MODELS.contains(&cpu.ext_model)
}

/// Whether the memory controller's registers are read.
fn reads_memory_controller(cpu: &CpuInfo) -> bool {
    reads_msrs(cpu) && CONTROLLER_MODELS.contains(&cpu.ext_model)
}

/// TjMax, the temperature the digital thermal sensors count down from (temperature target, bits 23:16).
fn tjmax(target: u64) -> Option<u32> {
    let celsius = bits(target, 23, 16) as u32;
    (50..=130).contains(&celsius).then_some(celsius)
}

/// A core's temperature from its thermal status: TjMax less the readout (bits 22:16),
/// when the reading-valid bit (31) is set.
fn core_temperature(status: u64, tjmax: u32) -> Option<f64> {
    (bits(status, 31, 31) == 1).then(|| f64::from(tjmax) - bits(status, 22, 16) as f64)
}

/// The package temperature from the package thermal status, which has no valid bit.
fn package_temperature(status: u64, tjmax: u32) -> Option<f64> {
    let readout = bits(status, 22, 16) as u32;
    (readout <= tjmax).then(|| f64::from(tjmax - readout))
}

/// The voltage the core requests, in volts (performance status, bits 47:32, in 1/8192 V).
fn core_voltage(status: u64) -> Option<f64> {
    let volts = bits(status, 47, 32) as f64 / 8192.0;
    (0.2..=2.0).contains(&volts).then_some(volts)
}

/// The lowest ratio (platform information, bits 47:40) and the highest: the single-core
/// turbo ratio when there is one, else the base ratio (bits 15:8).
fn ratio_range(platform: u64, turbo: Option<u64>) -> Option<(f64, f64)> {
    let (lowest, base) = (bits(platform, 47, 40), bits(platform, 15, 8));
    let highest = turbo.unwrap_or(base);
    (lowest != 0 && highest >= lowest).then_some((lowest as f64, highest as f64))
}

/// The ring's current ratio (uncore performance status, bits 6:0).
fn uncore_ratio(status: u64) -> Option<u32> {
    let ratio = bits(status, 6, 0) as u32;
    (ratio != 0).then_some(ratio)
}

/// The timings of a memory controller channel: tRP from the low half of its precharge
/// register, tRAS (bits 18:10) and tRCD (bits 26:19) from its high half, CL from bits
/// 22:16 of its ODT register; tRC is tRAS plus tRP. The command rate has no documented
/// field here and stays unknown.
fn memory_timings(pre_low: u32, pre_high: u32, odt: u32) -> Option<MemoryTimings> {
    let field = |value: u32, high: u32, low: u32| bits(u64::from(value), high, low) as u32;
    let (trp, tras, trcd, cl) =
        (field(pre_low, 7, 0), field(pre_high, 18, 10), field(pre_high, 26, 19), field(odt, 22, 16));
    (trp != 0 && tras != 0 && trcd != 0 && cl != 0).then(|| MemoryTimings {
        cas_latency: Some(cl as f32),
        trcd: Some(trcd),
        trp: Some(trp),
        tras: Some(tras),
        trc: Some(tras + trp),
        command_rate: None,
    })
}

/// Channels with a DIMM, from the DIMM size fields (bits 6:0, large; 22:16, small) of the
/// channels' DIMM registers. A controller that is not there reads all ones.
fn populated_channels(dimm_registers: &[u32]) -> u32 {
    dimm_registers
        .iter()
        .filter(|&&register| register != u32::MAX && (register & 0x7F != 0 || (register >> 16) & 0x7F != 0))
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_thermal_registers() {
        assert_eq!(tjmax(0x0064_0000), Some(100));
        assert_eq!(tjmax(0), None);
        // Valid reading 45 degrees below TjMax.
        assert_eq!(core_temperature(0x802D_0000, 100), Some(55.0));
        // A reading without its valid bit is no reading.
        assert_eq!(core_temperature(0x002D_0000, 100), None);
        assert_eq!(package_temperature(0x0028_0000, 100), Some(60.0));
    }

    #[test]
    fn decodes_voltage_and_ratios() {
        let volts = core_voltage(9871_u64 << 32).unwrap();
        assert!((volts - 1.2049).abs() < 1e-3, "{volts}");
        assert_eq!(core_voltage(0), None);
        let platform = (8_u64 << 40) | (21 << 8);
        assert_eq!(ratio_range(platform, Some(52)), Some((8.0, 52.0)));
        // Without turbo ratios the top is the base ratio.
        assert_eq!(ratio_range(platform, None), Some((8.0, 21.0)));
        assert_eq!(ratio_range(0, Some(52)), None);
        assert_eq!(uncore_ratio(0x0000_0000_0000_2F2F), Some(47));
        assert_eq!(uncore_ratio(0), None);
    }

    #[test]
    fn decodes_the_memory_controller_timings() {
        // DDR5-4800 at its JEDEC timings: CL40, tRCD 39, tRP 39, tRAS 77.
        let timings = memory_timings(39, (77 << 10) | (39 << 19), 40 << 16).unwrap();
        assert_eq!(timings.cas_latency, Some(40.0));
        assert_eq!(
            (timings.trcd, timings.trp, timings.tras, timings.trc),
            (Some(39), Some(39), Some(77), Some(116))
        );
        assert_eq!(timings.command_rate, None);
        // An unpopulated controller reads zero.
        assert_eq!(memory_timings(0, 0, 0), None);
    }

    #[test]
    fn counts_the_populated_sub_channels() {
        assert_eq!(populated_channels(&[0x20, 0x20, 0x20, 0x20]), 4);
        assert_eq!(populated_channels(&[0x20, 0x20, 0, 0]), 2);
        // A channel whose only DIMM is the small one counts as well.
        assert_eq!(populated_channels(&[0x20_0000, 0, 0, 0]), 1);
        // A controller that is not there reads all ones.
        assert_eq!(populated_channels(&[u32::MAX, 0x20, 0, 0]), 1);
    }

    #[test]
    fn only_client_intel_parts_are_read() {
        let intel =
            |ext_model| CpuInfo { vendor: CpuVendor::Intel, ext_family: 6, ext_model, ..CpuInfo::default() };
        assert!(reads_msrs(&intel(0xB7)));
        assert!(reads_msrs(&intel(0x2A)));
        // Server and low-power parts encode some of the registers differently.
        assert!(!reads_msrs(&intel(0x8F)));
        assert!(!reads_msrs(&intel(0x5C)));
        assert!(!reads_msrs(&CpuInfo { vendor: CpuVendor::Amd, ..intel(0xB7) }));
        assert!(reads_memory_controller(&intel(0xB7)) && reads_memory_controller(&intel(0x97)));
        assert!(!reads_memory_controller(&intel(0xA5)));
        assert!(has_uncore_ratio(&intel(0xB7)) && !has_uncore_ratio(&intel(0x2A)));
    }
}
