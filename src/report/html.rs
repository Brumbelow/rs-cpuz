//! HTML report.

use crate::brand;
use crate::model::{ClockSnapshot, CpuidLeaf, SystemInfo};

use super::doc::{self, Line};

/// Embedded stylesheet: a dark neutral theme with the brand accent.
const STYLE: &str = "\
:root{color-scheme:dark}
body{margin:0 auto;padding:24px;max-width:980px;background:#1b1c1e;color:#d4d4d4;font:14px/1.45 \"Segoe UI\",Tahoma,sans-serif}
h1{margin:0 0 24px;padding-bottom:8px;border-bottom:2px solid #ce422b;font-size:22px;color:#f2f2f2}
h2{margin:32px 0 10px;padding-left:10px;border-left:4px solid #ce422b;font-size:17px;color:#f2f2f2}
table{width:100%;margin:0 0 14px;border-collapse:collapse}
th,td{padding:3px 10px;border-bottom:1px solid #2c2d30;text-align:left;vertical-align:top}
th{width:270px;font-weight:normal;color:#9a9da3}
td{overflow-wrap:anywhere}
tr.h th{padding-top:10px;font-weight:600;color:#ce422b}
tr.d1 th{text-indent:18px}
tr.d2 th{text-indent:36px}
tr.d3 th{text-indent:54px}
tr.d4 th{text-indent:72px}
tr.d5 th{text-indent:90px}
pre{margin:0 0 14px;padding:10px 14px;overflow-x:auto;background:#232427;border:1px solid #2c2d30;font:12px/1.4 Consolas,\"Courier New\",monospace}
";

/// Renders the full report as a standalone HTML document.
pub fn html(sys: &SystemInfo, clocks: Option<&ClockSnapshot>) -> String {
    let title = escape(&format!("{} HTML Report", brand::APP_NAME));
    let mut out = format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title}</title>\n<style>\n{STYLE}</style>\n</head>\n<body>\n<h1>{title}</h1>\n"
    );
    for section in doc::build(sys, clocks) {
        out.push_str(&format!("<section>\n<h2>{}</h2>\n", escape(section.title)));
        push_lines(&mut out, &section.lines);
        out.push_str("</section>\n");
    }
    out.push_str("</body>\n</html>\n");
    out
}

/// Appends the lines of one section: runs of rows become tables, CPUID dumps
/// become `<pre>` blocks and blank lines end the current table.
fn push_lines(out: &mut String, lines: &[Line]) {
    let mut body = Body { out, in_table: false };
    for line in lines {
        match line {
            Line::Pair { depth, key, value } => body.row(*depth, key, value),
            Line::Short { key, value, .. } => body.row(0, key, value),
            Line::Tree { depth, label, value } => body.row(*depth, label, value),
            Line::Heading { depth, text, .. } => body.heading(*depth, text),
            Line::Cpuid(leaves) => body.cpuid(leaves),
            Line::Blank => body.end_table(),
        }
    }
    body.end_table();
}

/// Writes a section body, opening a `<table>` on the first row of each run of rows.
struct Body<'a> {
    out: &'a mut String,
    in_table: bool,
}

impl Body<'_> {
    fn row(&mut self, depth: u8, key: &str, value: &str) {
        self.begin_table();
        self.out.push_str(&format!(
            "<tr class=\"d{depth}\"><th>{}</th><td>{}</td></tr>\n",
            escape(key),
            escape(value)
        ));
    }

    fn heading(&mut self, depth: u8, text: &str) {
        self.begin_table();
        self.out
            .push_str(&format!("<tr class=\"h d{depth}\"><th colspan=\"2\">{}</th></tr>\n", escape(text)));
    }

    /// Writes a CPUID table as preformatted text: one row per (sub-)leaf.
    fn cpuid(&mut self, leaves: &[CpuidLeaf]) {
        self.end_table();
        self.out.push_str("<pre>Leaf        EAX         EBX         ECX         EDX\n");
        for l in leaves {
            self.out.push_str(&format!(
                "0x{:08X}  0x{:08X}  0x{:08X}  0x{:08X}  0x{:08X}\n",
                l.leaf, l.eax, l.ebx, l.ecx, l.edx
            ));
        }
        self.out.push_str("</pre>\n");
    }

    fn begin_table(&mut self) {
        if !self.in_table {
            self.out.push_str("<table>\n");
            self.in_table = true;
        }
    }

    fn end_table(&mut self) {
        if self.in_table {
            self.out.push_str("</table>\n");
            self.in_table = false;
        }
    }
}

/// Escapes text for use in HTML element content and attribute values.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    #[test]
    fn escapes_markup() {
        assert_eq!(escape("AMD Radeon(TM) Graphics"), "AMD Radeon(TM) Graphics");
        assert_eq!(escape("<b>\"R&D\"</b> 'x'"), "&lt;b&gt;&quot;R&amp;D&quot;&lt;/b&gt; &#39;x&#39;");
    }

    #[test]
    fn is_a_standalone_document() {
        let report = html(&demo::system(), Some(&demo::clocks()));
        assert!(report.starts_with("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">"));
        assert!(report.ends_with("</body>\n</html>\n"));
        assert!(report.contains(&format!("<h1>{} HTML Report</h1>", brand::APP_NAME)));
        assert!(report.contains("#ce422b"));
        for external in ["http://", "https://", "<script", "<link", "<img", "@import", "url("] {
            assert!(!report.contains(external), "found {external}");
        }
    }

    #[test]
    fn has_one_heading_per_section() {
        for sys in [demo::system(), SystemInfo::default()] {
            let report = html(&sys, None);
            let mut position = 0;
            for title in doc::SECTION_TITLES {
                let heading = format!("<section>\n<h2>{title}</h2>\n");
                let found = report[position..].find(&heading).unwrap_or_else(|| panic!("no {title} heading"));
                position += found + heading.len();
            }
            assert_eq!(report.matches("<h2>").count(), doc::SECTION_TITLES.len());
            assert_eq!(report.matches("<section>").count(), report.matches("</section>").count());
        }
    }

    #[test]
    fn carries_the_report_content() {
        let report = html(&demo::system(), Some(&demo::clocks()));
        for fragment in [
            "<tr class=\"d0\"><th>CPU Group 0</th><td>16 threads, mask=0xFFFF</td></tr>",
            "<tr class=\"d0\"><th>Socket 0</th><td></td></tr>",
            "<tr class=\"d3\"><th>CCX 0</th><td></td></tr>",
            "<tr class=\"d5\"><th>Thread 15</th><td>15</td></tr>",
            "<tr class=\"d1\"><th>Number of cores</th><td>8 (max 8)</td></tr>",
            "<tr class=\"d1\"><th>Name</th><td>AMD Ryzen 7 7700</td></tr>",
            "<tr class=\"d1\"><th>Base frequency (cores)</th><td>99.7 MHz</td></tr>",
            "<tr class=\"d1\"><th>Clock Speed 8</th><td>99.74 MHz (CPU BCLK)</td></tr>",
            "<tr class=\"h d0\"><th colspan=\"2\">CPU Thread (0, 0x0)</th></tr>",
            "<tr class=\"d1\"><th>AMD Topology</th><td>Node 0, CCD 0, CCX 0, core 7, thread 1</td></tr>",
            "<tr class=\"d1\"><th>Cache descriptor</th><td>Level 3, U, 32 MB, 16 thread(s)</td></tr>",
            "<pre>Leaf        EAX         EBX         ECX         EDX\n\
             0x00000000  0x0000000D  0x68747541  0x444D4163  0x69746E65\n\
             0x00000001  0x00A60F12  0x00100800  0xFED8320B  0x178BFBFF\n\
             0x0000000B  0x00000001  0x00000002  0x00000100  0x00000000\n",
            "0x80000026  0x00000004  0x00000010  0x00000403  0x0000000F\n</pre>",
            "<tr class=\"d0\"><th>Southbridge</th><td>AMD B650 rev. 51</td></tr>",
            "<tr class=\"d0\"><th>Memory Max Frequency</th><td>2800.0 MHz</td></tr>",
            "<tr class=\"d0\"><th>Mainboard Model</th><td>B650 EXAMPLE WIFI</td></tr>",
            "<tr class=\"d0\"><th>Description</th><td>Host Bridge</td></tr>",
            "<tr class=\"d0\"><th>Drive</th><td>0</td></tr>",
            "<tr class=\"d1\"><th>Controller</th><td>NVM Express (NVMe) Controller at bus 2, device 0, function 0</td></tr>",
            "<tr class=\"d1\"><th>Link Speed</th><td>PCI-E 4x @ 16.0 GT/s</td></tr>",
            "<tr class=\"d0\"><th>USB Device</th><td>Generic USB Hub, class=0x09, subclass=0x00, vendor=0x1D6B, product=0x0002</td></tr>",
            "<tr class=\"d0\"><th>API</th><td>NVIDIA NVAPI</td></tr>",
            "<tr class=\"h d0\"><th colspan=\"2\">Monitor 0</th></tr>",
            "<tr class=\"d1\"><th>Video Input</th><td>Display Port</td></tr>",
            "<tr class=\"d0\"><th>WHEA errors</th><td>0</td></tr>",
            "<tr class=\"d1\"><th>JEDEC #9</th><td>46.0-45-45-90-135 @ 2800 MHz (1.100 Volts)</td></tr>",
            "<tr class=\"h d0\"><th colspan=\"2\">Common header</th></tr>",
            "<tr class=\"h d0\"><th colspan=\"2\">Display adapter 0 (primary)</th></tr>",
            "<tr class=\"d2\"><th>Vendor ID</th><td>0x10DE (0x1462)</td></tr>",
            "<tr class=\"d0\"><th>Windows Version</th><td>Microsoft Windows 11 Home (x64) Build 26100.2314</td></tr>",
        ] {
            assert!(report.contains(fragment), "missing fragment: {fragment}");
        }
    }

    #[test]
    fn escapes_values_from_the_system() {
        let mut sys = demo::system();
        sys.cpu.name = "<script>alert(1)</script>".into();
        sys.gpus[0].name = "A & B \"Turbo\"".into();
        // A class code without a name of its own, so the description from Windows is shown.
        sys.pci[0].base_class = 0xFF;
        sys.pci[0].description = "<img src=x>".into();
        // Names that come straight from a device: a drive, a USB device and a monitor.
        sys.drives[0].name = "<b>SSD</b>".into();
        sys.usb[0].name = "Hub <script>".into();
        sys.monitors[0].model = Some("27\" <i>".into());
        let report = html(&sys, None);
        assert!(!report.contains("<script"));
        assert!(!report.contains("<img"));
        assert!(!report.contains("<b>") && !report.contains("<i>"));
        assert!(report.contains("<td>&lt;script&gt;alert(1)&lt;/script&gt;</td>"));
        assert!(report.contains("<td>A &amp; B &quot;Turbo&quot;</td>"));
        assert!(report.contains("<td>&lt;img src=x&gt;</td>"));
        assert!(report.contains("<td>&lt;b&gt;SSD&lt;/b&gt;</td>"));
        assert!(report.contains(
            "<td>Hub &lt;script&gt;, class=0x09, subclass=0x00, vendor=0x1D6B, product=0x0002</td>"
        ));
        assert!(report.contains("<td>27&quot; &lt;i&gt;</td>"));
    }

    #[test]
    fn balances_tables() {
        for sys in [demo::system(), SystemInfo::default()] {
            let report = html(&sys, Some(&demo::clocks()));
            assert_eq!(report.matches("<table>").count(), report.matches("</table>").count());
            assert_eq!(report.matches("<tr").count(), report.matches("</tr>").count());
            assert_eq!(report.matches("<pre>").count(), report.matches("</pre>").count());
            assert!(!report.contains("<table>\n</table>"));
        }
    }
}
