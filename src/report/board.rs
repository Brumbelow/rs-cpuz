//! Mainboard sections: BIOS, chipset and the memory modules.

use crate::model::{self, MainboardInfo, MemoryInfo, MemoryModule, Sensor, TimingProfile};

use super::doc::{Lines, Section};
use super::fmt;

/// Placeholder for a timing the profile does not specify.
const UNKNOWN_TIMING: &str = "n.a";

/// "BIOS": whether the firmware is UEFI. Vendor, version and date are in the DMI section.
pub(super) fn bios(board: &MainboardInfo) -> Section {
    let mut out = Lines::default();
    out.opt(0, "UEFI", board.bios.uefi.map(|uefi| if uefi { "Yes" } else { "No" }));
    out.into_section("BIOS")
}

/// "Chipset": the bridges, the graphic interface and the active memory configuration.
pub(super) fn chipset(board: &MainboardInfo, memory: &MemoryInfo) -> Section {
    let mut out = Lines::default();
    out.opt(0, "Northbridge", board.chipset.as_ref().map(fmt::bridge));
    out.opt(0, "Southbridge", board.southbridge.as_ref().map(fmt::bridge));
    out.opt(0, "Bus Specification", board.bus_specs.as_deref());
    if let Some(interface) = &board.graphic_interface {
        out.pair(0, "Graphic Interface", &interface.bus);
        out.opt(0, "PCI-E Link Width", fmt::graphic_link_width(&interface.link));
        out.opt(0, "PCI-E Link Speed", fmt::link_speed(&interface.link));
    }
    out.opt(0, "Memory Type", memory.kind.as_deref());
    out.pair(0, "Memory Size", model::size_text(memory.size_mb));
    out.opt(0, "Channels", memory.channels.as_deref());
    out.opt(
        0,
        "Memory Frequency",
        memory.dram_mhz.map(|mhz| memory_frequency(mhz, memory.fsb_dram.as_deref())),
    );
    out.opt(0, "Memory Max Frequency", memory.max_dram_mhz.map(|mhz| memory_frequency(mhz, None)));
    let timings = &memory.timings;
    out.opt(0, "CAS# latency (CL)", timings.cas_latency.map(|cl| format!("{cl:.1}")));
    out.opt(0, "RAS# to CAS# delay (tRCD)", timings.trcd);
    out.opt(0, "RAS# Precharge (tRP)", timings.trp);
    out.opt(0, "Cycle Time (tRAS)", timings.tras);
    out.opt(0, "Bank Cycle Time (tRC)", timings.trc);
    out.opt(0, "Command Rate (CR)", timings.command_rate.as_deref());
    out.opt(0, "Uncore Frequency", memory.controller_mhz.map(|mhz| format!("{mhz:.1} MHz")));
    out.blank();
    out.into_section("Chipset")
}

/// "Memory SPD": one numbered block per populated module, ending with its timings tables.
pub(super) fn spd(modules: &[MemoryModule]) -> Section {
    let mut out = Lines::default();
    for (n, module) in modules.iter().filter(|m| m.populated).enumerate() {
        out.pair(0, "DIMM #", (n + 1).to_string());
        out.pair(1, "Slot", slot_text(module));
        out.opt(1, "Memory type", module.kind.as_deref());
        out.opt(1, "Module format", module.form_factor.as_deref());
        out.opt(1, "Module Manufacturer", module.manufacturer.as_deref());
        out.opt(1, "SDRAM Manufacturer", module.dram_manufacturer.as_deref());
        out.opt(1, "Size", (module.size_mb > 0).then(|| format!("{} MBytes", module.size_mb)));
        out.opt(1, "Max bandwidth", module.max_bandwidth.as_deref());
        out.opt(1, "Part number", module.part_number.as_deref());
        out.opt(1, "Serial number", module.serial_number.as_deref());
        let date = module.week_year.map(|(week, year)| format!("Week {week:02}/Year {year:02}"));
        out.opt(1, "Manufacturing date", date);
        out.opt(1, "Nominal Voltage", module.voltage.map(|volts| format!("{volts:.2} Volts")));
        out.opt(1, "SPD Ext.", module.spd_ext.as_deref());
        out.opt(1, "Correction", module.correction.as_deref());
        out.opt(1, "Registered", module.registered.map(yes_no));
        out.opt(1, "Buffered", module.buffered.map(yes_no));
        out.opt(1, "Ranks", module.ranks);
        timing_tables(&mut out, &module.profiles);
        out.blank();
    }
    out.into_section("Memory SPD")
}

