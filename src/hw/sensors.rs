//! Sensors that need neither administrator rights nor a driver.
//!
//! Windows keeps a few readings in its performance counters: the processor's own energy
//! meters (Intel's running average power limit counters, `Energy Meter` set) and the ACPI
//! thermal zones the firmware describes. Drives report their temperature through the
//! storage queries. Whatever a machine lacks is simply left out.

use std::collections::BTreeSet;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhGetRawCounterArrayW, PdhOpenQueryW, PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA,
    PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_MORE_DATA, PDH_RAW_COUNTER_ITEM_W,
};

use super::gpu;
use super::storage;
use crate::model::{DriveInfo, Sensor, SensorKind};

/// Power of each energy meter in milliwatts, averaged between two collections.
const POWER: PCWSTR = w!(r"\Energy Meter(*)\Power");
/// Energy each meter counted since boot; a meter that never counts is not wired up.
const ENERGY: PCWSTR = w!(r"\Energy Meter(*)\Energy");
/// Thermal zone temperatures in tenths of a kelvin.
const ZONES: PCWSTR = w!(r"\Thermal Zone Information(*)\High Precision Temperature");

/// Readings outside this range (degrees Celsius) are no measurement.
const PLAUSIBLE_CELSIUS: std::ops::RangeInclusive<f64> = -40.0..=150.0;
/// Longest instance name that is looked at, in UTF-16 units.
const MAX_NAME: usize = 64;

/// Reads the sensors of this machine, again and again.
pub struct SensorMonitor {
    counters: Option<Counters>,
    /// Graphics adapters: name and locally unique id.
    adapters: Vec<(String, i64)>,
    /// Drives that reported a temperature: name, device path and bus type.
    drives: Vec<(String, String, Option<u32>)>,
}

impl SensorMonitor {
    /// Opens the performance counters and takes the first collection, which the power
    /// readings of the next [`SensorMonitor::sample`] are averaged from. `drives` are the
    /// detected drives; those that reported a temperature are read again on each sample.
    /// `graphics` adds the graphics adapters' temperatures and fans.
    pub fn new(drives: &[DriveInfo], graphics: bool) -> SensorMonitor {
        let drives = drives
            .iter()
            .filter(|drive| drive.health.is_some_and(|health| health.temperature_c.is_some()))
            .map(|drive| (drive.name.clone(), drive.device_path.clone(), drive.bus_type))
            .collect();
        let adapters = if graphics { gpu::sensor_sources() } else { Vec::new() };
        SensorMonitor { counters: Counters::open(), adapters, drives }
    }

    /// One reading of every sensor: processor power, thermal zones, graphics adapters, drives.
    pub fn sample(&mut self) -> Vec<Sensor> {
        let mut sensors = self.counters.as_mut().map(Counters::sample).unwrap_or_default();
        for (name, luid) in &self.adapters {
            sensors.extend(adapter_sensors(name, &gpu::performance(*luid)));
        }
        sensors.extend(self.drives.iter().filter_map(|(name, path, bus)| {
            let celsius = storage::temperature(path, *bus)?;
            Some(drive_sensor(name, f64::from(celsius)))
        }));
        sensors
    }
}

/// The temperature and fan of a graphics adapter, as far as its driver reports them.
fn adapter_sensors(name: &str, performance: &gpu::Performance) -> Vec<Sensor> {
    let sensor =
        |label: &str, kind, value: f64| Sensor { group: name.to_string(), name: label.into(), kind, value };
    let temperature = performance
        .deci_celsius
        .map(|tenths| sensor("Temperature", SensorKind::Temperature, f64::from(tenths) / 10.0));
    let fan = performance.fan_rpm.map(|rpm| sensor("Fan", SensorKind::Fan, f64::from(rpm)));
    temperature.into_iter().chain(fan).collect()
}

/// The temperatures the drives reported during detection.
pub fn drive_sensors(drives: &[DriveInfo]) -> Vec<Sensor> {
    drives
        .iter()
        .filter_map(|drive| Some(drive_sensor(&drive.name, f64::from(drive.health?.temperature_c?))))
        .collect()
}

fn drive_sensor(name: &str, celsius: f64) -> Sensor {
    Sensor {
        group: name.to_string(),
        name: "Temperature".into(),
        kind: SensorKind::Temperature,
        value: celsius,
    }
}

