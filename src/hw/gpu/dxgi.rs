//! Display adapter enumeration through DXGI.

use windows::core::Interface;
use windows::Win32::Foundation::LUID;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIDevice, IDXGIFactory1, DXGI_ADAPTER_DESC1,
    DXGI_ADAPTER_FLAG_SOFTWARE,
};

use super::kmt;

/// Enumeration stops here even if DXGI keeps handing out adapters or outputs.
const MAX_INDEX: u32 = 64;

/// PCI ids of the "Microsoft Basic Render Driver" software rasteriser.
const BASIC_RENDER_DRIVER: (u16, u16) = (0x1414, 0x008C);

/// The `LUID` an adapter id packs: high part above, low part below.
pub fn luid(packed: i64) -> LUID {
    LUID { LowPart: packed as u32, HighPart: (packed >> 32) as i32 }
}

/// One adapter as DXGI describes it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Adapter {
    pub name: String,
    pub vendor_id: u16,
    pub device_id: u16,
    pub subsys_vendor_id: u16,
    pub subsys_id: u16,
    pub revision_id: u8,
    /// Dedicated video memory in bytes.
    pub dedicated_bytes: u64,
    /// Locally unique id; two entries with the same id are the same adapter.
    pub luid: i64,
    pub software: bool,
    /// At least one of its outputs is part of the desktop.
    pub has_output: bool,
    /// One of its outputs is the primary display.
    pub drives_primary: bool,
    /// User-mode driver version, most significant field first.
    pub driver_version: Option<[u16; 4]>,
    /// The driver model in effect, as the graphics kernel numbers it: the WDDM version times 1000.
    pub wddm_version: Option<u32>,
    /// PCI bus, device and function, when the graphics kernel knows where the adapter sits.
    pub address: Option<(u8, u8, u8)>,
    /// Clocks, temperature and fan as the driver reports them to the graphics kernel.
    pub performance: kmt::Performance,
}

impl Adapter {
    /// Subsystem id and subsystem vendor id packed the way PCI hardware ids print them.
    pub fn subsys(&self) -> u32 {
        (u32::from(self.subsys_id) << 16) | u32::from(self.subsys_vendor_id)
    }

    /// Whether both adapters carry the same PCI ids, as identical cards do.
    pub fn same_hardware(&self, other: &Adapter) -> bool {
        (self.vendor_id, self.device_id, self.subsys(), self.revision_id)
            == (other.vendor_id, other.device_id, other.subsys(), other.revision_id)
    }

    fn is_software(&self) -> bool {
        self.software || (self.vendor_id, self.device_id) == BASIC_RENDER_DRIVER
    }
}

/// Lists the hardware adapters, the one driving the primary display first. `None` when
/// DXGI itself is not available.
pub fn enumerate() -> Option<Vec<Adapter>> {
    // SAFETY: creating the factory has no preconditions; DXGI does not need COM to be initialised.
    let factory = unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }.ok()?;
    let mut adapters = Vec::new();
    for index in 0..MAX_INDEX {
        // SAFETY: `factory` is a live interface; an index past the end is reported as an error.
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else {
            break;
        };
        adapters.extend(describe(&adapter));
    }
    Some(arrange(adapters))
}

fn describe(adapter: &IDXGIAdapter1) -> Option<Adapter> {
    // SAFETY: `adapter` is a live interface and the call only fills a plain structure.
    let desc = unsafe { adapter.GetDesc1() }.ok()?;
    let (has_output, drives_primary) = outputs(adapter);
    // SAFETY: the interface id outlives the call, which only reports a version number.
    let driver_version = unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID) }.ok().map(unpack_version);
    let kernel = kmt::query(desc.AdapterLuid);
    Some(Adapter {
        has_output,
        drives_primary,
        driver_version,
        wddm_version: kernel.wddm_version,
        address: kernel.address,
        performance: kmt::performance(desc.AdapterLuid),
        ..from_desc(&desc)
    })
}

