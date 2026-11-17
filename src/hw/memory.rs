//! System memory summary and per-slot module details.
//!
//! A module's SPD EEPROM and the memory controller's registers are only reachable
//! from ring 0, so everything here comes from the firmware's SMBIOS memory tables:
//! what sits in each slot, how large it is, its rated and configured speed. The
//! active timings, the DRAM vendor, the manufacturing date and the XMP / EXPO
//! profiles stay unknown.

mod channels;
mod dmi;
mod jedec;
mod speed;

use std::mem::size_of;

use windows::Win32::System::SystemInformation::{
    GetPhysicallyInstalledSystemMemory, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};

use crate::hw::clocks;
use crate::hw::smbios::Smbios;
use crate::model::{MemoryInfo, MemoryModule};
use dmi::{MemoryType, Slot};
use speed::DataRate;

/// Returns the memory summary and one entry per physical slot.
pub fn detect(smbios: Option<&Smbios>) -> (MemoryInfo, Vec<MemoryModule>) {
    let slots = smbios.map(dmi::slots).unwrap_or_default();
    let mut info = summarize(&slots, smbios.and_then(clocks::firmware_reference_mhz));
    if info.size_mb == 0 {
        info.size_mb = os_memory_mb().unwrap_or(0);
    }
    (info, slots.into_iter().map(|slot| slot.module).collect())
}

/// Builds the summary from the installed modules and the bus clock the firmware
/// states, if any. The controller frequency, the DC mode and the timings need the
/// memory controller's registers and stay unset.
fn summarize(slots: &[Slot], firmware_bus_mhz: Option<u16>) -> MemoryInfo {
    let installed: Vec<&Slot> = slots.iter().filter(|slot| slot.module.populated).collect();
    let kind = installed.iter().find_map(|slot| slot.kind);
    let double_data_rate = kind.filter(|kind| kind.is_double_data_rate());
    let rate = double_data_rate
        .and_then(|_| installed.iter().find_map(|slot| slot.module.configured_speed))
        .map(DataRate::from_mts);
    // The slowest module bounds them all. A rating below the rate in use is the
    // firmware restating a default, not a maximum.
    let max_rate = double_data_rate
        .and_then(|_| installed.iter().filter_map(|slot| slot.rated_speed).min())
        .map(DataRate::from_mts)
        .filter(|&max| rate.map_or(true, |rate| max >= rate));
    let labels = installed.iter().map(|slot| (slot.module.locator.as_str(), slot.bank_locator.as_str()));
    MemoryInfo {
        kind: kind.map(|kind| kind.name().to_string()),
        size_mb: installed.iter().map(|slot| slot.module.size_mb).sum(),
        channels: kind
            .zip(channels::populated(labels))
            .and_then(|(kind, count)| channels::describe(kind, count)),
        dram_mhz: rate.map(DataRate::clock_mhz),
        max_dram_mhz: max_rate.map(DataRate::clock_mhz),
        fsb_dram: rate
            .filter(|_| kind.is_some_and(|kind| has_nominal_base_clock(kind, firmware_bus_mhz)))
            .and_then(DataRate::base_clock_ratio),
        ..MemoryInfo::default()
    }
}

/// Whether the FSB:DRAM ratio may be stated against the nominal 100 MHz base clock.
///
/// Platforms for these memory types derive the DRAM clock from a base clock of
/// nominally 100 MHz; DDR3 and older also ran on 133, 166 and 200 MHz buses. The
/// base clock itself is out of reach from user mode, so a system on another one
/// (the 125 and 167 MHz straps of LGA2011-3, a raised base clock) is recognised
/// only when its firmware reports that as the processor's bus clock. The ratio is
/// then withheld rather than recomputed, because the processor's bus clock is not
/// on every platform the clock the memory controller multiplies. Firmware that
/// reports 100 MHz whatever the strap leaves the nominal ratio standing.
fn has_nominal_base_clock(kind: MemoryType, firmware_bus_mhz: Option<u16>) -> bool {
    matches!(kind, MemoryType::Ddr4 | MemoryType::Ddr5 | MemoryType::Lpddr4 | MemoryType::Lpddr5)
        && firmware_bus_mhz.map_or(true, |mhz| mhz == speed::BASE_CLOCK_MHZ)
}

/// Installed memory in MB as Windows sees it, for systems whose SMBIOS lists no
/// modules: the amount the firmware declares, else the memory the OS can use.
fn os_memory_mb() -> Option<u64> {
    let mut kilobytes = 0u64;
    // SAFETY: the call writes a single u64 through a valid pointer to a local.
    let declared = unsafe { GetPhysicallyInstalledSystemMemory(&mut kilobytes) };
    if declared.is_ok() && kilobytes != 0 {
        return Some(kilobytes / 1024);
    }
    let mut status = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // SAFETY: `status` is a live MEMORYSTATUSEX whose dwLength is set, as the API requires.
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    Some(status.ullTotalPhys / (1024 * 1024))
}

#[cfg(test)]
mod tests {
    use super::dmi::fixtures::{array, parse, smbios, system, table, Device};
    use super::*;

