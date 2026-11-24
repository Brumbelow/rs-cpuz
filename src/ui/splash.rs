//! The splash window shown while the hardware is detected.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::UpdateWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostQuitMessage, TranslateMessage, MSG,
    WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_NCDESTROY, WM_PAINT, WNDCLASS_STYLES, WS_EX_APPWINDOW,
    WS_POPUP,
};

use crate::brand;
use crate::config::Config;
use crate::hw::Stage;
use crate::model::{ClockSnapshot, SystemInfo};

use super::events::{Event, Inbox, Mailbox};
use super::format;
use super::gdi::{Align, FontId, Overflow, Painter};
use super::geom::{height, width, Scale, BASE_DPI};
use super::layout::splash;
use super::theme::Theme;
use super::widgets;
use super::win::{self, Canvas, Icons};

const CLASS: PCWSTR = w!("RsCpuzSplash");

struct Splash {
    theme: Theme,
    canvas: Canvas,
    stage: Stage,
}

/// Registers the window class of the splash.
pub fn register(icons: &Icons) -> bool {
    win::register_class(CLASS, WNDCLASS_STYLES::default(), procedure, Some(icons))
}

/// Paints the splash for detection stage `stage`.
pub fn draw(p: &Painter, stage: Stage) {
    let t = p.theme;
    let (window_width, window_height) = p.scale.size(splash::SIZE.0, splash::SIZE.1);
    let window = RECT { left: 0, top: 0, right: window_width, bottom: window_height };
    p.fill(window, t.window);

    // Logo mark and name, centred as a pair.
    let art = p.scale.rect(splash::ARTWORK);
    let side = height(art) * 2 / 3;
    let gap = p.scale.px(10);
    let name_width = p.text_width(brand::APP_NAME, FontId::Large);
    let left = art.left + (width(art) - side - gap - name_width) / 2;
    let top = art.top + (height(art) - side) / 2;
    brand::draw_mark(p.hdc, RECT { left, top, right: left + side, bottom: top + side }, t.accent);
    let name = RECT { left: left + side + gap, ..art };
    p.text(name, brand::APP_NAME, FontId::Large, t.chrome, Align::Left, Overflow::Visible);

    widgets::progress(p, p.scale.rect(splash::PROGRESS), stage.percent() as f32 / 100.0);
    widgets::note(p, p.scale.rect(splash::STAGE), stage.label(), Align::Left);
    let version = format::splash_version(brand::VERSION);
    widgets::note(p, p.scale.rect(splash::VERSION), &version, Align::Right);
    p.outline(window, t.outline);
}

impl Splash {
    fn handle(&mut self, hwnd: HWND, message: u32, wparam: WPARAM) -> Option<LRESULT> {
        match message {
            WM_PAINT => {
                let Splash { canvas, theme, stage } = self;
                canvas.present(hwnd, theme, |p| draw(p, *stage));
            }
            WM_ERASEBKGND => return Some(LRESULT(1)),
            // Esc gives up on the detection and ends the program, like closing the window.
            WM_KEYDOWN if wparam.0 == usize::from(VK_ESCAPE.0) => win::post_close(hwnd),
            // SAFETY: ends the splash's message loop; PostQuitMessage has no preconditions.
            WM_DESTROY => unsafe { PostQuitMessage(0) },
            _ => return None,
        }
        Some(LRESULT(0))
    }
}

extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: windows of this class are only created by `run`, with a `Splash` as their
    // state; the handler neither pumps messages nor destroys the window.
    unsafe {
        if message == WM_NCDESTROY {
            win::release::<Splash>(hwnd);
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        win::with_state(hwnd, |splash: &mut Splash| splash.handle(hwnd, message, wparam))
            .flatten()
            .unwrap_or_else(|| DefWindowProcW(hwnd, message, wparam, lparam))
    }
}

/// What detection produced: the system and the first clock sample.
type Detected = (SystemInfo, ClockSnapshot);

/// Folds one worker event into the detection result; returns the stage to display, if it changed.
fn absorb(event: Event, result: &mut Option<Detected>) -> Option<Stage> {
    match event {
        Event::Stage(stage) => return Some(stage),
        Event::Detected(sys, clocks) => *result = Some((*sys, clocks)),
        // The worker died before reporting: carry on with an empty model rather than hang.
        Event::WorkerExited if result.is_none() => *result = Some(Detected::default()),
        _ => {}
    }
    None
}

/// Waits for the detection result without a window, for when the splash cannot be created.
fn wait_without_window(inbox: &Inbox) -> Option<Detected> {
    let mut result = None;
    while result.is_none() {
        absorb(inbox.wait()?, &mut result);
    }
    result
}

/// Shows the splash until detection completes and returns what was detected.
/// Returns `None` when the user closed the splash first.
pub fn run(cfg: &Config, mailbox: &Mailbox, inbox: &Inbox) -> Option<Detected> {
    let state =
        Splash { theme: Theme::new(cfg), canvas: Canvas::new(cfg, BASE_DPI), stage: Stage::Processors };
    // A taskbar button, so that a splash covered by other windows can still be found.
    let Some(hwnd) = win::create_window(CLASS, brand::APP_NAME, WS_POPUP, WS_EX_APPWINDOW, None, state)
    else {
        return wait_without_window(inbox);
    };
    let dpi = win::place_on_pointer_monitor(hwnd, |dpi| Scale::new(dpi).size(splash::SIZE.0, splash::SIZE.1));
    // SAFETY: `hwnd` is the splash created above; its state is a `Splash`.
    unsafe {
        win::with_state(hwnd, |splash: &mut Splash| splash.canvas.set_dpi(cfg, dpi));
    }
    win::show(hwnd);
    mailbox.listen(Some(hwnd));

    let mut result = None;
    let mut message = MSG::default();
    // SAFETY: a plain message loop on the thread that owns `hwnd`. Worker events are handled
    // here, outside any window procedure, so destroying the splash from the loop is sound.
    unsafe {
        let _ = UpdateWindow(hwnd);
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            if message.hwnd != hwnd || message.message != win::WM_UI_EVENT {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
                continue;
            }
            for event in inbox.drain() {
                if let Some(stage) = absorb(event, &mut result) {
                    win::with_state(hwnd, |splash: &mut Splash| splash.stage = stage);
                    win::invalidate(hwnd);
                    let _ = UpdateWindow(hwnd);
                }
            }
            if result.is_some() {
                let _ = DestroyWindow(hwnd);
            }
        }
    }
    mailbox.listen(None);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    #[test]
    fn detection_result_is_taken_from_the_worker_events() {
        let mut result = None;
        assert_eq!(absorb(Event::Stage(Stage::Memory), &mut result), Some(Stage::Memory));
        assert!(result.is_none());
        assert_eq!(absorb(Event::Clocks(demo::clocks()), &mut result), None);
        assert!(result.is_none());
        assert_eq!(absorb(Event::Detected(Box::new(demo::system()), demo::clocks()), &mut result), None);
        assert_eq!(absorb(Event::WorkerExited, &mut result), None);
        let (sys, clocks) = result.expect("detected system");
        assert_eq!(sys.cpu.cores, 8);
        assert_eq!(clocks, demo::clocks());
    }

    #[test]
    fn a_dead_worker_yields_an_empty_system() {
        let mut result = None;
        absorb(Event::WorkerExited, &mut result);
        let (sys, clocks) = result.expect("fallback system");
        assert!(sys.cpu.name.is_empty());
        assert!(clocks.cores.is_empty());
    }
}
