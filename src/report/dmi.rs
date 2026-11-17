//! The DMI section: SMBIOS structures decoded field by field.
//!
//! Offsets and enumeration values follow the DMTF SMBIOS reference specification (DSP0134).

use crate::hw::smbios::memory_device::{
    configured_speed, field, form_factor_name, memory_type_name, rated_speed, DeviceSize,
};
use crate::hw::smbios::{kind, Smbios, SmbiosStructure};

use super::doc::{Lines, Section};

type Decoder = fn(&SmbiosStructure, &mut Lines);

/// The structure types this section decodes: SMBIOS type, block heading and decoder.
const BLOCKS: &[(u8, &str, Decoder)] = &[
    (kind::BIOS, "DMI BIOS", bios),
    (kind::SYSTEM, "DMI System Information", system),
    (kind::BASEBOARD, "DMI Baseboard", baseboard),
    (kind::CHASSIS, "DMI System Enclosure", enclosure),
    (kind::PROCESSOR, "DMI Processor", processor),
    (kind::PORT_CONNECTOR, "DMI Port Connector", port_connector),
    (kind::SYSTEM_SLOT, "DMI Extension Slot", slot),
    (kind::OEM_STRINGS, "DMI OEM Strings", oem_strings),
    (kind::MEMORY_ARRAY, "DMI Physical Memory Array", memory_array),
    (kind::MEMORY_DEVICE, "DMI Memory Device", memory_device),
    (kind::ADDITIONAL_INFORMATION, "DMI Additional Information Device", additional_information),
];

/// System enclosure types, indexed from 1.
const CHASSIS_TYPES: &[&str] = &[
    "Other",
    "Unknown",
    "Desktop",
    "Low Profile Desktop",
    "Pizza Box",
    "Mini Tower",
    "Tower",
    "Portable",
    "Laptop",
    "Notebook",
    "Hand Held",
    "Docking Station",
    "All in One",
    "Sub Notebook",
    "Space-saving",
    "Lunch Box",
    "Main Server Chassis",
    "Expansion Chassis",
    "SubChassis",
    "Bus Expansion Chassis",
    "Peripheral Chassis",
    "RAID Chassis",
    "Rack Mount Chassis",
    "Sealed-case PC",
    "Multi-system Chassis",
    "Compact PCI",
    "Advanced TCA",
    "Blade",
    "Blade Enclosure",
    "Tablet",
    "Convertible",
    "Detachable",
    "IoT Gateway",
    "Embedded PC",
    "Mini PC",
    "Stick PC",
];

/// Physical memory array locations, indexed from 1.
const ARRAY_LOCATIONS: &[&str] = &[
    "Other",
    "Unknown",
    "Motherboard",
    "ISA add-on card",
    "EISA add-on card",
    "PCI add-on card",
    "MCA add-on card",
    "PCMCIA add-on card",
    "Proprietary add-on card",
    "NuBus",
];

/// Physical memory array uses, indexed from 1.
const ARRAY_USES: &[&str] = &[
    "Other",
    "Unknown",
    "System Memory",
    "Video Memory",
    "Flash Memory",
    "Non-volatile RAM",
    "Cache Memory",
];

/// Memory error correction types, indexed from 1.
const ERROR_CORRECTIONS: &[&str] =
    &["Other", "Unknown", "None", "Parity", "Single-bit ECC", "Multi-bit ECC", "CRC"];

/// Port connector types, indexed from 1. The PC-98 connectors and "Other" lie apart from the run.
const CONNECTOR_TYPES: &[&str] = &[
    "Centronics",
    "Mini Centronics",
    "Proprietary",
    "DB-25 pin male",
    "DB-25 pin female",
    "DB-15 pin male",
    "DB-15 pin female",
    "DB-9 pin male",
    "DB-9 pin female",
    "RJ-11",
    "RJ-45",
    "50-pin MiniSCSI",
    "Mini-DIN",
    "Micro-DIN",
    "PS/2",
    "Infrared",
    "HP-HIL",
    "Access Bus (USB)",
    "SSA SCSI",
    "Circular DIN-8 male",
    "Circular DIN-8 female",
    "On Board IDE",
    "On Board Floppy",
    "9-pin Dual Inline (pin 10 cut)",
    "25-pin Dual Inline (pin 26 cut)",
    "50-pin Dual Inline",
    "68-pin Dual Inline",
    "On Board Sound Input from CD-ROM",
    "Mini-Centronics Type-14",
    "Mini-Centronics Type-26",
    "Mini-jack (headphones)",
    "BNC",
    "1394",
    "SAS/SATA Plug Receptacle",
    "USB Type-C Receptacle",
];

