//! The custom-drawn controls shared by every window.

use windows::Win32::Foundation::{COLORREF, POINT, RECT};

use super::format;
use super::gdi::{Align, FontId, Overflow, Painter, LABEL_FONTS, VALUE_FONTS};
use super::geom::{centred_square, height, inset, width};
use super::state::BarView;
use super::theme::blend;

/// Interaction state of a clickable control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    Normal,
    Hot,
    Pressed,
    Disabled,
}

impl Mood {
    /// The mood of a control given what the mouse is doing to it.
    pub fn of(enabled: bool, hot: bool, pressed: bool) -> Mood {
        match (enabled, hot, pressed) {
            (false, _, _) => Mood::Disabled,
            (true, true, true) => Mood::Pressed,
            (true, false, false) => Mood::Normal,
            (true, _, _) => Mood::Hot,
        }
    }
}

/// Horizontal padding of text inside boxes, in pixels at 96 DPI.
const TEXT_PAD: i32 = 2;
/// Width of the chevron zone at the right end of selectors and drop-down buttons, at 96 DPI.
pub const CHEVRON_ZONE: i32 = 16;

/// `r` without the horizontal text padding.
fn padded(p: &Painter, r: RECT) -> RECT {
    RECT { left: r.left + p.scale.px(TEXT_PAD), right: r.right - p.scale.px(TEXT_PAD), ..r }
}

/// One-pixel frame of a group.
pub fn frame(p: &Painter, r: RECT) {
    p.outline(r, p.theme.frame);
}

/// Caption of a group, horizontally centred in `r` and vertically centred on `edge`,
/// the frame line it sits on; the line behind the text is erased.
pub fn caption(p: &Painter, r: RECT, edge: i32, text: &str) {
    if text.is_empty() {
        return;
    }
    let (text_width, text_height) = p.text_size(text, FontId::Label);
    let left = if text_width > width(r) { r.left } else { r.left + (width(r) - text_width) / 2 };
    let top = edge - text_height / 2;
    let pad = p.scale.px(3);
    let area = RECT { left, top, right: left + text_width, bottom: top + text_height };
    // Only the frame line is erased: whatever else the text overlaps, such as the
    // border of the page above the first group, stays as it is.
    let line = p.hairline();
    let line_top = edge - line / 2;
    let gap = RECT { left: area.left - pad, top: line_top, right: area.right + pad, bottom: line_top + line };
    p.fill(gap, p.theme.panel);
    p.text(area, text, FontId::Label, p.theme.caption, Align::Left, Overflow::Visible);
}

/// Right-aligned label. Text too wide for `r` is condensed and may then extend to the left.
pub fn label(p: &Painter, r: RECT, text: &str, dim: bool) {
    let color = if dim { p.theme.dim } else { p.theme.label };
    let font = p.fit(text, &LABEL_FONTS, width(r));
    p.text(r, text, font, color, Align::Right, Overflow::Visible);
}

/// One line of text in the label colour, condensed when it is too wide for `r`.
pub fn note(p: &Painter, r: RECT, text: &str, align: Align) {
    let font = p.fit(text, &LABEL_FONTS, width(r));
    p.text(r, text, font, p.theme.label, align, Overflow::Ellipsis);
}

/// One line of value text inside `r`, condensed as far as needed before it is clipped.
fn value_text(p: &Painter, r: RECT, text: &str, color: COLORREF, align: Align) {
    let font = p.fit(text, &VALUE_FONTS, width(r));
    p.text(r, text, font, color, align, Overflow::Ellipsis);
}

/// One line of value text without a box, for table cells.
pub fn plain_value(p: &Painter, r: RECT, text: &str, align: Align) {
    value_text(p, r, text, p.theme.value, align);
}

/// Where the text of a value box goes and in which font, and whether all of it fits.
///
/// The fonts are tried from the natural one to the most condensed. A condensed
/// font that only just overflows may use the padding as well, right up to the
/// border, before the next one is tried or the text is clipped.
fn value_fit(p: &Painter, r: RECT, text: &str) -> (RECT, FontId, bool) {
    let roomy = padded(p, r);
    let tight = RECT { left: r.left + p.hairline(), right: r.right - p.hairline(), ..r };
    for font in VALUE_FONTS {
        let needed = p.text_width(text, font);
        if needed <= width(roomy) {
            return (roomy, font, true);
        }
        if font != FontId::Value && needed <= width(tight) {
            return (tight, font, true);
        }
    }
    (roomy, FontId::ValueTight, false)
}

/// Value box with one line of text, condensed as far as needed before it is clipped.
pub fn value(p: &Painter, r: RECT, text: &str, align: Align) {
    p.panel(r, p.theme.field, p.theme.field_border);
    let (area, font, _) = value_fit(p, r, text);
    p.text(area, text, font, p.theme.value, align, Overflow::Ellipsis);
}

