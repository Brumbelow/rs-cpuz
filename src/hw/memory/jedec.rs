//! JEDEC JEP106 manufacturer identification codes of memory module vendors.
//!
//! An id is a seven-bit code plus the number of `0x7F` continuation codes that
//! precede it (the "bank", counted from one). Bit 7 of every byte is an odd-parity
//! bit. A module's SPD stores the id as two bytes - the continuation count, then
//! the code - and firmware likes to print those bytes as hex text instead of
//! looking the vendor up, so the same id shows up as `04CD`, `0x04CD`,
//! `04CD000080AD` or as the raw sequence `7F7F7F7FCD`.

/// The JEP106 continuation code.
const CONTINUATION: u8 = 0x7F;

/// `(continuation codes, code including its parity bit, vendor)`.
///
/// Deliberately short: module vendors whose assignment is well established.
const VENDORS: &[(u8, u8, &str)] = &[
    (0, 0x2C, "Micron Technology"),
    (0, 0x89, "Intel"),
    (0, 0xAD, "SK hynix"),
    (0, 0xC1, "Infineon"),
    (0, 0xCE, "Samsung"),
    (1, 0x4F, "Transcend"),
    (1, 0x7A, "Apacer"),
    (1, 0x94, "SMART Modular"),
    (1, 0x98, "Kingston"),
    (1, 0xBA, "PNY"),
    (2, 0x9E, "Corsair"),
    (2, 0xFE, "Elpida"),
    (3, 0x0B, "Nanya Technology"),
    (3, 0x13, "GeIL"),
    (3, 0x25, "Kingmax"),
    (3, 0x94, "Mushkin"),
    (4, 0x43, "Ramaxel Technology"),
    (4, 0xCB, "ADATA Technology"),
    (4, 0xCD, "G.Skill"),
    (4, 0xEF, "Team Group"),
    (5, 0x02, "Patriot Memory"),
    (5, 0x51, "Qimonda"),
    (5, 0x9B, "Crucial Technology"),
];

/// Looks up a code (parity bit included) behind `continuations` continuation codes.
fn lookup(continuations: usize, code: u8) -> Option<&'static str> {
    VENDORS
        .iter()
        .find(|&&(bank, vendor_code, _)| usize::from(bank) == continuations && vendor_code == code)
        .map(|&(_, _, name)| name)
}

/// Names the vendor at the start of an id byte string. Understands the raw JEP106
/// sequence (leading `0x7F`s) and the two-byte SPD form, whose continuation count
/// may come with or without its parity bit.
///
/// Some firmware prints the code first (`2C00`). That order is only accepted when
/// the second byte has no code bits and so must be a count of zero; reading every id
/// both ways would turn one vendor's id into another's.
pub fn vendor(id: &[u8]) -> Option<&'static str> {
    let first = *id.first()?;
    if first == CONTINUATION {
        let continuations = id.iter().take_while(|&&byte| byte == CONTINUATION).count();
        return lookup(continuations, *id.get(continuations)?);
    }
    let second = *id.get(1)?;
    if second & 0x7F == 0 {
        lookup(0, first)
    } else {
        lookup(usize::from(first & 0x7F), second)
    }
}

