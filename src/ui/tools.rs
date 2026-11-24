//! The tool windows: the live clock table and the timer comparison.
//!
//! Both are small owned windows with the same chrome (title strip, close box,
//! a row of buttons), so the window mechanics live in the generic
//! [`ToolWindow`] and each tool only supplies its content.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_NEXT, VK_PRIOR, VK_RETURN, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, KillTimer, SendMessageW, SetForegroundWindow, SetTimer, CS_DROPSHADOW, WM_CAPTURECHANGED,
    WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NCDESTROY, WM_NCHITTEST, WM_PAINT, WM_TIMER, WS_POPUP, WS_SYSMENU,
};

use crate::config::Config;
use crate::hw::timers::{TimerSet, TIMER_COUNT};
use crate::model::{ClockSnapshot, Sensor};

use super::format;
use super::gdi::{Align, FontId, Painter};
use super::geom::{contains, dlu, height, width, Dlu, Scale};
use super::layout::{clocks_tool, sensors_tool, timers_tool, tool_close_box, TOOL_TITLE_HEIGHT};
use super::render;
use super::theme::Theme;
use super::widgets::{self, Glyph, Mood};
use super::win::{self, Canvas, Wheel};

/// Identifier of the repaint timer of an animated tool.
const REFRESH_TIMER: usize = 1;
/// Rows scrolled per wheel notch in the clock table.
const WHEEL_ROWS: isize = 3;

/// What a tool wants after one of its buttons was pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    Redraw,
    Close,
}

/// The content of a tool window.
pub trait Tool: 'static {
    const CLASS: PCWSTR;
    const TITLE: &'static str;
    /// Client size below the title strip, in dialog units.
    const SIZE: (i32, i32);
    /// Number passed to the owner when the window closes.
    const CODE: usize;

    /// Rectangle (below the title strip) and caption of every button.
    fn buttons(&self) -> Vec<(Dlu, &'static str)>;
    /// Draws everything except the chrome and the buttons.
    fn draw(&self, p: &Painter);
    /// Reacts to the button with index `button`.
    fn press(&mut self, button: usize) -> Response;
    /// The button Enter activates.
    fn default_button(&self) -> usize;
    /// Scrolls by `rows`; returns whether anything changed.
    fn scroll(&mut self, _rows: isize) -> bool {
        false
    }
    /// Rows the Page Up and Page Down keys scroll by.
    fn page_rows(&self) -> isize {
        0
    }
    /// Milliseconds between repaints while the content changes on its own.
    fn refresh_interval(&self) -> Option<u32> {
        None
    }
}

/// A rectangle given below the title strip, in window coordinates.
fn body(rect: Dlu) -> Dlu {
    rect.offset(0, TOOL_TITLE_HEIGHT)
}

/// Pixel size of the window of tool `T`.
pub fn window_size<T: Tool>(scale: Scale) -> (i32, i32) {
    scale.size(T::SIZE.0, T::SIZE.1 + TOOL_TITLE_HEIGHT)
}

/// The clickable parts of a tool window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    CloseBox,
    Button(usize),
}

fn hit_test<T: Tool>(scale: Scale, tool: &T, pt: POINT) -> Option<Hit> {
    if contains(scale.rect(tool_close_box(T::SIZE.0)), pt) {
        return Some(Hit::CloseBox);
    }
    tool.buttons().iter().position(|(rect, _)| contains(scale.rect(body(*rect)), pt)).map(Hit::Button)
}

fn on_title_strip<T: Tool>(scale: Scale, pt: POINT) -> bool {
    let strip = scale.rect(dlu(0, 0, T::SIZE.0, TOOL_TITLE_HEIGHT));
    contains(strip, pt) && !contains(scale.rect(tool_close_box(T::SIZE.0)), pt)
}

