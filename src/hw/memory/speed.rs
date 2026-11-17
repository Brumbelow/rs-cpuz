//! DDR data rates: the DRAM clock, its ratio to the base clock and the bandwidth
//! label of a module.

use super::dmi::MemoryType;

/// Nominal base clock of the platforms that state a ratio, in MHz.
pub const BASE_CLOCK_MHZ: u16 = 100;

/// JEDEC module ratings of the first three DDR generations, by data rate in MT/s.
const DDR_RATINGS: &[(u32, &str)] = &[(200, "PC1600"), (266, "PC2100"), (333, "PC2700"), (400, "PC3200")];
const DDR2_RATINGS: &[(u32, &str)] =
    &[(400, "PC2-3200"), (533, "PC2-4200"), (667, "PC2-5300"), (800, "PC2-6400"), (1066, "PC2-8500")];
const DDR3_RATINGS: &[(u32, &str)] = &[
    (800, "PC3-6400"),
    (1066, "PC3-8500"),
    (1333, "PC3-10600"),
    (1600, "PC3-12800"),
    (1866, "PC3-14900"),
    (2133, "PC3-17000"),
];

/// A DDR data rate, held in thirds of MT/s. Many speed grades are not whole numbers
/// (2666.67 MT/s) and reach us truncated or rounded; counting in thirds restores the
/// exact figure, so the clock and the ratios derived from it come out right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DataRate(u64);

impl DataRate {
    /// Takes a rate as firmware reports it: `2666` and `2667` both mean 2666.67 MT/s.
    pub fn from_mts(mts: u32) -> Self {
        let thirds = u64::from(mts) * 3;
        DataRate(match mts % 100 {
            33 => thirds + 1,
            66 => thirds + 2,
            67 => thirds - 1,
            _ => thirds,
        })
    }

    /// The rate in whole MT/s, truncated the way speed grades are named.
    fn mts(self) -> u64 {
        self.0 / 3
    }

    /// DRAM clock in MHz: half the data rate.
    pub fn clock_mhz(self) -> f64 {
        self.0 as f64 / 6.0
    }

    /// DRAM clock to the nearest MHz.
    fn clock_mhz_rounded(self) -> u64 {
        (self.0 + 3) / 6
    }

    /// Ratio of a 100 MHz base clock to the DRAM clock, e.g. `1:26` or `3:40`.
    /// `None` when the clock is not a multiple of a third of the base clock.
    pub fn base_clock_ratio(self) -> Option<String> {
        // clock / base = (thirds / 6) / base.
        let base_thirds = 6 * u64::from(BASE_CLOCK_MHZ);
        let divisor = gcd(self.0, base_thirds);
        let (base, dram) = (base_thirds / divisor, self.0 / divisor);
        (dram != 0 && matches!(base, 1 | 3)).then(|| format!("{base}:{dram}"))
    }
}

