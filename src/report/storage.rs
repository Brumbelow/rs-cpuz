//! The storage section: drives, their controllers and the volumes on them.

use crate::hw::storage::bus_name;
use crate::model::{DriveController, DriveInfo, VolumeInfo};

use super::doc::{Lines, Section};

/// Bytes in the binary gigabyte the sizes are stated in.
const GIB: f64 = (1u64 << 30) as f64;

/// "Storage": one block per drive with its identity, capacity, kind, bus, controller
/// and volumes.
pub(super) fn section(drives: &[DriveInfo]) -> Section {
    let mut out = Lines::default();
    for drive in drives {
        out.short("Drive", 1, drive.number.to_string());
        out.pair(1, "Device Path", &drive.device_path);
        out.pair(1, "Name", &drive.name);
        out.opt(1, "Revision", drive.revision.as_deref());
        out.opt(1, "Serial", drive.serial.as_deref());
        out.opt(1, "Capacity", drive.capacity_bytes.map(|bytes| format!("{:.1} GB", bytes as f64 / GIB)));
        out.pair(1, "Type", kind(drive));
        out.opt(1, "Bus Type", drive.bus_type.map(bus_text));
        out.pair(1, "Features", drive.features.join(", "));
        if let Some(controller) = &drive.controller {
            out.pair(1, "Controller", controller_text(controller));
            out.opt(1, "Link Speed", link_text(controller));
        }
        for volume in &drive.volumes {
            out.pair(1, "Volume", volume_text(volume));
        }
        out.blank();
    }
    out.into_section("Storage")
}

/// `Fixed, SSD`, `Fixed`, `Removable` - whether the medium comes out, and whether the
/// drive is known to be solid state.
fn kind(drive: &DriveInfo) -> String {
    let medium = if drive.removable { "Removable" } else { "Fixed" };
    if drive.solid_state {
        format!("{medium}, SSD")
    } else {
        medium.to_string()
    }
}

/// `NVMe (17)` - the name of a bus followed by the number Windows gives it.
fn bus_text(code: u32) -> String {
    format!("{} ({code})", bus_name(code).unwrap_or("Unknown"))
}

/// `NVM Express (NVMe) Controller at bus 2, device 0, function 0`.
fn controller_text(controller: &DriveController) -> String {
    format!(
        "{} at bus {}, device {}, function {}",
        controller.name, controller.bus, controller.device, controller.function
    )
}

/// `PCI-E 4x @ 16.0 GT/s` - width and speed of the link, when both are known.
fn link_text(controller: &DriveController) -> Option<String> {
    let link = controller.link?;
    Some(format!("PCI-E {}x @ {:.1} GT/s", link.width?, link.speed_gts?))
}

