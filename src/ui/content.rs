//! Maps the hardware model onto the controls of each tab page.

use crate::bench::{self, Phase};
use crate::brand;
use crate::model::{size_text, BridgeInfo, CacheInfo, CacheKind, GpuInfo, MemoryModule, TimingProfile};

use super::format;
use super::layout::{about, bench as bench_id, cpu, graphics, mainboard, memory, spd, Item, Kind, Tab};
use super::state::{BarView, BenchMode, ListId, View};

/// Number of profile columns in the SPD timings table.
const PROFILE_COLUMNS: usize = spd::PROFILES.len();

/// Text of a control: the model value for dynamic controls, the table text otherwise.
pub fn text(view: &View, tab: Tab, item: &Item) -> String {
    if item.kind == Kind::Combo {
        return combo_list(tab, item.id).map(|list| view.list_text(list)).unwrap_or_default();
    }
    let dynamic = match tab {
        Tab::Cpu => cpu_text(view, item.id),
        Tab::Mainboard => mainboard_text(view, item.id),
        Tab::Memory => memory_text(view, item.id),
        Tab::Spd => spd_text(view, item.id),
        Tab::Graphics => graphics_text(view, item.id),
        Tab::Bench => bench_text(view, item.id),
        Tab::About => about_text(view, item.id),
    };
    dynamic.unwrap_or_else(|| item.text.to_string())
}

/// The list a selector opens.
pub fn combo_list(tab: Tab, id: u16) -> Option<ListId> {
    match (tab, id) {
        (Tab::Cpu, cpu::SELECTION) => Some(ListId::Socket),
        (Tab::Spd, spd::SLOT) => Some(ListId::Slot),
        (Tab::Graphics, graphics::ADAPTER) => Some(ListId::Adapter),
        (Tab::Graphics, graphics::PERF_LEVEL) => Some(ListId::PerfLevel),
        (Tab::Bench, bench_id::THREADS) => Some(ListId::Threads),
        (Tab::Bench, bench_id::REFERENCE) => Some(ListId::Reference),
        _ => None,
    }
}

/// Whether an interactive control currently accepts clicks.
pub fn enabled(view: &View, tab: Tab, item: &Item) -> bool {
    let bench = &view.ui.bench;
    match (tab, item.kind, item.id) {
        (Tab::Bench, Kind::Combo, bench_id::THREADS) => bench.mode == BenchMode::Idle,
        (_, Kind::Combo, id) => combo_list(tab, id).is_some_and(|list| !view.list_items(list).is_empty()),
        (Tab::Bench, Kind::Button, bench_id::BENCH_BUTTON) => bench.mode != BenchMode::Stressing,
        (Tab::Bench, Kind::Button, bench_id::STRESS_BUTTON) => bench.mode != BenchMode::Benching,
        (Tab::Bench, Kind::Button, bench_id::SAVE_BUTTON) => bench.can_save(),
        _ => true,
    }
}

/// State of a check box.
pub fn checked(view: &View, tab: Tab, id: u16) -> bool {
    match (tab, id) {
        (Tab::Bench, bench_id::SINGLE_REFERENCE_CHECK) => view.ui.bench.reference_shown(Phase::Single),
        (Tab::Bench, bench_id::MULTI_REFERENCE_CHECK) => view.ui.bench.reference_shown(Phase::Multi),
        _ => false,
    }
}

/// Fill level and caption of a progress bar.
pub fn bar(view: &View, tab: Tab, id: u16) -> BarView {
    let bench = &view.ui.bench;
    match (tab, id) {
        (Tab::Bench, bench_id::SINGLE_BAR) => bench.bar(Phase::Single, false),
        (Tab::Bench, bench_id::SINGLE_REFERENCE_BAR) => bench.bar(Phase::Single, true),
        (Tab::Bench, bench_id::MULTI_BAR) => bench.bar(Phase::Multi, false),
        (Tab::Bench, bench_id::MULTI_REFERENCE_BAR) => bench.bar(Phase::Multi, true),
        _ => BarView::default(),
    }
}

fn opt(value: &Option<String>) -> String {
    value.clone().unwrap_or_default()
}