/// The sensor behind an energy meter instance (`rapl_package0_pkg`), with its power in
/// milliwatts. `None` for the total and for meters this tool does not know.
fn energy_sensor(instance: &str, milliwatts: f64) -> Option<Sensor> {
    let instance = instance.to_ascii_lowercase();
    let (package, plane) = instance.strip_prefix("rapl_package")?.split_once('_')?;
    let package: u32 = package.parse().ok()?;
    let name = match plane {
        "pkg" => "Package",
        "pp0" => "Cores",
        "pp1" => "Graphics",
        "dram" => "DRAM",
        "psys" => "Platform",
        _ => return None,
    };
    let group = if package == 0 { "CPU".to_string() } else { format!("CPU #{}", package + 1) };
    (milliwatts >= 0.0).then(|| Sensor {
        group,
        name: name.into(),
        kind: SensorKind::Power,
        value: milliwatts / 1000.0,
    })
}

/// The sensor of a thermal zone instance (`\_TZ.TZ00`), with its temperature in tenths
/// of a kelvin.
fn zone_sensor(instance: &str, deci_kelvin: i64) -> Option<Sensor> {
    let name = instance.rsplit('.').next().unwrap_or(instance).to_ascii_uppercase();
    let celsius = deci_kelvin as f64 / 10.0 - 273.15;
    PLAUSIBLE_CELSIUS.contains(&celsius).then(|| Sensor {
        group: "ACPI".into(),
        name,
        kind: SensorKind::Temperature,
        value: celsius,
    })
}

/// The performance counters the sensors are read from. Each is optional: most desktops
/// have no energy meters, and some firmware describes no thermal zone.
struct Counters {
    query: isize,
    power: Option<isize>,
    energy: Option<isize>,
    zones: Option<isize>,
    /// Receives the counter arrays; 64-bit words keep the items aligned.
    buffer: Vec<u64>,
}

