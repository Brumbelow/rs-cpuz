//! Window plumbing shared by every window class: registration, per-window
//! state, the double-buffered canvas and the message loop.

use std::cell::RefCell;
use std::ffi::c_void;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{FALSE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, ClientToScreen, EndPaint, GetMonitorInfoW, InvalidateRect, MonitorFromPoint,
    MonitorFromRect, ScreenToClient, HBRUSH, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    MONITOR_DEFAULTTOPRIMARY, PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyIcon, DispatchMessageW, GetClientRect, GetMessageW, GetSystemMetrics,
    GetWindowLongPtrW, GetWindowRect, LoadCursorW, PostMessageW, RegisterClassExW, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, TranslateMessage, GWLP_USERDATA, HICON, HMENU, HTCAPTION, HTCLIENT, IDC_ARROW,
    MSG, SM_CXICON, SM_CXSMICON, SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOW, WHEEL_DELTA, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_APP, WM_CLOSE, WNDCLASSEXW, WNDCLASS_STYLES, WNDPROC,
};

use crate::brand;
use crate::config::Config;

use super::gdi::{wide, Fonts, Painter, Surface};
use super::geom::{Scale, BASE_DPI};
use super::theme::Theme;

/// A worker thread queued events for the window: drain the inbox.
pub const WM_UI_EVENT: u32 = WM_APP + 1;
/// A popup list tells its owner the chosen entry: `wparam` = list code, `lparam` = index.
pub const WM_LIST_CHOICE: u32 = WM_APP + 2;
/// A popup list tells its owner it is gone: `wparam` = list code.
pub const WM_LIST_CLOSED: u32 = WM_APP + 3;
/// A tool window tells its owner it is gone: `wparam` = tool code.
pub const WM_TOOL_CLOSED: u32 = WM_APP + 4;

/// Module handle of the executable.
pub fn instance() -> HINSTANCE {
    // SAFETY: a null module name asks for the handle of the running executable.
    unsafe { GetModuleHandleW(PCWSTR::null()) }.map(|module| HINSTANCE(module.0)).unwrap_or_default()
}

/// The application icons in the two sizes a window class takes.
pub struct Icons {
    pub large: HICON,
    pub small: HICON,
}

impl Icons {
    pub fn create() -> Icons {
        // SAFETY: GetSystemMetrics has no preconditions.
        let (large, small) = unsafe { (GetSystemMetrics(SM_CXICON), GetSystemMetrics(SM_CXSMICON)) };
        Icons { large: brand::create_icon(large.max(16)), small: brand::create_icon(small.max(16)) }
    }
}

impl Drop for Icons {
    fn drop(&mut self) {
        for icon in [self.large, self.small] {
            if !icon.is_invalid() {
                // SAFETY: the icon was created by `brand::create_icon`, which hands over ownership.
                unsafe {
                    let _ = DestroyIcon(icon);
                }
            }
        }
    }
}

/// Registers a window class with the arrow cursor and no background brush.
pub fn register_class(
    name: PCWSTR,
    style: WNDCLASS_STYLES,
    procedure: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
    icons: Option<&Icons>,
) -> bool {
    let procedure: WNDPROC = Some(procedure);
    // SAFETY: the class structure is fully initialised and `name` points at a static string.
    unsafe {
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style,
            lpfnWndProc: procedure,
            hInstance: instance(),
            hIcon: icons.map(|icons| icons.large).unwrap_or_default(),
            hIconSm: icons.map(|icons| icons.small).unwrap_or_default(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH::default(),
            lpszClassName: name,
            ..Default::default()
        };
        RegisterClassExW(&class) != 0
    }
}