    /// A DDR4 SODIMM of `size_mb` running at `configured` MT/s.
    fn ddr4(locator: &str, bank: &str, size_mb: u16, configured: u16) -> Device {
        Device::ddr5(locator, bank)
            .word(0x0C, size_mb)
            .byte(0x0E, 0x0D)
            .byte(0x12, 0x1A)
            .word(0x20, configured)
    }

    /// A populated processor socket (type 4) whose external clock is `mhz`.
    fn socket(mhz: u16) -> Vec<u8> {
        let mut structure = vec![0u8; 0x1A];
        structure[0] = 4;
        structure[1] = 0x1A;
        structure[0x12..0x14].copy_from_slice(&mhz.to_le_bytes());
        structure[0x18] = 0x41;
        // The empty string set.
        structure.extend_from_slice(&[0, 0]);
        structure
    }

    fn summary(devices: &[Device]) -> MemoryInfo {
        summarize(&dmi::slots(&system(devices)), None)
    }

    #[test]
    fn summarizes_a_dual_channel_ddr5_system() {
        let info = summary(&[
            Device::empty("DIMMA1", "P0 CHANNEL A"),
            Device::ddr5("DIMMA2", "P0 CHANNEL A"),
            Device::empty("DIMMB1", "P0 CHANNEL B"),
            Device::ddr5("DIMMB2", "P0 CHANNEL B"),
        ]);
        assert_eq!(info.kind.as_deref(), Some("DDR5"));
        assert_eq!(info.size_mb, 32 * 1024);
        assert_eq!(info.channels.as_deref(), Some("4 x 32-bit"));
        assert_eq!(info.dram_mhz, Some(2600.0));
        assert_eq!(info.max_dram_mhz, Some(2800.0));
        assert_eq!(info.fsb_dram.as_deref(), Some("1:26"));
    }

    #[test]
    fn slowest_module_sets_the_maximum_frequency() {
        // Both modules run at 4800 MT/s, the most the slower one takes.
        let rated = |locator: &str, mts: u16| Device::ddr5(locator, "").word(0x15, mts).word(0x20, 4800);
        let info = summary(&[rated("DIMM_A1", 6000), rated("DIMM_B1", 4800)]);
        assert_eq!((info.dram_mhz, info.max_dram_mhz), (Some(2400.0), Some(2400.0)));
        // A module without a rating does not bound the others.
        let info = summary(&[rated("DIMM_A1", 0), rated("DIMM_B1", 5600)]);
        assert_eq!(info.max_dram_mhz, Some(2800.0));
        assert!(summary(&[rated("DIMM_A1", 0)]).max_dram_mhz.is_none());
    }

    #[test]
    fn rating_below_the_rate_in_use_is_no_maximum() {
        // Firmware that reports the JEDEC default as the rating of an overclocked module.
        let overclocked = Device::ddr5("DIMM_A1", "").word(0x15, 4800).word(0x20, 6000);
        let info = summary(&[overclocked]);
        assert_eq!(info.dram_mhz, Some(3000.0));
        assert!(info.max_dram_mhz.is_none());
        // Without a rate in use there is nothing for the rating to fall short of.
        let idle = Device::ddr5("DIMM_A1", "").word(0x20, 0);
        assert_eq!(summary(&[idle]).max_dram_mhz, Some(2800.0));
    }

    #[test]
    fn leaves_controller_data_unset() {
        let info = summary(&[Device::ddr5("DIMMA2", "P0 CHANNEL A")]);
        assert!(info.controller_mhz.is_none() && info.dc_mode.is_none());
        assert_eq!(info.timings, Default::default());
    }

    #[test]
    fn single_ddr5_module_fills_one_channel() {
        let info =
            summary(&[Device::ddr5("DIMMA2", "P0 CHANNEL A"), Device::empty("DIMMB2", "P0 CHANNEL B")]);
        assert_eq!(info.size_mb, 16 * 1024);
        assert_eq!(info.channels.as_deref(), Some("2 x 32-bit"));
    }

    #[test]
    fn summarizes_ddr4_with_a_fractional_speed_grade() {
        let info = summary(&[
            ddr4("ChannelA-DIMM0", "BANK 0", 8192, 2667),
            ddr4("ChannelB-DIMM0", "BANK 2", 8192, 2667),
        ]);
        assert_eq!(info.kind.as_deref(), Some("DDR4"));
        assert_eq!(info.size_mb, 16 * 1024);
        assert_eq!(info.channels.as_deref(), Some("Dual"));
        assert!(info.dram_mhz.is_some_and(|mhz| (mhz - 4000.0 / 3.0).abs() < 1e-9));
        assert_eq!(info.fsb_dram.as_deref(), Some("3:40"));
    }

    #[test]
    fn unlabelled_slots_leave_the_channel_count_unknown() {
        let info = summary(&[ddr4("DIMM1", "", 8192, 3200), ddr4("DIMM2", "", 8192, 3200)]);
        assert_eq!(info.size_mb, 16 * 1024);
        assert!(info.channels.is_none());
        assert_eq!(info.fsb_dram.as_deref(), Some("1:16"));
    }

