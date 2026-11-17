//! Plain-text report.

use crate::brand;
use crate::model::{ClockSnapshot, CpuidLeaf, SystemInfo};

use super::doc::{self, Line};

/// The rule under the title and every section heading.
const RULE: &str = "-------------------------------------------------------------------------";
const EOL: &str = "\r\n";
/// Width of a tab stop, in characters.
const TAB_WIDTH: usize = 8;
/// Column at which values start.
const VALUE_COLUMN: usize = 32;

/// Renders the full text report.
pub fn txt(sys: &SystemInfo, clocks: Option<&ClockSnapshot>) -> String {
    let mut out = String::new();
    push_heading(&mut out, &format!("{} TXT Report", brand::APP_NAME));
    for section in doc::build(sys, clocks) {
        push_heading(&mut out, section.title);
        push_lines(&mut out, &section.lines);
        push(&mut out, "");
    }
    out
}

/// Appends a title, the rule under it and a blank line.
fn push_heading(out: &mut String, title: &str) {
    push(out, title);
    push(out, RULE);
    push(out, "");
}

/// Appends the text form of `lines`, each terminated by CRLF.
fn push_lines(out: &mut String, lines: &[Line]) {
    for line in lines {
        match line {
            Line::Pair { depth, key, value } => {
                let column = usize::from(*depth) * TAB_WIDTH + key.chars().count();
                push(out, &format!("{}{key}{}{value}", tabs(*depth), tabs(padding(column))));
            }
            Line::Short { key, tabs: count, value } => {
                push(out, &format!("{key}{}{value}", tabs(*count)));
            }
            Line::Heading { depth, text, trail } => {
                push(out, &format!("{}{text}{}", tabs(*depth), tabs(*trail)));
            }
            Line::Tree { depth, label, value } => {
                let marker = if *depth == 0 { "" } else { "-- " };
                push(out, &format!("{}{marker}{label}\t{value}", tabs(*depth)));
            }
            Line::Cpuid(leaves) => push_cpuid(out, leaves),
            Line::Blank => push(out, ""),
        }
    }
}

/// Appends a CPUID table: one row per (sub-)leaf with EAX, EBX, ECX and EDX.
fn push_cpuid(out: &mut String, leaves: &[CpuidLeaf]) {
    push(out, "\tCPUID\t\t ");
    for l in leaves {
        let row = format!(
            "\t0x{:08X}\t\t0x{:08X}\t0x{:08X}\t0x{:08X}\t0x{:08X}",
            l.leaf, l.eax, l.ebx, l.ecx, l.edx
        );
        push(out, &row);
    }
}

fn push(out: &mut String, text: &str) {
    out.push_str(text);
    out.push_str(EOL);
}

fn tabs(count: u8) -> String {
    "\t".repeat(usize::from(count))
}

/// Number of tabs that take a key ending at `column` to the value column (at least one).
fn padding(column: usize) -> u8 {
    let tabs = VALUE_COLUMN.saturating_sub(column).div_ceil(TAB_WIDTH).max(1);
    u8::try_from(tabs).unwrap_or(u8::MAX)
}

