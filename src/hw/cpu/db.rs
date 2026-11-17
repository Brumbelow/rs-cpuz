//! What CPUID cannot say: code names, process nodes, sockets, stepping names
//! and rated power, looked up by vendor, family, model and marketing name.
//!
//! The tables are compiled from vendor documentation (processor specification
//! updates, revision guides and product pages). They are deliberately sparse:
//! a combination that is not listed, or a column that depends on something the
//! name does not reveal, stays unknown.

use super::name::{model_number, segment, Segment};
use crate::model::CpuVendor;
use Pkg::{Any, Split, Unknown};
use When::{Always, Name, Suffix};

/// What the tables know about one processor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Known {
    pub codename: &'static str,
    pub technology_nm: Option<u32>,
    pub package: Option<&'static str>,
}

/// A condition on the short marketing name.
#[derive(Clone, Copy)]
enum When {
    Always,
    /// The name contains one of these fragments.
    Name(&'static [&'static str]),
    /// The model number marks a soldered mobile part.
    Mobile,
    /// The model number marks a socketed part.
    Socketed,
    /// The model number ends in one of these suffixes.
    Suffix(&'static [&'static str]),
}

/// The package of a die, which may ship both socketed and soldered.
#[derive(Clone, Copy)]
enum Pkg {
    Unknown,
    Any(&'static str),
    /// Known for the socketed variant only.
    Socketed(&'static str),
    /// Known for the mobile variant only.
    Mobile(&'static str),
    /// Socketed variant, mobile variant.
    Split(&'static str, &'static str),
}

/// One row: display family and model range, optional stepping range and name
/// condition, and what they identify. The first matching row wins.
struct Rule {
    family: u16,
    models: (u16, u16),
    steppings: (u8, u8),
    when: When,
    codename: &'static str,
    /// Process node in nm, `0` when the vendor's node name has no nm figure worth showing.
    nm: u32,
    package: Pkg,
}

const fn span(
    family: u16,
    models: (u16, u16),
    when: When,
    codename: &'static str,
    nm: u32,
    package: Pkg,
) -> Rule {
    Rule { family, models, steppings: (0, 15), when, codename, nm, package }
}

const fn one(family: u16, model: u16, when: When, codename: &'static str, nm: u32, package: Pkg) -> Rule {
    span(family, (model, model), when, codename, nm, package)
}

/// A row limited to a stepping range, for model numbers reused across generations.
const fn stepped(
    family: u16,
    model: u16,
    steppings: (u8, u8),
    when: When,
    codename: &'static str,
    nm: u32,
    package: Pkg,
) -> Rule {
    Rule { family, models: (model, model), steppings, when, codename, nm, package }
}

const LGA775: &str = "Socket 775 LGA";
const LGA1366: &str = "Socket 1366 LGA";
const LGA1156: &str = "Socket 1156 LGA";
const LGA1155: &str = "Socket 1155 LGA";
const LGA1150: &str = "Socket 1150 LGA";
const LGA1151: &str = "Socket 1151 LGA";
const LGA1200: &str = "Socket 1200 LGA";
const LGA1700: &str = "Socket 1700 LGA";
const LGA1851: &str = "Socket 1851 LGA";
const LGA1356: &str = "Socket 1356 LGA";
const LGA1567: &str = "Socket 1567 LGA";
const LGA2011: &str = "Socket 2011 LGA";
const LGA2011_1: &str = "Socket 2011-1 LGA";
const LGA2011_3: &str = "Socket 2011-3 LGA";
const LGA2066: &str = "Socket 2066 LGA";
const LGA3647: &str = "Socket 3647 LGA";
const LGA4189: &str = "Socket 4189 LGA";
const LGA4677: &str = "Socket 4677 LGA";
const BGA1168: &str = "Socket 1168 FCBGA";
const BGA1356: &str = "Socket 1356 FCBGA";
const BGA1364: &str = "Socket 1364 FCBGA";
const BGA1440: &str = "Socket 1440 FCBGA";
const BGA1528: &str = "Socket 1528 FCBGA";
const BGA1744: &str = "Socket 1744 FCBGA";
const BGA1964: &str = "Socket 1964 FCBGA";
const BGA2049: &str = "Socket 2049 FCBGA";

/// Intel, family 6 unless noted. Model numbers are the display ("extended") models.
const INTEL: &[Rule] = &[
    // Core 2: 65 nm, then the 45 nm shrink.
    one(6, 0x0F, Name(&["Core 2 Quad Q6"]), "Kentsfield", 65, Any(LGA775)),
    // The first dual-core parts report a bare model number: `Core 2 6600`, `Core 2 T7200`.
    one(6, 0x0F, Name(&["Core 2 Duo E6", "Core 2 6"]), "Conroe", 65, Any(LGA775)),
    one(6, 0x0F, Name(&["Core 2 Duo E4", "Core 2 4"]), "Allendale", 65, Any(LGA775)),
    one(
        6,
        0x0F,
        Name(&["Core 2 Duo T", "Core 2 Duo L", "Core 2 Duo U", "Core 2 T", "Core 2 L", "Core 2 U"]),
        "Merom",
        65,
        Unknown,
    ),
    one(6, 0x17, Name(&["Core 2 Quad Q90", "Core 2 Quad Q91"]), "Penryn", 45, Unknown),
    one(6, 0x17, Name(&["Core 2 Quad Q8", "Core 2 Quad Q9"]), "Yorkfield", 45, Any(LGA775)),
    one(6, 0x17, Name(&["Core 2 Duo E7", "Core 2 Duo E8"]), "Wolfdale", 45, Any(LGA775)),
    one(
        6,
        0x17,
        Name(&["Core 2 Duo T", "Core 2 Duo P", "Core 2 Duo L", "Core 2 Duo U"]),
        "Penryn",
        45,
        Unknown,
    ),
    // Nehalem (45 nm) and Westmere (32 nm).
    one(6, 0x1A, Name(&["Xeon E55", "Xeon X55", "Xeon L55", "Xeon W55"]), "Gainestown", 45, Any(LGA1366)),
    one(6, 0x1A, Always, "Bloomfield", 45, Any(LGA1366)),
    // The mobile Extreme models `X 920` and `X 940` go by name: a detached `X` otherwise marks a desktop one.
    one(6, 0x1E, Name(&["i7 X 9"]), "Clarksfield", 45, Unknown),
    one(6, 0x1E, When::Mobile, "Clarksfield", 45, Unknown),
    one(6, 0x1E, When::Socketed, "Lynnfield", 45, Any(LGA1156)),
    one(6, 0x2E, Always, "Beckton", 45, Any(LGA1567)),
    one(6, 0x25, When::Mobile, "Arrandale", 32, Unknown),
    one(6, 0x25, When::Socketed, "Clarkdale", 32, Any(LGA1156)),
    one(6, 0x2C, Name(&["Xeon E56", "Xeon X56", "Xeon L56"]), "Westmere-EP", 32, Any(LGA1366)),
    one(6, 0x2C, Always, "Gulftown", 32, Any(LGA1366)),
    one(6, 0x2F, Always, "Westmere-EX", 32, Any(LGA1567)),
    // Sandy Bridge (32 nm) and Ivy Bridge (22 nm).
    one(6, 0x2A, Always, "Sandy Bridge", 32, Pkg::Socketed(LGA1155)),
    one(6, 0x2D, Name(&["Xeon E5 24"]), "Sandy Bridge-EN", 32, Any(LGA1356)),
    one(6, 0x2D, Name(&["Xeon"]), "Sandy Bridge-EP", 32, Any(LGA2011)),
    one(6, 0x2D, Always, "Sandy Bridge-E", 32, Any(LGA2011)),
    one(6, 0x3A, Always, "Ivy Bridge", 22, Pkg::Socketed(LGA1155)),
    one(6, 0x3E, Name(&["Xeon E7"]), "Ivy Bridge-EX", 22, Any(LGA2011_1)),
    one(6, 0x3E, Name(&["Xeon E5 24"]), "Ivy Bridge-EN", 22, Any(LGA1356)),
    one(6, 0x3E, Name(&["Xeon"]), "Ivy Bridge-EP", 22, Any(LGA2011)),
    one(6, 0x3E, Always, "Ivy Bridge-E", 22, Any(LGA2011)),
    // Haswell (22 nm) and Broadwell (14 nm).
    one(6, 0x3C, Always, "Haswell", 22, Pkg::Socketed(LGA1150)),
    one(6, 0x45, Always, "Haswell-ULT", 22, Any(BGA1168)),
    one(6, 0x46, Always, "Crystal Well", 22, Any(BGA1364)),
    one(6, 0x3F, Name(&["Xeon E7"]), "Haswell-EX", 22, Any(LGA2011_1)),
    one(6, 0x3F, Name(&["Xeon E5 24"]), "Haswell-EN", 22, Unknown),
    one(6, 0x3F, Name(&["Xeon"]), "Haswell-EP", 22, Any(LGA2011_3)),
    one(6, 0x3F, Always, "Haswell-E", 22, Any(LGA2011_3)),
    one(6, 0x3D, Always, "Broadwell-U", 14, Unknown),
    one(6, 0x47, Always, "Broadwell", 14, Split(LGA1150, BGA1364)),
    one(6, 0x4F, Name(&["Xeon E7"]), "Broadwell-EX", 14, Any(LGA2011_1)),
    one(6, 0x4F, Name(&["Xeon"]), "Broadwell-EP", 14, Any(LGA2011_3)),
    one(6, 0x4F, Always, "Broadwell-E", 14, Any(LGA2011_3)),
    one(6, 0x56, Always, "Broadwell-DE", 14, Any("Socket 1667 FCBGA")),
    // Skylake and its 14 nm successors.
    one(6, 0x4E, Suffix(&["U"]), "Skylake-U", 14, Any(BGA1356)),
    one(6, 0x4E, Always, "Skylake-U/Y", 14, Unknown),
    one(6, 0x5E, Always, "Skylake", 14, Split(LGA1151, BGA1440)),
    stepped(6, 0x55, (0, 4), Name(&["Xeon D"]), "Skylake-D", 14, Unknown),
    stepped(6, 0x55, (0, 4), Name(&["Xeon W-3"]), "Skylake-W", 14, Any(LGA3647)),
    stepped(6, 0x55, (0, 4), Name(&["Xeon W"]), "Skylake-W", 14, Any(LGA2066)),
    stepped(6, 0x55, (0, 4), Name(&["Xeon"]), "Skylake-SP", 14, Any(LGA3647)),
    stepped(6, 0x55, (0, 4), Always, "Skylake-X", 14, Any(LGA2066)),
    stepped(6, 0x55, (5, 7), Name(&["Xeon W-3"]), "Cascade Lake-W", 14, Any(LGA3647)),
    stepped(6, 0x55, (5, 7), Name(&["Xeon W"]), "Cascade Lake-W", 14, Any(LGA2066)),
    stepped(6, 0x55, (5, 7), Name(&["Xeon"]), "Cascade Lake-SP", 14, Any(LGA3647)),
    stepped(6, 0x55, (5, 7), Always, "Cascade Lake-X", 14, Any(LGA2066)),
    stepped(6, 0x55, (10, 11), Always, "Cooper Lake", 14, Any(LGA4189)),
    stepped(6, 0x8E, (9, 9), Always, "Kaby Lake-U/Y", 14, Unknown),
    stepped(
        6,
        0x8E,
        (10, 10),
        Name(&["8109U", "8259U", "8269U", "8559U", "8257U", "8279U", "8557U", "8569U"]),
        "Coffee Lake-U",
        14,
        Any(BGA1528),
    ),
    stepped(6, 0x8E, (10, 10), Always, "Kaby Lake-R", 14, Any(BGA1356)),
    stepped(6, 0x8E, (11, 12), Suffix(&["Y"]), "Amber Lake-Y", 14, Unknown),
    stepped(
        6,
        0x8E,
        (11, 12),
        Name(&["i7 10", "i5 10", "i3 10", "6405U", "5205U", "5305U"]),
        "Comet Lake-U",
        14,
        Any(BGA1528),
    ),
    stepped(6, 0x8E, (11, 12), Always, "Whiskey Lake-U", 14, Any(BGA1528)),
    stepped(6, 0x9E, (9, 9), Name(&["7740X", "7640X"]), "Kaby Lake-X", 14, Any(LGA2066)),
    stepped(6, 0x9E, (9, 9), Always, "Kaby Lake", 14, Split(LGA1151, BGA1440)),
    stepped(6, 0x9E, (10, 13), Always, "Coffee Lake", 14, Split(LGA1151, BGA1440)),
    one(6, 0xA5, Always, "Comet Lake", 14, Split(LGA1200, BGA1440)),
    one(6, 0xA6, Always, "Comet Lake-U", 14, Any(BGA1528)),
    one(6, 0xA7, Always, "Rocket Lake", 14, Any(LGA1200)),
    // 10 nm, including the node later renamed "Intel 7".
    one(6, 0x66, Always, "Cannon Lake", 10, Unknown),
    one(6, 0x7E, Always, "Ice Lake", 10, Unknown),
    one(6, 0x6A, Always, "Ice Lake-SP", 10, Any(LGA4189)),
    one(6, 0x6C, Always, "Ice Lake-D", 10, Unknown),
    one(6, 0x8C, Always, "Tiger Lake", 10, Unknown),
    one(6, 0x8D, Always, "Tiger Lake-H", 10, Pkg::Mobile("Socket 1787 FCBGA")),
    one(6, 0x8A, Always, "Lakefield", 10, Unknown),
    one(6, 0x97, Always, "Alder Lake", 10, Split(LGA1700, BGA1964)),
    one(6, 0x9A, Suffix(&["H", "HK", "P"]), "Alder Lake", 10, Any(BGA1744)),
    one(6, 0x9A, Always, "Alder Lake", 10, Unknown),
    one(6, 0xBE, Always, "Alder Lake-N", 10, Any("Socket 1264 FCBGA")),
    one(6, 0xB7, Always, "Raptor Lake", 10, Split(LGA1700, BGA1964)),
    one(6, 0xBF, Always, "Raptor Lake", 10, Split(LGA1700, BGA1964)),
    one(6, 0xBA, Suffix(&["H", "HK", "P"]), "Raptor Lake", 10, Any(BGA1744)),
    one(6, 0xBA, Always, "Raptor Lake", 10, Unknown),
    one(6, 0x8F, Name(&["Xeon w"]), "Sapphire Rapids-WS", 10, Any(LGA4677)),
    one(6, 0x8F, Always, "Sapphire Rapids", 10, Any(LGA4677)),
    one(6, 0xCF, Always, "Emerald Rapids", 10, Any(LGA4677)),
    // Intel 4 and the tiles built at TSMC; the "Intel 3" and "18A" node names carry no nm figure.
    one(6, 0xAA, Always, "Meteor Lake", 7, Any(BGA2049)),
    one(6, 0xAD, Always, "Granite Rapids", 0, Unknown),
    one(6, 0xAF, Always, "Sierra Forest", 0, Unknown),
    one(6, 0xBD, Always, "Lunar Lake", 3, Any("Socket 2833 FCBGA")),
    one(6, 0xC6, Always, "Arrow Lake", 3, Split(LGA1851, "Socket 2114 FCBGA")),
    one(6, 0xC5, Always, "Arrow Lake-H", 3, Any(BGA2049)),
    one(6, 0xB5, Always, "Arrow Lake-U", 0, Any(BGA2049)),
    one(6, 0xCC, Always, "Panther Lake", 0, Unknown),
    // Atom lines and Xeon Phi.
    one(6, 0x37, Always, "Bay Trail", 22, Unknown),
    one(6, 0x4D, Always, "Avoton", 22, Any("Socket 1283 FCBGA")),
    one(6, 0x4C, Name(&["Atom"]), "Cherry Trail", 14, Unknown),
    one(6, 0x4C, Always, "Braswell", 14, Any("Socket 1170 FCBGA")),
    one(6, 0x5C, Always, "Apollo Lake", 14, Any("Socket 1296 FCBGA")),
    one(6, 0x5F, Always, "Denverton", 14, Any("Socket 1310 FCBGA")),
    one(6, 0x7A, Always, "Gemini Lake", 14, Any("Socket 1090 FCBGA")),
    one(6, 0x9C, Always, "Jasper Lake", 10, Any("Socket 1338 FCBGA")),
    one(6, 0x96, Always, "Elkhart Lake", 10, Any("Socket 1493 FCBGA")),
    one(6, 0x57, Always, "Knights Landing", 14, Any(LGA3647)),
    one(6, 0x85, Always, "Knights Mill", 14, Any(LGA3647)),
];

const S939: &str = "Socket 939";
const AM2: &str = "Socket AM2 (940)";
const AM2_PLUS: &str = "Socket AM2+ (940)";
const AM3: &str = "Socket AM3 (938)";
const AM3_PLUS: &str = "Socket AM3+ (942)";
const FM1: &str = "Socket FM1 (905)";
const FM2: &str = "Socket FM2 (904)";
const FM2_PLUS: &str = "Socket FM2+ (906)";
const AM4: &str = "Socket AM4 (1331)";
const AM5: &str = "Socket AM5 (LGA1718)";
const C32: &str = "Socket C32 (LGA1207)";
const G34: &str = "Socket G34 (LGA1944)";
const TR4: &str = "Socket TR4 (LGA4094)";
const STRX4: &str = "Socket sTRX4 (LGA4094)";
const SWRX8: &str = "Socket sWRX8 (LGA4094)";
const STR5: &str = "Socket sTR5 (LGA4844)";
const SP3: &str = "Socket SP3 (LGA4094)";
const SP5: &str = "Socket SP5 (LGA6096)";
const SP6: &str = "Socket SP6 (LGA4844)";
const FP4: &str = "Socket FP4";
const FP5: &str = "Socket FP5";
const FP6: &str = "Socket FP6";
const FP8: &str = "Socket FP8";
const FL1: &str = "Socket FL1";

/// AMD. Families and models are the display ("extended") values.
const AMD: &[Rule] = &[
    // K8: 90 nm, then 65 nm.
    one(0x0F, 0x1F, Always, "Winchester", 90, Any(S939)),
    one(0x0F, 0x23, Always, "Toledo", 90, Any(S939)),
    one(0x0F, 0x27, Always, "San Diego", 90, Any(S939)),
    one(0x0F, 0x2B, Always, "Manchester", 90, Any(S939)),
    one(0x0F, 0x2F, Always, "Venice", 90, Any(S939)),
    one(0x0F, 0x43, Always, "Windsor", 90, Any(AM2)),
    one(0x0F, 0x4B, Always, "Windsor", 90, Any(AM2)),
    one(0x0F, 0x4F, Name(&["Sempron"]), "Manila", 90, Any(AM2)),
    one(0x0F, 0x4F, Always, "Orleans", 90, Any(AM2)),
    one(0x0F, 0x5F, Name(&["Sempron"]), "Manila", 90, Any(AM2)),
    one(0x0F, 0x5F, Always, "Orleans", 90, Any(AM2)),
    one(0x0F, 0x6B, Always, "Brisbane", 65, Any(AM2)),
    one(0x0F, 0x7F, Name(&["Sempron"]), "Sparta", 65, Any(AM2)),
    one(0x0F, 0x7F, Always, "Lima", 65, Any(AM2)),
    // K10: 65 nm, then 45 nm.
    one(0x10, 0x02, Name(&["Opteron"]), "Barcelona", 65, Unknown),
    one(0x10, 0x02, Name(&["Athlon"]), "Kuma", 65, Any(AM2_PLUS)),
    one(0x10, 0x02, Name(&["Phenom 8", "Phenom X3"]), "Toliman", 65, Any(AM2_PLUS)),
    one(0x10, 0x02, Always, "Agena", 65, Any(AM2_PLUS)),
    one(0x10, 0x04, Name(&["Opteron"]), "Shanghai", 45, Unknown),
    one(0x10, 0x04, Name(&["Phenom II X4 920", "Phenom II X4 940"]), "Deneb", 45, Any(AM2_PLUS)),
    one(0x10, 0x04, Name(&["Phenom II X4"]), "Deneb", 45, Any(AM3)),
    one(0x10, 0x04, Name(&["Phenom II X3"]), "Heka", 45, Any(AM3)),
    one(0x10, 0x04, Name(&["Phenom II X2"]), "Callisto", 45, Any(AM3)),
    one(0x10, 0x05, Name(&["Athlon II X4"]), "Propus", 45, Any(AM3)),
    one(0x10, 0x05, Name(&["Athlon II X3"]), "Rana", 45, Any(AM3)),
    one(0x10, 0x06, Name(&["Athlon II X2"]), "Regor", 45, Any(AM3)),
    one(0x10, 0x06, Name(&["Sempron 1"]), "Sargas", 45, Any(AM3)),
    one(0x10, 0x08, Name(&["Opteron 41"]), "Lisbon", 45, Any(C32)),
    one(0x10, 0x08, Name(&["Opteron"]), "Istanbul", 45, Any("Socket F (LGA1207)")),
    one(0x10, 0x09, Name(&["Opteron 61"]), "Magny-Cours", 45, Any(G34)),
    one(0x10, 0x0A, Name(&["Phenom II X6"]), "Thuban", 45, Any(AM3)),
    one(0x10, 0x0A, Name(&["Phenom II X4"]), "Zosma", 45, Any(AM3)),
    // Llano, the Bobcat and Jaguar small cores.
    one(0x12, 0x01, Always, "Llano", 32, Split(FM1, "Socket FS1 (722)")),
    span(0x14, (0x01, 0x02), Name(&["AMD C-"]), "Ontario", 40, Any("Socket FT1 (BGA413)")),
    span(0x14, (0x01, 0x02), Always, "Zacate", 40, Any("Socket FT1 (BGA413)")),
    one(
        0x16,
        0x00,
        Name(&["Athlon 5", "Sempron 3850", "Sempron 2650"]),
        "Kabini",
        28,
        Any("Socket AM1 (721)"),
    ),
    one(0x16, 0x00, Always, "Kabini", 28, Unknown),
    one(0x16, 0x30, Name(&["Micro"]), "Mullins", 28, Unknown),
    one(0x16, 0x30, Always, "Beema", 28, Unknown),
    // Bulldozer family: 32 nm, then 28 nm.
    one(0x15, 0x01, Name(&["Opteron 62"]), "Interlagos", 32, Any(G34)),
    one(0x15, 0x01, Name(&["Opteron 42"]), "Valencia", 32, Any(C32)),
    one(0x15, 0x01, Name(&["Opteron 32"]), "Zurich", 32, Any(AM3_PLUS)),
    one(0x15, 0x01, Name(&["FX-"]), "Zambezi", 32, Any(AM3_PLUS)),
    one(0x15, 0x02, Name(&["Opteron 63"]), "Abu Dhabi", 32, Any(G34)),
    one(0x15, 0x02, Name(&["Opteron 43"]), "Seoul", 32, Any(C32)),
    one(0x15, 0x02, Name(&["Opteron 33"]), "Delhi", 32, Any(AM3_PLUS)),
    one(0x15, 0x02, Name(&["FX-"]), "Vishera", 32, Any(AM3_PLUS)),
    one(0x15, 0x10, Always, "Trinity", 32, Pkg::Socketed(FM2)),
    one(0x15, 0x13, Always, "Richland", 32, Pkg::Socketed(FM2)),
    one(0x15, 0x30, Suffix(&["K"]), "Kaveri", 28, Any(FM2_PLUS)),
    one(0x15, 0x30, Always, "Kaveri", 28, Unknown),
    one(0x15, 0x38, Always, "Godavari", 28, Any(FM2_PLUS)),
    one(0x15, 0x60, Name(&["Athlon X4 845"]), "Carrizo", 28, Any(FM2_PLUS)),
    one(0x15, 0x60, Always, "Carrizo", 28, Pkg::Mobile(FP4)),
    one(0x15, 0x65, Always, "Bristol Ridge", 28, Split(AM4, FP4)),
    one(0x15, 0x70, Always, "Stoney Ridge", 28, Unknown),
    // Zen (14 nm), Zen+ (12 nm) and Zen 2 (7 nm).
    one(0x17, 0x01, Name(&["EPYC 3"]), "Snowy Owl", 14, Unknown),
    one(0x17, 0x01, Name(&["EPYC"]), "Naples", 14, Any(SP3)),
    one(0x17, 0x01, Name(&["Threadripper"]), "Whitehaven", 14, Any(TR4)),
    one(0x17, 0x01, Always, "Summit Ridge", 14, Any(AM4)),
    one(0x17, 0x08, Name(&["Threadripper"]), "Colfax", 12, Any(TR4)),
    one(0x17, 0x08, Always, "Pinnacle Ridge", 12, Any(AM4)),
    one(0x17, 0x11, Always, "Raven Ridge", 14, Split(AM4, FP5)),
    one(0x17, 0x18, Always, "Picasso", 12, Split(AM4, FP5)),
    one(0x17, 0x20, Always, "Dali", 14, Split(AM4, FP5)),
    one(0x17, 0x31, Name(&["EPYC"]), "Rome", 7, Any(SP3)),
    one(0x17, 0x31, Name(&["Threadripper PRO"]), "Castle Peak", 7, Any(SWRX8)),
    one(0x17, 0x31, Name(&["Threadripper"]), "Castle Peak", 7, Any(STRX4)),
    one(0x17, 0x60, Always, "Renoir", 7, Split(AM4, FP6)),
    one(0x17, 0x68, Always, "Lucienne", 7, Any(FP6)),
    one(0x17, 0x71, Always, "Matisse", 7, Any(AM4)),
    one(0x17, 0x90, Always, "Van Gogh", 7, Unknown),
    one(0x17, 0xA0, Always, "Mendocino", 6, Any("Socket FT6")),
    // Zen 3 (7 nm) and Zen 4 (5 nm, 4 nm for the mobile dies).
    one(0x19, 0x01, Always, "Milan", 7, Any(SP3)),
    one(0x19, 0x08, Always, "Chagall", 7, Any(SWRX8)),
    one(0x19, 0x18, Always, "Storm Peak", 5, Any(STR5)),
    span(0x19, (0x10, 0x1F), Always, "Genoa", 5, Any(SP5)),
    one(0x19, 0x21, Always, "Vermeer", 7, Any(AM4)),
    span(0x19, (0x40, 0x4F), Always, "Rembrandt", 6, Any("Socket FP7")),
    span(0x19, (0x50, 0x5F), Always, "Cezanne", 7, Split(AM4, FP6)),
    one(0x19, 0x61, When::Mobile, "Dragon Range", 5, Any(FL1)),
    one(0x19, 0x61, Always, "Raphael", 5, Any(AM5)),
    one(0x19, 0x74, Always, "Phoenix", 4, Pkg::Socketed(AM5)),
    one(0x19, 0x75, When::Mobile, "Hawk Point", 4, Unknown),
    one(0x19, 0x75, Always, "Phoenix", 4, Pkg::Socketed(AM5)),
    span(0x19, (0x78, 0x7F), Always, "Phoenix2", 4, Pkg::Socketed(AM5)),
    span(0x19, (0xA0, 0xAF), Name(&["EPYC 8"]), "Siena", 5, Any(SP6)),
    span(0x19, (0xA0, 0xAF), Always, "Bergamo", 5, Any(SP5)),
    // Zen 5 (4 nm, 3 nm for the dense server die).
    one(0x1A, 0x08, Always, "Shimada Peak", 4, Any(STR5)),
    span(0x1A, (0x00, 0x0F), Always, "Turin", 4, Any(SP5)),
    span(0x1A, (0x10, 0x1F), Always, "Turin Dense", 3, Any(SP5)),
    span(0x1A, (0x20, 0x2F), Always, "Strix Point", 4, Any(FP8)),
    span(0x1A, (0x40, 0x4F), When::Mobile, "Fire Range", 4, Any(FL1)),
    span(0x1A, (0x40, 0x4F), Name(&["EPYC"]), "Grado", 4, Any(AM5)),
    span(0x1A, (0x40, 0x4F), Always, "Granite Ridge", 4, Any(AM5)),
    span(0x1A, (0x60, 0x6F), Always, "Krackan Point", 4, Any(FP8)),
    span(0x1A, (0x70, 0x7F), Always, "Strix Halo", 4, Any("Socket FP11")),
];

const HYGON: &[Rule] = &[span(0x18, (0x00, 0x0F), Always, "Dhyana", 14, Unknown)];

/// Finds the first table row matching a processor. `family` and `model` are
/// the display values, `name` the short marketing name.
pub fn lookup(vendor: CpuVendor, family: u16, model: u16, stepping: u8, name: &str) -> Option<Known> {
    let rules = match vendor {
        CpuVendor::Intel => INTEL,
        CpuVendor::Amd => AMD,
        CpuVendor::Hygon => HYGON,
        _ => return None,
    };
    let mounting = segment(vendor, name);
    let suffix = model_number(name).map_or("", |m| m.suffix);
    let rule = rules.iter().find(|rule| {
        rule.family == family
            && (rule.models.0..=rule.models.1).contains(&model)
            && (rule.steppings.0..=rule.steppings.1).contains(&stepping)
            && match rule.when {
                Always => true,
                Name(fragments) => fragments.iter().any(|f| name.contains(f)),
                When::Mobile => mounting == Segment::Mobile,
                When::Socketed => mounting == Segment::Desktop,
                Suffix(suffixes) => suffixes.contains(&suffix),
            }
    })?;
    let package = match (rule.package, mounting) {
        (Any(package), _)
        | (Pkg::Socketed(package) | Split(package, _), Segment::Desktop)
        | (Pkg::Mobile(package) | Split(_, package), Segment::Mobile) => Some(package),
        _ => None,
    };
    Some(Known { codename: rule.codename, technology_nm: (rule.nm != 0).then_some(rule.nm), package })
}

/// Stepping names by vendor, display family, display model and stepping.
/// Only steppings whose name is unambiguous for the whole model are listed.
const REVISIONS: &[(CpuVendor, u16, u16, u8, &str)] = &[
    (CpuVendor::Intel, 6, 0x1A, 4, "C0"),
    (CpuVendor::Intel, 6, 0x1A, 5, "D0"),
    (CpuVendor::Intel, 6, 0x1E, 5, "B1"),
    (CpuVendor::Intel, 6, 0x25, 2, "C2"),
    (CpuVendor::Intel, 6, 0x25, 5, "K0"),
    (CpuVendor::Intel, 6, 0x2C, 2, "B1"),
    (CpuVendor::Intel, 6, 0x3C, 3, "C0"),
    (CpuVendor::Intel, 6, 0x5E, 3, "R0"),
    (CpuVendor::Intel, 6, 0x9E, 9, "B0"),
    (CpuVendor::Intel, 6, 0x9E, 10, "U0"),
    (CpuVendor::Intel, 6, 0x9E, 11, "B0"),
    (CpuVendor::Intel, 6, 0x9E, 12, "P0"),
    (CpuVendor::Intel, 6, 0x9E, 13, "R0"),
    (CpuVendor::Intel, 6, 0x8E, 10, "Y0"),
    (CpuVendor::Intel, 6, 0xA7, 1, "B0"),
    (CpuVendor::Intel, 6, 0x97, 2, "C0"),
    (CpuVendor::Intel, 6, 0x97, 5, "H0"),
    (CpuVendor::Intel, 6, 0xB7, 1, "B0"),
    (CpuVendor::Intel, 6, 0xBF, 2, "C0"),
    (CpuVendor::Intel, 6, 0xBF, 5, "H0"),
    (CpuVendor::Amd, 0x10, 0x02, 2, "DR-B2"),
    (CpuVendor::Amd, 0x10, 0x02, 3, "DR-B3"),
    (CpuVendor::Amd, 0x10, 0x04, 2, "RB-C2"),
    (CpuVendor::Amd, 0x10, 0x04, 3, "RB-C3"),
    (CpuVendor::Amd, 0x10, 0x05, 2, "BL-C2"),
    (CpuVendor::Amd, 0x10, 0x05, 3, "BL-C3"),
    (CpuVendor::Amd, 0x10, 0x06, 2, "DA-C2"),
    (CpuVendor::Amd, 0x10, 0x06, 3, "DA-C3"),
    (CpuVendor::Amd, 0x10, 0x0A, 0, "PH-E0"),
    (CpuVendor::Amd, 0x15, 0x01, 2, "OR-B2"),
    (CpuVendor::Amd, 0x15, 0x02, 0, "OR-C0"),
    (CpuVendor::Amd, 0x17, 0x01, 1, "ZP-B1"),
    (CpuVendor::Amd, 0x17, 0x01, 2, "ZP-B2"),
    (CpuVendor::Amd, 0x17, 0x08, 2, "PiR-B2"),
    (CpuVendor::Amd, 0x17, 0x11, 0, "RV-B0"),
    (CpuVendor::Amd, 0x17, 0x18, 1, "PCO-B1"),
    (CpuVendor::Amd, 0x17, 0x20, 1, "RV2-A1"),
    (CpuVendor::Amd, 0x17, 0x60, 1, "RN-A1"),
    (CpuVendor::Amd, 0x17, 0x68, 1, "LCN-A1"),
    (CpuVendor::Amd, 0x17, 0x71, 0, "MTS-B0"),
    (CpuVendor::Amd, 0x19, 0x01, 1, "GN-B1"),
    (CpuVendor::Amd, 0x19, 0x01, 2, "GN-B2"),
    (CpuVendor::Amd, 0x19, 0x11, 1, "RS-B1"),
    (CpuVendor::Amd, 0x19, 0x21, 0, "VMR-B0"),
    (CpuVendor::Amd, 0x19, 0x21, 2, "VMR-B2"),
    (CpuVendor::Amd, 0x19, 0x44, 1, "RMB-B1"),
    (CpuVendor::Amd, 0x19, 0x50, 0, "CZN-A0"),
    (CpuVendor::Amd, 0x19, 0x61, 2, "RPL-B2"),
    (CpuVendor::Amd, 0x19, 0x74, 1, "PHX-A1"),
    (CpuVendor::Amd, 0x1A, 0x44, 0, "GNR-B0"),
];

/// The vendor's name for a core stepping, e.g. `RPL-B2` or `C0`.
pub fn revision(vendor: CpuVendor, family: u16, model: u16, stepping: u8) -> Option<&'static str> {
    REVISIONS.iter().find(|r| (r.0, r.1, r.2, r.3) == (vendor, family, model, stepping)).map(|r| r.4)
}

/// Rated thermal design power: name prefix, watts, model numbers. Socketed
/// desktop and workstation parts only; a name must match in full.
const TDP: &[(&str, f32, &[&str])] = &[
    ("AMD Ryzen 9 ", 170.0, &["7950X", "7900X", "9950X", "9950X3D"]),
    ("AMD Ryzen 9 ", 120.0, &["7950X3D", "7900X3D", "9900X", "9900X3D"]),
    ("AMD Ryzen 9 ", 105.0, &["3950X", "3900XT", "3900X", "5950X", "5900X"]),
    ("AMD Ryzen 9 ", 65.0, &["3900", "5900", "7900"]),
    ("AMD Ryzen 7 ", 120.0, &["7800X3D", "9800X3D"]),
    ("AMD Ryzen 7 ", 105.0, &["2700X", "3800XT", "3800X", "5800X", "5800X3D", "5700X3D", "7700X"]),
    ("AMD Ryzen 7 ", 95.0, &["1800X", "1700X"]),
    (
        "AMD Ryzen 7 ",
        65.0,
        &["1700", "2700", "3700X", "5800", "5700X", "5700G", "5700", "7700", "8700G", "8700F", "9700X"],
    ),
    ("AMD Ryzen 5 ", 105.0, &["5600X3D", "7600X"]),
    ("AMD Ryzen 5 ", 95.0, &["1600X", "2600X", "3600XT", "3600X"]),
    (
        "AMD Ryzen 5 ",
        65.0,
        &[
            "1600", "1500X", "1400", "2600", "2400G", "3600", "3500X", "3500", "3400G", "5600X", "5600",
            "5600G", "5600GT", "5500", "5500GT", "7600", "7600X3D", "7500F", "8600G", "8500G", "8400F",
            "9600X", "9600",
        ],
    ),
    ("AMD Ryzen 3 ", 65.0, &["1300X", "1200", "2200G", "3300X", "3100", "3200G", "4100", "5300G", "8300G"]),
    ("AMD Ryzen Threadripper ", 350.0, &["7980X", "7970X", "7960X", "9980X", "9970X", "9960X"]),
    ("AMD Ryzen Threadripper ", 280.0, &["3990X", "3970X", "3960X"]),
    ("AMD Ryzen Threadripper ", 250.0, &["2990WX", "2970WX"]),
    ("AMD Ryzen Threadripper ", 180.0, &["1950X", "1920X", "1900X", "2950X", "2920X"]),
    (
        "AMD Ryzen Threadripper PRO ",
        350.0,
        &[
            "7995WX", "7985WX", "7975WX", "7965WX", "7955WX", "7945WX", "9995WX", "9985WX", "9975WX",
            "9965WX", "9955WX", "9945WX",
        ],
    ),
    (
        "AMD Ryzen Threadripper PRO ",
        280.0,
        &["3995WX", "3975WX", "3955WX", "3945WX", "5995WX", "5975WX", "5965WX", "5955WX", "5945WX"],
    ),
    ("AMD FX-", 220.0, &["9590", "9370"]),
    ("AMD FX-", 125.0, &["8350", "8320", "8370", "8150", "6350", "4350"]),
    ("AMD FX-", 95.0, &["8300", "8320E", "8370E", "6300", "6100", "4300", "4100"]),
    ("Intel Core Ultra 9 ", 125.0, &["285K"]),
    ("Intel Core Ultra 9 ", 65.0, &["285"]),
    ("Intel Core Ultra 9 ", 35.0, &["285T"]),
    ("Intel Core Ultra 7 ", 125.0, &["265K", "265KF"]),
    ("Intel Core Ultra 7 ", 65.0, &["265", "265F"]),
    ("Intel Core Ultra 7 ", 35.0, &["265T"]),
    ("Intel Core Ultra 5 ", 125.0, &["245K", "245KF"]),
    ("Intel Core Ultra 5 ", 65.0, &["245", "235", "225", "225F"]),
    ("Intel Core Ultra 5 ", 35.0, &["245T", "235T"]),
    (
        "Intel Core i9 ",
        165.0,
        &[
            "7980XE", "7960X", "7940X", "9980XE", "9960X", "9940X", "9920X", "9900X", "9820X", "10980XE",
            "10940X", "10920X", "10900X",
        ],
    ),
    ("Intel Core i9 ", 150.0, &["12900KS", "13900KS", "14900KS"]),
    ("Intel Core i9 ", 140.0, &["7920X", "7900X"]),
    ("Intel Core i9 ", 127.0, &["9900KS"]),
    (
        "Intel Core i9 ",
        125.0,
        &[
            "10900K", "10900KF", "10850K", "11900K", "11900KF", "12900K", "12900KF", "13900K", "13900KF",
            "14900K", "14900KF",
        ],
    ),
    ("Intel Core i9 ", 95.0, &["9900K", "9900KF"]),
    (
        "Intel Core i9 ",
        65.0,
        &[
            "9900", "10900", "10900F", "11900", "11900F", "12900", "12900F", "13900", "13900F", "14900",
            "14900F",
        ],
    ),
    ("Intel Core i9 ", 35.0, &["9900T", "10900T", "11900T", "12900T", "13900T", "14900T"]),
    ("Intel Core i7 ", 165.0, &["9800X"]),
    ("Intel Core i7 ", 150.0, &["3970X"]),
    (
        "Intel Core i7 ",
        140.0,
        &["5960X", "5930K", "5820K", "6950X", "6900K", "6850K", "6800K", "7820X", "7800X"],
    ),
    (
        "Intel Core i7 ",
        130.0,
        &[
            "920", "930", "940", "950", "960", "965", "975", "970", "980", "3960X", "3930K", "3820", "4960X",
            "4930K", "4820K",
        ],
    ),
    (
        "Intel Core i7 ",
        125.0,
        &[
            "10700K", "10700KF", "11700K", "11700KF", "12700K", "12700KF", "13700K", "13700KF", "14700K",
            "14700KF",
        ],
    ),
    ("Intel Core i7 ", 95.0, &["860", "870", "2600K", "2700K", "2600", "8700K", "8086K", "9700K", "9700KF"]),
    ("Intel Core i7 ", 91.0, &["6700K", "7700K"]),
    ("Intel Core i7 ", 88.0, &["4790K"]),
    ("Intel Core i7 ", 84.0, &["4770K", "4770", "4771", "4790"]),
    ("Intel Core i7 ", 77.0, &["3770K", "3770"]),
    (
        "Intel Core i7 ",
        65.0,
        &[
            "2600S", "3770S", "4770S", "4790S", "5775C", "6700", "7700", "8700", "9700", "9700F", "10700",
            "10700F", "11700", "11700F", "12700", "12700F", "13700", "13700F", "14700", "14700F",
        ],
    ),
    (
        "Intel Core i7 ",
        35.0,
        &["6700T", "7700T", "8700T", "9700T", "10700T", "11700T", "12700T", "13700T", "14700T"],
    ),
    (
        "Intel Core i5 ",
        125.0,
        &[
            "10600K", "10600KF", "11600K", "11600KF", "12600K", "12600KF", "13600K", "13600KF", "14600K",
            "14600KF",
        ],
    ),
    (
        "Intel Core i5 ",
        95.0,
        &["750", "760", "2300", "2310", "2320", "2400", "2500", "2500K", "2550K", "8600K", "9600K", "9600KF"],
    ),
    ("Intel Core i5 ", 91.0, &["6600K", "7600K"]),
    ("Intel Core i5 ", 88.0, &["4690K"]),
    ("Intel Core i5 ", 84.0, &["4430", "4440", "4460", "4570", "4590", "4670", "4670K", "4690"]),
    ("Intel Core i5 ", 77.0, &["3330", "3340", "3450", "3470", "3550", "3570", "3570K"]),
    (
        "Intel Core i5 ",
        65.0,
        &[
            "6400", "6500", "6600", "7400", "7500", "7600", "8400", "8500", "8600", "9400", "9400F", "9500",
            "9500F", "9600", "10400", "10400F", "10500", "10600", "11400", "11400F", "11500", "11600",
            "12400", "12400F", "12500", "12600", "13400", "13400F", "13500", "13600", "14400", "14400F",
            "14500", "14600",
        ],
    ),
    ("Intel Core i3 ", 91.0, &["8350K", "9350KF"]),
    (
        "Intel Core i3 ",
        65.0,
        &["2100", "2120", "2130", "8100", "9100", "9100F", "10100", "10100F", "10300", "10320"],
    ),
    ("Intel Core i3 ", 60.0, &["7350K", "12100", "13100", "14100"]),
    ("Intel Core i3 ", 58.0, &["12100F", "13100F", "14100F"]),
    ("Intel Core i3 ", 55.0, &["3220", "3225", "3240"]),
    ("Intel Core i3 ", 54.0, &["4130", "4150", "4160", "4170", "4330", "4340", "4350", "4360", "4370"]),
    ("Intel Core i3 ", 51.0, &["6100", "6300", "6320", "7100", "7300", "7320"]),
    ("Intel Core 2 Quad ", 95.0, &["Q8200", "Q8300", "Q8400", "Q9300", "Q9400", "Q9450", "Q9550", "Q9650"]),
    (
        "Intel Core 2 Duo ",
        65.0,
        &[
            "E4500", "E4600", "E4700", "E6550", "E6750", "E6850", "E7200", "E7300", "E7400", "E7500",
            "E7600", "E8200", "E8400", "E8500", "E8600",
        ],
    ),
    // The first Core 2 Duo models, whose brand strings give the bare number.
    ("Intel Core 2 ", 65.0, &["4300", "4400", "6300", "6400", "6600", "6700"]),
];

/// The rated TDP of a processor by its short marketing name, where the table is sure of it.
pub fn max_tdp(name: &str) -> Option<f32> {
    TDP.iter()
        .find(|(prefix, _, models)| name.strip_prefix(prefix).is_some_and(|model| models.contains(&model)))
        .map(|&(_, watts, _)| watts)
}

#[cfg(test)]
mod tests {
    use super::super::name::short_name;
    use super::*;
    use CpuVendor::{Amd, Intel};

    fn known(codename: &'static str, nm: u32, package: &'static str) -> Option<Known> {
        Some(Known { codename, technology_nm: Some(nm), package: (!package.is_empty()).then_some(package) })
    }

    #[test]
    fn identifies_amd_desktop_parts() {
        assert_eq!(
            lookup(Amd, 0x19, 0x61, 2, "AMD Ryzen 7 7700"),
            known("Raphael", 5, "Socket AM5 (LGA1718)")
        );
        assert_eq!(lookup(Amd, 0x19, 0x61, 2, "AMD Ryzen 9 7950X3D"), known("Raphael", 5, AM5));
        assert_eq!(lookup(Amd, 0x19, 0x61, 2, "AMD EPYC 4564P"), known("Raphael", 5, AM5));
        assert_eq!(lookup(Amd, 0x1A, 0x44, 0, "AMD Ryzen 9 9950X"), known("Granite Ridge", 4, AM5));
        assert_eq!(
            lookup(Amd, 0x19, 0x21, 2, "AMD Ryzen 7 5800X3D"),
            known("Vermeer", 7, "Socket AM4 (1331)")
        );
        assert_eq!(lookup(Amd, 0x17, 0x71, 0, "AMD Ryzen 5 3600"), known("Matisse", 7, AM4));
        assert_eq!(lookup(Amd, 0x17, 0x08, 2, "AMD Ryzen 7 2700X"), known("Pinnacle Ridge", 12, AM4));
        assert_eq!(lookup(Amd, 0x17, 0x01, 1, "AMD Ryzen 7 1700"), known("Summit Ridge", 14, AM4));
        assert_eq!(lookup(Amd, 0x15, 0x02, 0, "AMD FX-8350"), known("Vishera", 32, "Socket AM3+ (942)"));
        assert_eq!(
            lookup(Amd, 0x10, 0x04, 3, "AMD Phenom II X4 965"),
            known("Deneb", 45, "Socket AM3 (938)")
        );
        assert_eq!(lookup(Amd, 0x10, 0x04, 2, "AMD Phenom II X4 940"), known("Deneb", 45, AM2_PLUS));
        assert_eq!(lookup(Amd, 0x10, 0x0A, 0, "AMD Phenom II X6 1090T"), known("Thuban", 45, AM3));
        assert_eq!(lookup(Amd, 0x0F, 0x2F, 2, "AMD Athlon 64 3200+"), known("Venice", 90, "Socket 939"));
        assert_eq!(lookup(Amd, 0x0F, 0x6B, 1, "AMD Athlon 64 X2 4200+"), known("Brisbane", 65, AM2));
    }

    #[test]
    fn splits_amd_dies_sold_socketed_and_soldered() {
        assert_eq!(lookup(Amd, 0x19, 0x61, 2, "AMD Ryzen 9 7945HX"), known("Dragon Range", 5, "Socket FL1"));
        assert_eq!(lookup(Amd, 0x1A, 0x44, 0, "AMD Ryzen 9 9955HX3D"), known("Fire Range", 4, FL1));
        assert_eq!(lookup(Amd, 0x19, 0x50, 0, "AMD Ryzen 7 5700G"), known("Cezanne", 7, AM4));
        assert_eq!(lookup(Amd, 0x19, 0x50, 0, "AMD Ryzen 7 5800H"), known("Cezanne", 7, "Socket FP6"));
        assert_eq!(lookup(Amd, 0x17, 0x60, 1, "AMD Ryzen 7 PRO 4750U"), known("Renoir", 7, FP6));
        assert_eq!(lookup(Amd, 0x17, 0x18, 1, "AMD Ryzen 5 3400G"), known("Picasso", 12, AM4));
        assert_eq!(lookup(Amd, 0x19, 0x75, 2, "AMD Ryzen 7 8700G"), known("Phoenix", 4, AM5));
        assert_eq!(lookup(Amd, 0x19, 0x75, 2, "AMD Ryzen 7 8845HS"), known("Hawk Point", 4, ""));
        assert_eq!(lookup(Amd, 0x19, 0x74, 1, "AMD Ryzen 7 7840HS"), known("Phoenix", 4, ""));
        assert_eq!(
            lookup(Amd, 0x1A, 0x24, 0, "AMD Ryzen AI 9 HX 370"),
            known("Strix Point", 4, "Socket FP8")
        );
        // An embedded part on a desktop die: the code name holds, the package is not guessed.
        assert_eq!(lookup(Amd, 0x17, 0x11, 0, "AMD Ryzen Embedded V1605B"), known("Raven Ridge", 14, ""));
    }

    #[test]
    fn identifies_bristol_ridge_from_its_brand_strings() {
        // This generation spells the graphics part in capitals, or leaves its name out.
        let from_brand = |brand| lookup(Amd, 0x15, 0x65, 1, &short_name(Amd, brand));
        assert_eq!(
            from_brand("AMD A10-9700 RADEON R7, 10 COMPUTE CORES 4C+6G"),
            known("Bristol Ridge", 28, AM4)
        );
        assert_eq!(
            from_brand("AMD A12-9720P RADEON R7, 12 COMPUTE CORES 4C+8G"),
            known("Bristol Ridge", 28, FP4)
        );
        assert_eq!(
            from_brand("AMD PRO A10-8770 R7, 10 COMPUTE CORES 4C+6G"),
            known("Bristol Ridge", 28, AM4)
        );
    }

    #[test]
    fn identifies_amd_workstation_and_server_parts() {
        assert_eq!(lookup(Amd, 0x17, 0x01, 2, "AMD EPYC 7601"), known("Naples", 14, SP3));
        assert_eq!(lookup(Amd, 0x17, 0x01, 1, "AMD Ryzen Threadripper 1950X"), known("Whitehaven", 14, TR4));
        assert_eq!(lookup(Amd, 0x17, 0x31, 0, "AMD EPYC 7742"), known("Rome", 7, SP3));
        assert_eq!(
            lookup(Amd, 0x17, 0x31, 0, "AMD Ryzen Threadripper 3970X"),
            known("Castle Peak", 7, STRX4)
        );
        assert_eq!(
            lookup(Amd, 0x17, 0x31, 0, "AMD Ryzen Threadripper PRO 3995WX"),
            known("Castle Peak", 7, SWRX8)
        );
        assert_eq!(
            lookup(Amd, 0x19, 0x08, 2, "AMD Ryzen Threadripper PRO 5995WX"),
            known("Chagall", 7, SWRX8)
        );
        assert_eq!(lookup(Amd, 0x19, 0x18, 1, "AMD Ryzen Threadripper 7980X"), known("Storm Peak", 5, STR5));
        assert_eq!(lookup(Amd, 0x19, 0x11, 1, "AMD EPYC 9654"), known("Genoa", 5, SP5));
        assert_eq!(lookup(Amd, 0x19, 0xA0, 2, "AMD EPYC 9754"), known("Bergamo", 5, SP5));
        assert_eq!(lookup(Amd, 0x19, 0xA0, 2, "AMD EPYC 8534P"), known("Siena", 5, SP6));
        assert_eq!(lookup(Amd, 0x1A, 0x02, 1, "AMD EPYC 9755"), known("Turin", 4, SP5));
        assert_eq!(
            lookup(Amd, 0x1A, 0x08, 1, "AMD Ryzen Threadripper 9980X"),
            known("Shimada Peak", 4, STR5)
        );
        assert_eq!(lookup(Amd, 0x15, 0x01, 2, "AMD Opteron 6276"), known("Interlagos", 32, G34));
    }

    #[test]
    fn identifies_intel_desktop_parts() {
        assert_eq!(
            lookup(Intel, 6, 0xC6, 2, "Intel Core Ultra 9 285K"),
            known("Arrow Lake", 3, "Socket 1851 LGA")
        );
        assert_eq!(
            lookup(Intel, 6, 0xB7, 1, "Intel Core i9 14900K"),
            known("Raptor Lake", 10, "Socket 1700 LGA")
        );
        assert_eq!(lookup(Intel, 6, 0xBF, 2, "Intel Core i5 13400F"), known("Raptor Lake", 10, LGA1700));
        assert_eq!(lookup(Intel, 6, 0x97, 2, "Intel Core i7 12700K"), known("Alder Lake", 10, LGA1700));
        assert_eq!(
            lookup(Intel, 6, 0xA7, 1, "Intel Core i9 11900K"),
            known("Rocket Lake", 14, "Socket 1200 LGA")
        );
        assert_eq!(lookup(Intel, 6, 0xA5, 5, "Intel Core i9 10900K"), known("Comet Lake", 14, LGA1200));
        assert_eq!(
            lookup(Intel, 6, 0x9E, 12, "Intel Core i9 9900K"),
            known("Coffee Lake", 14, "Socket 1151 LGA")
        );
        assert_eq!(lookup(Intel, 6, 0x9E, 9, "Intel Core i7 7700K"), known("Kaby Lake", 14, LGA1151));
        assert_eq!(lookup(Intel, 6, 0x5E, 3, "Intel Core i7 6700K"), known("Skylake", 14, LGA1151));
        assert_eq!(lookup(Intel, 6, 0x3C, 3, "Intel Core i7 4790K"), known("Haswell", 22, "Socket 1150 LGA"));
        assert_eq!(
            lookup(Intel, 6, 0x3A, 9, "Intel Core i5 3570K"),
            known("Ivy Bridge", 22, "Socket 1155 LGA")
        );
        assert_eq!(lookup(Intel, 6, 0x2A, 7, "Intel Core i7 2600K"), known("Sandy Bridge", 32, LGA1155));
        assert_eq!(lookup(Intel, 6, 0x1E, 5, "Intel Core i5 750"), known("Lynnfield", 45, "Socket 1156 LGA"));
        assert_eq!(
            lookup(Intel, 6, 0x1A, 5, "Intel Core i7 920"),
            known("Bloomfield", 45, "Socket 1366 LGA")
        );
        assert_eq!(
            lookup(Intel, 6, 0x17, 10, "Intel Core 2 Duo E8500"),
            known("Wolfdale", 45, "Socket 775 LGA")
        );
        assert_eq!(lookup(Intel, 6, 0x0F, 11, "Intel Core 2 Quad Q6600"), known("Kentsfield", 65, LGA775));
        assert_eq!(lookup(Intel, 6, 0x0F, 11, "Intel Core 2 Duo E6750"), known("Conroe", 65, LGA775));
        assert_eq!(lookup(Intel, 6, 0x0F, 13, "Intel Core 2 Duo E4500"), known("Allendale", 65, LGA775));
    }

    #[test]
    fn identifies_the_first_core_2_models_by_their_bare_numbers() {
        let from_brand = |brand| lookup(Intel, 6, 0x0F, 6, &short_name(Intel, brand));
        assert_eq!(
            from_brand("Intel(R) Core(TM)2 CPU          6600  @ 2.40GHz"),
            known("Conroe", 65, LGA775)
        );
        assert_eq!(
            from_brand("Intel(R) Core(TM)2 CPU          6300  @ 1.86GHz"),
            known("Conroe", 65, LGA775)
        );
        assert_eq!(
            from_brand("Intel(R) Core(TM)2 CPU          4300  @ 1.80GHz"),
            known("Allendale", 65, LGA775)
        );
        assert_eq!(from_brand("Intel(R) Core(TM)2 CPU         T7200  @ 2.00GHz"), known("Merom", 65, ""));
        // An Extreme is neither of these, whatever its number starts with.
        assert_eq!(from_brand("Intel(R) Core(TM)2 CPU         X6800  @ 2.93GHz"), None);
    }

    #[test]
    fn older_desktop_parts_with_a_p_suffix_are_socketed() {
        assert_eq!(lookup(Intel, 6, 0x5E, 3, "Intel Core i5 6402P"), known("Skylake", 14, LGA1151));
        assert_eq!(lookup(Intel, 6, 0x5E, 3, "Intel Core i3 6098P"), known("Skylake", 14, LGA1151));
        assert_eq!(lookup(Intel, 6, 0x3A, 9, "Intel Core i5 3350P"), known("Ivy Bridge", 22, LGA1155));
        assert_eq!(lookup(Intel, 6, 0x2A, 7, "Intel Core i5 2380P"), known("Sandy Bridge", 32, LGA1155));
    }

    #[test]
    fn identifies_intel_mobile_parts() {
        assert_eq!(
            lookup(Intel, 6, 0xBD, 4, "Intel Core Ultra 7 258V"),
            known("Lunar Lake", 3, "Socket 2833 FCBGA")
        );
        assert_eq!(lookup(Intel, 6, 0xAA, 4, "Intel Core Ultra 7 155H"), known("Meteor Lake", 7, BGA2049));
        assert_eq!(
            lookup(Intel, 6, 0xC6, 2, "Intel Core Ultra 9 285HX"),
            known("Arrow Lake", 3, "Socket 2114 FCBGA")
        );
        assert_eq!(lookup(Intel, 6, 0xB7, 1, "Intel Core i9 13980HX"), known("Raptor Lake", 10, BGA1964));
        assert_eq!(lookup(Intel, 6, 0xBA, 2, "Intel Core i7 13700H"), known("Raptor Lake", 10, BGA1744));
        assert_eq!(lookup(Intel, 6, 0x9A, 3, "Intel Core i7 12700H"), known("Alder Lake", 10, BGA1744));
        // The U-series die ships in two packages the name does not tell apart.
        assert_eq!(lookup(Intel, 6, 0x9A, 4, "Intel Core i7 1255U"), known("Alder Lake", 10, ""));
        assert_eq!(
            lookup(Intel, 6, 0x9E, 10, "Intel Core i7 8750H"),
            known("Coffee Lake", 14, "Socket 1440 FCBGA")
        );
        assert_eq!(lookup(Intel, 6, 0x8E, 10, "Intel Core i5 8250U"), known("Kaby Lake-R", 14, BGA1356));
        assert_eq!(lookup(Intel, 6, 0x8E, 12, "Intel Core i7 10510U"), known("Comet Lake-U", 14, BGA1528));
        assert_eq!(lookup(Intel, 6, 0x8E, 12, "Intel Core i7 8565U"), known("Whiskey Lake-U", 14, BGA1528));
        assert_eq!(lookup(Intel, 6, 0x9A, 3, "Intel Core i7 1260P"), known("Alder Lake", 10, BGA1744));
        assert_eq!(lookup(Intel, 6, 0x1E, 5, "Intel Core i7 Q 720"), known("Clarksfield", 45, ""));
        // The mobile Extreme models write their `X` apart, as the desktop Extremes of the time do.
        assert_eq!(lookup(Intel, 6, 0x1E, 5, "Intel Core i7 X 920"), known("Clarksfield", 45, ""));
        assert_eq!(lookup(Intel, 6, 0x1E, 5, "Intel Core i7 X 940"), known("Clarksfield", 45, ""));
        assert_eq!(lookup(Intel, 6, 0x2C, 2, "Intel Core i7 X 980"), known("Gulftown", 32, LGA1366));
        assert_eq!(lookup(Intel, 6, 0x3C, 3, "Intel Core i7 4700MQ"), known("Haswell", 22, ""));
        // Neither clearly socketed nor clearly mobile: no row applies.
        assert_eq!(lookup(Intel, 6, 0x25, 5, "Intel Pentium P6100"), None);
    }

    #[test]
    fn identifies_intel_workstation_and_server_parts() {
        assert_eq!(
            lookup(Intel, 6, 0x55, 4, "Intel Core i9 7900X"),
            known("Skylake-X", 14, "Socket 2066 LGA")
        );
        assert_eq!(lookup(Intel, 6, 0x55, 7, "Intel Core i9 10980XE"), known("Cascade Lake-X", 14, LGA2066));
        assert_eq!(
            lookup(Intel, 6, 0x55, 4, "Intel Xeon Gold 6148"),
            known("Skylake-SP", 14, "Socket 3647 LGA")
        );
        assert_eq!(lookup(Intel, 6, 0x55, 7, "Intel Xeon W-2245"), known("Cascade Lake-W", 14, LGA2066));
        assert_eq!(lookup(Intel, 6, 0x55, 7, "Intel Xeon W-3275"), known("Cascade Lake-W", 14, LGA3647));
        assert_eq!(
            lookup(Intel, 6, 0x55, 11, "Intel Xeon Platinum 8380H"),
            known("Cooper Lake", 14, LGA4189)
        );
        assert_eq!(lookup(Intel, 6, 0x55, 8, "Intel Xeon Gold 6148"), None);
        assert_eq!(lookup(Intel, 6, 0x4F, 1, "Intel Xeon E5 2680 v4"), known("Broadwell-EP", 14, LGA2011_3));
        assert_eq!(lookup(Intel, 6, 0x4F, 1, "Intel Core i7 6950X"), known("Broadwell-E", 14, LGA2011_3));
        assert_eq!(lookup(Intel, 6, 0x2D, 7, "Intel Xeon E5 2670"), known("Sandy Bridge-EP", 32, LGA2011));
        assert_eq!(lookup(Intel, 6, 0x2C, 2, "Intel Xeon X5650"), known("Westmere-EP", 32, LGA1366));
        assert_eq!(
            lookup(Intel, 6, 0x8F, 8, "Intel Xeon w9 3495X"),
            known("Sapphire Rapids-WS", 10, LGA4677)
        );
        assert_eq!(lookup(Intel, 6, 0x6A, 6, "Intel Xeon Platinum 8380"), known("Ice Lake-SP", 10, LGA4189));
    }

    #[test]
    fn unknown_processors_stay_unknown() {
        assert_eq!(lookup(Intel, 6, 0xFE, 0, "Intel Core i7 99999K"), None);
        assert_eq!(lookup(Amd, 0x1B, 0x00, 0, "AMD Ryzen 9 19950X"), None);
        assert_eq!(lookup(Amd, 6, 0x97, 2, "Intel Core i7 12700K"), None);
        assert_eq!(lookup(CpuVendor::Other, 6, 0x97, 2, "Intel Core i7 12700K"), None);
        assert_eq!(
            lookup(CpuVendor::Hygon, 0x18, 0x00, 2, "Hygon C86 7185"),
            Some(Known { codename: "Dhyana", technology_nm: Some(14), package: None })
        );
        // A node without a meaningful nm figure reports none.
        assert_eq!(lookup(Intel, 6, 0xCC, 0, "Intel Core Ultra 7 355H").and_then(|k| k.technology_nm), None);
    }

    #[test]
    fn names_core_steppings() {
        assert_eq!(revision(Amd, 0x19, 0x61, 2), Some("RPL-B2"));
        assert_eq!(revision(Amd, 0x19, 0x21, 0), Some("VMR-B0"));
        assert_eq!(revision(Amd, 0x17, 0x08, 2), Some("PiR-B2"));
        assert_eq!(revision(Intel, 6, 0x97, 2), Some("C0"));
        assert_eq!(revision(Intel, 6, 0x1A, 5), Some("D0"));
        assert_eq!(revision(Amd, 0x19, 0x61, 3), None);
        assert_eq!(revision(Intel, 0x19, 0x61, 2), None);
    }

    #[test]
    fn rates_tdp_for_exact_names_only() {
        assert_eq!(max_tdp("AMD Ryzen 7 7700"), Some(65.0));
        assert_eq!(max_tdp("AMD Ryzen 7 7700X"), Some(105.0));
        assert_eq!(max_tdp("AMD Ryzen 9 7950X3D"), Some(120.0));
        assert_eq!(max_tdp("AMD Ryzen Threadripper 3970X"), Some(280.0));
        assert_eq!(max_tdp("AMD Ryzen Threadripper PRO 5995WX"), Some(280.0));
        assert_eq!(max_tdp("AMD FX-8350"), Some(125.0));
        assert_eq!(max_tdp("Intel Core i7 13700K"), Some(125.0));
        assert_eq!(max_tdp("Intel Core i7 3970X"), Some(150.0));
        assert_eq!(max_tdp("Intel Core i7 7700"), Some(65.0));
        assert_eq!(max_tdp("Intel Core i7 920"), Some(130.0));
        assert_eq!(max_tdp("Intel Core Ultra 9 285K"), Some(125.0));
        assert_eq!(max_tdp("Intel Core 2 Duo E8500"), Some(65.0));
        assert_eq!(max_tdp("Intel Core 2 Duo E6750"), Some(65.0));
        // The first Core 2 Duo models go by the bare number their brand strings give.
        assert_eq!(max_tdp("Intel Core 2 6600"), Some(65.0));
        assert_eq!(max_tdp("Intel Core 2 4300"), Some(65.0));
        assert_eq!(max_tdp("Intel Core 2 T7200"), None);
        // Mobile parts, unlisted models and near misses are left alone.
        assert_eq!(max_tdp("AMD Ryzen 7 7840HS"), None);
        assert_eq!(max_tdp("AMD Ryzen 7 PRO 7700"), None);
        assert_eq!(max_tdp("AMD Ryzen 7 77"), None);
        assert_eq!(max_tdp("Intel Core i7 13700HX"), None);
        assert_eq!(max_tdp(""), None);
    }

    #[test]
    fn tables_are_well_formed() {
        for rule in INTEL.iter().chain(AMD).chain(HYGON) {
            assert!(rule.models.0 <= rule.models.1, "{}", rule.codename);
            assert!(rule.steppings.0 <= rule.steppings.1 && rule.steppings.1 <= 15, "{}", rule.codename);
            assert!(!rule.codename.is_empty());
        }
        for (i, row) in REVISIONS.iter().enumerate() {
            assert!(
                REVISIONS[..i].iter().all(|r| (r.0, r.1, r.2, r.3) != (row.0, row.1, row.2, row.3)),
                "{}",
                row.4
            );
        }
        // A model number may be rated only once per product line.
        for (i, (prefix, _, models)) in TDP.iter().enumerate() {
            for model in *models {
                let earlier = TDP[..i].iter().any(|(p, _, m)| p == prefix && m.contains(model));
                let repeated = models.iter().filter(|m| *m == model).count() > 1;
                assert!(!earlier && !repeated, "{prefix}{model}");
            }
        }
    }
}
