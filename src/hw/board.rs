//! Mainboard, chipset, BIOS and graphic-interface details.

mod chipset;
mod dmi;

use windows::Win32::System::SystemInformation::{
    FirmwareTypeBios, FirmwareTypeUefi, GetFirmwareType, FIRMWARE_TYPE,
};

use crate::hw::pci;
use crate::hw::smbios::Smbios;
use crate::model::{BiosInfo, BridgeInfo, CpuInfo, GraphicInterface, MainboardInfo, PciDevice, PcieLink};

/// PCI base class and sub class of the devices the chipset rows are built from.
const HOST_BRIDGE: (u8, u8) = (0x06, 0x00);
const ISA_BRIDGE: (u8, u8) = (0x06, 0x01);
const PCI_BRIDGE: (u8, u8) = (0x06, 0x04);
const DISPLAY_CLASS: u8 = 0x03;

/// Describes the mainboard from the SMBIOS tables and the PCI device list.
/// `cpu` only refines names; nothing depends on it being filled in.
pub fn detect(smbios: Option<&Smbios>, pci: &[PciDevice], cpu: &CpuInfo) -> MainboardInfo {
    let mut board = describe(smbios, pci, cpu);
    board.bios.uefi = firmware_is_uefi();
    board
}

/// Everything in [`detect`] that is derived from its arguments alone.
fn describe(smbios: Option<&Smbios>, pci: &[PciDevice], cpu: &CpuInfo) -> MainboardInfo {
    let identity = smbios.map(dmi::identity).unwrap_or_default();
    // Portable machines carry product names that can look like desktop chipset names.
    let retail_model = (!smbios.is_some_and(dmi::is_portable)).then_some(identity.model.as_str());
    let host = pci.iter().find(|d| (d.bus, d.device, d.function) == (0, 0, 0) && has_class(d, HOST_BRIDGE));
    let isa = pci.iter().find(|d| d.bus == 0 && has_class(d, ISA_BRIDGE));

    MainboardInfo {
        bus_specs: bus_specs(pci, host),
        chipset: host.map(|bridge| northbridge(bridge, cpu)),
        southbridge: isa.map(|bridge| southbridge(bridge, host, retail_model)),
        lpcio: None,
        graphic_interface: graphic_interface(pci),
        bios: BiosInfo {
            brand: identity.bios_brand,
            version: identity.bios_version,
            date: identity.bios_date,
            uefi: None,
        },
        manufacturer: identity.manufacturer,
        model: identity.model,
        version: identity.version,
    }
}

fn has_class(device: &PciDevice, (base, sub): (u8, u8)) -> bool {
    device.is_class(base, sub)
}

/// The chipset row: the host bridge at 0:0.0.
fn northbridge(host: &PciDevice, cpu: &CpuInfo) -> BridgeInfo {
    bridge_info(host, chipset::northbridge_model(host.vendor_id, host.device_id, cpu))
}

/// The southbridge row: the LPC/eSPI bridge. Its model comes from the device id
/// when that names one chipset, then from the board model, then from the family.
fn southbridge(isa: &PciDevice, host: Option<&PciDevice>, retail_model: Option<&str>) -> BridgeInfo {
    let platform = host.and_then(|h| chipset::host_bridge(h.vendor_id, h.device_id));
    let model = chipset::lpc_bridge_model(isa.vendor_id, isa.device_id)
        .or_else(|| chipset::board_chipset(retail_model?, isa.vendor_id))
        .or_else(|| chipset::lpc_bridge_family(isa.vendor_id, isa.device_id, platform))
        .unwrap_or_default();
    bridge_info(isa, model.to_string())
}

fn bridge_info(device: &PciDevice, model: String) -> BridgeInfo {
    BridgeInfo {
        vendor: pci::vendor_name(device.vendor_id).unwrap_or_default().to_string(),
        model,
        revision: format!("{:02X}", device.revision),
    }
}

/// `PCI-Express 5.0 (32.0 GT/s)`: the fastest link the bridges on bus 0 support.
/// Windows rarely publishes link capabilities for bridges, so when none reports
/// one the speed documented for the platform's root ports stands in.
fn bus_specs(pci: &[PciDevice], host: Option<&PciDevice>) -> Option<String> {
    let reported = pci
        .iter()
        .filter(|d| d.bus == 0 && has_class(d, PCI_BRIDGE))
        .filter_map(|d| d.link?.max_speed_gts)
        .max_by(f32::total_cmp);
    let documented = || chipset::host_bridge(host?.vendor_id, host?.device_id)?.root_port_gts;
    let speed = reported.or_else(documented)?;
    Some(format!("PCI-Express {}.0 ({speed:.1} GT/s)", PcieLink::generation(speed)?))
}