/// Port types, indexed from 1. The 8251 serial ports and "Other" lie apart from the run.
const PORT_TYPES: &[&str] = &[
    "Parallel Port XT/AT Compatible",
    "Parallel Port PS/2",
    "Parallel Port ECP",
    "Parallel Port EPP",
    "Parallel Port ECP/EPP",
    "Serial Port XT/AT Compatible",
    "Serial Port 16450 Compatible",
    "Serial Port 16550 Compatible",
    "Serial Port 16550A Compatible",
    "SCSI Port",
    "MIDI Port",
    "Joy Stick Port",
    "Keyboard Port",
    "Mouse Port",
    "SSA SCSI",
    "USB",
    "FireWire (IEEE P1394)",
    "PCMCIA Type I",
    "PCMCIA Type II",
    "PCMCIA Type III",
    "Cardbus",
    "Access Bus Port",
    "SCSI II",
    "SCSI Wide",
    "PC-98",
    "PC-98-Hireso",
    "PC-H98",
    "Video Port",
    "Audio Port",
    "Modem Port",
    "Network Port",
    "SATA",
    "SAS",
    "MFDP (Multi-Function Display Port)",
    "Thunderbolt",
];

/// Layout of one entry of an additional information structure: its length byte
/// comes first, the string index sits at `ENTRY_STRING` and at least one value byte ends it.
const ENTRY_STRING: usize = 4;
const ENTRY_MIN_LENGTH: usize = 6;

/// "Current usage" values of a system slot.
const SLOT_AVAILABLE: u8 = 3;
const SLOT_IN_USE: u8 = 4;

/// "DMI": the SMBIOS version, then one block per decoded structure in firmware order.
pub(super) fn section(smbios: Option<&Smbios>) -> Section {
    let mut out = Lines::default();
    if let Some(smbios) = smbios {
        out.pair(0, "SMBIOS Version", format!("{}.{}", smbios.major, smbios.minor));
        out.blank();
        for structure in &smbios.structures {
            let Some((_, title, decode)) = BLOCKS.iter().find(|(kind, ..)| *kind == structure.kind) else {
                continue;
            };
            out.heading(0, *title, 2);
            decode(structure, &mut out);
            out.blank();
        }
    }
    out.into_section("DMI")
}

fn bios(s: &SmbiosStructure, out: &mut Lines) {
    out.opt(1, "vendor", s.string(0x04));
    out.opt(1, "version", s.string(0x05));
    out.opt(1, "date", s.string(0x08));
    out.opt(1, "ROM size", rom_size_kb(s).map(|kb| format!("{kb} KB")));
}

fn system(s: &SmbiosStructure, out: &mut Lines) {
    out.opt(1, "manufacturer", s.string(0x04));
    out.opt(1, "product", s.string(0x05));
    out.opt(1, "version", s.string(0x06));
    out.opt(1, "serial", s.string(0x07));
    out.opt(1, "UUID", uuid_text(s));
    out.opt(1, "SKU", s.string(0x19));
    out.opt(1, "family", s.string(0x1A));
}

fn baseboard(s: &SmbiosStructure, out: &mut Lines) {
    out.opt(1, "vendor", s.string(0x04));
    out.opt(1, "model", s.string(0x05));
    out.opt(1, "revision", s.string(0x06));
    out.opt(1, "serial", s.string(0x07));
}

fn enclosure(s: &SmbiosStructure, out: &mut Lines) {
    out.opt(1, "manufacturer", s.string(0x04));
    // Bit 7 of the type byte flags a chassis lock.
    out.opt(1, "chassis type", s.byte(0x05).map(|code| enum_name(CHASSIS_TYPES, code & 0x7F)));
    out.opt(1, "chassis serial", s.string(0x07));
}

fn processor(s: &SmbiosStructure, out: &mut Lines) {
    let mhz = |offset| s.word(offset).filter(|&mhz| mhz != 0).map(f64::from);
    let speed = |mhz: f64| format!("{mhz:.1} MHz");
    let (external, current) = (mhz(0x12), mhz(0x16));
    out.opt(1, "manufacturer", s.string(0x07));
    out.opt(1, "model", s.string(0x10));
    out.opt(1, "clock speed", current.map(speed));
    out.opt(1, "FSB speed", external.map(speed));
    out.opt(
        1,
        "multiplier",
        current.zip(external).map(|(current, external)| format!("{:.1}x", current / external)),
    );
    out.opt(1, "max clock speed", mhz(0x14).map(speed));
}

/// Both reference designators, the kind of port and the connector a cable goes
/// into (the header on the board when the port has no connector of its own).
fn port_connector(s: &SmbiosStructure, out: &mut Lines) {
    let designation = |offset, side: &str| s.string(offset).map(|name| format!("{name} ({side})"));
    out.opt(1, "designation", designation(0x04, "internal"));
    out.opt(1, "designation", designation(0x06, "external"));
    out.opt(1, "port type", s.byte(0x08).map(port_type));
    out.opt(1, "connector", [0x07, 0x05].into_iter().find_map(|offset| connector_type(s.byte(offset)?)));
}

/// The free-form strings of the system builder, as many as the structure counts.
fn oem_strings(s: &SmbiosStructure, out: &mut Lines) {
    let count = usize::from(s.byte(0x04).unwrap_or(0));
    for (index, text) in s.strings.iter().take(count).enumerate() {
        out.pair(1, &format!("string[{index}]"), text.trim());
    }
}

