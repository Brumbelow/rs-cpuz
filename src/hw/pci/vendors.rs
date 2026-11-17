//! Short names for the PCI vendor ids most often met in a PC (public PCI-SIG assignments):
//! chip makers as well as the board and system makers found in subsystem vendor ids.

/// Short vendor name for a PCI vendor id, e.g. `0x1022` -> `AMD`.
pub fn vendor_name(vendor_id: u16) -> Option<&'static str> {
    let name = match vendor_id {
        0x1000 => "LSI",
        0x1002 | 0x1022 => "AMD",
        0x1019 => "ECS",
        0x1025 => "Acer",
        0x1028 => "Dell",
        0x1039 => "SiS",
        0x103C => "HP",
        0x1043 => "ASUS",
        0x104C => "Texas Instruments",
        0x104D => "Sony",
        0x106B => "Apple",
        0x107D => "Leadtek",
        0x10B0 => "Gainward",
        0x10B5 => "PLX",
        0x10B9 => "ULi",
        0x10CF => "Fujitsu",
        0x10DE => "NVIDIA",
        0x10EC => "Realtek",
        0x1102 => "Creative",
        0x1106 => "VIA",
        0x1179 => "Toshiba",
        0x11AB | 0x1B4B => "Marvell",
        0x126F => "Silicon Motion",
        0x1297 => "Shuttle",
        0x1344 | 0xC0A9 => "Micron",
        0x1414 => "Microsoft",
        0x144D => "Samsung",
        0x1458 => "Gigabyte",
        0x1462 => "MSI",
        0x148C => "PowerColor",
        0x14C3 => "MediaTek",
        0x14E4 => "Broadcom",
        0x152D => "Quanta",
        0x1545 => "VisionTek",
        0x1558 => "Clevo",
        0x1565 => "Biostar",
        0x1569 => "Palit",
        0x15AD => "VMware",
        0x15B3 => "Mellanox",
        0x15B7 => "SanDisk",
        0x15D9 => "Supermicro",
        0x1682 | 0x1EAE => "XFX",
        0x168C => "Qualcomm Atheros",
        0x174B | 0x1DA2 => "Sapphire",
        0x1787 => "HIS",
        0x17AA => "Lenovo",
        0x17CB => "Qualcomm",
        0x1849 => "ASRock",
        0x1912 => "Renesas",
        0x196D => "Club 3D",
        0x196E => "PNY",
        0x197B => "JMicron",
        0x1987 => "Phison",
        0x19DA => "ZOTAC",
        0x1A58 => "Razer",
        0x1ACC => "Point of View",
        0x1AF4 | 0x1B36 => "Red Hat",
        0x1B0A => "Pegatron",
        0x1B21 => "ASMedia",
        0x1B4C => "GALAX",
        0x1C5C => "SK hynix",
        0x1D17 => "Zhaoxin",
        0x1D6A => "Aquantia",
        0x1D94 => "Hygon",
        0x1E0F => "KIOXIA",
        0x1ED3 => "Yeston",
        0x2646 => "Kingston",
        0x3842 => "EVGA",
        0x7377 => "Colorful",
        0x8086 => "Intel",
        _ => return None,
    };
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_common_vendors() {
        assert_eq!(vendor_name(0x8086), Some("Intel"));
        assert_eq!(vendor_name(0x1022), Some("AMD"));
        assert_eq!(vendor_name(0x1002), Some("AMD"));
        assert_eq!(vendor_name(0x10DE), Some("NVIDIA"));
        assert_eq!(vendor_name(0x1462), Some("MSI"));
    }

    #[test]
    fn names_board_makers() {
        assert_eq!(vendor_name(0x1043), Some("ASUS"));
        assert_eq!(vendor_name(0x1DA2), Some("Sapphire"));
        assert_eq!(vendor_name(0x174B), Some("Sapphire"));
        assert_eq!(vendor_name(0x3842), Some("EVGA"));
    }

    #[test]
    fn unknown_vendor_has_no_name() {
        assert_eq!(vendor_name(0x0000), None);
        assert_eq!(vendor_name(0x1234), None);
        assert_eq!(vendor_name(0xFFFF), None);
    }
}