/// `c:\, 930.5 GBytes (41.5 percent available)`.
fn volume_text(volume: &VolumeInfo) -> String {
    let size = volume.size_bytes as f64;
    let available = if volume.size_bytes == 0 { 0.0 } else { volume.free_bytes as f64 * 100.0 / size };
    format!("{}, {:.1} GBytes ({available:.1} percent available)", volume.mount, size / GIB)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use crate::model::PcieLink;
    use crate::report::txt::lines_of;

    #[test]
    fn describes_drives() {
        let lines = lines_of(&section(&demo::system().drives));
        let solid_state = [
            "Drive\t0",
            "\tDevice Path\t\t\\\\?\\scsi#disk&ven_nvme&prod_example_ssd_1tb#4&1a2b3c4d&0&000000#{53f56307-b6bf-11d0-94f2-00a0c91efb8b}",
            "\tName\t\t\tExample SSD 1TB",
            "\tRevision\t\tEX1.0",
            "\tSerial\t\t\tEX0000000001",
            "\tCapacity\t\t931.5 GB",
            "\tType\t\t\tFixed, SSD",
            "\tBus Type\t\tNVMe (17)",
            "\tFeatures\t\tSMART",
            "\tController\t\tNVM Express (NVMe) Controller at bus 2, device 0, function 0",
            "\tLink Speed\t\tPCI-E 4x @ 16.0 GT/s",
            "\tVolume\t\t\tc:\\, 930.5 GBytes (41.5 percent available)",
            "",
        ];
        assert_eq!(lines[..solid_state.len()], solid_state);
        let hard_disk = [
            "Drive\t1",
            "\tDevice Path\t\t\\\\?\\scsi#disk&ven_example&prod_hdd_4tb#4&5e6f7a8b&0&010000#{53f56307-b6bf-11d0-94f2-00a0c91efb8b}",
            "\tName\t\t\tExample HDD 4TB",
            "\tRevision\t\tEX2.0",
            "\tSerial\t\t\tEX0000000002",
            "\tCapacity\t\t3726.0 GB",
            "\tType\t\t\tFixed",
            "\tBus Type\t\tSATA (11)",
            "\tFeatures\t\tSMART",
            "\tController\t\tSerial ATA Controller - AHCI at bus 3, device 0, function 1",
            "\tVolume\t\t\td:\\, 1863.0 GBytes (75.0 percent available)",
            "\tVolume\t\t\te:\\, 1863.0 GBytes (100.0 percent available)",
            "",
        ];
        assert_eq!(lines[solid_state.len()..], hard_disk);
    }

    #[test]
    fn leaves_out_what_a_drive_does_not_tell() {
        let reader = DriveInfo {
            number: 3,
            device_path:
                r"\\?\usbstor#disk&ven_example&prod_reader#0001#{53f56307-b6bf-11d0-94f2-00a0c91efb8b}".into(),
            name: "Example Reader".into(),
            removable: true,
            bus_type: Some(7),
            ..Default::default()
        };
        assert_eq!(
            lines_of(&section(&[reader])),
            [
                "Drive\t3",
                "\tDevice Path\t\t\\\\?\\usbstor#disk&ven_example&prod_reader#0001#{53f56307-b6bf-11d0-94f2-00a0c91efb8b}",
                "\tName\t\t\tExample Reader",
                "\tType\t\t\tRemovable",
                "\tBus Type\t\tUSB (7)",
                "",
            ]
        );
        assert!(lines_of(&section(&[])).is_empty());
    }

    #[test]
    fn names_the_kind_and_bus() {
        let drive = |removable, solid_state| DriveInfo { removable, solid_state, ..Default::default() };
        assert_eq!(kind(&drive(false, true)), "Fixed, SSD");
        assert_eq!(kind(&drive(false, false)), "Fixed");
        assert_eq!(kind(&drive(true, true)), "Removable, SSD");
        assert_eq!(kind(&drive(true, false)), "Removable");
        assert_eq!(bus_text(17), "NVMe (17)");
        assert_eq!(bus_text(11), "SATA (11)");
        assert_eq!(bus_text(42), "Unknown (42)");
    }

    #[test]
    fn states_the_link_only_when_it_is_known() {
        let controller = |link| DriveController {
            name: "NVM Express (NVMe) Controller".into(),
            bus: 0x41,
            device: 0,
            function: 0,
            link,
        };
        let link = PcieLink { width: Some(2), speed_gts: Some(8.0), ..Default::default() };
        assert_eq!(
            controller_text(&controller(None)),
            "NVM Express (NVMe) Controller at bus 65, device 0, function 0"
        );
        assert_eq!(link_text(&controller(Some(link))).as_deref(), Some("PCI-E 2x @ 8.0 GT/s"));
        assert_eq!(link_text(&controller(Some(PcieLink { width: None, ..link }))), None);
        assert_eq!(link_text(&controller(Some(PcieLink { speed_gts: None, ..link }))), None);
        assert_eq!(link_text(&controller(None)), None);
    }

    #[test]
    fn sizes_volumes() {
        let volume = |size_bytes, free_bytes| VolumeInfo { mount: "f:\\".into(), size_bytes, free_bytes };
        assert_eq!(volume_text(&volume(64 << 30, 16 << 30)), "f:\\, 64.0 GBytes (25.0 percent available)");
        assert_eq!(volume_text(&volume(512 << 20, 0)), "f:\\, 0.5 GBytes (0.0 percent available)");
        assert_eq!(volume_text(&volume(0, 0)), "f:\\, 0.0 GBytes (0.0 percent available)");
    }
}
