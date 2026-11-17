//! Decoding of the SMBIOS memory structures: the physical memory array (type 16)
//! and its memory devices (type 17).
//!
//! Offsets and enumerations follow the DMTF SMBIOS reference specification (DSP0134).
//! The structures grew with almost every revision, so each field is read through the
//! bounds-checked accessors and simply stays unknown on firmware that predates it.

use super::{jedec, speed};
use crate::hw::smbios::memory_device::{self, field, DeviceSize};
use crate::hw::smbios::{self, kind, Smbios, SmbiosStructure};
use crate::model::MemoryModule;

/// Offsets into the physical memory array structure (type 16).
mod array_field {
    pub const USE: usize = 0x05;
    pub const ERROR_CORRECTION: usize = 0x06;
}

/// Memory array use: main system memory (as opposed to video, flash, cache ...).
const USE_SYSTEM_MEMORY: u8 = 0x03;

/// Stems of the factory-default strings left in unfilled fields, which come with a
/// prefix and a counter: `Manufacturer00`, `A1_SerNum0`, `Array1_PartNumber0`.
const DEFAULT_STEMS: &[&str] = &["manufacturer", "sernum", "partnum", "partnumber"];

/// The memory technologies the summary treats specially, plus a name for the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryType {
    Ddr,
    Ddr2,
    Ddr2FbDimm,
    Ddr3,
    Ddr4,
    Ddr5,
    Lpddr,
    Lpddr2,
    Lpddr3,
    Lpddr4,
    Lpddr5,
    /// Any other RAM technology, by its SMBIOS name.
    Other(&'static str),
}

impl MemoryType {
    /// Maps the Memory Type field. `None` for "other", "unknown" and non-RAM devices.
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0x12 => Self::Ddr,
            0x13 => Self::Ddr2,
            0x14 => Self::Ddr2FbDimm,
            0x18 => Self::Ddr3,
            0x1A => Self::Ddr4,
            0x1B => Self::Lpddr,
            0x1C => Self::Lpddr2,
            0x1D => Self::Lpddr3,
            0x1E => Self::Lpddr4,
            0x22 => Self::Ddr5,
            0x23 => Self::Lpddr5,
            0x01 | 0x02 => return None,
            code if is_firmware_storage(code) => return None,
            code => Self::Other(memory_device::memory_type_name(code)?),
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Ddr => "DDR",
            Self::Ddr2 => "DDR2",
            Self::Ddr2FbDimm => "DDR2 FB-DIMM",
            Self::Ddr3 => "DDR3",
            Self::Ddr4 => "DDR4",
            Self::Ddr5 => "DDR5",
            Self::Lpddr => "LPDDR",
            Self::Lpddr2 => "LPDDR2",
            Self::Lpddr3 => "LPDDR3",
            Self::Lpddr4 => "LPDDR4",
            Self::Lpddr5 => "LPDDR5",
            Self::Other(name) => name,
        }
    }

    /// Whether the speed fields are a double-data-rate figure: two transfers per clock.
    pub fn is_double_data_rate(self) -> bool {
        !matches!(self, Self::Other(_))
    }
}

/// ROM, flash and logical non-volatile devices: listed as memory devices by some
/// firmware, but never a system-memory slot.
fn is_firmware_storage(type_code: u8) -> bool {
    matches!(type_code, 0x08..=0x0C | 0x1F)
}

/// Size in MB of the installed module (0 when unknown); `None` for an empty slot.
fn installed_mb(size: DeviceSize) -> Option<u64> {
    match size {
        DeviceSize::Empty => None,
        DeviceSize::Unknown => Some(0),
        DeviceSize::Megabytes(mb) => Some(mb),
    }
}

/// The registered / unbuffered / load-reduced bits of the Type Detail field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TypeDetail(u16);

impl TypeDetail {
    const REGISTERED: u16 = 1 << 13;
    const UNBUFFERED: u16 = 1 << 14;
    const LOAD_REDUCED: u16 = 1 << 15;

    fn has(self, bit: u16) -> bool {
        self.0 & bit != 0
    }

    /// Whether the module registers its address and command lines. `None` when the
    /// firmware sets neither bit, or both.
    fn registered(self) -> Option<bool> {
        match (self.has(Self::REGISTERED), self.has(Self::UNBUFFERED)) {
            _ if self.has(Self::LOAD_REDUCED) => Some(true),
            (true, false) => Some(true),
            (false, true) => Some(false),
            _ => None,
        }
    }

