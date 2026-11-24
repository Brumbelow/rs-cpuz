//! Display adapter details.
//!
//! DXGI lists the adapters and the graphics kernel tells the driver model and bus
//! address of each. The display class key of the registry adds the driver details, the
//! PCI device list the link, and tables keyed by PCI id supply what no API reports (code
//! name, process node, reference specifications). On NVIDIA hardware the driver's NVAPI
//! library contributes the live clocks. Display devices the PCI list has but DXGI does
//! not, cards without a working driver, are described from the PCI side alone.

mod db;
mod driver;
mod dxgi;
mod kmt;

pub use kmt::Performance;
mod nvapi;

use super::pci;
use crate::model::{GpuInfo, GpuPerfLevel, PciDevice};
use driver::Driver;
use dxgi::Adapter;

const MIB: u64 = 1024 * 1024;

/// PCI base class of display controllers.
const CLASS_DISPLAY: u8 = 0x03;

/// The interfaces adapters are read through, by the names the reports list them under.
const API_D3D: &str = "D3D";
const API_NVAPI: &str = "NVIDIA NVAPI";

/// Lists the display adapters, primary first, and after them the cards in `pci` that
/// have no working driver. With them come the names of the interfaces that answered.
pub fn detect(pci: &[PciDevice]) -> (Vec<GpuInfo>, Vec<String>) {
    let adapters = dxgi::enumerate();
    let drivers = driver::installed();
    let has_nvidia = adapters.iter().flatten().any(|adapter| adapter.vendor_id == db::NVIDIA);
    let nvidia = has_nvidia.then(nvapi::query).flatten();
    let apis = answered(&[(API_D3D, adapters.is_some()), (API_NVAPI, nvidia.is_some())]);
    let gpus = assemble(&adapters.unwrap_or_default(), &drivers, pci, &nvidia.unwrap_or_default());
    (gpus, apis)
}

/// The names of the interfaces that answered, in the order given.
fn answered(apis: &[(&str, bool)]) -> Vec<String> {
    apis.iter().filter(|(_, answered)| *answered).map(|(name, _)| name.to_string()).collect()
}

/// Builds one record per adapter from the gathered sources, followed by one per display
/// device that has no adapter. `adapters` is primary first.
///
/// Some driver stacks make DXGI present one card as two adapters. Where the PCI list
/// (or, without one, the NVIDIA driver) knows fewer cards with an adapter's ids than
/// DXGI listed, the surplus entries are left out.
fn assemble(
    adapters: &[Adapter],
    drivers: &[Driver],
    pci: &[PciDevice],
    nvidia: &[nvapi::Gpu],
) -> Vec<GpuInfo> {
    let mut kept: Vec<&Adapter> = Vec::with_capacity(adapters.len());
    for adapter in adapters {
        let twins = kept.iter().filter(|other| other.same_hardware(adapter)).count();
        if card_count(adapter, pci, nvidia).is_some_and(|cards| twins >= cards) {
            continue;
        }
        kept.push(adapter);
    }
    let places = place(&kept, pci);
    let mut nvidia_taken = vec![false; nvidia.len()];
    let mut gpus = Vec::with_capacity(kept.len());
    for (adapter, place) in kept.iter().zip(&places) {
        let mut gpu = describe(adapter, driver::select(drivers, adapter));
        gpu.primary = gpus.is_empty();
        if let Some(device) = place.map(|found| &pci[found]) {
            gpu.location = Some((device.bus, device.device, device.function));
            gpu.link = device.link;
        }
        let bus = gpu.location.or(adapter.address).map(|(bus, _, _)| bus);
        if let Some(found) = nvapi::pick(nvidia, &nvidia_taken, adapter, bus) {
            nvidia_taken[found] = true;
            apply_live(&mut gpu, &nvidia[found]);
        }
        // The vendor's own interface knows the rated clocks; for the others the graphics
        // kernel tells the clocks in effect.
        if gpu.perf_levels.is_empty() {
            gpu.perf_levels = kernel_levels(&adapter.performance);
        }
        gpus.push(gpu);
    }
    gpus.extend(without_adapter(&kept, &places, pci));
    gpus
}

/// Pairs each adapter with its PCI device, as an index into `pci`.
///
/// The bus address the graphics kernel reports for an adapter settles it, so those
/// devices are set aside before anything else. An adapter without an address then takes
/// the best fit among the devices still free, which pairs identical cards in bus order.
fn place(adapters: &[&Adapter], pci: &[PciDevice]) -> Vec<Option<usize>> {
    let mut taken = vec![false; pci.len()];
    let mut places = vec![None; adapters.len()];
    for by_address in [true, false] {
        for (place, adapter) in places.iter_mut().zip(adapters) {
            if place.is_some() {
                continue;
            }
            *place =
                if by_address { at_address(adapter, pci, &taken) } else { match_pci(adapter, pci, &taken) };
            if let Some(found) = *place {
                taken[found] = true;
            }
        }
    }
    places
}