/// Converts the adapter description; everything that takes a further query is left for the caller.
fn from_desc(desc: &DXGI_ADAPTER_DESC1) -> Adapter {
    // Adapters that are not PCI devices report ids wider than 16 bits. Nothing here can
    // interpret those, so all their id fields read as 0.
    let is_pci = desc.VendorId <= 0xFFFF && desc.DeviceId <= 0xFFFF;
    let word = |value: u32| if is_pci { (value & 0xFFFF) as u16 } else { 0 };
    Adapter {
        name: wide_text(&desc.Description),
        vendor_id: word(desc.VendorId),
        device_id: word(desc.DeviceId),
        subsys_vendor_id: word(desc.SubSysId),
        subsys_id: word(desc.SubSysId >> 16),
        revision_id: (word(desc.Revision) & 0xFF) as u8,
        dedicated_bytes: desc.DedicatedVideoMemory as u64,
        luid: (i64::from(desc.AdapterLuid.HighPart) << 32) | i64::from(desc.AdapterLuid.LowPart),
        software: desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0,
        ..Default::default()
    }
}

/// Whether the adapter has an output on the desktop, and whether one of them is the primary display.
fn outputs(adapter: &IDXGIAdapter1) -> (bool, bool) {
    let (mut attached, mut primary) = (false, false);
    for index in 0..MAX_INDEX {
        // SAFETY: `adapter` is a live interface; an index past the end is reported as an error.
        let Ok(output) = (unsafe { adapter.EnumOutputs(index) }) else {
            break;
        };
        // SAFETY: `output` is a live interface and the call only fills a plain structure.
        let Ok(desc) = (unsafe { output.GetDesc() }) else {
            continue;
        };
        if desc.AttachedToDesktop.as_bool() {
            attached = true;
            // The primary display is the one at the origin of the virtual desktop.
            primary |= desc.DesktopCoordinates.left == 0 && desc.DesktopCoordinates.top == 0;
        }
    }
    (attached, primary)
}

/// Text of a NUL-terminated UTF-16 buffer, trimmed.
fn wide_text(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end]).trim().to_string()
}

/// Splits the packed version DXGI reports into its four 16-bit fields.
fn unpack_version(packed: i64) -> [u16; 4] {
    [(packed >> 48) as u16, (packed >> 32) as u16, (packed >> 16) as u16, packed as u16]
}

/// Drops software rasterisers and repeated entries, then moves the primary adapter to the front.
fn arrange(adapters: Vec<Adapter>) -> Vec<Adapter> {
    let mut kept: Vec<Adapter> = Vec::new();
    for adapter in adapters {
        if adapter.is_software() {
            continue;
        }
        match kept.iter_mut().find(|other| other.luid == adapter.luid) {
            Some(other) => {
                other.has_output |= adapter.has_output;
                other.drives_primary |= adapter.drives_primary;
            }
            None => kept.push(adapter),
        }
    }
    if let Some(index) = primary_index(&kept) {
        kept[..=index].rotate_right(1);
    }
    kept
}

/// The adapter showing the primary display; failing that the first one with a display, or simply the first.
fn primary_index(adapters: &[Adapter]) -> Option<usize> {
    adapters
        .iter()
        .position(|adapter| adapter.drives_primary)
        .or_else(|| adapters.iter().position(|adapter| adapter.has_output))
        .or(if adapters.is_empty() { None } else { Some(0) })
}

#[cfg(test)]
mod tests {
    use windows::Win32::Foundation::LUID;

    use super::*;

    fn adapter(name: &str, luid: i64) -> Adapter {
        Adapter { name: name.into(), vendor_id: 0x10DE, device_id: 0x2882, luid, ..Default::default() }
    }

    fn names(adapters: &[Adapter]) -> Vec<&str> {
        adapters.iter().map(|adapter| adapter.name.as_str()).collect()
    }

    #[test]
    fn skips_software_adapters() {
        let flagged = Adapter { software: true, ..adapter("WARP", 1) };
        let basic =
            Adapter { vendor_id: 0x1414, device_id: 0x008C, ..adapter("Microsoft Basic Render Driver", 2) };
        let arranged = arrange(vec![flagged, adapter("real", 3), basic]);
        assert_eq!(names(&arranged), ["real"]);
    }

    #[test]
    fn merges_repeated_entries() {
        let first = adapter("card", 7);
        let again = Adapter { has_output: true, drives_primary: true, ..adapter("card", 7) };
        let arranged = arrange(vec![first, adapter("other", 8), again]);
        assert_eq!(names(&arranged), ["card", "other"]);
        assert!(arranged[0].has_output && arranged[0].drives_primary);
    }