/// The number of entries, then the string each entry refers to.
fn additional_information(s: &SmbiosStructure, out: &mut Lines) {
    let strings = additional_strings(s);
    out.opt(1, "string #", (!strings.is_empty()).then_some(strings.len()));
    for (index, text) in strings.into_iter().enumerate() {
        out.opt(1, &format!("string[{index}]"), text);
    }
}

fn memory_array(s: &SmbiosStructure, out: &mut Lines) {
    out.opt(1, "location", s.byte(0x04).map(|code| enum_name(ARRAY_LOCATIONS, code)));
    out.opt(1, "usage", s.byte(0x05).map(|code| enum_name(ARRAY_USES, code)));
    out.opt(1, "correction", s.byte(0x06).map(|code| enum_name(ERROR_CORRECTIONS, code)));
    out.opt(1, "max capacity", array_capacity_mb(s).map(capacity_text));
    out.opt(1, "max# of devices", s.word(0x0D).filter(|&count| count != 0));
}

fn memory_device(s: &SmbiosStructure, out: &mut Lines) {
    let bits = |offset| {
        s.word(offset).filter(|&width| width != 0 && width != 0xFFFF).map(|width| format!("{width} bits"))
    };
    let id = |offset| s.word(offset).filter(|&id| id != 0).map(|id| format!("0x{id:X}"));
    out.opt(1, "designation", s.string(field::DEVICE_LOCATOR));
    out.opt(1, "format", s.byte(field::FORM_FACTOR).map(form_factor));
    out.opt(1, "type", s.byte(field::MEMORY_TYPE).map(memory_type));
    out.opt(1, "total width", bits(field::TOTAL_WIDTH));
    out.opt(1, "data width", bits(field::DATA_WIDTH));
    out.opt(1, "size", device_size_mb(s).map(capacity_text));
    out.opt(1, "speed", device_speed(s).map(|speed| format!("{speed} MHz")));
    out.opt(1, "manufacturer", s.string(field::MANUFACTURER));
    out.opt(1, "part number", s.string(field::PART_NUMBER));
    out.opt(1, "serial number", s.string(field::SERIAL_NUMBER));
    let millivolts = s.word(field::CONFIGURED_VOLTAGE).filter(|&mv| mv != 0);
    out.opt(1, "voltage", millivolts.map(|mv| format!("{:.3} Volts", f64::from(mv) / 1000.0)));
    out.opt(1, "manufacturer id", id(field::MODULE_MANUFACTURER_ID));
    out.opt(1, "product id", id(field::MODULE_PRODUCT_ID));
}

fn slot(s: &SmbiosStructure, out: &mut Lines) {
    out.opt(1, "designation", s.string(0x04));
    out.opt(1, "type", s.byte(0x05).map(slot_type));
    let populated = s.byte(0x07).and_then(|usage| match usage {
        SLOT_AVAILABLE => Some("no"),
        SLOT_IN_USE => Some("yes"),
        _ => None,
    });
    out.opt(1, "populated", populated);
}

/// Name of a 1-based SMBIOS enumeration value. Values outside the table print as hex.
fn enum_name(names: &[&str], code: u8) -> String {
    let name = code.checked_sub(1).and_then(|index| names.get(usize::from(index)));
    name_or_hex(name.copied(), code)
}

/// An enumeration value by name, or in hex when it has none.
fn name_or_hex(name: Option<&str>, code: u8) -> String {
    name.map_or_else(|| format!("0x{code:X}"), str::to_string)
}

fn form_factor(code: u8) -> String {
    name_or_hex(form_factor_name(code), code)
}

fn memory_type(code: u8) -> String {
    name_or_hex(memory_type_name(code), code)
}

/// Name of a port connector type. `None` for the value that says there is no connector.
fn connector_type(code: u8) -> Option<String> {
    let name = match code {
        0x00 => return None,
        0xA0 => "PC-98",
        0xA1 => "PC-98Hireso",
        0xA2 => "PC-H98",
        0xA3 => "PC-98Note",
        0xA4 => "PC-98Full",
        0xFF => "Other",
        code => return Some(enum_name(CONNECTOR_TYPES, code)),
    };
    Some(name.to_string())
}

fn port_type(code: u8) -> String {
    let name = match code {
        0x00 => "None",
        0xA0 => "8251 Compatible",
        0xA1 => "8251 FIFO Compatible",
        0xFF => "Other",
        code => return enum_name(PORT_TYPES, code),
    };
    name.to_string()
}

/// The string of every entry of an additional information structure, in order;
/// `None` for an entry without one. The walk ends at an entry that does not fit
/// the structure.
fn additional_strings(s: &SmbiosStructure) -> Vec<Option<&str>> {
    let mut strings = Vec::new();
    let mut offset = 0x05;
    for _ in 0..s.byte(0x04).unwrap_or(0) {
        let length = s.byte(offset).map_or(0, usize::from);
        if length < ENTRY_MIN_LENGTH || offset + length > s.data.len() {
            break;
        }
        strings.push(s.string(offset + ENTRY_STRING));
        offset += length;
    }
    strings
}

