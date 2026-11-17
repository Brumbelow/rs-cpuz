//! Control geometry of the frame, the tab pages, the splash and the tool windows.
//!
//! Each table lists the controls of one dialog in dialog units, relative to
//! that dialog's client area. The renderer draws from these tables and the
//! window code hit-tests against them, so both always agree.

use super::geom::{dlu, Dlu};

/// What a table entry is and how it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// One-pixel frame around a group of controls.
    Frame,
    /// Caption of a frame, drawn over the frame's top edge.
    Caption,
    /// Right-aligned label, vertically centred on the box it describes.
    Label,
    /// Right-aligned label of a row that modern systems leave empty.
    DimLabel,
    /// Left-aligned static text.
    Text,
    /// Centred text in the label colour.
    Heading,
    /// Centred text in the value colour.
    Info,
    /// The application name in the large font.
    Title,
    /// Value box with centred text.
    Value,
    /// Value box with left-aligned text.
    ValueLeft,
    /// Value box whose text wraps over two lines.
    ValueWrap,
    /// Value box of a row that modern systems leave empty.
    DimValue,
    /// Selector: a box with a chevron that opens a list.
    Combo,
    Button,
    Check,
    /// Progress bar with a caption inside.
    Bar,
    /// Boxed logo mark.
    Mark,
    /// Logo mark without a box.
    Emblem,
}

impl Kind {
    /// Controls that react to the mouse.
    pub const fn is_interactive(self) -> bool {
        matches!(self, Kind::Combo | Kind::Button | Kind::Check)
    }

    /// Controls that own their rectangle exclusively (nothing else may overlap them).
    pub const fn is_box(self) -> bool {
        matches!(
            self,
            Kind::Value
                | Kind::ValueLeft
                | Kind::ValueWrap
                | Kind::DimValue
                | Kind::Combo
                | Kind::Button
                | Kind::Check
                | Kind::Bar
                | Kind::Mark
                | Kind::Emblem
        )
    }
}

/// Identifier of the rows nothing looks up: frames, captions and labels whose text is fixed.
pub const STATIC: u16 = 0;

/// One control of a dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    pub kind: Kind,
    /// Identifier of a control that is looked up, for its text or because it reacts
    /// to the mouse; unique on its page. Every other row is [`STATIC`].
    pub id: u16,
    pub rect: Dlu,
    /// Static text; dynamic controls leave it empty or use it as the fallback.
    pub text: &'static str,
}

const fn item(kind: Kind, id: u16, x: i32, y: i32, w: i32, h: i32, text: &'static str) -> Item {
    Item { kind, id, rect: dlu(x, y, w, h), text }
}

/// A frame that is only drawn.
const fn frame(x: i32, y: i32, w: i32, h: i32) -> Item {
    item(Kind::Frame, STATIC, x, y, w, h, "")
}

/// A caption with fixed text.
const fn caption(x: i32, y: i32, w: i32, h: i32, text: &'static str) -> Item {
    item(Kind::Caption, STATIC, x, y, w, h, text)
}

/// A label with fixed text.
const fn label(x: i32, y: i32, w: i32, h: i32, text: &'static str) -> Item {
    item(Kind::Label, STATIC, x, y, w, h, text)
}

const fn value(id: u16, x: i32, y: i32, w: i32, h: i32) -> Item {
    item(Kind::Value, id, x, y, w, h, "")
}

const fn value_left(id: u16, x: i32, y: i32, w: i32, h: i32) -> Item {
    item(Kind::ValueLeft, id, x, y, w, h, "")
}

/// Height of a closed selector.
pub const COMBO_HEIGHT: i32 = 13;
/// Tallest box that still counts as one row of text when labels are aligned to boxes.
const MAX_ROW_HEIGHT: i32 = COMBO_HEIGHT;
/// Clearance kept between a label and the box or frame edge on its left.
const LABEL_GAP: i32 = 2;

const fn combo(id: u16, x: i32, y: i32, w: i32) -> Item {
    item(Kind::Combo, id, x, y, w, COMBO_HEIGHT, "")
}

const fn button(id: u16, x: i32, y: i32, w: i32, h: i32, text: &'static str) -> Item {
    item(Kind::Button, id, x, y, w, h, text)
}

// ---------------------------------------------------------------------------
// Main frame
// ---------------------------------------------------------------------------

/// Size of the main window in dialog units.
#[cfg(test)]
pub const FRAME: (i32, i32) = (278, 258);
/// Pixel size of the main window at 96 DPI. The reference window measures 416 x 418, one
/// pixel less each way than its dialog units convert to; the right and bottom margins absorb it.
pub const FRAME_PIXELS: (i32, i32) = (416, 418);
/// Height of the custom title strip of the main window.
pub const TITLE_HEIGHT: i32 = 20;
pub const MINIMIZE_BOX: Dlu = dlu(235, 3, 11, 11);
pub const CLOSE_BOX: Dlu = dlu(253, 3, 11, 11);
/// The tab control: the tab strip followed by the page area.
pub const TAB_CONTROL: Dlu = dlu(7, 25, 263, 212);
pub const TAB_STRIP_HEIGHT: i32 = 13;
/// Top-left corner of the tab pages inside the frame.
pub const PAGE_ORIGIN: (i32, i32) = (TAB_CONTROL.x + 1, TAB_CONTROL.y + TAB_STRIP_HEIGHT);
/// Area available to a tab page.
#[cfg(test)]
pub const PAGE_AREA: (i32, i32) = (TAB_CONTROL.w - 2, TAB_CONTROL.h - TAB_STRIP_HEIGHT);
pub const VERSION_LABEL: Dlu = dlu(10, 242, 66, 8);
pub const TOOLS_BUTTON: Dlu = dlu(137, 239, 54, 12);
pub const CLOSE_BUTTON: Dlu = dlu(204, 239, 54, 12);

// The title strip ends above the tab control.
const _: () = assert!(TITLE_HEIGHT <= TAB_CONTROL.y);

/// How far the clickable area of a check box extends to the right, covering its label.
pub const CHECK_REACH: i32 = 48;

/// The pages of the tab control, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    Cpu,
    Mainboard,
    Memory,
    Spd,
    Graphics,
    Bench,
    About,
}

