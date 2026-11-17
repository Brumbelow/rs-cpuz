//! The GDI objects the renderer draws with: an off-screen surface, the fonts and a painter.
//!
//! Fills and lines go through the stock DC brush and pen, so the fonts and the
//! surface are the only objects that are ever created; both are released on drop.

use std::ffi::c_void;

use windows::Win32::Foundation::{COLORREF, HANDLE, POINT, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateFontIndirectW, DeleteDC, DeleteObject, DrawTextW, FillRect,
    GdiFlush, GetStockObject, GetTextExtentPoint32W, GetTextFaceW, GetTextMetricsW, LineTo, MoveToEx,
    Polygon, SelectObject, SetBkMode, SetDCBrushColor, SetDCPenColor, SetTextColor, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY, DC_BRUSH, DC_PEN, DEFAULT_CHARSET, DIB_RGB_COLORS,
    DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOCLIP, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER,
    FW_NORMAL, FW_SEMIBOLD, HBITMAP, HBRUSH, HDC, HFONT, HGDIOBJ, LOGFONTW, TEXTMETRICW, TRANSPARENT,
};

use crate::config::Config;

use super::geom::Scale;
use super::theme::Theme;

/// Typeface used when the settings file names none; `FALLBACK_FACE` replaces it where it is missing.
const DEFAULT_FACE: &str = "Segoe UI";
const FALLBACK_FACE: &str = "Tahoma";
/// Pixel heights at 96 DPI of the fonts that are not configurable.
const TITLE_FONT_SIZE: i32 = 17;
const LARGE_FONT_SIZE: i32 = 30;

/// Encodes `text` as NUL-terminated UTF-16.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A 32-bit device-independent bitmap selected into its own memory DC.
pub struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u8,
    width: i32,
    height: i32,
}

