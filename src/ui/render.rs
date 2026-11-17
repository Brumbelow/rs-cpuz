//! Draws the main window from the layout tables and hit-tests against them.
//!
//! The same functions paint the live window and the snapshots: they only need
//! a painter (target DC, scale, fonts, palette) and a view of the state.

use windows::Win32::Foundation::{POINT, RECT};

use crate::brand;

use super::content;
use super::format;
use super::gdi::{Align, FontId, Overflow, Painter};
use super::geom::{centred_square, contains, dlu, inset, Dlu, Scale};
use super::layout::{
    cpu, Item, Kind, Tab, CHECK_REACH, CLOSE_BOX, CLOSE_BUTTON, FRAME_PIXELS, MINIMIZE_BOX, PAGE_ORIGIN,
    TAB_CONTROL, TAB_STRIP_HEIGHT, TITLE_HEIGHT, TOOLS_BUTTON, VERSION_LABEL,
};
use super::state::{ListId, Target, View};
use super::widgets::{self, Glyph, Mood};

/// Left edge of the logo mark in a title strip, in horizontal dialog units.
const TITLE_MARK_X: i32 = 5;

/// Pixel size of the main window at `scale`.
pub fn frame_size(scale: Scale) -> (i32, i32) {
    (scale.px(FRAME_PIXELS.0), scale.px(FRAME_PIXELS.1))
}

/// The page area of the tab control, in frame coordinates.
fn page_panel() -> Dlu {
    dlu(TAB_CONTROL.x, TAB_CONTROL.y + TAB_STRIP_HEIGHT, TAB_CONTROL.w, TAB_CONTROL.h - TAB_STRIP_HEIGHT)
}

/// A page rectangle in frame coordinates.
fn in_frame(rect: Dlu) -> Dlu {
    rect.offset(PAGE_ORIGIN.0, PAGE_ORIGIN.1)
}

fn mood(view: &View, target: Target, enabled: bool) -> Mood {
    Mood::of(enabled, view.ui.hot == Some(target), view.ui.pressed == Some(target))
}

/// Paints the complete main window.
pub fn main_window(p: &Painter, view: &View) {
    let (width, height) = frame_size(p.scale);
    let window = RECT { left: 0, top: 0, right: width, bottom: height };
    p.fill(window, p.theme.window);
    title_strip(p, width, TITLE_HEIGHT, brand::APP_NAME, FontId::Title);
    widgets::title_button(p, p.scale.rect(MINIMIZE_BOX), Glyph::Minimize, mood(view, Target::Minimize, true));
    widgets::title_button(p, p.scale.rect(CLOSE_BOX), Glyph::Close, mood(view, Target::CloseBox, true));

    p.panel(p.scale.rect(page_panel()), p.theme.panel, p.theme.frame);
    for tab in Tab::ALL {
        tab_header(p, view, tab);
    }
    for item in view.ui.tab.items() {
        page_item(p, view, item);
    }

    let version = format::short_version(brand::VERSION);
    p.text(
        p.scale.rect(VERSION_LABEL),
        &version,
        FontId::Label,
        p.theme.label,
        Align::Left,
        Overflow::Visible,
    );
    let tools =
        if view.ui.open == Some(ListId::ToolsMenu) { Mood::Pressed } else { mood(view, Target::Tools, true) };
    widgets::button(p, p.scale.rect(TOOLS_BUTTON), "Tools", tools, true);
    widgets::button(p, p.scale.rect(CLOSE_BUTTON), "Close", mood(view, Target::Close, true), false);
    p.outline(window, p.theme.outline);
}