impl Tab {
    pub const ALL: [Tab; 7] =
        [Tab::Cpu, Tab::Mainboard, Tab::Memory, Tab::Spd, Tab::Graphics, Tab::Bench, Tab::About];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn title(self) -> &'static str {
        match self {
            Tab::Cpu => "CPU",
            Tab::Mainboard => "Mainboard",
            Tab::Memory => "Memory",
            Tab::Spd => "SPD",
            Tab::Graphics => "Graphics",
            Tab::Bench => "Bench",
            Tab::About => "About",
        }
    }

    /// File name used when the tab is saved as an image.
    pub const fn file_name(self) -> &'static str {
        match self {
            Tab::Cpu => "cpu.bmp",
            Tab::Mainboard => "mainboard.bmp",
            Tab::Memory => "memory.bmp",
            Tab::Spd => "spd.bmp",
            Tab::Graphics => "graphics.bmp",
            Tab::Bench => "bench.bmp",
            Tab::About => "about.bmp",
        }
    }

    /// Size of the page dialog.
    #[cfg(test)]
    pub const fn size(self) -> (i32, i32) {
        match self {
            Tab::Cpu | Tab::Bench => (261, 199),
            _ => (261, 192),
        }
    }

    pub const fn items(self) -> &'static [Item] {
        match self {
            Tab::Cpu => CPU,
            Tab::Mainboard => MAINBOARD,
            Tab::Memory => MEMORY,
            Tab::Spd => SPD,
            Tab::Graphics => GRAPHICS,
            Tab::Bench => BENCH,
            Tab::About => ABOUT,
        }
    }

    /// The control with identifier `id` on this page. Rows without an identifier are never found.
    pub fn item(self, id: u16) -> Option<&'static Item> {
        self.items().iter().find(|item| item.id == id && id != STATIC)
    }

    /// The frame whose top edge runs behind `caption`.
    pub fn caption_frame(self, caption: &Item) -> Option<&'static Item> {
        let c = caption.rect;
        self.items().iter().filter(|item| item.kind == Kind::Frame).find(|frame| {
            let f = frame.rect;
            (c.y..=c.bottom()).contains(&f.y) && f.x <= c.x && c.right() <= f.right()
        })
    }

    /// The single-row box a label belongs to: the one sharing most of its vertical
    /// extent, preferring the nearest to its right.
    pub fn label_partner(self, label: &Item) -> Option<&'static Item> {
        let l = label.rect;
        let overlap = |b: Dlu| l.bottom().min(b.bottom()) - l.y.max(b.y);
        let distance = |b: Dlu| if b.x >= l.right() { b.x - l.right() } else { i32::MAX };
        self.items()
            .iter()
            .filter(|item| item.kind.is_box() && item.rect.h <= MAX_ROW_HEIGHT && overlap(item.rect) > 0)
            .max_by_key(|item| (overlap(item.rect), std::cmp::Reverse(distance(item.rect))))
    }

    /// How far left the text of a right-aligned label may reach: just short of
    /// the nearest box on its row or of the frame around it, and at least to the
    /// label's own left edge.
    pub fn label_limit(self, label: &Item) -> i32 {
        let l = label.rect;
        let same_row = |r: Dlu| r.y < l.bottom() && l.y < r.bottom();
        let edge = |item: &Item| {
            let r = item.rect;
            match item.kind {
                kind if kind.is_box() && same_row(r) && r.right() <= l.right() => Some(r.right()),
                Kind::Frame if same_row(r) && r.x <= l.x && l.right() <= r.right() => Some(r.x),
                _ => None,
            }
        };
        self.items().iter().filter_map(edge).max().map_or(l.x, |edge| (edge + LABEL_GAP).min(l.x))
    }

    /// The tab `step` positions to the right (negative: to the left), wrapping around.
    pub fn cycled(self, step: i32) -> Tab {
        let count = Tab::ALL.len() as i32;
        Tab::ALL[(self.index() as i32 + step).rem_euclid(count) as usize]
    }

    /// Width of the tab in the strip.
    const fn strip_width(self) -> i32 {
        match self {
            Tab::Cpu | Tab::Spd => 29,
            Tab::Mainboard => 47,
            Tab::Memory => 39,
            Tab::Graphics => 43,
            Tab::Bench | Tab::About => 38,
        }
    }

    /// Rectangle of the tab in the strip, in frame coordinates.
    pub fn strip_rect(self) -> Dlu {
        let before: i32 = Tab::ALL[..self.index()].iter().map(|tab| tab.strip_width()).sum();
        dlu(TAB_CONTROL.x + before, TAB_CONTROL.y, self.strip_width(), TAB_STRIP_HEIGHT)
    }
}

// ---------------------------------------------------------------------------
// Control identifiers
// ---------------------------------------------------------------------------
//
// Only the controls that are looked up have one: those whose text comes from
// the hardware model or the window state, and those that react to the mouse.
// The numbers are counted per page, in the hundred that belongs to the page,
// so an identifier never resolves on a page it was not meant for.

pub mod cpu {
    pub const CLOCKS_GROUP: u16 = 101;
    pub const CLOCKS_CAPTION: u16 = 102;
    pub const NAME: u16 = 103;
    pub const CODE_NAME: u16 = 104;
    pub const MAX_TDP: u16 = 105;
    pub const LOGO: u16 = 106;
    pub const PACKAGE: u16 = 107;
    pub const TECHNOLOGY: u16 = 108;
    pub const CORE_VOLTAGE: u16 = 109;
    pub const SPECIFICATION: u16 = 110;
    pub const FAMILY: u16 = 111;
    pub const MODEL: u16 = 112;
    pub const STEPPING: u16 = 113;
    pub const EXT_FAMILY: u16 = 114;
    pub const EXT_MODEL: u16 = 115;
    pub const REVISION: u16 = 116;
    pub const INSTRUCTIONS: u16 = 117;
    pub const CORE_SPEED: u16 = 118;
    pub const MULTIPLIER: u16 = 119;
    pub const BUS_SPEED: u16 = 120;
    pub const RATED_FSB: u16 = 121;
    pub const L1_DATA: u16 = 122;
    pub const L1_DATA_WAYS: u16 = 123;
    pub const L1_INST: u16 = 124;
    pub const L1_INST_WAYS: u16 = 125;
    pub const LEVEL2: u16 = 126;
    pub const LEVEL2_WAYS: u16 = 127;
    pub const LEVEL3: u16 = 128;
    pub const LEVEL3_WAYS: u16 = 129;
    pub const SELECTION: u16 = 130;
    pub const CORES: u16 = 131;
    pub const THREADS: u16 = 132;
}