/// Paints a complete tool window. `hot` and `pressed` describe the mouse feedback.
pub fn draw<T: Tool>(p: &Painter, tool: &T, hot: Option<Hit>, pressed: Option<Hit>) {
    let (window_width, window_height) = window_size::<T>(p.scale);
    let window = RECT { left: 0, top: 0, right: window_width, bottom: window_height };
    let mood = |hit: Hit| Mood::of(true, hot == Some(hit), pressed == Some(hit));
    p.fill(window, p.theme.window);
    render::title_strip(p, window_width, TOOL_TITLE_HEIGHT, T::TITLE, FontId::Strong);
    widgets::title_button(p, p.scale.rect(tool_close_box(T::SIZE.0)), Glyph::Close, mood(Hit::CloseBox));
    tool.draw(p);
    for (index, (rect, text)) in tool.buttons().into_iter().enumerate() {
        widgets::button(p, p.scale.rect(body(rect)), text, mood(Hit::Button(index)), false);
    }
    p.outline(window, p.theme.outline);
}

/// Rows a navigation key scrolls the content by, for a page of `page` rows.
/// Home and End ask for more than any table holds and rely on the tool to clamp.
fn key_scroll(key: u16, page: isize) -> Option<isize> {
    Some(match key {
        key if key == VK_UP.0 => -1,
        key if key == VK_DOWN.0 => 1,
        key if key == VK_PRIOR.0 => -page,
        key if key == VK_NEXT.0 => page,
        key if key == VK_HOME.0 => isize::MIN,
        key if key == VK_END.0 => isize::MAX,
        _ => return None,
    })
}

struct ToolWindow<T: Tool> {
    owner: HWND,
    cfg: Config,
    theme: Theme,
    canvas: Canvas,
    hot: Option<Hit>,
    pressed: Option<Hit>,
    tracking: bool,
    ticking: bool,
    wheel: Wheel,
    tool: T,
}

/// What the window procedure does after the state has been released.
enum Reply {
    Default,
    Done(isize),
    /// Move and resize the window (after a DPI change).
    Place(RECT),
}

impl<T: Tool> ToolWindow<T> {
    fn paint(&mut self, hwnd: HWND) {
        let ToolWindow { canvas, theme, tool, hot, pressed, .. } = self;
        canvas.present(hwnd, theme, |p| draw(p, tool, *hot, *pressed));
    }

    /// Starts or stops the repaint timer to match what the tool asks for.
    fn sync_timer(&mut self, hwnd: HWND) {
        let interval = self.tool.refresh_interval();
        // SAFETY: the timer belongs to `hwnd`, which is alive while its state is in use.
        unsafe {
            match interval {
                Some(milliseconds) if !self.ticking => {
                    self.ticking = SetTimer(hwnd, REFRESH_TIMER, milliseconds, None) != 0;
                }
                None if self.ticking => {
                    let _ = KillTimer(hwnd, REFRESH_TIMER);
                    self.ticking = false;
                }
                _ => {}
            }
        }
    }

    fn press(&mut self, hwnd: HWND, hit: Hit) {
        let response = match hit {
            Hit::CloseBox => Response::Close,
            Hit::Button(index) => self.tool.press(index),
        };
        match response {
            Response::Close => win::post_close(hwnd),
            Response::Redraw => {
                self.sync_timer(hwnd);
                win::invalidate(hwnd);
            }
        }
    }

    fn hover(&mut self, hwnd: HWND, pt: POINT) {
        let hot = hit_test(self.canvas.scale(), &self.tool, pt);
        if hot != self.hot {
            self.hot = hot;
            win::invalidate(hwnd);
        }
        if !self.tracking {
            self.tracking = win::track_mouse_leave(hwnd);
        }
    }

    fn button_down(&mut self, hwnd: HWND, pt: POINT) {
        self.pressed = hit_test(self.canvas.scale(), &self.tool, pt);
        if self.pressed.is_some() {
            win::capture_mouse(hwnd);
            win::invalidate(hwnd);
        }
    }

    fn button_up(&mut self, hwnd: HWND, pt: POINT) {
        let Some(pressed) = self.pressed.take() else { return };
        win::release_mouse();
        win::invalidate(hwnd);
        if hit_test(self.canvas.scale(), &self.tool, pt) == Some(pressed) {
            self.press(hwnd, pressed);
        }
    }

