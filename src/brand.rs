//! Application identity: name, colours and the logo mark.

// The `.ico` and PNG writers run in the build script; they are compiled here
// only so that their unit tests run with the rest of the crate.
#[cfg(test)]
mod ico;
mod mark;
#[cfg(test)]
mod png;

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;

use windows::Win32::Foundation::{COLORREF, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiAlphaBlend, SelectObject, AC_SRC_ALPHA,
    AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HDC,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconFromResourceEx, HICON, LR_DEFAULTCOLOR};

pub const APP_NAME: &str = "RS-CPUZ";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Accent colour as a GDI `COLORREF` (0x00BBGGRR).
pub const ACCENT: COLORREF = COLORREF(mark::ACCENT_RGB.swap_bytes() >> 8);

/// Largest edge the mark is rasterised at. Bigger targets are stretched from this size.
const MAX_RASTER: i32 = 512;

/// Icon format version expected by `CreateIconFromResourceEx`.
const ICON_FORMAT: u32 = 0x0003_0000;

/// Draws the logo mark scaled to fit `bounds`.
///
/// The mark keeps its square shape, is centred in `bounds` and is blended in
/// `color` over whatever the device context already holds.
pub fn draw_mark(hdc: HDC, bounds: RECT, color: COLORREF) {
    let width = bounds.right.saturating_sub(bounds.left);
    let height = bounds.bottom.saturating_sub(bounds.top);
    let side = width.min(height);
    if side <= 0 {
        return;
    }
    let (x, y) = (bounds.left + (width - side) / 2, bounds.top + (height - side) / 2);
    let raster = side.min(MAX_RASTER);
    let pixels = premultiplied(&mark::coverage(raster as u32), color);
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: raster,
            biHeight: -raster,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };

    // SAFETY: every GDI object created here is checked before use and released
    // before returning. `bits` points to the `raster * raster` 32-bit pixels of
    // the new bitmap, which is exactly the length of `pixels`.
    unsafe {
        let memory = CreateCompatibleDC(hdc);
        if memory.is_invalid() {
            return;
        }
        let mut bits: *mut c_void = ptr::null_mut();
        if let Ok(bitmap) = CreateDIBSection(memory, &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            if !bits.is_null() {
                ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u32>(), pixels.len());
                let previous = SelectObject(memory, bitmap);
                let _ = GdiAlphaBlend(hdc, x, y, side, side, memory, 0, 0, raster, raster, blend);
                SelectObject(memory, previous);
            }
            let _ = DeleteObject(bitmap);
        }
        let _ = DeleteDC(memory);
    }
}

/// Creates the application icon at `size` x `size` pixels. The caller owns the handle.
///
/// Returns a null handle when `size` is not positive or the icon cannot be created.
pub fn create_icon(size: i32) -> HICON {
    if size <= 0 {
        return HICON::default();
    }
    let side = size.min(MAX_RASTER);
    let image = mark::icon_image(side as u32, mark::ACCENT_RGB);
    // SAFETY: `image` is a complete icon image; the call copies what it needs from it.
    unsafe { CreateIconFromResourceEx(&image, TRUE, ICON_FORMAT, side, side, LR_DEFAULTCOLOR) }
        .unwrap_or_default()
}

