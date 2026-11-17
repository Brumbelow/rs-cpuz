//! Marketing names: shortening the CPUID brand string to the name shown on the
//! CPU tab, and reading what the model number says about the part.

use crate::model::CpuVendor;

/// Trademark marks that brand strings sprinkle between words.
const MARKS: [&str; 3] = ["(R)", "(TM)", "(C)"];
/// A brand string is only shortened when it names one of these.
const KNOWN_MAKERS: [&str; 7] = ["Intel", "AMD", "Hygon", "ZHAOXIN", "VIA", "Pentium", "Celeron"];
/// Words that end the name proper: the frequency, the graphics part, remarks in brackets.
/// Brand strings spell them in either case (`Radeon R7`, `RADEON R7`).
const TAIL_WORDS: [&str; 4] = ["@", "with", "w/", "Radeon"];
/// Spelled-out core counts, as in `Quad Core Processor`.
const COUNT_WORDS: [&str; 7] = ["dual", "triple", "quad", "six", "eight", "twelve", "sixteen"];

/// The short marketing name for a brand string:
/// `Intel(R) Core(TM) i7-13700K CPU @ 3.40GHz` becomes `Intel Core i7 13700K`,
/// `AMD Ryzen 7 7700 8-Core Processor` becomes `AMD Ryzen 7 7700`.
/// Brand strings of unknown makers only lose their trademark marks.
pub fn short_name(vendor: CpuVendor, brand: &str) -> String {
    let plain = strip_marks(brand);
    if !plain.split_whitespace().any(|w| KNOWN_MAKERS.contains(&w)) {
        return plain.split_whitespace().collect::<Vec<_>>().join(" ");
    }

    let spaced = plain.replace('@', " @ ");
    let mut words: Vec<&str> = spaced.split_whitespace().collect();
    if words.len() > 2 && is_ordinal(words[0]) && words[1] == "Gen" {
        words.drain(..2);
    }
    // A word closed by a comma is the graphics model of an APU (`R7, 10 COMPUTE CORES 4C+6G`).
    let ends_name = |w: &&str| {
        TAIL_WORDS.iter().any(|tail| w.eq_ignore_ascii_case(tail)) || w.starts_with('(') || w.ends_with(',')
    };
    if let Some(tail) = words.iter().skip(1).position(ends_name) {
        words.truncate(tail + 1);
        if words.last() == Some(&"APU") {
            words.pop();
        }
    }

    let intel = vendor == CpuVendor::Intel || words.contains(&"Intel");
    let mut name: Vec<&str> = Vec::with_capacity(words.len() + 1);
    let mut rest = words.as_slice();
    while let [word, following @ ..] = rest {
        rest = following;
        let next = following.first().copied().unwrap_or("");
        if is_filler(word) || (*word == "Genuine" && next == "Intel") {
            continue;
        }
        if COUNT_WORDS.contains(&word.to_ascii_lowercase().as_str()) && next.eq_ignore_ascii_case("core") {
            rest = &following[1..];
            continue;
        }
        match split_series(word).filter(|_| intel) {
            Some((series, number)) => name.extend([series, number]),
            None => name.push(word),
        }
    }
    // Early Xeon E5 brand strings carry a placeholder "0" where later ones say "v2".
    if name.len() > 2 && name.last() == Some(&"0") {
        name.pop();
    }
    if intel && !name.contains(&"Intel") {
        name.insert(0, "Intel");
    }
    name.join(" ")
}

/// The frequency advertised at the end of a brand string (`... @ 3.40GHz`), in MHz.
pub fn brand_mhz(brand: &str) -> Option<f64> {
    let tail = brand.rsplit_once('@')?.1.trim();
    let number_len = tail.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(tail.len());
    let (number, unit) = tail.split_at(number_len);
    let value: f64 = number.parse().ok()?;
    let scale = match unit.trim_start().get(..3)?.to_ascii_lowercase().as_str() {
        "ghz" => 1000.0,
        "mhz" => 1.0,
        _ => return None,
    };
    (value > 0.0).then(|| (value * scale).round())
}

/// The model number at the end of a short name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelNumber<'a> {
    /// The leading digit run: `13700` for `13700K`, `7950` for `7950X3D`.
    pub digits: &'a str,
    /// What follows the digits: `K`, `X3D`, `G7`, or empty. Parts that spell the
    /// suffix as a separate word (`HX 370`, `M 520`) report that word.
    pub suffix: &'a str,
}

