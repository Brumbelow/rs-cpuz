//! Display driver details from the display class key of the registry.
//!
//! Every installed display driver has a numbered subkey there (`0000`, `0001`, ...)
//! with its version, its date and the memory size the driver reported at start-up.

use std::cmp::Reverse;

use windows::core::{w, Owned, PCWSTR, PWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, REG_BINARY,
    REG_DWORD, REG_QWORD, REG_SZ, REG_VALUE_TYPE,
};

use super::dxgi::Adapter;

const DISPLAY_CLASS: PCWSTR =
    w!(r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}");

/// Registry key names are at most 255 characters long.
const MAX_KEY_NAME: usize = 256;

/// Upper bound on the subkeys visited and on the size of a value read.
const MAX_SUBKEYS: u32 = 1024;
const MAX_VALUE_BYTES: u32 = 4096;

/// The identifying fields of a PCI hardware id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HardwareId {
    pub vendor: u16,
    pub device: u16,
    /// Subsystem id in the high word, subsystem vendor id in the low word.
    pub subsys: Option<u32>,
    pub revision: Option<u8>,
}

/// One installed display driver.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Driver {
    pub description: String,
    /// The hardware id the driver package was matched against.
    pub hardware_id: HardwareId,
    pub version: Option<String>,
    /// Formatted as `MM/DD/YYYY`.
    pub date: Option<String>,
    /// Video memory size in bytes as reported by the driver.
    pub memory_bytes: Option<u64>,
}

/// Reads the display drivers registered for PCI devices.
pub fn installed() -> Vec<Driver> {
    let Some(class) = open(HKEY_LOCAL_MACHINE, DISPLAY_CLASS) else {
        return Vec::new();
    };
    let names = subkey_names(*class);
    names
        .iter()
        .filter(|name| is_instance_name(name))
        .filter_map(|name| open(*class, PCWSTR(name.as_ptr())))
        .filter_map(|key| read(*key))
        .collect()
}

fn read(key: HKEY) -> Option<Driver> {
    let hardware_id = parse_hardware_id(&text(key, w!("MatchingDeviceId"))?)?;
    Some(Driver {
        description: text(key, w!("DriverDesc")).unwrap_or_default(),
        hardware_id,
        version: text(key, w!("DriverVersion")),
        date: text(key, w!("DriverDate")).as_deref().and_then(format_date),
        memory_bytes: value(key, w!("HardwareInformation.qwMemorySize"))
            .and_then(|(kind, data)| decode_uint(kind, &data))
            .filter(|&bytes| bytes > 0),
    })
}

fn open(parent: HKEY, path: PCWSTR) -> Option<Owned<HKEY>> {
    let mut key = HKEY::default();
    // SAFETY: `path` is NUL-terminated and `key` is a valid out pointer. On success the
    // handle is ours to close, which `Owned` does.
    unsafe { (RegOpenKeyExW(parent, path, 0, KEY_READ, &mut key) == ERROR_SUCCESS).then(|| Owned::new(key)) }
}

/// Names of the direct subkeys, each NUL-terminated.
fn subkey_names(key: HKEY) -> Vec<Vec<u16>> {
    let mut names = Vec::new();
    for index in 0..MAX_SUBKEYS {
        let mut name = [0u16; MAX_KEY_NAME];
        let mut length = name.len() as u32;
        // SAFETY: `name` holds `length` characters and both outlive the call; the optional
        // class and timestamp outputs are not requested.
        let status = unsafe {
            RegEnumKeyExW(key, index, PWSTR(name.as_mut_ptr()), &mut length, None, PWSTR::null(), None, None)
        };
        if status != ERROR_SUCCESS {
            break;
        }
        // `length` excludes the terminator, which is kept so the name can be passed back as a path.
        names.push(name[..name.len().min(length as usize + 1)].to_vec());
    }
    names
}

/// Device instances are the four-digit subkeys; the class key also holds `Properties` and the like.
fn is_instance_name(name: &[u16]) -> bool {
    matches!(name, [digits @ .., 0] if digits.len() == 4
        && digits.iter().all(|&unit| (u16::from(b'0')..=u16::from(b'9')).contains(&unit)))
}