/// Bus family of a system slot. The PCI Express codes differ only by generation
/// and lane count, which the report does not break out.
fn slot_type(code: u8) -> String {
    let name = match code {
        0x01 => "Other",
        0x02 => "Unknown",
        0x03 => "ISA",
        0x04 => "MCA",
        0x05 => "EISA",
        0x06 | 0x0E => "PCI",
        0x07 => "PCMCIA",
        0x08 => "VL-VESA",
        0x09 => "Proprietary",
        0x0F..=0x11 | 0x13 => "AGP",
        0x12 => "PCI-X",
        0x14..=0x17 => "M.2",
        0x21..=0x23 => "PCI Express Mini",
        0xA5..=0xB6 | 0xB8..=0xC4 => "PCI Express",
        _ => return format!("0x{code:X}"),
    };
    name.to_string()
}

/// BIOS ROM size in KB: the 64 KB-unit byte, or the extended word once that byte saturates.
fn rom_size_kb(s: &SmbiosStructure) -> Option<u64> {
    let classic = (u64::from(s.byte(0x09)?) + 1) * 64;
    let extended = s.word(0x18).filter(|_| s.byte(0x09) == Some(0xFF)).and_then(|word| {
        let size = u64::from(word & 0x3FFF);
        match word >> 14 {
            0 => Some(size * 1024),
            1 => Some(size * 1024 * 1024),
            _ => None,
        }
    });
    extended.filter(|&kb| kb != 0).or(Some(classic))
}

/// `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}` - the system UUID. Its first three
/// fields are stored little-endian. `None` when the firmware left it unset.
fn uuid_text(s: &SmbiosStructure) -> Option<String> {
    let b: &[u8; 16] = s.data.get(0x08..0x18)?.try_into().ok()?;
    if b.iter().all(|&byte| byte == 0x00) || b.iter().all(|&byte| byte == 0xFF) {
        return None;
    }
    let tail: String = b[10..].iter().map(|byte| format!("{byte:02X}")).collect();
    Some(format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{tail}}}",
        u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        u16::from_le_bytes([b[4], b[5]]),
        u16::from_le_bytes([b[6], b[7]]),
        b[8],
        b[9],
    ))
}

/// Maximum capacity of a memory array in MB.
fn array_capacity_mb(s: &SmbiosStructure) -> Option<u64> {
    match s.dword(0x07)? {
        0 => None,
        // The KB field saturates at 2 TB; the extended field holds bytes.
        0x8000_0000 => s.qword(0x0F).map(|bytes| bytes >> 20),
        kb => Some(u64::from(kb) >> 10),
    }
}

/// Size of a memory device in MB. `None` for an empty socket or an unknown size.
fn device_size_mb(s: &SmbiosStructure) -> Option<u64> {
    match DeviceSize::read(s)? {
        DeviceSize::Megabytes(mb) => Some(mb),
        DeviceSize::Empty | DeviceSize::Unknown => None,
    }
}

/// Speed of a memory device in MT/s (MHz on pre-DDR parts): the speed it is
/// configured to run at, or its rated speed when the firmware reports only that.
fn device_speed(s: &SmbiosStructure) -> Option<u32> {
    configured_speed(s).or_else(|| rated_speed(s))
}