pub mod mainboard {
    pub const MANUFACTURER: u16 = 201;
    pub const MODEL: u16 = 202;
    pub const VERSION: u16 = 203;
    pub const BUS_SPECS: u16 = 204;
    pub const CHIPSET_VENDOR: u16 = 205;
    pub const CHIPSET_MODEL: u16 = 206;
    pub const CHIPSET_REVISION: u16 = 207;
    pub const SOUTHBRIDGE_VENDOR: u16 = 208;
    pub const SOUTHBRIDGE_MODEL: u16 = 209;
    pub const SOUTHBRIDGE_REVISION: u16 = 210;
    pub const LPCIO_VENDOR: u16 = 211;
    pub const LPCIO_MODEL: u16 = 212;
    pub const BIOS_BRAND: u16 = 213;
    pub const BIOS_VERSION: u16 = 214;
    pub const BIOS_DATE: u16 = 215;
    pub const MICROCODE: u16 = 216;
    pub const GI_BUS: u16 = 217;
    pub const GI_WIDTH: u16 = 218;
    pub const GI_MAX_WIDTH: u16 = 219;
    pub const GI_SPEED: u16 = 220;
    pub const GI_MAX_SPEED: u16 = 221;
}

pub mod memory {
    pub const TYPE: u16 = 301;
    pub const SIZE: u16 = 302;
    pub const CHANNELS: u16 = 303;
    pub const DC_MODE: u16 = 304;
    pub const UNCORE_FREQUENCY: u16 = 305;
    pub const DRAM_FREQUENCY: u16 = 306;
    pub const FSB_DRAM: u16 = 307;
    pub const CAS_LATENCY: u16 = 308;
    pub const TRCD: u16 = 309;
    pub const TRP: u16 = 310;
    pub const TRAS: u16 = 311;
    pub const TRC: u16 = 312;
    pub const COMMAND_RATE: u16 = 313;
}

pub mod spd {
    pub const SLOT: u16 = 401;
    pub const MODULE_TYPE: u16 = 402;
    pub const MODULE_SIZE: u16 = 403;
    pub const MAX_BANDWIDTH: u16 = 404;
    pub const SPD_EXT: u16 = 405;
    pub const MODULE_MANUFACTURER: u16 = 406;
    pub const WEEK_YEAR: u16 = 407;
    pub const DRAM_MANUFACTURER: u16 = 408;
    pub const BUFFERED: u16 = 409;
    pub const PART_NUMBER: u16 = 410;
    pub const CORRECTION: u16 = 411;
    pub const SERIAL_NUMBER: u16 = 412;
    pub const REGISTERED: u16 = 413;
    /// Column headers of the timings table.
    pub const PROFILES: [u16; 4] = [414, 415, 416, 417];
    /// Value boxes of the timings table, one row per timing, one column per profile.
    pub const TABLE: [[u16; 4]; 8] = [
        [418, 419, 420, 421],
        [422, 423, 424, 425],
        [426, 427, 428, 429],
        [430, 431, 432, 433],
        [434, 435, 436, 437],
        [438, 439, 440, 441],
        [442, 443, 444, 445],
        [446, 447, 448, 449],
    ];
}

pub mod graphics {
    pub const ADAPTER: u16 = 501;
    pub const PERF_LEVEL: u16 = 502;
    pub const NAME: u16 = 503;
    pub const LOGO: u16 = 504;
    pub const BOARD_MANUFACTURER: u16 = 505;
    pub const CODE_NAME: u16 = 506;
    pub const REVISION: u16 = 507;
    pub const TECHNOLOGY: u16 = 508;
    pub const TDP: u16 = 509;
    pub const ROPS_TMUS: u16 = 510;
    pub const CORES: u16 = 511;
    pub const CORE_CLOCK: u16 = 512;
    pub const SHADER_CLOCK: u16 = 513;
    pub const MEMORY_CLOCK: u16 = 514;
    pub const MEMORY_SIZE: u16 = 515;
    pub const MEMORY_TYPE: u16 = 516;
    pub const MEMORY_VENDOR: u16 = 517;
    pub const MEMORY_BUS_WIDTH: u16 = 518;
}

pub mod bench {
    pub const SINGLE_BAR: u16 = 601;
    pub const SINGLE_REFERENCE_BAR: u16 = 602;
    pub const MULTI_BAR: u16 = 603;
    pub const MULTI_REFERENCE_BAR: u16 = 604;
    pub const SINGLE_REFERENCE_CHECK: u16 = 605;
    pub const MULTI_REFERENCE_CHECK: u16 = 606;
    pub const THREADS: u16 = 607;
    pub const RATIO: u16 = 608;
    pub const BENCH_BUTTON: u16 = 609;
    pub const STRESS_BUTTON: u16 = 610;
    pub const SAVE_BUTTON: u16 = 611;
    pub const PROCESSOR: u16 = 612;
    pub const REFERENCE: u16 = 613;
    pub const VERSION: u16 = 614;
}

pub mod about {
    pub const EMBLEM: u16 = 701;
    pub const NAME: u16 = 702;
    pub const VERSION: u16 = 703;
    pub const DESCRIPTION: u16 = 704;
    pub const LICENSE: u16 = 705;
    pub const OS_NAME: u16 = 706;
    pub const OS_BUILD: u16 = 707;
    pub const DIRECTX: u16 = 708;
    pub const SAVE_TXT: u16 = 709;
    pub const SAVE_HTML: u16 = 710;
    pub const CLOCKS: u16 = 711;
    pub const TIMERS: u16 = 712;
}

// ---------------------------------------------------------------------------
// Tab pages
// ---------------------------------------------------------------------------