/// The cards DXGI does not list: display devices on the PCI bus that no adapter was
/// paired with. A card whose driver is missing, disabled or failed to start is one.
///
/// An adapter that could not be paired may still be one of these devices, so each of
/// them stands in for one device with its vendor and device id rather than being listed twice.
fn without_adapter(adapters: &[&Adapter], places: &[Option<usize>], pci: &[PciDevice]) -> Vec<GpuInfo> {
    let mut unplaced: Vec<(u16, u16)> = adapters
        .iter()
        .zip(places)
        .filter(|(_, place)| place.is_none())
        .map(|(adapter, _)| (adapter.vendor_id, adapter.device_id))
        .collect();
    let mut gpus = Vec::new();
    for (index, device) in pci.iter().enumerate() {
        if device.base_class != CLASS_DISPLAY
            || places.contains(&Some(index))
            || is_extra_function(device, pci)
        {
            continue;
        }
        if let Some(listed) = unplaced.iter().position(|&ids| ids == (device.vendor_id, device.device_id)) {
            unplaced.swap_remove(listed);
            continue;
        }
        gpus.push(describe_device(device));
    }
    gpus
}

/// Whether `device` is a further display function of a card whose function 0 is one
/// too, such as the second head of a dual-head card or a virtual function of a shared
/// GPU. Those are not cards of their own.
fn is_extra_function(device: &PciDevice, pci: &[PciDevice]) -> bool {
    device.function != 0
        && pci.iter().any(|other| {
            other.base_class == CLASS_DISPLAY
                && (other.bus, other.device, other.function) == (device.bus, device.device, 0)
        })
}

/// A card known from the PCI device list only. Its name is the description Windows has
/// for the device, and everything a running driver would report stays unknown.
fn describe_device(device: &PciDevice) -> GpuInfo {
    let adapter = Adapter {
        name: device.description.clone(),
        vendor_id: device.vendor_id,
        device_id: device.device_id,
        subsys_vendor_id: device.subsys_vendor_id,
        subsys_id: device.subsys_id,
        revision_id: device.revision,
        ..Default::default()
    };
    GpuInfo {
        location: Some((device.bus, device.device, device.function)),
        link: device.link,
        ..describe(&adapter, None)
    }
}

/// How many cards with the adapter's PCI ids exist, when the PCI list or - without a
/// matching entry there - the NVIDIA driver can tell.
fn card_count(adapter: &Adapter, pci: &[PciDevice], nvidia: &[nvapi::Gpu]) -> Option<usize> {
    let on_bus = pci.iter().filter(|device| pci_score(adapter, device).is_some()).count();
    let driven = nvidia.iter().filter(|gpu| gpu.describes(adapter)).count();
    [on_bus, driven].into_iter().find(|&count| count > 0)
}

/// Everything that follows from the adapter's ids and its driver entry.
fn describe(adapter: &Adapter, driver: Option<&Driver>) -> GpuInfo {
    let chip = db::chip(adapter.vendor_id, adapter.device_id, adapter.revision_id);
    let model = db::model(adapter.vendor_id, adapter.device_id);
    let memory_type = model.and_then(|model| model.memory_type).or(chip.and_then(|chip| chip.memory_type));
    let driver_version = driver
        .and_then(|driver| driver.version.clone())
        .or_else(|| adapter.driver_version.map(driver::version_text));
    GpuInfo {
        name: adapter.name.clone(),
        board_manufacturer: pci::vendor_name(adapter.subsys_vendor_id).map(String::from),
        codename: chip.map(|chip| chip.codename.to_string()),
        revision: (adapter.vendor_id != 0).then(|| revision_text(adapter.revision_id)),
        technology_nm: chip.and_then(|chip| chip.technology_nm),
        tdp_w: model.and_then(|model| model.tdp_w).map(f32::from),
        rops: model.and_then(|model| model.rops),
        tmus: db::texture_units(chip, model),
        cores: model.and_then(|model| model.cores),
        perf_levels: Vec::new(),
        memory_mb: memory_mb(adapter, driver),
        memory_type: memory_type.map(String::from),
        memory_vendor: None,
        memory_bus_width: model.and_then(|model| model.memory_bus_width),
        vendor_id: adapter.vendor_id,
        device_id: adapter.device_id,
        subsys_vendor_id: adapter.subsys_vendor_id,
        subsys_id: adapter.subsys_id,
        revision_id: adapter.revision_id,
        location: None,
        link: None,
        wddm_model: adapter.wddm_version.map(wddm_text),
        driver_version,
        driver_date: driver.and_then(|driver| driver.date.clone()),
        primary: false,
    }
}