/// Greatest common divisor; `gcd(0, n)` is `n`.
fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// The "Max Bandwidth" text of a module rated for `mts`: `DDR5-5600 (2800 MHz)`, or
/// the JEDEC module rating for the standard grades of the first three generations,
/// `PC3-12800 (800 MHz)`. `None` for memory that is not double data rate.
pub fn bandwidth(kind: MemoryType, mts: u32) -> Option<String> {
    if !kind.is_double_data_rate() || mts == 0 {
        return None;
    }
    let (generation, ratings) = match kind {
        MemoryType::Ddr => ("DDR", DDR_RATINGS),
        MemoryType::Ddr2 | MemoryType::Ddr2FbDimm => ("DDR2", DDR2_RATINGS),
        MemoryType::Ddr3 => ("DDR3", DDR3_RATINGS),
        other => (other.name(), &[][..]),
    };
    let rate = DataRate::from_mts(mts);
    let rating = ratings
        .iter()
        .find(|&&(grade, _)| DataRate::from_mts(grade) == rate)
        .map_or_else(|| format!("{generation}-{}", rate.mts()), |&(_, rating)| rating.to_string());
    Some(format!("{rating} ({} MHz)", rate.clock_mhz_rounded()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_fractional_speed_grades() {
        assert_eq!(DataRate::from_mts(5200).clock_mhz(), 2600.0);
        assert_eq!(DataRate::from_mts(3200).clock_mhz(), 1600.0);
        assert_eq!(DataRate::from_mts(2666), DataRate::from_mts(2667));
        assert_eq!(DataRate::from_mts(1866), DataRate::from_mts(1867));
        assert_eq!(DataRate::from_mts(1066), DataRate::from_mts(1067));
        assert_ne!(DataRate::from_mts(2666), DataRate::from_mts(2665));
        let clock = DataRate::from_mts(2666).clock_mhz();
        assert!((clock - 4000.0 / 3.0).abs() < 1e-9, "{clock}");
        let clock = DataRate::from_mts(2133).clock_mhz();
        assert!((clock - 3200.0 / 3.0).abs() < 1e-9, "{clock}");
    }

    #[test]
    fn handles_the_whole_input_range() {
        assert_eq!(DataRate::from_mts(0).clock_mhz(), 0.0);
        assert_eq!(DataRate::from_mts(0).base_clock_ratio(), None);
        assert_eq!(DataRate::from_mts(33).clock_mhz_rounded(), 17);
        assert_eq!(DataRate::from_mts(67).mts(), 66);
        assert_eq!(DataRate::from_mts(u32::MAX).mts(), u64::from(u32::MAX));
    }

    #[test]
    fn base_clock_ratios() {
        let ratio = |mts| DataRate::from_mts(mts).base_clock_ratio();
        assert_eq!(ratio(5200).as_deref(), Some("1:26"));
        assert_eq!(ratio(5600).as_deref(), Some("1:28"));
        assert_eq!(ratio(6000).as_deref(), Some("1:30"));
        assert_eq!(ratio(3200).as_deref(), Some("1:16"));
        assert_eq!(ratio(2666).as_deref(), Some("3:40"));
        assert_eq!(ratio(2667).as_deref(), Some("3:40"));
        assert_eq!(ratio(2133).as_deref(), Some("3:32"));
        assert_eq!(ratio(3733).as_deref(), Some("3:56"));
        assert_eq!(ratio(2400).as_deref(), Some("1:12"));
        // 1550 MHz and 2617.5 MHz are not thirds of the base clock.
        assert_eq!(ratio(3100), None);
        assert_eq!(ratio(5235), None);
    }

    #[test]
    fn bandwidth_of_current_generations() {
        assert_eq!(bandwidth(MemoryType::Ddr5, 5600).as_deref(), Some("DDR5-5600 (2800 MHz)"));
        assert_eq!(bandwidth(MemoryType::Ddr5, 4800).as_deref(), Some("DDR5-4800 (2400 MHz)"));
        assert_eq!(bandwidth(MemoryType::Ddr4, 3200).as_deref(), Some("DDR4-3200 (1600 MHz)"));
        assert_eq!(bandwidth(MemoryType::Ddr4, 2666).as_deref(), Some("DDR4-2666 (1333 MHz)"));
        assert_eq!(bandwidth(MemoryType::Ddr4, 2667).as_deref(), Some("DDR4-2666 (1333 MHz)"));
        assert_eq!(bandwidth(MemoryType::Ddr4, 2133).as_deref(), Some("DDR4-2133 (1067 MHz)"));
        assert_eq!(bandwidth(MemoryType::Lpddr4, 4266).as_deref(), Some("LPDDR4-4266 (2133 MHz)"));
        assert_eq!(bandwidth(MemoryType::Lpddr5, 6400).as_deref(), Some("LPDDR5-6400 (3200 MHz)"));
    }

    #[test]
    fn bandwidth_of_legacy_generations_uses_module_ratings() {
        assert_eq!(bandwidth(MemoryType::Ddr3, 1600).as_deref(), Some("PC3-12800 (800 MHz)"));
        assert_eq!(bandwidth(MemoryType::Ddr3, 1333).as_deref(), Some("PC3-10600 (667 MHz)"));
        assert_eq!(bandwidth(MemoryType::Ddr3, 1066).as_deref(), Some("PC3-8500 (533 MHz)"));
        assert_eq!(bandwidth(MemoryType::Ddr3, 1067).as_deref(), Some("PC3-8500 (533 MHz)"));
        assert_eq!(bandwidth(MemoryType::Ddr3, 1867).as_deref(), Some("PC3-14900 (933 MHz)"));
        assert_eq!(bandwidth(MemoryType::Ddr2, 800).as_deref(), Some("PC2-6400 (400 MHz)"));
        assert_eq!(bandwidth(MemoryType::Ddr2, 667).as_deref(), Some("PC2-5300 (333 MHz)"));
        assert_eq!(bandwidth(MemoryType::Ddr2, 533).as_deref(), Some("PC2-4200 (267 MHz)"));
        assert_eq!(bandwidth(MemoryType::Ddr2FbDimm, 667).as_deref(), Some("PC2-5300 (333 MHz)"));
        assert_eq!(bandwidth(MemoryType::Ddr, 400).as_deref(), Some("PC3200 (200 MHz)"));
        assert_eq!(bandwidth(MemoryType::Ddr, 333).as_deref(), Some("PC2700 (167 MHz)"));
        assert_eq!(bandwidth(MemoryType::Ddr, 266).as_deref(), Some("PC2100 (133 MHz)"));
        // Grades outside the JEDEC tables keep the plain form.
        assert_eq!(bandwidth(MemoryType::Ddr3, 2400).as_deref(), Some("DDR3-2400 (1200 MHz)"));
        assert_eq!(bandwidth(MemoryType::Ddr2, 900).as_deref(), Some("DDR2-900 (450 MHz)"));
        assert_eq!(bandwidth(MemoryType::Ddr2FbDimm, 900).as_deref(), Some("DDR2-900 (450 MHz)"));
    }

    #[test]
    fn no_bandwidth_without_a_ddr_rate() {
        assert_eq!(bandwidth(MemoryType::Ddr4, 0), None);
        assert_eq!(bandwidth(MemoryType::Other("SDRAM"), 133), None);
        assert_eq!(bandwidth(MemoryType::Other("RDRAM"), 800), None);
    }
}
