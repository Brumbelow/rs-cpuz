//! The two system dialogs the application uses: "Save As" and the error box.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Controls::Dialogs::{
    GetSaveFileNameW, OFN_EXPLORER, OFN_HIDEREADONLY, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT,
    OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

use crate::brand;

use super::gdi::wide;

/// A kind of file offered by the "Save As" dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileKind {
    /// Shown in the type list, e.g. `Text file`.
    pub description: &'static str,
    /// Without the dot, e.g. `txt`.
    pub extension: &'static str,
}

pub const TEXT_FILE: FileKind = FileKind { description: "Text file", extension: "txt" };
pub const HTML_FILE: FileKind = FileKind { description: "HTML file", extension: "htm" };
pub const BITMAP_FILE: FileKind = FileKind { description: "Bitmap image", extension: "bmp" };

/// Capacity of the path buffer handed to the dialog, in UTF-16 units.
const PATH_CAPACITY: usize = 1024;

/// The double-NUL-terminated filter list for `kind` plus an "All files" entry.
fn filter(kind: FileKind) -> Vec<u16> {
    let text = format!(
        "{description} (*.{extension})\0*.{extension}\0All files (*.*)\0*.*\0\0",
        description = kind.description,
        extension = kind.extension
    );
    text.encode_utf16().collect()
}

/// Copies `name` into a fixed path buffer, leaving it NUL-terminated.
fn path_buffer(name: &str) -> Vec<u16> {
    let mut buffer = vec![0u16; PATH_CAPACITY];
    for (slot, unit) in buffer.iter_mut().take(PATH_CAPACITY - 1).zip(name.encode_utf16()) {
        *slot = unit;
    }
    buffer
}

/// The path the dialog left in `buffer`, up to its first NUL.
fn path_from(buffer: &[u16]) -> Option<PathBuf> {
    let len = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
    (len > 0).then(|| PathBuf::from(OsString::from_wide(&buffer[..len])))
}

/// Asks where to save a file. Returns `None` when the user cancels.
pub fn save_file(owner: HWND, default_name: &str, kind: FileKind) -> Option<PathBuf> {
    let filter = filter(kind);
    let extension = wide(kind.extension);
    let mut path = path_buffer(default_name);
    let mut request = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        nFilterIndex: 1,
        lpstrFile: PWSTR(path.as_mut_ptr()),
        nMaxFile: PATH_CAPACITY as u32,
        lpstrDefExt: PCWSTR(extension.as_ptr()),
        Flags: OFN_EXPLORER | OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    // SAFETY: every pointer in `request` refers to a buffer that outlives the call, and
    // `nMaxFile` states the real capacity of the path buffer.
    let accepted = unsafe { GetSaveFileNameW(&mut request) }.as_bool();
    if accepted {
        path_from(&path)
    } else {
        None
    }
}

/// Shows an error message box owned by `owner`.
pub fn error(owner: Option<HWND>, text: &str) {
    let text = wide(text);
    let caption = wide(brand::APP_NAME);
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe {
        MessageBoxW(
            owner.unwrap_or_default(),
            PCWSTR(text.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_is_a_double_terminated_list() {
        let text = String::from_utf16(&filter(TEXT_FILE)).unwrap();
        assert_eq!(text, "Text file (*.txt)\0*.txt\0All files (*.*)\0*.*\0\0");
        let bitmap = String::from_utf16(&filter(BITMAP_FILE)).unwrap();
        assert!(bitmap.starts_with("Bitmap image (*.bmp)\0*.bmp\0"));
        assert!(String::from_utf16(&filter(HTML_FILE)).unwrap().contains("*.htm\0"));
    }

    #[test]
    fn path_buffer_round_trip() {
        let buffer = path_buffer("cpu.bmp");
        assert_eq!(buffer.len(), PATH_CAPACITY);
        assert_eq!(path_from(&buffer), Some(PathBuf::from("cpu.bmp")));
        assert_eq!(path_from(&path_buffer("")), None);
    }

    #[test]
    fn over_long_names_stay_terminated() {
        let long = "x".repeat(PATH_CAPACITY * 2);
        let buffer = path_buffer(&long);
        assert_eq!(buffer[PATH_CAPACITY - 1], 0);
        assert_eq!(path_from(&buffer).map(|p| p.as_os_str().len()), Some(PATH_CAPACITY - 1));
    }
}
