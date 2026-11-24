//! The main window: mouse, keyboard and worker events around the renderer.
//!
//! Message handlers run with exclusive access to the window state and must
//! not pump messages. Anything that does (dialogs, opening other windows,
//! resizing) is returned as a [`Command`] and carried out by the window
//! procedure after the state has been released.

use std::io;
use std::path::Path;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_CONTROL, VK_F5, VK_F6, VK_SHIFT, VK_TAB};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, PostQuitMessage, SetForegroundWindow, ShowWindow, CS_DROPSHADOW, SW_MINIMIZE,
    WM_CAPTURECHANGED, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_NCDESTROY, WM_NCHITTEST, WM_PAINT, WM_RBUTTONUP, WS_EX_APPWINDOW, WS_MINIMIZEBOX,
    WS_POPUP, WS_SYSMENU,
};

use crate::bench::{self, Phase, Reference};
use crate::brand;
use crate::config::Config;
use crate::elevation::{self, Elevation};
use crate::model::{ClockSnapshot, SystemInfo};
use crate::report;

use super::bmp;
use super::content;
use super::dialogs::{self, FileKind, BITMAP_FILE, HTML_FILE, TEXT_FILE};
use super::events::{Event, Inbox, Mailbox};
use super::format;
use super::gdi::{Painter, Surface};
use super::geom::{height, width, Scale, BASE_DPI};
use super::layout::{about, bench as bench_id, Kind, Tab, TOOLS_BUTTON};
use super::popup;
use super::render;
use super::state::{self, BenchMode, ListId, Target, ToolsEntry, UiState, View};
use super::theme::Theme;
use super::tools::{self, ClocksTool, SensorsTool, TimersTool, Tool};
use super::win::{self, Canvas, Icons};
use super::Options;

const CLASS: PCWSTR = w!("RsCpuzMain");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReportKind {
    Txt,
    Html,
}