const CPU: &[Item] = &[
    frame(2, 5, 257, 106),
    item(Kind::Frame, cpu::CLOCKS_GROUP, 2, 116, 119, 54, ""),
    frame(123, 116, 136, 54),
    frame(2, 173, 257, 21),
    caption(8, 2, 37, 9, "Processor"),
    item(Kind::Caption, cpu::CLOCKS_CAPTION, 8, 113, 60, 9, "Clocks (Core #0)"),
    caption(129, 113, 37, 9, "Caches"),
    label(9, 11, 39, 9, "Name"),
    value(cpu::NAME, 52, 10, 140, 10),
    label(5, 22, 43, 9, "Code Name"),
    value(cpu::CODE_NAME, 52, 21, 68, 10),
    label(123, 22, 32, 9, "Max TDP"),
    value(cpu::MAX_TDP, 158, 21, 34, 10),
    item(Kind::Mark, cpu::LOGO, 196, 9, 60, 43, ""),
    label(7, 33, 41, 9, "Package"),
    value(cpu::PACKAGE, 52, 32, 140, 10),
    label(8, 44, 40, 9, "Technology"),
    value(cpu::TECHNOLOGY, 52, 43, 38, 10),
    label(98, 44, 42, 9, "Core Voltage"),
    value(cpu::CORE_VOLTAGE, 144, 43, 48, 10),
    label(5, 59, 43, 9, "Specification"),
    value(cpu::SPECIFICATION, 52, 58, 203, 10),
    label(10, 70, 38, 9, "Family"),
    value(cpu::FAMILY, 52, 69, 35, 10),
    label(96, 70, 36, 9, "Model"),
    value(cpu::MODEL, 137, 69, 35, 10),
    label(178, 70, 35, 9, "Stepping"),
    value(cpu::STEPPING, 220, 69, 35, 10),
    label(8, 81, 40, 9, "Ext. Family"),
    value(cpu::EXT_FAMILY, 52, 80, 35, 10),
    label(95, 80, 37, 9, "Ext. Model"),
    value(cpu::EXT_MODEL, 137, 80, 35, 10),
    label(180, 81, 33, 9, "Revision"),
    value(cpu::REVISION, 220, 80, 35, 10),
    label(8, 92, 40, 9, "Instructions"),
    item(Kind::ValueWrap, cpu::INSTRUCTIONS, 52, 91, 203, 17, ""),
    label(4, 125, 43, 9, "Core Speed"),
    value(cpu::CORE_SPEED, 51, 124, 67, 10),
    label(4, 136, 43, 9, "Multiplier"),
    value(cpu::MULTIPLIER, 51, 135, 67, 10),
    label(4, 147, 43, 9, "Bus Speed"),
    value(cpu::BUS_SPEED, 51, 146, 67, 10),
    label(4, 157, 43, 9, "Rated FSB"),
    value(cpu::RATED_FSB, 51, 157, 67, 10),
    label(125, 125, 27, 9, "L1 Data"),
    value(cpu::L1_DATA, 157, 124, 63, 10),
    value(cpu::L1_DATA_WAYS, 221, 124, 34, 10),
    label(125, 136, 27, 9, "L1 Inst."),
    value(cpu::L1_INST, 157, 135, 63, 10),
    value(cpu::L1_INST_WAYS, 221, 135, 34, 10),
    label(125, 147, 27, 9, "Level 2"),
    value(cpu::LEVEL2, 157, 146, 63, 10),
    value(cpu::LEVEL2_WAYS, 221, 146, 34, 10),
    label(125, 157, 27, 9, "Level 3"),
    value(cpu::LEVEL3, 157, 157, 63, 10),
    value(cpu::LEVEL3_WAYS, 221, 157, 34, 10),
    label(8, 180, 38, 9, "Selection"),
    combo(cpu::SELECTION, 52, 177, 63),
    label(118, 180, 23, 9, "Cores"),
    value(cpu::CORES, 146, 179, 43, 10),
    label(192, 180, 30, 9, "Threads"),
    value(cpu::THREADS, 225, 179, 28, 10),
];

const MAINBOARD: &[Item] = &[
    frame(2, 7, 257, 81),
    frame(2, 96, 257, 41),
    frame(2, 146, 257, 44),
    caption(7, 3, 52, 9, "Motherboard"),
    caption(6, 92, 26, 9, "BIOS"),
    caption(7, 142, 61, 9, "Graphic Interface"),
    label(6, 14, 43, 9, "Manufacturer"),
    value_left(mainboard::MANUFACTURER, 54, 13, 197, 10),
    label(10, 25, 39, 9, "Model"),
    value_left(mainboard::MODEL, 54, 24, 126, 10),
    value(mainboard::VERSION, 181, 24, 70, 10),
    label(10, 36, 39, 9, "Bus Specs."),
    value_left(mainboard::BUS_SPECS, 54, 35, 197, 10),
    label(10, 52, 39, 9, "Chipset"),
    value(mainboard::CHIPSET_VENDOR, 54, 51, 46, 10),
    value(mainboard::CHIPSET_MODEL, 101, 51, 104, 10),
    label(210, 52, 15, 9, "Rev."),
    value(mainboard::CHIPSET_REVISION, 227, 51, 24, 10),
    label(6, 63, 43, 9, "Southbridge"),
    value(mainboard::SOUTHBRIDGE_VENDOR, 54, 62, 46, 10),
    value(mainboard::SOUTHBRIDGE_MODEL, 101, 62, 104, 10),
    label(210, 63, 15, 9, "Rev."),
    value(mainboard::SOUTHBRIDGE_REVISION, 227, 62, 24, 10),
    label(12, 76, 37, 9, "LPCIO"),
    value(mainboard::LPCIO_VENDOR, 54, 75, 46, 10),
    value(mainboard::LPCIO_MODEL, 101, 75, 104, 10),
    label(13, 102, 36, 9, "Brand"),
    value_left(mainboard::BIOS_BRAND, 54, 101, 197, 10),
    label(13, 113, 36, 9, "Version"),
    value_left(mainboard::BIOS_VERSION, 54, 112, 197, 10),
    label(13, 124, 36, 9, "Date"),
    value_left(mainboard::BIOS_DATE, 54, 123, 91, 10),
    label(150, 124, 51, 9, "CPU Microcode"),
    value(mainboard::MICROCODE, 206, 123, 45, 10),
    label(38, 153, 37, 9, "Bus"),
    value(mainboard::GI_BUS, 81, 152, 169, 10),
    label(10, 164, 65, 9, "Current Link Width"),
    value(mainboard::GI_WIDTH, 81, 163, 56, 10),
    label(138, 164, 55, 9, "Max. Supported"),
    value(mainboard::GI_MAX_WIDTH, 197, 163, 53, 10),
    label(5, 175, 70, 10, "Current Link Speed"),
    value(mainboard::GI_SPEED, 81, 174, 56, 10),
    label(138, 175, 55, 9, "Max. Supported"),
    value(mainboard::GI_MAX_SPEED, 197, 174, 53, 10),
];

