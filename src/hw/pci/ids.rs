//! Parsing of the id strings Windows assigns to a PCI device, e.g.
//! `PCI\VEN_10DE&DEV_2882&SUBSYS_51611462&REV_A1` and `PCI\CC_030000`.

/// Class code carried by a `CC_bbsspp` field (or its shorter `CC_bbss` form).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassCode {
    pub base: u8,
    pub sub: u8,
    pub prog_if: Option<u8>,
}

/// Identification fields gathered from the hardware and compatible ids of one device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IdFields {
    pub vendor_id: Option<u16>,
    pub device_id: Option<u16>,
    pub subsys_id: Option<u16>,
    pub subsys_vendor_id: Option<u16>,
    pub revision: Option<u8>,
    pub class: Option<ClassCode>,
}

impl IdFields {
    /// Collects the fields of every id in `ids`. Windows lists the most specific
    /// id first, so the first value seen for a field wins.
    pub fn from_ids<'a>(ids: impl IntoIterator<Item = &'a str>) -> Self {
        let mut fields = IdFields::default();
        for id in ids {
            fields.absorb(id);
        }
        fields
    }

    /// Merges the fields of one id string. Unknown or malformed fields are ignored.
    pub fn absorb(&mut self, id: &str) {
        // Drop the enumerator prefix (`PCI\`) and, for instance ids, the trailing instance part.
        let body = id.split('\\').nth(1).unwrap_or(id);
        for field in body.split('&') {
            let Some((key, value)) = field.split_once('_') else { continue };
            match key.to_ascii_uppercase().as_str() {
                "VEN" => self.vendor_id = self.vendor_id.or(hex16(value)),
                "DEV" => self.device_id = self.device_id.or(hex16(value)),
                "REV" => self.revision = self.revision.or(hex8(value)),
                "SUBSYS" => self.absorb_subsystem(value),
                "CC" => self.absorb_class(value),
                _ => {}
            }
        }
    }

    /// `SUBSYS_ssssvvvv`: the subsystem id comes first, then the subsystem vendor.
    fn absorb_subsystem(&mut self, value: &str) {
        if self.subsys_id.is_some() || value.len() != 8 || !value.is_ascii() {
            return;
        }
        if let (Some(id), Some(vendor)) = (hex16(&value[..4]), hex16(&value[4..])) {
            self.subsys_id = Some(id);
            self.subsys_vendor_id = Some(vendor);
        }
    }

    /// A class code with a programming interface replaces one without.
    fn absorb_class(&mut self, value: &str) {
        if self.class.is_some_and(|c| c.prog_if.is_some()) {
            return;
        }
        if let Some(class) = parse_class(value) {
            if self.class.is_none() || class.prog_if.is_some() {
                self.class = Some(class);
            }
        }
    }
}

fn parse_class(value: &str) -> Option<ClassCode> {
    if !value.is_ascii() || !matches!(value.len(), 4 | 6) {
        return None;
    }
    let prog_if = if value.len() == 6 { Some(hex8(&value[4..])?) } else { None };
    Some(ClassCode { base: hex8(&value[..2])?, sub: hex8(&value[2..4])?, prog_if })
}

/// Exactly four hex digits.
fn hex16(text: &str) -> Option<u16> {
    (text.len() == 4 && is_hex(text)).then(|| u16::from_str_radix(text, 16).ok()).flatten()
}

/// Exactly two hex digits.
fn hex8(text: &str) -> Option<u8> {
    (text.len() == 2 && is_hex(text)).then(|| u8::from_str_radix(text, 16).ok()).flatten()
}

fn is_hex(text: &str) -> bool {
    text.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_hardware_id() {
        let fields = IdFields::from_ids(["PCI\\VEN_10DE&DEV_2882&SUBSYS_51611462&REV_A1"]);
        assert_eq!(fields.vendor_id, Some(0x10DE));
        assert_eq!(fields.device_id, Some(0x2882));
        assert_eq!(fields.subsys_id, Some(0x5161));
        assert_eq!(fields.subsys_vendor_id, Some(0x1462));
        assert_eq!(fields.revision, Some(0xA1));
        assert_eq!(fields.class, None);
    }

    #[test]
    fn merges_fields_across_ids() {
        let fields = IdFields::from_ids([
            "PCI\\VEN_1022&DEV_14DB&SUBSYS_7D781462&REV_00",
            "PCI\\VEN_1022&DEV_14DB&SUBSYS_7D781462",
            "PCI\\VEN_1022&DEV_14DB&CC_060400",
            "PCI\\VEN_1022&DEV_14DB&CC_0604",
        ]);
        assert_eq!(fields.vendor_id, Some(0x1022));
        assert_eq!(fields.revision, Some(0x00));
        assert_eq!(fields.class, Some(ClassCode { base: 0x06, sub: 0x04, prog_if: Some(0x00) }));
    }

    #[test]
    fn first_value_wins() {
        let fields = IdFields::from_ids(["PCI\\VEN_8086&DEV_A0A8&REV_20", "PCI\\VEN_1234&DEV_5678&REV_01"]);
        assert_eq!(
            (fields.vendor_id, fields.device_id, fields.revision),
            (Some(0x8086), Some(0xA0A8), Some(0x20))
        );
    }

    #[test]
    fn full_class_code_replaces_short_one() {
        let fields = IdFields::from_ids(["PCI\\CC_0108", "PCI\\CC_010802", "PCI\\CC_0C0330"]);
        assert_eq!(fields.class, Some(ClassCode { base: 0x01, sub: 0x08, prog_if: Some(0x02) }));
        let short = IdFields::from_ids(["PCI\\VEN_1022&CC_0600"]);
        assert_eq!(short.class, Some(ClassCode { base: 0x06, sub: 0x00, prog_if: None }));
    }

    #[test]
    fn accepts_lower_case_and_instance_ids() {
        let fields =
            IdFields::from_ids(["pci\\ven_1b21&dev_1064&subsys_21162116&rev_02\\6&1a2b3c&0&00400012"]);
        assert_eq!(fields.vendor_id, Some(0x1B21));
        assert_eq!(fields.device_id, Some(0x1064));
        assert_eq!(fields.subsys_id, Some(0x2116));
        assert_eq!(fields.revision, Some(0x02));
    }

    #[test]
    fn ignores_malformed_fields() {
        let fields = IdFields::from_ids([
            "PCI\\VEN_10D&DEV_28820&SUBSYS_5161146&REV_A",
            "PCI\\VEN_+0DE&DEV_ZZZZ&CC_03&CC_0300000",
            "",
            "garbage",
        ]);
        assert_eq!(fields, IdFields::default());
    }

    #[test]
    fn tolerates_non_ascii_text() {
        // Multi-byte characters sitting where the fixed-width fields are cut.
        let fields = IdFields::from_ids([
            "PCI\\VEN_10D\u{c9}&DEV_\u{c9}\u{c9}&REV_\u{c9}",
            "PCI\\SUBSYS_516\u{c9}146&CC_0\u{c9}0&CC_030\u{c9}0",
        ]);
        assert_eq!(fields, IdFields::default());
    }
}
