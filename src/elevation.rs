//! Administrator rights: whether the program has them, and asking Windows for them.
//!
//! Most of what rs-cpuz reads needs no elevation. Elevated, it can also open the PawnIO
//! driver when that is installed. The window asks for elevation when it opens, but only
//! an administrator whose rights Windows filters (the usual case with UAC on) is asked:
//! a standard user would be asked for another account's password and would then run as
//! that account, with its own settings. The command-line modes never ask, since an
//! elevated copy is a new process that loses the console and any redirection.

use std::mem::size_of;

use windows::core::{w, Owned, HSTRING, PCWSTR};
use windows::Win32::Foundation::{ERROR_CANCELLED, HANDLE, HWND};
use windows::Win32::Security::{
    GetTokenInformation, TokenElevation, TokenElevationType, TokenElevationTypeLimited, TOKEN_ELEVATION,
    TOKEN_ELEVATION_TYPE, TOKEN_QUERY,
};
use windows::Win32::System::Environment::GetCommandLineW;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOZONECHECKS, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::config;

/// The rights the process runs with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elevation {
    /// Full administrator rights.
    Elevated,
    /// An administrator whose rights Windows filters until elevation is approved.
    Filtered,
    /// A standard user, or rights that cannot be told.
    Standard,
}

/// What the program was asked to do, as far as elevation goes.
#[derive(Debug, Clone, Copy)]
pub struct Request {
    /// The window is opened (not a command-line mode).
    pub window: bool,
    /// The sample system is shown.
    pub demo: bool,
    /// `-noadmin` was given.
    pub no_admin: bool,
    /// `Elevate=1` (the default) in the settings.
    pub wanted: bool,
}

/// Why a relaunch did not happen.
#[derive(Debug)]
pub enum Refused {
    /// The user answered "No" to the prompt.
    Declined,
    Failed(windows::core::Error),
}

/// The rights of this process.
pub fn current() -> Elevation {
    let mut token = HANDLE::default();
    // SAFETY: the pseudo handle of the current process is always valid and `token` is a
    // live HANDLE the call writes to.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
        return Elevation::Standard;
    }
    // SAFETY: the call above opened the handle; `Owned` closes it once.
    let token = unsafe { Owned::new(token) };
    let mut elevation = TOKEN_ELEVATION::default();
    let mut kind = TOKEN_ELEVATION_TYPE::default();
    let mut length = 0u32;
    // SAFETY: each buffer is a live structure of the size passed, of the type the class returns.
    let elevated = unsafe {
        GetTokenInformation(
            *token,
            TokenElevation,
            Some(std::ptr::from_mut(&mut elevation).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut length,
        )
    }
    .is_ok_and(|()| elevation.TokenIsElevated != 0);
    // SAFETY: as above.
    let filtered = unsafe {
        GetTokenInformation(
            *token,
            TokenElevationType,
            Some(std::ptr::from_mut(&mut kind).cast()),
            size_of::<TOKEN_ELEVATION_TYPE>() as u32,
            &mut length,
        )
    }
    .is_ok_and(|()| kind == TokenElevationTypeLimited);
    match (elevated, filtered) {
        (true, _) => Elevation::Elevated,
        (false, true) => Elevation::Filtered,
        (false, false) => Elevation::Standard,
    }
}

/// Whether the program should ask for elevation on start.
pub fn should_ask(elevation: Elevation, request: &Request) -> bool {
    elevation == Elevation::Filtered && request.window && !request.demo && !request.no_admin && request.wanted
}