const MEMORY: &[Item] = &[
    frame(0, 4, 257, 44),
    frame(0, 55, 197, 130),
    caption(6, 0, 35, 9, "General"),
    caption(8, 50, 34, 9, "Timings"),
    label(4, 12, 25, 9, "Type"),
    value(memory::TYPE, 36, 11, 67, 10),
    label(115, 12, 64, 9, "Channel #"),
    value(memory::CHANNELS, 184, 11, 67, 10),
    label(7, 23, 22, 9, "Size"),
    value(memory::SIZE, 36, 22, 67, 10),
    label(111, 23, 68, 9, "DC Mode"),
    value(memory::DC_MODE, 184, 22, 67, 10),
    label(115, 34, 64, 9, "Uncore Frequency"),
    value(memory::UNCORE_FREQUENCY, 184, 33, 67, 10),
    label(8, 61, 109, 9, "DRAM Frequency"),
    value(memory::DRAM_FREQUENCY, 125, 60, 57, 10),
    label(8, 72, 109, 9, "FSB:DRAM"),
    value(memory::FSB_DRAM, 125, 71, 57, 10),
    label(8, 83, 109, 9, "CAS# Latency (CL)"),
    value(memory::CAS_LATENCY, 125, 82, 57, 10),
    label(8, 94, 109, 9, "RAS# to CAS# Delay (tRCD)"),
    value(memory::TRCD, 125, 93, 57, 10),
    label(8, 105, 109, 9, "RAS# Precharge (tRP)"),
    value(memory::TRP, 125, 104, 57, 10),
    label(8, 116, 109, 9, "Cycle Time (tRAS)"),
    value(memory::TRAS, 125, 115, 57, 10),
    label(8, 127, 109, 9, "Bank Cycle Time (tRC)"),
    value(memory::TRC, 125, 126, 57, 10),
    label(8, 138, 109, 9, "Command Rate (CR)"),
    value(memory::COMMAND_RATE, 125, 137, 57, 10),
    item(Kind::DimLabel, STATIC, 8, 149, 109, 9, "DRAM Idle Timer"),
    item(Kind::DimValue, STATIC, 125, 148, 57, 10, ""),
    item(Kind::DimLabel, STATIC, 8, 160, 109, 9, "Total CAS# (tRDRAM)"),
    item(Kind::DimValue, STATIC, 125, 159, 57, 10, ""),
    item(Kind::DimLabel, STATIC, 8, 171, 109, 9, "Row To Column (tRCD)"),
    item(Kind::DimValue, STATIC, 125, 170, 57, 10, ""),
];

const SPD: &[Item] = &[
    frame(2, 5, 257, 79),
    frame(2, 89, 257, 101),
    caption(8, 1, 80, 9, "Memory Slot Selection"),
    caption(8, 85, 51, 9, "Timings Table"),
    combo(spd::SLOT, 7, 11, 58),
    value_left(spd::MODULE_TYPE, 70, 14, 84, 10),
    label(158, 16, 41, 9, "Module Size"),
    value(spd::MODULE_SIZE, 204, 14, 51, 10),
    label(4, 27, 60, 9, "Max Bandwidth"),
    value(spd::MAX_BANDWIDTH, 70, 26, 84, 10),
    label(158, 27, 41, 9, "SPD Ext."),
    value(spd::SPD_EXT, 204, 26, 51, 10),
    label(16, 38, 48, 9, "Module Manuf."),
    value(spd::MODULE_MANUFACTURER, 70, 37, 84, 10),
    label(158, 38, 41, 9, "Week/Year"),
    value(spd::WEEK_YEAR, 204, 37, 51, 10),
    label(16, 49, 48, 9, "DRAM Manuf."),
    value(spd::DRAM_MANUFACTURER, 70, 48, 84, 10),
    label(158, 49, 41, 9, "Buffered"),
    value(spd::BUFFERED, 204, 48, 51, 10),
    label(15, 60, 49, 9, "Part Number"),
    value(spd::PART_NUMBER, 70, 59, 84, 10),
    label(158, 60, 41, 9, "Correction"),
    value(spd::CORRECTION, 204, 59, 51, 10),
    label(8, 71, 56, 9, "Serial Number"),
    value(spd::SERIAL_NUMBER, 70, 70, 84, 10),
    label(158, 71, 41, 9, "Registered"),
    value(spd::REGISTERED, 204, 70, 51, 10),
    item(Kind::Heading, spd::PROFILES[0], 70, 90, 42, 9, ""),
    item(Kind::Heading, spd::PROFILES[1], 113, 90, 42, 9, ""),
    item(Kind::Heading, spd::PROFILES[2], 157, 90, 42, 9, ""),
    item(Kind::Heading, spd::PROFILES[3], 201, 90, 42, 9, ""),
    label(16, 101, 48, 9, "Frequency"),
    value(spd::TABLE[0][0], 70, 99, 42, 10),
    value(spd::TABLE[0][1], 113, 99, 42, 10),
    value(spd::TABLE[0][2], 157, 99, 42, 10),
    value(spd::TABLE[0][3], 201, 99, 42, 10),
    label(8, 112, 56, 9, "CAS# Latency"),
    value(spd::TABLE[1][0], 70, 111, 42, 10),
    value(spd::TABLE[1][1], 113, 111, 42, 10),
    value(spd::TABLE[1][2], 157, 111, 42, 10),
    value(spd::TABLE[1][3], 201, 111, 42, 10),
    label(8, 123, 56, 9, "RAS# to CAS#"),
    value(spd::TABLE[2][0], 70, 122, 42, 10),
    value(spd::TABLE[2][1], 113, 122, 42, 10),
    value(spd::TABLE[2][2], 157, 122, 42, 10),
    value(spd::TABLE[2][3], 201, 122, 42, 10),
    label(8, 134, 56, 9, "RAS# Precharge"),
    value(spd::TABLE[3][0], 70, 133, 42, 10),
    value(spd::TABLE[3][1], 113, 133, 42, 10),
    value(spd::TABLE[3][2], 157, 133, 42, 10),
    value(spd::TABLE[3][3], 201, 133, 42, 10),
    label(19, 145, 45, 9, "tRAS"),
    value(spd::TABLE[4][0], 70, 144, 42, 10),
    value(spd::TABLE[4][1], 113, 144, 42, 10),
    value(spd::TABLE[4][2], 157, 144, 42, 10),
    value(spd::TABLE[4][3], 201, 144, 42, 10),
    label(19, 156, 45, 9, "tRC"),
    value(spd::TABLE[5][0], 70, 155, 42, 10),
    value(spd::TABLE[5][1], 113, 155, 42, 10),
    value(spd::TABLE[5][2], 157, 155, 42, 10),
    value(spd::TABLE[5][3], 201, 155, 42, 10),
    label(5, 167, 59, 9, "Command Rate"),
    value(spd::TABLE[6][0], 70, 166, 42, 10),
    value(spd::TABLE[6][1], 113, 166, 42, 10),
    value(spd::TABLE[6][2], 157, 166, 42, 10),
    value(spd::TABLE[6][3], 201, 166, 42, 10),
    label(26, 178, 38, 9, "Voltage"),
    value(spd::TABLE[7][0], 70, 177, 42, 10),
    value(spd::TABLE[7][1], 113, 177, 42, 10),
    value(spd::TABLE[7][2], 157, 177, 42, 10),
    value(spd::TABLE[7][3], 201, 177, 42, 10),
];

