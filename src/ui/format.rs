//! Value formatting shared by the tab pages and the tool windows.

use crate::model::{CoreKind, CoreSet};

/// Machine architecture suffix shown next to the version number.
pub const ARCH: &str = if cfg!(target_arch = "x86_64") { "x64" } else { "x86" };

/// `65.0 W`
pub fn watts(value: f32) -> String {
    format!("{value:.1} W")
}

/// `5 nm`
pub fn nanometers(value: u32) -> String {
    format!("{value} nm")
}

/// `1.016 V`
pub fn core_voltage(value: f32) -> String {
    format!("{value:.3} V")
}

/// Upper-case hexadecimal without a prefix, as used for family, model and stepping.
pub fn hex(value: u32) -> String {
    format!("{value:X}")
}

/// `0xA601203`
pub fn microcode(value: u32) -> String {
    format!("0x{value:X}")
}

/// `5136.63 MHz`
pub fn mhz2(value: f64) -> String {
    format!("{value:.2} MHz")
}

/// `2600.1 MHz`
pub fn mhz1(value: f64) -> String {
    format!("{value:.1} MHz")
}

/// `2633 MHz`
pub fn mhz0(value: f64) -> String {
    format!("{value:.0} MHz")
}

/// A multiplier with one decimal, or two when it sits on a quarter step: `51.5`, `53.25`.
fn multiplier_number(value: f64) -> String {
    let tenths = value * 10.0;
    if (tenths - tenths.round()).abs() < 1e-6 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    }
}

/// `x 51.5 (4.0 - 53.5)` when both bounds are known, `x 51.5 (max 53.5)` when
/// only the upper one is, `x 51.5` otherwise.
pub fn multiplier(current: f64, min: Option<f64>, max: Option<f64>) -> String {
    let current = multiplier_number(current);
    match (min, max) {
        (Some(min), Some(max)) => {
            format!("x {current} ({} - {})", multiplier_number(min), multiplier_number(max))
        }
        (None, Some(max)) => format!("x {current} (max {})", multiplier_number(max)),
        _ => format!("x {current}"),
    }
}

/// `x 51.50`, the multiplier column of the Clocks tool.
pub fn multiplier2(value: f64) -> String {
    format!("x {value:.2}")
}

/// `x8`
pub fn link_width(lanes: u32) -> String {
    format!("x{lanes}")
}

/// `16.0 GT/s`
pub fn link_speed(gts: f32) -> String {
    format!("{gts:.1} GT/s")
}

/// `42.0 clocks`
pub fn latency_clocks(value: f32) -> String {
    format!("{value:.1} clocks")
}

/// `84 clocks`
pub fn clocks(value: u32) -> String {
    format!("{value} clocks")
}

/// `39 / 25`
pub fn week_year(week: u8, year: u8) -> String {
    format!("{week:02} / {year:02}")
}

pub fn yes_no(value: bool) -> &'static str {
    if value {
        "Yes"
    } else {
        "No"
    }
}

/// A profile voltage: `1.10 V`, or `1.375 V` when the value needs the third decimal.
pub fn profile_voltage(value: f32) -> String {
    let millivolts = (f64::from(value) * 1000.0).round() as i64;
    if millivolts % 10 == 0 {
        format!("{:.2} V", millivolts as f64 / 1000.0)
    } else {
        format!("{:.3} V", millivolts as f64 / 1000.0)
    }
}

/// Graphics memory: `512 MBytes`, `8 GBytes`, `11.25 GBytes`.
pub fn graphics_memory(megabytes: u64) -> String {
    if megabytes < 1024 {
        return format!("{megabytes} MBytes");
    }
    let hundredths = (megabytes as f64 / 1024.0 * 100.0).round() as u64;
    if hundredths % 100 == 0 {
        format!("{} GBytes", hundredths / 100)
    } else {
        format!("{}.{:02} GBytes", hundredths / 100, hundredths % 100)
    }
}

/// `128 bits`
pub fn bits(value: u32) -> String {
    format!("{value} bits")
}

/// `48 / 96`, `48 / -` when only the first is known, empty otherwise.
pub fn rops_tmus(rops: Option<u32>, tmus: Option<u32>) -> String {
    match (rops, tmus) {
        (Some(rops), Some(tmus)) => format!("{rops} / {tmus}"),
        (Some(rops), None) => format!("{rops} / -"),
        _ => String::new(),
    }
}

/// Benchmark score with one decimal.
pub fn score(value: f64) -> String {
    format!("{value:.1}")
}

/// Multi-thread ratio with two decimals.
pub fn ratio(value: f64) -> String {
    format!("{value:.2}")
}

/// `37 %` for a progress fraction in `0.0..=1.0`.
pub fn percent(fraction: f32) -> String {
    format!("{} %", (fraction.clamp(0.0, 1.0) * 100.0).round() as u32)
}

/// `12.34 sec`
pub fn seconds(value: f64) -> String {
    format!("{value:.2} sec")
}