/// Work that pumps messages or touches other windows; done once the state is released.
enum Command {
    Minimize,
    /// Move and resize the window (after a DPI change).
    Place(RECT),
    OpenList(popup::Request),
    SaveReport(ReportKind),
    /// Ask for a file name and save the image there.
    SaveImage(Surface, &'static str),
    CopyImage(Surface),
    OpenClocks,
    OpenTimers,
    OpenSensors,
    RestartAsAdmin,
    Error(String),
}

/// Outcome of a message handler.
enum Reply {
    /// Let the system handle the message.
    Default,
    Done(isize),
    Run(Command),
}

struct App {
    cfg: Config,
    theme: Theme,
    canvas: Canvas,
    sys: SystemInfo,
    clocks: ClockSnapshot,
    ui: UiState,
    mailbox: Mailbox,
    inbox: Inbox,
    /// The running benchmark or stress test.
    job: Option<bench::Job>,
    /// Serial number of the current benchmark run; events of older runs are dropped.
    run: u32,
    /// A `WM_MOUSELEAVE` request is armed.
    tracking: bool,
    clocks_tool: Option<HWND>,
    timers_tool: Option<HWND>,
    sensors_tool: Option<HWND>,
}

/// Registers the window class of the main window.
pub fn register(icons: &Icons) -> bool {
    win::register_class(CLASS, CS_DROPSHADOW, procedure, Some(icons))
}

/// Opens the main window for a detected system and runs it until it is closed.
/// Returns the process exit code.
pub fn run(
    cfg: &Config,
    opts: &Options,
    sys: SystemInfo,
    clocks: ClockSnapshot,
    mailbox: Mailbox,
    inbox: Inbox,
) -> i32 {
    // The benchmark measures this machine, also while the sample system is on show.
    let threads = bench::thread_limit((!opts.demo).then_some(&sys.cpu));
    let mut ui = super::initial_state(&sys, opts.core, threads);
    ui.bench.sample = opts.demo;
    // A copy that runs without full rights may restart with them; the sample system gains nothing.
    ui.offer_admin = !opts.demo && elevation::current() != Elevation::Elevated;
    let app = App {
        cfg: cfg.clone(),
        theme: Theme::new(cfg),
        canvas: Canvas::new(cfg, BASE_DPI),
        ui,
        sys,
        clocks,
        mailbox: mailbox.clone(),
        inbox,
        job: None,
        run: 0,
        tracking: false,
        clocks_tool: None,
        timers_tool: None,
        sensors_tool: None,
    };
    let style = WS_POPUP | WS_SYSMENU | WS_MINIMIZEBOX;
    let Some(hwnd) = win::create_window(CLASS, brand::APP_NAME, style, WS_EX_APPWINDOW, None, app) else {
        return 1;
    };
    // The window opens on the monitor the pointer is on, laid out for that monitor's DPI.
    let dpi = win::place_on_pointer_monitor(hwnd, |dpi| render::frame_size(Scale::new(dpi)));
    with_app(hwnd, |app| app.canvas.set_dpi(&app.cfg, dpi));
    mailbox.listen(Some(hwnd));
    win::show(hwnd);
    // SAFETY: SetForegroundWindow validates the handle; failing to get the foreground is harmless.
    unsafe {
        let _ = SetForegroundWindow(hwnd);
    }
    win::message_loop()
}

/// Runs `action` on the state of the main window `hwnd`.
fn with_app<R>(hwnd: HWND, action: impl FnOnce(&mut App) -> R) -> Option<R> {
    // SAFETY: only called with windows of class `CLASS`, whose state is an `App`.
    unsafe { win::with_state(hwnd, action) }
}

/// Name offered for a saved report: the computer name, as the reference tool does.
fn report_name(sys: &SystemInfo) -> String {
    let name = sys.os.computer_name.trim();
    if name.is_empty() {
        "report".to_string()
    } else {
        name.to_string()
    }
}

/// Name under which a result of this processor is stored as a benchmark reference.
///
/// A run on only `threads` of the `all_threads` logical processors says so in
/// its name, which keeps it apart from the result of a full run.
fn reference_name(sys: &SystemInfo, threads: u32, all_threads: u32) -> String {
    let processor = [&sys.cpu.specification, &sys.cpu.name]
        .into_iter()
        .map(|name| name.trim())
        .find(|name| !name.is_empty())
        .unwrap_or("This Processor");
    if threads < all_threads {
        format!("{processor} ({})", format::thread_count(threads))
    } else {
        processor.to_string()
    }
}

fn save_failure(path: &Path, error: &io::Error) -> String {
    format!("Could not save {}:\n{error}", path.display())
}

/// What a click on an interactive page control does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    OpenList(ListId),
    ToggleBench,
    ToggleStress,
    SaveReference,
    ToggleReference(Phase),
    SaveReport(ReportKind),
    OpenClocks,
    OpenTimers,
}

fn control_action(tab: Tab, id: u16) -> Option<Action> {
    if let Some(list) = content::combo_list(tab, id) {
        return Some(Action::OpenList(list));
    }
    Some(match (tab, id) {
        (Tab::Bench, bench_id::BENCH_BUTTON) => Action::ToggleBench,
        (Tab::Bench, bench_id::STRESS_BUTTON) => Action::ToggleStress,
        (Tab::Bench, bench_id::SAVE_BUTTON) => Action::SaveReference,
        (Tab::Bench, bench_id::SINGLE_REFERENCE_CHECK) => Action::ToggleReference(Phase::Single),
        (Tab::Bench, bench_id::MULTI_REFERENCE_CHECK) => Action::ToggleReference(Phase::Multi),
        (Tab::About, about::SAVE_TXT) => Action::SaveReport(ReportKind::Txt),
        (Tab::About, about::SAVE_HTML) => Action::SaveReport(ReportKind::Html),
        (Tab::About, about::CLOCKS) => Action::OpenClocks,
        (Tab::About, about::TIMERS) => Action::OpenTimers,
        _ => return None,
    })
}

/// The command behind entry `index` of the "Tools" menu as it is without the
/// administrator entry.
#[cfg(test)]
fn tools_menu_command(index: usize) -> Option<Command> {
    menu_command(state::tools_menu(false), index)
}

