//! Plain-data description of the machine, shared by the detection backends,
//! the UI and the report writers.
//!
//! Anything a backend may be unable to determine is an `Option` (or an empty
//! string / vector). The UI renders a missing value as an empty box and the
//! reports skip the line, so backends should never invent a value.

use crate::hw::smbios::Smbios;

#[derive(Debug, Clone, Default)]
pub struct SystemInfo {
    pub cpu: CpuInfo,
    pub mainboard: MainboardInfo,
    pub memory: MemoryInfo,
    /// One entry per physical memory slot, populated or not.
    pub modules: Vec<MemoryModule>,
    pub gpus: Vec<GpuInfo>,
    /// The interfaces the display adapters were read through, e.g. `D3D`.
    pub graphic_apis: Vec<String>,
    pub monitors: Vec<MonitorInfo>,
    pub drives: Vec<DriveInfo>,
    /// The devices on the USB ports, each hub followed by what is plugged into it.
    pub usb: Vec<UsbDevice>,
    pub os: OsInfo,
    pub pci: Vec<PciDevice>,
    pub smbios: Option<Smbios>,
    pub timers: Vec<TimerFrequency>,
}

// ---------------------------------------------------------------------------
// Processor
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CpuVendor {
    Intel,
    Amd,
    Hygon,
    Zhaoxin,
    Via,
    #[default]
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum CoreKind {
    /// Every core is the same kind (non-hybrid processor).
    #[default]
    Uniform,
    Performance,
    Efficient,
    LowPower,
}

impl CoreKind {
    /// Label prefix used across the UI and reports: `Core`, `P-Core`, `E-Core`, `LP-Core`.
    pub fn label(self) -> &'static str {
        match self {
            CoreKind::Uniform => "Core",
            CoreKind::Performance => "P-Core",
            CoreKind::Efficient => "E-Core",
            CoreKind::LowPower => "LP-Core",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CacheKind {
    Data,
    Instruction,
    #[default]
    Unified,
    Trace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheInfo {
    pub level: u8,
    pub kind: CacheKind,
    /// Size of one cache instance in KiB (in K-uops for a trace cache).
    pub size_kb: u32,
    /// Number of instances of this cache in the package.
    pub count: u32,
    /// Ways of associativity; `0` when unknown, `u32::MAX` when fully associative.
    pub ways: u32,
    /// Line size in bytes, `0` when unknown.
    pub line_size: u32,
    /// Logical processors sharing one instance.
    pub threads_sharing: u32,
}

impl CacheInfo {
    /// `8 x 32 KBytes`, `32 MBytes`, `8 x 1.25 MBytes` - the size column of the CPU tab.
    pub fn size_text(&self) -> String {
        self.sized("KBytes", "MBytes")
    }

    /// `8 x 32 KB`, `96 MB` - the compact size, for a line that lists several caches.
    pub fn short_size_text(&self) -> String {
        self.sized("KB", "MB")
    }

    fn sized(&self, kilobytes: &str, megabytes: &str) -> String {
        let kb = self.size_kb;
        let size = if self.kind == CacheKind::Trace {
            format!("{kb} Kuops")
        } else if kb < 1024 {
            format!("{kb} {kilobytes}")
        } else if kb % 1024 == 0 {
            format!("{} {megabytes}", kb / 1024)
        } else {
            format!("{:.2} {megabytes}", f64::from(kb) / 1024.0)
        };
        if self.count > 1 {
            format!("{} x {size}", self.count)
        } else {
            size
        }
    }

    /// `8-way`, `direct`, `fully` - the associativity column of the CPU tab.
    pub fn ways_text(&self) -> String {
        match self.ways {
            0 => String::new(),
            1 => "direct".to_string(),
            u32::MAX => "fully".to_string(),
            n => format!("{n}-way"),
        }
    }
}

/// A group of identical cores inside a package (one entry on non-hybrid parts).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CoreSet {
    pub kind: CoreKind,
    pub cores: u32,
    pub threads: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LogicalProcessor {
    /// Zero-based index across all processor groups.
    pub index: u32,
    pub group: u16,
    /// Number inside its processor group.
    pub number: u8,
    pub apic_id: u32,
    pub package: u32,
    /// Core index inside the package, dense and zero-based.
    pub core: u32,
    /// Thread index inside the core.
    pub smt: u32,
    pub kind: CoreKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CpuidLeaf {
    pub leaf: u32,
    pub subleaf: u32,
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
}

/// Raw CPUID leaves gathered while pinned to one logical processor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThreadDump {
    pub processor: u32,
    pub leaves: Vec<CpuidLeaf>,
}

#[derive(Debug, Clone, Default)]
pub struct CpuInfo {
    pub vendor: CpuVendor,
    /// The 12-character CPUID vendor string, e.g. `AuthenticAMD`.
    pub vendor_string: String,
    /// Short marketing name, e.g. `AMD Ryzen 7 7700`.
    pub name: String,
    /// The CPUID brand string, whitespace-normalised.
    pub specification: String,
    pub codename: Option<String>,
    /// Socket / package, e.g. `Socket AM5 (LGA1718)`.
    pub package: Option<String>,
    pub technology_nm: Option<u32>,
    pub max_tdp_w: Option<f32>,
    /// Needs a ring-0 backend; `None` in user mode.
    pub core_voltage: Option<f32>,
    pub family: u8,
    pub model: u8,
    pub stepping: u8,
    pub ext_family: u16,
    pub ext_model: u16,
    /// Core stepping / revision name, e.g. `RPL-B2`.
    pub revision: Option<String>,
    /// Instruction-set names in display order, e.g. `["MMX (+)", "SSE", ...]`.
    pub instructions: Vec<String>,
    /// The caches of the first package in display order, one entry per distinct geometry.
    pub caches: Vec<CacheInfo>,
    /// Cores and threads of the first package.
    pub cores: u32,
    pub threads: u32,
    /// The most cores and threads a package can hold according to CPUID.
    /// `None` when CPUID does not say, or says less than the package has.
    pub max_cores: Option<u32>,
    pub max_threads: Option<u32>,
    /// Cores of the first package that Windows schedules ahead of their peers,
    /// by core index. Empty when it ranks them all alike.
    pub preferred_cores: Vec<u32>,
    pub sockets: u32,
    pub core_sets: Vec<CoreSet>,
    pub logical: Vec<LogicalProcessor>,
    pub microcode: Option<u32>,
    /// Advertised base frequency in MHz (CPUID leaf 0x16, brand string or firmware).
    pub nominal_mhz: Option<f64>,
    pub max_cpuid_leaf: u32,
    pub max_ext_cpuid_leaf: u32,
    pub thread_dumps: Vec<ThreadDump>,
}

impl CpuInfo {
    pub fn is_hybrid(&self) -> bool {
        self.core_sets.len() > 1
    }

    /// The caches of one level and kind. More than one entry means the package
    /// mixes geometries there, as with a stacked L3 on one core complex only.
    pub fn caches_of(&self, level: u8, kind: CacheKind) -> Vec<&CacheInfo> {
        self.caches.iter().filter(|c| c.level == level && c.kind == kind).collect()
    }
}

// ---------------------------------------------------------------------------
// Clocks (sampled live, not part of `SystemInfo`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CoreClock {
    /// Core index inside the package.
    pub core: u32,
    pub kind: CoreKind,
    pub mhz: f64,
    pub multiplier: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClockSnapshot {
    pub cores: Vec<CoreClock>,
    pub bus_mhz: Option<f64>,
    /// Lowest multiplier the processor supports. Live sampling cannot tell: it takes ring 0.
    pub multiplier_min: Option<f64>,
    /// Highest multiplier the processor supports, as it or the firmware states it. A core's
    /// multiplier is measured and may read a little above it.
    pub multiplier_max: Option<f64>,
    pub rated_fsb_mhz: Option<f64>,
}

/// Frequency of one of the system time sources, for the report's Timers section.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TimerFrequency {
    pub name: String,
    pub hz: f64,
}

// ---------------------------------------------------------------------------
// Mainboard
// ---------------------------------------------------------------------------

/// Vendor / model / revision triple used for the chipset, southbridge and LPCIO rows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BridgeInfo {
    pub vendor: String,
    pub model: String,
    /// Two hex digits, e.g. `51`; empty when unknown.
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BiosInfo {
    pub brand: String,
    pub version: String,
    pub date: String,
    pub uefi: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PcieLink {
    pub width: Option<u32>,
    pub max_width: Option<u32>,
    /// Link speed in GT/s.
    pub speed_gts: Option<f32>,
    pub max_speed_gts: Option<f32>,
}

impl PcieLink {
    /// PCI-Express generation for a link speed: 2.5 -> 1, 5 -> 2, 8 -> 3, 16 -> 4, 32 -> 5, 64 -> 6.
    pub fn generation(speed_gts: f32) -> Option<u32> {
        match speed_gts {
            s if s < 2.0 => None,
            s if s < 4.0 => Some(1),
            s if s < 7.0 => Some(2),
            s if s < 12.0 => Some(3),
            s if s < 24.0 => Some(4),
            s if s < 48.0 => Some(5),
            _ => Some(6),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct GraphicInterface {
    /// e.g. `PCI-Express 4.0`.
    pub bus: String,
    pub link: PcieLink,
}

#[derive(Debug, Clone, Default)]
pub struct MainboardInfo {
    pub manufacturer: String,
    pub model: String,
    pub version: String,
    /// e.g. `PCI-Express 5.0 (32.0 GT/s)`.
    pub bus_specs: Option<String>,
    pub chipset: Option<BridgeInfo>,
    pub southbridge: Option<BridgeInfo>,
    /// Needs port I/O; `None` in user mode.
    pub lpcio: Option<BridgeInfo>,
    pub bios: BiosInfo,
    pub graphic_interface: Option<GraphicInterface>,
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

/// Active memory timings. All of them need a ring-0 backend, so they are `None` in user mode.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MemoryTimings {
    pub cas_latency: Option<f32>,
    pub trcd: Option<u32>,
    pub trp: Option<u32>,
    pub tras: Option<u32>,
    pub trc: Option<u32>,
    /// e.g. `1T`.
    pub command_rate: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct MemoryInfo {
    /// e.g. `DDR5`.
    pub kind: Option<String>,
    pub size_mb: u64,
    /// e.g. `2 x 32-bit`.
    pub channels: Option<String>,
    pub dc_mode: Option<String>,
    /// Memory controller / uncore frequency in MHz.
    pub controller_mhz: Option<f64>,
    /// DRAM clock in MHz (half of the DDR data rate).
    pub dram_mhz: Option<f64>,
    /// Highest DRAM clock in MHz the installed modules are all rated for.
    pub max_dram_mhz: Option<f64>,
    /// e.g. `1:26`.
    pub fsb_dram: Option<String>,
    pub timings: MemoryTimings,
}

/// One column of the SPD tab's timings table.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TimingProfile {
    /// e.g. `JEDEC #8`, `XMP-5600`, `EXPO-5200`.
    pub name: String,
    pub frequency_mhz: Option<f64>,
    pub cas_latency: Option<f32>,
    pub trcd: Option<u32>,
    pub trp: Option<u32>,
    pub tras: Option<u32>,
    pub trc: Option<u32>,
    pub command_rate: Option<String>,
    pub voltage: Option<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct MemoryModule {
    /// e.g. `Slot #1`.
    pub slot: String,
    /// Firmware designation, e.g. `DIMMA2`.
    pub locator: String,
    pub populated: bool,
    /// e.g. `DDR5`.
    pub kind: Option<String>,
    /// e.g. `UDIMM`, `SODIMM`.
    pub form_factor: Option<String>,
    pub size_mb: u64,
    /// e.g. `DDR5-5600 (2800 MHz)`.
    pub max_bandwidth: Option<String>,
    pub manufacturer: Option<String>,
    pub dram_manufacturer: Option<String>,
    pub part_number: Option<String>,
    pub serial_number: Option<String>,
    /// Manufacturing week and two-digit year.
    pub week_year: Option<(u8, u8)>,
    /// e.g. `XMP 3.0`, `EXPO 1.0`.
    pub spd_ext: Option<String>,
    /// e.g. `None`, `ECC`.
    pub correction: Option<String>,
    pub registered: Option<bool>,
    pub buffered: Option<bool>,
    pub ranks: Option<u32>,
    /// Configured speed as a DDR data rate in MT/s.
    pub configured_speed: Option<u32>,
    pub voltage: Option<f32>,
    pub profiles: Vec<TimingProfile>,
}

// ---------------------------------------------------------------------------
// Graphics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Default)]
pub struct GpuPerfLevel {
    /// e.g. `Current`, `Base`, `Boost`.
    pub name: String,
    pub core_mhz: Option<f64>,
    pub shader_mhz: Option<f64>,
    pub memory_mhz: Option<f64>,
}

impl GpuPerfLevel {
    /// Name of the level that holds the clocks the adapter is running at.
    pub const CURRENT: &'static str = "Current";
}

#[derive(Debug, Clone, Default)]
pub struct GpuInfo {
    pub name: String,
    pub board_manufacturer: Option<String>,
    pub codename: Option<String>,
    pub revision: Option<String>,
    pub technology_nm: Option<u32>,
    pub tdp_w: Option<f32>,
    pub rops: Option<u32>,
    pub tmus: Option<u32>,
    /// Shader / CUDA / execution cores.
    pub cores: Option<u32>,
    /// In the order the reports list them, which need not start with the live clocks.
    pub perf_levels: Vec<GpuPerfLevel>,
    pub memory_mb: Option<u64>,
    /// e.g. `GDDR6`.
    pub memory_type: Option<String>,
    pub memory_vendor: Option<String>,
    pub memory_bus_width: Option<u32>,
    pub vendor_id: u16,
    pub device_id: u16,
    pub subsys_vendor_id: u16,
    pub subsys_id: u16,
    pub revision_id: u8,
    /// PCI location (bus, device, function) when it could be matched.
    pub location: Option<(u8, u8, u8)>,
    pub link: Option<PcieLink>,
    pub driver_version: Option<String>,
    pub driver_date: Option<String>,
    pub wddm_model: Option<String>,
    pub primary: bool,
}

impl GpuInfo {
    /// Index of the performance level with the live clocks, the one a view starts on.
    /// The first level when none goes by that name.
    pub fn current_perf_level(&self) -> usize {
        self.perf_levels.iter().position(|level| level.name == GpuPerfLevel::CURRENT).unwrap_or(0)
    }
}

/// A display as its EDID describes it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MonitorInfo {
    /// Product name, e.g. `EX2700Q`.
    pub model: Option<String>,
    /// Manufacturer letters and product code, e.g. `EXM2700`.
    pub id: Option<String>,
    pub serial: Option<String>,
    /// Week of manufacture, 1 to 54, when the display states one.
    pub week: Option<u8>,
    /// Year of manufacture, or the model year for displays that state that instead.
    pub year: Option<u16>,
    /// Diagonal of the picture in inches.
    pub size_inches: Option<f32>,
    /// The largest mode the display lists with full timings.
    pub max_resolution: Option<DisplayMode>,
    /// Lowest and highest line frequency in kHz.
    pub horizontal_khz: Option<(u32, u32)>,
    /// Lowest and highest refresh rate in Hz.
    pub vertical_hz: Option<(u32, u32)>,
    pub max_pixel_clock_mhz: Option<u32>,
    pub gamma: Option<f32>,
    /// e.g. `Display Port`, `HDMI`, `Analog`.
    pub video_input: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    /// Refresh rate in whole Hz, rounded down.
    pub refresh_hz: u32,
}

// ---------------------------------------------------------------------------
// Storage, USB
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DriveInfo {
    /// Number of the physical drive, the `N` of `\\.\PhysicalDriveN`.
    pub number: u32,
    /// Path of the drive's disk device interface.
    pub device_path: String,
    pub name: String,
    /// Firmware revision.
    pub revision: Option<String>,
    pub serial: Option<String>,
    pub capacity_bytes: Option<u64>,
    /// The medium can be taken out of the drive.
    pub removable: bool,
    /// Windows knows the drive to have no seek penalty.
    pub solid_state: bool,
    /// The bus as Windows numbers it (`STORAGE_BUS_TYPE`), e.g. 17 for NVMe.
    pub bus_type: Option<u32>,
    /// e.g. `SMART`.
    pub features: Vec<String>,
    pub controller: Option<DriveController>,
    /// The mounted volumes that lie on the drive, by drive letter.
    pub volumes: Vec<VolumeInfo>,
}

/// The PCI device a drive is attached to.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DriveController {
    /// e.g. `NVM Express (NVMe) Controller`.
    pub name: String,
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    /// The controller's PCI Express link, when it is the drive's own.
    pub link: Option<PcieLink>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VolumeInfo {
    /// Root of the volume, e.g. `c:\`.
    pub mount: String,
    pub size_bytes: u64,
    pub free_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UsbDevice {
    /// Device description reported by Windows.
    pub name: String,
    /// Class and subclass code of the device descriptor. `None` when the hub the
    /// device is on did not hand the descriptor out.
    pub class: Option<(u8, u8)>,
    pub vendor_id: u16,
    pub product_id: u16,
}

// ---------------------------------------------------------------------------
// Software, PCI
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OsInfo {
    /// e.g. `Microsoft Windows 11 Home (x64)`.
    pub name: String,
    /// e.g. `Build 26100.2314`.
    pub build: String,
    /// e.g. `12.0`.
    pub directx: Option<String>,
    pub computer_name: String,
    /// e.g. `11/14/2023`.
    pub install_date: Option<String>,
    /// Hardware errors Windows has on record in its System event log.
    /// `None` when the log could not be read.
    pub whea_errors: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PciDevice {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub subsys_vendor_id: u16,
    pub subsys_id: u16,
    pub revision: u8,
    pub base_class: u8,
    pub sub_class: u8,
    pub prog_if: u8,
    /// Device description reported by Windows.
    pub description: String,
    pub link: Option<PcieLink>,
}

impl PciDevice {
    pub fn is_class(&self, base: u8, sub: u8) -> bool {
        self.base_class == base && self.sub_class == sub
    }
}

/// `32 GBytes`, `512 MBytes`, `15.9 GBytes` - memory sizes as shown in the UI.
pub fn size_text(mb: u64) -> String {
    if mb == 0 {
        String::new()
    } else if mb >= 1024 {
        if mb % 1024 == 0 {
            format!("{} GBytes", mb / 1024)
        } else {
            format!("{:.1} GBytes", mb as f64 / 1024.0)
        }
    } else {
        format!("{mb} MBytes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_size_text() {
        let mut c = CacheInfo {
            level: 1,
            kind: CacheKind::Data,
            size_kb: 32,
            count: 8,
            ways: 8,
            ..Default::default()
        };
        assert_eq!(c.size_text(), "8 x 32 KBytes");
        assert_eq!(c.short_size_text(), "8 x 32 KB");
        assert_eq!(c.ways_text(), "8-way");
        c = CacheInfo { level: 3, size_kb: 32 * 1024, count: 1, ways: 16, ..Default::default() };
        assert_eq!(c.size_text(), "32 MBytes");
        assert_eq!(c.short_size_text(), "32 MB");
        c = CacheInfo { level: 2, size_kb: 1280, count: 6, ways: 10, ..Default::default() };
        assert_eq!(c.size_text(), "6 x 1.25 MBytes");
        assert_eq!(c.short_size_text(), "6 x 1.25 MB");
        c = CacheInfo {
            level: 1,
            kind: CacheKind::Trace,
            size_kb: 12,
            count: 2,
            ways: u32::MAX,
            ..Default::default()
        };
        assert_eq!(c.size_text(), "2 x 12 Kuops");
        assert_eq!(c.ways_text(), "fully");
    }

    #[test]
    fn memory_size_text() {
        assert_eq!(size_text(0), "");
        assert_eq!(size_text(512), "512 MBytes");
        assert_eq!(size_text(32 * 1024), "32 GBytes");
        assert_eq!(size_text(16_280), "15.9 GBytes");
    }

    #[test]
    fn finds_the_live_perf_level() {
        let level = |name: &str| GpuPerfLevel { name: name.into(), ..Default::default() };
        let mut gpu = GpuInfo {
            perf_levels: vec![level("Base"), level("Boost"), level("Current")],
            ..Default::default()
        };
        assert_eq!(gpu.current_perf_level(), 2);
        gpu.perf_levels.truncate(2);
        assert_eq!(gpu.current_perf_level(), 0);
        gpu.perf_levels.clear();
        assert_eq!(gpu.current_perf_level(), 0);
    }

    #[test]
    fn pcie_generation() {
        assert_eq!(PcieLink::generation(2.5), Some(1));
        assert_eq!(PcieLink::generation(8.0), Some(3));
        assert_eq!(PcieLink::generation(16.0), Some(4));
        assert_eq!(PcieLink::generation(32.0), Some(5));
        assert_eq!(PcieLink::generation(0.0), None);
    }
}
