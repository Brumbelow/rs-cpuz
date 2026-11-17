//! Decoding of the base block of an EDID, the description a display hands to the
//! graphics card (VESA E-EDID 1.3 and 1.4).

use crate::model::{DisplayMode, MonitorInfo};

const BLOCK_LEN: usize = 128;
const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];

/// Offsets into the base block.
const MANUFACTURER: usize = 8;
const PRODUCT: usize = 10;
const SERIAL: usize = 12;
const WEEK: usize = 16;
const YEAR: usize = 17;
const VERSION: usize = 18;
const REVISION: usize = 19;
const VIDEO_INPUT: usize = 20;
const WIDTH_CM: usize = 21;
const HEIGHT_CM: usize = 22;
const GAMMA: usize = 23;
const DESCRIPTORS: usize = 54;

/// The base block ends with four descriptors of this size: detailed timings, or
/// display descriptors that carry text and limits.
const DESCRIPTOR_LEN: usize = 18;
const DESCRIPTOR_COUNT: usize = 4;

/// Tags of the display descriptors read here.
const TAG_SERIAL: u8 = 0xFF;
const TAG_RANGE_LIMITS: u8 = 0xFD;
const TAG_NAME: u8 = 0xFC;

/// The year the manufacturing date counts from.
const FIRST_YEAR: u16 = 1990;
/// Weeks a year can have in the manufacturing date; 0 and 0xFF are not weeks.
const WEEKS: std::ops::RangeInclusive<u8> = 1..=54;
/// The gamma byte of a display that describes its gamma elsewhere.
const GAMMA_ELSEWHERE: u8 = 0xFF;

const CM_PER_INCH: f32 = 2.54;

/// Decodes the base block at the start of `edid`. `None` unless it begins with the
/// EDID header and its checksum holds.
pub fn parse(edid: &[u8]) -> Option<MonitorInfo> {
    let block: &[u8; BLOCK_LEN] = edid.get(..BLOCK_LEN)?.try_into().ok()?;
    if block[..HEADER.len()] != HEADER || block.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) != 0 {
        return None;
    }
    // Revision 1.4 redefined the video input byte and added offsets to the range limits.
    let revision_4 = block[VERSION] == 1 && block[REVISION] >= 4;
    let descriptors =
        || block[DESCRIPTORS..DESCRIPTORS + DESCRIPTOR_COUNT * DESCRIPTOR_LEN].chunks_exact(DESCRIPTOR_LEN);
    let text = |tag| descriptors().find_map(|descriptor| display_text(descriptor, tag));
    let limits = descriptors().find_map(|descriptor| range_limits(descriptor, revision_4));
    let number = u32::from_le_bytes([block[SERIAL], block[SERIAL + 1], block[SERIAL + 2], block[SERIAL + 3]]);

    Some(MonitorInfo {
        model: text(TAG_NAME),
        id: product_id(block),
        serial: text(TAG_SERIAL).or_else(|| (number != 0).then(|| number.to_string())),
        week: Some(block[WEEK]).filter(|week| WEEKS.contains(week)),
        year: Some(FIRST_YEAR + u16::from(block[YEAR])),
        size_inches: diagonal_inches(block[WIDTH_CM], block[HEIGHT_CM]),
        max_resolution: largest(descriptors().filter_map(detailed_timing)),
        horizontal_khz: limits.map(|limits| limits.horizontal_khz),
        vertical_hz: limits.map(|limits| limits.vertical_hz),
        max_pixel_clock_mhz: limits.and_then(|limits| limits.max_pixel_clock_mhz),
        gamma: (block[GAMMA] != GAMMA_ELSEWHERE).then(|| (f32::from(block[GAMMA]) + 100.0) / 100.0),
        video_input: Some(video_input(block[VIDEO_INPUT], revision_4).to_string()),
    })
}