/// Type and raw data of a value; `None` when it is missing, empty or implausibly large.
fn value(key: HKEY, name: PCWSTR) -> Option<(REG_VALUE_TYPE, Vec<u8>)> {
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = 0u32;
    // SAFETY: without a data buffer the call only reports the type and size of the value.
    let status = unsafe { RegQueryValueExW(key, name, None, Some(&mut kind), None, Some(&mut size)) };
    if status != ERROR_SUCCESS || size == 0 || size > MAX_VALUE_BYTES {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    // SAFETY: `data` is `size` bytes long, which is the capacity passed in `size`.
    let status = unsafe {
        RegQueryValueExW(key, name, None, Some(&mut kind), Some(data.as_mut_ptr()), Some(&mut size))
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    data.truncate(size as usize);
    Some((kind, data))
}

fn text(key: HKEY, name: PCWSTR) -> Option<String> {
    let (kind, data) = value(key, name)?;
    decode_text(kind, &data)
}

/// A `REG_SZ` value as trimmed text; `None` when it is another type or blank.
fn decode_text(kind: REG_VALUE_TYPE, data: &[u8]) -> Option<String> {
    if kind != REG_SZ {
        return None;
    }
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|&unit| unit != 0)
        .collect();
    let text = String::from_utf16_lossy(&units).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// A `REG_QWORD` or `REG_DWORD` value (or binary data of either size) as an integer.
fn decode_uint(kind: REG_VALUE_TYPE, data: &[u8]) -> Option<u64> {
    if ![REG_QWORD, REG_DWORD, REG_BINARY].contains(&kind) {
        return None;
    }
    match *data {
        [a, b, c, d] => Some(u64::from(u32::from_le_bytes([a, b, c, d]))),
        [a, b, c, d, e, f, g, h] => Some(u64::from_le_bytes([a, b, c, d, e, f, g, h])),
        _ => None,
    }
}

/// Parses a PCI hardware id such as `PCI\VEN_10DE&DEV_2882&SUBSYS_51611462&REV_A1`.
/// The subsystem and revision parts are optional; ids of other buses yield `None`.
pub fn parse_hardware_id(id: &str) -> Option<HardwareId> {
    let (bus, fields) = id.trim().split_once('\\')?;
    if !bus.eq_ignore_ascii_case("PCI") {
        return None;
    }
    let (mut vendor, mut device, mut subsys, mut revision) = (None, None, None, None);
    for field in fields.split('&') {
        let Some((key, digits)) = field.split_once('_') else {
            continue;
        };
        match key.to_ascii_uppercase().as_str() {
            "VEN" => vendor = Some(hex(digits, 4)? as u16),
            "DEV" => device = Some(hex(digits, 4)? as u16),
            "SUBSYS" => subsys = Some(hex(digits, 8)?),
            "REV" => revision = Some(hex(digits, 2)? as u8),
            _ => {}
        }
    }
    Some(HardwareId { vendor: vendor?, device: device?, subsys, revision })
}

/// Parses exactly `width` hexadecimal digits.
fn hex(digits: &str, width: usize) -> Option<u32> {
    if digits.len() != width || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

/// Rewrites the registry's `M-D-YYYY` driver date as `MM/DD/YYYY`.
pub fn format_date(raw: &str) -> Option<String> {
    let mut fields = raw.trim().split(['-', '/']).map(|field| field.trim().parse::<u32>().ok());
    let (month, day, year) = (fields.next()??, fields.next()??, fields.next()??);
    let plausible = fields.next().is_none()
        && (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && (1980..=9999).contains(&year);
    plausible.then(|| format!("{month:02}/{day:02}/{year}"))
}

/// Dotted text of a four-field version number.
pub fn version_text(fields: [u16; 4]) -> String {
    let [a, b, c, d] = fields;
    format!("{a}.{b}.{c}.{d}")
}

/// Picks the driver entry that belongs to `adapter`, if any.
///
/// The hardware id has to agree with the adapter wherever it is specific. Stale entries
/// left behind by earlier installations are told apart by the version of the driver
/// DXGI actually loaded, then by the description. An entry kept from a card that was
/// since replaced can match in all of that, so among equals the one whose memory size is
/// closest to the adapter's wins, and only then the earliest.
pub fn select<'a>(drivers: &'a [Driver], adapter: &Adapter) -> Option<&'a Driver> {
    // `min_by_key` keeps the first of equal minima, which favours the earliest entry.
    drivers
        .iter()
        .filter_map(|driver| Some((score(driver, adapter)?, driver)))
        .min_by_key(|(score, driver)| (Reverse(*score), memory_gap(driver, adapter)))
        .map(|(_, driver)| driver)
}

/// How far the memory size a driver recorded is from what DXGI reports for the adapter.
/// The two never agree exactly: the registry holds the installed amount, DXGI a little
/// less. Entries without a size rank behind every entry with one.
fn memory_gap(driver: &Driver, adapter: &Adapter) -> u64 {
    driver.memory_bytes.map_or(u64::MAX, |bytes| bytes.abs_diff(adapter.dedicated_bytes))
}

/// How well a driver entry fits an adapter; `None` when it is for different hardware.
fn score(driver: &Driver, adapter: &Adapter) -> Option<u32> {
    let id = &driver.hardware_id;
    let fits = id.vendor == adapter.vendor_id
        && id.device == adapter.device_id
        && (id.subsys.is_none() || id.subsys == Some(adapter.subsys()))
        && (id.revision.is_none() || id.revision == Some(adapter.revision_id));
    if !fits {
        return None;
    }
    let loaded = adapter.driver_version.map(version_text);
    let mut score = 0;
    if loaded.is_some() && loaded == driver.version {
        score += 8;
    }
    if id.subsys.is_some() {
        score += 4;
    }
    if id.revision.is_some() {
        score += 2;
    }
    if !driver.description.is_empty() && driver.description.eq_ignore_ascii_case(&adapter.name) {
        score += 1;
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(text: &str) -> Vec<u8> {
        text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
    }

    fn name(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }

    #[test]
    fn parses_hardware_ids() {
        assert_eq!(
            parse_hardware_id(r"PCI\VEN_1002&DEV_164E&SUBSYS_7D781462&REV_C5"),
            Some(HardwareId {
                vendor: 0x1002,
                device: 0x164E,
                subsys: Some(0x7D78_1462),
                revision: Some(0xC5)
            })
        );
        assert_eq!(
            parse_hardware_id(r"pci\ven_10de&dev_2882&subsys_51611462"),
            Some(HardwareId { vendor: 0x10DE, device: 0x2882, subsys: Some(0x5161_1462), revision: None })
        );
        assert_eq!(
            parse_hardware_id(r"PCI\VEN_8086&DEV_A780&CC_0300"),
            Some(HardwareId { vendor: 0x8086, device: 0xA780, subsys: None, revision: None })
        );
    }

    #[test]
    fn rejects_other_hardware_ids() {
        assert_eq!(parse_hardware_id(r"ROOT\BasicDisplay"), None);
        assert_eq!(parse_hardware_id(r"USB\VID_17E9&PID_4301"), None);
        assert_eq!(parse_hardware_id(r"PCI\VEN_10DE"), None);
        assert_eq!(parse_hardware_id(r"PCI\VEN_10DEX&DEV_2882"), None);
        assert_eq!(parse_hardware_id(r"PCI\VEN_+0DE&DEV_2882"), None);
        assert_eq!(parse_hardware_id(r"PCI\VEN_10DE&DEV_2882&REV_A"), None);
        assert_eq!(parse_hardware_id("VEN_10DE&DEV_2882"), None);
        assert_eq!(parse_hardware_id(""), None);
    }

    #[test]
    fn formats_driver_dates() {
        assert_eq!(format_date("3-29-2023").as_deref(), Some("03/29/2023"));
        assert_eq!(format_date("10-27-2026").as_deref(), Some("10/27/2026"));
        assert_eq!(format_date(" 6/1/2021 ").as_deref(), Some("06/01/2021"));
        assert_eq!(format_date("13-1-2023"), None);
        assert_eq!(format_date("1-32-2023"), None);
        assert_eq!(format_date("1-2-23"), None);
        assert_eq!(format_date("1-2"), None);
        assert_eq!(format_date("1-2-2023-4"), None);
        assert_eq!(format_date("yesterday"), None);
        assert_eq!(format_date(""), None);
    }

    #[test]
    fn decodes_registry_values() {
        assert_eq!(decode_text(REG_SZ, &wide(" 32.0.16.2114 ")).as_deref(), Some("32.0.16.2114"));
        assert_eq!(decode_text(REG_SZ, &wide("")), None);
        assert_eq!(decode_text(REG_BINARY, &wide("text")), None);
        // A missing terminator and an odd trailing byte are tolerated.
        assert_eq!(decode_text(REG_SZ, &[0x41, 0x00, 0x42, 0x00, 0x43]).as_deref(), Some("AB"));

        assert_eq!(decode_uint(REG_QWORD, &8_585_740_288u64.to_le_bytes()), Some(8_585_740_288));
        assert_eq!(decode_uint(REG_BINARY, &536_870_912u64.to_le_bytes()), Some(536_870_912));
        assert_eq!(decode_uint(REG_DWORD, &0x8000_0000u32.to_le_bytes()), Some(0x8000_0000));
        assert_eq!(decode_uint(REG_QWORD, &[1, 2, 3]), None);
        assert_eq!(decode_uint(REG_SZ, &wide("123")), None);
    }

    #[test]
    fn recognises_instance_subkeys() {
        assert!(is_instance_name(&name("0000")));
        assert!(is_instance_name(&name("0012")));
        assert!(!is_instance_name(&name("Properties")));
        assert!(!is_instance_name(&name("00001")));
        assert!(!is_instance_name(&name("00a1")));
        assert!(!is_instance_name(&[0x30, 0x30, 0x30, 0x30]));
        assert!(!is_instance_name(&[]));
    }

    fn adapter() -> Adapter {
        Adapter {
            name: "Example Card".into(),
            vendor_id: 0x10DE,
            device_id: 0x2882,
            subsys_vendor_id: 0x1462,
            subsys_id: 0x5161,
            revision_id: 0xA1,
            driver_version: Some([32, 0, 15, 6094]),
            ..Default::default()
        }
    }

    fn driver(hardware_id: &str, description: &str, version: &str) -> Driver {
        Driver {
            description: description.into(),
            hardware_id: parse_hardware_id(hardware_id).unwrap(),
            version: Some(version.into()),
            ..Default::default()
        }
    }

    #[test]
    fn selects_the_matching_driver() {
        let drivers = [
            driver(r"PCI\VEN_1002&DEV_164E&SUBSYS_7D781462&REV_C5", "Other Card", "31.0.14046.0"),
            driver(r"PCI\VEN_10DE&DEV_2882&SUBSYS_51611043", "Example Card", "32.0.15.6094"),
            driver(r"PCI\VEN_10DE&DEV_2882&SUBSYS_51611462&REV_A2", "Example Card", "32.0.15.6094"),
            driver(r"pci\ven_10de&dev_2882&subsys_51611462", "Example Card", "32.0.15.6094"),
        ];
        assert_eq!(select(&drivers, &adapter()), Some(&drivers[3]));
        assert_eq!(select(&drivers[..3], &adapter()), None);
        assert_eq!(select(&[], &adapter()), None);
    }

    #[test]
    fn prefers_the_loaded_driver_over_stale_entries() {
        let drivers = [
            driver(r"PCI\VEN_10DE&DEV_2882&SUBSYS_51611462", "Example Card", "31.0.15.3623"),
            driver(r"PCI\VEN_10DE&DEV_2882", "Example Card", "32.0.15.6094"),
        ];
        assert_eq!(select(&drivers, &adapter()), Some(&drivers[1]));

        // Without a loaded version to compare, the more specific hardware id wins.
        let unknown = Adapter { driver_version: None, ..adapter() };
        assert_eq!(select(&drivers, &unknown), Some(&drivers[0]));
    }

    #[test]
    fn breaks_ties_by_description_then_order() {
        let drivers = [
            driver(r"PCI\VEN_10DE&DEV_2882", "Generic Adapter", "1.0.0.0"),
            driver(r"PCI\VEN_10DE&DEV_2882", "EXAMPLE CARD", "1.0.0.0"),
            driver(r"PCI\VEN_10DE&DEV_2882", "Example Card", "1.0.0.0"),
        ];
        assert_eq!(select(&drivers, &adapter()), Some(&drivers[1]));
    }

    #[test]
    fn breaks_ties_by_memory_size() {
        const MIB: u64 = 1024 * 1024;
        let entry = |memory_mb: Option<u64>| Driver {
            memory_bytes: memory_mb.map(|mb| mb * MIB),
            ..driver(r"PCI\VEN_10DE&DEV_2882&SUBSYS_51611462", "Example Card", "32.0.15.6094")
        };
        // The entry of a 4 GB card that was replaced by its 8 GB sibling comes first.
        let drivers = [entry(None), entry(Some(4096)), entry(Some(8192))];
        let with = |dedicated_mb: u64| Adapter { dedicated_bytes: dedicated_mb * MIB, ..adapter() };
        assert_eq!(select(&drivers, &with(8171)), Some(&drivers[2]));
        assert_eq!(select(&drivers, &with(4075)), Some(&drivers[1]));
        // An entry with a size is preferred to one without, however far off it is.
        assert_eq!(select(&drivers[..2], &with(8171)), Some(&drivers[1]));
        // Equal sizes leave it to the order of the keys.
        let twins = [Driver { date: Some("08/14/2024".into()), ..entry(Some(8192)) }, entry(Some(8192))];
        assert_eq!(select(&twins, &with(8171)), Some(&twins[0]));

        // The size only decides between entries that are otherwise equal.
        let outdated = Driver { version: Some("31.0.15.3623".into()), ..entry(Some(8192)) };
        let drivers = [outdated, entry(Some(4096))];
        assert_eq!(select(&drivers, &with(8171)), Some(&drivers[1]));
    }

    #[test]
    fn formats_versions() {
        assert_eq!(version_text([32, 0, 16, 2114]), "32.0.16.2114");
        assert_eq!(version_text([31, 0, 14046, 0]), "31.0.14046.0");
    }
}