/// The link of the graphics card: the display device on the lowest bus behind a
/// bridge. Integrated graphics sit on bus 0 or behind internal bridges on high buses.
fn graphic_interface(pci: &[PciDevice]) -> Option<GraphicInterface> {
    let linked = || pci.iter().filter(|d| d.base_class == DISPLAY_CLASS && d.link.is_some());
    let card = linked().filter(|d| d.bus > 0).min_by_key(|d| d.bus).or_else(|| linked().next())?;
    let link = card.link?;
    let bus = match link.max_speed_gts.or(link.speed_gts).and_then(PcieLink::generation) {
        Some(generation) => format!("PCI-Express {generation}.0"),
        None => "PCI-Express".to_string(),
    };
    Some(GraphicInterface { bus, link })
}

/// `Some(true)` on UEFI firmware, `Some(false)` on legacy BIOS.
fn firmware_is_uefi() -> Option<bool> {
    let mut kind = FIRMWARE_TYPE::default();
    // SAFETY: `kind` is a valid, writable FIRMWARE_TYPE for the duration of the call.
    unsafe { GetFirmwareType(&mut kind) }.ok()?;
    if kind == FirmwareTypeUefi {
        Some(true)
    } else if kind == FirmwareTypeBios {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::dmi::fixtures::{bios, chassis, smbios, structure};
    use super::*;
    use crate::hw::smbios::kind;
    use crate::model::CpuVendor;

    const AMD: u16 = 0x1022;
    const INTEL: u16 = 0x8086;
    const NVIDIA: u16 = 0x10DE;

    fn device(location: (u8, u8, u8), ids: (u16, u16), class: (u8, u8), revision: u8) -> PciDevice {
        PciDevice {
            bus: location.0,
            device: location.1,
            function: location.2,
            vendor_id: ids.0,
            device_id: ids.1,
            base_class: class.0,
            sub_class: class.1,
            revision,
            ..Default::default()
        }
    }

    fn link(width: u32, max_width: u32, speed: f32, max_speed: f32) -> Option<PcieLink> {
        Some(PcieLink {
            width: Some(width),
            max_width: Some(max_width),
            speed_gts: Some(speed),
            max_speed_gts: Some(max_speed),
        })
    }

    fn linked(mut device: PciDevice, link: Option<PcieLink>) -> PciDevice {
        device.link = link;
        device
    }

    /// An AM5 desktop the way Windows describes it: bridges carry no link properties.
    fn am5_devices() -> Vec<PciDevice> {
        vec![
            device((0, 0, 0), (AMD, 0x14D8), HOST_BRIDGE, 0x00),
            device((0, 1, 0), (AMD, 0x14DA), HOST_BRIDGE, 0x00),
            device((0, 1, 1), (AMD, 0x14DB), PCI_BRIDGE, 0x00),
            device((0, 8, 1), (AMD, 0x14DD), PCI_BRIDGE, 0x00),
            device((0, 0x14, 0), (AMD, 0x790B), (0x0C, 0x05), 0x71),
            device((0, 0x14, 3), (AMD, 0x790E), ISA_BRIDGE, 0x51),
            linked(device((1, 0, 0), (NVIDIA, 0x2882), (0x03, 0x00), 0xA1), link(8, 8, 16.0, 16.0)),
            linked(device((1, 0, 1), (NVIDIA, 0x22BE), (0x04, 0x03), 0xA1), link(8, 8, 16.0, 16.0)),
            linked(device((2, 0, 0), (0x1987, 0x5019), (0x01, 0x08), 0x01), link(4, 4, 16.0, 16.0)),
            linked(device((16, 0, 0), (0x1002, 0x164E), (0x03, 0x00), 0xC5), link(16, 16, 16.0, 16.0)),
        ]
    }

    fn am5_tables() -> Smbios {
        smbios(vec![
            bios("Example Firmware, LLC.", "2.A0", "09/21/2023"),
            structure(kind::SYSTEM, &["Example Computer Co., Ltd.", "Tower 5", "1.0"]),
            structure(kind::BASEBOARD, &["EXAMPLE", "PRO B650-EX WIFI (EX-1234)", "1.0"]),
            chassis(3),
        ])
    }

    fn bridge(vendor: &str, model: &str, revision: &str) -> Option<BridgeInfo> {
        Some(BridgeInfo { vendor: vendor.into(), model: model.into(), revision: revision.into() })
    }

    #[test]
    fn describes_an_am5_desktop() {
        let board = describe(Some(&am5_tables()), &am5_devices(), &CpuInfo::default());
        assert_eq!(board.manufacturer, "EXAMPLE");
        assert_eq!(board.model, "PRO B650-EX WIFI (EX-1234)");
        assert_eq!(board.version, "1.0");
        assert_eq!(board.chipset, bridge("AMD", "Ryzen SOC", "00"));
        assert_eq!(board.southbridge, bridge("AMD", "B650", "51"));
        assert_eq!(board.lpcio, None);
        assert_eq!(board.bus_specs.as_deref(), Some("PCI-Express 5.0 (32.0 GT/s)"));
        assert_eq!(
            board.graphic_interface,
            Some(GraphicInterface { bus: "PCI-Express 4.0".into(), link: link(8, 8, 16.0, 16.0).unwrap() })
        );
        assert_eq!(
            board.bios,
            BiosInfo {
                brand: "Example Firmware LLC.".into(),
                version: "2.A0".into(),
                date: "09/21/2023".into(),
                uefi: None,
            }
        );
    }

    #[test]
    fn nothing_known_gives_an_empty_board() {
        let board = describe(None, &[], &CpuInfo::default());
        assert_eq!(board.manufacturer, "");
        assert_eq!(board.chipset, None);
        assert_eq!(board.southbridge, None);
        assert_eq!(board.bus_specs, None);
        assert_eq!(board.graphic_interface, None);
        assert_eq!(board.bios, BiosInfo::default());
    }

    #[test]
    fn southbridge_prefers_the_device_id_over_the_board_model() {
        let tables = smbios(vec![structure(kind::BASEBOARD, &["EXAMPLE", "PRIME Z690-P", "Rev 1.xx"])]);
        let devices = [
            device((0, 0, 0), (INTEL, 0xA700), HOST_BRIDGE, 0x01),
            device((0, 0x1F, 0), (INTEL, 0x7A04), ISA_BRIDGE, 0x11),
        ];
        let board = describe(Some(&tables), &devices, &CpuInfo::default());
        assert_eq!(board.chipset, bridge("Intel", "Raptor Lake", "01"));
        assert_eq!(board.southbridge, bridge("Intel", "Z790", "11"));
        assert_eq!(board.bus_specs, None);
    }

    #[test]
    fn southbridge_falls_back_to_the_family() {
        let oem = smbios(vec![structure(kind::BASEBOARD, &["EXAMPLE", "0KWVT8", "A00"])]);
        let board = describe(Some(&oem), &am5_devices(), &CpuInfo::default());
        assert_eq!(board.southbridge, bridge("AMD", "FCH", "51"));

        let devices = [
            device((0, 0, 0), (INTEL, 0x9A14), HOST_BRIDGE, 0x01),
            device((0, 0x1F, 0), (INTEL, 0xA082), ISA_BRIDGE, 0x20),
        ];
        assert_eq!(describe(None, &devices, &CpuInfo::default()).southbridge, bridge("Intel", "PCH", "20"));

        let unknown = [device((0, 7, 0), (0x1106, 0x0001), ISA_BRIDGE, 0x02)];
        assert_eq!(describe(None, &unknown, &CpuInfo::default()).southbridge, bridge("VIA", "", "02"));
    }

    #[test]
    fn portable_product_names_are_not_read_as_chipsets() {
        let laptop = smbios(vec![structure(kind::BASEBOARD, &["EXAMPLE", "X570ZD", "1.0"]), chassis(10)]);
        let board = describe(Some(&laptop), &am5_devices(), &CpuInfo::default());
        assert_eq!(board.model, "X570ZD");
        assert_eq!(board.southbridge, bridge("AMD", "FCH", "51"));
    }

    #[test]
    fn chipset_takes_hints_from_the_processor() {
        let devices = [device((0, 0, 0), (INTEL, 0x0001), HOST_BRIDGE, 0x05)];
        let cpu =
            CpuInfo { vendor: CpuVendor::Intel, codename: Some("Arrow Lake".into()), ..Default::default() };
        assert_eq!(describe(None, &devices, &cpu).chipset, bridge("Intel", "Arrow Lake", "05"));
        assert_eq!(describe(None, &devices, &CpuInfo::default()).chipset, bridge("Intel", "", "05"));
    }

    #[test]
    fn host_bridge_must_sit_at_the_root() {
        let devices = [
            device((0, 1, 0), (AMD, 0x14DA), HOST_BRIDGE, 0x00),
            device((0, 0, 0), (AMD, 0x14D8), (0x08, 0x06), 0),
        ];
        assert_eq!(describe(None, &devices, &CpuInfo::default()).chipset, None);
    }

    #[test]
    fn bus_specs_prefer_reported_bridge_capabilities() {
        let mut devices = am5_devices();
        devices[2].link = link(8, 16, 16.0, 16.0);
        devices[3].link = link(16, 16, 8.0, 8.0);
        let host = devices.first();
        assert_eq!(bus_specs(&devices, host).as_deref(), Some("PCI-Express 4.0 (16.0 GT/s)"));

        // Bridges behind bus 0 and non-bridges do not count.
        let elsewhere = [
            linked(device((3, 0, 0), (AMD, 0x43F4), PCI_BRIDGE, 0x01), link(4, 4, 16.0, 64.0)),
            linked(device((0, 2, 0), (INTEL, 0x4680), (0x03, 0x00), 0x0C), link(1, 1, 2.5, 2.5)),
        ];
        assert_eq!(bus_specs(&elsewhere, None), None);
    }

    #[test]
    fn bus_specs_format_every_generation() {
        let port =
            |speed| [linked(device((0, 1, 0), (INTEL, 0x0001), PCI_BRIDGE, 0), link(1, 1, 2.5, speed))];
        assert_eq!(bus_specs(&port(2.5), None).as_deref(), Some("PCI-Express 1.0 (2.5 GT/s)"));
        assert_eq!(bus_specs(&port(5.0), None).as_deref(), Some("PCI-Express 2.0 (5.0 GT/s)"));
        assert_eq!(bus_specs(&port(8.0), None).as_deref(), Some("PCI-Express 3.0 (8.0 GT/s)"));
        assert_eq!(bus_specs(&port(64.0), None).as_deref(), Some("PCI-Express 6.0 (64.0 GT/s)"));
    }

    #[test]
    fn graphic_interface_prefers_the_card_on_the_lowest_bus() {
        let interface = graphic_interface(&am5_devices()).unwrap();
        assert_eq!(interface.bus, "PCI-Express 4.0");
        assert_eq!(interface.link.width, Some(8));

        let integrated_only: Vec<_> = am5_devices().into_iter().filter(|d| d.vendor_id != NVIDIA).collect();
        assert_eq!(graphic_interface(&integrated_only).unwrap().link.width, Some(16));

        let unlinked = [device((0, 2, 0), (INTEL, 0x4680), (0x03, 0x00), 0x0C)];
        assert_eq!(graphic_interface(&unlinked), None);
    }

    #[test]
    fn graphic_interface_bus_follows_the_card_capability() {
        let card = |link| [linked(device((1, 0, 0), (NVIDIA, 0x2882), (0x03, 0x00), 0xA1), link)];
        // A 4.0 card running at 3.0 speed in an older slot.
        assert_eq!(graphic_interface(&card(link(16, 16, 8.0, 16.0))).unwrap().bus, "PCI-Express 4.0");
        let current_only = Some(PcieLink { width: Some(16), speed_gts: Some(8.0), ..Default::default() });
        assert_eq!(graphic_interface(&card(current_only)).unwrap().bus, "PCI-Express 3.0");
        let width_only = Some(PcieLink { width: Some(16), ..Default::default() });
        assert_eq!(graphic_interface(&card(width_only)).unwrap().bus, "PCI-Express");
    }

    #[test]
    fn detect_adds_the_firmware_type_without_changing_the_rest() {
        let (tables, devices) = (am5_tables(), am5_devices());
        let detected = detect(Some(&tables), &devices, &CpuInfo::default());
        let described = describe(Some(&tables), &devices, &CpuInfo::default());
        assert_eq!(detected.model, described.model);
        assert_eq!(detected.southbridge, described.southbridge);
        assert_eq!(detected.bios.version, described.bios.version);
    }
}