impl Counters {
    fn open() -> Option<Counters> {
        let mut query = 0isize;
        // SAFETY: `query` is a live local that receives the handle; a null source means live data.
        if unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) } != ERROR_SUCCESS.0 {
            return None;
        }
        // From here on dropping `counters` closes the query.
        let mut counters = Counters { query, power: None, energy: None, zones: None, buffer: Vec::new() };
        counters.power = counters.add(POWER);
        counters.energy = counters.add(ENERGY);
        counters.zones = counters.add(ZONES);
        if counters.power.is_none() && counters.zones.is_none() {
            return None;
        }
        // The power readings are averages since the previous collection.
        // SAFETY: the query handle is open.
        unsafe { PdhCollectQueryData(counters.query) };
        Some(counters)
    }

    fn add(&self, path: PCWSTR) -> Option<isize> {
        let mut counter = 0isize;
        // SAFETY: the query handle is open, the path is a NUL-terminated literal and
        // `counter` is a live local that receives the handle.
        let status = unsafe { PdhAddEnglishCounterW(self.query, path, 0, &mut counter) };
        (status == ERROR_SUCCESS.0).then_some(counter)
    }

    fn sample(&mut self) -> Vec<Sensor> {
        // SAFETY: the query handle stays open for the lifetime of `self`.
        if unsafe { PdhCollectQueryData(self.query) } != ERROR_SUCCESS.0 {
            return Vec::new();
        }
        let counting: BTreeSet<String> = self
            .energy
            .map(|counter| self.raw(counter))
            .unwrap_or_default()
            .into_iter()
            .filter(|&(_, energy)| energy > 0)
            .map(|(instance, _)| instance.to_ascii_lowercase())
            .collect();
        let mut sensors: Vec<Sensor> = self
            .power
            .map(|counter| self.formatted(counter))
            .unwrap_or_default()
            .into_iter()
            .filter(|(instance, _)| counting.contains(&instance.to_ascii_lowercase()))
            .filter_map(|(instance, milliwatts)| energy_sensor(&instance, milliwatts))
            .collect();
        sensors.sort_by_key(|sensor| (sensor.group.clone(), plane_rank(&sensor.name)));
        let zones = self.zones.map(|counter| self.raw(counter)).unwrap_or_default();
        sensors.extend(zones.into_iter().filter_map(|(instance, value)| zone_sensor(&instance, value)));
        sensors
    }

    /// The raw value of every valid instance of `counter`.
    fn raw(&mut self, counter: isize) -> Vec<(String, i64)> {
        let Some(items) = self.fetch::<PDH_RAW_COUNTER_ITEM_W>(|size, count, items| {
            // SAFETY: `items` is null or points to `size` writable bytes of the buffer.
            unsafe { PdhGetRawCounterArrayW(counter, size, count, items) }
        }) else {
            return Vec::new();
        };
        items
            .iter()
            .filter(|item| matches!(item.RawValue.CStatus, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA))
            .filter_map(|item| Some((instance_name(item.szName.0)?, item.RawValue.FirstValue)))
            .collect()
    }

    /// The value of every valid instance of `counter`, as PDH computes it.
    fn formatted(&mut self, counter: isize) -> Vec<(String, f64)> {
        let Some(items) = self.fetch::<PDH_FMT_COUNTERVALUE_ITEM_W>(|size, count, items| {
            // SAFETY: `items` is null or points to `size` writable bytes of the buffer.
            unsafe { PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, size, count, items) }
        }) else {
            return Vec::new();
        };
        items
            .iter()
            .filter(|item| matches!(item.FmtValue.CStatus, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA))
            // SAFETY: PDH_FMT_DOUBLE was asked for, so the double member is the one written.
            .filter_map(|item| {
                Some((instance_name(item.szName.0)?, unsafe { item.FmtValue.Anonymous.doubleValue }))
            })
            .filter(|(_, value)| value.is_finite())
            .collect()
    }

    /// Fills the buffer through `call` and returns the items it wrote. The size PDH
    /// asks for is only dependable for a call without a buffer, hence up to three rounds.
    fn fetch<T>(&mut self, mut call: impl FnMut(&mut u32, &mut u32, Option<*mut T>) -> u32) -> Option<Vec<T>>
    where
        T: Copy,
    {
        for _ in 0..3 {
            let mut size = u32::try_from(self.buffer.len() * 8).ok()?;
            let mut count = 0u32;
            let items = (!self.buffer.is_empty()).then(|| self.buffer.as_mut_ptr().cast());
            match call(&mut size, &mut count, items) {
                s if s == ERROR_SUCCESS.0 => {
                    let count = count as usize;
                    if count.checked_mul(size_of::<T>())? > self.buffer.len() * 8 {
                        return None;
                    }
                    // SAFETY: PDH wrote `count` items at the start of the buffer (checked to fit
                    // above); the buffer is 8-byte aligned, and the items are copied out before
                    // the buffer is touched again.
                    let items =
                        unsafe { std::slice::from_raw_parts(self.buffer.as_ptr().cast::<T>(), count) };
                    return Some(items.to_vec());
                }
                PDH_MORE_DATA if self.buffer.is_empty() => self.buffer.resize((size as usize).div_ceil(8), 0),
                PDH_MORE_DATA => self.buffer.clear(),
                _ => return None,
            }
        }
        None
    }
}

impl Drop for Counters {
    fn drop(&mut self) {
        // SAFETY: the query handle is open and is not used after this.
        unsafe { PdhCloseQuery(self.query) };
    }
}

/// Display order of the energy meters inside one processor.
fn plane_rank(name: &str) -> u8 {
    ["Package", "Cores", "Graphics", "DRAM", "Platform"].iter().position(|plane| *plane == name).unwrap_or(9)
        as u8
}

