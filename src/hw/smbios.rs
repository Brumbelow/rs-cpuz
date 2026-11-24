//! Raw SMBIOS (DMI) table access.
//!
//! Windows hands the firmware's SMBIOS table to user mode through
//! `GetSystemFirmwareTable('RSMB')`. This module only splits that blob into
//! structures and gives typed access to their fields; interpreting them is up
//! to the callers (`board`, `memory`, the report writers). The decoding rules
//! more than one of them needs are kept here so that they cannot drift apart:
//! the memory device layout and the list of firmware filler strings.

pub mod memory_device;

use windows::Win32::System::SystemInformation::{GetSystemFirmwareTable, RSMB};

/// SMBIOS structure types used by this crate.
pub mod kind {
    pub const BIOS: u8 = 0;
    pub const SYSTEM: u8 = 1;
    pub const BASEBOARD: u8 = 2;
    pub const CHASSIS: u8 = 3;
    pub const PROCESSOR: u8 = 4;
    pub const PORT_CONNECTOR: u8 = 8;
    pub const SYSTEM_SLOT: u8 = 9;
    pub const OEM_STRINGS: u8 = 11;
    pub const MEMORY_ARRAY: u8 = 16;
    pub const MEMORY_DEVICE: u8 = 17;
    pub const MEMORY_DEVICE_MAPPED_ADDRESS: u8 = 20;
    pub const ADDITIONAL_INFORMATION: u8 = 40;
    pub const END_OF_TABLE: u8 = 127;
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SmbiosStructure {
    pub kind: u8,
    pub handle: u16,
    /// The formatted area, including the 4-byte header.
    pub data: Vec<u8>,
    /// The string set that follows the formatted area (index 1 is `strings[0]`).
    pub strings: Vec<String>,
}

impl SmbiosStructure {
    pub fn byte(&self, offset: usize) -> Option<u8> {
        self.data.get(offset).copied()
    }

