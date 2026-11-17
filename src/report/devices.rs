//! Device sections: PCI and USB devices, display adapters with their monitors, and
//! the software summary.

use crate::hw::pci::{class_name, vendor_name};
use crate::model::{GpuInfo, GpuPerfLevel, MonitorInfo, OsInfo, PciDevice, UsbDevice};

use super::doc::{Lines, Section};
use super::fmt;

/// "PCI Devices": what kind of device each one is, its location, IDs, class codes
/// and PCI Express link state.
pub(super) fn pci(devices: &[PciDevice]) -> Section {
    let mut out = Lines::default();
    for device in devices {
        out.pair(0, "Description", device_kind(device));
        let location = format!(
            "bus {0} (0x{0:02X}), device {1} (0x{1:02X}), function {2} (0x{2:02X})",
            device.bus, device.device, device.function
        );
        out.pair(0, "Location", location);
        out.heading(0, "Common header", 0);
        out.pair(1, "Vendor ID", vendor_id_text(device.vendor_id));
        out.pair(1, "Model ID", format!("0x{:04X}", device.device_id));
        out.pair(1, "Revision ID", format!("0x{:02X}", device.revision));
        out.pair(1, "PI", format!("0x{:02X}", device.prog_if));
        out.pair(1, "SubClass", format!("0x{:02X}", device.sub_class));
        out.pair(1, "BaseClass", format!("0x{:02X}", device.base_class));
        out.heading(0, "PCI header", 0);
        out.pair(1, "Subvendor ID", vendor_id_text(device.subsys_vendor_id));
        out.pair(1, "Subsystem ID", format!("0x{:04X}", device.subsys_id));
        if let Some(link) =
            device.link.as_ref().filter(|link| link.width.is_some() || link.speed_gts.is_some())
        {
            out.heading(0, "PCI capability", 0);
            out.pair(1, "Caps class", "PCI Express");
            out.opt(1, "Link width", fmt::device_link_width(link));
            out.opt(1, "Link speed", fmt::link_speed(link));
        }
        out.blank();
    }
    out.into_section("PCI Devices")
}

/// "USB Devices": one line per device with the class codes of its device descriptor
/// and its vendor and product id.
pub(super) fn usb(devices: &[UsbDevice]) -> Section {
    let mut out = Lines::default();
    for device in devices {
        out.pair(0, "USB Device", usb_text(device));
    }
    out.into_section("USB Devices")
}

/// "Graphics": the number of display adapters.
pub(super) fn graphics(gpus: &[GpuInfo]) -> Section {
    let mut out = Lines::default();
    out.opt(0, "Number of adapters", (!gpus.is_empty()).then_some(gpus.len()));
    out.into_section("Graphics")
}

/// "Graphic APIs": the interfaces the display adapters were read through.
pub(super) fn graphic_apis(apis: &[String]) -> Section {
    let mut out = Lines::default();
    for api in apis {
        out.pair(0, "API", api);
    }
    out.into_section("Graphic APIs")
}