/// The performance levels the graphics kernel's readings make: the highest clocks the
/// driver allows (`Max`, which can lie above the rated boost clock), then the clocks in
/// effect (`Current`). A level without a single reading is left out.
fn kernel_levels(performance: &kmt::Performance) -> Vec<GpuPerfLevel> {
    let mhz = |hz: Option<u64>| hz.map(|hz| hz as f64 / 1e6);
    let level = |name: &str, core: Option<u64>, memory: Option<u64>| {
        (core.is_some() || memory.is_some()).then(|| GpuPerfLevel {
            name: name.into(),
            core_mhz: mhz(core),
            shader_mhz: None,
            memory_mhz: mhz(memory),
        })
    };
    [
        level("Max", performance.max_core_hz, performance.max_memory_hz),
        level(GpuPerfLevel::CURRENT, performance.core_hz, performance.memory_hz),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// The adapters whose sensors can be read again later: name and locally unique id.
pub fn sensor_sources() -> Vec<(String, i64)> {
    dxgi::enumerate()
        .unwrap_or_default()
        .into_iter()
        .filter(|adapter| !adapter.software)
        .map(|adapter| (adapter.name, adapter.luid))
        .collect()
}

/// The temperature and fan readings of the adapter with id `luid`, as the graphics kernel has them.
pub fn performance(luid: i64) -> Performance {
    kmt::performance(dxgi::luid(luid))
}

/// Values read from the running driver take precedence over the tables.
fn apply_live(gpu: &mut GpuInfo, live: &nvapi::Gpu) {
    gpu.cores = live.cores.or(gpu.cores);
    gpu.memory_bus_width = live.memory_bus_width.or(gpu.memory_bus_width);
    gpu.perf_levels = live.perf_levels.clone();
}

/// Video memory in MiB. The size the driver recorded in the registry is preferred: it
/// is the installed amount, where DXGI reports what is left over for applications.
fn memory_mb(adapter: &Adapter, driver: Option<&Driver>) -> Option<u64> {
    let bytes = driver.and_then(|driver| driver.memory_bytes).unwrap_or(adapter.dedicated_bytes);
    (bytes >= MIB).then_some(bytes / MIB)
}

/// The PCI revision id as two hex digits. NVIDIA encodes the silicon revision as a
/// letter and a digit in it, so 0xA1 reads as revision `A1`.
fn revision_text(revision_id: u8) -> String {
    format!("{revision_id:02X}")
}

/// `2.7` for 2700: the graphics kernel numbers the driver model an adapter runs under
/// as the WDDM version times 1000. The model is not read off the driver version, which
/// only says what the package was built for; an older Windows runs it as an older model.
fn wddm_text(version: u32) -> String {
    format!("{}.{}", version / 1000, version % 1000 / 100)
}

/// Index of the free PCI device at the bus address the graphics kernel reported for
/// `adapter`. The device still has to carry the adapter's ids.
fn at_address(adapter: &Adapter, pci: &[PciDevice], taken: &[bool]) -> Option<usize> {
    let address = adapter.address?;
    pci.iter().enumerate().position(|(index, device)| {
        !taken.get(index).copied().unwrap_or(false)
            && (device.bus, device.device, device.function) == address
            && pci_score(adapter, device).is_some()
    })
}

/// Index of the PCI device that is `adapter`, skipping devices already `taken` by an
/// identical card.
///
/// Vendor and device id have to match. A subsystem or revision id that is known on the
/// PCI side and differs rules a device out; among the rest the closest match wins and
/// ties go to the lowest bus address.
fn match_pci(adapter: &Adapter, pci: &[PciDevice], taken: &[bool]) -> Option<usize> {
    // `max_by_key` keeps the last of equal maxima, so walk backwards to favour the first device.
    pci.iter()
        .enumerate()
        .rev()
        .filter(|(index, _)| !taken.get(*index).copied().unwrap_or(false))
        .filter_map(|(index, device)| Some((pci_score(adapter, device)?, index)))
        .max_by_key(|(score, _)| *score)
        .map(|(_, index)| index)
}

/// How well a PCI device fits an adapter; `None` when it is a different device.
fn pci_score(adapter: &Adapter, device: &PciDevice) -> Option<u32> {
    if (device.vendor_id, device.device_id) != (adapter.vendor_id, adapter.device_id)
        || adapter.vendor_id == 0
    {
        return None;
    }
    let mut score = 0;
    let subsys = (device.subsys_vendor_id, device.subsys_id);
    if subsys == (adapter.subsys_vendor_id, adapter.subsys_id) {
        score += 4;
    } else if subsys != (0, 0) {
        return None;
    }
    if device.revision == adapter.revision_id {
        score += 2;
    } else if device.revision != 0 {
        return None;
    }
    if device.base_class == CLASS_DISPLAY {
        score += 1;
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GpuPerfLevel, PcieLink};

    fn geforce() -> Adapter {
        Adapter {
            name: "Example GeForce".into(),
            vendor_id: 0x10DE,
            device_id: 0x2882,
            subsys_vendor_id: 0x1462,
            subsys_id: 0x5161,
            revision_id: 0xA1,
            dedicated_bytes: 8_000 * MIB,
            luid: 1,
            has_output: true,
            drives_primary: true,
            driver_version: Some([32, 0, 15, 6094]),
            wddm_version: Some(3200),
            ..Default::default()
        }
    }

    fn integrated() -> Adapter {
        Adapter {
            name: "Example Radeon Graphics".into(),
            vendor_id: 0x1002,
            device_id: 0x164E,
            subsys_vendor_id: 0x1043,
            subsys_id: 0x8877,
            revision_id: 0xC5,
            dedicated_bytes: 496 * MIB,
            luid: 2,
            wddm_version: Some(3100),
            ..Default::default()
        }
    }

    fn drivers() -> Vec<Driver> {
        vec![
            Driver {
                description: "Example Radeon Graphics".into(),
                hardware_id: driver::parse_hardware_id(r"PCI\VEN_1002&DEV_164E&SUBSYS_88771043&REV_C5")
                    .unwrap(),
                version: Some("31.0.14046.0".into()),
                date: Some("03/29/2023".into()),
                memory_bytes: Some(512 * MIB),
            },
            Driver {
                description: "Example GeForce".into(),
                hardware_id: driver::parse_hardware_id(r"pci\ven_10de&dev_2882&subsys_51611462").unwrap(),
                version: Some("32.0.15.6094".into()),
                date: Some("08/14/2024".into()),
                memory_bytes: Some(8188 * MIB),
            },
        ]
    }

    fn pci_device(bus: u8, vendor_id: u16, device_id: u16, subsys: (u16, u16), revision: u8) -> PciDevice {
        PciDevice {
            bus,
            vendor_id,
            device_id,
            subsys_vendor_id: subsys.0,
            subsys_id: subsys.1,
            revision,
            base_class: CLASS_DISPLAY,
            ..Default::default()
        }
    }

    fn link(width: u32, speed_gts: f32) -> PcieLink {
        PcieLink {
            width: Some(width),
            max_width: Some(width),
            speed_gts: Some(speed_gts),
            max_speed_gts: Some(speed_gts),
        }
    }

    fn level(core_mhz: f64) -> GpuPerfLevel {
        GpuPerfLevel { name: "Current".into(), core_mhz: Some(core_mhz), ..Default::default() }
    }

    /// The NVIDIA driver's entry for an example GeForce on `bus`, running at `core_mhz`.
    fn live(bus: u32, core_mhz: f64) -> nvapi::Gpu {
        nvapi::Gpu {
            device_id: 0x2882,
            subsys: 0x5161_1462,
            revision: 0xA1,
            bus: Some(bus),
            perf_levels: vec![level(core_mhz)],
            ..Default::default()
        }
    }

    #[test]
    fn lists_the_interfaces_that_answered() {
        assert_eq!(answered(&[(API_D3D, true), (API_NVAPI, true)]), ["D3D", "NVIDIA NVAPI"]);
        assert_eq!(answered(&[(API_D3D, true), (API_NVAPI, false)]), ["D3D"]);
        assert!(answered(&[(API_D3D, false), (API_NVAPI, false)]).is_empty());
    }

    #[test]
    fn the_graphics_kernel_supplies_the_clocks_nvapi_does_not() {
        let measured = kmt::Performance {
            core_hz: Some(450_000_000),
            max_core_hz: Some(1_600_000_000),
            memory_hz: None,
            max_memory_hz: Some(2_400_000_000),
            ..kmt::Performance::default()
        };
        let mhz = |core: Option<f64>, memory: Option<f64>, name: &str| GpuPerfLevel {
            name: name.into(),
            core_mhz: core,
            memory_mhz: memory,
            ..GpuPerfLevel::default()
        };
        assert_eq!(
            kernel_levels(&measured),
            [mhz(Some(1600.0), Some(2400.0), "Max"), mhz(Some(450.0), None, GpuPerfLevel::CURRENT)]
        );
        // Without a reading there is no level, and a level needs one reading.
        assert!(kernel_levels(&kmt::Performance::default()).is_empty());
        let max_only = kmt::Performance { max_core_hz: Some(1_600_000_000), ..kmt::Performance::default() };
        assert_eq!(kernel_levels(&max_only).len(), 1);
    }

    #[test]
    fn assembles_adapters_from_all_sources() {
        let pci = [
            PciDevice { base_class: 0x06, ..pci_device(0, 0x1022, 0x14D8, (0x1043, 0x8877), 0x00) },
            PciDevice { link: Some(link(8, 16.0)), ..pci_device(1, 0x10DE, 0x2882, (0x1462, 0x5161), 0xA1) },
            // The audio function of the card shares everything but the device id and the class.
            PciDevice {
                function: 1,
                base_class: 0x04,
                ..pci_device(1, 0x10DE, 0x22BE, (0x1462, 0x5161), 0xA1)
            },
            pci_device(16, 0x1002, 0x164E, (0x1043, 0x8877), 0xC5),
        ];
        let current = GpuPerfLevel {
            name: "Current".into(),
            core_mhz: Some(285.0),
            shader_mhz: None,
            memory_mhz: Some(5001.0),
        };
        let nvidia = [nvapi::Gpu {
            device_id: 0x2882,
            subsys: 0x5161_1462,
            revision: 0xA1,
            bus: Some(1),
            cores: Some(3000),
            memory_bus_width: None,
            perf_levels: vec![current.clone()],
        }];

        let gpus = assemble(&[geforce(), integrated()], &drivers(), &pci, &nvidia);
        assert_eq!(gpus.len(), 2);

        let card = &gpus[0];
        assert!(card.primary);
        assert_eq!(card.name, "Example GeForce");
        assert_eq!(card.board_manufacturer.as_deref(), Some("MSI"));
        assert_eq!(card.codename.as_deref(), Some("AD107"));
        assert_eq!(card.revision.as_deref(), Some("A1"));
        assert_eq!(card.technology_nm, Some(5));
        assert_eq!(card.tdp_w, Some(115.0));
        assert_eq!(card.rops, Some(48));
        // Derived from the table's core count: four per streaming multiprocessor.
        assert_eq!(card.tmus, Some(96));
        // The live core count replaces the table's, the table fills the missing bus width.
        assert_eq!(card.cores, Some(3000));
        assert_eq!(card.memory_bus_width, Some(128));
        assert_eq!(card.perf_levels, [current]);
        assert_eq!(card.memory_mb, Some(8188));
        assert_eq!(card.memory_type.as_deref(), Some("GDDR6"));
        assert_eq!(card.memory_vendor, None);
        assert_eq!((card.vendor_id, card.device_id), (0x10DE, 0x2882));
        assert_eq!((card.subsys_vendor_id, card.subsys_id, card.revision_id), (0x1462, 0x5161, 0xA1));
        assert_eq!(card.location, Some((1, 0, 0)));
        assert_eq!(card.link, Some(link(8, 16.0)));
        assert_eq!(card.driver_version.as_deref(), Some("32.0.15.6094"));
        assert_eq!(card.driver_date.as_deref(), Some("08/14/2024"));
        assert_eq!(card.wddm_model.as_deref(), Some("3.2"));

        let igpu = &gpus[1];
        assert!(!igpu.primary);
        assert_eq!(igpu.board_manufacturer.as_deref(), Some("ASUS"));
        assert_eq!(igpu.codename.as_deref(), Some("Raphael"));
        assert_eq!(igpu.revision.as_deref(), Some("C5"));
        assert_eq!(igpu.technology_nm, Some(6));
        assert_eq!(igpu.cores, Some(128));
        assert_eq!(igpu.memory_mb, Some(512));
        assert_eq!((igpu.memory_type.as_deref(), igpu.memory_bus_width, igpu.tdp_w), (None, None, None));
        assert!(igpu.perf_levels.is_empty());
        assert_eq!(igpu.location, Some((16, 0, 0)));
        assert_eq!(igpu.link, None);
        assert_eq!(igpu.driver_version.as_deref(), Some("31.0.14046.0"));
        assert_eq!(igpu.driver_date.as_deref(), Some("03/29/2023"));
        assert_eq!(igpu.wddm_model.as_deref(), Some("3.1"));
    }

    #[test]
    fn degrades_without_optional_sources() {
        let gpus = assemble(&[geforce()], &[], &[], &[]);
        let card = &gpus[0];
        assert!(card.primary);
        assert_eq!(card.codename.as_deref(), Some("AD107"));
        assert_eq!(card.cores, Some(3072));
        // DXGI still knows the memory size and the version of the loaded driver.
        assert_eq!(card.memory_mb, Some(8000));
        assert_eq!(card.driver_version.as_deref(), Some("32.0.15.6094"));
        assert_eq!(card.wddm_model.as_deref(), Some("3.2"));
        assert_eq!(card.driver_date, None);
        assert_eq!((card.location, card.link), (None, None));
        assert!(card.perf_levels.is_empty());
        assert!(assemble(&[], &drivers(), &[], &[]).is_empty());
    }

    #[test]
    fn driver_model_is_the_one_in_effect() {
        // Windows 10 runs the same 32.x driver package as WDDM 2.7.
        let on_windows_10 = Adapter { wddm_version: Some(2700), ..geforce() };
        let gpu = &assemble(&[on_windows_10], &drivers(), &[], &[])[0];
        assert_eq!(gpu.driver_version.as_deref(), Some("32.0.15.6094"));
        assert_eq!(gpu.wddm_model.as_deref(), Some("2.7"));

        // Without an answer from the graphics kernel the model is not guessed.
        let unanswered = Adapter { wddm_version: None, ..geforce() };
        assert_eq!(assemble(&[unanswered], &drivers(), &[], &[])[0].wddm_model, None);

        assert_eq!(wddm_text(1000), "1.0");
        assert_eq!(wddm_text(1105), "1.1");
        assert_eq!(wddm_text(2000), "2.0");
        assert_eq!(wddm_text(3100), "3.1");
        assert_eq!(wddm_text(3200), "3.2");
    }

    #[test]
    fn lists_cards_that_have_no_adapter() {
        let card = PciDevice {
            description: "Video Controller (VGA Compatible)".into(),
            link: Some(link(8, 16.0)),
            ..pci_device(1, 0x10DE, 0x2882, (0x1462, 0x5161), 0xA1)
        };
        let pci = [
            PciDevice { base_class: 0x06, ..pci_device(0, 0x1022, 0x14D8, (0x1043, 0x8877), 0x00) },
            card,
            pci_device(16, 0x1002, 0x164E, (0x1043, 0x8877), 0xC5),
        ];

        // No driver is loaded for either display device, so DXGI has nothing.
        let gpus = assemble(&[], &drivers(), &pci, &[]);
        assert_eq!(gpus.len(), 2);
        let gpu = &gpus[0];
        assert_eq!(gpu.name, "Video Controller (VGA Compatible)");
        assert_eq!(gpu.board_manufacturer.as_deref(), Some("MSI"));
        assert_eq!(gpu.codename.as_deref(), Some("AD107"));
        assert_eq!(gpu.revision.as_deref(), Some("A1"));
        assert_eq!(
            (gpu.technology_nm, gpu.cores, gpu.memory_type.as_deref()),
            (Some(5), Some(3072), Some("GDDR6"))
        );
        assert_eq!((gpu.vendor_id, gpu.device_id), (0x10DE, 0x2882));
        assert_eq!((gpu.subsys_vendor_id, gpu.subsys_id, gpu.revision_id), (0x1462, 0x5161, 0xA1));
        assert_eq!((gpu.location, gpu.link), (Some((1, 0, 0)), Some(link(8, 16.0))));
        // The class key still has an entry for these ids. It does not describe a card
        // whose driver is not running.
        assert_eq!((gpu.driver_version.as_deref(), gpu.driver_date.as_deref()), (None, None));
        assert_eq!((gpu.wddm_model.as_deref(), gpu.memory_mb), (None, None));
        assert!(!gpu.primary && gpu.perf_levels.is_empty());
        assert_eq!((gpus[1].codename.as_deref(), gpus[1].location), (Some("Raphael"), Some((16, 0, 0))));

        // The integrated adapter works and the card does not: it follows the adapters.
        let gpus = assemble(&[integrated()], &drivers(), &pci, &[]);
        assert_eq!(gpus.len(), 2);
        assert_eq!((gpus[0].name.as_str(), gpus[0].location), ("Example Radeon Graphics", Some((16, 0, 0))));
        assert_eq!(gpus[0].driver_version.as_deref(), Some("31.0.14046.0"));
        assert_eq!(
            (gpus[1].name.as_str(), gpus[1].location),
            ("Video Controller (VGA Compatible)", Some((1, 0, 0)))
        );
        assert!(gpus[0].primary && !gpus[1].primary);

        // Devices of other classes are not graphics cards, whatever else is missing.
        assert!(assemble(&[], &[], &pci[..1], &[]).is_empty());

        // Neither is a further display function of a card. On its own it is all there is.
        let second_head = PciDevice { function: 1, ..pci_device(1, 0x10DE, 0x2883, (0x1462, 0x5161), 0xA1) };
        let gpus = assemble(&[], &[], &[pci[1].clone(), second_head.clone()], &[]);
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].location, Some((1, 0, 0)));
        assert_eq!(assemble(&[], &[], &[second_head], &[])[0].location, Some((1, 0, 1)));
    }

    #[test]
    fn an_unpaired_adapter_is_not_listed_again() {
        // The device list has the card under another subsystem id than DXGI, so the two
        // are not paired.
        let pci = [pci_device(1, 0x10DE, 0x2882, (0x1043, 0x8888), 0xA1)];
        let gpus = assemble(&[geforce()], &[], &pci, &[]);
        assert_eq!(gpus.len(), 1);
        assert_eq!((gpus[0].name.as_str(), gpus[0].location), ("Example GeForce", None));

        // One adapter stands in for one device: a second one with those ids is another card.
        let two = [pci[0].clone(), pci_device(2, 0x10DE, 0x2882, (0x1043, 0x8888), 0xA1)];
        let gpus = assemble(&[geforce()], &[], &two, &[]);
        assert_eq!(gpus.len(), 2);
        assert_eq!(
            (gpus[1].board_manufacturer.as_deref(), gpus[1].location),
            (Some("ASUS"), Some((2, 0, 0)))
        );
    }

    #[test]
    fn unknown_hardware_stays_unknown() {
        let adapter = Adapter {
            name: "Mystery Adapter".into(),
            vendor_id: 0x1234,
            device_id: 0x1111,
            revision_id: 0x02,
            ..Default::default()
        };
        let gpu = &assemble(&[adapter], &drivers(), &[], &[])[0];
        assert_eq!(gpu.name, "Mystery Adapter");
        assert_eq!(gpu.revision.as_deref(), Some("02"));
        assert_eq!((gpu.board_manufacturer.as_deref(), gpu.codename.as_deref()), (None, None));
        assert_eq!(
            (gpu.technology_nm, gpu.cores, gpu.rops, gpu.tmus, gpu.tdp_w),
            (None, None, None, None, None)
        );
        assert_eq!((gpu.memory_mb, gpu.memory_type.as_deref(), gpu.memory_bus_width), (None, None, None));
        assert_eq!((gpu.driver_version.as_deref(), gpu.driver_date.as_deref()), (None, None));
        assert_eq!(gpu.wddm_model, None);

        // An adapter that is not a PCI device has no ids to render or look up.
        let virtual_adapter = Adapter { name: "Virtual Display".into(), ..Default::default() };
        assert_eq!(assemble(&[virtual_adapter], &[], &[], &[])[0].revision, None);
    }

    #[test]
    fn identical_cards_get_distinct_devices() {
        let pci = [
            pci_device(3, 0x10DE, 0x2882, (0x1462, 0x5161), 0xA1),
            pci_device(4, 0x10DE, 0x2882, (0x1462, 0x5161), 0xA1),
        ];
        // NVAPI lists the cards in the opposite order.
        let nvidia = [live(4, 400.0), live(3, 300.0)];
        let second = Adapter { luid: 9, has_output: false, drives_primary: false, ..geforce() };

        // Nothing tells the adapters apart, so they are paired with the devices in bus order.
        let gpus = assemble(&[geforce(), second], &drivers(), &pci, &nvidia);
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].location, Some((3, 0, 0)));
        assert_eq!(gpus[1].location, Some((4, 0, 0)));
        assert_eq!(gpus[0].perf_levels, [level(300.0)]);
        assert_eq!(gpus[1].perf_levels, [level(400.0)]);
        assert!(gpus[0].primary && !gpus[1].primary);
    }

    #[test]
    fn addressed_cards_get_their_own_devices() {
        // The display hangs on the card in the lower slot, which sits on the higher bus.
        let pci = [
            PciDevice { link: Some(link(16, 16.0)), ..pci_device(1, 0x10DE, 0x2882, (0x1462, 0x5161), 0xA1) },
            PciDevice { link: Some(link(4, 16.0)), ..pci_device(5, 0x10DE, 0x2882, (0x1462, 0x5161), 0xA1) },
        ];
        let nvidia = [live(1, 300.0), live(5, 500.0)];
        let primary = Adapter { address: Some((5, 0, 0)), ..geforce() };
        let other = Adapter {
            luid: 9,
            has_output: false,
            drives_primary: false,
            address: Some((1, 0, 0)),
            ..geforce()
        };

        let gpus = assemble(&[primary.clone(), other.clone()], &[], &pci, &nvidia);
        assert_eq!(gpus.len(), 2);
        assert!(gpus[0].primary && !gpus[1].primary);
        assert_eq!((gpus[0].location, gpus[0].link), (Some((5, 0, 0)), Some(link(4, 16.0))));
        assert_eq!((gpus[1].location, gpus[1].link), (Some((1, 0, 0)), Some(link(16, 16.0))));
        assert_eq!(gpus[0].perf_levels, [level(500.0)]);
        assert_eq!(gpus[1].perf_levels, [level(300.0)]);

        // An adapter without an address is not handed the device a later one names.
        let unaddressed = Adapter { address: None, ..primary.clone() };
        let gpus = assemble(&[unaddressed, other.clone()], &[], &pci, &nvidia);
        assert_eq!((gpus[0].location, gpus[1].location), (Some((5, 0, 0)), Some((1, 0, 0))));
        assert_eq!(gpus[0].perf_levels, [level(500.0)]);

        // Without a device list the address still tells the NVIDIA driver's entries apart.
        let gpus = assemble(&[primary, other], &[], &[], &nvidia);
        assert_eq!((gpus[0].location, gpus[1].location), (None, None));
        assert_eq!(gpus[0].perf_levels, [level(500.0)]);
        assert_eq!(gpus[1].perf_levels, [level(300.0)]);
    }

    #[test]
    fn an_address_has_to_name_the_adapters_device() {
        let pci = [
            pci_device(1, 0x10DE, 0x2882, (0x1462, 0x5161), 0xA1),
            pci_device(2, 0x10DE, 0x2882, (0x1462, 0x5161), 0xA1),
            pci_device(16, 0x1002, 0x164E, (0x1043, 0x8877), 0xC5),
        ];
        let free = [false; 3];
        let at = |address| Adapter { address: Some(address), ..geforce() };
        assert_eq!(at_address(&at((2, 0, 0)), &pci, &free), Some(1));
        assert_eq!(at_address(&at((2, 0, 0)), &pci, &[false, true, false]), None);
        assert_eq!(at_address(&at((2, 0, 1)), &pci, &free), None);
        // Another device sits there: the address is not believed and the ids decide.
        assert_eq!(at_address(&at((16, 0, 0)), &pci, &free), None);
        assert_eq!(place(&[&at((16, 0, 0)), &integrated()], &pci), [Some(0), Some(2)]);
        assert_eq!(at_address(&geforce(), &pci, &free), None);
    }

    #[test]
    fn leaves_out_cards_listed_twice() {
        let again = Adapter { luid: 9, has_output: false, drives_primary: false, ..geforce() };
        let listed = [geforce(), again, integrated()];
        let names = |gpus: &[GpuInfo]| gpus.iter().map(|gpu| gpu.name.clone()).collect::<Vec<_>>();
        let one_card = [pci_device(1, 0x10DE, 0x2882, (0x1462, 0x5161), 0xA1)];
        let one_gpu =
            [nvapi::Gpu { device_id: 0x2882, subsys: 0x5161_1462, revision: 0xA1, ..Default::default() }];

        // The PCI list knows a single such card.
        let gpus = assemble(&listed, &[], &one_card, &[]);
        assert_eq!(names(&gpus), ["Example GeForce", "Example Radeon Graphics"]);
        assert!(gpus[0].primary && !gpus[1].primary);
        assert_eq!(gpus[0].location, Some((1, 0, 0)));

        // Without a PCI list the NVIDIA driver's count decides.
        assert_eq!(assemble(&listed, &[], &[], &one_gpu).len(), 2);
        // The PCI list outranks it: two devices on the bus are two cards.
        let two_cards = [one_card[0].clone(), pci_device(2, 0x10DE, 0x2882, (0x1462, 0x5161), 0xA1)];
        assert_eq!(assemble(&listed, &[], &two_cards, &one_gpu).len(), 3);
        // With nothing to count against, every adapter is kept.
        assert_eq!(assemble(&listed, &[], &[], &[]).len(), 3);
    }

    #[test]
    fn matches_pci_devices() {
        let card = geforce();
        let exact = pci_device(1, 0x10DE, 0x2882, (0x1462, 0x5161), 0xA1);
        let other_board = pci_device(2, 0x10DE, 0x2882, (0x1043, 0x8888), 0xA1);
        let other_revision = pci_device(3, 0x10DE, 0x2882, (0x1462, 0x5161), 0xA2);
        let other_device = pci_device(4, 0x10DE, 0x2684, (0x1462, 0x5161), 0xA1);
        let bare = pci_device(5, 0x10DE, 0x2882, (0, 0), 0x00);

        let pci = [other_board.clone(), other_revision.clone(), other_device.clone(), bare.clone(), exact];
        assert_eq!(match_pci(&card, &pci, &[false; 5]), Some(4));
        // With the exact device taken, only the one with unknown details remains acceptable.
        assert_eq!(match_pci(&card, &pci, &[false, false, false, false, true]), Some(3));
        assert_eq!(match_pci(&card, &pci, &[false, false, false, true, true]), None);
        assert_eq!(match_pci(&card, &[other_board, other_revision, other_device], &[false; 3]), None);
        assert_eq!(match_pci(&card, &[], &[]), None);

        // A display-class device beats an unclassified one that is otherwise equal.
        let unclassified = PciDevice { base_class: 0, ..bare.clone() };
        assert_eq!(match_pci(&card, &[unclassified, bare.clone()], &[false; 2]), Some(1));
        // Equal candidates resolve to the first, and a short `taken` slice means "free".
        assert_eq!(match_pci(&card, &[bare.clone(), bare], &[]), Some(0));
    }

    #[test]
    fn non_pci_adapters_match_nothing() {
        let adapter = Adapter { name: "Virtual Display".into(), ..Default::default() };
        let blank = PciDevice::default();
        assert_eq!(match_pci(&adapter, &[blank], &[false]), None);
    }

    #[test]
    fn prefers_the_driver_memory_size() {
        let card = geforce();
        let all = drivers();
        assert_eq!(memory_mb(&card, Some(&all[1])), Some(8188));
        assert_eq!(memory_mb(&card, None), Some(8000));
        let silent = Driver { memory_bytes: None, ..all[1].clone() };
        assert_eq!(memory_mb(&card, Some(&silent)), Some(8000));
        let none = Adapter { dedicated_bytes: 0, ..geforce() };
        assert_eq!(memory_mb(&none, None), None);
    }

    #[test]
    fn a_replaced_cards_driver_entry_does_not_set_the_memory_size() {
        let entry = |memory_mb: u64| Driver { memory_bytes: Some(memory_mb * MIB), ..drivers()[1].clone() };
        // The first key is left over from the 4 GB model the 8 GB card replaced.
        let drivers = [entry(4096), entry(8192)];
        let card = Adapter { dedicated_bytes: 8171 * MIB, ..geforce() };
        assert_eq!(assemble(&[card], &drivers, &[], &[])[0].memory_mb, Some(8192));
    }

    #[test]
    fn renders_revisions() {
        assert_eq!(revision_text(0xA1), "A1");
        assert_eq!(revision_text(0xC5), "C5");
        assert_eq!(revision_text(0x04), "04");
    }
}