/// Title strip of a window `width` pixels wide: logo mark, title and an accent underline.
pub fn title_strip(p: &Painter, width: i32, height_units: i32, title: &str, font: FontId) {
    let strip = RECT { left: 0, top: 0, right: width, bottom: p.scale.y(height_units) };
    p.fill(strip, p.theme.title);
    let underline = p.scale.px(2);
    p.fill(RECT { top: strip.bottom - underline, ..strip }, p.theme.accent);

    let body = RECT { bottom: strip.bottom - underline, ..strip };
    let side = (body.bottom - body.top) * 3 / 5;
    let left = p.scale.x(TITLE_MARK_X);
    let top = body.top + (body.bottom - body.top - side) / 2;
    let mark = RECT { left, top, right: left + side, bottom: top + side };
    brand::draw_mark(p.hdc, mark, p.theme.accent);
    let text = RECT { left: mark.right + p.scale.px(6), ..body };
    p.text(text, title, font, p.theme.chrome, Align::Left, Overflow::Visible);
}

fn tab_header(p: &Painter, view: &View, tab: Tab) {
    let t = p.theme;
    let r = p.scale.rect(tab.strip_rect());
    let line = p.hairline();
    if view.ui.tab == tab {
        // The active tab merges with the page: it covers the page's top border, and
        // its side lines run down into that border so the outline stays closed.
        let merged = RECT { bottom: r.bottom + line, ..r };
        p.fill(merged, t.panel);
        p.fill(RECT { right: r.left + line, ..merged }, t.frame);
        p.fill(RECT { left: r.right - line, ..merged }, t.frame);
        p.fill(RECT { bottom: r.top + p.scale.px(2), ..r }, t.accent);
        p.text(r, tab.title(), FontId::Strong, t.chrome, Align::Centre, Overflow::Ellipsis);
        return;
    }
    let hot = view.ui.hot == Some(Target::Tab(tab));
    let body = RECT { top: r.top + p.scale.px(2), left: r.left + line, right: r.right - line, ..r };
    p.fill(body, if hot { t.control } else { t.title });
    let ink = if hot { t.chrome } else { t.label };
    p.text(r, tab.title(), FontId::Label, ink, Align::Centre, Overflow::Ellipsis);
}

/// Pixel rectangle of a page control.
fn item_rect(scale: Scale, item: &Item) -> RECT {
    scale.rect(in_frame(item.rect))
}

/// The rectangle a label is drawn in: its own right edge, the vertical extent of
/// the box it describes (so the text of a row lines up however the two were
/// rounded to pixels) and all the free space on its left.
fn label_rect(scale: Scale, tab: Tab, item: &Item) -> RECT {
    let own = item_rect(scale, item);
    let left = scale.x(PAGE_ORIGIN.0 + tab.label_limit(item));
    match tab.label_partner(item) {
        Some(partner) => {
            let row = item_rect(scale, partner);
            RECT { left, top: row.top, bottom: row.bottom, ..own }
        }
        None => RECT { left, ..own },
    }
}

/// Left-aligned text keeps its own width but shares the vertical extent of its row.
fn text_rect(scale: Scale, tab: Tab, item: &Item) -> RECT {
    let own = item_rect(scale, item);
    RECT { left: own.left, ..label_rect(scale, tab, item) }
}

/// Vertical pixel position of the frame line a caption sits on.
fn caption_edge(p: &Painter, tab: Tab, item: &Item) -> i32 {
    match tab.caption_frame(item) {
        Some(frame) => item_rect(p.scale, frame).top + p.hairline() / 2,
        None => {
            let r = item_rect(p.scale, item);
            (r.top + r.bottom) / 2
        }
    }
}