/// "Display Adapters": one block per adapter with its chip, memory, link, clocks and
/// driver, followed by one block per monitor.
pub(super) fn display_adapters(gpus: &[GpuInfo], monitors: &[MonitorInfo]) -> Section {
    let mut out = Lines::default();
    for (n, gpu) in gpus.iter().enumerate() {
        let primary = if gpu.primary { " (primary)" } else { "" };
        out.heading(0, format!("Display adapter {n}{primary}"), 1);
        out.pair(1, "Name", &gpu.name);
        out.opt(1, "Board Manufacturer", gpu.board_manufacturer.as_deref());
        out.opt(1, "Revision", gpu.revision.as_deref());
        out.opt(1, "Codename", gpu.codename.as_deref());
        out.opt(1, "TDP", gpu.tdp_w.map(|watts| format!("{watts:.1} W")));
        out.opt(1, "Cores", gpu.cores);
        out.opt(1, "ROP Units", gpu.rops);
        out.opt(1, "TM Units", gpu.tmus);
        out.opt(1, "Technology", gpu.technology_nm.map(|nm| format!("{nm} nm")));
        out.opt(1, "Memory type", gpu.memory_type.as_deref());
        out.opt(1, "Memory vendor", gpu.memory_vendor.as_deref());
        out.opt(1, "Memory size", gpu.memory_mb.map(|mb| format!("{mb} MB")));
        out.opt(1, "Memory bus width", gpu.memory_bus_width.map(|bits| format!("{bits} bits")));
        let link = gpu.link.unwrap_or_default();
        out.opt(1, "Current Link Width", link.width.map(|width| format!("x{width}")));
        out.opt(1, "Current Link Speed", link.speed_gts.map(|speed| format!("{speed:.1} GT/s")));
        pci_identity(&mut out, gpu);
        for level in &gpu.perf_levels {
            perf_level(&mut out, level);
        }
        out.opt(1, "Driver version", gpu.driver_version.as_deref());
        out.opt(1, "Driver date", gpu.driver_date.as_deref());
        out.opt(1, "WDDM Model", gpu.wddm_model.as_deref());
        out.blank();
    }
    if !monitors.is_empty() {
        // The monitors are set apart from the adapters by a second blank line.
        out.blank();
    }
    for (n, monitor) in monitors.iter().enumerate() {
        out.heading(0, format!("Monitor {n}"), 1);
        out.opt(1, "Model", monitor.model.as_deref());
        out.opt(1, "ID", monitor.id.as_deref());
        out.opt(1, "Serial", monitor.serial.as_deref());
        out.opt(1, "Manufacturing Date", manufacturing_date(monitor));
        out.opt(1, "Size", monitor.size_inches.map(|inches| format!("{inches:.1} inches")));
        let mode = monitor.max_resolution;
        out.opt(
            1,
            "Max Resolution",
            mode.map(|mode| format!("{} x {} @ {} Hz", mode.width, mode.height, mode.refresh_hz)),
        );
        let range = |(low, high): (u32, u32), unit: &str| format!("{low}-{high} {unit}");
        out.opt(1, "Horizontal Freq. Range", monitor.horizontal_khz.map(|khz| range(khz, "kHz")));
        out.opt(1, "Vertical Freq. Range", monitor.vertical_hz.map(|hz| range(hz, "Hz")));
        out.opt(1, "Max Pixel Clock", monitor.max_pixel_clock_mhz.map(|mhz| format!("{mhz} MHz")));
        out.opt(1, "Gamma Factor", monitor.gamma.map(gamma_text));
        out.opt(1, "Video Input", monitor.video_input.as_deref());
        out.blank();
    }
    out.into_section("Display Adapters")
}

/// "Software": the Windows version, its installation date, the DirectX generation
/// and the number of hardware errors Windows has logged.
pub(super) fn software(os: &OsInfo) -> Section {
    let mut out = Lines::default();
    out.pair(0, "Windows Version", [os.name.as_str(), os.build.as_str()].join(" ").trim());
    out.opt(0, "Windows Installation Date", os.install_date.as_deref());
    out.opt(0, "DirectX Version", os.directx.as_deref());
    out.opt(0, "WHEA errors", os.whea_errors);
    out.into_section("Software")
}

/// The name of the device's class, or what Windows calls the device when its class code has none.
fn device_kind(device: &PciDevice) -> &str {
    class_name(device.base_class, device.sub_class, device.prog_if).unwrap_or(&device.description)
}

