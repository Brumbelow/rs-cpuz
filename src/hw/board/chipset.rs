//! Chipset naming: host bridges, LPC/eSPI bridges and chipset names found in board models.
//!
//! The device ids come from the public PCI id assignments and the vendors' datasheets.

use crate::model::{CpuInfo, CpuVendor};

const AMD: u16 = 0x1022;
const ATI: u16 = 0x1002;
const INTEL: u16 = 0x8086;

/// What a host bridge id says about the platform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HostBridge {
    /// Family-level name shown in the chipset row.
    pub name: &'static str,
    /// Fastest link speed in GT/s the root ports of this family advertise, when
    /// the family alone settles it.
    pub root_port_gts: Option<f32>,
    /// The LPC/eSPI bridge next to this host bridge is a Platform Controller Hub.
    pub has_pch: bool,
}

/// Looks a host bridge up by its PCI ids.
pub fn host_bridge(vendor_id: u16, device_id: u16) -> Option<HostBridge> {
    match vendor_id {
        AMD => amd_host_bridge(device_id),
        INTEL => intel_host_bridge(device_id),
        _ => None,
    }
}

/// Root complexes of the Zen-based processors, which are systems-on-chip.
fn amd_host_bridge(device_id: u16) -> Option<HostBridge> {
    let root_port_gts = match device_id {
        // Zen and Zen+ (family 17h models 00h-0Fh), Raven Ridge / Picasso, Renoir / Cezanne.
        0x1450 | 0x15D0 | 0x1630 => Some(8.0),
        // Zen 2 and Zen 3 (Matisse, Vermeer, Castle Peak, Rome, Milan), Phoenix.
        0x1480 | 0x14E8 => Some(16.0),
        // Genoa / Storm Peak, Raphael / Granite Ridge.
        0x14A4 | 0x14D8 => Some(32.0),
        // Shared by Mendocino and Rembrandt, which differ.
        0x14B5 => None,
        _ => return None,
    };
    Some(HostBridge { name: "Ryzen SOC", root_port_gts, has_pch: false })
}

/// Intel host bridges: the processor families since Sandy Bridge, whose id blocks are
/// allocated per family, and the classic chipsets emulated by virtual machines.
fn intel_host_bridge(device_id: u16) -> Option<HostBridge> {
    let (name, has_pch) = match device_id {
        0x1237 => ("i440FX", false),
        0x7190 | 0x7192 => ("i440BX", false),
        0x29C0 => ("P35/G33/G31", false),
        0x0100..=0x010F => ("Sandy Bridge", true),
        0x0150..=0x015F => ("Ivy Bridge", true),
        0x0A00..=0x0A0F | 0x0C00..=0x0C0F | 0x0D00..=0x0D0F => ("Haswell", true),
        0x1600..=0x161F => ("Broadwell", true),
        0x1900..=0x191F => ("Skylake", true),
        0x5900..=0x591F => ("Kaby Lake", true),
        0x3E00..=0x3EFF => ("Coffee Lake", true),
        0x8A00..=0x8AFF => ("Ice Lake", true),
        0x9B00..=0x9BFF => ("Comet Lake", true),
        0x9A00..=0x9AFF => ("Tiger Lake", true),
        0x4C00..=0x4CFF => ("Rocket Lake", true),
        0x4600..=0x46FF => ("Alder Lake", true),
        0xA700..=0xA7FF => ("Raptor Lake", true),
        _ => return None,
    };
    Some(HostBridge { name, root_port_gts: None, has_pch })
}

/// Model text of the chipset row. The host bridge of current platforms is part
/// of the processor, so the processor refines what the table says: server and
/// desktop parts share root complex ids, and a host bridge missing from the
/// table still belongs to the processor's family.
pub fn northbridge_model(vendor_id: u16, device_id: u16, cpu: &CpuInfo) -> String {
    let known = host_bridge(vendor_id, device_id).map(|bridge| bridge.name);
    let model = match vendor_id {
        AMD if cpu.name.contains("EPYC") => Some("EPYC SOC"),
        AMD if cpu.name.contains("Ryzen") => known.or(Some("Ryzen SOC")),
        INTEL if cpu.vendor == CpuVendor::Intel => known.or(cpu.codename.as_deref()),
        _ => known,
    };
    model.unwrap_or_default().to_string()
}

/// Chipset model for LPC/eSPI bridges whose device id identifies one chipset.
pub fn lpc_bridge_model(vendor_id: u16, device_id: u16) -> Option<&'static str> {
    match vendor_id {
        INTEL => intel_lpc_bridge_model(device_id),
        ATI if device_id == 0x439D => Some("SB7x0/SB8x0/SB9x0"),
        _ => None,
    }
}