/// Creates a hidden window of `class` and attaches `state` to it.
///
/// The state is owned by the window from here on and dropped by [`release`]
/// when the window is destroyed. Messages sent during creation arrive before
/// the state is attached and must take the default path.
pub fn create_window<T>(
    class: PCWSTR,
    title: &str,
    style: WINDOW_STYLE,
    ex_style: WINDOW_EX_STYLE,
    owner: Option<HWND>,
    state: T,
) -> Option<HWND> {
    let title = wide(title);
    // SAFETY: `class` is a registered class name, `title` outlives the call and the state
    // pointer stored in the window is a leaked `Box<RefCell<T>>` reclaimed in `release`.
    unsafe {
        let hwnd = CreateWindowExW(
            ex_style,
            class,
            PCWSTR(title.as_ptr()),
            style,
            0,
            0,
            1,
            1,
            owner.unwrap_or_default(),
            HMENU::default(),
            instance(),
            None,
        )
        .ok()?;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(Box::new(RefCell::new(state))) as isize);
        Some(hwnd)
    }
}

/// Runs `action` with exclusive access to the state attached to `hwnd`.
///
/// Returns `None` when the window has no state (it is being created or is
/// gone) or when the state is already in use further up the call stack, which
/// happens when a handler triggers a nested message; such messages take the
/// default path. `action` must not pump messages or destroy the window.
///
/// # Safety
/// `hwnd` must be null, destroyed, or a window created by [`create_window`] with state type `T`.
pub unsafe fn with_state<T, R>(hwnd: HWND, action: impl FnOnce(&mut T) -> R) -> Option<R> {
    let cell = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<T>).as_ref()?;
    let mut state = cell.try_borrow_mut().ok()?;
    Some(action(&mut state))
}

/// Detaches and drops the state of `hwnd`; called for `WM_NCDESTROY`.
///
/// # Safety
/// `hwnd` must be a window created by [`create_window`] with state type `T`.
pub unsafe fn release<T>(hwnd: HWND) {
    let pointer = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut RefCell<T>;
    let Some(cell) = pointer.as_ref() else { return };
    // A state still borrowed by a handler further up the stack is leaked rather than freed under it.
    if cell.try_borrow_mut().is_ok() {
        drop(Box::from_raw(pointer));
    }
}

/// Client coordinates carried in the `lparam` of a mouse message.
pub fn point_from(lparam: LPARAM) -> POINT {
    POINT { x: i32::from(lparam.0 as u16 as i16), y: i32::from((lparam.0 >> 16) as u16 as i16) }
}

/// Low word of a `wparam`.
pub fn low_word(wparam: WPARAM) -> u32 {
    (wparam.0 & 0xFFFF) as u32
}

/// High word of a `wparam`.
pub fn high_word(wparam: WPARAM) -> u32 {
    ((wparam.0 >> 16) & 0xFFFF) as u32
}

/// Wheel travel of a `WM_MOUSEWHEEL` message, `WHEEL_DELTA` per notch; positive away from the user.
fn wheel_delta(wparam: WPARAM) -> isize {
    isize::from(high_word(wparam) as u16 as i16)
}

/// Turns the wheel messages of one window into rows to scroll.
///
/// Notched wheels send one `WHEEL_DELTA` per message, but precision touchpads
/// and free-spinning wheels send a stream of much smaller deltas, so the travel
/// that does not yet amount to a row is carried over to the next message.
#[derive(Debug, Default)]
pub struct Wheel {
    /// Travel not yet scrolled, in `WHEEL_DELTA`ths of a row.
    pending: isize,
}

impl Wheel {
    /// Rows to scroll for one `WM_MOUSEWHEEL` message when a full notch is
    /// worth `rows_per_notch`; positive away from the user.
    pub fn rows(&mut self, wparam: WPARAM, rows_per_notch: isize) -> isize {
        self.advance(wheel_delta(wparam), rows_per_notch)
    }

    fn advance(&mut self, delta: isize, rows_per_notch: isize) -> isize {
        let travel = delta * rows_per_notch;
        // Turning the wheel back starts afresh instead of first undoing the leftover.
        if travel.signum() * self.pending.signum() < 0 {
            self.pending = 0;
        }
        self.pending += travel;
        let rows = self.pending / WHEEL_DELTA as isize;
        self.pending -= rows * WHEEL_DELTA as isize;
        rows
    }
}

