//! Optional settings read from `rs-cpuz.ini` next to the executable.

use std::path::PathBuf;

pub const FILE_NAME: &str = "rs-cpuz.ini";

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
        }
    }
}

impl Config {
    /// Loads the settings file; a missing or unreadable file yields the defaults.
    pub fn load() -> Self {
        std::fs::read_to_string(exe_dir().join(FILE_NAME)).map(|text| Self::parse(&text)).unwrap_or_default()
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

    #[test]
    fn ignores_malformed_values() {
        let cfg = Config::parse("; comment\nTextFontSize=huge\nTextFontSize=500\nTextFontColor=zzz\nDMI=maybe\nnonsense\nUnknown=1\n");
        assert_eq!(cfg, Config::default());
    }
}