fn intel_lpc_bridge_model(device_id: u16) -> Option<&'static str> {
    let model = match device_id {
        0x7000 => "PIIX3",
        0x7110 => "PIIX4",
        0x27B0 => "ICH7DH",
        0x27B8 => "ICH7",
        0x27B9 => "ICH7-M",
        0x27BD => "ICH7-M DH",
        0x2810 => "ICH8",
        0x2811 => "ICH8M-E",
        0x2812 => "ICH8DH",
        0x2814 => "ICH8DO",
        0x2815 => "ICH8M",
        0x2912 => "ICH9DH",
        0x2914 => "ICH9DO",
        0x2916 => "ICH9R",
        0x2917 => "ICH9M-E",
        0x2918 => "ICH9",
        0x2919 => "ICH9M",
        0x3A14 => "ICH10DO",
        0x3A16 => "ICH10R",
        0x3A18 => "ICH10",
        0x3A1A => "ICH10D",
        0x3B02 => "P55",
        0x3B03 => "PM55",
        0x3B06 => "H55",
        0x3B07 => "QM57",
        0x3B08 => "H57",
        0x3B09 => "HM55",
        0x3B0A => "Q57",
        0x3B0B => "HM57",
        0x3B0F => "QS57",
        0x1C44 => "Z68",
        0x1C46 => "P67",
        0x1C47 => "UM67",
        0x1C49 => "HM65",
        0x1C4A => "H67",
        0x1C4B => "HM67",
        0x1C4C => "Q65",
        0x1C4D => "QS67",
        0x1C4E => "Q67",
        0x1C4F => "QM67",
        0x1C50 => "B65",
        0x1C5C => "H61",
        0x1E44 => "Z77",
        0x1E46 => "Z75",
        0x1E47 => "Q77",
        0x1E48 => "Q75",
        0x1E49 => "B75",
        0x1E4A => "H77",
        0x1E55 => "QM77",
        0x1E56 => "QS77",
        0x1E57 => "HM77",
        0x1E58 => "UM77",
        0x1E59 => "HM76",
        0x1E5D => "HM75",
        0x8C44 => "Z87",
        0x8C46 => "Z85",
        0x8C49 => "HM86",
        0x8C4A => "H87",
        0x8C4B => "HM87",
        0x8C4C => "Q85",
        0x8C4E => "Q87",
        0x8C4F => "QM87",
        0x8C50 => "B85",
        0x8C5C => "H81",
        0x8CC4 => "Z97",
        0x8CC6 => "H97",
        0xA143 => "H110",
        0xA144 => "H170",
        0xA145 => "Z170",
        0xA146 => "Q170",
        0xA147 => "Q150",
        0xA148 => "B150",
        0xA14D => "QM170",
        0xA14E => "HM170",
        0xA2C4 => "H270",
        0xA2C5 => "Z270",
        0xA2C6 => "Q270",
        0xA2C7 => "Q250",
        0xA2C8 => "B250",
        0xA2C9 => "Z370",
        0xA2D2 => "X299",
        0xA303 => "H310",
        0xA304 => "H370",
        0xA305 => "Z390",
        0xA306 => "Q370",
        0xA308 => "B360",
        0x0684 => "H470",
        0x0685 => "Z490",
        0x0687 => "Q470",
        0x0697 => "W480",
        0xA3C8 => "B460",
        0xA3DA => "H410",
        0x4384 => "Q570",
        0x4385 => "Z590",
        0x4386 => "H570",
        0x4387 => "B560",
        0x4388 => "H510",
        0x438F => "W580",
        0x7A83 => "Q670",
        0x7A84 => "Z690",
        0x7A85 => "H670",
        0x7A86 => "B660",
        0x7A87 => "H610",
        0x7A88 => "W680",
        0x7A04 => "Z790",
        0x7A05 => "H770",
        0x7A06 => "B760",
        _ => return None,
    };
    Some(model)
}

/// Family name for an LPC/eSPI bridge when neither its id nor the board model
/// gives the chipset: AMD's controller hubs share one id, and Intel pairs every
/// processor since Sandy Bridge with a Platform Controller Hub.
pub fn lpc_bridge_family(vendor_id: u16, device_id: u16, host: Option<HostBridge>) -> Option<&'static str> {
    match (vendor_id, device_id) {
        (AMD, 0x780E | 0x790E) => Some("FCH"),
        (INTEL, _) if host.is_some_and(|bridge| bridge.has_pch) => Some("PCH"),
        _ => None,
    }
}