fn cpu_text(view: &View, id: u16) -> Option<String> {
    let cpu = &view.sys.cpu;
    let clocks = view.clocks;
    let core = view.core();
    let cache = |level, kind| cpu.caches_of(level, kind);
    // A level that mixes cache sizes lists them all, in the compact form the box has room for.
    let size = |caches: Vec<&CacheInfo>| match caches[..] {
        [single] => single.size_text(),
        _ => caches.iter().map(|c| c.short_size_text()).collect::<Vec<_>>().join(" + "),
    };
    // The associativity is shown when it holds for every cache on the line.
    let ways = |caches: Vec<&CacheInfo>| match caches[..] {
        [first, ref rest @ ..] if rest.iter().all(|c| c.ways == first.ways) => first.ways_text(),
        _ => String::new(),
    };
    let level1_inst = || {
        let instruction = cache(1, CacheKind::Instruction);
        if instruction.is_empty() {
            cache(1, CacheKind::Trace)
        } else {
            instruction
        }
    };
    // The CPUID signature is only meaningful once the processor has been identified.
    let identified = !(cpu.vendor_string.is_empty() && cpu.name.is_empty() && cpu.specification.is_empty());
    let signature = |field: u32| if identified { format::hex(field) } else { String::new() };
    Some(match id {
        cpu::NAME => cpu.name.clone(),
        cpu::CODE_NAME => opt(&cpu.codename),
        cpu::MAX_TDP => cpu.max_tdp_w.map(format::watts).unwrap_or_default(),
        cpu::PACKAGE => opt(&cpu.package),
        cpu::TECHNOLOGY => cpu.technology_nm.map(format::nanometers).unwrap_or_default(),
        cpu::CORE_VOLTAGE => cpu.core_voltage.map(format::core_voltage).unwrap_or_default(),
        cpu::SPECIFICATION => cpu.specification.clone(),
        cpu::FAMILY => signature(u32::from(cpu.family)),
        cpu::MODEL => signature(u32::from(cpu.model)),
        cpu::STEPPING => signature(u32::from(cpu.stepping)),
        cpu::EXT_FAMILY => signature(u32::from(cpu.ext_family)),
        cpu::EXT_MODEL => signature(u32::from(cpu.ext_model)),
        cpu::REVISION => opt(&cpu.revision),
        cpu::INSTRUCTIONS => cpu.instructions.join(", "),
        cpu::CLOCKS_CAPTION => {
            let (kind, number) = core.map_or((Default::default(), view.ui.core), |c| (c.kind, c.core));
            format!("Clocks ({})", format::core_label(kind, number))
        }
        cpu::CORE_SPEED => core.map(|c| format::mhz2(c.mhz)).unwrap_or_default(),
        cpu::MULTIPLIER => core
            .and_then(|c| c.multiplier)
            .map(|m| format::multiplier(m, clocks.multiplier_min, clocks.multiplier_max))
            .unwrap_or_default(),
        cpu::BUS_SPEED => clocks.bus_mhz.map(format::mhz2).unwrap_or_default(),
        cpu::RATED_FSB => clocks.rated_fsb_mhz.map(format::mhz2).unwrap_or_default(),
        cpu::L1_DATA => size(cache(1, CacheKind::Data)),
        cpu::L1_DATA_WAYS => ways(cache(1, CacheKind::Data)),
        cpu::L1_INST => size(level1_inst()),
        cpu::L1_INST_WAYS => ways(level1_inst()),
        cpu::LEVEL2 => size(cache(2, CacheKind::Unified)),
        cpu::LEVEL2_WAYS => ways(cache(2, CacheKind::Unified)),
        cpu::LEVEL3 => size(cache(3, CacheKind::Unified)),
        cpu::LEVEL3_WAYS => ways(cache(3, CacheKind::Unified)),
        cpu::CORES => format::core_count(&cpu.core_sets, cpu.cores),
        cpu::THREADS if cpu.threads > 0 => cpu.threads.to_string(),
        cpu::THREADS => String::new(),
        _ => return None,
    })
}