const GRAPHICS: &[Item] = &[
    frame(2, 5, 257, 26),
    frame(2, 37, 257, 63),
    frame(2, 107, 123, 41),
    frame(130, 107, 129, 51),
    caption(7, 1, 84, 9, "Display Device Selection"),
    caption(7, 33, 24, 9, "GPU"),
    caption(9, 103, 26, 9, "Clocks"),
    caption(137, 103, 34, 9, "Memory"),
    combo(graphics::ADAPTER, 9, 12, 133),
    label(145, 14, 37, 9, "Perf Level"),
    combo(graphics::PERF_LEVEL, 187, 12, 66),
    label(3, 43, 48, 9, "Name"),
    value(graphics::NAME, 55, 42, 146, 10),
    // The logo box fills the free column beside the five value rows.
    item(Kind::Mark, graphics::LOGO, 206, 42, 47, 54, ""),
    label(3, 54, 48, 9, "Board Manuf."),
    value(graphics::BOARD_MANUFACTURER, 55, 53, 146, 10),
    label(8, 65, 43, 9, "Code Name"),
    value(graphics::CODE_NAME, 55, 64, 65, 10),
    label(130, 65, 27, 9, "Revision"),
    value(graphics::REVISION, 162, 64, 39, 10),
    label(11, 76, 40, 9, "Technology"),
    value(graphics::TECHNOLOGY, 55, 75, 65, 10),
    label(131, 76, 27, 9, "TDP"),
    value(graphics::TDP, 162, 75, 39, 10),
    label(6, 87, 45, 9, "ROPs / TMUs"),
    value(graphics::ROPS_TMUS, 55, 86, 65, 10),
    label(131, 87, 27, 9, "Cores"),
    value(graphics::CORES, 162, 86, 39, 10),
    label(4, 113, 47, 9, "GFX Core"),
    value(graphics::CORE_CLOCK, 58, 112, 60, 10),
    label(4, 124, 47, 9, "Shaders / SoC"),
    value(graphics::SHADER_CLOCK, 58, 123, 60, 10),
    label(4, 135, 47, 9, "Memory"),
    value(graphics::MEMORY_CLOCK, 58, 134, 60, 10),
    label(135, 113, 43, 9, "Size"),
    value(graphics::MEMORY_SIZE, 185, 112, 64, 10),
    label(135, 124, 43, 9, "Type"),
    value(graphics::MEMORY_TYPE, 185, 123, 64, 10),
    label(135, 135, 43, 9, "Vendor"),
    value(graphics::MEMORY_VENDOR, 185, 134, 64, 10),
    label(135, 146, 43, 9, "Bus Width"),
    value(graphics::MEMORY_BUS_WIDTH, 185, 145, 64, 10),
];

const BENCH: &[Item] = &[
    frame(2, 5, 257, 39),
    frame(2, 48, 257, 37),
    frame(2, 89, 257, 46),
    frame(2, 139, 257, 38),
    caption(10, 2, 62, 9, "CPU Single Thread"),
    caption(10, 45, 62, 9, "CPU Multi Thread"),
    label(6, 17, 54, 9, "This Processor"),
    item(Kind::Bar, bench::SINGLE_BAR, 65, 17, 185, 10, ""),
    item(Kind::Check, bench::SINGLE_REFERENCE_CHECK, 13, 30, 8, 8, ""),
    item(Kind::Text, STATIC, 25, 28, 36, 9, "Reference"),
    item(Kind::Bar, bench::SINGLE_REFERENCE_BAR, 65, 28, 185, 10, ""),
    label(6, 57, 54, 9, "This Processor"),
    item(Kind::Bar, bench::MULTI_BAR, 65, 57, 185, 10, ""),
    item(Kind::Check, bench::MULTI_REFERENCE_CHECK, 13, 70, 8, 8, ""),
    item(Kind::Text, STATIC, 25, 68, 36, 9, "Reference"),
    item(Kind::Bar, bench::MULTI_REFERENCE_BAR, 65, 68, 185, 10, ""),
    label(11, 95, 36, 10, "Run on"),
    combo(bench::THREADS, 52, 93, 70),
    label(125, 95, 63, 10, "Multi Thread Ratio"),
    value(bench::RATIO, 194, 95, 56, 10),
    button(bench::BENCH_BUTTON, 8, 115, 68, 13, "Bench CPU"),
    button(bench::STRESS_BUTTON, 78, 115, 68, 13, "Stress CPU"),
    button(bench::SAVE_BUTTON, 148, 115, 107, 13, "Save as Reference"),
    label(3, 147, 50, 10, "This Processor"),
    value(bench::PROCESSOR, 58, 146, 192, 10),
    label(13, 160, 40, 9, "Reference"),
    combo(bench::REFERENCE, 58, 159, 192),
    item(Kind::Label, bench::VERSION, 101, 187, 153, 9, ""),
];

const ABOUT: &[Item] = &[
    frame(2, 5, 257, 40),
    frame(2, 48, 257, 40),
    frame(2, 95, 257, 39),
    frame(2, 141, 257, 49),
    caption(5, 1, 60, 9, "About RS-CPUZ"),
    caption(5, 90, 65, 9, "Windows Version"),
    caption(6, 136, 30, 9, "Tools"),
    item(Kind::Emblem, about::EMBLEM, 14, 11, 29, 28, ""),
    item(Kind::Title, about::NAME, 50, 11, 160, 16, ""),
    item(Kind::Text, about::VERSION, 51, 28, 160, 9, ""),
    item(Kind::Info, about::DESCRIPTION, 16, 58, 229, 9, "Open-source system profiler for Windows"),
    item(Kind::Heading, about::LICENSE, 16, 70, 229, 9, ""),
    item(Kind::Info, about::OS_NAME, 16, 99, 229, 9, ""),
    item(Kind::Info, about::OS_BUILD, 16, 110, 229, 9, ""),
    item(Kind::Info, about::DIRECTX, 90, 121, 82, 9, ""),
    button(about::SAVE_TXT, 24, 152, 101, 13, "Save Report (.TXT)"),
    button(about::SAVE_HTML, 24, 166, 101, 13, "Save Report (.HTML)"),
    button(about::CLOCKS, 139, 152, 101, 13, "Clocks"),
    button(about::TIMERS, 139, 166, 101, 13, "Timers"),
];

