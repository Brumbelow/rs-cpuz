//! The report as data: sections made of lines, built once from the model and
//! then rendered as text or HTML.

use std::fmt::Display;

use crate::brand;
use crate::model::{ClockSnapshot, CpuidLeaf, SystemInfo};

use super::{board, devices, dmi, processor, storage};

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Line {
    /// `key<TABS>value`, indented by `depth` levels.
    Pair {
        depth: u8,
        key: String,
        value: String,
    },
    /// `key<TABS>value` with a set number of tabs, for the lines whose value starts
    /// short of the value column.
    Short {
        key: String,
        tabs: u8,
        value: String,
    },
    /// A block heading; the text report appends `trail` tabs to it.
    Heading {
        depth: u8,
        text: String,
        trail: u8,
    },
    /// One node of the APIC tree. `value` is empty on inner nodes.
    Tree {
        depth: u8,
        label: String,
        value: String,
    },
    /// The CPUID registers of one logical processor.
    Cpuid(Vec<CpuidLeaf>),
    Blank,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Section {
    pub title: &'static str,
    pub lines: Vec<Line>,
}

/// Collects the lines of one section. Pairs with an unknown value are dropped,
/// so section builders can list every field unconditionally.
#[derive(Debug, Default)]
pub(super) struct Lines(Vec<Line>);

impl Lines {
    /// Adds a `key` / `value` line unless `value` is empty.
    pub fn pair(&mut self, depth: u8, key: &str, value: impl Into<String>) {
        let value = single_line(value.into());
        if !value.is_empty() {
            self.0.push(Line::Pair { depth, key: key.to_string(), value });
        }
    }

    /// Adds a `key` / `value` line unless `value` is `None`.
    pub fn opt<T: Display>(&mut self, depth: u8, key: &str, value: Option<T>) {
        if let Some(value) = value {
            self.pair(depth, key, value.to_string());
        }
    }

    /// Adds a `key` / `value` line with `tabs` tabs between the two, unless `value` is empty.
    pub fn short(&mut self, key: &str, tabs: u8, value: impl Into<String>) {
        let value = single_line(value.into());
        if !value.is_empty() {
            self.0.push(Line::Short { key: key.to_string(), tabs, value });
        }
    }

    pub fn heading(&mut self, depth: u8, text: impl Into<String>, trail: u8) {
        self.0.push(Line::Heading { depth, text: text.into(), trail });
    }

    pub fn tree(&mut self, depth: u8, label: String, value: String) {
        self.0.push(Line::Tree { depth, label, value });
    }

    pub fn cpuid(&mut self, leaves: &[CpuidLeaf]) {
        self.0.push(Line::Cpuid(leaves.to_vec()));
    }

    pub fn blank(&mut self) {
        self.0.push(Line::Blank);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Moves every line of `other` to the end of this collection.
    pub fn append(&mut self, mut other: Lines) {
        self.0.append(&mut other.0);
    }

    pub fn into_section(self, title: &'static str) -> Section {
        Section { title, lines: self.0 }
    }
}

/// Replaces control characters with spaces: a tab or line break inside a
/// firmware or driver string would break the layout of the text report.
fn single_line(text: String) -> String {
    if text.contains(char::is_control) {
        text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
    } else {
        text
    }
}

/// Builds every section of the report, in output order.
pub(super) fn build(sys: &SystemInfo, clocks: Option<&ClockSnapshot>) -> Vec<Section> {
    vec![
        binaries(),
        processor::processors(&sys.cpu),
        processor::apics(&sys.cpu),
        processor::timers(&sys.timers),
        processor::information(&sys.cpu, clocks),
        processor::thread_dumps(&sys.cpu),
        board::bios(&sys.mainboard),
        board::chipset(&sys.mainboard, &sys.memory),
        board::spd(&sys.modules),
        board::monitoring(&sys.mainboard),
        devices::pci(&sys.pci),
        dmi::section(sys.smbios.as_ref()),
        storage::section(&sys.drives),
        devices::usb(&sys.usb),
        devices::graphics(&sys.gpus),
        devices::graphic_apis(&sys.graphic_apis),
        devices::display_adapters(&sys.gpus, &sys.monitors),
        devices::software(&sys.os),
    ]
}

/// The section titles in output order, for the tests of the document and its two renderings.
#[cfg(test)]
pub(super) const SECTION_TITLES: [&str; 18] = [
    "Binaries",
    "Processors",
    "APICs",
    "Timers",
    "Processors Information",
    "Thread dumps",
    "BIOS",
    "Chipset",
    "Memory SPD",
    "Monitoring",
    "PCI Devices",
    "DMI",
    "Storage",
    "USB Devices",
    "Graphics",
    "Graphic APIs",
    "Display Adapters",
    "Software",
];

fn binaries() -> Section {
    let mut out = Lines::default();
    out.pair(0, &format!("{} version", brand::APP_NAME), brand::VERSION);
    out.into_section("Binaries")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    #[test]
    fn keeps_values_on_one_line() {
        assert_eq!(single_line("B650 EXAMPLE WIFI".into()), "B650 EXAMPLE WIFI");
        assert_eq!(single_line("two\r\nlines\tand a tab".into()), "two  lines and a tab");
        let mut out = Lines::default();
        out.pair(1, "model", "Board\u{1}X");
        assert_eq!(
            out.into_section("Test").lines,
            [Line::Pair { depth: 1, key: "model".into(), value: "Board X".into() }]
        );
    }

    #[test]
    fn drops_unknown_values() {
        let mut out = Lines::default();
        out.pair(0, "Name", "");
        out.opt(1, "Codename", None::<&str>);
        assert!(out.is_empty());
        out.pair(0, "Name", "value");
        out.opt(1, "Count", Some(3));
        assert_eq!(
            out.into_section("Test").lines,
            [
                Line::Pair { depth: 0, key: "Name".into(), value: "value".into() },
                Line::Pair { depth: 1, key: "Count".into(), value: "3".into() },
            ]
        );
    }

    #[test]
    fn short_lines_keep_their_tabs() {
        let mut out = Lines::default();
        out.short("Drive", 1, "");
        assert!(out.is_empty());
        out.short("Drive", 1, "0");
        out.short("Mainboard Model", 2, "B650\tEXAMPLE");
        assert_eq!(
            out.into_section("Test").lines,
            [
                Line::Short { key: "Drive".into(), tabs: 1, value: "0".into() },
                Line::Short { key: "Mainboard Model".into(), tabs: 2, value: "B650 EXAMPLE".into() },
            ]
        );
    }

    #[test]
    fn sections_follow_the_report_order() {
        let titles: Vec<_> = build(&demo::system(), None).iter().map(|s| s.title).collect();
        assert_eq!(titles, SECTION_TITLES);
    }

    #[test]
    fn empty_system_keeps_every_heading() {
        let sections = build(&SystemInfo::default(), None);
        assert_eq!(sections.len(), SECTION_TITLES.len());
        assert!(sections.iter().skip(1).all(|s| s.lines.iter().all(|line| *line == Line::Blank)));
    }
}
