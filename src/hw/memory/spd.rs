//! The SPD data of the memory modules, read through the PawnIO driver's SMBus module.
//!
//! Reading only: no page is ever selected. A DDR4 module shows the page its EEPROM has
//! selected (normally the first, with the timings); a DDR5 module's hub shows the page
//! its page register holds, which the firmware chose. Whatever that page holds is
//! decoded: the base page gives the JEDEC timings, the manufacturing page the makers,
//! date, serial and part numbers. Platforms that block writes to the SPD (most current
//! Intel boards do) could not select another page anyway.

use super::jedec;
use crate::hw::pawnio::{Smbus, Transport};
use crate::model::{MemoryModule, TimingProfile};

/// The SMBus addresses of the SPD devices of up to eight modules.
const ADDRESSES: std::ops::RangeInclusive<u8> = 0x50..=0x57;
/// What a DDR5 SPD hub answers for its device type registers MR0 and MR1.
const SPD5118: (u8, u8) = (0x51, 0x18);
/// The hub's page register MR11: the page in bits 2:0, two-byte addressing in bit 3.
const MR11: u8 = 0x0B;

/// What one module's SPD gave.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Spd {
    pub registered: Option<bool>,
    pub buffered: Option<bool>,
    /// The timings at the module's highest JEDEC speed.
    pub jedec: Option<TimingProfile>,
    pub manufacturer: Option<&'static str>,
    pub dram_manufacturer: Option<&'static str>,
    /// Manufacturing week and two-digit year.
    pub week_year: Option<(u8, u8)>,
    pub serial: Option<String>,
    pub part_number: Option<String>,
}

/// Reads the SPD of every module that answers, each while holding the bus lock `lock`
/// takes. A module whose lock does not come is left out.
pub fn read<T: Transport, L>(smbus: &Smbus<T>, lock: impl Fn() -> Option<L>) -> Vec<(u8, Spd)> {
    ADDRESSES
        .filter_map(|address| {
            let _held = lock()?;
            Some((address, read_module(smbus, address)?))
        })
        .collect()
}

fn read_module<T: Transport>(smbus: &Smbus<T>, address: u8) -> Option<Spd> {
    let identity = (smbus.read_byte(address, 0)?, smbus.read_byte(address, 1)?);
    if identity == SPD5118 {
        let page_register = smbus.read_byte(address, MR11)?;
        // With two-byte addressing the hub does not answer offset reads like these.
        if page_register & 0x08 != 0 {
            return Some(Spd::default());
        }
        let page: Vec<u8> =
            (0x80..=0xFF).map(|offset| smbus.read_byte(address, offset)).collect::<Option<_>>()?;
        return Some(ddr5_page(page_register & 0x07, &page));
    }
    let page: Vec<u8> = (0..0x80).map(|offset| smbus.read_byte(address, offset)).collect::<Option<_>>()?;
    Some(ddr4_base(&page))
}

/// Decodes page `page` (128 bytes) of a DDR5 SPD: page 0 holds the base configuration
/// and the timings, page 4 the manufacturing data. Any other page says nothing here.
pub fn ddr5_page(page: u8, bytes: &[u8]) -> Spd {
    if bytes.len() < 128 {
        return Spd::default();
    }
    let word = |at: usize| u32::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]));
    match page {
        0 if bytes[2] == 0x12 => {
            let (registered, buffered) = module_type(bytes[3]);
            let tck = word(20);
            // Bytes 24 to 28: bit n stands for CAS latency 20 + 2n (DDR5 has only even ones).
            let mut mask = [0u8; 8];
            mask[..5].copy_from_slice(&bytes[24..29]);
            let latencies = cas_latencies(u64::from_le_bytes(mask), 40, 20, 2);
            let minimums = [word(30), word(32), word(34), word(36), word(38)];
            let jedec = profile(tck, minimums, &latencies, 1.1);
            Spd { registered, buffered, jedec, ..Spd::default() }
        }
        4 => Spd {
            manufacturer: jedec::vendor(&bytes[0..2]),
            week_year: bcd(bytes[4]).zip(bcd(bytes[3])),
            serial: Some(bytes[5..9].iter().map(|byte| format!("{byte:02X}")).collect()),
            part_number: text(&bytes[9..39]),
            dram_manufacturer: jedec::vendor(&bytes[40..42]),
            ..Spd::default()
        },
        _ => Spd::default(),
    }
}

