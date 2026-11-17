//! Mainboard and BIOS identification from the SMBIOS tables.

use crate::hw::smbios::{is_placeholder, kind, Smbios, SmbiosStructure};

/// String-index offsets shared by the system (type 1) and baseboard (type 2) structures.
const MANUFACTURER: usize = 4;
const PRODUCT: usize = 5;
const VERSION: usize = 6;

/// String-index offsets of the BIOS (type 0) structure.
const BIOS_VENDOR: usize = 4;
const BIOS_VERSION: usize = 5;
const BIOS_DATE: usize = 8;

/// Offset of the chassis type in the enclosure (type 3) structure.
const CHASSIS_TYPE: usize = 5;

/// Marker AMD firmware puts in front of its AGESA version string.
const AGESA_MARKER: &str = "AGESA!V9";

/// Board and BIOS strings; empty where the firmware has nothing meaningful.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Identity {
    pub manufacturer: String,
    pub model: String,
    pub version: String,
    pub bios_brand: String,
    pub bios_version: String,
    pub bios_date: String,
}

/// Reads the board identity from the baseboard structure, falling back to the
/// system structure, and the BIOS identity from the BIOS structure.
pub fn identity(smbios: &Smbios) -> Identity {
    let board = smbios.first(kind::BASEBOARD);
    let system = smbios.first(kind::SYSTEM);
    let bios = smbios.first(kind::BIOS);
    let field =
        |structure: Option<&SmbiosStructure>, offset| structure.and_then(|s| meaningful(s.string(offset)));

    // The version only describes the product it was read with.
    let (model, version) = [board, system]
        .into_iter()
        .find_map(|s| field(s, PRODUCT).map(|model| (model, field(s, VERSION).unwrap_or_default())))
        .unwrap_or_default();
    Identity {
        manufacturer: field(board, MANUFACTURER).or_else(|| field(system, MANUFACTURER)).unwrap_or_default(),
        model,
        version,
        bios_brand: field(bios, BIOS_VENDOR).unwrap_or_default(),
        bios_version: bios_version(field(bios, BIOS_VERSION), agesa_version(smbios)),
        bios_date: field(bios, BIOS_DATE).unwrap_or_default(),
    }
}

/// True when the enclosure is a portable one, whose model names are not retail board names.
pub fn is_portable(smbios: &Smbios) -> bool {
    // Bit 7 of the chassis type flags a chassis lock.
    let chassis = smbios.first(kind::CHASSIS).and_then(|s| s.byte(CHASSIS_TYPE)).map(|kind| kind & 0x7F);
    // Portable, laptop, notebook, hand held, sub notebook, tablet, convertible, detachable.
    matches!(chassis, Some(8..=11 | 14 | 30..=32))
}

/// The AGESA version AMD firmware publishes: the string that follows the marker
/// in the string set of one of the structures.
fn agesa_version(smbios: &Smbios) -> Option<&str> {
    smbios.structures.iter().find_map(|structure| {
        let marker = structure.strings.iter().position(|text| text.trim() == AGESA_MARKER)?;
        let version = structure.strings.get(marker + 1)?.trim();
        (!version.is_empty()).then_some(version)
    })
}

/// `2.A0`, or `2.A0 - AMD AGESA ComboAM5PI 1.0.8.0` when the firmware names its AGESA.
fn bios_version(version: Option<String>, agesa: Option<&str>) -> String {
    let agesa = agesa.map(|agesa| format!("AMD AGESA {agesa}"));
    version.into_iter().chain(agesa).collect::<Vec<_>>().join(" - ")
}

/// Drops placeholders and tidies what is left: commas are removed
/// (`Example Co., Ltd.` reads `Example Co. Ltd.`) and runs of whitespace collapsed.
fn meaningful(text: Option<&str>) -> Option<String> {
    let text = text?.replace(',', "");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!is_placeholder(&text) && text.chars().any(char::is_alphanumeric)).then_some(text)
}

#[cfg(test)]
pub mod fixtures {
    use super::*;

    /// Builds a structure whose formatted area references `strings` in order from offset 4.
    pub fn structure(kind: u8, strings: &[&str]) -> SmbiosStructure {
        let mut data = vec![kind, 0, 0, 0];
        data.extend(1..=strings.len() as u8);
        data.resize(data.len().max(12), 0);
        data[1] = data.len() as u8;
        SmbiosStructure { kind, handle: 0, data, strings: strings.iter().map(|s| s.to_string()).collect() }
    }

    /// A BIOS structure: vendor, version and release date sit at offsets 4, 5 and 8.
    pub fn bios(vendor: &str, version: &str, date: &str) -> SmbiosStructure {
        let mut bios = structure(kind::BIOS, &[vendor, version, date]);
        bios.data[6] = 0;
        bios.data[BIOS_DATE] = 3;
        bios
    }