fn mainboard_text(view: &View, id: u16) -> Option<String> {
    let board = &view.sys.mainboard;
    let bridge = |b: &Option<BridgeInfo>, field: fn(&BridgeInfo) -> &String| {
        b.as_ref().map(|b| field(b).clone()).unwrap_or_default()
    };
    let link = board.graphic_interface.as_ref().map(|gi| gi.link);
    let width = |w: Option<u32>| w.map(format::link_width).unwrap_or_default();
    let speed = |s: Option<f32>| s.map(format::link_speed).unwrap_or_default();
    Some(match id {
        mainboard::MANUFACTURER => board.manufacturer.clone(),
        mainboard::MODEL => board.model.clone(),
        mainboard::VERSION => board.version.clone(),
        mainboard::BUS_SPECS => opt(&board.bus_specs),
        mainboard::CHIPSET_VENDOR => bridge(&board.chipset, |b| &b.vendor),
        mainboard::CHIPSET_MODEL => bridge(&board.chipset, |b| &b.model),
        mainboard::CHIPSET_REVISION => bridge(&board.chipset, |b| &b.revision),
        mainboard::SOUTHBRIDGE_VENDOR => bridge(&board.southbridge, |b| &b.vendor),
        mainboard::SOUTHBRIDGE_MODEL => bridge(&board.southbridge, |b| &b.model),
        mainboard::SOUTHBRIDGE_REVISION => bridge(&board.southbridge, |b| &b.revision),
        mainboard::LPCIO_VENDOR => bridge(&board.lpcio, |b| &b.vendor),
        mainboard::LPCIO_MODEL => bridge(&board.lpcio, |b| &b.model),
        mainboard::BIOS_BRAND => board.bios.brand.clone(),
        mainboard::BIOS_VERSION => board.bios.version.clone(),
        mainboard::BIOS_DATE => board.bios.date.clone(),
        mainboard::MICROCODE => view.sys.cpu.microcode.map(format::microcode).unwrap_or_default(),
        mainboard::GI_BUS => board.graphic_interface.as_ref().map(|gi| gi.bus.clone()).unwrap_or_default(),
        mainboard::GI_WIDTH => width(link.and_then(|l| l.width)),
        mainboard::GI_MAX_WIDTH => width(link.and_then(|l| l.max_width)),
        mainboard::GI_SPEED => speed(link.and_then(|l| l.speed_gts)),
        mainboard::GI_MAX_SPEED => speed(link.and_then(|l| l.max_speed_gts)),
        _ => return None,
    })
}

fn memory_text(view: &View, id: u16) -> Option<String> {
    let mem = &view.sys.memory;
    let timings = &mem.timings;
    let cycles = |v: Option<u32>| v.map(format::clocks).unwrap_or_default();
    Some(match id {
        memory::TYPE => opt(&mem.kind),
        memory::SIZE => size_text(mem.size_mb),
        memory::CHANNELS => opt(&mem.channels),
        memory::DC_MODE => opt(&mem.dc_mode),
        memory::UNCORE_FREQUENCY => mem.controller_mhz.map(format::mhz1).unwrap_or_default(),
        memory::DRAM_FREQUENCY => mem.dram_mhz.map(format::mhz1).unwrap_or_default(),
        memory::FSB_DRAM => opt(&mem.fsb_dram),
        memory::CAS_LATENCY => timings.cas_latency.map(format::latency_clocks).unwrap_or_default(),
        memory::TRCD => cycles(timings.trcd),
        memory::TRP => cycles(timings.trp),
        memory::TRAS => cycles(timings.tras),
        memory::TRC => cycles(timings.trc),
        memory::COMMAND_RATE => opt(&timings.command_rate),
        _ => return None,
    })
}

/// The profiles shown in the timings table when a module has more than fit.
///
/// Like the reference tool, the table ends with the overclocking profiles (XMP,
/// EXPO, ...) and fills the columns before them with the fastest JEDEC entries;
/// at least half of the columns stay with JEDEC while there are enough of them.
fn shown_profiles(profiles: &[TimingProfile]) -> Vec<&TimingProfile> {
    let (standard, extended): (Vec<_>, Vec<_>) = profiles.iter().partition(|p| p.name.starts_with("JEDEC"));
    let reserved = standard.len().min(PROFILE_COLUMNS / 2);
    let extended_shown = extended.len().min(PROFILE_COLUMNS - reserved);
    let standard_shown = standard.len().min(PROFILE_COLUMNS - extended_shown);
    let fastest = standard.len() - standard_shown;
    standard.into_iter().skip(fastest).chain(extended.into_iter().take(extended_shown)).collect()
}