/// The adapter's PCI location and IDs, with the subsystem IDs in parentheses.
/// Left out when the adapter is not a PCI device (vendor ID 0).
fn pci_identity(out: &mut Lines, gpu: &GpuInfo) {
    if gpu.vendor_id == 0 {
        return;
    }
    match gpu.location {
        Some((bus, device, function)) => {
            let location = format!(
                "bus {bus} (0x{bus:X}), device {device} (0x{device:X}), function {function} (0x{function:X})"
            );
            out.pair(1, "PCI device", location);
        }
        None => out.heading(1, "PCI device", 0),
    }
    out.pair(2, "Vendor ID", format!("0x{:04X} (0x{:04X})", gpu.vendor_id, gpu.subsys_vendor_id));
    out.pair(2, "Model ID", format!("0x{:04X} (0x{:04X})", gpu.device_id, gpu.subsys_id));
    out.pair(2, "Revision ID", format!("0x{:02X}", gpu.revision_id));
}

/// A `Performance Level` line followed by the clocks the level specifies.
fn perf_level(out: &mut Lines, level: &GpuPerfLevel) {
    let clock = |mhz: f64| format!("{mhz:.1} MHz");
    out.pair(1, "Performance Level", &level.name);
    out.opt(2, "Core clock", level.core_mhz.map(clock));
    out.opt(2, "Shader clock", level.shader_mhz.map(clock));
    out.opt(2, "Memory clock", level.memory_mhz.map(clock));
}

/// `USB Composite Device, class=0xEF, subclass=0x02, vendor=0x1D6B, product=0x0104` -
/// the name of a USB device, the class codes when its hub gave them, and its ids.
fn usb_text(device: &UsbDevice) -> String {
    let mut fields = Vec::new();
    if !device.name.is_empty() {
        fields.push(device.name.clone());
    }
    if let Some((class, subclass)) = device.class {
        fields.push(format!("class=0x{class:02X}"));
        fields.push(format!("subclass=0x{subclass:02X}"));
    }
    fields.push(format!("vendor=0x{:04X}", device.vendor_id));
    fields.push(format!("product=0x{:04X}", device.product_id));
    fields.join(", ")
}

/// `Week 12, Year 2024`, or the year alone for a monitor that states no week.
fn manufacturing_date(monitor: &MonitorInfo) -> Option<String> {
    let year = monitor.year?;
    Some(match monitor.week {
        Some(week) => format!("Week {week}, Year {year}"),
        None => format!("Year {year}"),
    })
}

/// `2.2`, `2.45` - a gamma to the two decimals a monitor states it with, without
/// trailing zeros beyond the first.
fn gamma_text(gamma: f32) -> String {
    let text = format!("{gamma:.2}");
    text.strip_suffix('0').unwrap_or(&text).to_string()
}

