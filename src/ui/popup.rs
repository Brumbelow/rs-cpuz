//! The popup list behind the selectors and the two menus.
//!
//! The list is an owned popup window that takes the keyboard focus while it is
//! open and closes itself as soon as it loses activation. It reports the chosen
//! entry and its own disappearance to the owner with window messages.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_NEXT, VK_PRIOR, VK_RETURN, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, SendMessageW, CS_DROPSHADOW, WA_INACTIVE, WM_ACTIVATE, WM_DESTROY, WM_ERASEBKGND,
    WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT, WS_EX_TOOLWINDOW,
    WS_POPUP,
};

use crate::config::Config;

use super::gdi::{Align, FontId, Overflow, Painter, Surface, VALUE_FONTS};
use super::geom::{contains, width, Scale};
use super::state::ListId;
use super::theme::Theme;
use super::widgets;
use super::win::{self, Canvas, Wheel};

const CLASS: PCWSTR = w!("RsCpuzList");
/// Height of one entry, in vertical dialog units.
const ROW_HEIGHT: i32 = 11;
/// Entries shown before the list starts to scroll.
const MAX_ROWS: usize = 12;
/// Horizontal text padding and scroll bar width, in pixels at 96 DPI.
const TEXT_PAD: i32 = 7;
const SCROLL_BAR: i32 = widgets::SCROLL_BAR;
/// Entries scrolled per wheel notch.
const WHEEL_ROWS: isize = 3;

/// What to show and where.
pub struct Request {
    /// Fonts and colours of the owner.
    pub cfg: Config,
    pub list: ListId,
    pub items: Vec<String>,
    /// The current entry, marked in the list and highlighted initially.
    pub selected: Option<usize>,
    /// Screen rectangle the list drops down from; it is at least as wide.
    pub anchor: RECT,
    pub dpi: u32,
}

/// Scroll position and highlight of a list of `len` entries showing `rows` at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cursor {
    len: usize,
    rows: usize,
    top: usize,
    hot: Option<usize>,
}

impl Cursor {
    fn new(len: usize, selected: Option<usize>) -> Cursor {
        let rows = len.min(MAX_ROWS);
        let hot = selected.filter(|&index| index < len);
        let mut cursor = Cursor { len, rows, top: 0, hot };
        cursor.top = cursor.clamp_top(hot.unwrap_or(0).saturating_sub(rows / 2));
        cursor
    }

    fn clamp_top(&self, top: usize) -> usize {
        top.min(self.len - self.rows)
    }

    /// Highlights `index` and scrolls it into view.
    fn select(&mut self, index: usize) {
        if self.len == 0 {
            return;
        }
        let index = index.min(self.len - 1);
        self.hot = Some(index);
        if index < self.top {
            self.top = index;
        } else if index >= self.top + self.rows {
            self.top = index + 1 - self.rows;
        }
    }

    /// Moves the highlight by `delta` entries; from no highlight it enters at either end.
    fn step(&mut self, delta: isize) {
        let target = match self.hot {
            Some(index) => index.saturating_add_signed(delta),
            None if delta < 0 => self.len.saturating_sub(1),
            None => 0,
        };
        self.select(target);
    }

    /// Scrolls by `delta` entries without moving the highlight.
    fn scroll(&mut self, delta: isize) {
        self.top = self.clamp_top(self.top.saturating_add_signed(delta));
    }

    /// Whether there are more entries than rows, so the list shows a scroll indicator.
    fn scrolls(&self) -> bool {
        self.len > self.rows
    }

    /// The entry shown in visible row `row`.
    fn entry(&self, row: usize) -> Option<usize> {
        (row < self.rows).then_some(self.top + row)
    }
}

struct Popup {
    owner: HWND,
    list: ListId,
    items: Vec<String>,
    selected: Option<usize>,
    cursor: Cursor,
    wheel: Wheel,
    theme: Theme,
    canvas: Canvas,
}

/// Registers the window class of the popup list.
pub fn register() -> bool {
    win::register_class(CLASS, CS_DROPSHADOW, procedure, None)
}

/// Pixel rectangle of visible row `row` in a client area `client_width` wide.
fn row_rect(scale: Scale, client_width: i32, row: usize) -> RECT {
    let border = scale.px(1);
    let top = border + scale.y(ROW_HEIGHT) * row as i32;
    RECT { left: border, top, right: client_width - border, bottom: top + scale.y(ROW_HEIGHT) }
}