/// Drift between two time sources: `+0.42 ms`, `-1.07 ms`, `0.00 ms`.
pub fn drift_ms(seconds: f64) -> String {
    let ms = seconds * 1000.0;
    if ms.abs() < 0.005 {
        "0.00 ms".to_string()
    } else {
        format!("{ms:+.2} ms")
    }
}

/// `1 thread`, `16 threads`
pub fn thread_count(threads: u32) -> String {
    if threads == 1 {
        "1 thread".to_string()
    } else {
        format!("{threads} threads")
    }
}

/// `Core #3`, `P-Core #0`
pub fn core_label(kind: CoreKind, core: u32) -> String {
    format!("{} #{core}", kind.label())
}

/// The Cores box: a plain count, or `8P + 16E` on hybrid processors. Three kinds
/// of core only fit the box without the spaces: `6P+8E+2LP`.
pub fn core_count(sets: &[CoreSet], cores: u32) -> String {
    if sets.len() < 2 {
        return if cores == 0 { String::new() } else { cores.to_string() };
    }
    let suffix = |kind: CoreKind| match kind {
        CoreKind::Uniform => "",
        CoreKind::Performance => "P",
        CoreKind::Efficient => "E",
        CoreKind::LowPower => "LP",
    };
    let separator = if sets.len() > 2 { "+" } else { " + " };
    sets.iter().map(|set| format!("{}{}", set.cores, suffix(set.kind))).collect::<Vec<_>>().join(separator)
}

/// `Ver. 0.1.0.x64`, the label in the bottom-left corner of the frame.
pub fn short_version(version: &str) -> String {
    format!("Ver. {version}.{ARCH}")
}

/// `Version 0.1.0 (x64)`
pub fn long_version(version: &str) -> String {
    format!("Version {version} ({ARCH})")
}

/// `Version 0.1.0.x64`, the line on the splash.
pub fn splash_version(version: &str) -> String {
    format!("Version {version}.{ARCH}")
}

/// `text` without the parenthesised lists that detail some of its entries:
/// `AVX2, AVX512 (DQ, BW, VL), FMA3` becomes `AVX2, AVX512, FMA3`. A qualifier
/// that is not a list, as in `MMX (+)`, stays.
pub fn without_detail_lists(text: &str) -> String {
    let mut brief = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find(" (") {
        let Some(close) = rest[open..].find(')').map(|offset| open + offset) else { break };
        let group = &rest[open..=close];
        brief.push_str(&rest[..open]);
        if !group.contains(',') {
            brief.push_str(group);
        }
        rest = &rest[close + 1..];
    }
    brief.push_str(rest);
    brief
}