    #[test]
    fn processor_inside_the_slot_label_keeps_the_channels_apart() {
        let info = summary(&[Device::ddr5("DIMM_P0_A0", "BANK 0"), Device::ddr5("DIMM_P0_B0", "BANK 1")]);
        assert_eq!(info.channels.as_deref(), Some("4 x 32-bit"));
    }

    #[test]
    fn firmware_bus_clock_off_nominal_withholds_the_bus_ratio() {
        let modules = [ddr4("DIMM_A1", "", 8192, 3000), ddr4("DIMM_B1", "", 8192, 3000)];
        let info = |bus_mhz| detect(Some(&smbios(&[socket(bus_mhz), array(0x1000, 0x03, 0x03)], &modules))).0;
        assert_eq!(info(100).fsb_dram.as_deref(), Some("1:15"));
        // On the 125 MHz strap this DRAM clock is twelve times the base clock, not fifteen.
        let strapped = info(125);
        assert_eq!(strapped.dram_mhz, Some(1500.0));
        assert!(strapped.fsb_dram.is_none());
        assert!(info(167).fsb_dram.is_none() && info(103).fsb_dram.is_none());
        // An unknown or placeholder clock does not contradict the nominal one.
        assert_eq!(info(0).fsb_dram.as_deref(), Some("1:15"));
        assert_eq!(info(8000).fsb_dram.as_deref(), Some("1:15"));
        assert_eq!(summary(&modules).fsb_dram.as_deref(), Some("1:15"));
    }

    #[test]
    fn older_generations_get_a_clock_but_no_bus_ratio() {
        let ddr3 = ddr4("DIMM_A1", "", 4096, 1600).byte(0x12, 0x18);
        let info = summary(&[ddr3]);
        assert_eq!(info.kind.as_deref(), Some("DDR3"));
        assert_eq!(info.channels.as_deref(), Some("Single"));
        assert_eq!(info.dram_mhz, Some(800.0));
        assert!(info.fsb_dram.is_none());
    }

    #[test]
    fn missing_configured_speed_leaves_the_clock_unknown() {
        let info = summary(&[ddr4("DIMM_A1", "", 4096, 0)]);
        assert_eq!(info.size_mb, 4096);
        assert!(info.dram_mhz.is_none() && info.fsb_dram.is_none());
    }

    #[test]
    fn single_data_rate_memory_has_no_ddr_clock() {
        let sdram = ddr4("DIMM_A1", "", 256, 133).byte(0x12, 0x0F);
        let info = summary(&[sdram]);
        assert_eq!(info.kind.as_deref(), Some("SDRAM"));
        assert!(info.dram_mhz.is_none() && info.fsb_dram.is_none() && info.channels.is_none());
        assert!(info.max_dram_mhz.is_none());
    }

    #[test]
    fn no_installed_modules_yield_an_empty_summary() {
        for devices in [vec![], vec![Device::empty("DIMM 0", "P0 CHANNEL A")]] {
            let info = summary(&devices);
            assert!(info.kind.is_none() && info.channels.is_none() && info.dram_mhz.is_none());
            assert_eq!(info.size_mb, 0);
        }
    }

    #[test]
    fn detect_returns_the_modules_in_firmware_order() {
        let table =
            system(&[Device::empty("DIMMA1", "P0 CHANNEL A"), Device::ddr5("DIMMA2", "P0 CHANNEL A")]);
        let (info, modules) = detect(Some(&table));
        assert_eq!(info.size_mb, 16 * 1024);
        let slots: Vec<_> =
            modules.iter().map(|m| (m.slot.as_str(), m.locator.as_str(), m.populated)).collect();
        assert_eq!(slots, [("Slot #1", "DIMMA1", false), ("Slot #2", "DIMMA2", true)]);
    }

    #[test]
    fn survives_truncated_and_corrupted_tables() {
        let devices = [
            Device::empty("DIMMA1", "P0 CHANNEL A"),
            Device::ddr5("DIMMA2", "P0 CHANNEL A").word(0x0C, 0x7FFF).word(0x15, 0xFFFF).word(0x20, 0xFFFF),
        ];
        let table = table(&[array(0x1000, 0x03, 0x06)], &devices);
        for length in 0..=table.len() {
            summarize(&dmi::slots(&parse(&table[..length])), None);
        }
        // Flip every byte in turn, then overwrite it with each extreme.
        for index in 0..table.len() {
            for value in [!table[index], 0x00, 0x7F, 0xFF] {
                let mut corrupted = table.clone();
                corrupted[index] = value;
                summarize(&dmi::slots(&parse(&corrupted)), None);
            }
        }
    }

    #[test]
    fn detect_falls_back_to_the_os_for_the_size() {
        let (info, modules) = detect(None);
        assert!(modules.is_empty());
        assert!(info.kind.is_none() && info.channels.is_none() && info.dram_mhz.is_none());
        assert!(info.max_dram_mhz.is_none());
        assert_eq!(Some(info.size_mb), os_memory_mb());
        assert!(info.size_mb > 0);
    }
}