    /// Whether the module buffers its data lines. SMBIOS has one bit for "registered
    /// (buffered)", so a registered module that is not load-reduced stays undecided.
    fn buffered(self) -> Option<bool> {
        if self.has(Self::LOAD_REDUCED) {
            Some(true)
        } else {
            self.registered().filter(|registered| !registered)
        }
    }

    /// Narrows a plain `DIMM` to the variant the detail bits name.
    fn dimm_variant(self) -> &'static str {
        match self.registered() {
            _ if self.has(Self::LOAD_REDUCED) => "LRDIMM",
            Some(true) => "RDIMM",
            Some(false) => "UDIMM",
            None => "DIMM",
        }
    }
}

/// Names the Form Factor field, narrowing a plain DIMM by its detail bits.
/// `None` for "other" and "unknown".
fn form_factor(code: u8, detail: TypeDetail) -> Option<&'static str> {
    match code {
        0x01 | 0x02 => None,
        0x09 => Some(detail.dimm_variant()),
        code => memory_device::form_factor_name(code),
    }
}

/// An identifier that was never programmed: all zeros or all `F`s, with or without `0x`.
fn is_blank_id(lower: &str) -> bool {
    let digits = lower.strip_prefix("0x").unwrap_or(lower);
    let all = |digit: char| !digits.is_empty() && digits.chars().all(|c| c == digit);
    all('0') || (digits.len() >= 4 && all('f'))
}

/// Whether a firmware string is filler rather than a value: a stock placeholder,
/// a factory default or an identifier that was never programmed.
fn is_placeholder(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    let stem = lower.trim_end_matches(|c: char| c.is_ascii_digit());
    smbios::is_placeholder(&lower)
        || DEFAULT_STEMS.iter().any(|default| stem.ends_with(default))
        || is_blank_id(&lower)
}

/// The string at `offset`, unless the firmware left filler there.
fn text(device: &SmbiosStructure, offset: usize) -> Option<&str> {
    device.string(offset).filter(|text| !is_placeholder(text))
}

/// Resolves the module manufacturer. A name from the firmware is kept as it is; a
/// JEDEC id printed as hex is translated, falling back to the SPD-derived id field
/// and finally to the hex text itself.
fn manufacturer(text: Option<&str>, id: Option<u16>) -> Option<String> {
    let from_id_field = || id.and_then(|id| jedec::vendor(&id.to_le_bytes()));
    let name = match text.map(|text| (text, jedec::hex_id(text))) {
        Some((name, None)) => Some(name),
        Some((hex, Some(bytes))) => jedec::vendor(&bytes).or_else(from_id_field).or(Some(hex)),
        None => from_id_field(),
    };
    name.map(str::to_string)
}

/// Whether the device declares a total width equal to its data width, i.e. no bits
/// beyond the data bits that could hold a check code.
fn lacks_check_bits(device: &SmbiosStructure) -> bool {
    let width = |offset| device.word(offset).filter(|&bits| bits != 0 && bits != 0xFFFF);
    width(field::TOTAL_WIDTH).zip(width(field::DATA_WIDTH)).is_some_and(|(total, data)| total == data)
}

/// Error correction of a module: the method of the array it belongs to, or `None`
/// (the text) when the module has no check bits for that method to use.
fn correction(array: Option<&SmbiosStructure>, device: &SmbiosStructure) -> Option<&'static str> {
    let method = match array?.byte(array_field::ERROR_CORRECTION)? {
        0x03 => return Some("None"),
        0x04 => "Parity",
        0x05 | 0x06 => "ECC",
        0x07 => "CRC",
        _ => return None,
    };
    Some(if lacks_check_bits(device) { "None" } else { method })
}

/// One system-memory slot: its entry for the model plus what the summary needs.
#[derive(Debug, Clone)]
pub struct Slot {
    pub module: MemoryModule,
    /// Firmware bank label, e.g. `P0 CHANNEL A`; empty when there is none.
    pub bank_locator: String,
    /// Technology of the installed module.
    pub kind: Option<MemoryType>,
    /// Speed the installed module is rated for, in MT/s (MHz on pre-DDR parts).
    pub rated_speed: Option<u32>,
}