    fn key(&mut self, hwnd: HWND, key: u16) -> Reply {
        if let Some(rows) = key_scroll(key, self.tool.page_rows()) {
            if self.tool.scroll(rows) {
                win::invalidate(hwnd);
            }
            return Reply::Done(0);
        }
        match key {
            key if key == VK_ESCAPE.0 => win::post_close(hwnd),
            key if key == VK_RETURN.0 => self.press(hwnd, Hit::Button(self.tool.default_button())),
            _ => return Reply::Default,
        }
        Reply::Done(0)
    }

    /// Stops the repaint timer and tells the owner that the window is gone.
    fn destroyed(&mut self, hwnd: HWND) {
        // SAFETY: the timer was started by `sync_timer` on this window; SendMessageW validates
        // the owner handle and the owner only updates its own state.
        unsafe {
            if self.ticking {
                let _ = KillTimer(hwnd, REFRESH_TIMER);
            }
            SendMessageW(self.owner, win::WM_TOOL_CLOSED, WPARAM(T::CODE), LPARAM(0));
        }
    }

    fn handle(&mut self, hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Reply {
        match message {
            WM_PAINT => self.paint(hwnd),
            WM_ERASEBKGND => return Reply::Done(1),
            WM_NCHITTEST => {
                let pt = win::to_client(hwnd, win::point_from(lparam));
                return Reply::Done(win::hit_code(on_title_strip::<T>(self.canvas.scale(), pt)));
            }
            WM_MOUSEMOVE => self.hover(hwnd, win::point_from(lparam)),
            WM_MOUSELEAVE => {
                self.tracking = false;
                self.hot = None;
                win::invalidate(hwnd);
            }
            WM_LBUTTONDOWN => self.button_down(hwnd, win::point_from(lparam)),
            WM_LBUTTONUP => self.button_up(hwnd, win::point_from(lparam)),
            WM_CAPTURECHANGED => {
                self.pressed = None;
                win::invalidate(hwnd);
            }
            WM_MOUSEWHEEL => {
                if self.tool.scroll(-self.wheel.rows(wparam, WHEEL_ROWS)) {
                    win::invalidate(hwnd);
                }
            }
            WM_KEYDOWN => return self.key(hwnd, win::low_word(wparam) as u16),
            WM_TIMER => win::invalidate(hwnd),
            WM_DPICHANGED => {
                self.canvas.set_dpi(&self.cfg, win::high_word(wparam));
                win::invalidate(hwnd);
                let size = window_size::<T>(self.canvas.scale());
                // SAFETY: this is the `lparam` of the WM_DPICHANGED message being handled.
                return Reply::Place(unsafe { win::suggested_placement(lparam, size) });
            }
            WM_DESTROY => self.destroyed(hwnd),
            _ => return Reply::Default,
        }
        Reply::Done(0)
    }
}

extern "system" fn procedure<T: Tool>(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: windows of class `T::CLASS` are only created by `open::<T>`, with a
    // `ToolWindow<T>` as their state; the handlers neither pump messages nor destroy the window.
    unsafe {
        if message == WM_NCDESTROY {
            win::release::<ToolWindow<T>>(hwnd);
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        let reply =
            win::with_state(hwnd, |window: &mut ToolWindow<T>| window.handle(hwnd, message, wparam, lparam));
        match reply.unwrap_or(Reply::Default) {
            Reply::Default => DefWindowProcW(hwnd, message, wparam, lparam),
            Reply::Done(result) => LRESULT(result),
            Reply::Place(r) => {
                win::place(hwnd, r.left, r.top, width(r), height(r));
                LRESULT(0)
            }
        }
    }
}

/// Registers the window class of tool `T`.
pub fn register<T: Tool>() -> bool {
    win::register_class(T::CLASS, CS_DROPSHADOW, procedure::<T>, None)
}

/// Opens the window of `tool` centred over `owner`.
pub fn open<T: Tool>(owner: HWND, cfg: &Config, tool: T) -> Option<HWND> {
    let dpi = win::dpi_of(owner);
    let canvas = Canvas::new(cfg, dpi);
    let (window_width, window_height) = window_size::<T>(canvas.scale());
    let window = ToolWindow {
        owner,
        cfg: cfg.clone(),
        theme: Theme::new(cfg),
        canvas,
        hot: None,
        pressed: None,
        tracking: false,
        ticking: false,
        wheel: Wheel::default(),
        tool,
    };
    let hwnd = win::create_window(
        T::CLASS,
        T::TITLE,
        WS_POPUP | WS_SYSMENU,
        Default::default(),
        Some(owner),
        window,
    )?;
    // Centred over the owner, but never off the owner's monitor.
    let over = win::window_rect(owner);
    let area = win::work_area_near(over).unwrap_or(over);
    let (x, y) =
        win::kept_in(area, win::centred_in(over, window_width, window_height), window_width, window_height);
    win::place(hwnd, x, y, window_width, window_height);
    win::show(hwnd);
    Some(hwnd)
}

/// Brings an already open tool window to the front.
pub fn focus(hwnd: HWND) {
    // SAFETY: SetForegroundWindow validates the handle.
    unsafe {
        let _ = SetForegroundWindow(hwnd);
    }
}

// ---------------------------------------------------------------------------
// Clocks
// ---------------------------------------------------------------------------

/// One line of the clock table: device, clock speed, multiplier.
pub type ClockRow = [String; 3];

/// The table shown by the Clocks tool: the bus clock followed by every core.
pub fn clock_rows(clocks: &ClockSnapshot) -> Vec<ClockRow> {
    let bus = clocks.bus_mhz.map(format::mhz2).unwrap_or_default();
    let cores = clocks.cores.iter().map(|core| {
        [
            format::core_label(core.kind, core.core),
            format::mhz2(core.mhz),
            core.multiplier.map(format::multiplier2).unwrap_or_default(),
        ]
    });
    std::iter::once(["CPU BCLK".to_string(), bus, String::new()]).chain(cores).collect()
}

/// The Clocks tool: a live table of the bus clock and the clock of every core.
pub struct ClocksTool {
    rows: Vec<ClockRow>,
    /// Index of the first visible row.
    top: usize,
}

impl ClocksTool {
    pub fn new(clocks: &ClockSnapshot) -> Self {
        ClocksTool { rows: clock_rows(clocks), top: 0 }
    }

    /// Largest valid scroll position.
    fn max_top(&self) -> usize {
        self.rows.len().saturating_sub(clocks_tool::visible_rows())
    }

    /// Replaces the table with a fresh sample, keeping the scroll position where possible.
    fn refresh(&mut self, clocks: &ClockSnapshot) {
        self.rows = clock_rows(clocks);
        self.top = self.top.min(self.max_top());
    }
}

/// Column titles and widths of a three-column table.
type Columns = [(&'static str, i32); 3];

/// Horizontal pixel spans of the text in the three table columns.
fn column_spans(scale: Scale, columns: &Columns) -> [(i32, i32); 3] {
    let pad = scale.px(6);
    let mut left = body(clocks_tool::TABLE).x;
    columns.map(|(_, column_width)| {
        let span = (scale.x(left) + pad, scale.x(left + column_width) - pad);
        left += column_width;
        span
    })
}

/// Draws a scrolling table: frame, header, the rows from `top` on and the scroll bar.
fn draw_table(p: &Painter, columns: &Columns, rows: &[[String; 3]], top: usize) {
    let table = body(clocks_tool::TABLE);
    let frame = p.scale.rect(table);
    let header_bottom = p.scale.y(table.y + clocks_tool::HEADER_HEIGHT);
    p.panel(frame, p.theme.panel, p.theme.frame);
    draw_header(p, columns, frame, header_bottom);
    draw_rows(p, columns, rows, top);
    let line = p.hairline();
    let track = RECT {
        left: frame.right - line - p.scale.px(widgets::SCROLL_BAR),
        top: header_bottom + line,
        right: frame.right - line,
        bottom: frame.bottom - line,
    };
    widgets::scroll_bar(p, track, top, clocks_tool::visible_rows(), rows.len());
}

/// The scroll position `rows` away from `top`, kept inside a table of `len` rows.
fn scrolled(top: usize, rows: isize, len: usize) -> usize {
    top.saturating_add_signed(rows).min(len.saturating_sub(clocks_tool::visible_rows()))
}

/// Header band with the column titles, and the lines between the columns.
fn draw_header(p: &Painter, columns: &Columns, frame: RECT, header_bottom: i32) {
    let t = p.theme;
    let line = p.hairline();
    let band = RECT {
        left: frame.left + line,
        top: frame.top + line,
        right: frame.right - line,
        bottom: header_bottom,
    };
    p.fill(band, t.control);
    p.fill(RECT { top: header_bottom, bottom: header_bottom + line, ..frame }, t.frame);
    let pad = p.scale.px(6);
    for ((title, _), (left, right)) in columns.iter().zip(column_spans(p.scale, columns)) {
        widgets::note(p, RECT { left, right, ..band }, title, Align::Left);
        let separator = left - pad;
        if separator > frame.left {
            p.fill(RECT { left: separator, right: separator + line, ..frame }, t.frame);
        }
    }
}

/// The visible rows: names in the label colour, readings in the value colour.
fn draw_rows(p: &Painter, columns: &Columns, rows: &[[String; 3]], top: usize) {
    let first = body(clocks_tool::TABLE).y + clocks_tool::HEADER_HEIGHT + 1;
    let spans = column_spans(p.scale, columns);
    for (slot, row) in rows.iter().skip(top).take(clocks_tool::visible_rows()).enumerate() {
        let top = p.scale.y(first + clocks_tool::ROW_HEIGHT * slot as i32);
        let bottom = p.scale.y(first + clocks_tool::ROW_HEIGHT * (slot as i32 + 1));
        for (index, (text, (left, right))) in row.iter().zip(spans).enumerate() {
            let cell = RECT { left, top, right, bottom };
            if index == 0 {
                widgets::note(p, cell, text, Align::Left);
            } else {
                widgets::plain_value(p, cell, text, Align::Left);
            }
        }
    }
}

impl Tool for ClocksTool {
    const CLASS: PCWSTR = w!("RsCpuzClocks");
    const TITLE: &'static str = "Clocks";
    const SIZE: (i32, i32) = clocks_tool::SIZE;
    const CODE: usize = 1;

    fn buttons(&self) -> Vec<(Dlu, &'static str)> {
        vec![(clocks_tool::OK, "OK")]
    }

    fn default_button(&self) -> usize {
        0
    }

    fn press(&mut self, _button: usize) -> Response {
        Response::Close
    }

    fn scroll(&mut self, rows: isize) -> bool {
        let top = scrolled(self.top, rows, self.rows.len());
        let changed = top != self.top;
        self.top = top;
        changed
    }

    fn page_rows(&self) -> isize {
        clocks_tool::visible_rows() as isize
    }

    fn draw(&self, p: &Painter) {
        draw_table(p, &clocks_tool::COLUMNS, &self.rows, self.top);
    }
}

/// Feeds a fresh clock sample to an open Clocks window.
pub fn update_clocks(hwnd: HWND, clocks: &ClockSnapshot) {
    // SAFETY: `hwnd` was returned by `open::<ClocksTool>` and the owner forgets it when the
    // window reports its destruction, so its state is a `ToolWindow<ClocksTool>`.
    let updated =
        unsafe { win::with_state(hwnd, |window: &mut ToolWindow<ClocksTool>| window.tool.refresh(clocks)) };
    if updated.is_some() {
        win::invalidate(hwnd);
    }
}

// ---------------------------------------------------------------------------
// Sensors
// ---------------------------------------------------------------------------

/// The Sensors tool: a live table of every sensor with its reading and the highest
/// reading since the window opened.
pub struct SensorsTool {
    rows: Vec<[String; 3]>,
    /// The highest reading of each sensor so far, by group and name.
    highest: Vec<Sensor>,
    /// Index of the first visible row.
    top: usize,
}

impl SensorsTool {
    pub fn new(sensors: &[Sensor]) -> Self {
        let mut tool = SensorsTool { rows: Vec::new(), highest: Vec::new(), top: 0 };
        tool.refresh(sensors);
        tool
    }

    /// Takes a fresh reading of every sensor, keeping the scroll position where possible.
    fn refresh(&mut self, sensors: &[Sensor]) {
        for sensor in sensors {
            let same = |known: &&mut Sensor| known.group == sensor.group && known.name == sensor.name;
            match self.highest.iter_mut().find(same) {
                Some(known) => known.value = known.value.max(sensor.value),
                None => self.highest.push(sensor.clone()),
            }
        }
        self.rows = sensors
            .iter()
            .map(|sensor| {
                let highest = self
                    .highest
                    .iter()
                    .find(|known| known.group == sensor.group && known.name == sensor.name);
                [
                    format!("{} {}", sensor.group, sensor.name),
                    sensor.value_text(),
                    highest.map(Sensor::value_text).unwrap_or_default(),
                ]
            })
            .collect();
        if self.rows.is_empty() {
            self.rows.push(["No sensor readings".to_string(), String::new(), String::new()]);
        }
        self.top = scrolled(self.top, 0, self.rows.len());
    }
}

impl Tool for SensorsTool {
    const CLASS: PCWSTR = w!("RsCpuzSensors");
    const TITLE: &'static str = "Sensors";
    const SIZE: (i32, i32) = sensors_tool::SIZE;
    const CODE: usize = 3;

    fn buttons(&self) -> Vec<(Dlu, &'static str)> {
        vec![(sensors_tool::OK, "OK")]
    }

    fn default_button(&self) -> usize {
        0
    }

    fn press(&mut self, _button: usize) -> Response {
        Response::Close
    }

    fn scroll(&mut self, rows: isize) -> bool {
        let top = scrolled(self.top, rows, self.rows.len());
        let changed = top != self.top;
        self.top = top;
        changed
    }

    fn page_rows(&self) -> isize {
        clocks_tool::visible_rows() as isize
    }

    fn draw(&self, p: &Painter) {
        draw_table(p, &sensors_tool::COLUMNS, &self.rows, self.top);
    }
}

/// Feeds a fresh sensor reading to an open Sensors window.
pub fn update_sensors(hwnd: HWND, sensors: &[Sensor]) {
    // SAFETY: `hwnd` was returned by `open::<SensorsTool>` and the owner forgets it when the
    // window reports its destruction, so its state is a `ToolWindow<SensorsTool>`.
    let updated =
        unsafe { win::with_state(hwnd, |window: &mut ToolWindow<SensorsTool>| window.tool.refresh(sensors)) };
    if updated.is_some() {
        win::invalidate(hwnd);
    }
}

// ---------------------------------------------------------------------------
// Timers
// ---------------------------------------------------------------------------

/// Repaint interval while the timers run, in milliseconds.
const TIMER_REFRESH_MS: u32 = 100;

/// Column titles: the names of the time sources, or `Timer N` where one has no name.
pub fn timer_headings(names: [&str; TIMER_COUNT]) -> [String; TIMER_COUNT] {
    let mut index = 0;
    names.map(|name| {
        index += 1;
        if name.is_empty() {
            format!("Timer {index}")
        } else {
            name.to_string()
        }
    })
}

/// Drift of every time source against the first one, in seconds.
pub fn timer_drift(elapsed: [f64; TIMER_COUNT]) -> [f64; TIMER_COUNT] {
    let reference = elapsed.first().copied().unwrap_or(0.0);
    elapsed.map(|seconds| seconds - reference)
}

/// The Timers tool: several time sources run side by side so drift between them shows.
#[derive(Default)]
pub struct TimersTool {
    timers: TimerSet,
}

impl Tool for TimersTool {
    const CLASS: PCWSTR = w!("RsCpuzTimers");
    const TITLE: &'static str = "Timers";
    const SIZE: (i32, i32) = timers_tool::SIZE;
    const CODE: usize = 2;

    fn buttons(&self) -> Vec<(Dlu, &'static str)> {
        let toggle = if self.timers.is_running() { "Stop" } else { "Start" };
        vec![(timers_tool::START, toggle), (timers_tool::RESET, "Reset"), (timers_tool::CLOSE, "Close")]
    }

    fn default_button(&self) -> usize {
        0
    }

    fn press(&mut self, button: usize) -> Response {
        match button {
            0 if self.timers.is_running() => self.timers.stop(),
            0 => self.timers.start(),
            1 => self.timers.reset(),
            _ => return Response::Close,
        }
        Response::Redraw
    }

    fn refresh_interval(&self) -> Option<u32> {
        self.timers.is_running().then_some(TIMER_REFRESH_MS)
    }

    fn draw(&self, p: &Painter) {
        let place = |rect: &Dlu| p.scale.rect(body(*rect));
        let elapsed = self.timers.elapsed();
        for (heading, rect) in timer_headings(self.timers.names()).iter().zip(&timers_tool::HEADINGS) {
            widgets::note(p, place(rect), heading, Align::Centre);
        }
        for (seconds, rect) in elapsed.iter().zip(&timers_tool::ELAPSED) {
            widgets::value(p, place(rect), &format::seconds(*seconds), Align::Centre);
        }
        for (seconds, rect) in timer_drift(elapsed).iter().zip(&timers_tool::DRIFT) {
            widgets::value(p, place(rect), &format::drift_ms(*seconds), Align::Centre);
        }
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY;

    use super::*;
    use crate::demo;
    use crate::model::{CoreClock, CoreKind, SensorKind};

    #[test]
    fn clock_table_lists_the_bus_and_every_core() {
        let rows = clock_rows(&demo::clocks());
        assert_eq!(rows.len(), 9);
        assert_eq!(rows[0], ["CPU BCLK", "99.74 MHz", ""]);
        assert_eq!(rows[1], ["Core #0", "5136.63 MHz", "x 51.50"]);
        assert_eq!(rows[8][0], "Core #7");
    }

    #[test]
    fn clock_table_handles_unknown_values_and_hybrid_cores() {
        let clocks = ClockSnapshot {
            cores: vec![
                CoreClock { core: 0, kind: CoreKind::Performance, mhz: 5000.0, multiplier: None },
                CoreClock { core: 8, kind: CoreKind::Efficient, mhz: 3900.0, multiplier: Some(39.0) },
            ],
            ..Default::default()
        };
        let rows = clock_rows(&clocks);
        assert_eq!(rows[0], ["CPU BCLK", "", ""]);
        assert_eq!(rows[1], ["P-Core #0", "5000.00 MHz", ""]);
        assert_eq!(rows[2], ["E-Core #8", "3900.00 MHz", "x 39.00"]);
        assert_eq!(clock_rows(&ClockSnapshot::default()).len(), 1);
    }

    #[test]
    fn sensor_table_lists_readings_and_the_highest_seen() {
        let sensor = |group: &str, name: &str, kind, value| Sensor {
            group: group.into(),
            name: name.into(),
            kind,
            value,
        };
        let mut tool = SensorsTool::new(&[
            sensor("CPU", "Package", SensorKind::Power, 21.5),
            sensor("ACPI", "TZ00", SensorKind::Temperature, 56.05),
        ]);
        assert_eq!(tool.rows, [["CPU Package", "21.50 W", "21.50 W"], ["ACPI TZ00", "56.0 °C", "56.0 °C"]]);
        tool.refresh(&[
            sensor("CPU", "Package", SensorKind::Power, 30.0),
            sensor("ACPI", "TZ00", SensorKind::Temperature, 50.0),
        ]);
        assert_eq!(tool.rows, [["CPU Package", "30.00 W", "30.00 W"], ["ACPI TZ00", "50.0 °C", "56.0 °C"]]);
        assert_eq!(SensorsTool::new(&[]).rows, [["No sensor readings", "", ""]]);
        assert_eq!(tool.press(0), Response::Close);
    }

    #[test]
    fn clock_table_scrolls_within_bounds() {
        let many = ClockSnapshot {
            cores: (0..64).map(|core| CoreClock { core, mhz: 3000.0, ..Default::default() }).collect(),
            ..Default::default()
        };
        let mut tool = ClocksTool::new(&many);
        let visible = clocks_tool::visible_rows();
        assert_eq!(tool.max_top(), 65 - visible);
        assert!(tool.scroll(3));
        assert_eq!(tool.top, 3);
        assert!(tool.scroll(1000));
        assert_eq!(tool.top, 65 - visible);
        assert!(!tool.scroll(1));
        tool.refresh(&demo::clocks());
        assert_eq!(tool.top, 0, "a shorter table pulls the scroll position back");
        assert!(!tool.scroll(-1));
        assert_eq!(tool.press(0), Response::Close);
    }

    #[test]
    fn clock_table_scrolls_with_the_keyboard() {
        let many = ClockSnapshot {
            cores: (0..64).map(|core| CoreClock { core, mhz: 3000.0, ..Default::default() }).collect(),
            ..Default::default()
        };
        let mut tool = ClocksTool::new(&many);
        let page = tool.page_rows();
        assert_eq!(page, clocks_tool::visible_rows() as isize);
        let mut press = |key: VIRTUAL_KEY| {
            let rows = key_scroll(key.0, page).expect("a navigation key");
            tool.scroll(rows);
            tool.top
        };
        assert_eq!(press(VK_DOWN), 1);
        assert_eq!(press(VK_NEXT), 1 + clocks_tool::visible_rows());
        assert_eq!(press(VK_UP), clocks_tool::visible_rows());
        assert_eq!(press(VK_END), 65 - clocks_tool::visible_rows());
        assert_eq!(press(VK_NEXT), 65 - clocks_tool::visible_rows(), "stops at the last row");
        assert_eq!(press(VK_PRIOR), 65 - 2 * clocks_tool::visible_rows());
        assert_eq!(press(VK_HOME), 0);
        assert_eq!(press(VK_UP), 0, "stops at the first row");
        assert_eq!(key_scroll(VK_RETURN.0, page), None);
        assert_eq!(key_scroll(VK_ESCAPE.0, page), None);

        // A tool without a table ignores the keys.
        let mut timers = TimersTool::default();
        assert_eq!(key_scroll(VK_NEXT.0, timers.page_rows()), Some(0));
        assert!(!timers.scroll(isize::MAX));
    }

    #[test]
    fn timer_columns() {
        assert_eq!(timer_headings(["QPC", "", "TSC", ""]), ["QPC", "Timer 2", "TSC", "Timer 4"]);
        let drift = timer_drift([10.0, 10.5, 9.75, 10.0]);
        assert_eq!(drift, [0.0, 0.5, -0.25, 0.0]);
    }

    #[test]
    fn tool_windows_add_the_title_strip() {
        let scale = Scale::new(96);
        assert_eq!(window_size::<ClocksTool>(scale), scale.size(275, 215 + TOOL_TITLE_HEIGHT));
        assert_eq!(window_size::<TimersTool>(scale), scale.size(195, 78 + TOOL_TITLE_HEIGHT));
        assert_eq!(body(dlu(4, 62, 50, 13)), dlu(4, 62 + TOOL_TITLE_HEIGHT, 50, 13));
    }

    #[test]
    fn tool_hit_testing() {
        let scale = Scale::new(96);
        let tool = ClocksTool::new(&demo::clocks());
        let centre = |rect: Dlu| {
            let r = scale.rect(rect);
            POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 }
        };
        assert_eq!(hit_test(scale, &tool, centre(body(clocks_tool::OK))), Some(Hit::Button(0)));
        assert_eq!(hit_test(scale, &tool, centre(tool_close_box(275))), Some(Hit::CloseBox));
        assert_eq!(hit_test(scale, &tool, centre(body(clocks_tool::TABLE))), None);
        assert!(on_title_strip::<ClocksTool>(scale, POINT { x: 40, y: 8 }));
        assert!(!on_title_strip::<ClocksTool>(scale, centre(tool_close_box(275))));
        assert!(!on_title_strip::<ClocksTool>(scale, centre(body(clocks_tool::OK))));
    }
}