/// The text lines of one section body, for tests of the section builders.
#[cfg(test)]
pub(super) fn lines_of(section: &doc::Section) -> Vec<String> {
    let mut out = String::new();
    push_lines(&mut out, &section.lines);
    out.lines().map(String::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    #[test]
    fn pads_keys_to_the_value_column() {
        assert_eq!(padding(0), 4);
        assert_eq!(padding(13), 3);
        assert_eq!(padding(16), 2);
        assert_eq!(padding(17), 2);
        assert_eq!(padding(24), 1);
        assert_eq!(padding(31), 1);
        assert_eq!(padding(32), 1);
        assert_eq!(padding(40), 1);
    }

    #[test]
    fn renders_each_line_kind() {
        let mut out = String::new();
        let lines = [
            Line::Pair { depth: 0, key: "UEFI".into(), value: "Yes".into() },
            Line::Pair { depth: 1, key: "Number of threads".into(), value: "16".into() },
            Line::Pair { depth: 2, key: "Core clock".into(), value: "285.0 MHz".into() },
            Line::Short { key: "Drive".into(), tabs: 1, value: "0".into() },
            Line::Heading { depth: 0, text: "DMI BIOS".into(), trail: 2 },
            Line::Heading { depth: 0, text: "Common header".into(), trail: 0 },
            Line::Tree { depth: 0, label: "Socket 0".into(), value: String::new() },
            Line::Tree { depth: 2, label: "Thread 3".into(), value: "3".into() },
            Line::Blank,
        ];
        push_lines(&mut out, &lines);
        assert_eq!(
            out,
            "UEFI\t\t\t\tYes\r\n\tNumber of threads\t16\r\n\t\tCore clock\t285.0 MHz\r\nDrive\t0\r\n\
             DMI BIOS\t\t\r\nCommon header\r\nSocket 0\t\r\n\t\t-- Thread 3\t3\r\n\r\n"
        );
    }

    #[test]
    fn starts_with_the_title_block() {
        let report = txt(&demo::system(), Some(&demo::clocks()));
        let head = format!(
            "{name} TXT Report\r\n{RULE}\r\n\r\nBinaries\r\n{RULE}\r\n\r\n{name} version\t\t\t{version}\r\n\r\n\
             Processors\r\n{RULE}\r\n\r\nCPU Groups\t\t\t1\r\n",
            name = brand::APP_NAME,
            version = brand::VERSION
        );
        assert!(report.starts_with(&head));
        assert_eq!(RULE.len(), 73);
    }

    #[test]
    fn uses_crlf_line_endings_only() {
        let report = txt(&demo::system(), Some(&demo::clocks()));
        assert!(report.ends_with("\r\n"));
        assert_eq!(report.matches('\n').count(), report.matches("\r\n").count());
        assert_eq!(report.matches('\r').count(), report.matches("\r\n").count());
    }

    #[test]
    fn prints_every_section_heading_in_order() {
        for sys in [demo::system(), SystemInfo::default()] {
            let report = txt(&sys, None);
            let mut position = 0;
            for title in doc::SECTION_TITLES {
                let heading = format!("\r\n{title}\r\n{RULE}\r\n\r\n");
                let found = report[position..].find(&heading).unwrap_or_else(|| panic!("no {title} heading"));
                position += found + heading.len();
            }
        }
    }

    #[test]
    fn demo_report_contains_key_lines() {
        let report = txt(&demo::system(), Some(&demo::clocks()));
        for line in [
            "CPU Group 0\t\t\t16 threads, mask=0xFFFF",
            "\t\t\t\t\t-- Thread 15\t15",
            "\tACPI timer\t\t3.580 MHz",
            "Socket 1\t\t\tID = 0",
            "\tNumber of threads\t16 (max 16)",
            "\tL3 cache\t\t32 MB (16-way, 64-byte line)",
            "CPU Thread (0, 0x0)\t",
            "\tAMD Topology\t\tNode 0, CCD 0, CCX 0, core 7, thread 1",
            "\tCache descriptor\tLevel 2, U, 1 MB, 2 thread(s)",
            "UEFI\t\t\t\tYes",
            "Southbridge\t\t\tAMD B650 rev. 51",
            "PCI-E Link Width\t\tx8 (max 8x)",
            "Memory Frequency\t\t2600.1 MHz (1:26)",
            "Memory Max Frequency\t\t2800.0 MHz",
            "DIMM #\t\t\t\t2",
            "\tJEDEC #8\t\t42.0-43-43-85-127 @ 2633 MHz (1.100 Volts)",
            "Mainboard Model\t\tB650 EXAMPLE WIFI",
            "Description\t\t\tVGA Controller",
            "Location\t\t\tbus 1 (0x01), device 0 (0x00), function 0 (0x00)",
            "Drive\t0",
            "\tBus Type\t\tNVMe (17)",
            "\tVolume\t\t\tc:\\, 930.5 GBytes (41.5 percent available)",
            "USB Device\t\t\tUSB Composite Device, class=0xEF, subclass=0x02, vendor=0x1D6B, product=0x0104",
            "Number of adapters\t\t2",
            "API\t\t\t\tD3D",
            "Display adapter 0 (primary)\t",
            "\tCurrent Link Speed\t16.0 GT/s",
            "Monitor 0\t",
            "\tMax Resolution\t\t3840 x 2160 @ 59 Hz",
            "Windows Version\t\t\tMicrosoft Windows 11 Home (x64) Build 26100.2314",
            "DirectX Version\t\t\t12.0",
            "WHEA errors\t\t\t0",
        ] {
            assert!(report.contains(&format!("\r\n{line}\r\n")), "missing line: {line:?}");
        }
    }
}