/// Value box of a row that is not used on this system.
pub fn dim_value(p: &Painter, r: RECT) {
    p.panel(r, p.theme.field_dim, p.theme.field_dim_border);
}

/// The fonts of wrapped text. It stops at the first condensed variant: a long
/// text reads better shortened than squeezed into two lines of the tightest one.
const WRAPPED_FONTS: [FontId; 2] = [FontId::Value, FontId::ValueNarrow];

/// Value box whose text wraps over two lines. The condensed font is used when it makes
/// the whole text fit. Text that is still too long loses its parenthesised detail lists,
/// and only then is the second line clipped with an ellipsis.
pub fn wrapped_value(p: &Painter, r: RECT, text: &str) {
    p.panel(r, p.theme.field, p.theme.field_border);
    let inner = padded(p, r);
    let available = width(inner);
    let wrap = |text: &str, font: FontId| format::wrap_two_lines(text, available, |s| p.text_width(s, font));
    let fitting = |text: &str| {
        WRAPPED_FONTS.into_iter().find(|&font| p.text_width(&wrap(text, font).1, font) <= available)
    };
    let brief = format::without_detail_lists(text);
    let (text, font) = match fitting(text) {
        Some(font) => (text, font),
        None => (brief.as_str(), fitting(&brief).unwrap_or(FontId::Value)),
    };
    let (first, second) = wrap(text, font);
    if second.is_empty() {
        p.text(inner, &first, font, p.theme.value, Align::Left, Overflow::Ellipsis);
        return;
    }
    let (_, line) = p.text_size("Ag", font);
    let pitch = line.min((height(r) - 2 * p.hairline()) / 2);
    let top = r.top + (height(r) - pitch - line) / 2;
    for (text, top) in [(first, top), (second, top + pitch)] {
        let row = RECT { top, bottom: top + line, ..inner };
        p.text(row, &text, font, p.theme.value, Align::Left, Overflow::Ellipsis);
    }
}

/// Fill and border of a button or selector.
fn control_colors(p: &Painter, mood: Mood) -> (COLORREF, COLORREF) {
    let t = p.theme;
    match mood {
        Mood::Normal => (t.control, t.control_border),
        Mood::Hot => (t.control_hot, t.accent),
        Mood::Pressed => (t.control_pressed, t.accent),
        Mood::Disabled => (t.field_dim, t.field_dim_border),
    }
}

/// `color` for an enabled control, the dimmed text colour for a disabled one.
fn control_ink(p: &Painter, mood: Mood, color: COLORREF) -> COLORREF {
    if mood == Mood::Disabled {
        p.theme.dim
    } else {
        color
    }
}

/// A downward chevron centred on `centre`.
fn chevron(p: &Painter, centre: POINT, color: COLORREF) {
    let half = p.scale.px(3);
    let top = centre.y - half / 2;
    let points = [
        POINT { x: centre.x - half, y: top },
        POINT { x: centre.x + half, y: top },
        POINT { x: centre.x, y: top + half },
    ];
    p.polygon(&points, color);
}

/// Centre of the chevron zone at the right end of `r`.
fn chevron_centre(p: &Painter, r: RECT) -> POINT {
    POINT { x: r.right - p.scale.px(CHEVRON_ZONE) / 2 - p.hairline(), y: (r.top + r.bottom) / 2 }
}

/// Push button; `drop_down` adds a chevron at the right.
pub fn button(p: &Painter, r: RECT, text: &str, mood: Mood, drop_down: bool) {
    let (fill, border) = control_colors(p, mood);
    let ink = control_ink(p, mood, p.theme.chrome);
    p.panel(r, fill, border);
    let mut area = inset(r, p.hairline());
    if drop_down {
        chevron(p, chevron_centre(p, r), ink);
        let zone = p.scale.px(CHEVRON_ZONE);
        area.left += zone / 2;
        area.right -= zone / 2;
    }
    let font = p.fit(text, &LABEL_FONTS, width(area));
    p.text(area, text, font, ink, Align::Centre, Overflow::Ellipsis);
}

/// Selector: the current entry, in the value colour like the boxes it selects
/// for, and a chevron. `open` keeps the accent border while its list shows.
pub fn combo(p: &Painter, r: RECT, text: &str, mood: Mood, open: bool) {
    let (fill, border) = control_colors(p, mood);
    let border = if open { p.theme.accent } else { border };
    p.panel(r, fill, border);
    chevron(p, chevron_centre(p, r), control_ink(p, mood, p.theme.chrome));
    value_text(p, combo_text_area(p, r), text, control_ink(p, mood, p.theme.value), Align::Left);
}

