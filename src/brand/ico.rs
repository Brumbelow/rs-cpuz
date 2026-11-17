//! Writer for the executable's `.ico` file, used by the build script.

use super::{mark, png};

/// Images this large are stored PNG-compressed, the form Windows expects for them.
const PNG_THRESHOLD: u32 = 256;

const HEADER_BYTES: usize = 6;
const ENTRY_BYTES: usize = 16;

/// Builds a `.ico` file holding the mark in colour `rgb` (`0xRRGGBB`) at each
/// of `sizes`, which the format limits to 256 pixels.
pub fn file(sizes: &[u32], rgb: u32) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&size| image(size, rgb)).collect();

    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = HEADER_BYTES + ENTRY_BYTES * images.len();
    for (&size, image) in sizes.iter().zip(&images) {
        // Width and height; a zero byte stands for 256 pixels.
        out.extend_from_slice(&[size as u8, size as u8, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(image.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += image.len();
    }
    for image in &images {
        out.extend_from_slice(image);
    }
    out
}

/// One image of the file: a bitmap, or a PNG from `PNG_THRESHOLD` pixels up.
fn image(size: u32, rgb: u32) -> Vec<u8> {
    if size < PNG_THRESHOLD {
        return mark::icon_image(size, rgb);
    }
    let [blue, green, red, _] = rgb.to_le_bytes();
    let rgba: Vec<u8> = mark::coverage(size).iter().flat_map(|&alpha| [red, green, blue, alpha]).collect();
    png::encode(size, size, &rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dword(bytes: &[u8], offset: usize) -> usize {
        u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]]) as usize
    }

    #[test]
    fn directory_describes_every_image() {
        let sizes = [16, 48, 256];
        let ico = file(&sizes, mark::ACCENT_RGB);
        assert_eq!(ico[..6], [0, 0, 1, 0, 3, 0]);

        let mut expected_offset = HEADER_BYTES + ENTRY_BYTES * sizes.len();
        for (index, &size) in sizes.iter().enumerate() {
            let entry = &ico[HEADER_BYTES + ENTRY_BYTES * index..][..ENTRY_BYTES];
            assert_eq!(entry[..8], [size as u8, size as u8, 0, 0, 1, 0, 32, 0]);
            let (length, offset) = (dword(entry, 8), dword(entry, 12));
            assert_eq!(offset, expected_offset);
            assert_eq!(ico[offset..offset + length], image(size, mark::ACCENT_RGB));
            expected_offset += length;
        }
        assert_eq!(ico.len(), expected_offset);
    }

    #[test]
    fn large_images_are_png_and_small_ones_bitmaps() {
        assert_eq!(image(64, 0)[..4], 40u32.to_le_bytes());
        let large = image(256, 0);
        assert_eq!(large[..4], [0x89, b'P', b'N', b'G']);
        assert!(large.len() < 16 * 1024, "the flat artwork should compress well, got {} bytes", large.len());
    }

    #[test]
    fn empty_size_list_is_an_empty_icon() {
        assert_eq!(file(&[], 0), [0, 0, 1, 0, 0, 0]);
    }
}