impl Surface {
    /// Creates a `width` x `height` surface; `None` when GDI refuses or the size is empty.
    pub fn new(width: i32, height: i32) -> Option<Surface> {
        if width <= 0 || height <= 0 || width > 16_384 || height > 16_384 {
            return None;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: plain GDI calls; every handle is checked before use and released on failure.
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return None;
            }
            let mut bits: *mut c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, HANDLE::default(), 0);
            match bitmap {
                Ok(bitmap) if !bits.is_null() => {
                    let previous = SelectObject(dc, bitmap);
                    Some(Surface { dc, bitmap, previous, bits: bits.cast(), width, height })
                }
                Ok(bitmap) => {
                    let _ = DeleteObject(bitmap);
                    let _ = DeleteDC(dc);
                    None
                }
                Err(_) => {
                    let _ = DeleteDC(dc);
                    None
                }
            }
        }
    }

    pub fn dc(&self) -> HDC {
        self.dc
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    /// The pixels as bottom-up BGRX rows, the layout of a 32-bit `BI_RGB` bitmap.
    pub fn pixels(&self) -> &[u8] {
        let len = self.width as usize * self.height as usize * 4;
        // SAFETY: `bits` points at the DIB section's `width * height * 4` bytes, which live as
        // long as `self`; GdiFlush makes pending drawing visible in that memory.
        unsafe {
            let _ = GdiFlush();
            std::slice::from_raw_parts(self.bits, len)
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

/// The fonts of the user interface.
///
/// The dialog geometry was laid out for a narrow 8 pt dialog font, so the label
/// and value fonts come with horizontally condensed variants for text that
/// would otherwise not fit its box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontId {
    /// Labels, captions and buttons.
    Label,
    LabelNarrow,
    /// Semi-bold label text: the selected tab and the titles of the tool windows.
    Strong,
    /// Semi-bold text of the value boxes.
    Value,
    ValueNarrow,
    /// The value font condensed once more, the last step before text is clipped.
    ValueTight,
    /// Window titles.
    Title,
    /// The application name on the splash and the About page.
    Large,
}

/// The label font and its condensed variant.
pub const LABEL_FONTS: [FontId; 2] = [FontId::Label, FontId::LabelNarrow];
/// The value font and its condensed variants, widest first.
pub const VALUE_FONTS: [FontId; 3] = [FontId::Value, FontId::ValueNarrow, FontId::ValueTight];

const FONT_COUNT: usize = FontId::Large as usize + 1;
/// Text width of the condensed variants, in percent of the natural width. The font
/// mapper only offers whole character widths, so a variant lands near its target
/// rather than on it; the two targets are far enough apart to give different fonts.
const NARROW_PERCENT: i32 = 88;
const TIGHT_PERCENT: i32 = 74;
/// Text measured to compare the widths of font variants.
const WIDTH_SAMPLE: &str = "Sample 0123456789 MHz";

/// A memory DC for measuring fonts, deleted on drop.
struct ScratchDc(HDC);

impl ScratchDc {
    fn new() -> Option<ScratchDc> {
        // SAFETY: creating a memory DC has no preconditions.
        let dc = unsafe { CreateCompatibleDC(None) };
        (!dc.is_invalid()).then_some(ScratchDc(dc))
    }

    /// Runs `measure` with `font` selected and restores the previous font.
    fn with_font<R>(&self, font: HFONT, measure: impl FnOnce(HDC) -> R) -> R {
        // SAFETY: GDI validates both handles; the previous font is selected back before returning.
        unsafe {
            let previous = SelectObject(self.0, font);
            let result = measure(self.0);
            SelectObject(self.0, previous);
            result
        }
    }
}

impl Drop for ScratchDc {
    fn drop(&mut self) {
        // SAFETY: the DC was created in `new` and nothing stays selected into it.
        unsafe {
            let _ = DeleteDC(self.0);
        }
    }
}

/// Typeface, cell height and weight of a font.
#[derive(Clone, Copy)]
struct FontSpec<'a> {
    face: &'a str,
    height: i32,
    weight: u32,
}

impl FontSpec<'_> {
    /// Creates the font; `width` is the average character width or 0 for the natural one.
    /// The handle may be invalid when GDI is out of resources.
    fn create(self, width: i32) -> HFONT {
        let mut font = LOGFONTW {
            lfHeight: self.height.max(1),
            lfWidth: width.max(0),
            lfWeight: self.weight as i32,
            lfCharSet: DEFAULT_CHARSET,
            lfQuality: CLEARTYPE_QUALITY,
            ..Default::default()
        };
        for (slot, unit) in font.lfFaceName.iter_mut().take(31).zip(self.face.encode_utf16()) {
            *slot = unit;
        }
        // SAFETY: `font` is a fully initialised LOGFONTW.
        unsafe { CreateFontIndirectW(&font) }
    }
}

fn delete_font(font: HFONT) {
    if !font.is_invalid() {
        // SAFETY: fonts are only deleted while not selected into any DC.
        unsafe {
            let _ = DeleteObject(font);
        }
    }
}

/// The font set for one DPI.
pub struct Fonts {
    handles: [HFONT; FONT_COUNT],
}

impl Fonts {
    /// Creates the fonts from the settings, scaled for `scale`.
    pub fn new(cfg: &Config, scale: Scale) -> Fonts {
        let dc = ScratchDc::new();
        let label_face = resolve_face(dc.as_ref(), &cfg.label_font_name);
        let value_face = resolve_face(dc.as_ref(), &cfg.text_font_name);
        let size = |px: u32| scale.px(i32::try_from(px).unwrap_or(14));
        let label = FontSpec { face: &label_face, height: size(cfg.label_font_size), weight: FW_NORMAL.0 };
        let value = FontSpec { face: &value_face, height: size(cfg.text_font_size), weight: FW_SEMIBOLD.0 };
        let heading =
            |height: i32| FontSpec { face: &label_face, height: scale.px(height), weight: FW_SEMIBOLD.0 };

        let mut handles = [HFONT::default(); FONT_COUNT];
        let mut set = |id: FontId, font: HFONT| handles[id as usize] = font;
        set(FontId::Label, label.create(0));
        set(FontId::LabelNarrow, condensed(dc.as_ref(), label, NARROW_PERCENT));
        set(FontId::Strong, FontSpec { weight: FW_SEMIBOLD.0, ..label }.create(0));
        set(FontId::Value, value.create(0));
        set(FontId::ValueNarrow, condensed(dc.as_ref(), value, NARROW_PERCENT));
        set(FontId::ValueTight, condensed(dc.as_ref(), value, TIGHT_PERCENT));
        set(FontId::Title, heading(TITLE_FONT_SIZE).create(0));
        set(FontId::Large, heading(LARGE_FONT_SIZE).create(0));
        Fonts { handles }
    }

