//! Dialog units and their conversion to device pixels.
//!
//! Every layout table is written in the dialog units (DLU) of an 8 pt
//! "MS Shell Dlg" dialog: at 96 DPI one horizontal unit is 1.5 px and one
//! vertical unit is 1.625 px. Edges are converted independently so adjoining
//! boxes stay adjoining at every scale.

use windows::Win32::Foundation::{POINT, RECT};

/// The DPI the layout tables and font sizes are written for.
pub const BASE_DPI: u32 = 96;

/// A rectangle in dialog units: left, top, width, height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dlu {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Shorthand constructor for the layout tables.
pub const fn dlu(x: i32, y: i32, w: i32, h: i32) -> Dlu {
    Dlu { x, y, w, h }
}

impl Dlu {
    pub const fn right(self) -> i32 {
        self.x + self.w
    }

    pub const fn bottom(self) -> i32 {
        self.y + self.h
    }

    /// The same rectangle moved by `(dx, dy)`.
    pub const fn offset(self, dx: i32, dy: i32) -> Dlu {
        dlu(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// Whether `other` lies completely inside this rectangle.
    #[cfg(test)]
    pub const fn contains(self, other: Dlu) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }

    /// Whether the two rectangles share any area.
    #[cfg(test)]
    pub const fn intersects(self, other: Dlu) -> bool {
        self.x < other.right() && other.x < self.right() && self.y < other.bottom() && other.y < self.bottom()
    }
}

/// Converts dialog units and 96-DPI pixel measures to device pixels for one DPI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scale {
    dpi: u32,
}

impl Scale {
    /// A scale for `dpi`; values the system would never report are clamped to a sane range.
    pub fn new(dpi: u32) -> Self {
        Scale { dpi: dpi.clamp(BASE_DPI / 2, BASE_DPI * 10) }
    }

    pub fn dpi(self) -> u32 {
        self.dpi
    }

    /// Horizontal dialog units to pixels: `units * 1.5 * dpi / 96`, rounded to nearest.
    pub fn x(self, units: i32) -> i32 {
        round_div(i64::from(units) * 3 * i64::from(self.dpi), 2 * i64::from(BASE_DPI))
    }

    /// Vertical dialog units to pixels: `units * 1.625 * dpi / 96`, rounded to nearest.
    pub fn y(self, units: i32) -> i32 {
        round_div(i64::from(units) * 13 * i64::from(self.dpi), 8 * i64::from(BASE_DPI))
    }

    /// A pixel measure given at 96 DPI, scaled and rounded; never less than one pixel for a positive input.
    pub fn px(self, px96: i32) -> i32 {
        let scaled = round_div(i64::from(px96) * i64::from(self.dpi), i64::from(BASE_DPI));
        if px96 > 0 {
            scaled.max(1)
        } else {
            scaled
        }
    }

    pub fn rect(self, r: Dlu) -> RECT {
        RECT { left: self.x(r.x), top: self.y(r.y), right: self.x(r.right()), bottom: self.y(r.bottom()) }
    }

    /// Pixel size of a window that is `w` x `h` dialog units.
    pub fn size(self, w: i32, h: i32) -> (i32, i32) {
        (self.x(w), self.y(h))
    }
}

/// `numerator / denominator` rounded half away from zero; `denominator` must be positive.
fn round_div(numerator: i64, denominator: i64) -> i32 {
    let half = denominator / 2;
    let rounded =
        if numerator >= 0 { (numerator + half) / denominator } else { (numerator - half) / denominator };
    rounded.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

pub fn width(r: RECT) -> i32 {
    r.right - r.left
}

pub fn height(r: RECT) -> i32 {
    r.bottom - r.top
}

/// `r` shrunk by `d` pixels on every side.
pub fn inset(r: RECT, d: i32) -> RECT {
    RECT { left: r.left + d, top: r.top + d, right: r.right - d, bottom: r.bottom - d }
}

pub fn contains(r: RECT, pt: POINT) -> bool {
    pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom
}

/// The largest square centred in `r`.
pub fn centred_square(r: RECT) -> RECT {
    let side = width(r).min(height(r)).max(0);
    let left = r.left + (width(r) - side) / 2;
    let top = r.top + (height(r) - side) / 2;
    RECT { left, top, right: left + side, bottom: top + side }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_scale_matches_the_dialog_unit_definition() {
        let s = Scale::new(96);
        assert_eq!(s.x(2), 3);
        assert_eq!(s.x(278), 417);
        assert_eq!(s.y(8), 13);
        assert_eq!(s.y(258), 419);
        assert_eq!(s.px(1), 1);
        assert_eq!(s.px(14), 14);
        assert_eq!(s.px(0), 0);
    }

    #[test]
    fn scales_with_dpi() {
        let s = Scale::new(192);
        assert_eq!(s.x(278), 834);
        assert_eq!(s.y(8), 26);
        assert_eq!(s.px(14), 28);
        let s = Scale::new(120);
        assert_eq!(s.x(4), 8);
        assert_eq!(s.px(1), 1);
        assert_eq!(Scale::new(144).px(1), 2);
    }

    #[test]
    fn dpi_is_clamped() {
        assert_eq!(Scale::new(0).dpi(), 48);
        assert_eq!(Scale::new(1_000_000).dpi(), 960);
    }

    #[test]
    fn adjoining_boxes_share_an_edge_at_every_scale() {
        for dpi in [96, 120, 144, 168, 192, 240] {
            let s = Scale::new(dpi);
            let left = s.rect(dlu(157, 124, 63, 10));
            let right = s.rect(dlu(220, 124, 35, 10));
            assert_eq!(left.right, right.left, "dpi {dpi}");
        }
    }

    #[test]
    fn rect_helpers() {
        let r = RECT { left: 10, top: 20, right: 50, bottom: 40 };
        assert_eq!((width(r), height(r)), (40, 20));
        assert_eq!(inset(r, 2), RECT { left: 12, top: 22, right: 48, bottom: 38 });
        assert!(contains(r, POINT { x: 10, y: 20 }));
        assert!(!contains(r, POINT { x: 50, y: 20 }));
        assert_eq!(centred_square(r), RECT { left: 20, top: 20, right: 40, bottom: 40 });
    }

    #[test]
    fn dlu_relations() {
        let page = dlu(0, 0, 261, 199);
        assert!(page.contains(dlu(2, 5, 257, 106)));
        assert!(!page.contains(dlu(250, 5, 20, 10)));
        assert!(dlu(0, 0, 10, 10).intersects(dlu(9, 9, 5, 5)));
        assert!(!dlu(0, 0, 10, 10).intersects(dlu(10, 0, 5, 5)));
        assert_eq!(dlu(1, 2, 3, 4).offset(10, 20), dlu(11, 22, 3, 4));
    }
}
