//! Colours of the dark theme.

use windows::Win32::Foundation::COLORREF;

use crate::brand;
use crate::config::Config;

/// Builds a GDI colour (`0x00BBGGRR`) from its components.
pub const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((b as u32) << 16 | (g as u32) << 8 | r as u32)
}

/// Converts a `0xRRGGBB` value, as stored in the settings file, to a GDI colour.
pub const fn from_rrggbb(value: u32) -> COLORREF {
    rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

/// Mixes `a` and `b`; `weight` is the share of `b` in 256ths.
pub const fn blend(a: COLORREF, b: COLORREF, weight: u32) -> COLORREF {
    let weight = if weight > 256 { 256 } else { weight };
    COLORREF(mix(a.0, b.0, 0, weight) | mix(a.0, b.0, 8, weight) | mix(a.0, b.0, 16, weight))
}

/// Mixes the colour channel at `shift` of two packed colours.
const fn mix(a: u32, b: u32, shift: u32, weight: u32) -> u32 {
    let ca = (a >> shift) & 0xFF;
    let cb = (b >> shift) & 0xFF;
    ((ca * (256 - weight) + cb * weight + 128) >> 8) << shift
}

/// The palette every window is drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Window background.
    pub window: COLORREF,
    /// One-pixel outline of the windows.
    pub outline: COLORREF,
    /// Title strip background.
    pub title: COLORREF,
    /// Tab page background.
    pub panel: COLORREF,
    /// Group frames and the page border.
    pub frame: COLORREF,
    /// Value box background and border.
    pub field: COLORREF,
    pub field_border: COLORREF,
    /// Background and border of the boxes of unused rows.
    pub field_dim: COLORREF,
    pub field_dim_border: COLORREF,
    /// Buttons and selectors in their resting, hovered and pressed states.
    pub control: COLORREF,
    pub control_hot: COLORREF,
    pub control_pressed: COLORREF,
    pub control_border: COLORREF,
    /// Label text.
    pub label: COLORREF,
    /// Text of unused rows and disabled controls.
    pub dim: COLORREF,
    /// Frame captions and headings that should stand out a little.
    pub caption: COLORREF,
    /// Value text; the one colour the settings file can change.
    pub value: COLORREF,
    /// Text of the window itself: titles, tab titles, button captions and their glyphs.
    pub chrome: COLORREF,
    pub accent: COLORREF,
    /// Fill of the reference bars.
    pub neutral_bar: COLORREF,
    /// Text drawn on top of a bar or an accent-coloured surface.
    pub on_accent: COLORREF,
}

impl Theme {
    /// The built-in palette with the value colour overridden by the settings file.
    pub fn new(cfg: &Config) -> Self {
        let window = rgb(0x12, 0x12, 0x14);
        let panel = rgb(0x1C, 0x1C, 0x20);
        let field = rgb(0x27, 0x27, 0x2D);
        let control = rgb(0x30, 0x30, 0x38);
        let white = rgb(0xFF, 0xFF, 0xFF);
        Theme {
            window,
            outline: rgb(0x3A, 0x3A, 0x42),
            title: rgb(0x1A, 0x1A, 0x1E),
            panel,
            frame: rgb(0x3C, 0x3C, 0x46),
            field,
            field_border: rgb(0x45, 0x45, 0x50),
            field_dim: blend(panel, field, 96),
            field_dim_border: rgb(0x30, 0x30, 0x38),
            control,
            control_hot: blend(control, white, 28),
            control_pressed: blend(control, window, 128),
            control_border: rgb(0x52, 0x52, 0x5E),
            label: rgb(0xB9, 0xB9, 0xC2),
            dim: rgb(0x6C, 0x6C, 0x76),
            caption: rgb(0xD8, 0xD8, 0xE0),
            value: cfg.text_font_color.map_or(white, from_rrggbb),
            chrome: white,
            accent: brand::ACCENT,
            neutral_bar: rgb(0x5E, 0x66, 0x76),
            on_accent: white,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_conversions() {
        assert_eq!(rgb(0x12, 0x34, 0x56), COLORREF(0x0056_3412));
        assert_eq!(from_rrggbb(0x00FF80), COLORREF(0x0080_FF00));
        assert_eq!(from_rrggbb(0xCE422B), rgb(0xCE, 0x42, 0x2B));
    }

    #[test]
    fn blending() {
        let black = rgb(0, 0, 0);
        let white = rgb(255, 255, 255);
        assert_eq!(blend(black, white, 0), black);
        assert_eq!(blend(black, white, 256), white);
        assert_eq!(blend(black, white, 1000), white);
        assert_eq!(blend(black, white, 128), rgb(128, 128, 128));
        assert_eq!(blend(rgb(10, 20, 30), rgb(10, 20, 30), 77), rgb(10, 20, 30));
    }

    #[test]
    fn value_colour_comes_from_the_settings() {
        let mut cfg = Config::default();
        assert_eq!(Theme::new(&cfg).value, rgb(255, 255, 255));
        cfg.text_font_color = Some(0x00FF80);
        assert_eq!(Theme::new(&cfg).value, rgb(0x00, 0xFF, 0x80));
        assert_eq!(Theme::new(&cfg).accent, brand::ACCENT);
    }

    #[test]
    fn only_the_value_colour_follows_the_settings() {
        let mut cfg = Config::default();
        let plain = Theme::new(&cfg);
        assert_eq!(plain.chrome, rgb(255, 255, 255));
        cfg.text_font_color = Some(0x00FF80);
        assert_eq!(Theme::new(&cfg), Theme { value: rgb(0x00, 0xFF, 0x80), ..plain });
    }
}