fn page_item(p: &Painter, view: &View, item: &Item) {
    let tab = view.ui.tab;
    let t = p.theme;
    let r = item_rect(p.scale, item);
    let text = || content::text(view, tab, item);
    let target = Target::Control(item.id);
    match item.kind {
        Kind::Frame => widgets::frame(p, r),
        Kind::Caption => widgets::caption(p, r, caption_edge(p, tab, item), &text()),
        Kind::Label => widgets::label(p, label_rect(p.scale, tab, item), &text(), false),
        Kind::DimLabel => widgets::label(p, label_rect(p.scale, tab, item), &text(), true),
        Kind::Text => widgets::note(p, text_rect(p.scale, tab, item), &text(), Align::Left),
        Kind::Heading => widgets::note(p, r, &text(), Align::Centre),
        Kind::Info => p.text(r, &text(), FontId::Value, t.value, Align::Centre, Overflow::Ellipsis),
        Kind::Title => p.text(r, &text(), FontId::Large, t.chrome, Align::Left, Overflow::Visible),
        Kind::Value => widgets::value(p, r, &text(), Align::Centre),
        Kind::ValueLeft => widgets::value(p, r, &text(), Align::Left),
        Kind::ValueWrap => widgets::wrapped_value(p, r, &text()),
        Kind::DimValue => widgets::dim_value(p, r),
        Kind::Combo => {
            let list = content::combo_list(tab, item.id);
            let open = list.is_some() && view.ui.open == list;
            widgets::combo(p, r, &text(), mood(view, target, content::enabled(view, tab, item)), open);
        }
        Kind::Button => {
            widgets::button(p, r, &text(), mood(view, target, content::enabled(view, tab, item)), false);
        }
        Kind::Check => widgets::check(p, r, content::checked(view, tab, item.id), mood(view, target, true)),
        Kind::Bar => widgets::bar(p, r, &content::bar(view, tab, item.id)),
        Kind::Mark => {
            p.panel(r, t.field, t.field_border);
            brand::draw_mark(p.hdc, centred_square(inset(r, p.scale.px(6))), t.accent);
        }
        Kind::Emblem => brand::draw_mark(p.hdc, centred_square(r), t.accent),
    }
}

/// Clickable area of an interactive page control, in frame coordinates.
fn hit_area(item: &Item) -> Dlu {
    let rect = in_frame(item.rect);
    if item.kind == Kind::Check {
        dlu(rect.x, rect.y, CHECK_REACH, rect.h)
    } else {
        rect
    }
}

/// The interactive element under `pt` (client pixels), if it is enabled.
pub fn hit_test(scale: Scale, view: &View, pt: POINT) -> Option<Target> {
    let fixed = [
        (MINIMIZE_BOX, Target::Minimize),
        (CLOSE_BOX, Target::CloseBox),
        (TOOLS_BUTTON, Target::Tools),
        (CLOSE_BUTTON, Target::Close),
    ];
    if let Some((_, target)) = fixed.iter().find(|(rect, _)| contains(scale.rect(*rect), pt)) {
        return Some(*target);
    }
    if let Some(tab) = Tab::ALL.into_iter().find(|tab| contains(scale.rect(tab.strip_rect()), pt)) {
        return Some(Target::Tab(tab));
    }
    let tab = view.ui.tab;
    tab.items()
        .iter()
        .filter(|item| item.kind.is_interactive())
        .find(|item| contains(scale.rect(hit_area(item)), pt) && content::enabled(view, tab, item))
        .map(|item| Target::Control(item.id))
}

/// Whether `pt` is on the draggable part of the title strip.
pub fn on_title_strip(scale: Scale, pt: POINT) -> bool {
    let (width, _) = frame_size(scale);
    let strip = RECT { left: 0, top: 0, right: width, bottom: scale.y(TITLE_HEIGHT) };
    contains(strip, pt) && !contains(scale.rect(MINIMIZE_BOX), pt) && !contains(scale.rect(CLOSE_BOX), pt)
}

/// Whether `pt` is inside the Clocks group of the CPU tab, where the core menu opens.
pub fn on_clocks_group(scale: Scale, tab: Tab, pt: POINT) -> bool {
    tab == Tab::Cpu
        && Tab::Cpu.item(cpu::CLOCKS_GROUP).is_some_and(|group| contains(item_rect(scale, group), pt))
}