/// Finds the model number of a short name, skipping a trailing revision such as `v4`.
/// `None` when the name does not end in something with digits.
pub fn model_number(name: &str) -> Option<ModelNumber<'_>> {
    let mut words: Vec<&str> = name.split_whitespace().collect();
    if words.len() > 1 && words.last().is_some_and(|w| is_revision(w)) {
        words.pop();
    }
    let word = words.pop()?;
    let word = word.rsplit('-').next().unwrap_or(word);
    let numbered = &word[word.find(|c: char| c.is_ascii_digit())?..];
    let (digits, suffix) =
        numbered.split_at(numbered.find(|c: char| !c.is_ascii_digit()).unwrap_or(numbered.len()));
    let detached = words.last().filter(|w| w.len() <= 2 && w.bytes().all(|b| b.is_ascii_uppercase()));
    Some(ModelNumber {
        digits,
        suffix: if suffix.is_empty() { detached.copied().unwrap_or("") } else { suffix },
    })
}

/// How a part is mounted, which decides the package of dies sold both ways.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Segment {
    /// Socketed desktop, workstation or server part.
    Desktop,
    /// Soldered mobile part.
    Mobile,
    Unknown,
}

const INTEL_MOBILE: [&str; 20] = [
    "U", "UL", "UE", "Y", "H", "HK", "HX", "HS", "HF", "HQ", "HL", "M", "QM", "MQ", "MX", "XM", "LM", "UM",
    "V", "Q",
];
const INTEL_DESKTOP: [&str; 10] = ["", "K", "KF", "KS", "F", "T", "S", "X", "XE", "C"];
/// Intel product lines whose plain model numbers are socketed parts.
const INTEL_SOCKETED_LINES: [&str; 10] = [
    "Core i",
    "Core Ultra",
    "Core 3 ",
    "Core 5 ",
    "Core 7 ",
    "Core 9 ",
    "Xeon",
    "Pentium G",
    "Pentium Gold G",
    "Celeron G",
];
const AMD_MOBILE: [&str; 9] = ["U", "H", "HS", "HX", "HX3D", "C", "M", "MX", "P"];
const AMD_DESKTOP: [&str; 11] = ["", "X", "XT", "G", "GE", "GT", "F", "X3D", "E", "K", "T"];
/// Product lines that only exist as socketed server or workstation parts.
const SOCKETED_LINES: [&str; 3] = ["EPYC", "Threadripper", "Opteron"];

/// Classifies a short name by its model number. Anything not clearly one or the other is `Unknown`.
pub fn segment(vendor: CpuVendor, name: &str) -> Segment {
    if name.contains("Embedded") {
        return Segment::Unknown;
    }
    if SOCKETED_LINES.iter().any(|line| name.contains(line)) {
        return Segment::Desktop;
    }
    let Some(model) = model_number(name) else { return Segment::Unknown };
    match vendor {
        CpuVendor::Intel => intel_segment(name, model),
        CpuVendor::Amd => amd_segment(name, model),
        _ => Segment::Unknown,
    }
}

fn intel_segment(name: &str, model: ModelNumber) -> Segment {
    let suffix = model.suffix;
    // Xeon suffixes describe the server role (`8468H`, `6980P`); only `M` marks a mobile one.
    let xeon = name.contains("Xeon");
    // `1165G7` and `7Y30`: a graphics level or the Y-series marker followed by more digits.
    let numbered = |lead: char| {
        suffix.strip_prefix(lead).is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
    };
    let mobile = if xeon {
        suffix == "M"
    } else if suffix == "P" {
        // A mobile line in the 12th and 13th generation only (`1260P`, `1340P`);
        // the Core parts that carried the letter before are socketed (`3350P`, `6402P`).
        model.digits.len() == 4 && (model.digits.starts_with("12") || model.digits.starts_with("13"))
    } else {
        INTEL_MOBILE.contains(&suffix) || numbered('G') || numbered('Y')
    };
    if mobile {
        return Segment::Mobile;
    }
    let socketed_line = INTEL_SOCKETED_LINES.iter().any(|line| name.contains(line));
    let socketed_suffix = INTEL_DESKTOP.contains(&suffix)
        || (suffix == "G" && xeon)
        || (suffix == "P" && name.contains("Core i"));
    if socketed_line && socketed_suffix {
        Segment::Desktop
    } else {
        Segment::Unknown
    }
}

