//! Optional settings read from `rs-cpuz.ini`, and the folders rs-cpuz keeps files in.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "rs-cpuz.ini";

/// Folder under `%APPDATA%` for the files of a copy whose own folder is closed to it.
const APP_FOLDER: &str = "rs-cpuz";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Font of the value boxes; empty selects the built-in default.
    pub text_font_name: String,
    /// Pixel height of the value font at 96 DPI.
    pub text_font_size: u32,
    /// Colour of the value text as `0xRRGGBB`; `None` keeps the theme colour.
    pub text_font_color: Option<u32>,
    /// Font of the labels; empty selects the built-in default.
    pub label_font_name: String,
    /// Pixel height of the label font at 96 DPI.
    pub label_font_size: u32,
    /// Read the firmware DMI (SMBIOS) tables.
    pub dmi: bool,
    /// Enumerate PCI devices (chipset, graphic interface).
    pub pci: bool,
    /// Query the display adapters.
    pub display: bool,
    /// Ask for administrator rights when the window opens.
    pub elevate: bool,
    /// Use the PawnIO driver, when it is installed and the program is elevated.
    pub pawnio: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            text_font_name: String::new(),
            text_font_size: 14,
            text_font_color: None,
            label_font_name: String::new(),
            label_font_size: 14,
            dmi: true,
            pci: true,
            display: true,
            elevate: true,
            pawnio: true,
        }
    }
}

impl Config {
    /// Loads the settings file from the executable's folder, else from `%APPDATA%\rs-cpuz`;
    /// without one the defaults apply.
    pub fn load() -> Self {
        Self::load_from(&search_dirs())
    }

    /// Loads the settings file from the first of `dirs` that has one.
    fn load_from(dirs: &[PathBuf]) -> Self {
        dirs.iter()
            .find_map(|dir| fs::read(dir.join(FILE_NAME)).ok())
            .map(|bytes| Self::parse_bytes(&bytes))
            .unwrap_or_default()
    }

    /// Parses a settings file in whatever encoding Notepad saved it: UTF-8 with or without
    /// a byte order mark, UTF-16, or the ANSI code page (read as Latin-1, which keeps the
    /// ASCII keys and most accented names).
    fn parse_bytes(bytes: &[u8]) -> Self {
        let wide = |big_endian: bool| {
            let units: Vec<u16> = bytes[2..]
                .chunks_exact(2)
                .map(|pair| {
                    if big_endian {
                        u16::from_be_bytes([pair[0], pair[1]])
                    } else {
                        u16::from_le_bytes([pair[0], pair[1]])
                    }
                })
                .collect();
            String::from_utf16_lossy(&units)
        };
        let text = match bytes {
            [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
            [0xFF, 0xFE, ..] => wide(false),
            [0xFE, 0xFF, ..] => wide(true),
            _ => match std::str::from_utf8(bytes) {
                Ok(text) => text.to_string(),
                Err(_) => bytes.iter().map(|&byte| char::from(byte)).collect(),
            },
        };
        Self::parse(&text)
    }

    /// Parses INI text. Unknown keys, sections and malformed values are ignored.
    pub fn parse(text: &str) -> Self {
        let mut cfg = Config::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with([';', '#', '[']) {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            let flag = |current: bool| match value.to_ascii_lowercase().as_str() {
                "1" | "on" | "true" | "yes" => true,
                "0" | "off" | "false" | "no" => false,
                _ => current,
            };
            let size = |current: u32| value.parse().ok().filter(|n| (6..=72).contains(n)).unwrap_or(current);
            match key.trim().to_ascii_lowercase().as_str() {
                "textfontname" => cfg.text_font_name = value.to_string(),
                "textfontsize" => cfg.text_font_size = size(cfg.text_font_size),
                "textfontcolor" => {
                    cfg.text_font_color = u32::from_str_radix(value, 16)
                        .ok()
                        .filter(|c| *c <= 0xFF_FFFF)
                        .or(cfg.text_font_color)
                }
                "labelfontname" => cfg.label_font_name = value.to_string(),
                "labelfontsize" => cfg.label_font_size = size(cfg.label_font_size),
                "dmi" => cfg.dmi = flag(cfg.dmi),
                "pci" => cfg.pci = flag(cfg.pci),
                "display" => cfg.display = flag(cfg.display),
                "elevate" => cfg.elevate = flag(cfg.elevate),
                "pawnio" => cfg.pawnio = flag(cfg.pawnio),
                _ => {}
            }
        }
        cfg
    }
}

/// Directory containing the running executable (the current directory if it cannot be determined).
pub fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)).unwrap_or_default()
}

/// `%APPDATA%\rs-cpuz`, where a copy whose own folder is closed to it keeps its files.
pub fn app_data_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join(APP_FOLDER))
}