/// Pixel rectangle of a page control, in client coordinates.
pub fn control_rect(scale: Scale, tab: Tab, id: u16) -> Option<RECT> {
    tab.item(id).map(|item| item_rect(scale, item))
}

#[cfg(test)]
mod tests {
    use windows::Win32::Foundation::COLORREF;

    use super::*;
    use crate::config::Config;
    use crate::demo;
    use crate::ui::gdi::{Fonts, Surface};
    use crate::ui::layout::{about, bench, spd};
    use crate::ui::state::UiState;
    use crate::ui::theme::{rgb, Theme};

    /// The main window showing one tab of the sample system, drawn off screen.
    struct Shot {
        surface: Surface,
        scale: Scale,
        theme: Theme,
    }

    impl Shot {
        /// `None` in a session where GDI gives out no drawing surface.
        fn take(cfg: &Config, dpi: u32, tab: Tab) -> Option<Shot> {
            let scale = Scale::new(dpi);
            let (width, height) = frame_size(scale);
            let surface = Surface::new(width, height)?;
            let fonts = Fonts::new(cfg, scale);
            let theme = Theme::new(cfg);
            let sys = demo::system();
            let clocks = demo::clocks();
            let mut ui = UiState::new(&sys, None, 16, Vec::new());
            ui.tab = tab;
            let painter = Painter { hdc: surface.dc(), scale, fonts: &fonts, theme: &theme };
            main_window(&painter, &View { sys: &sys, clocks: &clocks, ui: &ui });
            Some(Shot { surface, scale, theme })
        }

        /// The pixels of `r`, row by row from its top-left corner.
        fn area(&self, r: RECT) -> Vec<COLORREF> {
            let (width, height) = (self.surface.width(), self.surface.height());
            let pixels = self.surface.pixels();
            let at = |x: i32, y: i32| {
                let offset = (((height - 1 - y) * width + x) * 4) as usize;
                rgb(pixels[offset + 2], pixels[offset + 1], pixels[offset])
            };
            (r.top..r.bottom).flat_map(|y| (r.left..r.right).map(move |x| at(x, y))).collect()
        }

        /// Whether every pixel of `r` has the colour `color`.
        fn is_filled(&self, r: RECT, color: COLORREF) -> bool {
            self.area(r).iter().all(|&pixel| pixel == color)
        }
    }

