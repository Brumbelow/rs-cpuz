//! Image export: `.bmp` files and the clipboard's device-independent bitmap.

use std::io;
use std::path::Path;

use windows::Win32::Foundation::{GlobalFree, HANDLE, HWND};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_DIB;

use super::gdi::Surface;

const FILE_HEADER_SIZE: usize = 14;
const INFO_HEADER_SIZE: usize = 40;
/// 96 DPI expressed in pixels per metre.
const PIXELS_PER_METRE: u32 = 3780;

/// Encodes a `BITMAPINFOHEADER` followed by the pixels: the body of a `.bmp`
/// file and the `CF_DIB` clipboard format.
///
/// `bgrx` holds bottom-up 32-bit rows. The unused fourth byte of every pixel is
/// set to `0xFF` so programs that read it as alpha see an opaque image.
/// Returns `None` when the buffer does not match the dimensions.
pub fn dib(width: u32, height: u32, bgrx: &[u8]) -> Option<Vec<u8>> {
    let size = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
    if size == 0 || bgrx.len() != size || i32::try_from(width).is_err() || i32::try_from(height).is_err() {
        return None;
    }
    let image_size = u32::try_from(size).ok()?;
    let mut out = Vec::with_capacity(INFO_HEADER_SIZE + size);
    out.extend_from_slice(&(INFO_HEADER_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&image_size.to_le_bytes());
    out.extend_from_slice(&PIXELS_PER_METRE.to_le_bytes());
    out.extend_from_slice(&PIXELS_PER_METRE.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for pixel in bgrx.chunks_exact(4) {
        out.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 0xFF]);
    }
    Some(out)
}

/// Encodes a complete `.bmp` file.
pub fn file(width: u32, height: u32, bgrx: &[u8]) -> Option<Vec<u8>> {
    let body = dib(width, height, bgrx)?;
    let total = u32::try_from(FILE_HEADER_SIZE + body.len()).ok()?;
    let mut out = Vec::with_capacity(FILE_HEADER_SIZE + body.len());
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&((FILE_HEADER_SIZE + INFO_HEADER_SIZE) as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Some(out)
}

fn dimensions(surface: &Surface) -> (u32, u32) {
    (surface.width().unsigned_abs(), surface.height().unsigned_abs())
}

/// Writes the surface to `path` as a 32-bit `.bmp` file.
pub fn save(surface: &Surface, path: &Path) -> io::Result<()> {
    let (width, height) = dimensions(surface);
    let bytes = file(width, height, surface.pixels())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "the image could not be encoded"))?;
    std::fs::write(path, bytes)
}

/// Puts the surface on the clipboard as `CF_DIB`. Returns whether the clipboard took it.
pub fn copy_to_clipboard(owner: HWND, surface: &Surface) -> bool {
    let (width, height) = dimensions(surface);
    let Some(bytes) = dib(width, height, surface.pixels()) else { return false };
    // SAFETY: the global block is `bytes.len()` long and only written while locked. Ownership
    // passes to the clipboard when SetClipboardData succeeds; otherwise the block is freed here.
    unsafe {
        let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else { return false };
        let target = GlobalLock(memory).cast::<u8>();
        if target.is_null() {
            let _ = GlobalFree(memory);
            return false;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), target, bytes.len());
        let _ = GlobalUnlock(memory);
        if OpenClipboard(owner).is_err() {
            let _ = GlobalFree(memory);
            return false;
        }
        let stored =
            EmptyClipboard().is_ok() && SetClipboardData(u32::from(CF_DIB.0), HANDLE(memory.0)).is_ok();
        if !stored {
            let _ = GlobalFree(memory);
        }
        let _ = CloseClipboard();
        stored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le32(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
    }

    #[test]
    fn dib_header_and_pixels() {
        let pixels = [1, 2, 3, 0, 4, 5, 6, 0, 7, 8, 9, 0, 10, 11, 12, 0, 13, 14, 15, 0, 16, 17, 18, 0];
        let out = dib(3, 2, &pixels).unwrap();
        assert_eq!(out.len(), 40 + 24);
        assert_eq!(le32(&out, 0), 40);
        assert_eq!(le32(&out, 4), 3);
        assert_eq!(le32(&out, 8), 2);
        assert_eq!(&out[12..16], [1, 0, 32, 0]);
        assert_eq!(le32(&out, 16), 0, "BI_RGB");
        assert_eq!(le32(&out, 20), 24);
        assert_eq!(&out[40..48], [1, 2, 3, 0xFF, 4, 5, 6, 0xFF]);
    }

    #[test]
    fn file_header_points_at_the_pixels() {
        let out = file(1, 1, &[9, 8, 7, 0]).unwrap();
        assert_eq!(&out[..2], b"BM");
        assert_eq!(le32(&out, 2) as usize, out.len());
        assert_eq!(le32(&out, 10), 54);
        assert_eq!(&out[54..], [9, 8, 7, 0xFF]);
    }

    #[test]
    fn mismatched_buffers_are_rejected() {
        assert!(dib(2, 2, &[0; 15]).is_none());
        assert!(dib(0, 2, &[]).is_none());
        assert!(file(2, 1, &[0; 4]).is_none());
        assert!(dib(u32::MAX, 1, &[0; 4]).is_none());
    }
}