/// One cell of the timings table.
fn profile_cell(profile: &TimingProfile, row: usize) -> String {
    let number = |v: Option<u32>| v.map(|v| v.to_string()).unwrap_or_default();
    match row {
        0 => profile.frequency_mhz.map(format::mhz0).unwrap_or_default(),
        1 => profile.cas_latency.map(|cl| format!("{cl:.1}")).unwrap_or_default(),
        2 => number(profile.trcd),
        3 => number(profile.trp),
        4 => number(profile.tras),
        5 => number(profile.trc),
        6 => opt(&profile.command_rate),
        7 => profile.voltage.map(format::profile_voltage).unwrap_or_default(),
        _ => String::new(),
    }
}

fn spd_text(view: &View, id: u16) -> Option<String> {
    let module = view.module().filter(|module| module.populated);
    let profiles = module.map(|m| shown_profiles(&m.profiles)).unwrap_or_default();
    if let Some(column) = spd::PROFILES.iter().position(|&heading| heading == id) {
        return Some(profiles.get(column).map(|p| p.name.clone()).unwrap_or_default());
    }
    for (row, ids) in spd::TABLE.iter().enumerate() {
        if let Some(column) = ids.iter().position(|&cell| cell == id) {
            return Some(profiles.get(column).map(|p| profile_cell(p, row)).unwrap_or_default());
        }
    }
    let field = |f: fn(&MemoryModule) -> Option<String>| module.and_then(f).unwrap_or_default();
    Some(match id {
        spd::MODULE_TYPE => field(|m| m.kind.clone()),
        spd::MODULE_SIZE => module.map(|m| size_text(m.size_mb)).unwrap_or_default(),
        spd::MAX_BANDWIDTH => field(|m| m.max_bandwidth.clone()),
        spd::SPD_EXT => field(|m| m.spd_ext.clone()),
        spd::MODULE_MANUFACTURER => field(|m| m.manufacturer.clone()),
        spd::WEEK_YEAR => field(|m| m.week_year.map(|(week, year)| format::week_year(week, year))),
        spd::DRAM_MANUFACTURER => field(|m| m.dram_manufacturer.clone()),
        spd::BUFFERED => field(|m| m.buffered.map(|b| format::yes_no(b).to_string())),
        spd::PART_NUMBER => field(|m| m.part_number.clone()),
        spd::CORRECTION => field(|m| m.correction.clone()),
        spd::SERIAL_NUMBER => field(|m| m.serial_number.clone()),
        spd::REGISTERED => field(|m| m.registered.map(|r| format::yes_no(r).to_string())),
        _ => return None,
    })
}

fn graphics_text(view: &View, id: u16) -> Option<String> {
    let gpu = view.gpu();
    let level = view.perf_level();
    let clock = |mhz: Option<f64>| mhz.map(format::mhz1).unwrap_or_default();
    let field = |f: fn(&GpuInfo) -> Option<String>| gpu.and_then(f).unwrap_or_default();
    Some(match id {
        graphics::NAME => gpu.map(|g| g.name.clone()).unwrap_or_default(),
        graphics::BOARD_MANUFACTURER => field(|g| g.board_manufacturer.clone()),
        graphics::CODE_NAME => field(|g| g.codename.clone()),
        graphics::REVISION => field(|g| g.revision.clone()),
        graphics::TECHNOLOGY => field(|g| g.technology_nm.map(format::nanometers)),
        graphics::TDP => field(|g| g.tdp_w.map(format::watts)),
        graphics::ROPS_TMUS => gpu.map(|g| format::rops_tmus(g.rops, g.tmus)).unwrap_or_default(),
        graphics::CORES => field(|g| g.cores.map(|c| c.to_string())),
        graphics::CORE_CLOCK => clock(level.and_then(|l| l.core_mhz)),
        graphics::SHADER_CLOCK => clock(level.and_then(|l| l.shader_mhz)),
        graphics::MEMORY_CLOCK => clock(level.and_then(|l| l.memory_mhz)),
        graphics::MEMORY_SIZE => field(|g| g.memory_mb.map(format::graphics_memory)),
        graphics::MEMORY_TYPE => field(|g| g.memory_type.clone()),
        graphics::MEMORY_VENDOR => field(|g| g.memory_vendor.clone()),
        graphics::MEMORY_BUS_WIDTH => field(|g| g.memory_bus_width.map(format::bits)),
        _ => return None,
    })
}