/// Turns coverage values into the premultiplied BGRA pixels `GdiAlphaBlend` expects.
fn premultiplied(coverage: &[u8], color: COLORREF) -> Vec<u32> {
    let [red, green, blue, _] = color.0.to_le_bytes();
    coverage
        .iter()
        .map(|&alpha| {
            let scale = |channel: u8| (u32::from(channel) * u32::from(alpha) + 127) / 255;
            u32::from(alpha) << 24 | scale(red) << 16 | scale(green) << 8 | scale(blue)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Gdi::{GdiFlush, HBITMAP, HGDIOBJ};
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, DrawIconEx, DI_NORMAL};

    const WHITE: u32 = 0x00FF_FFFF;

    /// An off-screen 32-bit drawing surface, initially white.
    struct Surface {
        dc: HDC,
        bitmap: HBITMAP,
        previous: HGDIOBJ,
        bits: *mut u32,
        width: i32,
    }

    impl Surface {
        fn new(width: i32, height: i32) -> Self {
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            // SAFETY: the handles are checked right below and released in `drop`;
            // `bits` covers `width * height` pixels of the new bitmap.
            unsafe {
                let dc = CreateCompatibleDC(None);
                assert!(!dc.is_invalid());
                let mut bits: *mut c_void = ptr::null_mut();
                let bitmap =
                    CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, None, 0).expect("DIB section");
                assert!(!bits.is_null());
                let bits = bits.cast::<u32>();
                std::slice::from_raw_parts_mut(bits, (width * height) as usize).fill(WHITE);
                let previous = SelectObject(dc, bitmap);
                Surface { dc, bitmap, previous, bits, width }
            }
        }

        /// The pixel at (`x`, `y`) as `0xRRGGBB`.
        fn pixel(&self, x: i32, y: i32) -> u32 {
            // SAFETY: the tests only read inside the surface, after GDI has finished drawing.
            unsafe {
                let _ = GdiFlush();
                *self.bits.add((y * self.width + x) as usize) & 0x00FF_FFFF
            }
        }
    }

    impl Drop for Surface {
        fn drop(&mut self) {
            // SAFETY: the handles were created in `new` and are released exactly once.
            unsafe {
                SelectObject(self.dc, self.previous);
                let _ = DeleteObject(self.bitmap);
                let _ = DeleteDC(self.dc);
            }
        }
    }

    /// `color` (`0xRRGGBB`) at opacity `alpha` over white.
    fn over_white(color: u32, alpha: u8) -> [i32; 3] {
        let [blue, green, red, _] = color.to_le_bytes();
        [red, green, blue]
            .map(|c| (i32::from(c) * i32::from(alpha) + 255 * (255 - i32::from(alpha)) + 127) / 255)
    }

    fn channels(pixel: u32) -> [i32; 3] {
        let [blue, green, red, _] = pixel.to_le_bytes();
        [red, green, blue].map(i32::from)
    }

    fn assert_close(actual: u32, expected: [i32; 3], context: &str) {
        let actual = channels(actual);
        assert!(
            actual.iter().zip(expected).all(|(a, e)| (a - e).abs() <= 2),
            "{context}: {actual:?} vs {expected:?}"
        );
    }

    #[test]
    fn accent_is_rust_orange() {
        assert_eq!(ACCENT, COLORREF(0x002B_42CE));
    }

    #[test]
    fn premultiplies_by_coverage() {
        let pixels = premultiplied(&[0, 255, 128], COLORREF(0x0010_80FF));
        assert_eq!(pixels, [0, 0xFFFF_8010, 0x8080_4008]);
    }

    #[test]
    fn draws_the_mark_centred_and_blended() {
        let surface = Surface::new(48, 32);
        draw_mark(surface.dc, RECT { left: 0, top: 0, right: 48, bottom: 32 }, COLORREF(0x0000_00FF));

        // The 32-pixel mark sits in the middle, leaving 8 untouched pixels on each side.
        let mask = mark::coverage(32);
        for y in 0..32 {
            for x in 0..48 {
                let alpha = if (8..40).contains(&x) { mask[(y * 32 + x - 8) as usize] } else { 0 };
                assert_close(surface.pixel(x, y), over_white(0xFF_00_00, alpha), &format!("pixel {x},{y}"));
            }
        }
    }

    #[test]
    fn stretches_the_mark_beyond_the_largest_raster() {
        let side = MAX_RASTER + 88;
        let surface = Surface::new(side, side);
        draw_mark(surface.dc, RECT { left: 0, top: 0, right: side, bottom: side }, COLORREF(0));
        assert_eq!(surface.pixel(side / 4, side / 4), 0, "body");
        assert_eq!(surface.pixel(side / 2, side / 32), 0, "centre pin");
        assert_eq!(surface.pixel(side / 2, side / 2), WHITE, "hole");
        assert_eq!(surface.pixel(side / 32, side / 32), WHITE, "corner");
        assert_eq!(surface.pixel(side - 1 - side / 4, side - 1 - side / 4), 0, "far side of the body");
    }

    #[test]
    fn ignores_empty_bounds() {
        let surface = Surface::new(8, 8);
        draw_mark(surface.dc, RECT { left: 4, top: 4, right: 4, bottom: 8 }, COLORREF(0));
        draw_mark(surface.dc, RECT { left: 6, top: 6, right: 2, bottom: 2 }, COLORREF(0));
        draw_mark(surface.dc, RECT { left: i32::MIN, top: 0, right: i32::MAX, bottom: 0 }, COLORREF(0));
        assert!((0..8).all(|i| surface.pixel(i, i) == WHITE));
    }

    #[test]
    fn icon_shows_the_mark_in_the_accent_colour() {
        let icon = create_icon(32);
        assert!(!icon.is_invalid());
        let surface = Surface::new(32, 32);
        // SAFETY: both handles are valid for the duration of the calls.
        unsafe {
            DrawIconEx(surface.dc, 0, 0, icon, 32, 32, 0, None, DI_NORMAL).expect("DrawIconEx");
            DestroyIcon(icon).expect("DestroyIcon");
        }

        let mask = mark::coverage(32);
        for y in 0..32 {
            for x in 0..32 {
                let expected = over_white(mark::ACCENT_RGB, mask[(y * 32 + x) as usize]);
                assert_close(surface.pixel(x, y), expected, &format!("pixel {x},{y}"));
            }
        }
    }

    #[test]
    fn rejects_non_positive_icon_sizes() {
        assert!(create_icon(0).is_invalid());
        assert!(create_icon(-16).is_invalid());
    }
}