    pub fn word(&self, offset: usize) -> Option<u16> {
        let b = self.data.get(offset..offset + 2)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn dword(&self, offset: usize) -> Option<u32> {
        let b = self.data.get(offset..offset + 4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn qword(&self, offset: usize) -> Option<u64> {
        let b = self.data.get(offset..offset + 8)?;
        Some(u64::from_le_bytes(b.try_into().ok()?))
    }

    /// The string referenced by the string-index byte at `offset`, trimmed.
    /// Returns `None` for index 0, a missing string or an empty one.
    pub fn string(&self, offset: usize) -> Option<&str> {
        let index = usize::from(self.byte(offset)?);
        let s = self.strings.get(index.checked_sub(1)?)?.trim();
        (!s.is_empty()).then_some(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Smbios {
    pub major: u8,
    pub minor: u8,
    pub structures: Vec<SmbiosStructure>,
}

impl Smbios {
    pub fn of_kind(&self, kind: u8) -> impl Iterator<Item = &SmbiosStructure> {
        self.structures.iter().filter(move |s| s.kind == kind)
    }

    pub fn first(&self, kind: u8) -> Option<&SmbiosStructure> {
        self.of_kind(kind).next()
    }
}

/// Strings firmware vendors leave in fields nobody filled in, in lower case.
const PLACEHOLDERS: &[&str] = &[
    "to be filled by o.e.m.",
    "to be filled by o.e.m",
    "default string",
    "system product name",
    "system manufacturer",
    "system version",
    "not applicable",
    "not specified",
    "not available",
    "not installed",
    "undefined",
    "unknown",
    "invalid",
    "none",
    "null",
    "n/a",
    "empty",
    "[empty]",
    "no dimm",
    "nodimm",
    "oem",
    "o.e.m.",
];

/// Whether a firmware string is one of the stock fillers rather than a value.
/// Case and surrounding whitespace are ignored.
pub fn is_placeholder(text: &str) -> bool {
    PLACEHOLDERS.contains(&text.trim().to_ascii_lowercase().as_str())
}

/// Reads and parses the firmware SMBIOS table. `None` when the firmware exposes none.
pub fn read() -> Option<Smbios> {
    // SAFETY: the first call only queries the required size; the second writes at most
    // `buf.len()` bytes into a buffer we own.
    let raw = unsafe {
        let size = GetSystemFirmwareTable(RSMB, 0, None) as usize;
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size];
        let written = GetSystemFirmwareTable(RSMB, 0, Some(&mut buf)) as usize;
        if written == 0 || written > buf.len() {
            return None;
        }
        buf.truncate(written);
        buf
    };
    parse_raw(&raw)
}

/// Parses a `RawSMBIOSData` blob: an 8-byte header (calling method, major, minor,
/// DMI revision, table length) followed by the structure table.
pub fn parse_raw(raw: &[u8]) -> Option<Smbios> {
    let header = raw.get(..8)?;
    let length = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    let table = raw.get(8..)?;
    let table = table.get(..length).unwrap_or(table);
    Some(Smbios { major: header[1], minor: header[2], structures: parse_table(table) })
}

/// Splits a structure table into its structures. Stops at the end-of-table
/// marker or at the first malformed structure.
pub fn parse_table(table: &[u8]) -> Vec<SmbiosStructure> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 4 <= table.len() {
        let kind = table[pos];
        let length = usize::from(table[pos + 1]);
        if length < 4 || pos + length > table.len() {
            break;
        }
        let handle = u16::from_le_bytes([table[pos + 2], table[pos + 3]]);
        let data = table[pos..pos + length].to_vec();

        // The string set is a run of NUL-terminated strings closed by an extra NUL.
        let mut strings = Vec::new();
        let mut cursor = pos + length;
        loop {
            let rest = &table[cursor.min(table.len())..];
            let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
            if end == 0 {
                break;
            }
            strings.push(String::from_utf8_lossy(&rest[..end]).into_owned());
            cursor += end + 1;
        }
        // An empty string set is two NULs; a non-empty one ends with a single extra NUL.
        cursor += if strings.is_empty() { 2 } else { 1 };

        out.push(SmbiosStructure { kind, handle, data, strings });
        if kind == kind::END_OF_TABLE {
            break;
        }
        pos = cursor;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        let mut t = Vec::new();
        // Type 2 (baseboard), length 8, handle 0x0002, strings: "ACME", "Board-1 ".
        t.extend_from_slice(&[2, 8, 0x02, 0x00, 1, 2, 0, 0]);
        t.extend_from_slice(b"ACME\0Board-1 \0\0");
        // Type 17 (memory device) with no strings, length 6.
        t.extend_from_slice(&[17, 6, 0x10, 0x00, 0x34, 0x12]);
        t.extend_from_slice(&[0, 0]);
        // End of table.
        t.extend_from_slice(&[127, 4, 0xFF, 0x00, 0, 0]);
        // Trailing garbage that must be ignored.
        t.extend_from_slice(&[9, 9, 9]);
        t
    }

    #[test]
    fn splits_structures_and_strings() {
        let s = parse_table(&fixture());
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].kind, kind::BASEBOARD);
        assert_eq!(s[0].handle, 2);
        assert_eq!(s[0].string(4), Some("ACME"));
        assert_eq!(s[0].string(5), Some("Board-1"));
        assert_eq!(s[0].string(6), None);
        assert_eq!(s[1].kind, kind::MEMORY_DEVICE);
        assert!(s[1].strings.is_empty());
        assert_eq!(s[1].word(4), Some(0x1234));
        assert_eq!(s[1].dword(4), None);
        assert_eq!(s[2].kind, kind::END_OF_TABLE);
    }

    #[test]
    fn parses_raw_header() {
        let table = fixture();
        let mut raw = vec![0, 3, 5, 0];
        raw.extend_from_slice(&(table.len() as u32).to_le_bytes());
        raw.extend_from_slice(&table);
        let smbios = parse_raw(&raw).unwrap();
        assert_eq!((smbios.major, smbios.minor), (3, 5));
        assert_eq!(smbios.of_kind(kind::MEMORY_DEVICE).count(), 1);
        assert!(smbios.first(kind::BIOS).is_none());
    }

    #[test]
    fn recognises_firmware_fillers() {
        for filler in [
            "To Be Filled By O.E.M.",
            "Default string",
            "System Product Name",
            " Not Specified ",
            "Not Installed",
            "Undefined",
            "UNKNOWN",
            "NULL",
            "N/A",
            "[Empty]",
            "NO DIMM",
            "OEM",
        ] {
            assert!(is_placeholder(filler), "{filler}");
        }
        for value in ["Example Memory", "PRIME B550M-A", "1.0", "Nonesuch", "OEM-1234", ""] {
            assert!(!is_placeholder(value), "{value}");
        }
    }

    #[test]
    fn tolerates_truncated_tables() {
        let table = fixture();
        assert_eq!(parse_table(&table[..5]).len(), 0);
        assert_eq!(parse_table(&table[..20]).len(), 1);
        assert!(parse_raw(&[0, 1, 2]).is_none());
    }
}