/// AMD chipset names. A name that extends another one comes first.
const AMD_CHIPSETS: &[&str] = &[
    "X870E", "X870", "B850", "B840", "X670E", "X670", "B650E", "B650", "A620", "X570", "B550", "A520",
    "X470", "B450", "X370", "B350", "A320", "X399", "TRX40", "TRX50", "WRX80", "WRX90",
];

/// Intel chipset names since the 100 series. A name that extends another one comes first.
const INTEL_CHIPSETS: &[&str] = &[
    "Z890", "W880", "Q870", "B860", "H810", "Z790", "W790", "H770", "B760", "Z690", "W680", "H670", "Q670",
    "B660", "H610", "Z590", "W580", "H570", "Q570", "B560", "H510", "Z490", "W480", "H470", "Q470", "B460",
    "H410", "Z390", "Z370", "H370", "Q370", "B365", "B360", "H310C", "H310", "X299", "Z270", "H270", "Q270",
    "B250", "Q250", "Z170", "H170", "Q170", "B150", "Q150", "H110",
];

/// Finds the chipset name in a retail board model such as `PRO B650-EX WIFI (EX-1234)`
/// or `H110M-A/M.2`. Only names of the given bridge vendor are considered.
pub fn board_chipset(model: &str, vendor_id: u16) -> Option<&'static str> {
    let names = match vendor_id {
        AMD => AMD_CHIPSETS,
        INTEL => INTEL_CHIPSETS,
        _ => return None,
    };
    let text = model.to_ascii_uppercase();
    let bytes = text.as_bytes();
    (0..bytes.len()).filter(|&at| starts_name(bytes, at)).find_map(|at| {
        let rest = &bytes[at..];
        names.iter().copied().find(|name| {
            rest.starts_with(name.as_bytes()) && !rest.get(name.len()).is_some_and(u8::is_ascii_digit)
        })
    })
}