/// `EXM2700`: the three letters of the manufacturer, packed five bits each, and the
/// product code in hexadecimal.
fn product_id(block: &[u8; BLOCK_LEN]) -> Option<String> {
    let packed = u16::from_be_bytes([block[MANUFACTURER], block[MANUFACTURER + 1]]);
    let letter = |shift: u16| match (packed >> shift & 0x1F) as u8 {
        index @ 1..=26 => Some(char::from(b'A' + index - 1)),
        _ => None,
    };
    let product = u16::from_le_bytes([block[PRODUCT], block[PRODUCT + 1]]);
    Some(format!("{}{}{}{product:04X}", letter(10)?, letter(5)?, letter(0)?))
}

/// Diagonal of a picture of the given width and height. A display that states an
/// aspect ratio instead, or nothing at all, has one of the two at zero.
fn diagonal_inches(width_cm: u8, height_cm: u8) -> Option<f32> {
    (width_cm != 0 && height_cm != 0).then(|| f32::from(width_cm).hypot(f32::from(height_cm)) / CM_PER_INCH)
}

/// How the display is connected, from the video input byte.
fn video_input(input: u8, revision_4: bool) -> &'static str {
    if input & 0x80 == 0 {
        return "Analog";
    }
    // Before revision 1.4 the low bits of a digital input say nothing about the connector.
    match input & 0x0F {
        1 if revision_4 => "DVI",
        2 | 3 if revision_4 => "HDMI",
        4 if revision_4 => "MDDI",
        5 if revision_4 => "Display Port",
        _ => "Digital",
    }
}

/// A descriptor is a display descriptor when the pixel clock a detailed timing starts
/// with is zero; its tag follows a reserved byte.
fn display_tag(descriptor: &[u8]) -> Option<u8> {
    (descriptor[..3] == [0, 0, 0]).then_some(descriptor[3])
}

/// The text of a display descriptor with the given tag. It ends at a line feed and
/// is padded with spaces.
fn display_text(descriptor: &[u8], tag: u8) -> Option<String> {
    if display_tag(descriptor) != Some(tag) {
        return None;
    }
    let text: String = descriptor[5..]
        .iter()
        .take_while(|&&byte| byte != b'\n')
        .map(|&byte| if byte.is_ascii_graphic() { char::from(byte) } else { ' ' })
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// The frequencies a display can be driven at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RangeLimits {
    vertical_hz: (u32, u32),
    horizontal_khz: (u32, u32),
    max_pixel_clock_mhz: Option<u32>,
}

/// Decodes a display range limits descriptor. With `offsets` (revision 1.4) its flag
/// byte raises the rates that do not fit a byte by 255.
fn range_limits(descriptor: &[u8], offsets: bool) -> Option<RangeLimits> {
    if display_tag(descriptor) != Some(TAG_RANGE_LIMITS) {
        return None;
    }
    let flags = if offsets { descriptor[4] } else { 0 };
    let rate = |value: u8, raised: bool| u32::from(value) + if raised { 255 } else { 0 };
    // Two bits per direction: the high one raises the maximum, both together the minimum too.
    Some(RangeLimits {
        vertical_hz: (rate(descriptor[5], flags & 0x03 == 0x03), rate(descriptor[6], flags & 0x02 != 0)),
        horizontal_khz: (rate(descriptor[7], flags & 0x0C == 0x0C), rate(descriptor[8], flags & 0x08 != 0)),
        max_pixel_clock_mhz: (descriptor[9] != 0).then(|| u32::from(descriptor[9]) * 10),
    })
}

