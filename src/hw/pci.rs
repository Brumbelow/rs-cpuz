//! PCI device enumeration through the Windows device tree (no direct config-space access).

mod classes;
mod ids;
mod vendors;

use windows::core::GUID;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SPDRP_ADDRESS, SPDRP_BUSNUMBER, SPDRP_COMPATIBLEIDS, SPDRP_DEVICEDESC, SPDRP_FRIENDLYNAME,
    SPDRP_HARDWAREID,
};
use windows::Win32::Devices::Properties::DEVPROPKEY;

use super::setup::{Device, DeviceSet};
use crate::model::{PciDevice, PcieLink};
use ids::IdFields;

pub use classes::class_name;
pub use vendors::vendor_name;

/// Property set of the PCI bus driver (`DEVPKEY_PciDevice_*` in the Windows SDK). The
/// `windows` crate only exposes these keys behind its Wi-Fi feature, so they are spelled out here.
const PCI_DEVICE_PROPERTIES: GUID = GUID::from_u128(0x3ab22e31_8264_4b4e_9af5_a8d2d8e33e62);

const fn pci_property(pid: u32) -> DEVPROPKEY {
    DEVPROPKEY { fmtid: PCI_DEVICE_PROPERTIES, pid }
}

const PCI_BASE_CLASS: DEVPROPKEY = pci_property(3);
const PCI_SUB_CLASS: DEVPROPKEY = pci_property(4);
const PCI_PROG_IF: DEVPROPKEY = pci_property(5);
const PCI_CURRENT_LINK_SPEED: DEVPROPKEY = pci_property(9);
const PCI_CURRENT_LINK_WIDTH: DEVPROPKEY = pci_property(10);
const PCI_MAX_LINK_SPEED: DEVPROPKEY = pci_property(11);
const PCI_MAX_LINK_WIDTH: DEVPROPKEY = pci_property(12);

/// Lists the PCI devices Windows knows about, sorted by bus, device and function.
pub fn enumerate() -> Vec<PciDevice> {
    match DeviceSet::of_enumerator("PCI") {
        Some(set) => sorted(set.devices().filter_map(|device| interpret(read(&device))).collect()),
        None => Vec::new(),
    }
}

/// Orders devices by their full bus number, device and function.
fn sorted(mut found: Vec<(u32, PciDevice)>) -> Vec<PciDevice> {
    found.sort_by_key(|(bus, device)| (*bus, device.device, device.function));
    found.into_iter().map(|(_, device)| device).collect()
}

/// What the device tree says about one device, before interpretation.
#[derive(Debug, Clone, Default)]
struct RawDevice {
    hardware_ids: Vec<String>,
    compatible_ids: Vec<String>,
    /// Bus number; anything above the low byte (a PCI segment) only affects the ordering.
    bus: Option<u32>,
    /// `device << 16 | function`.
    address: Option<u32>,
    /// Base class, sub class and programming interface from the PCI bus driver.
    class: [Option<u32>; 3],
    description: String,
    link: RawLink,
}

/// PCI Express link properties as the PCI bus driver reports them: widths in
/// lanes, speeds as the generation code of the link registers.
#[derive(Debug, Clone, Copy, Default)]
struct RawLink {
    speed: Option<u32>,
    width: Option<u32>,
    max_speed: Option<u32>,
    max_width: Option<u32>,
}

/// Turns the raw properties into a model entry plus the full bus number used
/// for sorting. `None` when the device cannot be identified or located.
fn interpret(raw: RawDevice) -> Option<(u32, PciDevice)> {
    let ids = IdFields::from_ids(raw.hardware_ids.iter().chain(&raw.compatible_ids).map(String::as_str));
    let (bus, address) = (raw.bus?, raw.address?);
    let class_property = |index: usize| raw.class[index].and_then(|value| u8::try_from(value).ok());
    let device = PciDevice {
        bus: (bus & 0xFF) as u8,
        device: u8::try_from(address >> 16).ok()?,
        function: (address & 0xFF) as u8,
        vendor_id: ids.vendor_id?,
        device_id: ids.device_id?,
        subsys_vendor_id: ids.subsys_vendor_id.unwrap_or(0),
        subsys_id: ids.subsys_id.unwrap_or(0),
        revision: ids.revision.unwrap_or(0),
        base_class: ids.class.map(|c| c.base).or_else(|| class_property(0)).unwrap_or(0),
        sub_class: ids.class.map(|c| c.sub).or_else(|| class_property(1)).unwrap_or(0),
        prog_if: ids.class.and_then(|c| c.prog_if).or_else(|| class_property(2)).unwrap_or(0),
        description: raw.description,
        link: decode_link(&raw.link),
    };
    Some((bus, device))
}