/// Decodes the base page (first 128 bytes) of a DDR4 SPD: module type and timings, in
/// medium time base units of 125 ps corrected by signed fine time base bytes of 1 ps.
pub fn ddr4_base(bytes: &[u8]) -> Spd {
    if bytes.len() < 128 || bytes[2] != 0x0C {
        return Spd::default();
    }
    let fine = |at: usize| i32::from(bytes[at] as i8);
    let time = |medium: u32, fine_at: usize| u32::try_from(medium as i32 * 125 + fine(fine_at)).unwrap_or(0);
    let tck = time(u32::from(bytes[18]), 125);
    let tras = ((u32::from(bytes[27] & 0x0F) << 8) | u32::from(bytes[28])) * 125;
    let trc = time((u32::from(bytes[27] >> 4) << 8) | u32::from(bytes[29]), 120);
    // Bytes 20 to 23: bit n stands for CAS latency 7 + n, or 23 + n when bit 31 is set.
    let mask = u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    let first = if mask >> 31 == 1 { 23 } else { 7 };
    let latencies = cas_latencies(u64::from(mask), 30, first, 1);
    let minimums =
        [time(bytes[24].into(), 123), time(bytes[25].into(), 122), time(bytes[26].into(), 121), tras, trc];
    let jedec = profile(tck, minimums, &latencies, 1.2);
    let (registered, buffered) = module_type(bytes[3]);
    Spd { registered, buffered, jedec, ..Spd::default() }
}

/// The JEDEC profile at cycle time `tck` from the minimum tAA, tRCD, tRP, tRAS and tRC in
/// picoseconds, at the lowest of the module's CAS latencies `latencies` that covers tAA.
fn profile(tck: u32, minimums: [u32; 5], latencies: &[u32], volts: f32) -> Option<TimingProfile> {
    let [taa, trcd, trp, tras, trc] = minimums;
    if tck == 0 || taa == 0 {
        return None;
    }
    Some(TimingProfile {
        name: "JEDEC".into(),
        frequency_mhz: Some(clock_mhz(tck)),
        cas_latency: Some(cas_latency(clocks(taa, tck), latencies) as f32),
        trcd: Some(clocks(trcd, tck)),
        trp: Some(clocks(trp, tck)),
        tras: Some(clocks(tras, tck)),
        trc: Some(clocks(trc, tck)),
        command_rate: None,
        voltage: Some(volts),
    })
}

/// The clock frequency at cycle time `tck_ps`, on the grid of 100/3 MHz that every DDR4
/// and DDR5 speed grade sits on: the cycle time is given in whole picoseconds, so 416 ps
/// stands for 2400 MHz rather than 2403.85.
fn clock_mhz(tck_ps: u32) -> f64 {
    let step = 100.0 / 3.0;
    (1e6 / f64::from(tck_ps) / step).round() * step
}

/// A minimum time in clocks, by the SPD rounding rule: up, after allowing 2.6 % slack.
fn clocks(time_ps: u32, tck_ps: u32) -> u32 {
    (f64::from(time_ps) / f64::from(tck_ps) + 0.974).floor() as u32
}

/// The CAS latencies set in the first `count` bits of `mask`, bit n standing for
/// `first + step * n`.
fn cas_latencies(mask: u64, count: u32, first: u32, step: u32) -> Vec<u32> {
    (0..count).filter(|bit| (mask >> bit) & 1 == 1).map(|bit| first + step * bit).collect()
}

/// The lowest supported CAS latency of at least `minimum` clocks; `minimum` itself when
/// the module lists none that long.
fn cas_latency(minimum: u32, supported: &[u32]) -> u32 {
    supported.iter().copied().filter(|&latency| latency >= minimum).min().unwrap_or(minimum)
}

/// Registered and buffered, from the module type (low nibble of byte 3): RDIMM and
/// LRDIMM are both; UDIMM and SODIMM neither.
fn module_type(byte: u8) -> (Option<bool>, Option<bool>) {
    match byte & 0x0F {
        0x01 | 0x04 => (Some(true), Some(true)),
        0x02 | 0x03 => (Some(false), Some(false)),
        _ => (None, None),
    }
}

/// A binary-coded decimal byte, e.g. 0x24 -> 24.
fn bcd(byte: u8) -> Option<u8> {
    let (tens, ones) = (byte >> 4, byte & 0x0F);
    (tens < 10 && ones < 10).then_some(tens * 10 + ones)
}