fn bench_text(view: &View, id: u16) -> Option<String> {
    let state = &view.ui.bench;
    Some(match id {
        bench_id::RATIO => state.ratio().map(format::ratio).unwrap_or_default(),
        bench_id::BENCH_BUTTON if state.mode == BenchMode::Benching => "Stop".to_string(),
        bench_id::STRESS_BUTTON if state.mode == BenchMode::Stressing => "Stop".to_string(),
        bench_id::PROCESSOR => view.sys.cpu.specification.clone(),
        bench_id::VERSION => format!("{} Benchmark Version {}", brand::APP_NAME, bench::VERSION),
        _ => return None,
    })
}

fn about_text(view: &View, id: u16) -> Option<String> {
    let os = &view.sys.os;
    Some(match id {
        about::NAME => brand::APP_NAME.to_string(),
        about::VERSION => format::long_version(brand::VERSION),
        about::LICENSE => format!("License: {}", env!("CARGO_PKG_LICENSE")),
        about::OS_NAME => os.name.clone(),
        about::OS_BUILD => os.build.clone(),
        about::DIRECTX => os.directx.as_ref().map(|version| format!("DirectX {version}")).unwrap_or_default(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use crate::model::{ClockSnapshot, SystemInfo};
    use crate::ui::layout::STATIC;
    use crate::ui::state::UiState;

    fn value(view: &View, tab: Tab, id: u16) -> String {
        let item = tab.item(id).unwrap_or_else(|| panic!("{tab:?} has no control {id}"));
        text(view, tab, item)
    }

    fn with_demo(check: impl FnOnce(&View)) {
        let sys = demo::system();
        let clocks = demo::clocks();
        let ui = UiState::new(&sys, None, 16, Vec::new());
        check(&View { sys: &sys, clocks: &clocks, ui: &ui });
    }

    #[test]
    fn cpu_page() {
        with_demo(|v| {
            assert_eq!(value(v, Tab::Cpu, cpu::NAME), "AMD Ryzen 7 7700");
            assert_eq!(value(v, Tab::Cpu, cpu::CODE_NAME), "Raphael");
            assert_eq!(value(v, Tab::Cpu, cpu::MAX_TDP), "65.0 W");
            assert_eq!(value(v, Tab::Cpu, cpu::TECHNOLOGY), "5 nm");
            assert_eq!(value(v, Tab::Cpu, cpu::CORE_VOLTAGE), "1.016 V");
            assert_eq!(value(v, Tab::Cpu, cpu::FAMILY), "F");
            assert_eq!(value(v, Tab::Cpu, cpu::EXT_FAMILY), "19");
            assert_eq!(value(v, Tab::Cpu, cpu::EXT_MODEL), "61");
            assert!(value(v, Tab::Cpu, cpu::INSTRUCTIONS).starts_with("MMX (+), SSE, SSE2"));
            assert_eq!(value(v, Tab::Cpu, cpu::CLOCKS_CAPTION), "Clocks (Core #0)");
            assert_eq!(value(v, Tab::Cpu, cpu::CORE_SPEED), "5136.63 MHz");
            assert_eq!(value(v, Tab::Cpu, cpu::MULTIPLIER), "x 51.5 (4.0 - 53.5)");
            assert_eq!(value(v, Tab::Cpu, cpu::BUS_SPEED), "99.74 MHz");
            assert_eq!(value(v, Tab::Cpu, cpu::RATED_FSB), "");
            assert_eq!(value(v, Tab::Cpu, cpu::L1_DATA), "8 x 32 KBytes");
            assert_eq!(value(v, Tab::Cpu, cpu::L1_DATA_WAYS), "8-way");
            assert_eq!(value(v, Tab::Cpu, cpu::LEVEL2), "8 x 1 MBytes");
            assert_eq!(value(v, Tab::Cpu, cpu::LEVEL3), "32 MBytes");
            assert_eq!(value(v, Tab::Cpu, cpu::LEVEL3_WAYS), "16-way");
            assert_eq!(value(v, Tab::Cpu, cpu::SELECTION), "Socket #1");
            assert_eq!(value(v, Tab::Cpu, cpu::CORES), "8");
            assert_eq!(value(v, Tab::Cpu, cpu::THREADS), "16");
        });
    }

    #[test]
    fn multiplier_shows_the_maximum_when_the_minimum_is_unknown() {
        // The shape of a live sample: the minimum cannot be read from user mode.
        let sys = demo::system();
        let clocks = ClockSnapshot { multiplier_min: None, ..demo::clocks() };
        let ui = UiState::new(&sys, None, 16, Vec::new());
        let v = View { sys: &sys, clocks: &clocks, ui: &ui };
        assert_eq!(value(&v, Tab::Cpu, cpu::MULTIPLIER), "x 51.5 (max 53.5)");
    }

    #[test]
    fn caches_of_different_size_share_a_box() {
        let level3 = |ways: u32| {
            // A second core complex under a larger, stacked L3.
            let mut sys = demo::system();
            let plain = sys.cpu.caches[3];
            sys.cpu.caches.insert(3, CacheInfo { size_kb: 96 * 1024, ways, ..plain });
            let clocks = demo::clocks();
            let ui = UiState::new(&sys, None, 16, Vec::new());
            let v = View { sys: &sys, clocks: &clocks, ui: &ui };
            assert_eq!(value(&v, Tab::Cpu, cpu::LEVEL2), "8 x 1 MBytes");
            (value(&v, Tab::Cpu, cpu::LEVEL3), value(&v, Tab::Cpu, cpu::LEVEL3_WAYS))
        };
        assert_eq!(level3(16), ("96 MB + 32 MB".to_string(), "16-way".to_string()));
        // An associativity that holds for one of the caches only is not shown.
        assert_eq!(level3(12), ("96 MB + 32 MB".to_string(), String::new()));
    }

    #[test]
    fn mainboard_page() {
        with_demo(|v| {
            assert_eq!(value(v, Tab::Mainboard, mainboard::MODEL), "B650 EXAMPLE WIFI");
            assert_eq!(value(v, Tab::Mainboard, mainboard::VERSION), "1.0");
            assert_eq!(value(v, Tab::Mainboard, mainboard::CHIPSET_MODEL), "Ryzen SOC");
            assert_eq!(value(v, Tab::Mainboard, mainboard::SOUTHBRIDGE_REVISION), "51");
            assert_eq!(value(v, Tab::Mainboard, mainboard::LPCIO_MODEL), "NCT6687");
            assert_eq!(value(v, Tab::Mainboard, mainboard::MICROCODE), "0xA601203");
            assert_eq!(value(v, Tab::Mainboard, mainboard::GI_BUS), "PCI-Express 4.0");
            assert_eq!(value(v, Tab::Mainboard, mainboard::GI_WIDTH), "x8");
            assert_eq!(value(v, Tab::Mainboard, mainboard::GI_MAX_SPEED), "16.0 GT/s");
        });
    }

    #[test]
    fn memory_page() {
        with_demo(|v| {
            assert_eq!(value(v, Tab::Memory, memory::TYPE), "DDR5");
            assert_eq!(value(v, Tab::Memory, memory::SIZE), "32 GBytes");
            assert_eq!(value(v, Tab::Memory, memory::CHANNELS), "4 x 32-bit");
            assert_eq!(value(v, Tab::Memory, memory::DC_MODE), "");
            assert_eq!(value(v, Tab::Memory, memory::UNCORE_FREQUENCY), "2600.1 MHz");
            assert_eq!(value(v, Tab::Memory, memory::FSB_DRAM), "1:26");
            assert_eq!(value(v, Tab::Memory, memory::CAS_LATENCY), "42.0 clocks");
            assert_eq!(value(v, Tab::Memory, memory::TRC), "125 clocks");
            assert_eq!(value(v, Tab::Memory, memory::COMMAND_RATE), "1T");
        });
    }

    #[test]
    fn spd_page() {
        with_demo(|v| {
            assert_eq!(value(v, Tab::Spd, spd::SLOT), "Slot #2");
            assert_eq!(value(v, Tab::Spd, spd::MODULE_TYPE), "DDR5");
            assert_eq!(value(v, Tab::Spd, spd::MODULE_SIZE), "16 GBytes");
            assert_eq!(value(v, Tab::Spd, spd::WEEK_YEAR), "39 / 25");
            assert_eq!(value(v, Tab::Spd, spd::REGISTERED), "No");
            assert_eq!(value(v, Tab::Spd, spd::PROFILES[0]), "JEDEC #8");
            assert_eq!(value(v, Tab::Spd, spd::PROFILES[3]), "EXPO-5200");
            assert_eq!(value(v, Tab::Spd, spd::TABLE[0][0]), "2633 MHz");
            assert_eq!(value(v, Tab::Spd, spd::TABLE[1][2]), "46.0");
            assert_eq!(value(v, Tab::Spd, spd::TABLE[4][3]), "84");
            assert_eq!(value(v, Tab::Spd, spd::TABLE[6][0]), "");
            assert_eq!(value(v, Tab::Spd, spd::TABLE[7][1]), "1.10 V");
        });
    }

    #[test]
    fn empty_slot_shows_blank_fields() {
        let sys = demo::system();
        let clocks = demo::clocks();
        let mut ui = UiState::new(&sys, None, 16, Vec::new());
        ui.slot = 0;
        let v = View { sys: &sys, clocks: &clocks, ui: &ui };
        assert_eq!(value(&v, Tab::Spd, spd::SLOT), "Slot #1");
        for item in Tab::Spd.items().iter().filter(|item| item.kind.is_box() && item.kind != Kind::Combo) {
            assert_eq!(text(&v, Tab::Spd, item), "", "control {}", item.id);
        }
        assert_eq!(value(&v, Tab::Spd, spd::PROFILES[0]), "");
    }

    #[test]
    fn profile_columns_mix_jedec_and_overclocking_profiles() {
        let named = |name: &str| TimingProfile { name: name.into(), ..Default::default() };
        let jedec: Vec<TimingProfile> = (1..=10).map(|n| named(&format!("JEDEC #{n}"))).collect();
        let names = |profiles: &[TimingProfile]| -> Vec<String> {
            shown_profiles(profiles).into_iter().map(|p| p.name.clone()).collect()
        };
        assert_eq!(names(&jedec), ["JEDEC #7", "JEDEC #8", "JEDEC #9", "JEDEC #10"]);
        assert_eq!(names(&jedec[..2]), ["JEDEC #1", "JEDEC #2"]);
        assert!(names(&[]).is_empty());

        let mut with_one = jedec.clone();
        with_one.push(named("XMP-5600"));
        assert_eq!(names(&with_one), ["JEDEC #8", "JEDEC #9", "JEDEC #10", "XMP-5600"]);

        let mut with_four = jedec.clone();
        with_four.extend(["XMP-5600", "XMP-5200", "EXPO-5600", "EXPO-5200"].map(named));
        assert_eq!(names(&with_four), ["JEDEC #9", "JEDEC #10", "XMP-5600", "XMP-5200"]);

        let mut mostly_extended = vec![named("JEDEC #1")];
        mostly_extended.extend(["XMP-1", "XMP-2", "XMP-3", "XMP-4"].map(named));
        assert_eq!(names(&mostly_extended), ["JEDEC #1", "XMP-1", "XMP-2", "XMP-3"]);

        let only_extended: Vec<TimingProfile> = (1..=6).map(|n| named(&format!("XMP-{n}"))).collect();
        assert_eq!(names(&only_extended), ["XMP-1", "XMP-2", "XMP-3", "XMP-4"]);
    }

    #[test]
    fn graphics_page() {
        with_demo(|v| {
            assert_eq!(value(v, Tab::Graphics, graphics::ADAPTER), "NVIDIA GeForce RTX 4060");
            assert_eq!(value(v, Tab::Graphics, graphics::PERF_LEVEL), "Current");
            assert_eq!(value(v, Tab::Graphics, graphics::TECHNOLOGY), "5 nm");
            assert_eq!(value(v, Tab::Graphics, graphics::TDP), "115.0 W");
            assert_eq!(value(v, Tab::Graphics, graphics::ROPS_TMUS), "48 / 96");
            assert_eq!(value(v, Tab::Graphics, graphics::CORE_CLOCK), "285.0 MHz");
            assert_eq!(value(v, Tab::Graphics, graphics::SHADER_CLOCK), "");
            assert_eq!(value(v, Tab::Graphics, graphics::MEMORY_SIZE), "8 GBytes");
            assert_eq!(value(v, Tab::Graphics, graphics::MEMORY_BUS_WIDTH), "128 bits");
        });
    }

    #[test]
    fn bench_page() {
        let sys = demo::system();
        let clocks = demo::clocks();
        let mut ui = UiState::new(&sys, None, 16, Vec::new());
        {
            let v = View { sys: &sys, clocks: &clocks, ui: &ui };
            assert_eq!(value(&v, Tab::Bench, bench_id::BENCH_BUTTON), "Bench CPU");
            assert_eq!(value(&v, Tab::Bench, bench_id::STRESS_BUTTON), "Stress CPU");
            assert_eq!(value(&v, Tab::Bench, bench_id::THREADS), "16 threads");
            assert_eq!(value(&v, Tab::Bench, bench_id::PROCESSOR), "AMD Ryzen 7 7700 8-Core Processor");
            assert_eq!(value(&v, Tab::Bench, bench_id::RATIO), "");
            assert!(value(&v, Tab::Bench, bench_id::VERSION).starts_with("RS-CPUZ Benchmark Version "));
            let save = Tab::Bench.item(bench_id::SAVE_BUTTON).unwrap();
            assert!(!enabled(&v, Tab::Bench, save));
            assert!(checked(&v, Tab::Bench, bench_id::SINGLE_REFERENCE_CHECK));
            assert!(bar(&v, Tab::Bench, bench_id::SINGLE_BAR).visible);
        }
        ui.bench.begin_bench();
        let v = View { sys: &sys, clocks: &clocks, ui: &ui };
        assert_eq!(value(&v, Tab::Bench, bench_id::BENCH_BUTTON), "Stop");
        let stress = Tab::Bench.item(bench_id::STRESS_BUTTON).unwrap();
        let threads = Tab::Bench.item(bench_id::THREADS).unwrap();
        assert!(!enabled(&v, Tab::Bench, stress));
        assert!(!enabled(&v, Tab::Bench, threads));
    }

    #[test]
    fn save_button_needs_scores_of_the_processor_on_show() {
        let sys = demo::system();
        let clocks = demo::clocks();
        let mut ui = UiState::new(&sys, None, 16, Vec::new());
        let save = Tab::Bench.item(bench_id::SAVE_BUTTON).unwrap();
        ui.bench.single = Some(700.0);
        ui.bench.multi = Some(7000.0);
        assert!(enabled(&View { sys: &sys, clocks: &clocks, ui: &ui }, Tab::Bench, save));
        // With the sample system on show the scores are those of another processor.
        ui.bench.sample = true;
        assert!(!enabled(&View { sys: &sys, clocks: &clocks, ui: &ui }, Tab::Bench, save));
    }

    #[test]
    fn about_page() {
        with_demo(|v| {
            assert_eq!(value(v, Tab::About, about::NAME), "RS-CPUZ");
            assert!(value(v, Tab::About, about::VERSION).starts_with("Version "));
            assert_eq!(value(v, Tab::About, about::OS_NAME), "Microsoft Windows 11 Home (x64)");
            assert_eq!(value(v, Tab::About, about::DIRECTX), "DirectX 12.0");
            assert_eq!(value(v, Tab::About, about::SAVE_TXT), "Save Report (.TXT)");
        });
    }

    #[test]
    fn rows_without_an_identifier_show_their_own_text() {
        with_demo(|v| {
            for tab in Tab::ALL {
                for item in tab.items().iter().filter(|item| item.id == STATIC) {
                    assert_eq!(text(v, tab, item), item.text, "{tab:?}: {item:?}");
                }
            }
        });
    }

    #[test]
    fn empty_model_renders_blank_values() {
        let sys = SystemInfo::default();
        let clocks = ClockSnapshot::default();
        let ui = UiState::new(&sys, None, 1, Vec::new());
        let v = View { sys: &sys, clocks: &clocks, ui: &ui };
        assert_eq!(value(&v, Tab::Cpu, cpu::CORE_SPEED), "");
        assert_eq!(value(&v, Tab::Cpu, cpu::CORES), "");
        assert_eq!(value(&v, Tab::Cpu, cpu::FAMILY), "");
        assert_eq!(value(&v, Tab::Cpu, cpu::EXT_MODEL), "");
        assert_eq!(value(&v, Tab::Cpu, cpu::THREADS), "");
        assert_eq!(value(&v, Tab::Cpu, cpu::CLOCKS_CAPTION), "Clocks (Core #0)");
        assert_eq!(value(&v, Tab::Mainboard, mainboard::GI_WIDTH), "");
        assert_eq!(value(&v, Tab::Memory, memory::SIZE), "");
        assert_eq!(value(&v, Tab::Graphics, graphics::NAME), "");
        assert_eq!(value(&v, Tab::Graphics, graphics::ADAPTER), "");
        let adapter = Tab::Graphics.item(graphics::ADAPTER).unwrap();
        assert!(!enabled(&v, Tab::Graphics, adapter));
        for tab in Tab::ALL {
            for item in tab.items() {
                let _ = text(&v, tab, item);
            }
        }
    }
}