/// `None` when the device reports no PCI Express link at all.
fn decode_link(raw: &RawLink) -> Option<PcieLink> {
    let link = PcieLink {
        width: raw.width.and_then(link_width),
        max_width: raw.max_width.and_then(link_width),
        speed_gts: raw.speed.and_then(link_speed_gts),
        max_speed_gts: raw.max_speed.and_then(link_speed_gts),
    };
    (link != PcieLink::default()).then_some(link)
}

/// Link speed in GT/s for the generation code used by the PCI Express link registers.
fn link_speed_gts(code: u32) -> Option<f32> {
    match code {
        1 => Some(2.5),
        2 => Some(5.0),
        3 => Some(8.0),
        4 => Some(16.0),
        5 => Some(32.0),
        6 => Some(64.0),
        _ => None,
    }
}

/// Lane count, when it is one a PCI Express link can have.
fn link_width(lanes: u32) -> Option<u32> {
    matches!(lanes, 1 | 2 | 4 | 8 | 12 | 16 | 32).then_some(lanes)
}

/// Reads what the device tree has on one device.
fn read(device: &Device<'_>) -> RawDevice {
    RawDevice {
        hardware_ids: device.strings(SPDRP_HARDWAREID),
        compatible_ids: device.strings(SPDRP_COMPATIBLEIDS),
        bus: device.dword(SPDRP_BUSNUMBER),
        address: device.dword(SPDRP_ADDRESS),
        class: [PCI_BASE_CLASS, PCI_SUB_CLASS, PCI_PROG_IF].map(|key| device.property(&key)),
        description: [SPDRP_FRIENDLYNAME, SPDRP_DEVICEDESC]
            .into_iter()
            .find_map(|property| device.strings(property).into_iter().next())
            .unwrap_or_default(),
        link: RawLink {
            speed: device.property(&PCI_CURRENT_LINK_SPEED),
            width: device.property(&PCI_CURRENT_LINK_WIDTH),
            max_speed: device.property(&PCI_MAX_LINK_SPEED),
            max_width: device.property(&PCI_MAX_LINK_WIDTH),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graphics_card() -> RawDevice {
        RawDevice {
            hardware_ids: vec![
                "PCI\\VEN_10DE&DEV_2882&SUBSYS_51611462&REV_A1".into(),
                "PCI\\VEN_10DE&DEV_2882&CC_030000".into(),
            ],
            compatible_ids: vec!["PCI\\VEN_10DE&CC_030000".into(), "PCI\\CC_0300".into()],
            bus: Some(1),
            address: Some(0),
            class: [Some(3), Some(0), Some(0)],
            description: "Example Graphics Adapter".into(),
            link: RawLink { speed: Some(4), width: Some(8), max_speed: Some(4), max_width: Some(8) },
        }
    }

    #[test]
    fn interprets_a_device() {
        let (bus, device) = interpret(graphics_card()).unwrap();
        assert_eq!(bus, 1);
        assert_eq!(
            device,
            PciDevice {
                bus: 1,
                device: 0,
                function: 0,
                vendor_id: 0x10DE,
                device_id: 0x2882,
                subsys_vendor_id: 0x1462,
                subsys_id: 0x5161,
                revision: 0xA1,
                base_class: 0x03,
                sub_class: 0x00,
                prog_if: 0x00,
                description: "Example Graphics Adapter".into(),
                link: Some(PcieLink {
                    width: Some(8),
                    max_width: Some(8),
                    speed_gts: Some(16.0),
                    max_speed_gts: Some(16.0),
                }),
            }
        );
    }

    #[test]
    fn splits_address_into_device_and_function() {
        let raw = RawDevice { bus: Some(0), address: Some(0x0014_0003), ..graphics_card() };
        let (_, device) = interpret(raw).unwrap();
        assert_eq!((device.bus, device.device, device.function), (0, 0x14, 3));
    }

    #[test]
    fn keeps_the_segment_for_sorting_only() {
        let raw = RawDevice { bus: Some(0x0001_00E1), ..graphics_card() };
        let (bus, device) = interpret(raw).unwrap();
        assert_eq!(bus, 0x0001_00E1);
        assert_eq!(device.bus, 0xE1);
    }

    #[test]
    fn class_falls_back_to_the_bus_driver_properties() {
        let raw = RawDevice {
            hardware_ids: vec!["PCI\\VEN_1022&DEV_790E&SUBSYS_7D781462&REV_51".into()],
            compatible_ids: Vec::new(),
            class: [Some(6), Some(1), Some(0)],
            link: RawLink::default(),
            ..graphics_card()
        };
        let (_, device) = interpret(raw).unwrap();
        assert!(device.is_class(0x06, 0x01));
        assert_eq!(device.revision, 0x51);
        assert_eq!(device.link, None);
    }

    #[test]
    fn class_ids_win_over_out_of_range_properties() {
        let raw = RawDevice { class: [Some(0x1FF), None, None], ..graphics_card() };
        let (_, device) = interpret(raw).unwrap();
        assert_eq!((device.base_class, device.sub_class, device.prog_if), (0x03, 0x00, 0x00));
    }

    #[test]
    fn skips_devices_without_identity_or_location() {
        assert!(interpret(RawDevice { bus: None, ..graphics_card() }).is_none());
        assert!(interpret(RawDevice { address: None, ..graphics_card() }).is_none());
        assert!(interpret(RawDevice { address: Some(0x0100_0000), ..graphics_card() }).is_none());
        let anonymous = RawDevice { hardware_ids: Vec::new(), compatible_ids: Vec::new(), ..graphics_card() };
        assert!(interpret(anonymous).is_none());
    }

    #[test]
    fn sorts_by_bus_device_and_function() {
        let at = |bus: u32, address: u32| {
            interpret(RawDevice { bus: Some(bus), address: Some(address), ..graphics_card() }).unwrap()
        };
        let found =
            vec![at(0x0001_0000, 0), at(16, 0), at(0, 0x0014_0003), at(1, 1), at(0, 0x0001_0001), at(1, 0)];
        let order: Vec<_> = sorted(found).iter().map(|d| (d.bus, d.device, d.function)).collect();
        assert_eq!(order, [(0, 1, 1), (0, 0x14, 3), (1, 0, 0), (1, 0, 1), (16, 0, 0), (0, 0, 0)]);
    }

    #[test]
    fn decodes_link_speeds_and_widths() {
        assert_eq!(link_speed_gts(1), Some(2.5));
        assert_eq!(link_speed_gts(3), Some(8.0));
        assert_eq!(link_speed_gts(5), Some(32.0));
        assert_eq!(link_speed_gts(6), Some(64.0));
        assert_eq!(link_speed_gts(0), None);
        assert_eq!(link_speed_gts(7), None);
        assert_eq!(link_width(16), Some(16));
        assert_eq!(link_width(0), None);
        assert_eq!(link_width(3), None);
    }

    #[test]
    fn partial_link_is_kept_and_empty_link_dropped() {
        let partial = RawLink { speed: Some(2), width: None, max_speed: Some(9), max_width: Some(4) };
        assert_eq!(
            decode_link(&partial),
            Some(PcieLink { width: None, max_width: Some(4), speed_gts: Some(5.0), max_speed_gts: None })
        );
        assert_eq!(decode_link(&RawLink::default()), None);
        assert_eq!(decode_link(&RawLink { speed: Some(0), width: Some(0), ..Default::default() }), None);
    }
}