/// Reads a manufacturer string that is really an id printed as hex: whole bytes, at
/// least two of them, optionally spaced and prefixed with `0x` or `JEDEC ID:`.
/// `None` for anything else, in particular for a vendor name.
pub fn hex_id(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    let text = match text.get(..9) {
        Some(prefix) if prefix.eq_ignore_ascii_case("JEDEC ID:") => text[9..].trim_start(),
        _ => text,
    };
    let text = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")).unwrap_or(text);
    let digits: Vec<u8> = text
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .map(|c| c.to_digit(16).and_then(|digit| u8::try_from(digit).ok()))
        .collect::<Option<_>>()?;
    (digits.len() >= 4 && digits.len() % 2 == 0)
        .then(|| digits.chunks_exact(2).map(|pair| (pair[0] << 4) | pair[1]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn from_text(text: &str) -> Option<&'static str> {
        vendor(&hex_id(text)?)
    }

    #[test]
    fn table_codes_carry_odd_parity_and_are_unique() {
        for (index, &(bank, code, name)) in VENDORS.iter().enumerate() {
            assert_eq!(code.count_ones() % 2, 1, "{name}");
            assert_ne!(code & 0x7F, 0, "{name}");
            assert_ne!(code, CONTINUATION, "{name}");
            assert!(VENDORS[..index].iter().all(|&(b, c, _)| (b, c) != (bank, code)), "{name}");
        }
    }

    #[test]
    fn spd_byte_order() {
        assert_eq!(vendor(&[0x80, 0x2C]), Some("Micron Technology"));
        assert_eq!(vendor(&[0x80, 0xAD]), Some("SK hynix"));
        assert_eq!(vendor(&[0x80, 0xCE]), Some("Samsung"));
        assert_eq!(vendor(&[0x01, 0x98]), Some("Kingston"));
        assert_eq!(vendor(&[0x02, 0x9E]), Some("Corsair"));
        assert_eq!(vendor(&[0x83, 0x0B]), Some("Nanya Technology"));
        assert_eq!(vendor(&[0x04, 0xCB]), Some("ADATA Technology"));
        assert_eq!(vendor(&[0x04, 0xCD]), Some("G.Skill"));
        assert_eq!(vendor(&[0x04, 0xEF]), Some("Team Group"));
        assert_eq!(vendor(&[0x85, 0x02]), Some("Patriot Memory"));
        assert_eq!(vendor(&[0x85, 0x9B]), Some("Crucial Technology"));
    }

    #[test]
    fn continuation_count_without_parity_bit() {
        assert_eq!(vendor(&[0x00, 0xAD]), Some("SK hynix"));
        assert_eq!(vendor(&[0x03, 0x0B]), Some("Nanya Technology"));
        assert_eq!(vendor(&[0x05, 0x9B]), Some("Crucial Technology"));
    }

    #[test]
    fn code_first_order_needs_a_zero_count() {
        assert_eq!(vendor(&[0x2C, 0x00]), Some("Micron Technology"));
        assert_eq!(vendor(&[0xCE, 0x80]), Some("Samsung"));
        assert_eq!(vendor(&[0xAD, 0x00, 0xB3, 0x00]), Some("SK hynix"));
        // With a non-zero count the bytes are read in SPD order only.
        assert_eq!(vendor(&[0x98, 0x01]), None);
        assert_eq!(vendor(&[0xCD, 0x04]), None);
        // Bank 3 code 0x85 and bank 12 code 0x83 are other vendors, not the table
        // entries their bytes spell backwards (Patriot Memory, Nanya Technology).
        assert_eq!(vendor(&[0x02, 0x85]), None);
        assert_eq!(vendor(&[0x0B, 0x83]), None);
    }

    #[test]
    fn raw_continuation_sequence() {
        assert_eq!(vendor(&[0x7F, 0x98, 0, 0, 0, 0, 0, 0]), Some("Kingston"));
        assert_eq!(vendor(&[0x7F, 0x7F, 0x9E]), Some("Corsair"));
        assert_eq!(vendor(&[0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 0x51]), Some("Qimonda"));
        assert_eq!(vendor(&[0x7F, 0x7F]), None);
        assert_eq!(vendor(&[0x7F, 0x2C]), None);
    }

    #[test]
    fn same_code_in_different_banks() {
        assert_eq!(vendor(&[0x01, 0x94]), Some("SMART Modular"));
        assert_eq!(vendor(&[0x83, 0x94]), Some("Mushkin"));
        assert_eq!(vendor(&[0x80, 0x94]), None);
    }

    #[test]
    fn unknown_or_malformed_ids() {
        assert_eq!(vendor(&[]), None);
        assert_eq!(vendor(&[0x2C]), None);
        assert_eq!(vendor(&[0x00, 0x00]), None);
        assert_eq!(vendor(&[0xFF, 0xFF]), None);
        // The right code without its parity bit is not trusted.
        assert_eq!(vendor(&[0x80, 0x2D]), None);
        assert_eq!(vendor(&[0x80, 0x4E]), None);
    }

    #[test]
    fn hex_text_forms() {
        assert_eq!(hex_id("04CD"), Some(vec![0x04, 0xCD]));
        assert_eq!(hex_id(" 0x2c00 "), Some(vec![0x2C, 0x00]));
        assert_eq!(hex_id("0X859B"), Some(vec![0x85, 0x9B]));
        assert_eq!(hex_id("80AD000080AD"), Some(vec![0x80, 0xAD, 0, 0, 0x80, 0xAD]));
        assert_eq!(hex_id("JEDEC ID:7F 98 00 00"), Some(vec![0x7F, 0x98, 0, 0]));
        assert_eq!(hex_id("jedec id: AD 00"), Some(vec![0xAD, 0x00]));
        for not_an_id in
            ["", "2C", "0x2C", "2C0", "Samsung", "ADATA", "Micron Technology", "0x", "JEDEC ID:", "04CG"]
        {
            assert_eq!(hex_id(not_an_id), None, "{not_an_id}");
        }
    }

    #[test]
    fn translates_hex_text() {
        assert_eq!(from_text("0x2C00"), Some("Micron Technology"));
        assert_eq!(from_text("80AD000080AD"), Some("SK hynix"));
        assert_eq!(from_text("04CD"), Some("G.Skill"));
        assert_eq!(from_text("00CE00B300CE"), Some("Samsung"));
        assert_eq!(from_text("AD00B300000000000000"), Some("SK hynix"));
        assert_eq!(from_text("JEDEC ID:7F 7F 9E 00 00 00 00 00"), Some("Corsair"));
        assert_eq!(from_text("0x0B7A"), None);
        assert_eq!(from_text("Kingston"), None);
    }
}