/// Splits `text` into at most two lines no wider than `max_width`, breaking at spaces.
///
/// `measure` returns the width of a string in the units of `max_width`. Whatever
/// does not fit the first line goes to the second in full; the caller clips it.
pub fn wrap_two_lines(text: &str, max_width: i32, measure: impl Fn(&str) -> i32) -> (String, String) {
    if measure(text) <= max_width {
        return (text.to_string(), String::new());
    }
    let mut split = None;
    for (index, _) in text.match_indices(' ') {
        if measure(&text[..index]) > max_width {
            break;
        }
        split = Some(index);
    }
    match split {
        Some(index) => (text[..index].to_string(), text[index + 1..].to_string()),
        None => (text.to_string(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processor_values() {
        assert_eq!(watts(65.0), "65.0 W");
        assert_eq!(nanometers(5), "5 nm");
        assert_eq!(core_voltage(1.016), "1.016 V");
        assert_eq!(hex(0x19), "19");
        assert_eq!(hex(0xF), "F");
        assert_eq!(hex(0x61), "61");
        assert_eq!(microcode(0x0A60_1203), "0xA601203");
    }

    #[test]
    fn frequencies() {
        assert_eq!(mhz2(5136.628), "5136.63 MHz");
        assert_eq!(mhz1(2600.14), "2600.1 MHz");
        assert_eq!(mhz0(2633.4), "2633 MHz");
    }

    #[test]
    fn multipliers() {
        assert_eq!(multiplier(51.5, Some(4.0), Some(53.5)), "x 51.5 (4.0 - 53.5)");
        assert_eq!(multiplier(51.5, None, Some(53.5)), "x 51.5 (max 53.5)");
        assert_eq!(multiplier(41.6, None, Some(47.25)), "x 41.6 (max 47.25)");
        // The current value is a measurement and is shown as such, even above the stated maximum.
        assert_eq!(multiplier(53.7, None, Some(53.5)), "x 53.7 (max 53.5)");
        assert_eq!(multiplier(51.5, Some(4.0), None), "x 51.5");
        assert_eq!(multiplier(36.0, None, None), "x 36.0");
        assert_eq!(multiplier(53.25, Some(4.0), Some(53.5)), "x 53.25 (4.0 - 53.5)");
        assert_eq!(multiplier2(51.5), "x 51.50");
    }

    #[test]
    fn links_and_timings() {
        assert_eq!(link_width(8), "x8");
        assert_eq!(link_speed(16.0), "16.0 GT/s");
        assert_eq!(link_speed(2.5), "2.5 GT/s");
        assert_eq!(latency_clocks(42.0), "42.0 clocks");
        assert_eq!(clocks(84), "84 clocks");
        assert_eq!(week_year(9, 5), "09 / 05");
        assert_eq!(yes_no(true), "Yes");
        assert_eq!(yes_no(false), "No");
    }

    #[test]
    fn profile_voltages() {
        assert_eq!(profile_voltage(1.1), "1.10 V");
        assert_eq!(profile_voltage(1.35), "1.35 V");
        assert_eq!(profile_voltage(1.375), "1.375 V");
        assert_eq!(profile_voltage(1.2), "1.20 V");
    }

    #[test]
    fn graphics_values() {
        assert_eq!(graphics_memory(512), "512 MBytes");
        assert_eq!(graphics_memory(8192), "8 GBytes");
        assert_eq!(graphics_memory(8188), "8 GBytes");
        assert_eq!(graphics_memory(11_520), "11.25 GBytes");
        assert_eq!(graphics_memory(1536), "1.50 GBytes");
        assert_eq!(bits(128), "128 bits");
        assert_eq!(rops_tmus(Some(48), Some(96)), "48 / 96");
        assert_eq!(rops_tmus(Some(48), None), "48 / -");
        assert_eq!(rops_tmus(None, Some(96)), "");
    }

    #[test]
    fn bench_and_timer_values() {
        assert_eq!(score(712.34), "712.3");
        assert_eq!(ratio(9.876), "9.88");
        assert_eq!(percent(0.374), "37 %");
        assert_eq!(percent(7.0), "100 %");
        assert_eq!(percent(-1.0), "0 %");
        assert_eq!(seconds(12.345), "12.35 sec");
        assert_eq!(drift_ms(0.0), "0.00 ms");
        assert_eq!(drift_ms(0.00042), "+0.42 ms");
        assert_eq!(drift_ms(-0.00107), "-1.07 ms");
        assert_eq!(thread_count(1), "1 thread");
        assert_eq!(thread_count(16), "16 threads");
    }

    #[test]
    fn core_labels_and_counts() {
        assert_eq!(core_label(CoreKind::Uniform, 3), "Core #3");
        assert_eq!(core_label(CoreKind::Performance, 0), "P-Core #0");
        let uniform = [CoreSet { kind: CoreKind::Uniform, cores: 8, threads: 16 }];
        assert_eq!(core_count(&uniform, 8), "8");
        assert_eq!(core_count(&[], 0), "");
        let hybrid = [
            CoreSet { kind: CoreKind::Performance, cores: 8, threads: 16 },
            CoreSet { kind: CoreKind::Efficient, cores: 16, threads: 16 },
        ];
        assert_eq!(core_count(&hybrid, 24), "8P + 16E");
        let three_kinds = [
            CoreSet { kind: CoreKind::Performance, cores: 6, threads: 12 },
            CoreSet { kind: CoreKind::Efficient, cores: 8, threads: 8 },
            CoreSet { kind: CoreKind::LowPower, cores: 2, threads: 2 },
        ];
        assert_eq!(core_count(&three_kinds, 16), "6P+8E+2LP");
    }

    #[test]
    fn versions() {
        assert_eq!(short_version("1.2.3"), format!("Ver. 1.2.3.{ARCH}"));
        assert_eq!(long_version("1.2.3"), format!("Version 1.2.3 ({ARCH})"));
        assert_eq!(splash_version("1.2.3"), format!("Version 1.2.3.{ARCH}"));
    }

    #[test]
    fn detail_lists_are_dropped() {
        assert_eq!(
            without_detail_lists("MMX (+), AVX2, AVX512 (DQ, BW, VL), FMA3"),
            "MMX (+), AVX2, AVX512, FMA3"
        );
        assert_eq!(without_detail_lists("AVX512 (DQ, BW)"), "AVX512");
        assert_eq!(without_detail_lists("MMX, SSE"), "MMX, SSE");
        assert_eq!(without_detail_lists("AVX512 (DQ, BW"), "AVX512 (DQ, BW");
        assert_eq!(without_detail_lists(""), "");
    }

    #[test]
    fn wrapping() {
        let width = |s: &str| s.len() as i32;
        assert_eq!(wrap_two_lines("MMX, SSE", 20, width), ("MMX, SSE".to_string(), String::new()));
        assert_eq!(
            wrap_two_lines("MMX, SSE, SSE2, AVX", 10, width),
            ("MMX, SSE,".to_string(), "SSE2, AVX".to_string())
        );
        assert_eq!(
            wrap_two_lines("MMX, SSE, SSE2, AVX, AVX2", 9, width),
            ("MMX, SSE,".to_string(), "SSE2, AVX, AVX2".to_string())
        );
        assert_eq!(wrap_two_lines("Unbreakable", 4, width), ("Unbreakable".to_string(), String::new()));
        assert_eq!(wrap_two_lines("", 4, width), (String::new(), String::new()));
    }
}