/// The part of a selector that holds its text, left of the chevron zone.
fn combo_text_area(p: &Painter, r: RECT) -> RECT {
    RECT { left: r.left + p.scale.px(TEXT_PAD + 2), right: r.right - p.scale.px(CHEVRON_ZONE), ..r }
}

/// Check box drawn as the largest square inside `r`.
pub fn check(p: &Painter, r: RECT, checked: bool, mood: Mood) {
    let t = p.theme;
    let square = centred_square(r);
    let border = if mood == Mood::Normal { t.control_border } else { t.accent };
    if !checked {
        p.panel(square, t.field, border);
        return;
    }
    p.panel(square, t.accent, t.accent);
    let side = width(square);
    let at = |x: i32, y: i32| POINT { x: square.left + side * x / 12, y: square.top + side * y / 12 };
    let thickness = p.scale.px(2);
    p.stroke(at(2, 6), at(5, 9), thickness, t.on_accent);
    p.stroke(at(5, 9), at(10, 3), thickness, t.on_accent);
}

/// Progress bar with its caption centred on top.
pub fn bar(p: &Painter, r: RECT, view: &BarView) {
    if !view.visible {
        return;
    }
    p.panel(r, p.theme.field, p.theme.field_border);
    let inner = inset(r, p.hairline());
    let filled = (f64::from(width(inner)) * f64::from(view.fraction.clamp(0.0, 1.0))).round() as i32;
    let color = if view.reference { p.theme.neutral_bar } else { p.theme.accent };
    p.fill(RECT { right: inner.left + filled, ..inner }, color);
    value_text(p, inner, &view.text, p.theme.on_accent, Align::Centre);
}

/// Plain progress bar without a caption.
pub fn progress(p: &Painter, r: RECT, fraction: f32) {
    bar(p, r, &BarView { visible: true, fraction, ..BarView::default() });
}

/// Width of the scroll indicator of lists and tables, in pixels at 96 DPI.
pub const SCROLL_BAR: i32 = 4;

/// Top and bottom of the scroll thumb inside a track `span` pixels long, for a list
/// of `total` entries showing `visible` of them starting at `first`.
fn thumb_span(span: i32, first: usize, visible: usize, total: usize) -> (i32, i32) {
    let total = total.max(1) as i64;
    let at = |entry: usize| (i64::from(span) * (entry as i64).min(total) / total) as i32;
    (at(first), at(first.saturating_add(visible)))
}

/// Scroll indicator filling `track`. Nothing is drawn while every entry is visible.
pub fn scroll_bar(p: &Painter, track: RECT, first: usize, visible: usize, total: usize) {
    if total <= visible {
        return;
    }
    p.fill(track, p.theme.field_dim);
    let (top, bottom) = thumb_span(height(track), first, visible, total);
    let thumb = RECT { top: track.top + top, bottom: track.top + bottom.max(top + p.hairline()), ..track };
    p.fill(thumb, p.theme.control_border);
}

/// The glyphs of the title-strip buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Minimize,
    Close,
}