    fn get(&self, id: FontId) -> HFONT {
        self.handles[id as usize]
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        for font in self.handles {
            delete_font(font);
        }
    }
}

/// Typefaces to try for a configured name, in order of preference.
fn face_candidates(configured: &str) -> Vec<&str> {
    let configured = configured.trim();
    let mut faces = Vec::with_capacity(3);
    if !configured.is_empty() {
        faces.push(configured);
    }
    faces.push(DEFAULT_FACE);
    faces.push(FALLBACK_FACE);
    faces
}

/// The first candidate typeface that is installed.
fn resolve_face(dc: Option<&ScratchDc>, configured: &str) -> String {
    let candidates = face_candidates(configured);
    let found = dc.and_then(|dc| candidates.iter().copied().find(|face| face_is_installed(dc, face)));
    found.unwrap_or(FALLBACK_FACE).to_string()
}

/// Whether the font mapper resolves `face` to itself rather than to a substitute.
fn face_is_installed(dc: &ScratchDc, face: &str) -> bool {
    let font = FontSpec { face, height: 14, weight: FW_NORMAL.0 }.create(0);
    if font.is_invalid() {
        return false;
    }
    let mut name = [0u16; 64];
    // SAFETY: `hdc` is the scratch DC and `name` is a writable buffer of the stated length.
    let len = dc.with_font(font, |hdc| unsafe { GetTextFaceW(hdc, Some(&mut name)) });
    delete_font(font);
    let len = usize::try_from(len).unwrap_or(0).min(name.len());
    let mapped = String::from_utf16_lossy(&name[..len]);
    mapped.trim_end_matches('\0').eq_ignore_ascii_case(face)
}

/// Whether the default typeface is installed. Tests that measure text are written
/// for its metrics and skip themselves on a system without it.
#[cfg(test)]
pub fn has_default_face() -> bool {
    ScratchDc::new().is_some_and(|dc| face_is_installed(&dc, DEFAULT_FACE))
}

/// Average character width of `font` in pixels; 0 when it cannot be measured.
fn average_width(dc: &ScratchDc, font: HFONT) -> i32 {
    let mut metrics = TEXTMETRICW::default();
    // SAFETY: `hdc` is the scratch DC and `metrics` is a valid TEXTMETRICW.
    let measured = dc.with_font(font, |hdc| unsafe { GetTextMetricsW(hdc, &mut metrics) });
    if measured.as_bool() {
        metrics.tmAveCharWidth
    } else {
        0
    }
}

/// Width of the sample text in `font`; 0 when it cannot be measured.
fn sample_width(dc: &ScratchDc, font: HFONT) -> i32 {
    let sample: Vec<u16> = WIDTH_SAMPLE.encode_utf16().collect();
    let mut size = SIZE::default();
    // SAFETY: `hdc` is the scratch DC; the string and the SIZE outlive the call.
    let measured = dc.with_font(font, |hdc| unsafe { GetTextExtentPoint32W(hdc, &sample, &mut size) });
    if measured.as_bool() {
        size.cx
    } else {
        0
    }
}

/// Chooses the character width whose sample text is closest to `percent` of the
/// `natural` sample width. `candidates` pairs a character width with the sample
/// width it produced; only candidates narrower than the natural font qualify.
fn pick_width(natural: i32, percent: i32, candidates: &[(i32, i32)]) -> Option<i32> {
    candidates
        .iter()
        .filter(|&&(_, sample)| sample > 0 && sample < natural)
        .min_by_key(|&&(width, sample)| ((sample * 100 - natural * percent).abs(), width))
        .map(|&(width, _)| width)
}