    fn centre(scale: Scale, rect: Dlu) -> POINT {
        let r = scale.rect(rect);
        POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 }
    }

    fn on_page(scale: Scale, tab: Tab, id: u16) -> POINT {
        centre(scale, in_frame(tab.item(id).unwrap().rect))
    }

    #[test]
    fn frame_matches_the_reference_window() {
        assert_eq!(frame_size(Scale::new(96)), (416, 418));
        assert_eq!(frame_size(Scale::new(192)), (832, 836));
        assert_eq!(frame_size(Scale::new(144)), (624, 627));
    }

    #[test]
    fn frame_controls_fit_the_window_at_every_scale() {
        for dpi in [96, 120, 144, 168, 192, 288] {
            let scale = Scale::new(dpi);
            let (width, height) = frame_size(scale);
            let fixed = [MINIMIZE_BOX, CLOSE_BOX, TAB_CONTROL, VERSION_LABEL, TOOLS_BUTTON, CLOSE_BUTTON];
            for r in fixed.map(|rect| scale.rect(rect)) {
                assert!(r.left > 0 && r.top > 0 && r.right < width && r.bottom < height, "dpi {dpi}: {r:?}");
            }
            // The tab control keeps the same margin on both sides.
            let tabs = scale.rect(TAB_CONTROL);
            assert!((tabs.left - (width - tabs.right)).abs() <= 1, "dpi {dpi}");
        }
    }

    #[test]
    fn hit_testing_follows_the_layout() {
        let sys = demo::system();
        let clocks = demo::clocks();
        let mut ui = UiState::new(&sys, None, 16, Vec::new());
        for dpi in [96, 144] {
            let scale = Scale::new(dpi);
            ui.tab = Tab::Cpu;
            let view = View { sys: &sys, clocks: &clocks, ui: &ui };
            assert_eq!(hit_test(scale, &view, centre(scale, CLOSE_BOX)), Some(Target::CloseBox));
            assert_eq!(hit_test(scale, &view, centre(scale, MINIMIZE_BOX)), Some(Target::Minimize));
            assert_eq!(hit_test(scale, &view, centre(scale, TOOLS_BUTTON)), Some(Target::Tools));
            assert_eq!(hit_test(scale, &view, centre(scale, CLOSE_BUTTON)), Some(Target::Close));
            for tab in Tab::ALL {
                assert_eq!(hit_test(scale, &view, centre(scale, tab.strip_rect())), Some(Target::Tab(tab)));
            }
            let selection = on_page(scale, Tab::Cpu, cpu::SELECTION);
            assert_eq!(hit_test(scale, &view, selection), Some(Target::Control(cpu::SELECTION)));
            assert_eq!(hit_test(scale, &view, on_page(scale, Tab::Cpu, cpu::NAME)), None);
            assert!(on_clocks_group(scale, Tab::Cpu, on_page(scale, Tab::Cpu, cpu::CORE_SPEED)));
            assert!(!on_clocks_group(scale, Tab::Cpu, on_page(scale, Tab::Cpu, cpu::LEVEL2)));
            assert!(!on_clocks_group(scale, Tab::Spd, on_page(scale, Tab::Cpu, cpu::CORE_SPEED)));
        }
    }

    #[test]
    fn controls_only_react_on_their_own_tab() {
        let sys = demo::system();
        let clocks = demo::clocks();
        let scale = Scale::new(96);
        let mut ui = UiState::new(&sys, None, 16, Vec::new());
        ui.tab = Tab::Spd;
        let view = View { sys: &sys, clocks: &clocks, ui: &ui };
        assert_eq!(
            hit_test(scale, &view, on_page(scale, Tab::Spd, spd::SLOT)),
            Some(Target::Control(spd::SLOT))
        );
        assert_eq!(hit_test(scale, &view, on_page(scale, Tab::Bench, bench::BENCH_BUTTON)), None);
    }

    #[test]
    fn disabled_controls_are_not_hit() {
        let sys = demo::system();
        let clocks = demo::clocks();
        let scale = Scale::new(96);
        let mut ui = UiState::new(&sys, None, 16, Vec::new());
        ui.tab = Tab::Bench;
        {
            let view = View { sys: &sys, clocks: &clocks, ui: &ui };
            let save = on_page(scale, Tab::Bench, bench::SAVE_BUTTON);
            assert_eq!(hit_test(scale, &view, save), None);
            let stress = on_page(scale, Tab::Bench, bench::STRESS_BUTTON);
            assert_eq!(hit_test(scale, &view, stress), Some(Target::Control(bench::STRESS_BUTTON)));
            let label = centre(scale, in_frame(dlu(30, 28, 20, 9)));
            assert_eq!(hit_test(scale, &view, label), Some(Target::Control(bench::SINGLE_REFERENCE_CHECK)));
        }
        ui.bench.begin_bench();
        let view = View { sys: &sys, clocks: &clocks, ui: &ui };
        assert_eq!(hit_test(scale, &view, on_page(scale, Tab::Bench, bench::STRESS_BUTTON)), None);
        assert_eq!(hit_test(scale, &view, on_page(scale, Tab::Bench, bench::THREADS)), None);
    }

    #[test]
    fn title_strip_excludes_its_buttons() {
        let scale = Scale::new(96);
        assert!(on_title_strip(scale, POINT { x: 100, y: 10 }));
        assert!(!on_title_strip(scale, centre(scale, CLOSE_BOX)));
        assert!(!on_title_strip(scale, centre(scale, MINIMIZE_BOX)));
        assert!(!on_title_strip(scale, POINT { x: 100, y: 60 }));
    }

    #[test]
    fn control_rects_are_in_client_coordinates() {
        let scale = Scale::new(96);
        let r = control_rect(scale, Tab::Cpu, cpu::SELECTION).unwrap();
        assert_eq!((r.left, r.top), (scale.x(8 + 52), scale.y(38 + 177)));
        assert!(control_rect(scale, Tab::Cpu, 1).is_none());
    }

    #[test]
    fn page_border_is_open_under_the_active_tab_only() {
        for dpi in [96, 120, 144, 168, 192] {
            for tab in Tab::ALL {
                let Some(shot) = Shot::take(&Config::default(), dpi, tab) else { return };
                let (scale, t) = (shot.scale, shot.theme);
                let line = scale.px(1);
                let panel = scale.rect(page_panel());
                let active = scale.rect(tab.strip_rect());
                let border = RECT { bottom: panel.top + line, ..panel };
                let left_of_tab = RECT { right: active.left + line, ..border };
                let under_tab = RECT { left: active.left + line, right: active.right - line, ..border };
                let right_of_tab = RECT { left: active.right - line, ..border };
                assert!(shot.is_filled(left_of_tab, t.frame), "{dpi} dpi, {tab:?}: left of the tab");
                assert!(shot.is_filled(under_tab, t.panel), "{dpi} dpi, {tab:?}: under the tab");
                assert!(shot.is_filled(right_of_tab, t.frame), "{dpi} dpi, {tab:?}: right of the tab");

                // The side lines of the tab reach down into the border without a gap.
                let sides = RECT { top: active.top + scale.px(2), bottom: border.bottom, ..active };
                assert!(
                    shot.is_filled(RECT { right: sides.left + line, ..sides }, t.frame),
                    "{dpi} dpi, {tab:?}"
                );
                assert!(
                    shot.is_filled(RECT { left: sides.right - line, ..sides }, t.frame),
                    "{dpi} dpi, {tab:?}"
                );
            }
        }
    }

    #[test]
    fn value_text_settings_leave_the_window_chrome_alone() {
        let plain = Config::default();
        let styled = Config { text_font_color: Some(0x00FF80), text_font_size: 18, ..Config::default() };
        let values = [(Tab::Cpu, cpu::NAME), (Tab::Bench, bench::PROCESSOR), (Tab::About, about::OS_NAME)];
        for (tab, value) in values {
            let (Some(before), Some(after)) = (Shot::take(&plain, 96, tab), Shot::take(&styled, 96, tab))
            else {
                return;
            };
            let scale = before.scale;
            let same = |r: RECT| before.area(r) == after.area(r);
            let (width, height) = frame_size(scale);
            assert!(
                same(RECT { left: 0, top: 0, right: width, bottom: scale.y(TITLE_HEIGHT) }),
                "{tab:?}: title"
            );
            let strip = dlu(TAB_CONTROL.x, TAB_CONTROL.y, TAB_CONTROL.w, TAB_STRIP_HEIGHT);
            assert!(same(scale.rect(strip)), "{tab:?}: tab titles");
            let bottom_row =
                RECT { left: 0, top: scale.y(TAB_CONTROL.bottom()), right: width, bottom: height };
            assert!(same(bottom_row), "{tab:?}: version, Tools and Close");
            for item in tab.items() {
                let r = item_rect(scale, item);
                match item.kind {
                    Kind::Button | Kind::Title => assert!(same(r), "{tab:?}: {:?}", item.text),
                    Kind::Combo => {
                        let chevron = RECT { left: r.right - scale.px(widgets::CHEVRON_ZONE), ..r };
                        assert!(same(chevron), "{tab:?}: chevron of a selector");
                    }
                    _ => {}
                }
            }
            let r = control_rect(scale, tab, value).unwrap();
            assert!(!same(r), "{tab:?}: the values do follow the settings");
        }
    }
}