/// `16 GB`, `512 MB` - a capacity given in MB.
fn capacity_text(mb: u64) -> String {
    if mb >= 1024 && mb % 1024 == 0 {
        format!("{} GB", mb / 1024)
    } else {
        format!("{mb} MB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::txt::lines_of;

    /// Builds a structure of `length` bytes with `fields` written at their offsets.
    fn structure(kind: u8, length: usize, fields: &[(usize, &[u8])], strings: &[&str]) -> SmbiosStructure {
        let mut data = vec![0u8; length];
        data[0] = kind;
        data[1] = length as u8;
        for (offset, bytes) in fields {
            data[*offset..*offset + bytes.len()].copy_from_slice(bytes);
        }
        SmbiosStructure { kind, handle: 0, data, strings: strings.iter().map(|s| s.to_string()).collect() }
    }

    fn decode(structure: SmbiosStructure) -> Vec<String> {
        let smbios = Smbios { major: 3, minor: 5, structures: vec![structure] };
        lines_of(&section(Some(&smbios))).split_off(2)
    }

    #[test]
    fn prints_the_version_and_skips_unknown_structures() {
        // Type 7 (cache information) is one of the structures the section does not decode.
        let smbios = Smbios {
            major: 3,
            minor: 5,
            structures: vec![structure(7, 0x1B, &[], &["L1"]), structure(kind::END_OF_TABLE, 4, &[], &[])],
        };
        assert_eq!(lines_of(&section(Some(&smbios))), ["SMBIOS Version\t\t\t3.5", ""]);
        assert!(lines_of(&section(None)).is_empty());
    }

    #[test]
    fn decodes_bios() {
        let fields: &[(usize, &[u8])] = &[(0x04, &[1, 2]), (0x08, &[3, 0xFF]), (0x18, &[0x10, 0x00])];
        let s = structure(kind::BIOS, 0x1A, fields, &["Example Firmware Inc.", "2.A0", "09/21/2023"]);
        assert_eq!(
            decode(s),
            [
                "DMI BIOS\t\t",
                "\tvendor\t\t\tExample Firmware Inc.",
                "\tversion\t\t\t2.A0",
                "\tdate\t\t\t09/21/2023",
                "\tROM size\t\t16384 KB",
                "",
            ]
        );
    }

    #[test]
    fn computes_rom_sizes() {
        // Structures older than SMBIOS 3.1 end before the extended size word.
        let classic = |size: u8| rom_size_kb(&structure(kind::BIOS, 0x12, &[(0x09, &[size])], &[]));
        assert_eq!(classic(0x0F), Some(1024));
        assert_eq!(classic(0xFF), Some(16 * 1024));

        let extended = |size: u8, word: u16| {
            let fields: &[(usize, &[u8])] = &[(0x09, &[size]), (0x18, &word.to_le_bytes())];
            rom_size_kb(&structure(kind::BIOS, 0x1A, fields, &[]))
        };
        assert_eq!(extended(0x7F, 32), Some(8 * 1024));
        assert_eq!(extended(0xFF, 32), Some(32 * 1024));
        assert_eq!(extended(0xFF, 0x4002), Some(2 * 1024 * 1024));
        assert_eq!(extended(0xFF, 0), Some(16 * 1024));
        assert_eq!(rom_size_kb(&structure(kind::BIOS, 8, &[], &[])), None);
    }

    #[test]
    fn decodes_system_information() {
        let uuid: [u8; 16] =
            [0x33, 0x22, 0x11, 0x00, 0x55, 0x44, 0x77, 0x66, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF];
        let fields: &[(usize, &[u8])] = &[(0x04, &[1, 2, 3, 4]), (0x08, &uuid), (0x19, &[5, 6])];
        let strings = ["Example Computer Inc.", "Tower One", "1.0", "SN-TEST-0001", "Desktop", "Towers"];
        assert_eq!(
            decode(structure(kind::SYSTEM, 0x1B, fields, &strings)),
            [
                "DMI System Information\t\t",
                "\tmanufacturer\t\tExample Computer Inc.",
                "\tproduct\t\t\tTower One",
                "\tversion\t\t\t1.0",
                "\tserial\t\t\tSN-TEST-0001",
                "\tUUID\t\t\t{00112233-4455-6677-8899-AABBCCDDEEFF}",
                "\tSKU\t\t\tDesktop",
                "\tfamily\t\t\tTowers",
                "",
            ]
        );
    }

    #[test]
    fn hides_unset_uuids() {
        for filler in [0x00, 0xFF] {
            assert_eq!(uuid_text(&structure(kind::SYSTEM, 0x1B, &[(0x08, &[filler; 16])], &[])), None);
        }
        assert_eq!(uuid_text(&structure(kind::SYSTEM, 0x08, &[], &[])), None);
    }

    #[test]
    fn decodes_baseboard_and_enclosure() {
        let board = structure(
            kind::BASEBOARD,
            0x0F,
            &[(0x04, &[1, 2, 3, 4])],
            &["ACME", "B650 TEST", "1.0", "BSN-1"],
        );
        assert_eq!(
            decode(board),
            [
                "DMI Baseboard\t\t",
                "\tvendor\t\t\tACME",
                "\tmodel\t\t\tB650 TEST",
                "\trevision\t\t1.0",
                "\tserial\t\t\tBSN-1",
                "",
            ]
        );
        // Type 0x83: a desktop chassis with the lock bit set.
        let chassis =
            structure(kind::CHASSIS, 0x15, &[(0x04, &[1, 0x83, 0, 2])], &["ACME", "Default string"]);
        assert_eq!(
            decode(chassis),
            [
                "DMI System Enclosure\t\t",
                "\tmanufacturer\t\tACME",
                "\tchassis type\t\tDesktop",
                "\tchassis serial\t\tDefault string",
                "",
            ]
        );
    }

    #[test]
    fn decodes_processor() {
        let fields: &[(usize, &[u8])] = &[
            (0x07, &[1]),
            (0x10, &[2]),
            (0x12, &100u16.to_le_bytes()),
            (0x14, &5350u16.to_le_bytes()),
            (0x16, &3800u16.to_le_bytes()),
        ];
        let s = structure(
            kind::PROCESSOR,
            0x30,
            fields,
            &["Example Devices Inc.", "Example CPU 8-Core Processor"],
        );
        assert_eq!(
            decode(s),
            [
                "DMI Processor\t\t",
                "\tmanufacturer\t\tExample Devices Inc.",
                "\tmodel\t\t\tExample CPU 8-Core Processor",
                "\tclock speed\t\t3800.0 MHz",
                "\tFSB speed\t\t100.0 MHz",
                "\tmultiplier\t\t38.0x",
                "\tmax clock speed\t\t5350.0 MHz",
                "",
            ]
        );
        // An empty socket reports no speeds at all.
        assert_eq!(decode(structure(kind::PROCESSOR, 0x30, &[], &[])), ["DMI Processor\t\t", ""]);
    }

    #[test]
    fn decodes_memory_array() {
        let fields: &[(usize, &[u8])] =
            &[(0x04, &[3, 3, 3]), (0x07, &(128u32 << 20).to_le_bytes()), (0x0D, &4u16.to_le_bytes())];
        assert_eq!(
            decode(structure(kind::MEMORY_ARRAY, 0x17, fields, &[])),
            [
                "DMI Physical Memory Array\t\t",
                "\tlocation\t\tMotherboard",
                "\tusage\t\t\tSystem Memory",
                "\tcorrection\t\tNone",
                "\tmax capacity\t\t128 GB",
                "\tmax# of devices\t\t4",
                "",
            ]
        );
    }

    #[test]
    fn reads_extended_array_capacity() {
        let fields: &[(usize, &[u8])] =
            &[(0x07, &0x8000_0000u32.to_le_bytes()), (0x0F, &(4u64 << 40).to_le_bytes())];
        assert_eq!(array_capacity_mb(&structure(kind::MEMORY_ARRAY, 0x17, fields, &[])), Some(4 << 20));
        assert_eq!(array_capacity_mb(&structure(kind::MEMORY_ARRAY, 0x17, &[], &[])), None);
    }

    #[test]
    fn decodes_memory_device() {
        let fields: &[(usize, &[u8])] = &[
            (0x08, &64u16.to_le_bytes()),
            (0x0A, &64u16.to_le_bytes()),
            (0x0C, &16384u16.to_le_bytes()),
            (0x0E, &[9]),
            (0x10, &[1]),
            (0x12, &[0x22]),
            (0x15, &5600u16.to_le_bytes()),
            (0x17, &[2, 3, 0, 4]),
            (0x20, &5200u16.to_le_bytes()),
            (0x26, &1100u16.to_le_bytes()),
            (0x2C, &0x2C80u16.to_le_bytes()),
        ];
        let strings = ["DIMMA2", "Example Memory", "00000001", "EX16G56C46U5"];
        assert_eq!(
            decode(structure(kind::MEMORY_DEVICE, 0x5C, fields, &strings)),
            [
                "DMI Memory Device\t\t",
                "\tdesignation\t\tDIMMA2",
                "\tformat\t\t\tDIMM",
                "\ttype\t\t\tDDR5",
                "\ttotal width\t\t64 bits",
                "\tdata width\t\t64 bits",
                "\tsize\t\t\t16 GB",
                "\tspeed\t\t\t5200 MHz",
                "\tmanufacturer\t\tExample Memory",
                "\tpart number\t\tEX16G56C46U5",
                "\tserial number\t\t00000001",
                "\tvoltage\t\t\t1.100 Volts",
                "\tmanufacturer id\t\t0x2C80",
                "",
            ]
        );
    }

    #[test]
    fn decodes_empty_memory_socket() {
        let fields: &[(usize, &[u8])] =
            &[(0x08, &[0xFF, 0xFF, 0xFF, 0xFF]), (0x0E, &[2]), (0x10, &[1]), (0x12, &[2])];
        assert_eq!(
            decode(structure(kind::MEMORY_DEVICE, 0x5C, fields, &["DIMMA1"])),
            [
                "DMI Memory Device\t\t",
                "\tdesignation\t\tDIMMA1",
                "\tformat\t\t\tUnknown",
                "\ttype\t\t\tUnknown",
                ""
            ]
        );
    }

    #[test]
    fn decodes_camm_module_of_unknown_size() {
        let fields: &[(usize, &[u8])] =
            &[(0x0C, &0x7FFFu16.to_le_bytes()), (0x0E, &[0x11]), (0x10, &[1]), (0x12, &[0x23])];
        assert_eq!(
            decode(structure(kind::MEMORY_DEVICE, 0x5C, fields, &["CAMM1"])),
            [
                "DMI Memory Device\t\t",
                "\tdesignation\t\tCAMM1",
                "\tformat\t\t\tCAMM",
                "\ttype\t\t\tLPDDR5",
                ""
            ]
        );
    }

    #[test]
    fn reads_memory_sizes_and_speeds() {
        let device = |size: u16, extended: u32| {
            structure(
                kind::MEMORY_DEVICE,
                0x5C,
                &[(0x0C, &size.to_le_bytes()), (0x1C, &extended.to_le_bytes())],
                &[],
            )
        };
        assert_eq!(device_size_mb(&device(0, 0)), None);
        assert_eq!(device_size_mb(&device(0xFFFF, 0)), None);
        assert_eq!(device_size_mb(&device(8192, 0)), Some(8192));
        assert_eq!(device_size_mb(&device(0x8000 | 2048, 0)), Some(2));
        assert_eq!(device_size_mb(&device(0x7FFF, 65536)), Some(65536));
        // The escape to an extended size the firmware left unset.
        assert_eq!(device_size_mb(&device(0x7FFF, 0)), None);

        // Rated speed (with its extended form) and configured speed (with its extended form).
        let speeds = |rated: u16, rated_ext: u32, configured: u16, configured_ext: u32| {
            let fields: &[(usize, &[u8])] = &[
                (0x15, &rated.to_le_bytes()),
                (0x54, &rated_ext.to_le_bytes()),
                (0x20, &configured.to_le_bytes()),
                (0x58, &configured_ext.to_le_bytes()),
            ];
            device_speed(&structure(kind::MEMORY_DEVICE, 0x5C, fields, &[]))
        };
        assert_eq!(speeds(0, 0, 0, 0), None);
        assert_eq!(speeds(3200, 0, 0, 0), Some(3200));
        assert_eq!(speeds(3200, 0, 2666, 0), Some(2666));
        assert_eq!(speeds(0xFFFF, 70_000, 0, 0), Some(70_000));
        assert_eq!(speeds(0xFFFF, 70_000, 0xFFFF, 66_000), Some(66_000));
        assert_eq!(speeds(0xFFFF, 0, 0xFFFF, 0), None);
        // A structure from before SMBIOS 2.7 ends ahead of the configured speed.
        let old = structure(kind::MEMORY_DEVICE, 0x1B, &[(0x15, &400u16.to_le_bytes())], &[]);
        assert_eq!(device_speed(&old), Some(400));
        assert_eq!(device_speed(&structure(kind::MEMORY_DEVICE, 0x15, &[], &[])), None);
    }

    #[test]
    fn formats_capacities() {
        assert_eq!(capacity_text(512), "512 MB");
        assert_eq!(capacity_text(1024), "1 GB");
        assert_eq!(capacity_text(1536), "1536 MB");
        assert_eq!(capacity_text(131_072), "128 GB");
    }

    #[test]
    fn decodes_extension_slots() {
        let slot = |kind_code: u8, usage: u8| {
            decode(structure(kind::SYSTEM_SLOT, 0x11, &[(0x04, &[1, kind_code, 0, usage])], &["PCIE1"]))
        };
        assert_eq!(
            slot(0xB6, 4),
            [
                "DMI Extension Slot\t\t",
                "\tdesignation\t\tPCIE1",
                "\ttype\t\t\tPCI Express",
                "\tpopulated\t\tyes",
                ""
            ]
        );
        assert_eq!(slot(0x06, 3)[2..4], ["\ttype\t\t\tPCI", "\tpopulated\t\tno"]);
        // Usage "unknown" says nothing about the slot being populated.
        assert_eq!(slot(0x17, 2)[2..], ["\ttype\t\t\tM.2", ""]);
    }

    #[test]
    fn decodes_port_connectors() {
        // An outside USB port wired to a header on the board.
        let usb =
            structure(kind::PORT_CONNECTOR, 9, &[(0x04, &[1, 0x00, 2, 0x12, 0x10])], &["J1A1", "USB 3.2"]);
        assert_eq!(
            decode(usb),
            [
                "DMI Port Connector\t\t",
                "\tdesignation\t\tJ1A1 (internal)",
                "\tdesignation\t\tUSB 3.2 (external)",
                "\tport type\t\tUSB",
                "\tconnector\t\tAccess Bus (USB)",
                "",
            ]
        );
        // No connector on either side leaves the line out.
        let video = structure(kind::PORT_CONNECTOR, 9, &[(0x04, &[1, 0, 2, 0, 0x1C])], &["J2", "Display"]);
        assert_eq!(
            decode(video)[1..],
            [
                "\tdesignation\t\tJ2 (internal)",
                "\tdesignation\t\tDisplay (external)",
                "\tport type\t\tVideo Port",
                ""
            ]
        );
        // A header without an outside connector is named by its own type.
        let header = structure(kind::PORT_CONNECTOR, 9, &[(0x04, &[1, 0x16, 0, 0, 0xFF])], &["IDE1"]);
        assert_eq!(
            decode(header)[1..],
            ["\tdesignation\t\tIDE1 (internal)", "\tport type\t\tOther", "\tconnector\t\tOn Board IDE", ""]
        );
        // A structure cut short of the port type still names its designators.
        let short = structure(kind::PORT_CONNECTOR, 6, &[(0x04, &[1, 0x0F])], &["KB1"]);
        assert_eq!(decode(short)[1..], ["\tdesignation\t\tKB1 (internal)", "\tconnector\t\tPS/2", ""]);
    }

    #[test]
    fn decodes_oem_strings() {
        let counted =
            structure(kind::OEM_STRINGS, 5, &[(0x04, &[2])], &["Build 42", " Factory A ", "not counted"]);
        assert_eq!(
            decode(counted),
            ["DMI OEM Strings\t\t", "\tstring[0]\t\tBuild 42", "\tstring[1]\t\tFactory A", ""]
        );
        // A count beyond the string set prints the strings that exist.
        let overstated = structure(kind::OEM_STRINGS, 5, &[(0x04, &[3])], &["Only"]);
        assert_eq!(decode(overstated)[1..], ["\tstring[0]\t\tOnly", ""]);
        assert_eq!(decode(structure(kind::OEM_STRINGS, 4, &[], &["Stray"])), ["DMI OEM Strings\t\t", ""]);
    }

    #[test]
    fn decodes_additional_information() {
        // Two entries: the first refers to the second string, the other holds a value only.
        let fields: &[(usize, &[u8])] = &[
            (0x04, &[2]),
            (0x05, &[6, 0x11, 0x00, 0x05, 2, 0x01]),
            (0x0B, &[7, 0x12, 0x00, 0x08, 0, 0x34, 0x12]),
        ];
        let s = structure(kind::ADDITIONAL_INFORMATION, 0x12, fields, &["unused", "Firmware module 9"]);
        assert_eq!(
            decode(s),
            [
                "DMI Additional Information Device\t\t",
                "\tstring #\t\t2",
                "\tstring[0]\t\tFirmware module 9",
                ""
            ]
        );
    }

    #[test]
    fn additional_information_stops_at_a_broken_entry() {
        let one_entry = |count: u8, length: u8| {
            let fields: &[(usize, &[u8])] = &[(0x04, &[count]), (0x05, &[length, 0, 0, 0, 1, 0])];
            decode(structure(kind::ADDITIONAL_INFORMATION, 0x0B, fields, &["A"]))
        };
        // The count promises three entries, the structure holds one.
        assert_eq!(one_entry(3, 6)[1..], ["\tstring #\t\t1", "\tstring[0]\t\tA", ""]);
        // An entry that runs past the structure, and one too short to hold its fields.
        assert_eq!(one_entry(1, 0x40), ["DMI Additional Information Device\t\t", ""]);
        assert_eq!(one_entry(1, 3), ["DMI Additional Information Device\t\t", ""]);
        assert_eq!(one_entry(0, 6), ["DMI Additional Information Device\t\t", ""]);
        let bare = structure(kind::ADDITIONAL_INFORMATION, 4, &[], &[]);
        assert_eq!(decode(bare), ["DMI Additional Information Device\t\t", ""]);
    }

    #[test]
    fn names_ports_and_connectors() {
        assert_eq!(port_type(0x00), "None");
        assert_eq!(port_type(0x01), "Parallel Port XT/AT Compatible");
        assert_eq!(port_type(0x10), "USB");
        assert_eq!(port_type(0x1F), "Network Port");
        assert_eq!(port_type(0x23), "Thunderbolt");
        assert_eq!(port_type(0x24), "0x24");
        assert_eq!(port_type(0xA1), "8251 FIFO Compatible");
        assert_eq!(port_type(0xFF), "Other");
        assert_eq!(connector_type(0x00), None);
        assert_eq!(connector_type(0x0B).as_deref(), Some("RJ-45"));
        assert_eq!(connector_type(0x12).as_deref(), Some("Access Bus (USB)"));
        assert_eq!(connector_type(0x1F).as_deref(), Some("Mini-jack (headphones)"));
        assert_eq!(connector_type(0x23).as_deref(), Some("USB Type-C Receptacle"));
        assert_eq!(connector_type(0x24).as_deref(), Some("0x24"));
        assert_eq!(connector_type(0xA4).as_deref(), Some("PC-98Full"));
        assert_eq!(connector_type(0xFF).as_deref(), Some("Other"));
    }

    #[test]
    fn names_enumerations() {
        assert_eq!(enum_name(CHASSIS_TYPES, 3), "Desktop");
        assert_eq!(enum_name(CHASSIS_TYPES, 10), "Notebook");
        assert_eq!(enum_name(CHASSIS_TYPES, 36), "Stick PC");
        assert_eq!(enum_name(CHASSIS_TYPES, 37), "0x25");
        assert_eq!(enum_name(CHASSIS_TYPES, 0), "0x0");
        assert_eq!(form_factor(13), "SODIMM");
        assert_eq!(form_factor(0x11), "CAMM");
        assert_eq!(form_factor(0x12), "0x12");
        assert_eq!(enum_name(ERROR_CORRECTIONS, 6), "Multi-bit ECC");
        assert_eq!(memory_type(0x1A), "DDR4");
        assert_eq!(memory_type(0x15), "0x15");
        assert_eq!(slot_type(0xA5), "PCI Express");
        assert_eq!(slot_type(0xBD), "PCI Express");
        assert_eq!(slot_type(0xB7), "0xB7");
        assert_eq!(slot_type(0x13), "AGP");
    }
}