/// The command behind entry `index` of the "Tools" menu `entries`.
fn menu_command(entries: Vec<ToolsEntry>, index: usize) -> Option<Command> {
    Some(match entries.get(index)? {
        ToolsEntry::SaveTxt => Command::SaveReport(ReportKind::Txt),
        ToolsEntry::SaveHtml => Command::SaveReport(ReportKind::Html),
        ToolsEntry::Clocks => Command::OpenClocks,
        ToolsEntry::Timers => Command::OpenTimers,
        ToolsEntry::Sensors => Command::OpenSensors,
        ToolsEntry::RestartAsAdmin => Command::RestartAsAdmin,
    })
}

impl App {
    fn view(&self) -> View<'_> {
        View { sys: &self.sys, clocks: &self.clocks, ui: &self.ui }
    }

    fn hit(&self, pt: POINT) -> Option<Target> {
        render::hit_test(self.canvas.scale(), &self.view(), pt)
    }

    fn paint(&mut self, hwnd: HWND) {
        let App { canvas, theme, sys, clocks, ui, .. } = self;
        canvas.present(hwnd, theme, |p| render::main_window(p, &View { sys, clocks, ui }));
    }

    /// The window as an image, without hover or pressed feedback.
    fn capture(&self) -> Option<Surface> {
        let scale = self.canvas.scale();
        let (frame_width, frame_height) = render::frame_size(scale);
        let surface = Surface::new(frame_width, frame_height)?;
        let ui = self.ui.at_rest();
        let painter = Painter { hdc: surface.dc(), scale, fonts: self.canvas.fonts(), theme: &self.theme };
        render::main_window(&painter, &View { sys: &self.sys, clocks: &self.clocks, ui: &ui });
        Some(surface)
    }

    /// Prepares the popup for `list`, dropping down from the client rectangle `anchor`.
    fn open_list(&mut self, hwnd: HWND, list: ListId, anchor: RECT) -> Reply {
        // A click on the selector whose list is showing only closes that list.
        if self.ui.open == Some(list) {
            return Reply::Done(0);
        }
        let view = self.view();
        let items = view.list_items(list);
        if items.is_empty() {
            return Reply::Done(0);
        }
        let request = popup::Request {
            cfg: self.cfg.clone(),
            list,
            items,
            selected: view.list_selection(list),
            anchor: win::to_screen(hwnd, anchor),
            dpi: self.canvas.scale().dpi(),
        };
        self.ui.open = Some(list);
        win::invalidate(hwnd);
        Reply::Run(Command::OpenList(request))
    }

    fn mouse_move(&mut self, hwnd: HWND, pt: POINT) {
        let hot = self.hit(pt);
        if hot != self.ui.hot {
            self.ui.hot = hot;
            win::invalidate(hwnd);
        }
        if !self.tracking {
            self.tracking = win::track_mouse_leave(hwnd);
        }
    }

    fn left_down(&mut self, hwnd: HWND, pt: POINT) -> Reply {
        let Some(target) = self.hit(pt) else { return Reply::Done(0) };
        let scale = self.canvas.scale();
        let tab = self.ui.tab;
        match target {
            // Tabs and lists react on button-down, like their system counterparts.
            Target::Tab(tab) => {
                self.ui.tab = tab;
                win::invalidate(hwnd);
                Reply::Done(0)
            }
            Target::Tools => self.open_list(hwnd, ListId::ToolsMenu, scale.rect(TOOLS_BUTTON)),
            Target::Control(id) if tab.item(id).is_some_and(|item| item.kind == Kind::Combo) => {
                match (content::combo_list(tab, id), render::control_rect(scale, tab, id)) {
                    (Some(list), Some(anchor)) => self.open_list(hwnd, list, anchor),
                    _ => Reply::Done(0),
                }
            }
            _ => {
                self.ui.pressed = Some(target);
                win::capture_mouse(hwnd);
                win::invalidate(hwnd);
                Reply::Done(0)
            }
        }
    }

    fn left_up(&mut self, hwnd: HWND, pt: POINT) -> Reply {
        let Some(pressed) = self.ui.pressed.take() else { return Reply::Done(0) };
        win::release_mouse();
        win::invalidate(hwnd);
        if self.hit(pt) != Some(pressed) {
            return Reply::Done(0);
        }
        match pressed {
            Target::Minimize => Reply::Run(Command::Minimize),
            Target::CloseBox | Target::Close => {
                win::post_close(hwnd);
                Reply::Done(0)
            }
            Target::Control(id) => self.click(self.ui.tab, id),
            Target::Tab(_) | Target::Tools => Reply::Done(0),
        }
    }

    /// Opens the core menu when the right button is released inside the Clocks group.
    fn right_up(&mut self, hwnd: HWND, pt: POINT) -> Reply {
        if !render::on_clocks_group(self.canvas.scale(), self.ui.tab, pt) {
            return Reply::Default;
        }
        self.open_list(hwnd, ListId::CoreMenu, RECT { left: pt.x, top: pt.y, right: pt.x, bottom: pt.y })
    }

    fn click(&mut self, tab: Tab, id: u16) -> Reply {
        match control_action(tab, id) {
            Some(Action::ToggleBench) => self.toggle_bench(),
            Some(Action::ToggleStress) => return self.toggle_stress(),
            Some(Action::ToggleReference(phase)) => self.ui.bench.toggle_reference(phase),
            Some(Action::SaveReference) => return self.save_reference(),
            Some(Action::SaveReport(kind)) => return Reply::Run(Command::SaveReport(kind)),
            Some(Action::OpenClocks) => return Reply::Run(Command::OpenClocks),
            Some(Action::OpenTimers) => return Reply::Run(Command::OpenTimers),
            Some(Action::OpenList(_)) | None => {}
        }
        Reply::Done(0)
    }

    /// Starts the benchmark, or stops the one that is running.
    fn toggle_bench(&mut self) {
        match self.ui.bench.mode {
            BenchMode::Benching => {
                self.job = None;
                self.ui.bench.end();
            }
            BenchMode::Idle => {
                self.run = self.run.wrapping_add(1);
                let run = self.run;
                let mailbox = self.mailbox.clone();
                self.ui.bench.begin_bench();
                self.job = Some(bench::Job::bench(self.ui.bench.threads, move |event| {
                    mailbox.post(Event::Bench(run, event));
                }));
            }
            BenchMode::Stressing => {}
        }
    }

    /// Starts the stress test, or stops the one that is running.
    fn toggle_stress(&mut self) -> Reply {
        match self.ui.bench.mode {
            BenchMode::Stressing => {
                self.job = None;
                self.ui.bench.end();
            }
            BenchMode::Idle => {
                self.job = bench::Job::stress(self.ui.bench.threads);
                if self.job.is_none() {
                    return Reply::Run(Command::Error("The stress test could not be started.".to_string()));
                }
                self.ui.bench.mode = BenchMode::Stressing;
            }
            BenchMode::Benching => {}
        }
        Reply::Done(0)
    }

    /// Stores the current scores as a reference and selects it.
    fn save_reference(&mut self) -> Reply {
        let bench = &self.ui.bench;
        let (true, Some(single), Some(multi)) = (bench.can_save(), bench.single, bench.multi) else {
            return Reply::Done(0);
        };
        let name = reference_name(&self.sys, bench.run_threads, bench.max_threads);
        let reference = Reference { name, single, multi };
        let path = super::reference_path();
        if let Err(error) = bench::save_reference(&path, &reference) {
            return Reply::Run(Command::Error(save_failure(&path, &error)));
        }
        self.ui.bench.references = super::saved_references();
        self.ui.bench.reference = self.ui.bench.references.iter().position(|r| r.name == reference.name);
        Reply::Done(0)
    }

    fn key(&mut self, hwnd: HWND, key: u16) -> Reply {
        match key {
            k if k == VK_F5.0 => match self.capture() {
                Some(image) => Reply::Run(Command::SaveImage(image, self.ui.tab.file_name())),
                None => Reply::Done(0),
            },
            k if k == VK_F6.0 => match self.capture() {
                Some(image) => Reply::Run(Command::CopyImage(image)),
                None => Reply::Done(0),
            },
            k if k == VK_TAB.0 && win::key_down(VK_CONTROL.0) => {
                let step = if win::key_down(VK_SHIFT.0) { -1 } else { 1 };
                self.ui.tab = self.ui.tab.cycled(step);
                win::invalidate(hwnd);
                Reply::Done(0)
            }
            _ => Reply::Default,
        }
    }

    /// Applies the events queued by the worker threads.
    fn drain(&mut self, hwnd: HWND) {
        for event in self.inbox.drain() {
            match event {
                Event::Clocks(clocks) => {
                    self.clocks = clocks;
                    if let Some(tool) = self.clocks_tool {
                        tools::update_clocks(tool, &self.clocks);
                    }
                }
                Event::Sensors(sensors) => {
                    if let Some(tool) = self.sensors_tool {
                        tools::update_sensors(tool, &sensors);
                    }
                }
                Event::Bench(run, event) if run == self.run => {
                    self.ui.bench.apply(event);
                    if self.ui.bench.mode == BenchMode::Idle {
                        self.job = None;
                    }
                }
                _ => {}
            }
        }
        win::invalidate(hwnd);
    }

    /// Handles the entry chosen in a popup list.
    fn choice(&mut self, hwnd: HWND, code: usize, index: usize) -> Reply {
        let Some(list) = ListId::from_code(code) else { return Reply::Done(0) };
        if list == ListId::ToolsMenu {
            return menu_command(state::tools_menu(self.ui.offer_admin), index)
                .map_or(Reply::Done(0), Reply::Run);
        }
        self.ui.choose(list, index, &self.sys, &self.clocks);
        win::invalidate(hwnd);
        Reply::Done(0)
    }

    fn handle(&mut self, hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Reply {
        match message {
            WM_PAINT => self.paint(hwnd),
            WM_ERASEBKGND => return Reply::Done(1),
            WM_NCHITTEST => {
                let pt = win::to_client(hwnd, win::point_from(lparam));
                return Reply::Done(win::hit_code(render::on_title_strip(self.canvas.scale(), pt)));
            }
            WM_MOUSEMOVE => self.mouse_move(hwnd, win::point_from(lparam)),
            WM_MOUSELEAVE => {
                self.tracking = false;
                self.ui.hot = None;
                win::invalidate(hwnd);
            }
            WM_LBUTTONDOWN => return self.left_down(hwnd, win::point_from(lparam)),
            WM_LBUTTONUP => return self.left_up(hwnd, win::point_from(lparam)),
            WM_RBUTTONUP => return self.right_up(hwnd, win::point_from(lparam)),
            WM_CAPTURECHANGED => {
                self.ui.pressed = None;
                win::invalidate(hwnd);
            }
            WM_KEYDOWN => return self.key(hwnd, win::low_word(wparam) as u16),
            win::WM_UI_EVENT => self.drain(hwnd),
            win::WM_LIST_CHOICE => return self.choice(hwnd, wparam.0, lparam.0.unsigned_abs()),
            win::WM_LIST_CLOSED => {
                if self.ui.open.map(ListId::code) == Some(wparam.0) {
                    self.ui.open = None;
                    win::invalidate(hwnd);
                }
            }
            win::WM_TOOL_CLOSED => match wparam.0 {
                ClocksTool::CODE => self.clocks_tool = None,
                TimersTool::CODE => self.timers_tool = None,
                SensorsTool::CODE => {
                    self.sensors_tool = None;
                    self.mailbox.want_sensors(false);
                }
                _ => {}
            },
            WM_DPICHANGED => {
                self.canvas.set_dpi(&self.cfg, win::high_word(wparam));
                win::invalidate(hwnd);
                let size = render::frame_size(self.canvas.scale());
                // SAFETY: this is the `lparam` of the WM_DPICHANGED message being handled.
                return Reply::Run(Command::Place(unsafe { win::suggested_placement(lparam, size) }));
            }
            WM_DESTROY => {
                // Stop the wake-ups and the benchmark threads before the window goes away.
                self.mailbox.listen(None);
                self.job = None;
                // SAFETY: ends the message loop of this thread; no preconditions.
                unsafe { PostQuitMessage(0) };
            }
            _ => return Reply::Default,
        }
        Reply::Done(0)
    }
}