/// A chipset name starts a word, or follows the single `A` some vendors put
/// in front of it (`AB350M-Gaming 3`, `GA-AX370-Gaming 5`).
fn starts_name(bytes: &[u8], at: usize) -> bool {
    let starts_word = |at: usize| at == 0 || !bytes[at - 1].is_ascii_alphanumeric();
    starts_word(at) || (bytes[at - 1] == b'A' && starts_word(at - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpu(vendor: CpuVendor, name: &str, codename: Option<&str>) -> CpuInfo {
        CpuInfo { vendor, name: name.into(), codename: codename.map(String::from), ..Default::default() }
    }

    #[test]
    fn names_amd_root_complexes() {
        let raphael = host_bridge(AMD, 0x14D8).unwrap();
        assert_eq!(raphael.name, "Ryzen SOC");
        assert_eq!(raphael.root_port_gts, Some(32.0));
        assert_eq!(host_bridge(AMD, 0x1480).unwrap().root_port_gts, Some(16.0));
        assert_eq!(host_bridge(AMD, 0x1630).unwrap().root_port_gts, Some(8.0));
        assert_eq!(host_bridge(AMD, 0x14B5).unwrap().root_port_gts, None);
        assert_eq!(host_bridge(AMD, 0x14DA), None);
    }

    #[test]
    fn names_intel_host_bridges_by_family() {
        assert_eq!(host_bridge(INTEL, 0xA700).unwrap().name, "Raptor Lake");
        assert_eq!(host_bridge(INTEL, 0x4660).unwrap().name, "Alder Lake");
        assert_eq!(host_bridge(INTEL, 0x3EC2).unwrap().name, "Coffee Lake");
        assert_eq!(host_bridge(INTEL, 0x191F).unwrap().name, "Skylake");
        assert_eq!(host_bridge(INTEL, 0x0154).unwrap().name, "Ivy Bridge");
        assert!(host_bridge(INTEL, 0xA700).unwrap().has_pch);
        assert!(!host_bridge(INTEL, 0x1237).unwrap().has_pch);
        assert_eq!(host_bridge(INTEL, 0x1980), None);
        assert_eq!(host_bridge(0x1106, 0x0100), None);
    }

    #[test]
    fn northbridge_model_uses_table_then_processor() {
        let unknown = CpuInfo::default();
        assert_eq!(northbridge_model(AMD, 0x14D8, &unknown), "Ryzen SOC");
        assert_eq!(northbridge_model(AMD, 0x14A4, &cpu(CpuVendor::Amd, "AMD EPYC 9354", None)), "EPYC SOC");
        assert_eq!(
            northbridge_model(AMD, 0x0001, &cpu(CpuVendor::Amd, "AMD Ryzen 9 9950X", None)),
            "Ryzen SOC"
        );
        assert_eq!(northbridge_model(AMD, 0x0001, &unknown), "");
        assert_eq!(northbridge_model(INTEL, 0xA700, &unknown), "Raptor Lake");
        let arrow_lake = cpu(CpuVendor::Intel, "Intel Core Ultra 7 265K", Some("Arrow Lake"));
        assert_eq!(northbridge_model(INTEL, 0x0001, &arrow_lake), "Arrow Lake");
        assert_eq!(northbridge_model(INTEL, 0x0001, &cpu(CpuVendor::Intel, "Intel Core", None)), "");
        assert_eq!(northbridge_model(INTEL, 0x0001, &cpu(CpuVendor::Amd, "", Some("Raphael"))), "");
        assert_eq!(northbridge_model(0x1106, 0x0001, &arrow_lake), "");
    }

    #[test]
    fn names_lpc_bridges() {
        assert_eq!(lpc_bridge_model(INTEL, 0x7A04), Some("Z790"));
        assert_eq!(lpc_bridge_model(INTEL, 0x7A86), Some("B660"));
        assert_eq!(lpc_bridge_model(INTEL, 0xA305), Some("Z390"));
        assert_eq!(lpc_bridge_model(INTEL, 0x7110), Some("PIIX4"));
        assert_eq!(lpc_bridge_model(INTEL, 0xFFFF), None);
        assert_eq!(lpc_bridge_model(ATI, 0x439D), Some("SB7x0/SB8x0/SB9x0"));
        assert_eq!(lpc_bridge_model(AMD, 0x790E), None);
    }

    #[test]
    fn names_lpc_bridge_families() {
        assert_eq!(lpc_bridge_family(AMD, 0x790E, None), Some("FCH"));
        assert_eq!(lpc_bridge_family(AMD, 0x1234, host_bridge(AMD, 0x14D8)), None);
        assert_eq!(lpc_bridge_family(INTEL, 0x1234, host_bridge(INTEL, 0x9A14)), Some("PCH"));
        assert_eq!(lpc_bridge_family(INTEL, 0x1234, host_bridge(INTEL, 0x1237)), None);
        assert_eq!(lpc_bridge_family(INTEL, 0x1234, None), None);
    }

    #[test]
    fn finds_chipset_in_board_model() {
        assert_eq!(board_chipset("PRO B650-EX WIFI (EX-1234)", AMD), Some("B650"));
        assert_eq!(board_chipset("ROG STRIX B650E-F GAMING WIFI", AMD), Some("B650E"));
        assert_eq!(board_chipset("X670E AORUS MASTER", AMD), Some("X670E"));
        assert_eq!(board_chipset("B550M DS3H", AMD), Some("B550"));
        assert_eq!(board_chipset("TUF GAMING X570-PLUS", AMD), Some("X570"));
        assert_eq!(board_chipset("AB350M-Gaming 3", AMD), Some("B350"));
        assert_eq!(board_chipset("GA-AX370-Gaming 5", AMD), Some("X370"));
        assert_eq!(board_chipset("a520m-hdv", AMD), Some("A520"));
        assert_eq!(board_chipset("H110M-A/M.2", INTEL), Some("H110"));
        assert_eq!(board_chipset("H310CM-HDV", INTEL), Some("H310C"));
        assert_eq!(board_chipset("Z890 AORUS ELITE", INTEL), Some("Z890"));
    }

    #[test]
    fn rejects_lookalikes_and_foreign_names() {
        assert_eq!(board_chipset("0KWVT8", AMD), None);
        assert_eq!(board_chipset("MB6500-PRO", AMD), None);
        assert_eq!(board_chipset("B6500", AMD), None);
        assert_eq!(board_chipset("ZB650", AMD), None);
        assert_eq!(board_chipset("AAB350", AMD), None);
        assert_eq!(board_chipset("Z790 AORUS", AMD), None);
        assert_eq!(board_chipset("PRO B650-EX WIFI", INTEL), None);
        assert_eq!(board_chipset("B650", 0x1106), None);
        assert_eq!(board_chipset("", AMD), None);
    }

    #[test]
    fn longer_chipset_names_are_not_shadowed() {
        for names in [AMD_CHIPSETS, INTEL_CHIPSETS] {
            for (index, name) in names.iter().enumerate() {
                assert!(
                    !names[..index].iter().any(|earlier| name.starts_with(earlier)),
                    "{name} is shadowed"
                );
            }
        }
    }
}