/// Whether the key is down as of the message being processed.
pub fn key_down(virtual_key: u16) -> bool {
    // SAFETY: GetKeyState has no preconditions.
    unsafe { GetKeyState(i32::from(virtual_key)) < 0 }
}

/// DPI of the monitor the window is on. The process is per-monitor DPI aware
/// through its manifest alone, so this is the monitor's own value.
pub fn dpi_of(hwnd: HWND) -> u32 {
    // SAFETY: GetDpiForWindow returns 0 for an invalid window.
    match unsafe { GetDpiForWindow(hwnd) } {
        0 => BASE_DPI,
        dpi => dpi,
    }
}

fn work_area(monitor: HMONITOR) -> Option<RECT> {
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    // SAFETY: `info` is a properly sized MONITORINFO.
    unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool().then_some(info.rcWork)
}

/// Work area of the primary monitor.
pub fn primary_work_area() -> Option<RECT> {
    // SAFETY: MonitorFromPoint has no preconditions.
    work_area(unsafe { MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY) })
}

/// Work area of the monitor nearest to `rect` (screen coordinates).
pub fn work_area_near(rect: RECT) -> Option<RECT> {
    // SAFETY: `rect` is a valid RECT for the duration of the call.
    work_area(unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST) })
}

/// Top-left corner that centres a `width` x `height` window in `area`.
pub fn centred_in(area: RECT, width: i32, height: i32) -> (i32, i32) {
    (area.left + (area.right - area.left - width) / 2, area.top + (area.bottom - area.top - height) / 2)
}

/// Client rectangle of a window; empty when the window is gone.
pub fn client_rect(hwnd: HWND) -> RECT {
    let mut rect = RECT::default();
    // SAFETY: `rect` is a valid RECT to write to.
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
    }
    rect
}

/// Screen rectangle of a window; empty when the window is gone.
pub fn window_rect(hwnd: HWND) -> RECT {
    let mut rect = RECT::default();
    // SAFETY: `rect` is a valid RECT to write to.
    unsafe {
        let _ = GetWindowRect(hwnd, &mut rect);
    }
    rect
}

/// Converts a screen point to the client coordinates of `hwnd`.
pub fn to_client(hwnd: HWND, mut point: POINT) -> POINT {
    // SAFETY: `point` is a valid POINT to convert in place.
    unsafe {
        let _ = ScreenToClient(hwnd, &mut point);
    }
    point
}

/// Converts a client rectangle of `hwnd` to screen coordinates.
pub fn to_screen(hwnd: HWND, rect: RECT) -> RECT {
    let mut corner = POINT { x: rect.left, y: rect.top };
    // SAFETY: `corner` is a valid POINT to convert in place.
    unsafe {
        let _ = ClientToScreen(hwnd, &mut corner);
    }
    RECT {
        left: corner.x,
        top: corner.y,
        right: corner.x + rect.right - rect.left,
        bottom: corner.y + rect.bottom - rect.top,
    }
}

/// Where a fixed-size window goes after a DPI change: the corner the system
/// suggests in `WM_DPICHANGED`, with the window's own new `size`.
///
/// # Safety
/// `lparam` must be the `lparam` of a `WM_DPICHANGED` message that is still being processed.
pub unsafe fn suggested_placement(lparam: LPARAM, size: (i32, i32)) -> RECT {
    let corner = (lparam.0 as *const RECT).as_ref().map_or((0, 0), |rect| (rect.left, rect.top));
    RECT { left: corner.0, top: corner.1, right: corner.0 + size.0, bottom: corner.1 + size.1 }
}

/// Result of `WM_NCHITTEST` for a point on the draggable title strip or elsewhere in the window.
pub fn hit_code(on_title_strip: bool) -> isize {
    (if on_title_strip { HTCAPTION } else { HTCLIENT }) as isize
}

/// Routes all mouse input to `hwnd` until [`release_mouse`], so a press is followed to its release.
pub fn capture_mouse(hwnd: HWND) {
    // SAFETY: SetCapture validates the window handle.
    unsafe {
        SetCapture(hwnd);
    }
}