/// "Monitoring": the board whose sensors a monitoring tool would read, then every sensor
/// detection read, grouped by what it belongs to.
pub(super) fn monitoring(board: &MainboardInfo, sensors: &[Sensor]) -> Section {
    let mut out = Lines::default();
    out.short("Mainboard Model", 2, &board.model);
    let mut groups: Vec<&str> = Vec::new();
    for sensor in sensors {
        if !groups.contains(&sensor.group.as_str()) {
            groups.push(&sensor.group);
        }
    }
    if !groups.is_empty() {
        out.blank();
    }
    for group in groups {
        out.short("Hardware monitor", 1, group);
        for sensor in sensors.iter().filter(|sensor| sensor.group == group) {
            out.pair(1, &sensor.name, sensor.value_text());
        }
    }
    out.into_section("Monitoring")
}

fn yes_no(flag: bool) -> &'static str {
    if flag {
        "yes"
    } else {
        "no"
    }
}

/// `2600.1 MHz (1:26)` - the DRAM clock and, when known, its ratio to the bus clock.
fn memory_frequency(mhz: f64, ratio: Option<&str>) -> String {
    match ratio {
        Some(ratio) => format!("{mhz:.1} MHz ({ratio})"),
        None => format!("{mhz:.1} MHz"),
    }
}

/// `Slot #2 (DIMMA2)` - the slot label followed by the firmware designation.
fn slot_text(module: &MemoryModule) -> String {
    match (module.slot.is_empty(), module.locator.is_empty()) {
        (false, false) => format!("{} ({})", module.slot, module.locator),
        (false, true) => module.slot.clone(),
        (true, _) => module.locator.clone(),
    }
}

/// Appends one timings table per run of profiles of the same family (JEDEC, XMP, EXPO).
/// The command-rate and voltage columns appear only when a profile of the run has them.
fn timing_tables(out: &mut Lines, profiles: &[TimingProfile]) {
    for run in profiles.chunk_by(|a, b| family(&a.name) == family(&b.name)) {
        let Some(first) = run.first() else { continue };
        let with_rate = run.iter().any(|p| p.command_rate.is_some());
        let with_voltage = run.iter().any(|p| p.voltage.is_some());
        let columns = format!(
            "CL-tRCD-tRP-tRAS-tRC{} @ frequency{}",
            if with_rate { "-CR" } else { "" },
            if with_voltage { " (voltage)" } else { "" }
        );
        out.pair(0, &format!("{} timings table", family(&first.name)), columns);
        for profile in run {
            out.pair(1, &profile.name, timing_row(profile, with_rate, with_voltage));
        }
    }
}

/// `JEDEC` for `JEDEC #8`, `XMP` for `XMP-5600`: a profile name up to its number.
fn family(name: &str) -> &str {
    name.split(['#', '-']).next().unwrap_or(name).trim()
}