    pub fn chassis(chassis_type: u8) -> SmbiosStructure {
        let mut chassis = structure(kind::CHASSIS, &["Example Computer Inc."]);
        chassis.data[CHASSIS_TYPE] = chassis_type;
        chassis
    }

    pub fn smbios(structures: Vec<SmbiosStructure>) -> Smbios {
        Smbios { major: 3, minor: 5, structures }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn reads_board_and_bios() {
        let tables = smbios(vec![
            bios("Example Firmware, LLC.", "2.A0", "09/21/2023"),
            structure(kind::SYSTEM, &["Example Computer Co., Ltd.", "Tower 5", "2.0"]),
            structure(kind::BASEBOARD, &["EXAMPLE", "PRO B650-EX WIFI (EX-1234)", "1.0"]),
        ]);
        assert_eq!(
            identity(&tables),
            Identity {
                manufacturer: "EXAMPLE".into(),
                model: "PRO B650-EX WIFI (EX-1234)".into(),
                version: "1.0".into(),
                bios_brand: "Example Firmware LLC.".into(),
                bios_version: "2.A0".into(),
                bios_date: "09/21/2023".into(),
            }
        );
    }

    #[test]
    fn falls_back_to_the_system_structure() {
        let tables = smbios(vec![
            structure(kind::SYSTEM, &["Example Computer Co., Ltd.", "Notebook 14", "Rev A"]),
            structure(kind::BASEBOARD, &["To be filled by O.E.M.", "Default string", "1.0"]),
        ]);
        let id = identity(&tables);
        assert_eq!(id.manufacturer, "Example Computer Co. Ltd.");
        assert_eq!(id.model, "Notebook 14");
        assert_eq!(id.version, "Rev A");
        assert_eq!(id.bios_brand, "");

        let system_only = smbios(vec![structure(kind::SYSTEM, &["Maker", "Model", "System Version"])]);
        let id = identity(&system_only);
        assert_eq!(
            (id.manufacturer.as_str(), id.model.as_str(), id.version.as_str()),
            ("Maker", "Model", "")
        );
    }

    #[test]
    fn version_is_not_mixed_between_structures() {
        let tables = smbios(vec![
            structure(kind::SYSTEM, &["System manufacturer", "System Product Name", "System Version"]),
            structure(kind::BASEBOARD, &["Maker", "PRIME B550M-A", "Default string"]),
        ]);
        let id = identity(&tables);
        assert_eq!(
            (id.manufacturer.as_str(), id.model.as_str(), id.version.as_str()),
            ("Maker", "PRIME B550M-A", "")
        );
    }

    #[test]
    fn empty_tables_give_empty_identity() {
        assert_eq!(identity(&smbios(Vec::new())), Identity::default());
    }

    #[test]
    fn drops_placeholders_and_tidies_text() {
        assert_eq!(meaningful(Some("To Be Filled By O.E.M.")), None);
        assert_eq!(meaningful(Some("Default string")), None);
        assert_eq!(meaningful(Some("System Product Name")), None);
        assert_eq!(meaningful(Some("N/A")), None);
        assert_eq!(meaningful(Some("Undefined")), None);
        assert_eq!(meaningful(Some("NULL")), None);
        assert_eq!(meaningful(Some("Not  Installed")), None);
        assert_eq!(meaningful(Some("....")), None);
        assert_eq!(meaningful(Some("")), None);
        assert_eq!(meaningful(None), None);
        assert_eq!(meaningful(Some("Example  Firmware,  LLC.")), Some("Example Firmware LLC.".into()));
        assert_eq!(meaningful(Some("x.x")), Some("x.x".into()));
    }

    #[test]
    fn appends_agesa_version() {
        let tables = smbios(vec![
            bios("Example Firmware", "2.A0", "09/21/2023"),
            structure(40, &[AGESA_MARKER, "ComboAM5PI 1.0.8.0"]),
        ]);
        assert_eq!(identity(&tables).bios_version, "2.A0 - AMD AGESA ComboAM5PI 1.0.8.0");

        let marker_only =
            smbios(vec![bios("Example Firmware", "F4", "01/02/2024"), structure(40, &[AGESA_MARKER])]);
        assert_eq!(identity(&marker_only).bios_version, "F4");
        assert_eq!(bios_version(None, Some("ComboAm4v2PI 1.2.0.B")), "AMD AGESA ComboAm4v2PI 1.2.0.B");
        assert_eq!(bios_version(None, None), "");
    }

    #[test]
    fn recognises_portable_enclosures() {
        assert!(is_portable(&smbios(vec![chassis(10)])));
        assert!(is_portable(&smbios(vec![chassis(0x80 | 9)])));
        assert!(is_portable(&smbios(vec![chassis(31)])));
        assert!(!is_portable(&smbios(vec![chassis(3)])));
        assert!(!is_portable(&smbios(vec![chassis(13)])));
        assert!(!is_portable(&smbios(Vec::new())));
    }
}