/// The part of visible row `row` that shows and selects its entry: the row
/// without the scroll indicator of a list that scrolls.
fn entry_rect(scale: Scale, client_width: i32, row: usize, scrolls: bool) -> RECT {
    let bar = if scrolls { scale.px(SCROLL_BAR) } else { 0 };
    let row = row_rect(scale, client_width, row);
    RECT { right: row.right - bar, ..row }
}

/// The entry under client point `pt` of a list `client_width` pixels wide.
fn entry_at(scale: Scale, client_width: i32, cursor: &Cursor, pt: POINT) -> Option<usize> {
    (0..cursor.rows)
        .find(|&row| contains(entry_rect(scale, client_width, row, cursor.scrolls()), pt))
        .and_then(|row| cursor.entry(row))
}

/// Client size for `rows` entries whose widest text is `text_width` pixels, at least `min_width` wide.
fn client_size(scale: Scale, rows: usize, text_width: i32, min_width: i32, scrolls: bool) -> (i32, i32) {
    let bar = if scrolls { scale.px(SCROLL_BAR) } else { 0 };
    let wanted = text_width + 2 * scale.px(TEXT_PAD) + bar + 2 * scale.px(1);
    (wanted.max(min_width), scale.y(ROW_HEIGHT) * rows as i32 + 2 * scale.px(1))
}

/// Where the list goes: below the anchor, or above it when it would leave the work area.
fn position(anchor: RECT, size: (i32, i32), work: RECT) -> (i32, i32) {
    let below = anchor.bottom;
    let y = if below + size.1 <= work.bottom || anchor.top - size.1 < work.top {
        below
    } else {
        anchor.top - size.1
    };
    let x = anchor.left.min(work.right - size.0).max(work.left);
    (x, y)
}

/// Opens the list. Returns `false` when there is nothing to show or the window could not be created.
pub fn open(owner: HWND, request: Request) -> bool {
    if request.items.is_empty() {
        return false;
    }
    let canvas = Canvas::new(&request.cfg, request.dpi);
    let theme = Theme::new(&request.cfg);
    let scale = canvas.scale();
    let cursor = Cursor::new(request.items.len(), request.selected);
    let text_width = Surface::new(1, 1).map_or(0, |surface| {
        let painter = Painter { hdc: surface.dc(), scale, fonts: canvas.fonts(), theme: &theme };
        request.items.iter().map(|item| painter.text_width(item, FontId::Value)).max().unwrap_or(0)
    });
    let work = win::work_area_near(request.anchor).unwrap_or(request.anchor);
    let (wanted, rows_height) =
        client_size(scale, cursor.rows, text_width, width(request.anchor), cursor.scrolls());
    let size = (wanted.min(width(work).max(1)), rows_height);
    let (x, y) = position(request.anchor, size, work);

    let popup = Popup {
        owner,
        list: request.list,
        items: request.items,
        selected: request.selected,
        cursor,
        wheel: Wheel::default(),
        theme,
        canvas,
    };
    let Some(hwnd) = win::create_window(CLASS, "", WS_POPUP, WS_EX_TOOLWINDOW, Some(owner), popup) else {
        return false;
    };
    win::place(hwnd, x, y, size.0, size.1);
    win::show(hwnd);
    true
}

impl Popup {
    fn paint(&mut self, hwnd: HWND) {
        let Popup { canvas, theme, items, cursor, selected, .. } = self;
        canvas.present(hwnd, theme, |p| draw(p, hwnd, items, cursor, *selected));
    }

    /// The entry under a client point.
    fn entry_at(&self, hwnd: HWND, pt: POINT) -> Option<usize> {
        entry_at(self.canvas.scale(), width(win::client_rect(hwnd)), &self.cursor, pt)
    }

    fn choose(&self, hwnd: HWND, index: usize) {
        win::post(self.owner, win::WM_LIST_CHOICE, self.list.code(), index as isize);
        win::post_close(hwnd);
    }

    fn key(&mut self, hwnd: HWND, key: u16) {
        let page = self.cursor.rows.max(1) as isize;
        match key {
            k if k == VK_UP.0 => self.cursor.step(-1),
            k if k == VK_DOWN.0 => self.cursor.step(1),
            k if k == VK_PRIOR.0 => self.cursor.step(-page),
            k if k == VK_NEXT.0 => self.cursor.step(page),
            k if k == VK_HOME.0 => self.cursor.select(0),
            k if k == VK_END.0 => self.cursor.select(usize::MAX),
            k if k == VK_ESCAPE.0 => win::post_close(hwnd),
            k if k == VK_RETURN.0 => match self.cursor.hot {
                Some(index) => self.choose(hwnd, index),
                None => win::post_close(hwnd),
            },
            _ => return,
        }
        win::invalidate(hwnd);
    }