    #[test]
    fn keeps_identical_cards_with_distinct_ids() {
        let arranged = arrange(vec![adapter("card", 1), adapter("card", 2)]);
        assert_eq!(arranged.len(), 2);
    }

    #[test]
    fn primary_adapter_comes_first() {
        let integrated = Adapter { has_output: true, ..adapter("integrated", 1) };
        let headless = adapter("headless", 2);
        let discrete = Adapter { has_output: true, drives_primary: true, ..adapter("discrete", 3) };
        let arranged = arrange(vec![integrated.clone(), headless.clone(), discrete]);
        assert_eq!(names(&arranged), ["discrete", "integrated", "headless"]);

        // Without a primary output the first adapter that shows anything wins.
        let arranged = arrange(vec![headless.clone(), integrated]);
        assert_eq!(names(&arranged), ["integrated", "headless"]);

        // Without any output the enumeration order stands.
        let arranged = arrange(vec![headless, adapter("second", 4)]);
        assert_eq!(names(&arranged), ["headless", "second"]);
        assert!(arrange(Vec::new()).is_empty());
    }

    fn description(text: &str) -> [u16; 128] {
        let mut buffer = [0u16; 128];
        for (slot, unit) in buffer.iter_mut().zip(text.encode_utf16()) {
            *slot = unit;
        }
        buffer
    }

    #[test]
    fn converts_descriptions() {
        let desc = DXGI_ADAPTER_DESC1 {
            Description: description(" Example Card \0junk"),
            VendorId: 0x10DE,
            DeviceId: 0x2882,
            SubSysId: 0x5161_1462,
            Revision: 0xA1,
            DedicatedVideoMemory: 0x1234_5678,
            AdapterLuid: LUID { LowPart: 0x8000_0001, HighPart: 2 },
            ..Default::default()
        };
        let card = from_desc(&desc);
        assert_eq!(card.name, "Example Card");
        assert_eq!((card.vendor_id, card.device_id), (0x10DE, 0x2882));
        assert_eq!((card.subsys_vendor_id, card.subsys_id, card.revision_id), (0x1462, 0x5161, 0xA1));
        assert_eq!(card.subsys(), 0x5161_1462);
        assert_eq!(card.dedicated_bytes, 0x1234_5678);
        assert_eq!(card.luid, 0x2_8000_0001);
        assert!(!card.software && !card.has_output && !card.drives_primary);
        assert_eq!((card.driver_version, card.wddm_version, card.address), (None, None, None));

        let software = from_desc(&DXGI_ADAPTER_DESC1 { Flags: 2, ..desc });
        assert!(software.software);
    }

    #[test]
    fn non_pci_adapters_have_no_ids() {
        // An ACPI-enumerated adapter reports a four-character vendor code instead of a PCI id.
        let desc = DXGI_ADAPTER_DESC1 {
            Description: description("System-on-chip graphics"),
            VendorId: 0x4D4F_4351,
            DeviceId: 0x4135_3430,
            SubSysId: 0x1462_10DE,
            Revision: 0x0107,
            ..Default::default()
        };
        let adapter = from_desc(&desc);
        assert_eq!(adapter.name, "System-on-chip graphics");
        assert_eq!(
            (adapter.vendor_id, adapter.device_id, adapter.subsys(), adapter.revision_id),
            (0, 0, 0, 0)
        );
    }

    #[test]
    fn decodes_text_and_versions() {
        assert_eq!(wide_text(&description(" Some Adapter \0junk")), "Some Adapter");
        assert_eq!(wide_text(&[0x41, 0x42]), "AB");
        assert_eq!(wide_text(&[]), "");
        assert_eq!(unpack_version(0x0020_0000_0010_0842), [32, 0, 16, 2114]);
    }

    #[test]
    fn compares_hardware_identity() {
        let card = adapter("card", 1);
        assert!(card.same_hardware(&adapter("same card, other id", 2)));
        assert!(!card.same_hardware(&Adapter { revision_id: 0xA1, ..adapter("card", 1) }));
        assert!(!card.same_hardware(&Adapter { subsys_id: 0x5161, ..adapter("card", 1) }));
        assert!(!card.same_hardware(&Adapter { device_id: 0x2684, ..adapter("card", 1) }));
    }
}
