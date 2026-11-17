//! Layout of the memory device structure (type 17): where its fields sit, how the
//! size and speed fields escape to their extended forms and what the form factor and
//! memory type values are called.
//!
//! Offsets and enumeration values follow the DMTF SMBIOS reference specification (DSP0134).
//! Nothing here judges a value. "Other", "Unknown" and the non-RAM types keep their
//! names; whether they are worth showing is up to the caller.

use super::SmbiosStructure;

/// Offsets into the structure.
pub mod field {
    pub const ARRAY_HANDLE: usize = 0x04;
    pub const TOTAL_WIDTH: usize = 0x08;
    pub const DATA_WIDTH: usize = 0x0A;
    pub const SIZE: usize = 0x0C;
    pub const FORM_FACTOR: usize = 0x0E;
    pub const DEVICE_LOCATOR: usize = 0x10;
    pub const BANK_LOCATOR: usize = 0x11;
    pub const MEMORY_TYPE: usize = 0x12;
    pub const TYPE_DETAIL: usize = 0x13;
    pub const SPEED: usize = 0x15;
    pub const MANUFACTURER: usize = 0x17;
    pub const SERIAL_NUMBER: usize = 0x18;
    pub const PART_NUMBER: usize = 0x1A;
    pub const ATTRIBUTES: usize = 0x1B;
    pub const EXTENDED_SIZE: usize = 0x1C;
    pub const CONFIGURED_SPEED: usize = 0x20;
    pub const CONFIGURED_VOLTAGE: usize = 0x26;
    pub const MODULE_MANUFACTURER_ID: usize = 0x2C;
    pub const MODULE_PRODUCT_ID: usize = 0x2E;
    pub const EXTENDED_SPEED: usize = 0x54;
    pub const EXTENDED_CONFIGURED_SPEED: usize = 0x58;
}

/// Form factors, indexed from 1.
const FORM_FACTORS: &[&str] = &[
    "Other",
    "Unknown",
    "SIMM",
    "SIP",
    "Chip",
    "DIP",
    "ZIP",
    "Proprietary Card",
    "DIMM",
    "TSOP",
    "Row of chips",
    "RIMM",
    "SODIMM",
    "SRIMM",
    "FB-DIMM",
    "Die",
    "CAMM",
];

/// The Size field of a memory device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceSize {
    /// No module in the slot.
    Empty,
    /// A module is installed but the firmware does not know its size.
    Unknown,
    Megabytes(u64),
}

impl DeviceSize {
    /// Decodes the size word: megabytes, or kilobytes when bit 15 is set, with 0x7FFF
    /// deferring to the 32-bit Extended Size field. `None` when the structure is too
    /// short to hold a size at all.
    pub fn read(device: &SmbiosStructure) -> Option<Self> {
        Some(match device.word(field::SIZE)? {
            0 => Self::Empty,
            0xFFFF => Self::Unknown,
            0x7FFF => match device.dword(field::EXTENDED_SIZE).map(|mb| mb & 0x7FFF_FFFF) {
                Some(0) => Self::Unknown,
                Some(mb) => Self::Megabytes(u64::from(mb)),
                // Before the extended field existed this was just the largest plain value.
                None => Self::Megabytes(0x7FFF),
            },
            kb if kb & 0x8000 != 0 => Self::Megabytes(u64::from(kb & 0x7FFF) / 1024),
            mb => Self::Megabytes(u64::from(mb)),
        })
    }
}

/// The speed the device is rated for, in MT/s (MHz on pre-DDR parts).
pub fn rated_speed(device: &SmbiosStructure) -> Option<u32> {
    speed_field(device, field::SPEED, field::EXTENDED_SPEED)
}

/// The speed the device is configured to run at, in the unit of the rated speed.
pub fn configured_speed(device: &SmbiosStructure) -> Option<u32> {
    speed_field(device, field::CONFIGURED_SPEED, field::EXTENDED_CONFIGURED_SPEED)
}

/// Reads a speed word: 0 means unknown and 0xFFFF defers to the 32-bit extended
/// field at `extended`.
fn speed_field(device: &SmbiosStructure, offset: usize, extended: usize) -> Option<u32> {
    let speed = match device.word(offset)? {
        0xFFFF => device.dword(extended)? & 0x7FFF_FFFF,
        speed => u32::from(speed),
    };
    (speed != 0).then_some(speed)
}

/// Name of a Form Factor value. `None` for values the specification does not define.
pub fn form_factor_name(code: u8) -> Option<&'static str> {
    FORM_FACTORS.get(usize::from(code.checked_sub(1)?)).copied()
}