    fn handle(&mut self, hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match message {
            WM_PAINT => self.paint(hwnd),
            WM_ERASEBKGND => return Some(LRESULT(1)),
            WM_MOUSEMOVE => {
                let hot = self.entry_at(hwnd, win::point_from(lparam));
                if hot.is_some() && hot != self.cursor.hot {
                    self.cursor.hot = hot;
                    win::invalidate(hwnd);
                }
            }
            WM_LBUTTONUP => {
                if let Some(index) = self.entry_at(hwnd, win::point_from(lparam)) {
                    self.choose(hwnd, index);
                }
            }
            WM_MOUSEWHEEL => {
                let rows = self.wheel.rows(wparam, WHEEL_ROWS);
                if rows != 0 {
                    self.cursor.scroll(-rows);
                    win::invalidate(hwnd);
                }
            }
            WM_KEYDOWN => self.key(hwnd, win::low_word(wparam) as u16),
            WM_ACTIVATE if win::low_word(wparam) == WA_INACTIVE => win::post_close(hwnd),
            WM_DESTROY => {
                // SAFETY: SendMessageW validates the owner handle; the owner only updates its own state.
                unsafe {
                    SendMessageW(self.owner, win::WM_LIST_CLOSED, WPARAM(self.list.code()), LPARAM(0));
                }
            }
            _ => return None,
        }
        Some(LRESULT(0))
    }
}

/// One entry: highlighted when hot, marked with an accent bar when it is the current one.
fn draw_row(p: &Painter, r: RECT, text: &str, hot: bool, current: bool) {
    let t = p.theme;
    if hot {
        p.fill(r, t.accent);
    } else if current {
        p.fill(RECT { right: r.left + p.scale.px(2), ..r }, t.accent);
    }
    let area = RECT { left: r.left + p.scale.px(TEXT_PAD), right: r.right - p.scale.px(TEXT_PAD), ..r };
    let font = p.fit(text, &VALUE_FONTS, width(area));
    p.text(area, text, font, if hot { t.on_accent } else { t.value }, Align::Left, Overflow::Ellipsis);
}

fn draw(p: &Painter, hwnd: HWND, items: &[String], cursor: &Cursor, selected: Option<usize>) {
    let client = win::client_rect(hwnd);
    p.panel(client, p.theme.field, p.theme.accent);
    for row in 0..cursor.rows {
        let Some(index) = cursor.entry(row) else { break };
        let r = entry_rect(p.scale, width(client), row, cursor.scrolls());
        let text = items.get(index).map_or("", String::as_str);
        draw_row(p, r, text, cursor.hot == Some(index), selected == Some(index));
    }
    let border = p.hairline();
    let track = RECT {
        left: client.right - border - p.scale.px(SCROLL_BAR),
        top: client.top + border,
        right: client.right - border,
        bottom: client.bottom - border,
    };
    widgets::scroll_bar(p, track, cursor.top, cursor.rows, cursor.len);
}

extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: windows of this class are only created by `open`, with a `Popup` as their
    // state; the handlers neither pump messages nor destroy the window.
    unsafe {
        if message == WM_NCDESTROY {
            win::release::<Popup>(hwnd);
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        win::with_state(hwnd, |popup: &mut Popup| popup.handle(hwnd, message, wparam, lparam))
            .flatten()
            .unwrap_or_else(|| DefWindowProcW(hwnd, message, wparam, lparam))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_lists_show_every_entry() {
        let cursor = Cursor::new(4, Some(2));
        assert_eq!((cursor.rows, cursor.top, cursor.hot), (4, 0, Some(2)));
        assert_eq!(cursor.entry(3), Some(3));
        assert_eq!(cursor.entry(4), None);
        assert_eq!(Cursor::new(3, Some(9)).hot, None, "a selection outside the list is ignored");
    }

    #[test]
    fn long_lists_open_around_the_selection() {
        let cursor = Cursor::new(64, Some(40));
        assert_eq!(cursor.rows, MAX_ROWS);
        assert_eq!(cursor.top, 40 - MAX_ROWS / 2);
        assert_eq!(Cursor::new(64, Some(63)).top, 64 - MAX_ROWS);
        assert_eq!(Cursor::new(64, None).top, 0);
    }

    #[test]
    fn keyboard_steps_keep_the_highlight_visible() {
        let mut cursor = Cursor::new(64, None);
        cursor.step(1);
        assert_eq!(cursor.hot, Some(0));
        cursor.step(-1);
        assert_eq!(cursor.hot, Some(0), "stops at the first entry");
        cursor.step(MAX_ROWS as isize);
        assert_eq!((cursor.hot, cursor.top), (Some(MAX_ROWS), 1));
        cursor.select(usize::MAX);
        assert_eq!((cursor.hot, cursor.top), (Some(63), 64 - MAX_ROWS));
        cursor.step(5);
        assert_eq!(cursor.hot, Some(63), "stops at the last entry");
        cursor.select(0);
        assert_eq!((cursor.hot, cursor.top), (Some(0), 0));

        let mut from_end = Cursor::new(5, None);
        from_end.step(-1);
        assert_eq!(from_end.hot, Some(4));
    }

    #[test]
    fn wheel_scrolling_is_clamped() {
        let mut cursor = Cursor::new(20, Some(0));
        cursor.scroll(3);
        assert_eq!((cursor.top, cursor.hot), (3, Some(0)));
        cursor.scroll(100);
        assert_eq!(cursor.top, 20 - MAX_ROWS);
        cursor.scroll(-100);
        assert_eq!(cursor.top, 0);
        let mut short = Cursor::new(3, None);
        short.scroll(5);
        assert_eq!(short.top, 0);
    }

    #[test]
    fn empty_cursor_is_inert() {
        let mut cursor = Cursor::new(0, None);
        cursor.step(1);
        cursor.select(3);
        cursor.scroll(2);
        assert_eq!((cursor.rows, cursor.top, cursor.hot), (0, 0, None));
        assert_eq!(cursor.entry(0), None);
    }

    #[test]
    fn list_size_and_rows() {
        let scale = Scale::new(96);
        let (w, h) = client_size(scale, 4, 50, 100, false);
        assert_eq!((w, h), (100, 4 * 18 + 2));
        let (w, _) = client_size(scale, 12, 200, 100, true);
        assert_eq!(w, 200 + 14 + 4 + 2);
        let first = row_rect(scale, 100, 0);
        let second = row_rect(scale, 100, 1);
        assert_eq!((first.left, first.top, first.right), (1, 1, 99));
        assert_eq!(first.bottom, second.top);
    }

    #[test]
    fn scroll_indicator_is_not_part_of_the_entries() {
        let scale = Scale::new(96);
        let on_second_row = |x: i32| POINT { x, y: row_rect(scale, 100, 1).top + 2 };
        let long = Cursor::new(16, Some(8));
        assert_eq!(long.top, 2);
        assert_eq!(entry_at(scale, 100, &long, on_second_row(50)), Some(3));
        assert_eq!(entry_at(scale, 100, &long, on_second_row(94)), Some(3));
        for x in 95..100 {
            assert_eq!(entry_at(scale, 100, &long, on_second_row(x)), None, "x {x}");
        }
        // The same strip belongs to the entries of a list that shows everything.
        let short = Cursor::new(4, None);
        assert_eq!(entry_at(scale, 100, &short, on_second_row(97)), Some(1));
        assert_eq!(entry_at(scale, 100, &short, on_second_row(99)), None, "the border");
        assert_eq!(entry_at(scale, 100, &short, POINT { x: 50, y: 500 }), None);

        let wide = Scale::new(192);
        assert_eq!(entry_rect(wide, 200, 0, true).right, 200 - 2 - 8);
        assert_eq!(entry_rect(wide, 200, 0, false), row_rect(wide, 200, 0));
    }

    #[test]
    fn list_drops_down_or_flips_up() {
        let work = RECT { left: 0, top: 0, right: 1000, bottom: 800 };
        let anchor = RECT { left: 100, top: 300, right: 200, bottom: 320 };
        assert_eq!(position(anchor, (150, 200), work), (100, 320));
        let low = RECT { left: 100, top: 700, right: 200, bottom: 720 };
        assert_eq!(position(low, (150, 200), work), (100, 500));
        let right = RECT { left: 950, top: 300, right: 990, bottom: 320 };
        assert_eq!(position(right, (150, 200), work), (850, 320));
        let cramped = RECT { left: 0, top: 100, right: 50, bottom: 120 };
        assert_eq!(position(cramped, (150, 790), work), (0, 120), "stays below when neither side fits");
    }
}
