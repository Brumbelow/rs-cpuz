//! What happens when the program meets a bug of its own.
//!
//! Release builds run without a console, so a panic would end the program without a
//! word. The hook installed here writes what happened to a log file and, in the window,
//! says so in a message box. Detection stages run through [`guarded`]: a stage that
//! panics is only logged, and detection carries on without its data.

use std::any::Any;
use std::cell::Cell;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use windows::core::HSTRING;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

use crate::brand;

thread_local! {
    /// Set while this thread runs work whose panic is caught and dealt with.
    static GUARDED: Cell<bool> = const { Cell::new(false) };
}

/// Installs the panic hook. `window` selects how a fatal panic is told: in a message box
/// (the window), or on standard error the way Rust does (the command-line modes).
pub fn install(window: bool) {
    let default = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let place = info.location().map(|at| (at.file(), at.line(), at.column()));
        let text = report_text(&payload_text(info.payload()), place);
        let log = log_path();
        let logged = log.as_deref().is_some_and(|path| append(path, &text).is_ok());
        if is_guarded() {
            return;
        }
        if !window {
            default(info);
            return;
        }
        let saved = match (logged, &log) {
            (true, Some(path)) => format!("\n\nThe details were saved to {}.", path.display()),
            _ => String::new(),
        };
        let main_thread = std::thread::current().name() == Some("main");
        let message = format!("{}\n\n{text}{saved}", outcome(main_thread));
        // SAFETY: both strings outlive the call; no owner window is needed.
        unsafe {
            MessageBoxW(None, &HSTRING::from(message), &HSTRING::from(brand::APP_NAME), MB_OK | MB_ICONERROR);
        }
    }));
}

/// Runs `work`, catching a panic: `Err` with its payload when it panicked. The panic hook
/// only logs such a panic.
pub fn guarded<T>(work: impl FnOnce() -> T) -> std::thread::Result<T> {
    let outer = GUARDED.with(|flag| flag.replace(true));
    let result = panic::catch_unwind(AssertUnwindSafe(work));
    GUARDED.with(|flag| flag.set(outer));
    result
}

/// Whether the calling thread runs guarded work.
pub fn is_guarded() -> bool {
    GUARDED.with(Cell::get)
}

/// What a panic means for the user: the program closes when its main thread panicked;
/// another thread's panic ends only that thread's work, such as the clock updates.
fn outcome(main_thread: bool) -> String {
    if main_thread {
        format!("{} ran into a problem it cannot recover from and has to close.", brand::APP_NAME)
    } else {
        format!(
            "{} ran into a problem in a background task, which stopped. The window stays open, \
             but part of what it shows may no longer update.",
            brand::APP_NAME
        )
    }
}

/// `rs-cpuz 0.1.0 (x64) panicked at src/hw/usb.rs:120:9: index out of bounds`.
fn report_text(message: &str, place: Option<(&str, u32, u32)>) -> String {
    let at = place.map_or_else(String::new, |(file, line, column)| format!(" at {file}:{line}:{column}"));
    format!("rs-cpuz {} ({}) panicked{at}: {message}", env!("CARGO_PKG_VERSION"), std::env::consts::ARCH)
}

/// The message a panic was raised with.
fn payload_text(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        text.to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "(no message)".to_string()
    }
}

/// `%LOCALAPPDATA%\rs-cpuz\crash.log`; `None` without a local application data folder.
fn log_path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("rs-cpuz").join("crash.log"))
}

/// Appends one report to the log, behind the local time, creating the folder as needed.
fn append(path: &Path, text: &str) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    // SAFETY: GetLocalTime only fills in the structure it returns.
    let now = unsafe { GetLocalTime() };
    let stamp = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        now.wYear, now.wMonth, now.wDay, now.wHour, now.wMinute, now.wSecond
    );
    let mut log = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(log, "{stamp} {text}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarded_work_returns_its_value_or_the_failure() {
        assert_eq!(guarded(|| 5).ok(), Some(5));
        assert!(guarded(|| -> u32 { panic!("a detection backend tripped") }).is_err());
        // The marker is only set while the work runs.
        assert!(!is_guarded());
        assert_eq!(guarded(is_guarded).ok(), Some(true));
        assert!(!is_guarded());
    }

    #[test]
    fn reports_name_the_version_place_and_message() {
        let text = report_text("index out of bounds", Some(("src/hw/usb.rs", 120, 9)));
        assert!(text.starts_with(&format!("rs-cpuz {} ", env!("CARGO_PKG_VERSION"))), "{text}");
        assert!(text.ends_with("panicked at src/hw/usb.rs:120:9: index out of bounds"), "{text}");
        assert!(report_text("no place", None).ends_with("panicked: no place"));
    }

    #[test]
    fn the_message_says_whether_the_program_closes() {
        assert!(outcome(true).ends_with("has to close."));
        // A worker thread that panics takes only its own work along.
        let background = outcome(false);
        assert!(!background.contains("close") && background.contains("no longer update"), "{background}");
    }

    #[test]
    fn payloads_become_text() {
        assert_eq!(payload_text(&"literal"), "literal");
        assert_eq!(payload_text(&String::from("formatted 7")), "formatted 7");
        assert_eq!(payload_text(&42_u8), "(no message)");
    }

    #[test]
    fn the_log_keeps_every_report() {
        let dir = std::env::temp_dir().join(format!("rs-cpuz-test-{}-crash", std::process::id()));
        let path = dir.join("crash.log");
        let _ = std::fs::remove_dir_all(&dir);
        append(&path, "first").unwrap();
        append(&path, "second").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with("first") && lines[1].ends_with("second"), "{lines:?}");
    }
}