// ---------------------------------------------------------------------------
// Splash and tool windows
// ---------------------------------------------------------------------------

pub mod splash {
    use super::{dlu, Dlu};

    pub const SIZE: (i32, i32) = (160, 85);
    /// The logo mark and the name.
    pub const ARTWORK: Dlu = dlu(7, 8, 143, 45);
    pub const PROGRESS: Dlu = dlu(4, 57, 152, 10);
    pub const STAGE: Dlu = dlu(5, 71, 80, 10);
    pub const VERSION: Dlu = dlu(92, 71, 64, 10);
}

/// Height of the custom title strip of the tool windows.
pub const TOOL_TITLE_HEIGHT: i32 = 14;

/// Close box in the title strip of a tool window that is `width` units wide.
pub const fn tool_close_box(width: i32) -> Dlu {
    dlu(width - 15, 2, 11, 10)
}

pub mod clocks_tool {
    use super::{dlu, Dlu};

    /// Client size below the title strip.
    pub const SIZE: (i32, i32) = (275, 215);
    /// The table fills the dialog above the button row.
    pub const TABLE: Dlu = dlu(3, 3, 269, 190);
    pub const OK: Dlu = dlu(221, 197, 50, 14);
    pub const HEADER_HEIGHT: i32 = 11;
    pub const ROW_HEIGHT: i32 = 10;
    /// Column titles and widths; the widths add up to the table width.
    pub const COLUMNS: [(&str, i32); 3] = [("Device", 113), ("Clock Speed", 82), ("Multiplier", 74)];

    /// Number of data rows that fit below the header.
    pub const fn visible_rows() -> usize {
        ((TABLE.h - HEADER_HEIGHT - 1) / ROW_HEIGHT) as usize
    }
}

pub mod timers_tool {
    use super::{dlu, Dlu};

    /// Client size below the title strip.
    pub const SIZE: (i32, i32) = (195, 78);
    pub const HEADINGS: [Dlu; 4] =
        [dlu(8, 8, 44, 9), dlu(53, 8, 44, 9), dlu(98, 8, 44, 9), dlu(143, 8, 44, 9)];
    pub const ELAPSED: [Dlu; 4] =
        [dlu(8, 22, 44, 10), dlu(53, 22, 44, 10), dlu(98, 22, 44, 10), dlu(143, 22, 44, 10)];
    pub const DRIFT: [Dlu; 4] =
        [dlu(8, 33, 44, 10), dlu(53, 33, 44, 10), dlu(98, 33, 44, 10), dlu(143, 33, 44, 10)];
    pub const START: Dlu = dlu(4, 62, 50, 13);
    pub const RESET: Dlu = dlu(55, 62, 50, 13);
    pub const CLOSE: Dlu = dlu(141, 62, 50, 13);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_fit_the_tab_control() {
        for tab in Tab::ALL {
            let (w, h) = tab.size();
            assert!(w <= PAGE_AREA.0 && h <= PAGE_AREA.1, "{tab:?}");
        }
        assert_eq!(PAGE_ORIGIN, (8, 38));
        assert!(TAB_CONTROL.bottom() < TOOLS_BUTTON.y);
    }

    #[test]
    fn no_item_leaves_its_page() {
        for tab in Tab::ALL {
            let (w, h) = tab.size();
            let page = dlu(0, 0, w, h);
            for item in tab.items() {
                assert!(page.contains(item.rect), "{tab:?}: {item:?}");
                assert!(item.rect.w > 0 && item.rect.h > 0, "{tab:?}: {item:?}");
            }
        }
    }

    #[test]
    fn boxes_do_not_overlap() {
        for tab in Tab::ALL {
            let boxes: Vec<&Item> = tab.items().iter().filter(|item| item.kind.is_box()).collect();
            for (i, a) in boxes.iter().enumerate() {
                for b in &boxes[i + 1..] {
                    assert!(!a.rect.intersects(b.rect), "{tab:?}: {a:?} overlaps {b:?}");
                }
            }
        }
    }

    #[test]
    fn labels_do_not_run_into_boxes() {
        for tab in Tab::ALL {
            let items = tab.items();
            for text in items.iter().filter(|i| matches!(i.kind, Kind::Label | Kind::DimLabel | Kind::Text)) {
                for other in items.iter().filter(|i| i.kind.is_box()) {
                    assert!(!text.rect.intersects(other.rect), "{tab:?}: {:?} overlaps {other:?}", text.text);
                }
            }
        }
    }