/// A horizontally condensed variant of a font, about `percent` of its natural width.
///
/// The font mapper takes an average character width, which does not translate
/// into a text width directly, so every width below the natural one is tried
/// and measured. Without a DC the natural font is returned.
fn condensed(dc: Option<&ScratchDc>, spec: FontSpec, percent: i32) -> HFONT {
    let Some(dc) = dc else { return spec.create(0) };
    let natural_font = spec.create(0);
    let natural = sample_width(dc, natural_font);
    let widest = average_width(dc, natural_font);
    delete_font(natural_font);
    let candidates: Vec<(i32, i32)> = (1..=widest)
        .map(|width| {
            let font = spec.create(width);
            let sample = sample_width(dc, font);
            delete_font(font);
            (width, sample)
        })
        .collect();
    spec.create(pick_width(natural, percent, &candidates).unwrap_or(0))
}

/// Horizontal placement of a line of text inside its rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Centre,
    Right,
}

/// What happens to text wider than its rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    /// Clip with an end ellipsis.
    Ellipsis,
    /// Let the text run past the rectangle.
    Visible,
}

/// Drawing primitives bound to a target DC, a scale, a font set and a palette.
#[derive(Clone, Copy)]
pub struct Painter<'a> {
    pub hdc: HDC,
    pub scale: Scale,
    pub fonts: &'a Fonts,
    pub theme: &'a Theme,
}