/// Asks for a file name and writes the report there.
fn save_report(hwnd: HWND, kind: ReportKind) {
    let Some(name) = with_app(hwnd, |app| report_name(&app.sys)) else { return };
    let file_kind: FileKind = match kind {
        ReportKind::Txt => TEXT_FILE,
        ReportKind::Html => HTML_FILE,
    };
    let Some(path) = dialogs::save_file(hwnd, &name, file_kind) else { return };
    let result = with_app(hwnd, |app| match kind {
        ReportKind::Txt => report::write_txt(&app.sys, Some(&app.clocks), &path),
        ReportKind::Html => report::write_html(&app.sys, Some(&app.clocks), &path),
    });
    if let Some(Err(error)) = result {
        dialogs::error(Some(hwnd), &save_failure(&path, &error));
    }
}

/// Opens a tool window, or brings the one that is already open to the front. `true` when
/// a new window opened.
fn open_tool<T: Tool>(
    hwnd: HWND,
    slot: fn(&mut App) -> &mut Option<HWND>,
    make: impl FnOnce(&App) -> T,
) -> bool {
    if let Some(Some(existing)) = with_app(hwnd, |app| *slot(app)) {
        tools::focus(existing);
        return false;
    }
    let Some((cfg, tool)) = with_app(hwnd, |app| (app.cfg.clone(), make(app))) else { return false };
    let opened = tools::open(hwnd, &cfg, tool);
    with_app(hwnd, |app| *slot(app) = opened);
    opened.is_some()
}