/// `42.0-43-43-85-127 @ 2633 MHz (1.100 Volts)` - one row of a timings table.
fn timing_row(profile: &TimingProfile, with_rate: bool, with_voltage: bool) -> String {
    let unknown = || UNKNOWN_TIMING.to_string();
    let cycles = |value: Option<u32>| value.map_or_else(unknown, |cycles| cycles.to_string());
    let mut timings = vec![
        profile.cas_latency.map_or_else(unknown, |cl| format!("{cl:.1}")),
        cycles(profile.trcd),
        cycles(profile.trp),
        cycles(profile.tras),
        cycles(profile.trc),
    ];
    if with_rate {
        timings.push(profile.command_rate.clone().unwrap_or_else(unknown));
    }
    let frequency = profile.frequency_mhz.map_or_else(unknown, |mhz| format!("{mhz:.0} MHz"));
    let mut row = format!("{} @ {frequency}", timings.join("-"));
    if with_voltage {
        let voltage = profile.voltage.map_or_else(unknown, |volts| format!("{volts:.3} Volts"));
        row.push_str(&format!(" ({voltage})"));
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use crate::model::{BiosInfo, MemoryTimings, SensorKind};
    use crate::report::txt::lines_of;

    fn profile(name: &str, mhz: f64) -> TimingProfile {
        TimingProfile {
            name: name.into(),
            frequency_mhz: Some(mhz),
            cas_latency: Some(22.0),
            trcd: Some(22),
            trp: Some(22),
            tras: Some(44),
            trc: Some(66),
            ..Default::default()
        }
    }

    #[test]
    fn reports_the_firmware_type() {
        let board =
            |uefi| MainboardInfo { bios: BiosInfo { uefi, ..Default::default() }, ..Default::default() };
        assert_eq!(lines_of(&bios(&board(Some(true)))), ["UEFI\t\t\t\tYes"]);
        assert_eq!(lines_of(&bios(&board(Some(false)))), ["UEFI\t\t\t\tNo"]);
        assert!(lines_of(&bios(&board(None))).is_empty());
    }

    #[test]
    fn describes_the_chipset() {
        let sys = demo::system();
        assert_eq!(
            lines_of(&chipset(&sys.mainboard, &sys.memory)),
            [
                "Northbridge\t\t\tAMD Ryzen SOC rev. 00",
                "Southbridge\t\t\tAMD B650 rev. 51",
                "Bus Specification\t\tPCI-Express 5.0 (32.0 GT/s)",
                "Graphic Interface\t\tPCI-Express 4.0",
                "PCI-E Link Width\t\tx8 (max 8x)",
                "PCI-E Link Speed\t\t16.0 GT/s (max 16.0 GT/s)",
                "Memory Type\t\t\tDDR5",
                "Memory Size\t\t\t32 GBytes",
                "Channels\t\t\t4 x 32-bit",
                "Memory Frequency\t\t2600.1 MHz (1:26)",
                "Memory Max Frequency\t\t2800.0 MHz",
                "CAS# latency (CL)\t\t42.0",
                "RAS# to CAS# delay (tRCD)\t42",
                "RAS# Precharge (tRP)\t\t42",
                "Cycle Time (tRAS)\t\t84",
                "Bank Cycle Time (tRC)\t\t125",
                "Command Rate (CR)\t\t1T",
                "Uncore Frequency\t\t2600.1 MHz",
                "",
            ]
        );
    }

    #[test]
    fn skips_what_user_mode_cannot_read() {
        let memory = MemoryInfo {
            kind: Some("DDR4".into()),
            size_mb: 16_280,
            dram_mhz: Some(1600.0),
            max_dram_mhz: None,
            timings: MemoryTimings::default(),
            ..Default::default()
        };
        assert_eq!(
            lines_of(&chipset(&MainboardInfo::default(), &memory)),
            ["Memory Type\t\t\tDDR4", "Memory Size\t\t\t15.9 GBytes", "Memory Frequency\t\t1600.0 MHz", ""]
        );
    }

    #[test]
    fn lists_populated_modules_only() {
        let lines = lines_of(&spd(&demo::system().modules));
        let expected = [
            "DIMM #\t\t\t\t1",
            "\tSlot\t\t\tSlot #2 (DIMMA2)",
            "\tMemory type\t\tDDR5",
            "\tModule format\t\tUDIMM",
            "\tModule Manufacturer\tExample Memory",
            "\tSDRAM Manufacturer\tExample Memory",
            "\tSize\t\t\t16384 MBytes",
            "\tMax bandwidth\t\tDDR5-5600 (2800 MHz)",
            "\tPart number\t\tEX16G56C46U5",
            "\tSerial number\t\t00000001",
            "\tManufacturing date\tWeek 39/Year 25",
            "\tNominal Voltage\t\t1.10 Volts",
            "\tSPD Ext.\t\tEXPO, XMP",
            "\tCorrection\t\tNone",
            "\tRegistered\t\tno",
            "\tBuffered\t\tno",
            "\tRanks\t\t\t1",
            "JEDEC timings table\t\tCL-tRCD-tRP-tRAS-tRC @ frequency (voltage)",
            "\tJEDEC #8\t\t42.0-43-43-85-127 @ 2633 MHz (1.100 Volts)",
            "\tJEDEC #9\t\t46.0-45-45-90-135 @ 2800 MHz (1.100 Volts)",
            "XMP timings table\t\tCL-tRCD-tRP-tRAS-tRC @ frequency (voltage)",
            "\tXMP-5600\t\t46.0-45-45-90-135 @ 2800 MHz (1.100 Volts)",
            "EXPO timings table\t\tCL-tRCD-tRP-tRAS-tRC @ frequency (voltage)",
            "\tEXPO-5200\t\t42.0-42-42-84-125 @ 2600 MHz (1.100 Volts)",
            "",
        ];
        assert_eq!(lines[..expected.len()], expected);
        assert_eq!(lines[expected.len()], "DIMM #\t\t\t\t2");
        assert_eq!(lines[expected.len() + 1], "\tSlot\t\t\tSlot #4 (DIMMB2)");
        assert_eq!(lines.len(), 2 * expected.len());
    }

    #[test]
    fn names_the_board_for_monitoring() {
        assert_eq!(
            lines_of(&monitoring(&demo::system().mainboard, &[])),
            ["Mainboard Model\t\tB650 EXAMPLE WIFI"]
        );
        assert!(lines_of(&monitoring(&MainboardInfo::default(), &[])).is_empty());
    }

    #[test]
    fn lists_the_sensors_by_what_they_belong_to() {
        let sensor = |group: &str, name: &str, kind, value| Sensor {
            group: group.into(),
            name: name.into(),
            kind,
            value,
        };
        let sensors = [
            sensor("CPU", "Package", SensorKind::Power, 21.5),
            sensor("CPU", "Cores", SensorKind::Power, 14.6),
            sensor("ACPI", "TZ00", SensorKind::Temperature, 56.05),
        ];
        let board = MainboardInfo { model: "0XD433".into(), ..MainboardInfo::default() };
        let lines: Vec<String> = lines_of(&monitoring(&board, &sensors))
            .iter()
            .map(|line| line.split('\t').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("|"))
            .collect();
        assert_eq!(
            lines,
            [
                "Mainboard Model|0XD433",
                "",
                "Hardware monitor|CPU",
                "Package|21.50 W",
                "Cores|14.60 W",
                "Hardware monitor|ACPI",
                "TZ00|56.0 °C",
            ]
        );
    }

    #[test]
    fn names_the_slot() {
        let module = |slot: &str, locator: &str| MemoryModule {
            slot: slot.into(),
            locator: locator.into(),
            ..Default::default()
        };
        assert_eq!(slot_text(&module("Slot #1", "DIMM 0")), "Slot #1 (DIMM 0)");
        assert_eq!(slot_text(&module("Slot #1", "")), "Slot #1");
        assert_eq!(slot_text(&module("", "ChannelA-DIMM0")), "ChannelA-DIMM0");
        assert_eq!(slot_text(&module("", "")), "");
    }

    #[test]
    fn groups_profiles_by_family() {
        assert_eq!(family("JEDEC #10"), "JEDEC");
        assert_eq!(family("XMP-6000"), "XMP");
        assert_eq!(family("EXPO-5200"), "EXPO");
        assert_eq!(family("Custom"), "Custom");

        let mut out = Lines::default();
        timing_tables(
            &mut out,
            &[profile("JEDEC #1", 1366.4), profile("JEDEC #2", 1633.0), profile("XMP-3200", 1600.0)],
        );
        assert_eq!(
            lines_of(&out.into_section("Test")),
            [
                "JEDEC timings table\t\tCL-tRCD-tRP-tRAS-tRC @ frequency",
                "\tJEDEC #1\t\t22.0-22-22-44-66 @ 1366 MHz",
                "\tJEDEC #2\t\t22.0-22-22-44-66 @ 1633 MHz",
                "XMP timings table\t\tCL-tRCD-tRP-tRAS-tRC @ frequency",
                "\tXMP-3200\t\t22.0-22-22-44-66 @ 1600 MHz",
            ]
        );
    }

    #[test]
    fn marks_unknown_timings() {
        let xmp = TimingProfile {
            command_rate: Some("2T".into()),
            voltage: Some(1.35),
            trc: None,
            ..profile("XMP-3200", 1600.0)
        };
        assert_eq!(timing_row(&xmp, true, true), "22.0-22-22-44-n.a-2T @ 1600 MHz (1.350 Volts)");
        let bare = TimingProfile { name: "XMP-3600".into(), ..Default::default() };
        assert_eq!(timing_row(&bare, true, true), "n.a-n.a-n.a-n.a-n.a-n.a @ n.a (n.a)");
        assert_eq!(timing_row(&bare, false, false), "n.a-n.a-n.a-n.a-n.a @ n.a");
    }
}