/// ASCII text with its padding removed.
fn text(bytes: &[u8]) -> Option<String> {
    let text: String =
        bytes.iter().map(|&byte| if byte.is_ascii_graphic() { char::from(byte) } else { ' ' }).collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Adds what the SPDs gave to the slots they belong to: by serial number where the SPD
/// states one that a slot has, then in order to the populated slots left over.
pub fn apply(modules: &mut [MemoryModule], spds: &[(u8, Spd)]) {
    let mut taken = vec![false; modules.len()];
    let mut placed: Vec<(usize, &Spd)> = Vec::new();
    let mut unplaced: Vec<&Spd> = Vec::new();
    for (_, spd) in spds {
        let by_serial = spd.serial.as_deref().and_then(|serial| {
            modules.iter().enumerate().position(|(index, module)| {
                !taken[index]
                    && module
                        .serial_number
                        .as_deref()
                        .is_some_and(|known| known.trim().eq_ignore_ascii_case(serial))
            })
        });
        match by_serial {
            Some(index) => {
                taken[index] = true;
                placed.push((index, spd));
            }
            None => unplaced.push(spd),
        }
    }
    for spd in unplaced {
        if let Some(index) = (0..modules.len()).find(|&index| !taken[index] && modules[index].populated) {
            taken[index] = true;
            placed.push((index, spd));
        }
    }
    for (index, spd) in placed {
        let module = &mut modules[index];
        module.registered = module.registered.or(spd.registered);
        module.buffered = module.buffered.or(spd.buffered);
        module.manufacturer = module.manufacturer.take().or(spd.manufacturer.map(str::to_string));
        module.dram_manufacturer =
            spd.dram_manufacturer.map(str::to_string).or(module.dram_manufacturer.take());
        module.week_year = spd.week_year.or(module.week_year);
        module.part_number = module.part_number.take().or_else(|| spd.part_number.clone());
        module.profiles.extend(spd.jedec.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(bytes: &mut [u8], at: usize, value: u16) {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    /// Page 0 of a DDR5-4800B unbuffered module at its JEDEC timings (picoseconds): tAA
    /// 16.0 ns, CAS latencies 22, 26 to 32, 36 and 40.
    fn ddr5_base() -> Vec<u8> {
        let mut page = vec![0u8; 128];
        page[2] = 0x12;
        page[3] = 0x02;
        page[24..26].copy_from_slice(&[0x7A, 0x05]);
        for (at, ps) in [(20, 416), (30, 16_000), (32, 16_000), (34, 16_000), (36, 32_000), (38, 48_000)] {
            word(&mut page, at, ps);
        }
        page
    }

    /// Page 4 of the same module: the manufacturing data of this PC's Samsung modules.
    fn ddr5_manufacturing() -> Vec<u8> {
        let mut page = vec![0x20u8; 128];
        page[0..2].copy_from_slice(&[0x80, 0xCE]);
        page[2] = 0x01;
        page[3..5].copy_from_slice(&[0x23, 0x41]);
        page[5..9].copy_from_slice(&[0x47, 0x50, 0x8C, 0x97]);
        page[9..9 + 18].copy_from_slice(b"M323R2GA3BB0-CQKOL");
        page[40..42].copy_from_slice(&[0x80, 0xCE]);
        page
    }

    #[test]
    fn decodes_the_base_page_of_a_ddr5_module() {
        let spd = ddr5_page(0, &ddr5_base());
        assert_eq!((spd.registered, spd.buffered), (Some(false), Some(false)));
        let jedec = spd.jedec.unwrap();
        assert!(jedec.frequency_mhz.is_some_and(|mhz| (mhz - 2400.0).abs() < 0.01), "{jedec:?}");
        // tAA spans 39 clocks, but DDR5 runs only even latencies: the module's next is 40.
        assert_eq!(jedec.cas_latency, Some(40.0));
        assert_eq!((jedec.trcd, jedec.trp, jedec.tras, jedec.trc), (Some(39), Some(39), Some(77), Some(116)));
        assert_eq!(jedec.voltage, Some(1.1));
        assert_eq!(spd.serial, None);
    }

    #[test]
    fn decodes_the_manufacturing_page_of_a_ddr5_module() {
        let spd = ddr5_page(4, &ddr5_manufacturing());
        assert_eq!(spd.manufacturer, Some("Samsung"));
        assert_eq!(spd.dram_manufacturer, Some("Samsung"));
        assert_eq!(spd.week_year, Some((41, 23)));
        assert_eq!(spd.serial.as_deref(), Some("47508C97"));
        assert_eq!(spd.part_number.as_deref(), Some("M323R2GA3BB0-CQKOL"));
        assert!(spd.jedec.is_none());
        // Pages that hold neither say nothing.
        assert_eq!(ddr5_page(2, &ddr5_base()), Spd::default());
        assert_eq!(ddr5_page(0, &ddr5_base()[..40]), Spd::default());
    }

    #[test]
    fn decodes_the_base_page_of_a_ddr4_module() {
        // DDR4-2666 at 19-19-19-43: medium time base 125 ps, no fine corrections.
        let mut page = vec![0u8; 128];
        page[2] = 0x0C;
        page[3] = 0x01;
        page[18] = 6;
        page[24] = 110;
        page[25] = 110;
        page[26] = 110;
        page[27] = 0x11;
        page[28] = 0x00;
        page[29] = 110;
        // CAS latencies 10 to 20.
        page[20..22].copy_from_slice(&[0xF8, 0x3F]);
        let spd = ddr4_base(&page);
        assert_eq!((spd.registered, spd.buffered), (Some(true), Some(true)));
        let jedec = spd.jedec.unwrap();
        assert!(jedec.frequency_mhz.is_some_and(|mhz| (mhz - 1333.33).abs() < 0.01), "{jedec:?}");
        assert_eq!(jedec.cas_latency, Some(19.0));
        assert_eq!((jedec.trcd, jedec.trp, jedec.tras, jedec.trc), (Some(19), Some(19), Some(43), Some(61)));
        assert_eq!(jedec.voltage, Some(1.2));
        // Without 19 among them the module runs the next one up.
        page[21] = 0x2F;
        assert_eq!(ddr4_base(&page).jedec.unwrap().cas_latency, Some(20.0));
        // A page that is no DDR4 base page.
        page[2] = 0x12;
        assert_eq!(ddr4_base(&page), Spd::default());
    }

    #[test]
    fn rounds_the_clock_to_the_speed_grades() {
        // DDR5-4800, DDR4-2666, DDR5-5600 and DDR4-2933.
        for (tck, mhz) in [(416, 2400.0), (750, 1333.33), (357, 2800.0), (682, 1466.67)] {
            assert!((clock_mhz(tck) - mhz).abs() < 0.01, "{tck} ps: {}", clock_mhz(tck));
        }
    }

    #[test]
    fn picks_the_lowest_supported_cas_latency_that_covers_taa() {
        assert_eq!(cas_latencies(0b1011, 30, 7, 1), [7, 8, 10]);
        // DDR4's high range starts at 23; bit 31 itself is no latency.
        assert_eq!(cas_latencies(u64::from(0x8000_0001_u32), 30, 23, 1), [23]);
        assert_eq!(cas_latencies(1 << 39, 40, 20, 2), [98]);
        assert_eq!(cas_latency(39, &[22, 36, 40, 42]), 40);
        assert_eq!(cas_latency(40, &[22, 36, 40, 42]), 40);
        // A module that lists nothing long enough, or nothing at all, keeps the minimum.
        assert_eq!(cas_latency(44, &[22, 40]), 44);
        assert_eq!(cas_latency(39, &[]), 39);
    }

    #[test]
    fn spd_data_joins_the_slot_with_its_serial_number() {
        let module = |serial: &str| MemoryModule {
            populated: true,
            serial_number: Some(serial.into()),
            ..MemoryModule::default()
        };
        let mut modules = vec![MemoryModule::default(), module("03403664"), module("47508C97")];
        let first = ddr5_page(4, &ddr5_manufacturing());
        let base = ddr5_page(0, &ddr5_base());
        apply(&mut modules, &[(0x50, first), (0x52, base)]);
        // The serial number places the first module; the second takes the populated slot left.
        assert_eq!(modules[2].dram_manufacturer.as_deref(), Some("Samsung"));
        assert_eq!(modules[2].week_year, Some((41, 23)));
        assert_eq!(modules[1].profiles.len(), 1);
        assert_eq!(modules[1].registered, Some(false));
        assert!(modules[0].profiles.is_empty() && modules[0].dram_manufacturer.is_none());
    }
}
