//! The logo mark: a minimal microchip glyph, rasterised in plain Rust.
//!
//! The build script compiles this file as well (to generate the executable's
//! icon), so it must not use anything outside `std`.

/// Accent colour as `0xRRGGBB`.
pub const ACCENT_RGB: u32 = 0xCE_42_2B;

/// Sub-samples per pixel edge on the pixels the outline passes through.
const SAMPLES: u32 = 8;

/// An axis-aligned rectangle with rounded corners, in pixel coordinates.
struct RoundRect {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    radius: f64,
}

impl RoundRect {
    fn contains(&self, x: f64, y: f64) -> bool {
        if x < self.x0 || x >= self.x1 || y < self.y0 || y >= self.y1 {
            return false;
        }
        let dx = (self.x0 + self.radius - x).max(x - (self.x1 - self.radius)).max(0.0);
        let dy = (self.y0 + self.radius - y).max(y - (self.y1 - self.radius)).max(0.0);
        dx * dx + dy * dy <= self.radius * self.radius
    }
}

/// The glyph laid out for one raster size: the chip body and three pin bars
/// in each direction, minus the hole in the middle.
struct Glyph {
    body: RoundRect,
    hole: RoundRect,
    bars: [RoundRect; 6],
}

impl Glyph {
    /// Lays the glyph out on a 16-unit grid scaled to `size` pixels. Every
    /// straight edge is snapped to a whole pixel so small sizes stay crisp.
    fn new(size: u32) -> Self {
        let extent = f64::from(size);
        let unit = extent / 16.0;
        let pin_length = (2.0 * unit).round().max(1.0);
        let pin_width = centred(2.0 * unit, size);
        let pitch = (4.0 * unit).round().max(pin_width + 1.0);
        let hole = centred(4.0 * unit, size);

        let span = |width: f64, offset: f64| {
            let start = (extent - width) / 2.0 + offset;
            (start, start + width)
        };
        let bar = |offset: f64, vertical: bool| {
            let (near, far) = span(pin_width, offset);
            let radius = pin_width / 4.0;
            if vertical {
                RoundRect { x0: near, y0: 0.0, x1: far, y1: extent, radius }
            } else {
                RoundRect { x0: 0.0, y0: near, x1: extent, y1: far, radius }
            }
        };
        let (hole_near, hole_far) = span(hole, 0.0);
        Glyph {
            body: RoundRect {
                x0: pin_length,
                y0: pin_length,
                x1: extent - pin_length,
                y1: extent - pin_length,
                radius: unit,
            },
            hole: RoundRect { x0: hole_near, y0: hole_near, x1: hole_far, y1: hole_far, radius: unit / 2.0 },
            bars: [
                bar(-pitch, true),
                bar(0.0, true),
                bar(pitch, true),
                bar(-pitch, false),
                bar(0.0, false),
                bar(pitch, false),
            ],
        }
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        (self.body.contains(x, y) || self.bars.iter().any(|bar| bar.contains(x, y)))
            && !self.hole.contains(x, y)
    }

    /// Fraction of the pixel at (`px`, `py`) covered by the glyph, as an alpha value.
    fn pixel(&self, px: u32, py: u32) -> u8 {
        let (x, y) = (f64::from(px), f64::from(py));
        let centre = self.contains(x + 0.5, y + 0.5);
        let corners = [(x, y), (x + 1.0, y), (x, y + 1.0), (x + 1.0, y + 1.0)];
        if corners.iter().all(|&(cx, cy)| self.contains(cx, cy) == centre) {
            return if centre { 255 } else { 0 };
        }
        let step = 1.0 / f64::from(SAMPLES);
        let offsets = || (0..SAMPLES).map(move |i| (f64::from(i) + 0.5) * step);
        let hits = offsets().flat_map(|dy| offsets().map(move |dx| (dx, dy)));
        let covered = hits.filter(|&(dx, dy)| self.contains(x + dx, y + dy)).count() as u32;
        ((covered * 255 + SAMPLES * SAMPLES / 2) / (SAMPLES * SAMPLES)) as u8
    }
}

/// The whole number of pixels closest to `width` that can be centred in `size`
/// pixels without ending on a half pixel (ties round down).
fn centred(width: f64, size: u32) -> f64 {
    let parity = f64::from(size % 2);
    (((width - parity) / 2.0 - 0.5).ceil() * 2.0 + parity).max(2.0 - parity)
}

/// Rasterises the mark into a `size` x `size` square. Returns one coverage
/// (alpha) byte per pixel, rows running from top to bottom.
pub fn coverage(size: u32) -> Vec<u8> {
    let glyph = Glyph::new(size);
    (0..size).flat_map(|y| (0..size).map(move |x| (x, y))).map(|(x, y)| glyph.pixel(x, y)).collect()
}