    #[test]
    fn identifiers_are_unique_and_belong_to_their_page() {
        for tab in Tab::ALL {
            let mut ids: Vec<u16> =
                tab.items().iter().map(|item| item.id).filter(|&id| id != STATIC).collect();
            let first = 100 * (tab.index() as u16 + 1) + 1;
            assert!(ids.iter().all(|id| (first..first + 99).contains(id)), "{tab:?}: {ids:?}");
            let count = ids.len();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), count, "{tab:?}");
        }
    }

    #[test]
    fn controls_that_are_looked_up_have_an_identifier() {
        for tab in Tab::ALL {
            // Whatever shows a value or reacts to the mouse is found through its identifier.
            for item in tab.items().iter().filter(|item| item.kind.is_box() && item.kind != Kind::DimValue) {
                assert_ne!(item.id, STATIC, "{tab:?}: {item:?}");
                assert_eq!(tab.item(item.id), Some(item));
            }
            // The other rows are drawn as the table has them: empty boxes and frames, fixed text.
            for item in tab.items().iter().filter(|item| item.id == STATIC) {
                let blank = matches!(item.kind, Kind::Frame | Kind::DimValue);
                assert_eq!(item.text.is_empty(), blank, "{tab:?}: {item:?}");
            }
            assert_eq!(tab.item(STATIC), None, "{tab:?}");
        }
    }

    #[test]
    fn every_group_has_a_frame_under_its_caption() {
        for tab in Tab::ALL {
            for cap in tab.items().iter().filter(|i| i.kind == Kind::Caption) {
                let frame = tab.caption_frame(cap);
                assert!(frame.is_some(), "{tab:?}: caption {:?} has no frame edge", cap.text);
            }
        }
        let clocks = Tab::Cpu.item(cpu::CLOCKS_CAPTION).unwrap();
        assert_eq!(Tab::Cpu.caption_frame(clocks).map(|f| f.id), Some(cpu::CLOCKS_GROUP));
    }

    #[test]
    fn labels_find_the_box_of_their_row() {
        let partner = |tab: Tab, text: &str| {
            let label = tab.items().iter().find(|i| i.text == text).unwrap();
            tab.label_partner(label).map(|b| b.id)
        };
        assert_eq!(partner(Tab::Cpu, "Max TDP"), Some(cpu::MAX_TDP));
        assert_eq!(partner(Tab::Cpu, "Ext. Model"), Some(cpu::EXT_MODEL));
        assert_eq!(partner(Tab::Cpu, "Selection"), Some(cpu::SELECTION));
        assert_eq!(
            partner(Tab::Cpu, "Instructions"),
            None,
            "two-line boxes keep the label on the first line"
        );
        assert_eq!(partner(Tab::Mainboard, "CPU Microcode"), Some(mainboard::MICROCODE));
        assert_eq!(partner(Tab::Spd, "Module Size"), Some(spd::MODULE_SIZE));
        assert_eq!(partner(Tab::Spd, "Frequency"), Some(spd::TABLE[0][0]));
        assert_eq!(partner(Tab::Bench, "Run on"), Some(bench::THREADS));
        assert_eq!(partner(Tab::Bench, "Multi Thread Ratio"), Some(bench::RATIO));
        for tab in Tab::ALL {
            for label in tab.items().iter().filter(|i| matches!(i.kind, Kind::Label | Kind::DimLabel)) {
                if label.text != "Instructions" && !label.text.is_empty() {
                    assert!(tab.label_partner(label).is_some(), "{tab:?}: {:?}", label.text);
                }
            }
        }
    }

    #[test]
    fn labels_may_grow_into_free_space_only() {
        let limit = |tab: Tab, text: &str| {
            let label = tab.items().iter().find(|i| i.text == text).unwrap();
            tab.label_limit(label)
        };
        assert_eq!(limit(Tab::Cpu, "Core Voltage"), 92, "stops short of the Technology box");
        assert_eq!(limit(Tab::Cpu, "Specification"), 4, "stops short of the group frame");
        assert_eq!(limit(Tab::Cpu, "Max TDP"), 122);
        assert_eq!(limit(Tab::Mainboard, "CPU Microcode"), 147);
        assert_eq!(limit(Tab::Memory, "Type"), 2);
        for tab in Tab::ALL {
            for label in tab.items().iter().filter(|i| matches!(i.kind, Kind::Label | Kind::DimLabel)) {
                let limit = tab.label_limit(label);
                assert!(limit <= label.rect.x && limit >= 0, "{tab:?}: {:?}", label.text);
            }
        }
    }

    #[test]
    fn tab_strip_covers_the_tab_control() {
        let mut x = TAB_CONTROL.x;
        for tab in Tab::ALL {
            let r = tab.strip_rect();
            assert_eq!(r.x, x, "{tab:?}");
            assert_eq!((r.y, r.h), (TAB_CONTROL.y, TAB_STRIP_HEIGHT));
            x = r.right();
        }
        assert_eq!(x, TAB_CONTROL.right());
    }

    #[test]
    fn tabs_cycle_in_both_directions() {
        assert_eq!(Tab::Cpu.cycled(1), Tab::Mainboard);
        assert_eq!(Tab::About.cycled(1), Tab::Cpu);
        assert_eq!(Tab::Cpu.cycled(-1), Tab::About);
        assert_eq!(Tab::Spd.cycled(-8), Tab::Memory);
        for (i, tab) in Tab::ALL.iter().enumerate() {
            assert_eq!(tab.index(), i);
        }
    }

    #[test]
    fn file_names_match_the_tab_titles() {
        for tab in Tab::ALL {
            assert_eq!(tab.file_name(), format!("{}.bmp", tab.title().to_lowercase()));
        }
    }

    #[test]
    fn spd_table_ids_exist_on_the_page() {
        for row in spd::TABLE {
            for id in row {
                assert_eq!(Tab::Spd.item(id).map(|i| i.kind), Some(Kind::Value));
            }
        }
        for id in spd::PROFILES {
            assert_eq!(Tab::Spd.item(id).map(|i| i.kind), Some(Kind::Heading));
        }
    }

    #[test]
    fn frame_controls_stay_inside_the_frame() {
        let window = dlu(0, 0, FRAME.0, FRAME.1);
        for r in [MINIMIZE_BOX, CLOSE_BOX, TAB_CONTROL, VERSION_LABEL, TOOLS_BUTTON, CLOSE_BUTTON] {
            assert!(window.contains(r), "{r:?}");
        }
        assert!(MINIMIZE_BOX.bottom() <= TITLE_HEIGHT && CLOSE_BOX.bottom() <= TITLE_HEIGHT);
    }

    #[test]
    fn tool_layouts_are_consistent() {
        let clocks = dlu(0, 0, clocks_tool::SIZE.0, clocks_tool::SIZE.1);
        assert!(clocks.contains(clocks_tool::TABLE) && clocks.contains(clocks_tool::OK));
        assert!(!clocks_tool::TABLE.intersects(clocks_tool::OK));
        assert_eq!(clocks_tool::COLUMNS.iter().map(|c| c.1).sum::<i32>(), clocks_tool::TABLE.w);
        assert_eq!(clocks_tool::visible_rows(), 17);

        let timers = dlu(0, 0, timers_tool::SIZE.0, timers_tool::SIZE.1);
        let all = timers_tool::HEADINGS
            .iter()
            .chain(&timers_tool::ELAPSED)
            .chain(&timers_tool::DRIFT)
            .chain([&timers_tool::START, &timers_tool::RESET, &timers_tool::CLOSE]);
        for r in all {
            assert!(timers.contains(*r), "{r:?}");
        }
        let splash_window = dlu(0, 0, splash::SIZE.0, splash::SIZE.1);
        for r in [splash::ARTWORK, splash::PROGRESS, splash::STAGE, splash::VERSION] {
            assert!(splash_window.contains(r), "{r:?}");
        }
        assert!(tool_close_box(195).bottom() <= TOOL_TITLE_HEIGHT);
    }
}