/// The mode a detailed timing descriptor defines; `None` for a display descriptor.
fn detailed_timing(descriptor: &[u8]) -> Option<DisplayMode> {
    let clock_hz = u64::from(u16::from_le_bytes([descriptor[0], descriptor[1]])) * 10_000;
    // Twelve-bit values: eight low bits in a byte of their own, four high bits in a shared one.
    let wide = |low: u8, high: u8| u32::from(low) | u32::from(high & 0x0F) << 8;
    let width = wide(descriptor[2], descriptor[4] >> 4);
    let lines = wide(descriptor[5], descriptor[7] >> 4);
    if clock_hz == 0 || width == 0 || lines == 0 {
        return None;
    }
    let total_width = width + wide(descriptor[3], descriptor[4]);
    let total_lines = lines + wide(descriptor[6], descriptor[7]);
    // An interlaced timing describes one field, half the lines of the picture.
    let interlaced = descriptor[17] & 0x80 != 0;
    Some(DisplayMode {
        width,
        height: if interlaced { lines * 2 } else { lines },
        refresh_hz: (clock_hz / u64::from(total_width * total_lines)) as u32,
    })
}

/// The mode with the most pixels; the first of equals, which is the one the display prefers.
fn largest(modes: impl Iterator<Item = DisplayMode>) -> Option<DisplayMode> {
    modes.fold(None, |best: Option<DisplayMode>, mode| match best {
        Some(best) if best.width * best.height >= mode.width * mode.height => Some(best),
        _ => Some(mode),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A display descriptor with `tag` and up to thirteen bytes of payload after its header.
    fn display_descriptor(tag: u8, payload: &[u8]) -> [u8; DESCRIPTOR_LEN] {
        let mut descriptor = [0u8; DESCRIPTOR_LEN];
        descriptor[3] = tag;
        descriptor[5..5 + payload.len()].copy_from_slice(payload);
        descriptor
    }

    /// A text descriptor: the text, a line feed and padding.
    fn text_descriptor(tag: u8, text: &str) -> [u8; DESCRIPTOR_LEN] {
        let mut payload = [b' '; 13];
        payload[..text.len()].copy_from_slice(text.as_bytes());
        if text.len() < payload.len() {
            payload[text.len()] = b'\n';
        }
        display_descriptor(tag, &payload)
    }

    /// A detailed timing for a `width` x `lines` picture with the given blanking.
    fn timing(clock_10khz: u16, width: u16, h_blank: u16, lines: u16, v_blank: u16) -> [u8; DESCRIPTOR_LEN] {
        let mut descriptor = [0u8; DESCRIPTOR_LEN];
        descriptor[..2].copy_from_slice(&clock_10khz.to_le_bytes());
        descriptor[2] = width as u8;
        descriptor[3] = h_blank as u8;
        descriptor[4] = ((width >> 8) << 4 | h_blank >> 8) as u8;
        descriptor[5] = lines as u8;
        descriptor[6] = v_blank as u8;
        descriptor[7] = ((lines >> 8) << 4 | v_blank >> 8) as u8;
        descriptor
    }

    /// Lays out a base block and seals it with its checksum.
    fn block(fixed: &[(usize, u8)], descriptors: [[u8; DESCRIPTOR_LEN]; DESCRIPTOR_COUNT]) -> Vec<u8> {
        let mut block = vec![0u8; BLOCK_LEN];
        block[..HEADER.len()].copy_from_slice(&HEADER);
        for &(offset, value) in fixed {
            block[offset] = value;
        }
        for (slot, descriptor) in descriptors.iter().enumerate() {
            let start = DESCRIPTORS + slot * DESCRIPTOR_LEN;
            block[start..start + DESCRIPTOR_LEN].copy_from_slice(descriptor);
        }
        seal(&mut block);
        block
    }

    fn seal(block: &mut [u8]) {
        let sum = block[..BLOCK_LEN - 1].iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        block[BLOCK_LEN - 1] = sum.wrapping_neg();
    }

    /// A made-up 27-inch display on a DisplayPort input: 3840 x 2160 at just under
    /// 60 Hz, built in week 12 of 2024.
    fn sample() -> Vec<u8> {
        block(
            &[
                // "EXM": E = 5, X = 24, M = 13.
                (MANUFACTURER, 0x17),
                (MANUFACTURER + 1, 0x0D),
                (PRODUCT, 0x00),
                (PRODUCT + 1, 0x27),
                (SERIAL, 0x15),
                (SERIAL + 1, 0xCD),
                (SERIAL + 2, 0x5B),
                (SERIAL + 3, 0x07),
                (WEEK, 12),
                (YEAR, 34),
                (VERSION, 1),
                (REVISION, 4),
                (VIDEO_INPUT, 0xB5),
                (WIDTH_CM, 60),
                (HEIGHT_CM, 34),
                (GAMMA, 120),
            ],
            [
                timing(53_325, 3840, 160, 2160, 62),
                display_descriptor(TAG_RANGE_LIMITS, &[40, 144, 30, 222, 60, 0x00, b'\n']),
                text_descriptor(TAG_NAME, "EX2700Q"),
                text_descriptor(TAG_SERIAL, "EXAMPLE000001"),
            ],
        )
    }

    #[test]
    fn decodes_a_display() {
        let monitor = parse(&sample()).unwrap();
        assert_eq!(monitor.model.as_deref(), Some("EX2700Q"));
        assert_eq!(monitor.id.as_deref(), Some("EXM2700"));
        assert_eq!(monitor.serial.as_deref(), Some("EXAMPLE000001"));
        assert_eq!((monitor.week, monitor.year), (Some(12), Some(2024)));
        // 60 cm by 34 cm.
        assert!((monitor.size_inches.unwrap() - 27.15).abs() < 0.01);
        // 533.25 MHz over 4000 x 2222 clocks is 59.996 Hz.
        assert_eq!(monitor.max_resolution, Some(DisplayMode { width: 3840, height: 2160, refresh_hz: 59 }));
        assert_eq!(monitor.horizontal_khz, Some((30, 222)));
        assert_eq!(monitor.vertical_hz, Some((40, 144)));
        assert_eq!(monitor.max_pixel_clock_mhz, Some(600));
        assert_eq!(monitor.gamma, Some(2.2));
        assert_eq!(monitor.video_input.as_deref(), Some("Display Port"));
    }

    #[test]
    fn extension_blocks_are_not_needed() {
        let mut edid = sample();
        let alone = parse(&edid).unwrap();
        edid.extend([0xA5; 2 * BLOCK_LEN]);
        assert_eq!(parse(&edid).unwrap(), alone);
    }

    #[test]
    fn rejects_what_is_not_an_edid() {
        let edid = sample();
        assert_eq!(parse(&[]), None);
        assert_eq!(parse(&edid[..BLOCK_LEN - 1]), None);
        assert_eq!(parse(&[0u8; BLOCK_LEN]), None);

        let mut damaged = edid.clone();
        damaged[GAMMA] ^= 0x01;
        assert_eq!(parse(&damaged), None);

        let mut headless = edid;
        headless[1] = 0x00;
        seal(&mut headless);
        assert_eq!(parse(&headless), None);
    }

    #[test]
    fn a_bare_block_leaves_the_unknown_out() {
        // An analog display of the 1.3 days that states its size as an aspect ratio
        // and nothing in its descriptors.
        let filler = display_descriptor(0x10, &[]);
        let edid = block(
            &[(VERSION, 1), (REVISION, 3), (VIDEO_INPUT, 0x08), (WIDTH_CM, 79), (GAMMA, 0xFF), (YEAR, 14)],
            [filler; DESCRIPTOR_COUNT],
        );
        let monitor = parse(&edid).unwrap();
        assert_eq!(
            monitor,
            MonitorInfo { year: Some(2004), video_input: Some("Analog".into()), ..Default::default() }
        );
    }

    #[test]
    fn falls_back_to_the_serial_number_field() {
        let mut edid = sample();
        let serial_slot = DESCRIPTORS + 3 * DESCRIPTOR_LEN;
        edid[serial_slot..serial_slot + DESCRIPTOR_LEN].copy_from_slice(&display_descriptor(0x10, &[]));
        seal(&mut edid);
        assert_eq!(parse(&edid).unwrap().serial.as_deref(), Some("123456789"));
    }

    #[test]
    fn reads_the_manufacturing_date() {
        let dated = |week: u8, year: u8| {
            let mut edid = sample();
            edid[WEEK] = week;
            edid[YEAR] = year;
            seal(&mut edid);
            let monitor = parse(&edid).unwrap();
            (monitor.week, monitor.year)
        };
        assert_eq!(dated(1, 0), (Some(1), Some(1990)));
        assert_eq!(dated(54, 36), (Some(54), Some(2026)));
        // No week given, and a model year instead of a date.
        assert_eq!(dated(0, 33), (None, Some(2023)));
        assert_eq!(dated(0xFF, 35), (None, Some(2025)));
        assert_eq!(dated(55, 35), (None, Some(2025)));
    }

    #[test]
    fn packs_three_letters_and_a_product_code() {
        let id = |high: u8, low: u8, product: u16| {
            let mut block = [0u8; BLOCK_LEN];
            block[MANUFACTURER] = high;
            block[MANUFACTURER + 1] = low;
            block[PRODUCT..PRODUCT + 2].copy_from_slice(&product.to_le_bytes());
            product_id(&block)
        };
        assert_eq!(id(0x17, 0x0D, 0x2700).as_deref(), Some("EXM2700"));
        // "AAA" and "ZZZ", the ends of the alphabet.
        assert_eq!(id(0x04, 0x21, 0x0001).as_deref(), Some("AAA0001"));
        assert_eq!(id(0x6B, 0x5A, 0xFFFF).as_deref(), Some("ZZZFFFF"));
        // A letter index of 0 or above 26 is not a letter.
        assert_eq!(id(0x00, 0x00, 0x1234), None);
        assert_eq!(id(0x7F, 0xFF, 0x1234), None);
    }

    #[test]
    fn names_the_video_input() {
        assert_eq!(video_input(0x00, true), "Analog");
        assert_eq!(video_input(0x7F, false), "Analog");
        assert_eq!(video_input(0x80, true), "Digital");
        assert_eq!(video_input(0x81, true), "DVI");
        assert_eq!(video_input(0xA2, true), "HDMI");
        assert_eq!(video_input(0xA3, true), "HDMI");
        assert_eq!(video_input(0x84, true), "MDDI");
        assert_eq!(video_input(0xB5, true), "Display Port");
        assert_eq!(video_input(0x8F, true), "Digital");
        // The same bits meant something else before revision 1.4.
        assert_eq!(video_input(0x81, false), "Digital");
        assert_eq!(video_input(0x85, false), "Digital");
    }

    #[test]
    fn measures_the_diagonal() {
        assert!((diagonal_inches(53, 30).unwrap() - 23.98).abs() < 0.01);
        assert!((diagonal_inches(34, 19).unwrap() - 15.33).abs() < 0.01);
        assert_eq!(diagonal_inches(0, 30), None);
        assert_eq!(diagonal_inches(16, 0), None);
        assert_eq!(diagonal_inches(0, 0), None);
    }

    #[test]
    fn reads_descriptor_text() {
        assert_eq!(display_text(&text_descriptor(TAG_NAME, "EX2700Q"), TAG_NAME).as_deref(), Some("EX2700Q"));
        // Thirteen characters leave no room for the line feed.
        assert_eq!(
            display_text(&text_descriptor(TAG_NAME, "THIRTEEN CHAR"), TAG_NAME).as_deref(),
            Some("THIRTEEN CHAR")
        );
        assert_eq!(display_text(&text_descriptor(TAG_NAME, "EX2700Q"), TAG_SERIAL), None);
        assert_eq!(display_text(&text_descriptor(TAG_NAME, ""), TAG_NAME), None);
        assert_eq!(display_text(&text_descriptor(TAG_NAME, "   "), TAG_NAME), None);
        // Bytes outside printable ASCII do not reach the report.
        let odd = display_descriptor(TAG_NAME, &[b'A', 0x00, 0x09, 0xE9, b'B', b'\n', b'C']);
        assert_eq!(display_text(&odd, TAG_NAME).as_deref(), Some("A   B"));
        // A detailed timing is not text, whatever its fourth byte is.
        let mut timing = timing(14_850, 1920, 280, 1080, 45);
        timing[3] = TAG_NAME;
        assert_eq!(display_text(&timing, TAG_NAME), None);
    }

    #[test]
    fn applies_the_range_limit_offsets() {
        let limits = |flags: u8, revision_4: bool| {
            let mut descriptor = display_descriptor(TAG_RANGE_LIMITS, &[48, 170, 242, 2, 80]);
            descriptor[4] = flags;
            range_limits(&descriptor, revision_4).unwrap()
        };
        let plain = limits(0x00, true);
        assert_eq!((plain.vertical_hz, plain.horizontal_khz), ((48, 170), (242, 2)));
        assert_eq!(plain.max_pixel_clock_mhz, Some(800));
        // Maximum horizontal rate raised: 242 to 257 kHz.
        assert_eq!(limits(0x08, true).horizontal_khz, (242, 257));
        // Both horizontal rates raised.
        assert_eq!(limits(0x0C, true).horizontal_khz, (497, 257));
        // Maximum vertical rate raised, then both.
        assert_eq!(limits(0x02, true).vertical_hz, (48, 425));
        assert_eq!(limits(0x03, true).vertical_hz, (303, 425));
        // The reserved combination raises nothing, and neither does an older revision.
        assert_eq!(limits(0x05, true), plain);
        assert_eq!(limits(0x0F, false), plain);

        let silent = display_descriptor(TAG_RANGE_LIMITS, &[50, 75, 30, 83, 0]);
        assert_eq!(range_limits(&silent, true).unwrap().max_pixel_clock_mhz, None);
        assert_eq!(range_limits(&text_descriptor(TAG_NAME, "EX2700Q"), true), None);
    }

    #[test]
    fn decodes_detailed_timings() {
        // 1920 x 1080 at 148.5 MHz over 2200 x 1125 clocks: 60 Hz.
        let full_hd = timing(14_850, 1920, 280, 1080, 45);
        assert_eq!(
            detailed_timing(&full_hd),
            Some(DisplayMode { width: 1920, height: 1080, refresh_hz: 60 })
        );
        // The interlaced form carries one field of 540 lines.
        let mut interlaced = timing(7_425, 1920, 280, 540, 22);
        interlaced[17] = 0x80;
        assert_eq!(
            detailed_timing(&interlaced),
            Some(DisplayMode { width: 1920, height: 1080, refresh_hz: 60 })
        );
        // The largest twelve-bit sizes do not overflow.
        assert_eq!(
            detailed_timing(&timing(65_535, 4095, 4095, 4095, 4095)),
            Some(DisplayMode { width: 4095, height: 4095, refresh_hz: 9 })
        );
        assert_eq!(detailed_timing(&text_descriptor(TAG_NAME, "EX2700Q")), None);
        assert_eq!(detailed_timing(&timing(14_850, 0, 280, 1080, 45)), None);
        assert_eq!(detailed_timing(&timing(14_850, 1920, 280, 0, 45)), None);
    }

    #[test]
    fn picks_the_largest_mode() {
        let mode = |width, height, refresh_hz| DisplayMode { width, height, refresh_hz };
        let modes = [mode(1920, 1080, 60), mode(2560, 1440, 59), mode(2560, 1440, 144), mode(1280, 720, 60)];
        assert_eq!(largest(modes.into_iter()), Some(mode(2560, 1440, 59)));
        assert_eq!(largest(std::iter::empty()), None);
    }
}