/// Starts an elevated copy of the program with the same arguments, after the user agreed
/// to the prompt. `owner` is the window the prompt belongs to, if any.
pub fn relaunch(owner: Option<HWND>) -> Result<(), Refused> {
    let exe = std::env::current_exe().map_err(|error| Refused::Failed(windows::core::Error::from(error)))?;
    // SAFETY: GetCommandLineW returns the process's command line, which lives as long as the process.
    let line = unsafe { GetCommandLineW().to_string() }.unwrap_or_default();
    let parameters = HSTRING::from(relaunch_arguments(command_tail(&line)));
    let file = HSTRING::from(exe.as_os_str());
    let directory = HSTRING::from(config::exe_dir().as_os_str());
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_NOZONECHECKS | SEE_MASK_FLAG_NO_UI,
        hwnd: owner.unwrap_or_default(),
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        lpDirectory: PCWSTR(directory.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: `info` and the strings it points to outlive the call.
    unsafe { ShellExecuteExW(&mut info) }.map_err(|error| {
        if error.code() == ERROR_CANCELLED.to_hresult() {
            Refused::Declined
        } else {
            Refused::Failed(error)
        }
    })
}

/// The arguments of a command line, after the program name, as they were typed. The name
/// is parsed the way the C runtime and Rust's own arguments parse it: it ends at the first
/// space or tab outside quotes, and every quote opens or closes a quoted part.
fn command_tail(line: &str) -> &str {
    let mut quoted = false;
    for (at, character) in line.char_indices() {
        match character {
            '"' => quoted = !quoted,
            ' ' | '\t' if !quoted => return line[at..].trim_start_matches([' ', '\t']),
            _ => {}
        }
    }
    ""
}

/// The arguments the elevated copy gets: the same, plus `-noadmin`, so that a copy that
/// still is not elevated (UAC off for a standard user) does not ask again.
fn relaunch_arguments(tail: &str) -> String {
    if tail.is_empty() {
        "-noadmin".to_string()
    } else {
        format!("{tail} -noadmin")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_filtered_administrator_is_asked_in_the_window() {
        let ask = |elevation, window, demo, no_admin, wanted| {
            should_ask(elevation, &Request { window, demo, no_admin, wanted })
        };
        assert!(ask(Elevation::Filtered, true, false, false, true));
        // Already elevated, a standard user (whom the prompt would ask for another account's
        // password), the command-line modes, the sample system, and either switch: no prompt.
        assert!(!ask(Elevation::Elevated, true, false, false, true));
        assert!(!ask(Elevation::Standard, true, false, false, true));
        assert!(!ask(Elevation::Filtered, false, false, false, true));
        assert!(!ask(Elevation::Filtered, true, true, false, true));
        assert!(!ask(Elevation::Filtered, true, false, true, true));
        assert!(!ask(Elevation::Filtered, true, false, false, false));
    }

    #[test]
    fn the_arguments_follow_the_program_name() {
        assert_eq!(command_tail(r#""C:\Program Files\rs-cpuz\rs-cpuz.exe" -core=3 -demo"#), "-core=3 -demo");
        assert_eq!(command_tail(r"C:\tools\rs-cpuz.exe   -txt=a"), "-txt=a");
        assert_eq!(command_tail(r#""C:\a b\rs-cpuz.exe""#), "");
        assert_eq!(command_tail("rs-cpuz.exe"), "");
        // Quotes inside the arguments are kept as they were typed.
        assert_eq!(command_tail(r#"rs-cpuz.exe "-txt=my report" -core=2"#), r#""-txt=my report" -core=2"#);
        assert_eq!(command_tail(""), "");
        // A quoted part inside the name keeps its spaces, and a closing quote does not end
        // the name: the program itself got no `-core=2` from the second line either.
        assert_eq!(command_tail(r#"C:\"Program Files"\rs-cpuz\rs-cpuz.exe -core=2"#), "-core=2");
        assert_eq!(command_tail(r#""C:\a b\rs-cpuz.exe"-core=2"#), "");
    }

    #[test]
    fn the_relaunch_keeps_the_arguments_and_asks_no_more() {
        assert_eq!(relaunch_arguments("-core=3"), "-core=3 -noadmin");
        assert_eq!(relaunch_arguments(""), "-noadmin");
    }

    #[test]
    fn this_process_has_an_elevation_state() {
        // Whatever the test runs under, the token can be asked.
        assert!(matches!(current(), Elevation::Elevated | Elevation::Filtered | Elevation::Standard));
    }
}