/// Decodes every memory device that is a system-memory slot, in firmware order.
pub fn slots(smbios: &Smbios) -> Vec<Slot> {
    let mut slots: Vec<Slot> = smbios
        .of_kind(kind::MEMORY_DEVICE)
        .filter_map(|device| decode(device, owning_array(smbios, device)))
        .collect();
    for (index, slot) in slots.iter_mut().enumerate() {
        slot.module.slot = format!("Slot #{}", index + 1);
    }
    slots
}

/// The physical memory array a device belongs to.
fn owning_array<'a>(smbios: &'a Smbios, device: &SmbiosStructure) -> Option<&'a SmbiosStructure> {
    let handle = device.word(field::ARRAY_HANDLE)?;
    smbios.of_kind(kind::MEMORY_ARRAY).find(|array| array.handle == handle)
}

/// Decodes one memory device. `None` when it is not a system-memory slot: it belongs
/// to a video / flash / cache array, is a ROM or flash chip, or is malformed.
fn decode(device: &SmbiosStructure, array: Option<&SmbiosStructure>) -> Option<Slot> {
    let foreign_array =
        array.and_then(|array| array.byte(array_field::USE)).is_some_and(|usage| usage != USE_SYSTEM_MEMORY);
    let type_code = device.byte(field::MEMORY_TYPE).unwrap_or(0);
    if foreign_array || is_firmware_storage(type_code) {
        return None;
    }
    let size = installed_mb(DeviceSize::read(device)?);
    let kind = size.and_then(|_| MemoryType::from_code(type_code));
    let module = size.map(|size_mb| installed(device, array, kind, size_mb)).unwrap_or_default();
    let label = |offset| device.string(offset).unwrap_or_default().to_string();
    Some(Slot {
        module: MemoryModule { locator: label(field::DEVICE_LOCATOR), ..module },
        bank_locator: label(field::BANK_LOCATOR),
        kind,
        rated_speed: size.and_then(|_| memory_device::rated_speed(device)),
    })
}

/// Builds the entry of a populated slot, labels aside. The SPD-only fields (DRAM
/// manufacturer, manufacturing date, SPD extensions, timing profiles) keep their defaults.
fn installed(
    device: &SmbiosStructure,
    array: Option<&SmbiosStructure>,
    kind: Option<MemoryType>,
    size_mb: u64,
) -> MemoryModule {
    let detail = TypeDetail(device.word(field::TYPE_DETAIL).unwrap_or(0));
    let rated = memory_device::rated_speed(device);
    let owned = |text: Option<&str>| text.map(str::to_string);
    MemoryModule {
        populated: true,
        kind: owned(kind.map(MemoryType::name)),
        form_factor: owned(form_factor(device.byte(field::FORM_FACTOR).unwrap_or(0), detail)),
        size_mb,
        max_bandwidth: kind.zip(rated).and_then(|(kind, mts)| speed::bandwidth(kind, mts)),
        manufacturer: manufacturer(
            text(device, field::MANUFACTURER),
            device.word(field::MODULE_MANUFACTURER_ID),
        ),
        part_number: owned(text(device, field::PART_NUMBER)),
        serial_number: owned(text(device, field::SERIAL_NUMBER)),
        correction: owned(correction(array, device)),
        registered: detail.registered(),
        buffered: detail.buffered(),
        ranks: device.byte(field::ATTRIBUTES).map(|bits| u32::from(bits & 0x0F)).filter(|&ranks| ranks != 0),
        configured_speed: memory_device::configured_speed(device),
        voltage: device
            .word(field::CONFIGURED_VOLTAGE)
            .filter(|&millivolts| millivolts != 0)
            .map(|millivolts| f32::from(millivolts) / 1000.0),
        ..MemoryModule::default()
    }
}

#[cfg(test)]
pub(super) mod fixtures {
    //! Hand-built SMBIOS tables. Structures are serialised to raw bytes and go back
    //! through the real table parser, so the decoders see what firmware would give them.

    use crate::hw::smbios::{parse_table, Smbios};

    /// A memory device (type 17) under construction.
    pub struct Device {
        body: Vec<u8>,
        strings: Vec<String>,
    }

    impl Device {
        /// A zeroed structure of `length` bytes (0x15 = SMBIOS 2.1 ... 0x5C = 3.3) that
        /// belongs to array `0x1000`.
        pub fn new(length: u8) -> Self {
            let mut body = vec![0u8; usize::from(length)];
            body[0] = 17;
            body[1] = length;
            Device { body, strings: Vec::new() }.word(0x04, 0x1000)
        }