/// `0x1022 (AMD)` - a vendor ID followed by the vendor's name when it is a known one.
fn vendor_id_text(id: u16) -> String {
    match vendor_name(id) {
        Some(name) => format!("0x{id:04X} ({name})"),
        None => format!("0x{id:04X}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use crate::model::PcieLink;
    use crate::report::txt::lines_of;

    #[test]
    fn names_known_vendors() {
        assert_eq!(vendor_id_text(0x1022), "0x1022 (AMD)");
        assert_eq!(vendor_id_text(0x8086), "0x8086 (Intel)");
        assert_eq!(vendor_id_text(0x10DE), "0x10DE (NVIDIA)");
        assert_eq!(vendor_id_text(0x0000), "0x0000");
        assert_eq!(vendor_id_text(0xFFFF), "0xFFFF");
    }

    #[test]
    fn describes_pci_devices() {
        assert_eq!(
            lines_of(&pci(&demo::system().pci)),
            [
                "Description\t\t\tHost Bridge",
                "Location\t\t\tbus 0 (0x00), device 0 (0x00), function 0 (0x00)",
                "Common header",
                "\tVendor ID\t\t0x1022 (AMD)",
                "\tModel ID\t\t0x14D8",
                "\tRevision ID\t\t0x00",
                "\tPI\t\t\t0x00",
                "\tSubClass\t\t0x00",
                "\tBaseClass\t\t0x06",
                "PCI header",
                "\tSubvendor ID\t\t0x1462 (MSI)",
                "\tSubsystem ID\t\t0x7D78",
                "",
                "Description\t\t\tVGA Controller",
                "Location\t\t\tbus 1 (0x01), device 0 (0x00), function 0 (0x00)",
                "Common header",
                "\tVendor ID\t\t0x10DE (NVIDIA)",
                "\tModel ID\t\t0x2882",
                "\tRevision ID\t\t0xA1",
                "\tPI\t\t\t0x00",
                "\tSubClass\t\t0x00",
                "\tBaseClass\t\t0x03",
                "PCI header",
                "\tSubvendor ID\t\t0x1462 (MSI)",
                "\tSubsystem ID\t\t0x5161",
                "PCI capability",
                "\tCaps class\t\tPCI Express",
                "\tLink width\t\t8x (max 8x)",
                "\tLink speed\t\t16.0 GT/s (max 16.0 GT/s)",
                "",
            ]
        );
    }

    #[test]
    fn describes_devices_by_their_class() {
        let device = |class: (u8, u8, u8), description: &str| PciDevice {
            base_class: class.0,
            sub_class: class.1,
            prog_if: class.2,
            description: description.into(),
            ..Default::default()
        };
        let described = |device: PciDevice| lines_of(&pci(&[device])).swap_remove(0);
        assert_eq!(
            described(device((0x0C, 0x03, 0x30), "Example USB 3.2 Host Controller")),
            "Description\t\t\tUSB Controller (xHCI)"
        );
        assert_eq!(
            described(device((0x01, 0x08, 0x02), "Standard NVM Express Controller")),
            "Description\t\t\tNVM Express (NVMe) Controller"
        );
        // A class code without an assignment leaves the name Windows gives the device,
        // and so does a device whose class could not be read at all.
        assert_eq!(
            described(device((0xFF, 0x00, 0x00), "Example Sensor Hub")),
            "Description\t\t\tExample Sensor Hub"
        );
        assert_eq!(
            described(device((0x00, 0x00, 0x00), "Example Legacy Card")),
            "Description\t\t\tExample Legacy Card"
        );
        // Neither a class name nor a description: the line is left out.
        assert_eq!(
            described(device((0xFF, 0x00, 0x00), "")),
            "Location\t\t\tbus 0 (0x00), device 0 (0x00), function 0 (0x00)"
        );
    }

    #[test]
    fn omits_the_capability_block_without_a_link_state() {
        let device = PciDevice {
            bus: 0x10,
            device: 0x1F,
            function: 7,
            link: Some(PcieLink { max_width: Some(4), ..Default::default() }),
            ..Default::default()
        };
        let lines = lines_of(&pci(&[device]));
        assert_eq!(lines[0], "Location\t\t\tbus 16 (0x10), device 31 (0x1F), function 7 (0x07)");
        assert!(!lines.iter().any(|line| line.contains("capability")));
    }

    #[test]
    fn counts_adapters() {
        assert_eq!(lines_of(&graphics(&demo::system().gpus)), ["Number of adapters\t\t2"]);
        assert!(lines_of(&graphics(&[])).is_empty());
    }

    #[test]
    fn describes_display_adapters() {
        let lines = lines_of(&display_adapters(&demo::system().gpus, &[]));
        let discrete = [
            "Display adapter 0 (primary)\t",
            "\tName\t\t\tNVIDIA GeForce RTX 4060",
            "\tBoard Manufacturer\tExample Computer Inc.",
            "\tRevision\t\tA1",
            "\tCodename\t\tAD107",
            "\tTDP\t\t\t115.0 W",
            "\tCores\t\t\t3072",
            "\tROP Units\t\t48",
            "\tTM Units\t\t96",
            "\tTechnology\t\t5 nm",
            "\tMemory type\t\tGDDR6",
            "\tMemory vendor\t\tSamsung",
            "\tMemory size\t\t8188 MB",
            "\tMemory bus width\t128 bits",
            "\tCurrent Link Width\tx8",
            "\tCurrent Link Speed\t16.0 GT/s",
            "\tPCI device\t\tbus 1 (0x1), device 0 (0x0), function 0 (0x0)",
            "\t\tVendor ID\t0x10DE (0x1462)",
            "\t\tModel ID\t0x2882 (0x5161)",
            "\t\tRevision ID\t0xA1",
            "\tPerformance Level\tCurrent",
            "\t\tCore clock\t285.0 MHz",
            "\t\tMemory clock\t5001.0 MHz",
            "\tDriver version\t\t32.0.15.6094",
            "\tDriver date\t\t08/14/2024",
            "\tWDDM Model\t\t3.2",
            "",
        ];
        assert_eq!(lines[..discrete.len()], discrete);
        let integrated = [
            "Display adapter 1\t",
            "\tName\t\t\tAMD Radeon(TM) Graphics",
            "\tBoard Manufacturer\tExample Computer Inc.",
            "\tRevision\t\tC5",
            "\tCodename\t\tRaphael",
            "\tCores\t\t\t128",
            "\tTechnology\t\t6 nm",
            "\tMemory size\t\t512 MB",
            "\tPCI device\t\tbus 16 (0x10), device 0 (0x0), function 0 (0x0)",
            "\t\tVendor ID\t0x1002 (0x1462)",
            "\t\tModel ID\t0x164E (0x7D78)",
            "\t\tRevision ID\t0xC5",
            "\tDriver version\t\t31.0.14046.0",
            "\tDriver date\t\t03/29/2023",
            "\tWDDM Model\t\t3.1",
            "",
        ];
        assert_eq!(lines[discrete.len()..], integrated);
    }

    #[test]
    fn handles_adapters_off_the_pci_bus() {
        let software = GpuInfo { name: "Basic Render Driver".into(), ..Default::default() };
        assert_eq!(
            lines_of(&display_adapters(&[software], &[])),
            ["Display adapter 0\t", "\tName\t\t\tBasic Render Driver", ""]
        );

        let unlocated =
            GpuInfo { vendor_id: 0x8086, device_id: 0x9A49, revision_id: 1, ..Default::default() };
        assert_eq!(
            lines_of(&display_adapters(&[unlocated], &[])),
            [
                "Display adapter 0\t",
                "\tPCI device",
                "\t\tVendor ID\t0x8086 (0x0000)",
                "\t\tModel ID\t0x9A49 (0x0000)",
                "\t\tRevision ID\t0x01",
                "",
            ]
        );
    }

    #[test]
    fn lists_monitors_after_the_adapters() {
        let sys = demo::system();
        let adapters = lines_of(&display_adapters(&sys.gpus, &[]));
        let lines = lines_of(&display_adapters(&sys.gpus, &sys.monitors));
        assert_eq!(lines[..adapters.len()], adapters);
        let monitors = [
            "",
            "Monitor 0\t",
            "\tModel\t\t\tEX2700Q",
            "\tID\t\t\tEXM2700",
            "\tSerial\t\t\tEXAMPLE000001",
            "\tManufacturing Date\tWeek 12, Year 2024",
            "\tSize\t\t\t27.2 inches",
            "\tMax Resolution\t\t3840 x 2160 @ 59 Hz",
            "\tHorizontal Freq. Range\t30-222 kHz",
            "\tVertical Freq. Range\t40-144 Hz",
            "\tMax Pixel Clock\t\t600 MHz",
            "\tGamma Factor\t\t2.2",
            "\tVideo Input\t\tDisplay Port",
            "",
            "Monitor 1\t",
            "\tModel\t\t\tEX2400",
            "\tID\t\t\tEXM2400",
            "\tManufacturing Date\tYear 2019",
            "\tSize\t\t\t23.8 inches",
            "\tMax Resolution\t\t1920 x 1080 @ 60 Hz",
            "\tGamma Factor\t\t2.2",
            "\tVideo Input\t\tHDMI",
            "",
        ];
        assert_eq!(lines[adapters.len()..], monitors);

        // Without adapters the monitors still stand apart from the section heading.
        let alone = lines_of(&display_adapters(&[], &sys.monitors[1..]));
        assert_eq!(alone[..3], ["", "Monitor 0\t", "\tModel\t\t\tEX2400"]);
        let blank = MonitorInfo::default();
        assert_eq!(lines_of(&display_adapters(&[], &[blank])), ["", "Monitor 0\t", ""]);
    }

    #[test]
    fn dates_and_gammas_keep_what_the_monitor_states() {
        let dated = |week, year| manufacturing_date(&MonitorInfo { week, year, ..Default::default() });
        assert_eq!(dated(Some(6), Some(2023)).as_deref(), Some("Week 6, Year 2023"));
        assert_eq!(dated(None, Some(2023)).as_deref(), Some("Year 2023"));
        assert_eq!(dated(Some(6), None), None);
        assert_eq!(gamma_text(2.2), "2.2");
        assert_eq!(gamma_text(2.45), "2.45");
        assert_eq!(gamma_text(1.0), "1.0");
        assert_eq!(gamma_text(3.54), "3.54");
    }

    #[test]
    fn lists_usb_devices() {
        assert_eq!(
            lines_of(&usb(&demo::system().usb)),
            [
                "USB Device\t\t\tGeneric USB Hub, class=0x09, subclass=0x00, vendor=0x1D6B, product=0x0002",
                "USB Device\t\t\tUSB Input Device, class=0x00, subclass=0x00, vendor=0x1D6B, product=0x0101",
                "USB Device\t\t\tUSB Composite Device, class=0xEF, subclass=0x02, vendor=0x1D6B, product=0x0104",
            ]
        );
        // The class codes are left out when the hub did not hand out the descriptor,
        // and so is a name Windows does not have.
        let silent = UsbDevice {
            name: "USB Mass Storage Device".into(),
            vendor_id: 0x1D6B,
            product_id: 0x0105,
            class: None,
        };
        assert_eq!(usb_text(&silent), "USB Mass Storage Device, vendor=0x1D6B, product=0x0105");
        let nameless = UsbDevice {
            class: Some((0xFF, 0x47)),
            vendor_id: 0x1D6B,
            product_id: 0x0106,
            ..Default::default()
        };
        assert_eq!(usb_text(&nameless), "class=0xFF, subclass=0x47, vendor=0x1D6B, product=0x0106");
        assert!(lines_of(&usb(&[])).is_empty());
    }

    #[test]
    fn lists_graphic_apis() {
        assert_eq!(
            lines_of(&graphic_apis(&demo::system().graphic_apis)),
            ["API\t\t\t\tD3D", "API\t\t\t\tNVIDIA NVAPI"]
        );
        assert!(lines_of(&graphic_apis(&[])).is_empty());
    }

    #[test]
    fn summarises_software() {
        assert_eq!(
            lines_of(&software(&demo::system().os)),
            [
                "Windows Version\t\t\tMicrosoft Windows 11 Home (x64) Build 26100.2314",
                "Windows Installation Date\t11/14/2023",
                "DirectX Version\t\t\t12.0",
                "WHEA errors\t\t\t0",
            ]
        );
        let faulty = OsInfo { whea_errors: Some(17), ..Default::default() };
        assert_eq!(lines_of(&software(&faulty)), ["WHEA errors\t\t\t17"]);
        let partial = OsInfo { build: "Build 19045".into(), ..Default::default() };
        assert_eq!(lines_of(&software(&partial)), ["Windows Version\t\t\tBuild 19045"]);
        assert!(lines_of(&software(&OsInfo::default())).is_empty());
    }
}