/// Encodes the mark in colour `rgb` as an icon image: the device-independent
/// bitmap layout shared by `.ico` files, icon resources and
/// `CreateIconFromResourceEx` (header, bottom-up BGRA pixels, 1-bit mask).
pub fn icon_image(size: u32, rgb: u32) -> Vec<u8> {
    let alpha = coverage(size);
    let width = size as usize;
    let mask_stride = width.div_ceil(32) * 4;
    let image_bytes = (width * 4 + mask_stride) * width;
    let [blue, green, red, _] = rgb.to_le_bytes();

    let mut out = Vec::with_capacity(40 + image_bytes);
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&(size * 2).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(image_bytes as u32).to_le_bytes());
    out.extend_from_slice(&[0; 16]);

    let rows = || alpha.chunks_exact(width.max(1)).rev();
    for row in rows() {
        for &a in row {
            out.extend_from_slice(&[blue, green, red, a]);
        }
    }
    for row in rows() {
        let mut bits = vec![0u8; mask_stride];
        for (x, _) in row.iter().enumerate().filter(|(_, &a)| a == 0) {
            bits[x / 8] |= 0x80 >> (x % 8);
        }
        out.extend_from_slice(&bits);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(mask: &[u8], size: u32, x: u32, y: u32) -> u8 {
        mask[(y * size + x) as usize]
    }

    /// Renders the mark as text: `#` opaque, `.` empty, `+` partly covered.
    fn art(size: u32) -> Vec<String> {
        let glyph = |&alpha: &u8| match alpha {
            0 => '.',
            255 => '#',
            _ => '+',
        };
        coverage(size).chunks(size as usize).map(|row| row.iter().map(glyph).collect()).collect()
    }

    #[test]
    fn sixteen_pixel_mark_sits_on_whole_pixels() {
        // Only the rounded corners of the body, the hole and the pin tips are anti-aliased.
        let expected = [
            "...++..++..++...",
            "...##..##..##...",
            "..+##########+..",
            "+##############+",
            "+##############+",
            "..############..",
            "..####+..+####..",
            "+#####....#####+",
            "+#####....#####+",
            "..####+..+####..",
            "..############..",
            "+##############+",
            "+##############+",
            "..+##########+..",
            "...##..##..##...",
            "...++..++..++...",
        ];
        assert_eq!(art(16), expected);
    }

    #[test]
    fn straight_edges_stay_sharp_at_every_icon_size() {
        for size in [16, 20, 24, 32, 40, 48, 64, 256] {
            // The row and column through the middle cross pins, body and hole but no rounded corner
            // other than the two pin tips at their ends.
            let art = art(size);
            let middle = &art[size as usize / 2];
            assert!(!middle[1..size as usize - 1].contains('+'), "size {size}: {middle}");
            let column: String = art.iter().map(|row| &row[size as usize / 2..][..1]).collect();
            assert_eq!(&column, middle, "size {size}");
        }
    }

    #[test]
    fn mark_is_symmetric() {
        for size in [15, 16, 20, 24, 33, 48] {
            let mask = coverage(size);
            for y in 0..size {
                for x in 0..size {
                    let value = at(&mask, size, x, y);
                    assert_eq!(value, at(&mask, size, size - 1 - x, y), "size {size} mirror x at {x},{y}");
                    assert_eq!(value, at(&mask, size, x, size - 1 - y), "size {size} mirror y at {x},{y}");
                    assert_eq!(value, at(&mask, size, y, x), "size {size} transpose at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn corners_are_empty_and_body_is_solid() {
        for size in [16, 20, 24, 32, 40, 48, 64, 256] {
            let mask = coverage(size);
            assert_eq!(at(&mask, size, 0, 0), 0, "size {size}");
            assert_eq!(at(&mask, size, size / 2, size / 2), 0, "size {size} hole");
            assert_eq!(at(&mask, size, size / 4, size / 4), 255, "size {size} body");
            assert_eq!(at(&mask, size, size / 2, size / 16), 255, "size {size} centre pin");
        }
    }

    #[test]
    fn tiny_and_empty_sizes_do_not_panic() {
        assert!(coverage(0).is_empty());
        assert!(icon_image(0, ACCENT_RGB).len() >= 40);
        for size in 1..12 {
            assert_eq!(coverage(size).len(), (size * size) as usize);
        }
    }

    #[test]
    fn centred_widths_share_the_parity_of_the_size() {
        assert_eq!(centred(2.0, 16), 2.0);
        assert_eq!(centred(2.5, 20), 2.0);
        assert_eq!(centred(3.0, 24), 2.0);
        assert_eq!(centred(4.0, 32), 4.0);
        assert_eq!(centred(2.1, 17), 3.0);
        assert_eq!(centred(0.2, 3), 1.0);
        assert_eq!(centred(0.2, 4), 2.0);
    }

    #[test]
    fn icon_image_layout() {
        let image = icon_image(16, 0x11_22_33);
        // Header, 16x16 BGRA pixels, 16 mask rows padded to 32 bits.
        assert_eq!(image.len(), 40 + 16 * 16 * 4 + 16 * 4);
        assert_eq!(image[..4], 40u32.to_le_bytes());
        assert_eq!(image[4..8], 16u32.to_le_bytes());
        assert_eq!(image[8..12], 32u32.to_le_bytes());
        assert_eq!(image[14..16], 32u16.to_le_bytes());

        // Rows are bottom-up: the first two stored rows are the bottom pins.
        let pixel = |x: usize, y: usize| &image[40 + (y * 16 + x) * 4..][..4];
        assert_eq!(pixel(0, 0), [0x33, 0x22, 0x11, 0]);
        assert_eq!(pixel(7, 1), [0x33, 0x22, 0x11, 255]);

        // The mask marks exactly the fully transparent pixels of that row.
        let mask = &image[40 + 16 * 16 * 4..][..4];
        assert_eq!(mask, [0b1110_0110, 0b0110_0111, 0, 0]);
    }
}