/// The folder rs-cpuz writes its files to: next to the executable when that folder takes
/// new files (a copy run from a download folder or a USB stick), else `%APPDATA%\rs-cpuz`
/// (a copy installed under Program Files, run without administrator rights).
pub fn data_dir() -> PathBuf {
    choose_data_dir(exe_dir(), app_data_dir())
}

/// The folders files are read from: the executable's, then `%APPDATA%\rs-cpuz`.
pub fn search_dirs() -> Vec<PathBuf> {
    std::iter::once(exe_dir()).chain(app_data_dir()).collect()
}

fn choose_data_dir(exe: PathBuf, app_data: Option<PathBuf>) -> PathBuf {
    if accepts_files(&exe) {
        return exe;
    }
    match app_data {
        Some(dir) if fs::create_dir_all(&dir).is_ok() => dir,
        _ => exe,
    }
}

/// Whether a new file can be created in `dir`: tried with a probe that is removed again.
fn accepts_files(dir: &Path) -> bool {
    let probe = dir.join(format!(".rs-cpuz-probe-{}", std::process::id()));
    match OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_is_default() {
        assert_eq!(Config::parse(""), Config::default());
    }

    #[test]
    fn parses_known_keys() {
        let cfg = Config::parse(
            "[RS-CPUZ]\r\nTextFontName=Consolas\r\nTextFontSize=16\r\nTextFontColor=00FF80\r\n\
             LabelFontName = Segoe UI \r\nLabelFontSize=12\r\nDMI=0\r\nPCI=off\r\nDisplay=1\r\n",
        );
        assert_eq!(cfg.text_font_name, "Consolas");
        assert_eq!(cfg.text_font_size, 16);
        assert_eq!(cfg.text_font_color, Some(0x00FF80));
        assert_eq!(cfg.label_font_name, "Segoe UI");
        assert_eq!(cfg.label_font_size, 12);
        assert!(!cfg.dmi);
        assert!(!cfg.pci);
        assert!(cfg.display);
    }

    /// A fresh, empty folder under the temporary directory.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rs-cpuz-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn data_stays_next_to_a_writable_executable() {
        let exe = scratch("portable");
        assert!(accepts_files(&exe));
        assert_eq!(choose_data_dir(exe.clone(), Some(exe.join("unused"))), exe);
        // The probe leaves nothing behind.
        assert_eq!(std::fs::read_dir(&exe).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&exe);
    }

    #[test]
    fn data_moves_to_app_data_when_the_executable_folder_is_closed() {
        let root = scratch("installed");
        let closed = root.join("no such folder");
        let app_data = root.join("AppData").join("rs-cpuz");
        assert!(!accepts_files(&closed));
        assert_eq!(choose_data_dir(closed.clone(), Some(app_data.clone())), app_data);
        assert!(app_data.is_dir());
        // Without an application data folder there is nowhere better to go.
        assert_eq!(choose_data_dir(closed.clone(), None), closed);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn settings_come_from_the_first_folder_that_has_them() {
        let root = scratch("settings");
        let (exe, app_data) = (root.join("exe"), root.join("appdata"));
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::create_dir_all(&app_data).unwrap();
        std::fs::write(app_data.join(FILE_NAME), "TextFontSize=20").unwrap();
        assert_eq!(Config::load_from(&[exe.clone(), app_data.clone()]).text_font_size, 20);
        std::fs::write(exe.join(FILE_NAME), "TextFontSize=18").unwrap();
        assert_eq!(Config::load_from(&[exe, app_data]).text_font_size, 18);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_settings_saved_in_any_windows_encoding() {
        let text = "LabelFontName=Ségoe\r\nDMI=0\r\n";
        let check = |bytes: Vec<u8>, encoding: &str| {
            let cfg = Config::parse_bytes(&bytes);
            assert_eq!(cfg.label_font_name, "Ségoe", "{encoding}");
            assert!(!cfg.dmi, "{encoding}");
        };
        check(text.as_bytes().to_vec(), "UTF-8");
        check([&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat(), "UTF-8 with BOM");
        let wide: Vec<u8> =
            [0xFEFF_u16].into_iter().chain(text.encode_utf16()).flat_map(u16::to_le_bytes).collect();
        check(wide, "UTF-16");
        let ansi: Vec<u8> = text.chars().map(|c| u8::try_from(u32::from(c)).unwrap()).collect();
        check(ansi, "ANSI");
    }

    #[test]
    fn elevation_and_the_driver_can_be_switched_off() {
        let defaults = Config::default();
        assert!(defaults.elevate && defaults.pawnio);
        let cfg = Config::parse("Elevate=0\nPawnIO=off\n");
        assert!(!cfg.elevate && !cfg.pawnio);
    }

    #[test]
    fn ignores_malformed_values() {
        let cfg = Config::parse("; comment\nTextFontSize=huge\nTextFontSize=500\nTextFontColor=zzz\nDMI=maybe\nnonsense\nUnknown=1\n");
        assert_eq!(cfg, Config::default());
    }
}