impl Painter<'_> {
    pub fn fill(&self, r: RECT, color: COLORREF) {
        if r.right <= r.left || r.bottom <= r.top {
            return;
        }
        // SAFETY: `hdc` is a valid DC for the lifetime of the painter; the DC brush is a stock object.
        unsafe {
            SetDCBrushColor(self.hdc, color);
            FillRect(self.hdc, &r, HBRUSH(GetStockObject(DC_BRUSH).0));
        }
    }

    /// Thickness of frames and borders: one pixel at 96 DPI.
    pub fn hairline(&self) -> i32 {
        self.scale.px(1)
    }

    /// Draws a border just inside `r`.
    pub fn outline(&self, r: RECT, color: COLORREF) {
        let t = self.hairline();
        self.fill(RECT { bottom: r.top + t, ..r }, color);
        self.fill(RECT { top: r.bottom - t, ..r }, color);
        self.fill(RECT { right: r.left + t, ..r }, color);
        self.fill(RECT { left: r.right - t, ..r }, color);
    }

    /// Fills `r` and draws a border just inside it.
    pub fn panel(&self, r: RECT, fill: COLORREF, border: COLORREF) {
        self.fill(r, fill);
        self.outline(r, border);
    }

    /// Draws one line of text, vertically centred in `r`.
    pub fn text(&self, r: RECT, text: &str, font: FontId, color: COLORREF, align: Align, overflow: Overflow) {
        if text.is_empty() {
            return;
        }
        let mut buffer: Vec<u16> = text.encode_utf16().collect();
        let mut rect = r;
        let horizontal = match align {
            Align::Left => DT_LEFT,
            Align::Centre => DT_CENTER,
            Align::Right => DT_RIGHT,
        };
        let clipping = match overflow {
            Overflow::Ellipsis => DT_END_ELLIPSIS,
            Overflow::Visible => DT_NOCLIP,
        };
        // SAFETY: `hdc` is valid; the previously selected font is restored so ours is never
        // left selected when it is deleted.
        unsafe {
            let previous = SelectObject(self.hdc, self.fonts.get(font));
            SetBkMode(self.hdc, TRANSPARENT);
            SetTextColor(self.hdc, color);
            DrawTextW(
                self.hdc,
                &mut buffer,
                &mut rect,
                DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | horizontal | clipping,
            );
            SelectObject(self.hdc, previous);
        }
    }

    /// Width and height of `text` in pixels.
    pub fn text_size(&self, text: &str, font: FontId) -> (i32, i32) {
        let buffer: Vec<u16> =
            if text.is_empty() { vec![u16::from(b' ')] } else { text.encode_utf16().collect() };
        let mut size = SIZE::default();
        // SAFETY: `hdc` is valid and the previously selected font is restored.
        unsafe {
            let previous = SelectObject(self.hdc, self.fonts.get(font));
            let _ = GetTextExtentPoint32W(self.hdc, &buffer, &mut size);
            SelectObject(self.hdc, previous);
        }
        (if text.is_empty() { 0 } else { size.cx }, size.cy)
    }

    pub fn text_width(&self, text: &str, font: FontId) -> i32 {
        self.text_size(text, font).0
    }

    /// The first of `fonts` in which `text` is at most `available` pixels wide; the last one otherwise.
    pub fn fit(&self, text: &str, fonts: &[FontId], available: i32) -> FontId {
        let fallback = fonts.last().copied().unwrap_or(FontId::Label);
        fonts.iter().copied().find(|&font| self.text_width(text, font) <= available).unwrap_or(fallback)
    }

    /// Fills a convex polygon.
    pub fn polygon(&self, points: &[POINT], color: COLORREF) {
        // SAFETY: `hdc` is valid; brush and pen are stock objects and the previous ones are restored.
        unsafe {
            let brush = SelectObject(self.hdc, GetStockObject(DC_BRUSH));
            let pen = SelectObject(self.hdc, GetStockObject(DC_PEN));
            SetDCBrushColor(self.hdc, color);
            SetDCPenColor(self.hdc, color);
            let _ = Polygon(self.hdc, points);
            SelectObject(self.hdc, pen);
            SelectObject(self.hdc, brush);
        }
    }

    /// Draws a line `thickness` pixels wide by stacking one-pixel lines horizontally.
    pub fn stroke(&self, from: POINT, to: POINT, thickness: i32, color: COLORREF) {
        // SAFETY: `hdc` is valid; the pen is a stock object and the previous one is restored.
        unsafe {
            let pen = SelectObject(self.hdc, GetStockObject(DC_PEN));
            SetDCPenColor(self.hdc, color);
            for offset in 0..thickness.max(1) {
                let _ = MoveToEx(self.hdc, from.x + offset, from.y, None);
                let _ = LineTo(self.hdc, to.x + offset, to.y);
            }
            SelectObject(self.hdc, pen);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_strings_are_terminated() {
        assert_eq!(wide("ab"), [97, 98, 0]);
        assert_eq!(wide(""), [0]);
    }

    #[test]
    fn face_candidates_end_with_the_fallbacks() {
        assert_eq!(face_candidates(""), [DEFAULT_FACE, FALLBACK_FACE]);
        assert_eq!(face_candidates("  Consolas "), ["Consolas", DEFAULT_FACE, FALLBACK_FACE]);
    }

    #[test]
    fn surface_starts_black_and_reports_its_size() {
        // Sessions without a usable GDI (some service contexts) cannot create surfaces at all.
        let Some(surface) = Surface::new(8, 4) else { return };
        assert_eq!((surface.width(), surface.height()), (8, 4));
        assert_eq!(surface.pixels().len(), 8 * 4 * 4);
        assert!(surface.pixels().iter().all(|&byte| byte == 0));
        assert!(Surface::new(0, 4).is_none());
        assert!(Surface::new(4, -1).is_none());
    }

    #[test]
    fn fills_land_in_the_pixel_buffer() {
        let Some(surface) = Surface::new(4, 4) else { return };
        let fonts = Fonts::new(&Config::default(), Scale::new(96));
        let theme = Theme::new(&Config::default());
        let painter = Painter { hdc: surface.dc(), scale: Scale::new(96), fonts: &fonts, theme: &theme };
        painter.fill(RECT { left: 0, top: 0, right: 4, bottom: 4 }, COLORREF(0x0030_2010));
        let pixels = surface.pixels();
        assert_eq!(&pixels[..3], [0x30, 0x20, 0x10]);
        assert!(painter.text_width("RS-CPUZ", FontId::Label) > 0);
        assert_eq!(painter.text_width("", FontId::Label), 0);
        assert_eq!(painter.hairline(), 1);
    }

    #[test]
    fn condensed_width_is_the_closest_narrower_candidate() {
        // Character width -> sample width, as measured for a 7 px wide natural font of 150 px.
        let candidates = [(1, 30), (2, 52), (3, 77), (4, 105), (5, 132), (6, 150), (7, 150)];
        assert_eq!(pick_width(150, 88, &candidates), Some(5));
        assert_eq!(pick_width(150, 74, &candidates), Some(4));
        assert_eq!(pick_width(150, 100, &candidates), Some(5), "never as wide as the natural font");
        assert_eq!(pick_width(150, 1, &candidates), Some(1));
        assert_eq!(pick_width(150, 88, &[(6, 150), (7, 160)]), None);
        assert_eq!(pick_width(150, 88, &[(3, 0)]), None, "unmeasurable candidates are skipped");
        assert_eq!(pick_width(100, 80, &[(4, 70), (5, 90)]), Some(4), "ties go to the narrower font");
    }

    #[test]
    fn fitting_prefers_the_natural_font() {
        let Some(surface) = Surface::new(4, 4) else { return };
        let fonts = Fonts::new(&Config::default(), Scale::new(96));
        let theme = Theme::new(&Config::default());
        let painter = Painter { hdc: surface.dc(), scale: Scale::new(96), fonts: &fonts, theme: &theme };
        let text = "x 51.5 (4.0 - 53.5)";
        let natural = painter.text_width(text, FontId::Value);
        assert_eq!(painter.fit(text, &VALUE_FONTS, natural), FontId::Value);
        assert_eq!(painter.fit(text, &VALUE_FONTS, 0), FontId::ValueTight);
        let narrow = painter.text_width(text, FontId::ValueNarrow);
        assert!(narrow <= natural);
        assert!(painter.text_width(text, FontId::ValueTight) <= narrow);
        assert_eq!(painter.fit(text, &[], 0), FontId::Label);
    }

    #[test]
    fn value_fonts_condense_in_two_distinct_steps() {
        let Some(surface) = Surface::new(4, 4) else { return };
        if !has_default_face() {
            return;
        }
        let cfg = Config::default();
        let theme = Theme::new(&cfg);
        for dpi in [96, 120, 144, 168, 192] {
            let scale = Scale::new(dpi);
            let fonts = Fonts::new(&cfg, scale);
            let painter = Painter { hdc: surface.dc(), scale, fonts: &fonts, theme: &theme };
            let [natural, narrow, tight] = VALUE_FONTS.map(|font| painter.text_width(WIDTH_SAMPLE, font));
            assert!(tight < narrow && narrow < natural, "dpi {dpi}: {natural} / {narrow} / {tight}");
            // Whole character widths put the tightest variant near its target, never far below.
            assert!(tight * 100 >= natural * 66, "dpi {dpi}: {natural} / {tight}");
        }
    }

    #[test]
    fn strong_font_follows_the_label_settings() {
        let Some(surface) = Surface::new(4, 4) else { return };
        let scale = Scale::new(96);
        let theme = Theme::new(&Config::default());
        let measure = |cfg: &Config, font: FontId| {
            let fonts = Fonts::new(cfg, scale);
            Painter { hdc: surface.dc(), scale, fonts: &fonts, theme: &theme }.text_size("Mainboard", font)
        };
        let plain = Config::default();
        assert_eq!(measure(&plain, FontId::Strong), measure(&plain, FontId::Value));
        assert!(measure(&plain, FontId::Strong).0 > measure(&plain, FontId::Label).0, "semi-bold is wider");

        let large_values = Config { text_font_size: 28, ..Config::default() };
        assert_eq!(measure(&large_values, FontId::Strong), measure(&plain, FontId::Strong));
        assert!(measure(&large_values, FontId::Value).1 > measure(&plain, FontId::Value).1);
        let large_labels = Config { label_font_size: 28, ..Config::default() };
        assert!(measure(&large_labels, FontId::Strong).1 > measure(&plain, FontId::Strong).1);
    }
}