/// A borderless title-strip button.
pub fn title_button(p: &Painter, r: RECT, glyph: Glyph, mood: Mood) {
    let t = p.theme;
    let (fill, ink) = match (mood, glyph) {
        (Mood::Hot, Glyph::Close) => (Some(t.accent), t.on_accent),
        (Mood::Pressed, Glyph::Close) => (Some(blend(t.accent, t.window, 96)), t.on_accent),
        (Mood::Hot, Glyph::Minimize) => (Some(t.control_hot), t.chrome),
        (Mood::Pressed, Glyph::Minimize) => (Some(t.control_pressed), t.chrome),
        _ => (None, t.label),
    };
    if let Some(fill) = fill {
        p.fill(r, fill);
    }
    let centre = POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 };
    let half = p.scale.px(4);
    let thickness = p.hairline();
    match glyph {
        Glyph::Minimize => {
            let top = centre.y + half - thickness;
            p.fill(
                RECT { left: centre.x - half, top, right: centre.x + half + 1, bottom: top + thickness },
                ink,
            );
        }
        Glyph::Close => {
            let (left, right) = (centre.x - half, centre.x + half);
            let (top, bottom) = (centre.y - half, centre.y + half);
            p.stroke(POINT { x: left, y: top }, POINT { x: right + 1, y: bottom + 1 }, thickness, ink);
            p.stroke(POINT { x: left, y: bottom }, POINT { x: right + 1, y: top - 1 }, thickness, ink);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::model::{CoreKind, CoreSet};
    use crate::ui::gdi::{has_default_face, Fonts, Surface};
    use crate::ui::geom::Scale;
    use crate::ui::layout::{cpu, graphics, mainboard, spd, Kind, Tab};
    use crate::ui::render::control_rect;
    use crate::ui::theme::Theme;

    /// Runs `check` with a painter for the default settings at each common scale.
    /// Does nothing where text cannot be measured as the layout expects it.
    fn at_common_scales(check: impl Fn(u32, &Painter)) {
        let Some(surface) = Surface::new(4, 4) else { return };
        if !has_default_face() {
            return;
        }
        let cfg = Config::default();
        let theme = Theme::new(&cfg);
        for dpi in [96, 120, 144, 168, 192] {
            let scale = Scale::new(dpi);
            let fonts = Fonts::new(&cfg, scale);
            check(dpi, &Painter { hdc: surface.dc(), scale, fonts: &fonts, theme: &theme });
        }
    }

    /// Whether `text` is shown in full by control `id` of `tab`.
    fn shown_in_full(p: &Painter, tab: Tab, id: u16, text: &str) -> bool {
        let r = control_rect(p.scale, tab, id).expect("a control of the page");
        match tab.item(id).map(|item| item.kind) {
            Some(Kind::Combo) => {
                let area = width(combo_text_area(p, r));
                p.text_width(text, p.fit(text, &VALUE_FONTS, area)) <= area
            }
            _ => value_fit(p, r, text).2,
        }
    }

    #[test]
    fn long_but_realistic_values_are_shown_in_full() {
        let three_kinds = [(CoreKind::Performance, 6), (CoreKind::Efficient, 8), (CoreKind::LowPower, 2)]
            .map(|(kind, cores)| CoreSet { kind, cores, threads: cores });
        let cores = format::core_count(&three_kinds, 16);
        let cases = [
            (Tab::Cpu, cpu::CORES, cores.as_str()),
            (Tab::Cpu, cpu::CORES, "8P + 16E"),
            (Tab::Mainboard, mainboard::MODEL, "MAG B650 TOMAHAWK WIFI (MS-7D75)"),
            (Tab::Spd, spd::PART_NUMBER, "BLS16G4D32AESB.M16FE"),
            (Tab::Spd, spd::PART_NUMBER, "F5-6000J3038F16GX2-TZ5N"),
            (Tab::Graphics, graphics::ADAPTER, "NVIDIA GeForce RTX 4060 Laptop GPU"),
            (Tab::Graphics, graphics::NAME, "NVIDIA GeForce RTX 4060 Laptop GPU"),
        ];
        at_common_scales(|dpi, p| {
            for (tab, id, text) in cases {
                assert!(shown_in_full(p, tab, id, text), "{dpi} dpi: {text:?} is clipped on the {tab:?} tab");
            }
        });
    }

    #[test]
    fn value_boxes_condense_before_they_clip() {
        at_common_scales(|dpi, p| {
            let r = control_rect(p.scale, Tab::Cpu, cpu::CORES).expect("the Cores box");
            let roomy = padded(p, r);
            assert_eq!(value_fit(p, r, "8"), (roomy, FontId::Value, true), "{dpi} dpi");
            // Text that fits no font is clipped inside the padding, in the tightest one.
            let endless = "0123456789".repeat(8);
            assert_eq!(value_fit(p, r, &endless), (roomy, FontId::ValueTight, false), "{dpi} dpi");
            // Every font in between is used as soon as the wider ones overflow.
            let mut text = String::new();
            let mut used = Vec::new();
            while text.len() < 80 {
                text.push('8');
                let (area, font, fits) = value_fit(p, r, &text);
                assert!(!fits || p.text_width(&text, font) <= width(area), "{dpi} dpi: {text}");
                if !used.contains(&font) {
                    used.push(font);
                }
            }
            assert_eq!(used, VALUE_FONTS, "{dpi} dpi");
        });
    }

    #[test]
    fn mood_reflects_mouse_state() {
        assert_eq!(Mood::of(true, false, false), Mood::Normal);
        assert_eq!(Mood::of(true, true, false), Mood::Hot);
        assert_eq!(Mood::of(true, true, true), Mood::Pressed);
        assert_eq!(Mood::of(true, false, true), Mood::Hot, "pressed but dragged off the control");
        assert_eq!(Mood::of(false, true, true), Mood::Disabled);
    }

    #[test]
    fn scroll_thumb_tracks_the_visible_range() {
        assert_eq!(thumb_span(200, 0, 10, 40), (0, 50));
        assert_eq!(thumb_span(200, 30, 10, 40), (150, 200));
        assert_eq!(thumb_span(200, 35, 10, 40), (175, 200), "never past the end of the track");
        assert_eq!(thumb_span(200, 0, 0, 0), (0, 0));
        assert_eq!(thumb_span(100, 1, 1, 1000), (0, 0));
    }
}