fn amd_segment(name: &str, model: ModelNumber) -> Segment {
    // Ryzen went to three-digit model numbers for mobile parts only.
    if AMD_MOBILE.contains(&model.suffix) || (name.contains("Ryzen") && model.digits.len() == 3) {
        Segment::Mobile
    } else if AMD_DESKTOP.contains(&model.suffix) {
        Segment::Desktop
    } else {
        Segment::Unknown
    }
}

fn strip_marks(brand: &str) -> String {
    let mut out = String::with_capacity(brand.len());
    let mut rest = brand;
    while let Some(open) = rest.find('(') {
        let (head, tail) = rest.split_at(open);
        out.push_str(head);
        let mark = MARKS.iter().find(|m| tail.get(..m.len()).is_some_and(|t| t.eq_ignore_ascii_case(m)));
        match mark {
            Some(mark) => rest = &tail[mark.len()..],
            None => {
                out.push('(');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `13th`, `2nd`, ... as in `13th Gen Intel(R) Core(TM)`.
fn is_ordinal(word: &str) -> bool {
    let digits = word.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && matches!(&word[digits.len()..], "st" | "nd" | "rd" | "th")
}

/// `v2` ... `v6`, the generation mark after a Xeon model number.
fn is_revision(word: &str) -> bool {
    word.strip_prefix('v').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Words that say nothing about the model: `CPU`, `Processor`, `8-Core`, `64-Cores`.
fn is_filler(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    matches!(lower.as_str(), "cpu" | "processor") || lower.ends_with("-core") || lower.ends_with("-cores")
}

/// Splits the words Intel glues together: `i7-13700K` into `i7` + `13700K`, `Core2` into `Core` + `2`.
fn split_series(word: &str) -> Option<(&str, &str)> {
    if word == "Core2" {
        return Some(word.split_at(4));
    }
    match word.as_bytes() {
        [letter, digit, b'-', _, ..] if letter.is_ascii_alphabetic() && digit.is_ascii_digit() => {
            Some((&word[..2], &word[3..]))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CpuVendor::{Amd, Intel};

    #[test]
    fn shortens_intel_brand_strings() {
        let cases = [
            ("Intel(R) Core(TM) i7-13700K CPU @ 3.40GHz", "Intel Core i7 13700K"),
            ("13th Gen Intel(R) Core(TM) i5-13600K", "Intel Core i5 13600K"),
            ("12th Gen Intel(R) Core(TM) i7-12700H", "Intel Core i7 12700H"),
            ("11th Gen Intel(R) Core(TM) i7-1165G7 @ 2.80GHz", "Intel Core i7 1165G7"),
            ("Intel(R) Core(TM) Ultra 7 155H", "Intel Core Ultra 7 155H"),
            ("Intel(R) Core(TM) Ultra 9 285K", "Intel Core Ultra 9 285K"),
            ("Intel(R) Core(TM) 7 150U", "Intel Core 7 150U"),
            ("Intel(R) Core(TM) i9-14900K", "Intel Core i9 14900K"),
            ("Intel(R) Core(TM) i9-10980XE CPU @ 3.00GHz", "Intel Core i9 10980XE"),
            ("Intel(R) Core(TM) i7-4790K CPU @ 4.00GHz", "Intel Core i7 4790K"),
            ("Intel(R) Core(TM) i5-2500K CPU @ 3.30GHz", "Intel Core i5 2500K"),
            ("Intel(R) Core(TM) i7 CPU         920  @ 2.67GHz", "Intel Core i7 920"),
            ("Intel(R) Core(TM) i5 CPU       M 520  @ 2.40GHz", "Intel Core i5 M 520"),
            ("Intel(R) Core(TM) i7 CPU       X 920  @ 2.00GHz", "Intel Core i7 X 920"),
            ("Intel(R) Core(TM) i7 CPU       X 940  @ 2.13GHz", "Intel Core i7 X 940"),
            ("Intel(R) Core(TM) i5-6402P CPU @ 2.80GHz", "Intel Core i5 6402P"),
            ("Intel(R) Core(TM) i3-6098P CPU @ 3.60GHz", "Intel Core i3 6098P"),
            ("Intel(R) Core(TM) m3-7Y30 CPU @ 1.00GHz", "Intel Core m3 7Y30"),
            ("Intel(R) Core(TM) i3-N305", "Intel Core i3 N305"),
            ("Intel(R) Core(TM)2 Duo CPU     E8500  @ 3.16GHz", "Intel Core 2 Duo E8500"),
            ("Intel(R) Core(TM)2 Quad CPU    Q6600  @ 2.40GHz", "Intel Core 2 Quad Q6600"),
            // The first Core 2 processors name neither the line nor the letter of the model.
            ("Intel(R) Core(TM)2 CPU          6600  @ 2.40GHz", "Intel Core 2 6600"),
            ("Intel(R) Core(TM)2 CPU          6300  @ 1.86GHz", "Intel Core 2 6300"),
            ("Intel(R) Core(TM)2 CPU          4300  @ 1.80GHz", "Intel Core 2 4300"),
            ("Intel(R) Core(TM)2 CPU         T7200  @ 2.00GHz", "Intel Core 2 T7200"),
            ("Intel(R) Pentium(R) CPU G4560 @ 3.50GHz", "Intel Pentium G4560"),
            ("Intel(R) Pentium(R) Gold G5400 CPU @ 3.70GHz", "Intel Pentium Gold G5400"),
            ("Pentium(R) Dual-Core  CPU      E5200  @ 2.50GHz", "Intel Pentium E5200"),
            ("Intel(R) Celeron(R) N5105 @ 2.00GHz", "Intel Celeron N5105"),
            ("Intel(R) Atom(TM) x5-Z8350  CPU @ 1.44GHz", "Intel Atom x5 Z8350"),
            ("Intel(R) N100", "Intel N100"),
            ("Intel(R) Xeon(R) CPU E5-2680 v4 @ 2.40GHz", "Intel Xeon E5 2680 v4"),
            ("Intel(R) Xeon(R) CPU E5-2670 0 @ 2.60GHz", "Intel Xeon E5 2670"),
            ("Intel(R) Xeon(R) CPU           X5650  @ 2.67GHz", "Intel Xeon X5650"),
            ("Intel(R) Xeon(R) Platinum 8380 CPU @ 2.30GHz", "Intel Xeon Platinum 8380"),
            ("Intel(R) Xeon(R) Gold 6248R CPU @ 3.00GHz", "Intel Xeon Gold 6248R"),
            ("Intel(R) Xeon(R) W-2245 CPU @ 3.90GHz", "Intel Xeon W-2245"),
            ("Intel(R) Xeon(R) w9-3495X", "Intel Xeon w9 3495X"),
            ("Intel(R) Xeon(R) E-2288G CPU @ 3.70GHz", "Intel Xeon E-2288G"),
            ("Genuine Intel(R) CPU 0000 @ 2.40GHz", "Intel 0000"),
        ];
        for (brand, expected) in cases {
            assert_eq!(short_name(Intel, brand), expected, "{brand}");
        }
    }

    #[test]
    fn shortens_amd_brand_strings() {
        let cases = [
            ("AMD Ryzen 7 7700 8-Core Processor", "AMD Ryzen 7 7700"),
            ("AMD Ryzen 9 7950X3D 16-Core Processor", "AMD Ryzen 9 7950X3D"),
            ("AMD Ryzen 9 9950X 16-Core Processor", "AMD Ryzen 9 9950X"),
            ("AMD Ryzen 5 5600X 6-Core Processor", "AMD Ryzen 5 5600X"),
            ("AMD Ryzen 7 1700 Eight-Core Processor", "AMD Ryzen 7 1700"),
            ("AMD Ryzen 5 1600 Six-Core Processor", "AMD Ryzen 5 1600"),
            ("AMD Ryzen 5 3400G with Radeon Vega Graphics", "AMD Ryzen 5 3400G"),
            ("AMD Ryzen 7 5800H with Radeon Graphics", "AMD Ryzen 7 5800H"),
            ("AMD Ryzen 7 PRO 4750U with Radeon Graphics", "AMD Ryzen 7 PRO 4750U"),
            ("AMD Ryzen 7 7840HS w/ Radeon 780M Graphics", "AMD Ryzen 7 7840HS"),
            ("AMD Ryzen AI 9 HX 370 w/ Radeon 890M", "AMD Ryzen AI 9 HX 370"),
            ("AMD Ryzen Threadripper 3970X 32-Core Processor", "AMD Ryzen Threadripper 3970X"),
            ("AMD Ryzen Threadripper PRO 5995WX 64-Cores", "AMD Ryzen Threadripper PRO 5995WX"),
            ("AMD EPYC 7742 64-Core Processor", "AMD EPYC 7742"),
            ("AMD FX(tm)-8350 Eight-Core Processor", "AMD FX-8350"),
            ("AMD A10-7850K Radeon R7, 12 Compute Cores 4C+8G", "AMD A10-7850K"),
            ("AMD A10-9700 RADEON R7, 10 COMPUTE CORES 4C+6G", "AMD A10-9700"),
            ("AMD A12-9720P RADEON R7, 12 COMPUTE CORES 4C+8G", "AMD A12-9720P"),
            ("AMD A9-9420 RADEON R5, 5 COMPUTE CORES 2C+3G", "AMD A9-9420"),
            ("AMD PRO A10-8770 R7, 10 COMPUTE CORES 4C+6G", "AMD PRO A10-8770"),
            ("AMD A8-3850 APU with Radeon(tm) HD Graphics", "AMD A8-3850"),
            ("AMD A8-5600K APU with Radeon(tm) HD Graphics", "AMD A8-5600K"),
            ("AMD Phenom(tm) II X4 965 Processor", "AMD Phenom II X4 965"),
            ("AMD Phenom(tm) 9950 Quad-Core Processor", "AMD Phenom 9950"),
            ("AMD Athlon(tm) II X2 250 Processor", "AMD Athlon II X2 250"),
            ("AMD Athlon(tm) 64 X2 Dual Core Processor 4200+", "AMD Athlon 64 X2 4200+"),
            ("AMD Athlon(tm) X4 860K Quad Core Processor", "AMD Athlon X4 860K"),
            ("AMD Athlon 3000G with Radeon Vega 3 Graphics", "AMD Athlon 3000G"),
            ("AMD Opteron(tm) Processor 6174", "AMD Opteron 6174"),
            ("Quad-Core AMD Opteron(tm) Processor 8356", "AMD Opteron 8356"),
            ("AMD E-350 Processor", "AMD E-350"),
            ("AMD Custom APU 0405", "AMD Custom APU 0405"),
            ("AMD Eng Sample: 100-000000514-13_Y", "AMD Eng Sample: 100-000000514-13_Y"),
        ];
        for (brand, expected) in cases {
            assert_eq!(short_name(Amd, brand), expected, "{brand}");
        }
    }

    #[test]
    fn shortens_other_makers_conservatively() {
        assert_eq!(short_name(CpuVendor::Hygon, "Hygon C86 7185 32-core Processor"), "Hygon C86 7185");
        assert_eq!(
            short_name(CpuVendor::Zhaoxin, "ZHAOXIN KaiXian KX-U6780A@2.7GHz"),
            "ZHAOXIN KaiXian KX-U6780A"
        );
        assert_eq!(short_name(CpuVendor::Via, "VIA Nano processor U2250 (1.6GHz Capable)"), "VIA Nano U2250");
        // Unknown makers keep every word, including the ones that look like filler.
        assert_eq!(short_name(Amd, "QEMU  Virtual CPU version 2.5+"), "QEMU Virtual CPU version 2.5+");
        assert_eq!(short_name(CpuVendor::Other, "Virtual CPU @ 3.41GHz"), "Virtual CPU @ 3.41GHz");
        assert_eq!(short_name(Intel, ""), "");
    }

    #[test]
    fn reads_brand_frequency() {
        assert_eq!(brand_mhz("Intel(R) Core(TM) i7-13700K CPU @ 3.40GHz"), Some(3400.0));
        assert_eq!(brand_mhz("Intel(R) Core(TM) i7 CPU 920 @ 2.67GHz"), Some(2670.0));
        assert_eq!(brand_mhz("Intel(R) Core(TM)2 Duo CPU E8500 @ 3.16GHz"), Some(3160.0));
        assert_eq!(brand_mhz("AMD FX(tm)-8350 Processor @ 4.00 GHz"), Some(4000.0));
        assert_eq!(brand_mhz("Intel(R) Pentium(R) III CPU @ 800MHz"), Some(800.0));
        assert_eq!(brand_mhz("ZHAOXIN KaiXian KX-U6780A@2.7GHz"), Some(2700.0));
        assert_eq!(brand_mhz("AMD Ryzen 7 7700 8-Core Processor"), None);
        assert_eq!(brand_mhz("Genuine Intel(R) CPU @ GHz"), None);
        assert_eq!(brand_mhz("Odd CPU @ 0.00GHz"), None);
        assert_eq!(brand_mhz("Odd CPU @ 3.4 parsecs"), None);
        assert_eq!(brand_mhz("Odd CPU @ 3.4"), None);
        assert_eq!(brand_mhz("Intel(R) Xeon(R) CPU @ 2.20GHz (ES)"), Some(2200.0));
    }

    #[test]
    fn reads_model_numbers() {
        let parse = |name| model_number(name).map(|m| (m.digits, m.suffix));
        assert_eq!(parse("Intel Core i7 13700K"), Some(("13700", "K")));
        assert_eq!(parse("AMD Ryzen 9 7950X3D"), Some(("7950", "X3D")));
        assert_eq!(parse("AMD Ryzen 7 7700"), Some(("7700", "")));
        assert_eq!(parse("Intel Core i7 1165G7"), Some(("1165", "G7")));
        assert_eq!(parse("Intel Core m3 7Y30"), Some(("7", "Y30")));
        assert_eq!(parse("Intel Xeon E5 2680 v4"), Some(("2680", "")));
        assert_eq!(parse("Intel Core 2 Duo E8500"), Some(("8500", "")));
        assert_eq!(parse("AMD A10-7850K"), Some(("7850", "K")));
        assert_eq!(parse("AMD FX-8350"), Some(("8350", "")));
        assert_eq!(parse("AMD Ryzen AI 9 HX 370"), Some(("370", "HX")));
        assert_eq!(parse("Intel Core i5 M 520"), Some(("520", "M")));
        assert_eq!(parse("AMD Phenom II X4 965"), Some(("965", "")));
        assert_eq!(parse("AMD Ryzen Z1 Extreme"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn classifies_segments() {
        use Segment::*;
        let intel = [
            ("Intel Core i7 13700K", Desktop),
            ("Intel Core i5 12400F", Desktop),
            ("Intel Core i7 7700", Desktop),
            ("Intel Core i7 5775C", Desktop),
            ("Intel Core i5 2380P", Desktop),
            ("Intel Core i5 3350P", Desktop),
            ("Intel Core i5 6402P", Desktop),
            ("Intel Core i3 6098P", Desktop),
            ("Intel Core Ultra 9 285K", Desktop),
            ("Intel Core 5 120", Desktop),
            ("Intel Xeon E3 1230 v3", Desktop),
            ("Intel Xeon E-2288G", Desktop),
            ("Intel Pentium G4560", Desktop),
            ("Intel Core i9 13980HX", Mobile),
            ("Intel Core i7 12700H", Mobile),
            ("Intel Core i7 1260P", Mobile),
            ("Intel Core i5 1340P", Mobile),
            ("Intel Core i5 8250U", Mobile),
            ("Intel Core i7 1165G7", Mobile),
            ("Intel Core m3 7Y30", Mobile),
            ("Intel Core i7 4700MQ", Mobile),
            ("Intel Core Ultra 7 155H", Mobile),
            ("Intel Core Ultra 7 258V", Mobile),
            ("Intel Core i5 M 520", Mobile),
            ("Intel Xeon E3 1505M v5", Mobile),
            ("Intel Xeon Platinum 8468H", Unknown),
            ("Intel Xeon 6980P", Unknown),
            ("Intel Xeon E3 1265L v2", Unknown),
            ("Intel Core i7 8809G", Unknown),
            ("Intel Core i7 5775R", Unknown),
            ("Intel Pentium 987", Unknown),
            ("Intel Celeron N5105", Unknown),
        ];
        for (name, expected) in intel {
            assert_eq!(segment(Intel, name), expected, "{name}");
        }
        let amd = [
            ("AMD Ryzen 7 7700", Desktop),
            ("AMD Ryzen 9 7950X3D", Desktop),
            ("AMD Ryzen 5 5600G", Desktop),
            ("AMD Ryzen 5 PRO 4650GE", Desktop),
            ("AMD Athlon 200GE", Desktop),
            ("AMD A10-7850K", Desktop),
            ("AMD EPYC 4564P", Desktop),
            ("AMD Ryzen Threadripper PRO 5995WX", Desktop),
            ("AMD Ryzen 9 7945HX", Mobile),
            ("AMD Ryzen 9 7945HX3D", Mobile),
            ("AMD Ryzen 7 5800H", Mobile),
            ("AMD Ryzen 7 7840HS", Mobile),
            ("AMD Ryzen 5 5500U", Mobile),
            ("AMD Ryzen AI 9 HX 370", Mobile),
            ("AMD Ryzen 7 260", Mobile),
            ("AMD A12-9700P", Mobile),
            ("AMD Ryzen Embedded V1605B", Unknown),
            ("AMD Ryzen Z1 Extreme", Unknown),
            ("AMD Athlon 64 X2 4200+", Unknown),
        ];
        for (name, expected) in amd {
            assert_eq!(segment(Amd, name), expected, "{name}");
        }
        assert_eq!(segment(CpuVendor::Other, "Virtual CPU 7700"), Unknown);
    }
}