/// Carries out a command. The window state is not borrowed here, so the dialogs
/// and windows opened below may send messages back to the main window.
fn execute(hwnd: HWND, command: Command) {
    match command {
        Command::Minimize => {
            // SAFETY: ShowWindow validates the window handle.
            unsafe {
                let _ = ShowWindow(hwnd, SW_MINIMIZE);
            }
        }
        Command::Place(r) => win::place(hwnd, r.left, r.top, width(r), height(r)),
        Command::OpenList(request) => {
            let list = request.list;
            if !popup::open(hwnd, request) {
                with_app(hwnd, |app| {
                    if app.ui.open == Some(list) {
                        app.ui.open = None;
                    }
                });
                win::invalidate(hwnd);
            }
        }
        Command::SaveReport(kind) => save_report(hwnd, kind),
        Command::SaveImage(image, name) => {
            if let Some(path) = dialogs::save_file(hwnd, name, BITMAP_FILE) {
                if let Err(error) = bmp::save(&image, &path) {
                    dialogs::error(Some(hwnd), &save_failure(&path, &error));
                }
            }
        }
        Command::CopyImage(image) => {
            if !bmp::copy_to_clipboard(hwnd, &image) {
                dialogs::error(Some(hwnd), "The image could not be copied to the clipboard.");
            }
        }
        Command::OpenClocks => {
            open_tool(hwnd, |app| &mut app.clocks_tool, |app| ClocksTool::new(&app.clocks));
        }
        Command::OpenTimers => {
            open_tool(hwnd, |app| &mut app.timers_tool, |_| TimersTool::default());
        }
        Command::OpenSensors => {
            // The worker reads the sensors only while their window is open.
            if open_tool(hwnd, |app| &mut app.sensors_tool, |app| SensorsTool::new(&app.sys.sensors)) {
                with_app(hwnd, |app| app.mailbox.want_sensors(true));
            }
        }
        Command::RestartAsAdmin => match elevation::relaunch(Some(hwnd)) {
            // The elevated copy takes over.
            Ok(()) => win::post_close(hwnd),
            Err(elevation::Refused::Declined) => {}
            Err(elevation::Refused::Failed(error)) => dialogs::error(
                Some(hwnd),
                &format!("The program could not be restarted as administrator: {error}"),
            ),
        },
        Command::Error(text) => dialogs::error(Some(hwnd), &text),
    }
}

extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: windows of this class are only created by `run`, with an `App` as their
    // state; the handlers neither pump messages nor destroy the window.
    unsafe {
        if message == WM_NCDESTROY {
            win::release::<App>(hwnd);
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
    }
    match with_app(hwnd, |app| app.handle(hwnd, message, wparam, lparam)).unwrap_or(Reply::Default) {
        // SAFETY: forwards the unhandled message unchanged.
        Reply::Default => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        Reply::Done(result) => LRESULT(result),
        Reply::Run(command) => {
            execute(hwnd, command);
            LRESULT(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use crate::ui::layout::{cpu, graphics, spd};

    #[test]
    fn page_controls_map_to_actions() {
        assert_eq!(control_action(Tab::Cpu, cpu::SELECTION), Some(Action::OpenList(ListId::Socket)));
        assert_eq!(control_action(Tab::Spd, spd::SLOT), Some(Action::OpenList(ListId::Slot)));
        assert_eq!(
            control_action(Tab::Graphics, graphics::PERF_LEVEL),
            Some(Action::OpenList(ListId::PerfLevel))
        );
        assert_eq!(control_action(Tab::Bench, bench_id::BENCH_BUTTON), Some(Action::ToggleBench));
        assert_eq!(
            control_action(Tab::Bench, bench_id::MULTI_REFERENCE_CHECK),
            Some(Action::ToggleReference(Phase::Multi))
        );
        assert_eq!(control_action(Tab::About, about::SAVE_HTML), Some(Action::SaveReport(ReportKind::Html)));
        assert_eq!(control_action(Tab::About, about::TIMERS), Some(Action::OpenTimers));
        assert_eq!(control_action(Tab::Cpu, cpu::NAME), None);
        assert_eq!(control_action(Tab::Mainboard, bench_id::BENCH_BUTTON), None);
    }

    #[test]
    fn every_interactive_control_has_an_action() {
        for tab in Tab::ALL {
            for item in tab.items().iter().filter(|item| item.kind.is_interactive()) {
                assert!(control_action(tab, item.id).is_some(), "{tab:?} control {}", item.id);
            }
        }
    }

    #[test]
    fn tools_menu_entries_have_commands() {
        assert!(matches!(tools_menu_command(0), Some(Command::SaveReport(ReportKind::Txt))));
        assert!(matches!(tools_menu_command(1), Some(Command::SaveReport(ReportKind::Html))));
        assert!(matches!(tools_menu_command(2), Some(Command::OpenClocks)));
        assert!(matches!(tools_menu_command(3), Some(Command::OpenTimers)));
        assert!(matches!(tools_menu_command(4), Some(Command::OpenSensors)));
        assert!(tools_menu_command(5).is_none());
        assert!(matches!(menu_command(state::tools_menu(true), 5), Some(Command::RestartAsAdmin)));
    }

    #[test]
    fn default_names() {
        let mut sys = demo::system();
        assert_eq!(report_name(&sys), "EXAMPLE-PC");
        assert_eq!(reference_name(&sys, 16, 16), "AMD Ryzen 7 7700 8-Core Processor");
        sys.os.computer_name = "  ".into();
        sys.cpu.specification.clear();
        assert_eq!(report_name(&sys), "report");
        assert_eq!(reference_name(&sys, 16, 16), "AMD Ryzen 7 7700");
        sys.cpu.name.clear();
        assert_eq!(reference_name(&sys, 16, 16), "This Processor");
    }

    #[test]
    fn reduced_runs_are_stored_under_their_own_name() {
        let sys = demo::system();
        assert_eq!(reference_name(&sys, 4, 16), "AMD Ryzen 7 7700 8-Core Processor (4 threads)");
        assert_eq!(reference_name(&sys, 1, 16), "AMD Ryzen 7 7700 8-Core Processor (1 thread)");
        assert_eq!(reference_name(&sys, 1, 1), "AMD Ryzen 7 7700 8-Core Processor");

        // Both entries live side by side in the reference file and read back as they were named.
        let dir = std::env::temp_dir().join(format!("rs-cpuz-test-{}-reduced-runs", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("references.txt");
        let full = Reference { name: reference_name(&sys, 16, 16), single: 700.0, multi: 7000.0 };
        let reduced = Reference { name: reference_name(&sys, 4, 16), single: 700.0, multi: 2700.0 };
        let saved = bench::save_reference(&path, &full).and_then(|()| bench::save_reference(&path, &reduced));
        let loaded = bench::load_references(&path);
        let _ = std::fs::remove_dir_all(&dir);
        saved.unwrap();
        assert_eq!(loaded, [full, reduced]);
    }

    #[test]
    fn failure_message_names_the_file() {
        let error = io::Error::new(io::ErrorKind::PermissionDenied, "access denied");
        let text = save_failure(Path::new("out.txt"), &error);
        assert!(text.contains("out.txt") && text.contains("access denied"));
    }
}