        pub fn byte(mut self, offset: usize, value: u8) -> Self {
            self.body[offset] = value;
            self
        }

        pub fn word(mut self, offset: usize, value: u16) -> Self {
            self.body[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
            self
        }

        pub fn dword(mut self, offset: usize, value: u32) -> Self {
            self.body[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            self
        }

        /// Appends `text` to the string set and points the index byte at `offset` to it.
        /// An empty `text` clears the index instead: a string set cannot hold one.
        pub fn string(mut self, offset: usize, text: &str) -> Self {
            if text.is_empty() {
                self.body[offset] = 0;
            } else {
                self.strings.push(text.to_string());
                self.body[offset] = self.strings.len() as u8;
            }
            self
        }

        /// An empty slot the way current firmware reports it.
        pub fn empty(locator: &str, bank: &str) -> Self {
            Device::new(0x5C)
                .word(0x08, 0xFFFF)
                .word(0x0A, 0xFFFF)
                .byte(0x0E, 0x02)
                .string(0x10, locator)
                .string(0x11, bank)
                .byte(0x12, 0x02)
                .word(0x13, 0x0004)
                .string(0x17, "Unknown")
                .string(0x18, "Unknown")
                .string(0x1A, "Unknown")
        }

        /// A 16 GB unbuffered DDR5-5600 DIMM running at 5200 MT/s and 1.1 V.
        pub fn ddr5(locator: &str, bank: &str) -> Self {
            Device::new(0x5C)
                .word(0x08, 64)
                .word(0x0A, 64)
                .word(0x0C, 16 * 1024)
                .byte(0x0E, 0x09)
                .string(0x10, locator)
                .string(0x11, bank)
                .byte(0x12, 0x22)
                .word(0x13, 0x4080)
                .word(0x15, 5600)
                .string(0x17, "Example Memory")
                .string(0x18, "1A2B3C4D")
                .string(0x1A, "EX16G56C46U5   ")
                .byte(0x1B, 0x01)
                .word(0x20, 5200)
                .word(0x26, 1100)
        }

        fn serialise(&self, handle: u16, table: &mut Vec<u8>) {
            let mut body = self.body.clone();
            body[2..4].copy_from_slice(&handle.to_le_bytes());
            push_structure(table, &body, &self.strings);
        }
    }

    fn push_structure(table: &mut Vec<u8>, body: &[u8], strings: &[String]) {
        table.extend_from_slice(body);
        for text in strings {
            table.extend_from_slice(text.as_bytes());
            table.push(0);
        }
        table.extend_from_slice(if strings.is_empty() { &[0, 0] } else { &[0] });
    }

    /// A physical memory array (type 16) with handle `handle`.
    pub fn array(handle: u16, usage: u8, error_correction: u8) -> Vec<u8> {
        let mut body = vec![0u8; 0x17];
        body[0] = 16;
        body[1] = 0x17;
        body[2..4].copy_from_slice(&handle.to_le_bytes());
        body[4] = 0x03;
        body[5] = usage;
        body[6] = error_correction;
        let mut table = Vec::new();
        push_structure(&mut table, &body, &[]);
        table
    }

    /// The structure table holding `arrays` followed by `devices` (handles 0x1100,
    /// 0x1101 ...) and the end-of-table marker.
    pub fn table(arrays: &[Vec<u8>], devices: &[Device]) -> Vec<u8> {
        let mut table = arrays.concat();
        for (index, device) in devices.iter().enumerate() {
            device.serialise(0x1100 + index as u16, &mut table);
        }
        table.extend_from_slice(&[127, 4, 0xFF, 0xFE, 0, 0]);
        table
    }

    /// Runs a structure table through the SMBIOS parser.
    pub fn parse(table: &[u8]) -> Smbios {
        Smbios { major: 3, minor: 5, structures: parse_table(table) }
    }

    /// Parses the table holding `arrays` and `devices`.
    pub fn smbios(arrays: &[Vec<u8>], devices: &[Device]) -> Smbios {
        parse(&table(arrays, devices))
    }

    /// A table with one system-memory array without error correction (handle 0x1000).
    pub fn system(devices: &[Device]) -> Smbios {
        smbios(&[array(0x1000, 0x03, 0x03)], devices)
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{array, smbios, system, Device};
    use super::*;

    /// Decodes a single device in a plain system-memory array.
    fn one(device: Device) -> Slot {
        let mut slots = slots(&system(&[device]));
        assert_eq!(slots.len(), 1);
        slots.remove(0)
    }

    #[test]
    fn decodes_a_current_ddr5_module() {
        let slot = one(Device::ddr5("DIMMA2", "P0 CHANNEL A"));
        let m = &slot.module;
        assert_eq!(slot.kind, Some(MemoryType::Ddr5));
        assert_eq!(slot.bank_locator, "P0 CHANNEL A");
        assert_eq!(slot.rated_speed, Some(5600));
        assert_eq!((m.slot.as_str(), m.locator.as_str(), m.populated), ("Slot #1", "DIMMA2", true));
        assert_eq!(m.kind.as_deref(), Some("DDR5"));
        assert_eq!(m.form_factor.as_deref(), Some("UDIMM"));
        assert_eq!(m.size_mb, 16 * 1024);
        assert_eq!(m.max_bandwidth.as_deref(), Some("DDR5-5600 (2800 MHz)"));
        assert_eq!(m.manufacturer.as_deref(), Some("Example Memory"));
        assert_eq!(m.part_number.as_deref(), Some("EX16G56C46U5"));
        assert_eq!(m.serial_number.as_deref(), Some("1A2B3C4D"));
        assert_eq!(m.correction.as_deref(), Some("None"));
        assert_eq!((m.registered, m.buffered), (Some(false), Some(false)));
        assert_eq!(m.ranks, Some(1));
        assert_eq!(m.configured_speed, Some(5200));
        assert_eq!(m.voltage, Some(1.1));
    }

    #[test]
    fn leaves_spd_only_fields_unknown() {
        let m = one(Device::ddr5("DIMMA2", "P0 CHANNEL A")).module;
        assert!(m.dram_manufacturer.is_none() && m.week_year.is_none() && m.spd_ext.is_none());
        assert!(m.profiles.is_empty());
    }

    #[test]
    fn empty_slots_keep_only_their_labels() {
        let slots = slots(&system(&[
            Device::empty("DIMMA1", "P0 CHANNEL A"),
            Device::ddr5("DIMMA2", "P0 CHANNEL A"),
            Device::empty("DIMMB1", "P0 CHANNEL B"),
        ]));
        let labels: Vec<_> =
            slots.iter().map(|s| (s.module.slot.as_str(), s.module.locator.as_str())).collect();
        assert_eq!(labels, [("Slot #1", "DIMMA1"), ("Slot #2", "DIMMA2"), ("Slot #3", "DIMMB1")]);
        let empty = &slots[2];
        assert_eq!(empty.bank_locator, "P0 CHANNEL B");
        assert!(!empty.module.populated && empty.kind.is_none() && empty.rated_speed.is_none());
        assert!(empty.module.kind.is_none() && empty.module.form_factor.is_none());
        assert!(empty.module.manufacturer.is_none() && empty.module.serial_number.is_none());
        assert_eq!(empty.module.size_mb, 0);
        assert!(empty.module.correction.is_none() && empty.module.registered.is_none());
    }

    #[test]
    fn short_legacy_structure_decodes_what_it_has() {
        // SMBIOS 2.1: the structure ends after the type detail word.
        let legacy = Device::new(0x15)
            .word(0x0C, 256)
            .byte(0x0E, 0x09)
            .string(0x10, "DIMM 1")
            .string(0x11, "BANK 0")
            .byte(0x12, 0x0F)
            .word(0x13, 0x0080);
        let slot = one(legacy);
        let m = &slot.module;
        assert_eq!(slot.kind, Some(MemoryType::Other("SDRAM")));
        assert_eq!(
            (m.kind.as_deref(), m.form_factor.as_deref(), m.size_mb),
            (Some("SDRAM"), Some("DIMM"), 256)
        );
        assert!(m.populated && m.max_bandwidth.is_none() && m.manufacturer.is_none());
        assert!(m.serial_number.is_none() && m.part_number.is_none() && m.ranks.is_none());
        assert!(m.configured_speed.is_none() && m.voltage.is_none());
        assert_eq!((m.registered, m.buffered), (None, None));
    }

    #[test]
    fn structure_without_a_size_field_is_not_a_slot() {
        assert!(slots(&system(&[Device::new(0x0C)])).is_empty());
    }

    #[test]
    fn version_2_3_structure_has_speed_but_no_configured_speed() {
        let ddr = Device::new(0x1B)
            .word(0x0C, 512)
            .byte(0x0E, 0x09)
            .string(0x10, "A0")
            .byte(0x12, 0x12)
            .word(0x15, 400)
            .string(0x17, "7F98000000000000")
            .string(0x1A, "EX512D400");
        let m = one(ddr).module;
        assert_eq!(m.max_bandwidth.as_deref(), Some("PC3200 (200 MHz)"));
        assert_eq!(m.manufacturer.as_deref(), Some("Kingston"));
        assert_eq!(m.part_number.as_deref(), Some("EX512D400"));
        assert!(m.configured_speed.is_none() && m.ranks.is_none() && m.voltage.is_none());
    }

    #[test]
    fn size_word_units_and_escapes() {
        let size = |device: Device| one(device.string(0x10, "X")).module;
        assert_eq!(size(Device::new(0x5C).word(0x0C, 8192)).size_mb, 8192);
        // Bit 15 selects kilobytes.
        assert_eq!(size(Device::new(0x5C).word(0x0C, 0x8000 | 16_384)).size_mb, 16);
        // 0x7FFF defers to the extended size, whose top bit is reserved.
        let extended = Device::new(0x5C).word(0x0C, 0x7FFF).dword(0x1C, 0x8000_0000 | 65_536);
        assert_eq!(size(extended).size_mb, 65_536);
        // A structure that predates the extended field means the value literally.
        assert_eq!(size(Device::new(0x1C).word(0x0C, 0x7FFF)).size_mb, 0x7FFF);
        // Installed, size unknown.
        let unknown = size(Device::new(0x5C).word(0x0C, 0xFFFF));
        assert!(unknown.populated);
        assert_eq!(unknown.size_mb, 0);
        assert_eq!(size(Device::new(0x5C).word(0x0C, 0x7FFF)).size_mb, 0);
    }

    #[test]
    fn speed_escapes_to_the_extended_fields() {
        let fast = Device::ddr5("DIMM 0", "")
            .word(0x15, 0xFFFF)
            .dword(0x54, 70_000)
            .word(0x20, 0xFFFF)
            .dword(0x58, 0x8000_0000 | 66_000);
        let m = one(fast).module;
        assert_eq!(m.max_bandwidth.as_deref(), Some("DDR5-70000 (35000 MHz)"));
        assert_eq!(m.configured_speed, Some(66_000));

        let unknown = one(Device::ddr5("DIMM 0", "").word(0x15, 0).word(0x20, 0)).module;
        assert!(unknown.max_bandwidth.is_none() && unknown.configured_speed.is_none());

        // The escape value on a structure too short for the extended field.
        let short = Device::new(0x28).word(0x0C, 4096).byte(0x12, 0x1A).word(0x15, 0xFFFF).word(0x20, 0xFFFF);
        let m = one(short).module;
        assert!(m.max_bandwidth.is_none() && m.configured_speed.is_none());
    }

    #[test]
    fn form_factor_names() {
        let plain = TypeDetail(0x0080);
        assert_eq!(form_factor(0x09, plain), Some("DIMM"));
        assert_eq!(form_factor(0x09, TypeDetail(0x4080)), Some("UDIMM"));
        assert_eq!(form_factor(0x09, TypeDetail(0x2080)), Some("RDIMM"));
        assert_eq!(form_factor(0x09, TypeDetail(0xA080)), Some("LRDIMM"));
        assert_eq!(form_factor(0x0D, TypeDetail(0x4080)), Some("SODIMM"));
        assert_eq!(form_factor(0x0B, plain), Some("Row of chips"));
        assert_eq!(form_factor(0x0F, plain), Some("FB-DIMM"));
        assert_eq!(form_factor(0x11, plain), Some("CAMM"));
        for unknown in [0x00, 0x01, 0x02, 0x12, 0xFF] {
            assert_eq!(form_factor(unknown, plain), None);
        }
    }

    #[test]
    fn type_detail_bits() {
        let bits = |detail: u16| (TypeDetail(detail).registered(), TypeDetail(detail).buffered());
        assert_eq!(bits(0x4000), (Some(false), Some(false)));
        assert_eq!(bits(0x2000), (Some(true), None));
        assert_eq!(bits(0x8000), (Some(true), Some(true)));
        assert_eq!(bits(0xA000), (Some(true), Some(true)));
        assert_eq!(bits(0x0080), (None, None));
        // Contradictory firmware: registered and unbuffered at once.
        assert_eq!(bits(0x6000), (None, None));
    }

    #[test]
    fn memory_type_names() {
        let name = |code| MemoryType::from_code(code).map(MemoryType::name);
        assert_eq!(name(0x12), Some("DDR"));
        assert_eq!(name(0x13), Some("DDR2"));
        assert_eq!(name(0x14), Some("DDR2 FB-DIMM"));
        assert_eq!(name(0x18), Some("DDR3"));
        assert_eq!(name(0x1A), Some("DDR4"));
        assert_eq!(name(0x1D), Some("LPDDR3"));
        assert_eq!(name(0x1E), Some("LPDDR4"));
        assert_eq!(name(0x22), Some("DDR5"));
        assert_eq!(name(0x23), Some("LPDDR5"));
        assert_eq!(name(0x0F), Some("SDRAM"));
        for unknown in [0x00, 0x01, 0x02, 0x09, 0x15, 0x1F, 0x25] {
            assert_eq!(name(unknown), None);
        }
        // Every technology goes by the name the specification gives its code.
        for code in 0..=u8::MAX {
            if let Some(kind) = MemoryType::from_code(code) {
                assert_eq!(Some(kind.name()), memory_device::memory_type_name(code), "{code:#04X}");
            }
        }
        assert!(MemoryType::Lpddr4.is_double_data_rate());
        assert!(!MemoryType::Other("SDRAM").is_double_data_rate());
    }

    #[test]
    fn placeholders_become_none() {
        for filler in [
            "Unknown",
            "UNKNOWN",
            " Not Specified ",
            "NO DIMM",
            "None",
            "N/A",
            "[Empty]",
            "To Be Filled By O.E.M.",
            "OEM",
            "Not Applicable",
            "Manufacturer00",
            "A1_Manufacturer0",
            "SerNum01",
            "Array1_PartNumber0",
            "ModulePartNumber00",
            "00000000",
            "0x00000000",
            "FFFFFFFF",
            "ffffffffffff",
        ] {
            assert!(is_placeholder(filler), "{filler}");
        }
        for value in ["Example Memory", "EX16G56C46U5", "1A2B3C4D", "00000001", "F", "FF", "0x2C00", "Nanya"]
        {
            assert!(!is_placeholder(value), "{value}");
        }

        let blank = Device::ddr5("DIMM 0", "")
            .string(0x17, "Manufacturer00")
            .string(0x18, "00000000")
            .string(0x1A, "NO DIMM");
        let m = one(blank).module;
        assert!(m.manufacturer.is_none() && m.serial_number.is_none() && m.part_number.is_none());
    }

    #[test]
    fn manufacturer_resolution() {
        let resolve = |text, id| manufacturer(text, id);
        // A firmware name is kept verbatim, whatever the id field says.
        assert_eq!(resolve(Some("Example Memory"), Some(0xCE80)).as_deref(), Some("Example Memory"));
        // Hex ids in the encodings firmware uses.
        assert_eq!(resolve(Some("0x2C00"), None).as_deref(), Some("Micron Technology"));
        assert_eq!(resolve(Some("80AD000080AD"), None).as_deref(), Some("SK hynix"));
        assert_eq!(resolve(Some("04CD"), None).as_deref(), Some("G.Skill"));
        // An id that is not in the table: the id field decides, else the text stays.
        assert_eq!(resolve(Some("0B7A"), Some(0x9801)).as_deref(), Some("Kingston"));
        assert_eq!(resolve(Some("0B7A"), Some(0)).as_deref(), Some("0B7A"));
        assert_eq!(resolve(Some("0B7A"), None).as_deref(), Some("0B7A"));
        // No usable string: the SPD-derived id field alone (LSB = continuation count).
        assert_eq!(resolve(None, Some(0x2C80)).as_deref(), Some("Micron Technology"));
        assert_eq!(resolve(None, Some(0x9B85)).as_deref(), Some("Crucial Technology"));
        assert_eq!(resolve(None, Some(0)), None);
        assert_eq!(resolve(None, None), None);
    }

    #[test]
    fn manufacturer_id_field_backs_up_a_placeholder_string() {
        let device = Device::ddr5("DIMM 0", "").string(0x17, "Unknown").word(0x2C, 0xCE80);
        assert_eq!(one(device).module.manufacturer.as_deref(), Some("Samsung"));
    }

    #[test]
    fn rank_and_voltage_zero_mean_unknown() {
        let m = one(Device::ddr5("DIMM 0", "").byte(0x1B, 0xF0).word(0x26, 0)).module;
        assert!(m.ranks.is_none() && m.voltage.is_none());
        let m = one(Device::ddr5("DIMM 0", "").byte(0x1B, 0x12).word(0x26, 1350)).module;
        assert_eq!((m.ranks, m.voltage), (Some(2), Some(1.35)));
    }

    #[test]
    fn correction_comes_from_the_owning_array() {
        let ecc_module = || Device::ddr5("DIMM 0", "").word(0x08, 72);
        let with = |code: u8, device: Device| {
            let table = smbios(&[array(0x1000, 0x03, code)], &[device]);
            slots(&table).remove(0).module.correction
        };
        assert_eq!(with(0x03, ecc_module()).as_deref(), Some("None"));
        assert_eq!(with(0x04, ecc_module()).as_deref(), Some("Parity"));
        assert_eq!(with(0x05, ecc_module()).as_deref(), Some("ECC"));
        assert_eq!(with(0x06, ecc_module()).as_deref(), Some("ECC"));
        assert_eq!(with(0x07, ecc_module()).as_deref(), Some("CRC"));
        assert_eq!(with(0x01, ecc_module()), None);
        assert_eq!(with(0x02, ecc_module()), None);
        // An ECC-capable array holding a module without check bits.
        assert_eq!(with(0x06, Device::ddr5("DIMM 0", "")).as_deref(), Some("None"));
        // Unknown widths leave the array's word standing.
        let widthless = Device::ddr5("DIMM 0", "").word(0x08, 0xFFFF).word(0x0A, 0xFFFF);
        assert_eq!(with(0x06, widthless).as_deref(), Some("ECC"));
    }

    #[test]
    fn correction_is_unknown_without_an_array() {
        let table = smbios(&[], &[Device::ddr5("DIMM 0", "")]);
        let slots = slots(&table);
        assert_eq!(slots.len(), 1);
        assert!(slots[0].module.correction.is_none());
    }

    #[test]
    fn devices_follow_their_own_array() {
        let table = smbios(
            &[array(0x1000, 0x03, 0x03), array(0x1001, 0x03, 0x06)],
            &[
                Device::ddr5("CPU1_DIMM_A1", "").word(0x08, 80),
                Device::ddr5("CPU2_DIMM_A1", "").word(0x08, 80).word(0x04, 0x1001),
            ],
        );
        let corrections: Vec<_> = slots(&table).into_iter().map(|slot| slot.module.correction).collect();
        assert_eq!(corrections, [Some("None".to_string()), Some("ECC".to_string())]);
    }

    #[test]
    fn skips_devices_that_are_not_system_memory() {
        let flash_chip =
            Device::new(0x1B).word(0x0C, 16).byte(0x0E, 0x05).string(0x10, "SYSTEM ROM").byte(0x12, 0x09);
        let video = Device::ddr5("VRAM 0", "").word(0x04, 0x1001);
        let namespace = Device::ddr5("NVDIMM NS0", "").byte(0x12, 0x1F);
        let table = smbios(
            &[array(0x1000, 0x03, 0x03), array(0x1001, 0x04, 0x03)],
            &[flash_chip, Device::ddr5("DIMM 0", ""), video, namespace, Device::empty("DIMM 1", "")],
        );
        let slots = slots(&table);
        let labels: Vec<_> =
            slots.iter().map(|s| (s.module.slot.as_str(), s.module.locator.as_str())).collect();
        assert_eq!(labels, [("Slot #1", "DIMM 0"), ("Slot #2", "DIMM 1")]);
    }

    #[test]
    fn missing_locator_strings_are_empty() {
        let slot = one(Device::new(0x5C).word(0x0C, 1024).byte(0x12, 0x1A));
        assert_eq!((slot.module.locator.as_str(), slot.bank_locator.as_str()), ("", ""));
        assert_eq!(slot.module.kind.as_deref(), Some("DDR4"));
        assert!(slot.module.form_factor.is_none());
    }
}