/// The instance name an item points to, read up to [`MAX_NAME`] units.
fn instance_name(name: *const u16) -> Option<String> {
    if name.is_null() {
        return None;
    }
    let mut units = Vec::new();
    for index in 0..MAX_NAME {
        // SAFETY: PDH NUL-terminates every instance name inside the buffer it wrote,
        // and the scan stops at that NUL or at MAX_NAME.
        let unit = unsafe { *name.add(index) };
        if unit == 0 {
            return Some(String::from_utf16_lossy(&units));
        }
        units.push(unit);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DriveHealth, DriveInfo};

    #[test]
    fn energy_meters_name_the_processor_planes() {
        let reading =
            |instance, milliwatts| energy_sensor(instance, milliwatts).map(|s| (s.group, s.name, s.value));
        assert_eq!(reading("rapl_package0_pkg", 21_500.0), Some(("CPU".into(), "Package".into(), 21.5)));
        assert_eq!(reading("RAPL_Package0_PP0", 14_625.0), Some(("CPU".into(), "Cores".into(), 14.625)));
        assert_eq!(reading("rapl_package0_pp1", 0.0), Some(("CPU".into(), "Graphics".into(), 0.0)));
        assert_eq!(reading("rapl_package0_dram", 1_500.0), Some(("CPU".into(), "DRAM".into(), 1.5)));
        // The second package of a two-socket board gets its own group.
        assert_eq!(reading("rapl_package1_pkg", 1_000.0), Some(("CPU #2".into(), "Package".into(), 1.0)));
        assert!(energy_sensor("rapl_package0_pkg", 21_000.0).is_some_and(|s| s.kind == SensorKind::Power));
        // The sum, meters nobody documents, and readings that cannot be power.
        assert_eq!(reading("_total", 30_000.0), None);
        assert_eq!(reading("emi_something_else", 1_000.0), None);
        assert_eq!(reading("rapl_package0_pkg", -5.0), None);
        assert_eq!(reading("rapl_package0_pkg", f64::NAN), None);
    }

    #[test]
    fn thermal_zones_read_in_degrees_celsius() {
        let zone = zone_sensor(r"\_tz.tz00", 3292).unwrap();
        assert_eq!(
            (zone.group.as_str(), zone.name.as_str(), zone.kind),
            ("ACPI", "TZ00", SensorKind::Temperature)
        );
        assert!((zone.value - 56.05).abs() < 1e-9);
        assert_eq!(zone_sensor(r"\_TZ.CPUZ", 3032).map(|z| z.name), Some("CPUZ".to_string()));
        // Zones that read nothing sensible are left out.
        assert!(zone_sensor(r"\_tz.tz01", 0).is_none());
        assert!(zone_sensor(r"\_tz.tz02", 9999).is_none());
    }

    #[test]
    fn drives_report_the_temperature_detection_read() {
        let drive = |name: &str, temperature_c| DriveInfo {
            name: name.into(),
            health: Some(DriveHealth { temperature_c, ..DriveHealth::default() }),
            ..DriveInfo::default()
        };
        let sensors = drive_sensors(&[drive("SSD", Some(38.0)), drive("HDD", None), DriveInfo::default()]);
        assert_eq!(sensors.len(), 1);
        assert_eq!((sensors[0].group.as_str(), sensors[0].name.as_str()), ("SSD", "Temperature"));
        assert_eq!(sensors[0].value_text(), "38.0 °C");
    }

    #[test]
    fn graphics_cards_report_temperature_and_fan() {
        let performance =
            gpu::Performance { deci_celsius: Some(364), fan_rpm: Some(0), ..gpu::Performance::default() };
        let sensors = adapter_sensors("NVIDIA GeForce RTX 4060 Ti", &performance);
        let readings: Vec<(&str, String)> =
            sensors.iter().map(|s| (s.name.as_str(), s.value_text())).collect();
        assert_eq!(readings, [("Temperature", "36.4 °C".to_string()), ("Fan", "0 RPM".to_string())]);
        assert!(sensors.iter().all(|s| s.group == "NVIDIA GeForce RTX 4060 Ti"));
        assert!(adapter_sensors("Intel(R) UHD Graphics 770", &gpu::Performance::default()).is_empty());
    }

    #[test]
    fn sampling_this_machine_gives_plausible_readings() {
        let mut monitor = SensorMonitor::new(&[], true);
        std::thread::sleep(std::time::Duration::from_millis(50));
        for sensor in monitor.sample() {
            let plausible = match sensor.kind {
                SensorKind::Power => (0.0..2_000.0).contains(&sensor.value),
                SensorKind::Temperature => (-40.0..=150.0).contains(&sensor.value),
                SensorKind::Fan => sensor.value >= 0.0,
                SensorKind::Voltage => (0.0..3.0).contains(&sensor.value),
            };
            assert!(plausible, "{sensor:?}");
        }
    }
}