/// Ends the capture started by [`capture_mouse`].
pub fn release_mouse() {
    // SAFETY: ReleaseCapture has no preconditions.
    unsafe {
        let _ = ReleaseCapture();
    }
}

/// Shows a window and gives it the activation.
pub fn show(hwnd: HWND) {
    // SAFETY: ShowWindow validates the window handle.
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
}

/// Moves and resizes a window without changing its Z order or activation.
pub fn place(hwnd: HWND, x: i32, y: i32, width: i32, height: i32) {
    // SAFETY: SetWindowPos validates the window handle.
    unsafe {
        let _ = SetWindowPos(hwnd, None, x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// Requests a repaint of the whole client area.
pub fn invalidate(hwnd: HWND) {
    // SAFETY: InvalidateRect validates the window handle.
    unsafe {
        let _ = InvalidateRect(hwnd, None, FALSE);
    }
}

/// Asks for a `WM_MOUSELEAVE` when the mouse leaves the client area.
pub fn track_mouse_leave(hwnd: HWND) -> bool {
    let mut request = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    // SAFETY: `request` is a properly sized TRACKMOUSEEVENT.
    unsafe { TrackMouseEvent(&mut request) }.is_ok()
}

/// Posts `WM_CLOSE`, the only way windows of this crate are closed from their own handlers.
pub fn post_close(hwnd: HWND) {
    post(hwnd, WM_CLOSE, 0, 0);
}

/// Posts a message; a window that is already gone is ignored.
pub fn post(hwnd: HWND, message: u32, wparam: usize, lparam: isize) {
    // SAFETY: PostMessageW validates the window handle and may be called from any thread.
    unsafe {
        let _ = PostMessageW(hwnd, message, WPARAM(wparam), LPARAM(lparam));
    }
}

/// Rebuilds a window handle from the integer it was stored as for another thread.
pub fn hwnd_from(raw: isize) -> HWND {
    HWND(raw as *mut c_void)
}

/// Runs the message loop until `WM_QUIT` and returns its exit code.
pub fn message_loop() -> i32 {
    let mut message = MSG::default();
    // SAFETY: `message` is a valid MSG; GetMessageW returns 0 for WM_QUIT and -1 on error.
    unsafe {
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    i32::try_from(message.wParam.0).unwrap_or(0)
}

/// Per-window drawing resources: the fonts for the window's DPI and its back buffer.
pub struct Canvas {
    scale: Scale,
    fonts: Fonts,
    surface: Option<Surface>,
}

impl Canvas {
    pub fn new(cfg: &Config, dpi: u32) -> Canvas {
        let scale = Scale::new(dpi);
        Canvas { scale, fonts: Fonts::new(cfg, scale), surface: None }
    }

    pub fn scale(&self) -> Scale {
        self.scale
    }

    pub fn fonts(&self) -> &Fonts {
        &self.fonts
    }

    /// Switches to another DPI: new fonts, and the back buffer is rebuilt on the next paint.
    pub fn set_dpi(&mut self, cfg: &Config, dpi: u32) {
        let scale = Scale::new(dpi);
        if scale != self.scale {
            self.scale = scale;
            self.fonts = Fonts::new(cfg, scale);
            self.surface = None;
        }
    }

    /// Answers `WM_PAINT`: draws into the back buffer and copies it to the window.
    pub fn present(&mut self, hwnd: HWND, theme: &Theme, draw: impl FnOnce(&Painter)) {
        let mut paint = PAINTSTRUCT::default();
        let mut client = RECT::default();
        // SAFETY: BeginPaint/EndPaint bracket the use of the window DC; the handles are
        // valid for the duration of the WM_PAINT message this is called from.
        unsafe {
            let target = BeginPaint(hwnd, &mut paint);
            let _ = GetClientRect(hwnd, &mut client);
            let (width, height) = (client.right, client.bottom);
            let stale = self.surface.as_ref().map_or(true, |s| s.width() != width || s.height() != height);
            if stale {
                self.surface = Surface::new(width, height);
            }
            match &self.surface {
                Some(surface) => {
                    draw(&Painter { hdc: surface.dc(), scale: self.scale, fonts: &self.fonts, theme });
                    let _ = BitBlt(target, 0, 0, width, height, surface.dc(), 0, 0, SRCCOPY);
                }
                None => draw(&Painter { hdc: target, scale: self.scale, fonts: &self.fonts, theme }),
            }
            let _ = EndPaint(hwnd, &paint);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_coordinates_are_signed() {
        let lparam = LPARAM(((-5i16 as u16 as isize) << 16) | (300u16 as isize));
        assert_eq!(point_from(lparam), POINT { x: 300, y: -5 });
        assert_eq!(point_from(LPARAM(0x0010_FFFF)), POINT { x: -1, y: 16 });
    }

    #[test]
    fn words_and_wheel() {
        let wparam = WPARAM(0x0090_0001);
        assert_eq!(low_word(wparam), 1);
        assert_eq!(high_word(wparam), 0x90);
        assert_eq!(wheel_delta(WPARAM(0x0078_0000)), 120);
        assert_eq!(wheel_delta(WPARAM(0xFF88_0000)), -120);
        assert_eq!(wheel_delta(WPARAM(0x00F0_0008)), 240, "the key flags in the low word are ignored");
        assert_eq!(wheel_delta(WPARAM(0xFFE2_0000)), -30);
        assert_eq!(hit_code(true), 2);
        assert_eq!(hit_code(false), 1);
    }

    #[test]
    fn whole_notches_scroll_at_once() {
        let mut wheel = Wheel::default();
        assert_eq!(wheel.rows(WPARAM(0x0078_0000), 3), 3);
        assert_eq!(wheel.rows(WPARAM(0xFF88_0000), 3), -3);
        assert_eq!(wheel.rows(WPARAM(0x00F0_0000), 3), 6);
        assert_eq!(wheel.rows(WPARAM(0x0078_0000), 1), 1);
        assert_eq!(wheel.pending, 0);
    }

    #[test]
    fn small_wheel_deltas_add_up() {
        // A third of a notch is one row of three.
        let mut wheel = Wheel::default();
        assert_eq!([40, 40, 40].map(|delta| wheel.advance(delta, 3)), [1, 1, 1]);
        assert_eq!(wheel.pending, 0);

        // The stream of a touchpad: ten notches of travel in either direction.
        for direction in [1, -1] {
            let mut wheel = Wheel::default();
            let rows: isize = (0..40).map(|_| wheel.advance(30 * direction, 3)).sum();
            assert_eq!(rows, 30 * direction);
        }

        // Travel short of a row is kept, not rounded.
        let mut wheel = Wheel::default();
        assert_eq!(wheel.advance(119, 1), 0);
        assert_eq!(wheel.advance(1, 1), 1);
        assert_eq!(wheel.advance(-119, 1), 0);
        assert_eq!(wheel.advance(-1, 1), -1);

        // A fast flick carries its remainder like any other message.
        let mut wheel = Wheel::default();
        assert_eq!(wheel.advance(200, 3), 5);
        assert_eq!(wheel.advance(200, 1), 1);
        assert_eq!(wheel.advance(40, 1), 1, "the 80 units left over from the flick count");
    }

    #[test]
    fn reversing_the_wheel_drops_the_leftover() {
        let mut wheel = Wheel::default();
        assert_eq!(wheel.advance(30, 3), 0);
        assert_eq!(wheel.advance(-30, 3), 0, "the leftover of the other direction is not owed back");
        assert_eq!(wheel.pending, -90);
        assert_eq!(wheel.advance(-10, 3), -1);
        assert_eq!(wheel.advance(39, 3), 0);
        assert_eq!(wheel.advance(1, 3), 1);
        assert_eq!(wheel.advance(0, 3), 0);
    }

    #[test]
    fn centring() {
        let area = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
        assert_eq!(centred_in(area, 417, 419), (751, 310));
        let second = RECT { left: -1280, top: 100, right: 0, bottom: 1124 };
        assert_eq!(centred_in(second, 400, 400), (-840, 412));
    }
}