/// Name of a Memory Type value. `None` for values the specification does not define.
pub fn memory_type_name(code: u8) -> Option<&'static str> {
    Some(match code {
        0x01 => "Other",
        0x02 => "Unknown",
        0x03 => "DRAM",
        0x04 => "EDRAM",
        0x05 => "VRAM",
        0x06 => "SRAM",
        0x07 => "RAM",
        0x08 => "ROM",
        0x09 => "FLASH",
        0x0A => "EEPROM",
        0x0B => "FEPROM",
        0x0C => "EPROM",
        0x0D => "CDRAM",
        0x0E => "3DRAM",
        0x0F => "SDRAM",
        0x10 => "SGRAM",
        0x11 => "RDRAM",
        0x12 => "DDR",
        0x13 => "DDR2",
        0x14 => "DDR2 FB-DIMM",
        0x18 => "DDR3",
        0x19 => "FBD2",
        0x1A => "DDR4",
        0x1B => "LPDDR",
        0x1C => "LPDDR2",
        0x1D => "LPDDR3",
        0x1E => "LPDDR4",
        0x1F => "Logical non-volatile device",
        0x20 => "HBM",
        0x21 => "HBM2",
        0x22 => "DDR5",
        0x23 => "LPDDR5",
        0x24 => "HBM3",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::smbios::kind;

    /// A memory device of `length` bytes with `fields` written at their offsets.
    fn device(length: usize, fields: &[(usize, &[u8])]) -> SmbiosStructure {
        let mut data = vec![0u8; length];
        data[0] = kind::MEMORY_DEVICE;
        data[1] = length as u8;
        for (offset, bytes) in fields {
            data[*offset..*offset + bytes.len()].copy_from_slice(bytes);
        }
        SmbiosStructure { kind: kind::MEMORY_DEVICE, handle: 0, data, strings: Vec::new() }
    }

    #[test]
    fn reads_the_size_word_and_its_escapes() {
        let size = |word: u16, extended: u32| {
            let fields: &[(usize, &[u8])] =
                &[(field::SIZE, &word.to_le_bytes()), (field::EXTENDED_SIZE, &extended.to_le_bytes())];
            DeviceSize::read(&device(0x5C, fields))
        };
        assert_eq!(size(0, 0), Some(DeviceSize::Empty));
        assert_eq!(size(0xFFFF, 0), Some(DeviceSize::Unknown));
        assert_eq!(size(8192, 0), Some(DeviceSize::Megabytes(8192)));
        // Bit 15 selects kilobytes.
        assert_eq!(size(0x8000 | 16_384, 0), Some(DeviceSize::Megabytes(16)));
        // 0x7FFF defers to the extended size, whose top bit is reserved.
        assert_eq!(size(0x7FFF, 0x8000_0000 | 65_536), Some(DeviceSize::Megabytes(65_536)));
        assert_eq!(size(0x7FFF, 0), Some(DeviceSize::Unknown));

        // A structure that predates the extended field means the value literally.
        let legacy = device(0x1C, &[(field::SIZE, &0x7FFFu16.to_le_bytes())]);
        assert_eq!(DeviceSize::read(&legacy), Some(DeviceSize::Megabytes(0x7FFF)));
        assert_eq!(DeviceSize::read(&device(0x0C, &[])), None);
    }

    #[test]
    fn reads_speeds_and_their_escapes() {
        let speeds = |rated: u16, rated_ext: u32, configured: u16, configured_ext: u32| {
            let fields: &[(usize, &[u8])] = &[
                (field::SPEED, &rated.to_le_bytes()),
                (field::EXTENDED_SPEED, &rated_ext.to_le_bytes()),
                (field::CONFIGURED_SPEED, &configured.to_le_bytes()),
                (field::EXTENDED_CONFIGURED_SPEED, &configured_ext.to_le_bytes()),
            ];
            let device = device(0x5C, fields);
            (rated_speed(&device), configured_speed(&device))
        };
        assert_eq!(speeds(5600, 0, 5200, 0), (Some(5600), Some(5200)));
        assert_eq!(speeds(3200, 0, 0, 0), (Some(3200), None));
        assert_eq!(speeds(0xFFFF, 70_000, 0xFFFF, 0x8000_0000 | 66_000), (Some(70_000), Some(66_000)));
        assert_eq!(speeds(0xFFFF, 0, 0xFFFF, 0), (None, None));

        // The escape value on a structure too short for the extended fields.
        let fields: &[(usize, &[u8])] =
            &[(field::SPEED, &[0xFF, 0xFF]), (field::CONFIGURED_SPEED, &[0xFF, 0xFF])];
        let short = device(0x28, fields);
        assert_eq!((rated_speed(&short), configured_speed(&short)), (None, None));
        // Before SMBIOS 2.7 the structure ends ahead of the configured speed.
        let old = device(0x1B, &[(field::SPEED, &400u16.to_le_bytes())]);
        assert_eq!((rated_speed(&old), configured_speed(&old)), (Some(400), None));
    }

    #[test]
    fn names_form_factors_and_memory_types() {
        assert_eq!(form_factor_name(0x01), Some("Other"));
        assert_eq!(form_factor_name(0x02), Some("Unknown"));
        assert_eq!(form_factor_name(0x09), Some("DIMM"));
        assert_eq!(form_factor_name(0x0D), Some("SODIMM"));
        assert_eq!(form_factor_name(0x11), Some("CAMM"));
        assert_eq!(form_factor_name(0x00), None);
        assert_eq!(form_factor_name(0x12), None);

        assert_eq!(memory_type_name(0x02), Some("Unknown"));
        assert_eq!(memory_type_name(0x09), Some("FLASH"));
        assert_eq!(memory_type_name(0x0F), Some("SDRAM"));
        assert_eq!(memory_type_name(0x1A), Some("DDR4"));
        assert_eq!(memory_type_name(0x22), Some("DDR5"));
        assert_eq!(memory_type_name(0x00), None);
        assert_eq!(memory_type_name(0x15), None);
        assert_eq!(memory_type_name(0x25), None);
    }
}
