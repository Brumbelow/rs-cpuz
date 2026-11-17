//! Value formatters shared by the report sections.

use crate::hw::cpu::CacheDesc;
use crate::model::{BridgeInfo, CacheInfo, CacheKind, PcieLink};

/// `3.580 MHz`, `1.000 KHz`, `64.000 Hz` - a timer frequency in its natural unit.
pub(super) fn frequency(hz: f64) -> String {
    if hz >= 1e6 {
        format!("{:.3} MHz", hz / 1e6)
    } else if hz >= 1e3 {
        format!("{:.3} KHz", hz / 1e3)
    } else {
        format!("{hz:.3} Hz")
    }
}

/// `L1 Data cache`, `L2 cache`, `Trace cache` - the report label of a cache.
pub(super) fn cache_label(cache: &CacheInfo) -> String {
    match cache.kind {
        CacheKind::Data => format!("L{} Data cache", cache.level),
        CacheKind::Instruction => format!("L{} Instruction cache", cache.level),
        CacheKind::Unified => format!("L{} cache", cache.level),
        CacheKind::Trace => "Trace cache".to_string(),
    }
}

/// `8 x 32 KB (8-way, 64-byte line)` - size, associativity and line size of the
/// caches of one level and kind. A package that mixes sizes there lists them all,
/// `96 MB + 32 MB (16-way, 64-byte line)`, each with its own details should those differ too.
pub(super) fn cache_text(caches: &[CacheInfo]) -> String {
    let sizes: Vec<String> = caches.iter().map(cache_size).collect();
    let details: Vec<String> = caches.iter().map(cache_details).collect();
    if details.windows(2).all(|pair| pair[0] == pair[1]) {
        return format!("{}{}", sizes.join(" + "), details.first().map_or("", String::as_str));
    }
    let each: Vec<String> =
        sizes.iter().zip(&details).map(|(size, detail)| format!("{size}{detail}")).collect();
    each.join(" + ")
}

/// `8 x 32 KB`, `32 MB`, `2 x 12 Kuops` - instance count and size of a cache.
fn cache_size(cache: &CacheInfo) -> String {
    let kb = cache.size_kb;
    let size = if cache.kind == CacheKind::Trace {
        format!("{kb} Kuops")
    } else if kb <= 1024 {
        format!("{kb} KB")
    } else if kb % 1024 == 0 {
        format!("{} MB", kb / 1024)
    } else {
        format!("{:.2} MB", f64::from(kb) / 1024.0)
    };
    if cache.count > 1 {
        format!("{} x {size}", cache.count)
    } else {
        size
    }
}

/// ` (8-way, 64-byte line)` - what is known of the associativity and line size, with the leading space.
fn cache_details(cache: &CacheInfo) -> String {
    let ways = match cache.ways {
        0 => None,
        1 => Some("direct mapped".to_string()),
        u32::MAX => Some("fully associative".to_string()),
        n => Some(format!("{n}-way")),
    };
    let line = (cache.line_size > 0).then(|| format!("{}-byte line", cache.line_size));
    let details: Vec<String> = ways.into_iter().chain(line).collect();
    if details.is_empty() {
        String::new()
    } else {
        format!(" ({})", details.join(", "))
    }
}

/// `Level 2, U, 1 MB, 2 thread(s)` - one cache as the CPUID of a logical processor
/// describes it: level, kind (data, instruction, unified or trace), size and the
/// number of threads it can be shared by, when CPUID gives one.
pub(super) fn cache_descriptor(cache: &CacheDesc) -> String {
    let kind = match cache.kind {
        CacheKind::Data => "D",
        CacheKind::Instruction => "I",
        CacheKind::Unified => "U",
        CacheKind::Trace => "T",
    };
    let kb = cache.size_kb;
    let size = if kb >= 1024 && kb % 1024 == 0 { format!("{} MB", kb / 1024) } else { format!("{kb} KB") };
    match cache.max_sharing {
        Some(threads) => format!("Level {}, {kind}, {size}, {threads} thread(s)", cache.level),
        None => format!("Level {}, {kind}, {size}", cache.level),
    }
}

/// `AMD B650 rev. 51` - a chipset or southbridge line.
pub(super) fn bridge(bridge: &BridgeInfo) -> String {
    let mut text = [bridge.vendor.as_str(), bridge.model.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if !text.is_empty() && !bridge.revision.is_empty() {
        text.push_str(&format!(" rev. {}", bridge.revision));
    }
    text
}

/// `x8 (max 8x)` - the link width of the mainboard's graphic interface.
pub(super) fn graphic_link_width(link: &PcieLink) -> Option<String> {
    let width = link.width?;
    Some(match link.max_width {
        Some(max) => format!("x{width} (max {max}x)"),
        None => format!("x{width}"),
    })
}

/// `8x (max 16x)` - the link width of a PCI device.
pub(super) fn device_link_width(link: &PcieLink) -> Option<String> {
    let width = link.width?;
    Some(match link.max_width {
        Some(max) => format!("{width}x (max {max}x)"),
        None => format!("{width}x"),
    })
}

/// `16.0 GT/s (max 32.0 GT/s)` - current and maximum link speed.
pub(super) fn link_speed(link: &PcieLink) -> Option<String> {
    let speed = link.speed_gts?;
    Some(match link.max_speed_gts {
        Some(max) => format!("{speed:.1} GT/s (max {max:.1} GT/s)"),
        None => format!("{speed:.1} GT/s"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache(level: u8, kind: CacheKind, size_kb: u32, count: u32, ways: u32, line_size: u32) -> CacheInfo {
        CacheInfo { level, kind, size_kb, count, ways, line_size, threads_sharing: 1 }
    }

    #[test]
    fn scales_frequencies() {
        assert_eq!(frequency(3_579_545.0), "3.580 MHz");
        assert_eq!(frequency(10_000_000.0), "10.000 MHz");
        assert_eq!(frequency(1_000.0), "1.000 KHz");
        assert_eq!(frequency(1_024.0), "1.024 KHz");
        assert_eq!(frequency(64.0), "64.000 Hz");
    }

    #[test]
    fn labels_caches() {
        assert_eq!(cache_label(&cache(1, CacheKind::Data, 32, 8, 8, 64)), "L1 Data cache");
        assert_eq!(cache_label(&cache(1, CacheKind::Instruction, 32, 8, 8, 64)), "L1 Instruction cache");
        assert_eq!(cache_label(&cache(2, CacheKind::Unified, 1024, 8, 8, 64)), "L2 cache");
        assert_eq!(cache_label(&cache(1, CacheKind::Trace, 12, 2, 8, 0)), "Trace cache");
    }

    #[test]
    fn describes_caches() {
        assert_eq!(cache_text(&[cache(1, CacheKind::Data, 32, 8, 8, 64)]), "8 x 32 KB (8-way, 64-byte line)");
        assert_eq!(
            cache_text(&[cache(2, CacheKind::Unified, 1024, 8, 8, 64)]),
            "8 x 1024 KB (8-way, 64-byte line)"
        );
        assert_eq!(
            cache_text(&[cache(3, CacheKind::Unified, 32 * 1024, 1, 16, 64)]),
            "32 MB (16-way, 64-byte line)"
        );
        assert_eq!(
            cache_text(&[cache(2, CacheKind::Unified, 1280, 6, 10, 64)]),
            "6 x 1.25 MB (10-way, 64-byte line)"
        );
        assert_eq!(cache_text(&[cache(1, CacheKind::Trace, 12, 2, 8, 0)]), "2 x 12 Kuops (8-way)");
        assert_eq!(
            cache_text(&[cache(1, CacheKind::Data, 16, 1, 1, 32)]),
            "16 KB (direct mapped, 32-byte line)"
        );
        assert_eq!(cache_text(&[cache(1, CacheKind::Data, 8, 1, u32::MAX, 0)]), "8 KB (fully associative)");
        assert_eq!(cache_text(&[cache(2, CacheKind::Unified, 256, 4, 0, 0)]), "4 x 256 KB");
        assert_eq!(cache_text(&[]), "");
    }

    #[test]
    fn describes_the_caches_of_a_thread() {
        let described = |level, kind, size_kb, max_sharing| {
            let cache =
                CacheDesc { level, kind, size_kb, ways: 8, line_size: 64, share_shift: None, max_sharing };
            cache_descriptor(&cache)
        };
        assert_eq!(described(1, CacheKind::Data, 32, Some(2)), "Level 1, D, 32 KB, 2 thread(s)");
        assert_eq!(described(1, CacheKind::Instruction, 32, Some(2)), "Level 1, I, 32 KB, 2 thread(s)");
        assert_eq!(described(2, CacheKind::Unified, 1024, Some(2)), "Level 2, U, 1 MB, 2 thread(s)");
        assert_eq!(described(3, CacheKind::Unified, 32 * 1024, Some(16)), "Level 3, U, 32 MB, 16 thread(s)");
        // A size between whole megabytes stays exact, and the span of APIC ids is printed as stated.
        assert_eq!(described(2, CacheKind::Unified, 1280, Some(1)), "Level 2, U, 1280 KB, 1 thread(s)");
        assert_eq!(
            described(3, CacheKind::Unified, 36 * 1024, Some(128)),
            "Level 3, U, 36 MB, 128 thread(s)"
        );
        // The older descriptor leaves say nothing about sharing.
        assert_eq!(described(2, CacheKind::Unified, 512, None), "Level 2, U, 512 KB");
        assert_eq!(described(1, CacheKind::Trace, 12, None), "Level 1, T, 12 KB");
    }

    #[test]
    fn sums_caches_of_different_size() {
        let stacked = cache(3, CacheKind::Unified, 96 * 1024, 1, 16, 64);
        let plain = cache(3, CacheKind::Unified, 32 * 1024, 1, 16, 64);
        assert_eq!(cache_text(&[stacked, plain]), "96 MB + 32 MB (16-way, 64-byte line)");
        let pair = cache(3, CacheKind::Unified, 16 * 1024, 2, 16, 64);
        assert_eq!(cache_text(&[stacked, pair]), "96 MB + 2 x 16 MB (16-way, 64-byte line)");
        // Details that differ stay with their cache.
        let narrow = cache(3, CacheKind::Unified, 8 * 1024, 1, 8, 64);
        assert_eq!(cache_text(&[plain, narrow]), "32 MB (16-way, 64-byte line) + 8 MB (8-way, 64-byte line)");
        // Nothing known beyond the sizes.
        let bare = |size_kb| cache(2, CacheKind::Unified, size_kb, 4, 0, 0);
        assert_eq!(cache_text(&[bare(256), bare(512)]), "4 x 256 KB + 4 x 512 KB");
    }

    #[test]
    fn describes_bridges() {
        let full = BridgeInfo { vendor: "AMD".into(), model: "B650".into(), revision: "51".into() };
        assert_eq!(bridge(&full), "AMD B650 rev. 51");
        let bare = BridgeInfo { vendor: String::new(), model: "Ryzen SOC".into(), revision: String::new() };
        assert_eq!(bridge(&bare), "Ryzen SOC");
        assert_eq!(bridge(&BridgeInfo { revision: "01".into(), ..Default::default() }), "");
    }

    #[test]
    fn describes_links() {
        let link = PcieLink {
            width: Some(8),
            max_width: Some(16),
            speed_gts: Some(16.0),
            max_speed_gts: Some(32.0),
        };
        assert_eq!(graphic_link_width(&link).as_deref(), Some("x8 (max 16x)"));
        assert_eq!(device_link_width(&link).as_deref(), Some("8x (max 16x)"));
        assert_eq!(link_speed(&link).as_deref(), Some("16.0 GT/s (max 32.0 GT/s)"));

        let current = PcieLink { width: Some(4), speed_gts: Some(2.5), ..Default::default() };
        assert_eq!(graphic_link_width(&current).as_deref(), Some("x4"));
        assert_eq!(device_link_width(&current).as_deref(), Some("4x"));
        assert_eq!(link_speed(&current).as_deref(), Some("2.5 GT/s"));

        let unknown = PcieLink { max_width: Some(16), max_speed_gts: Some(8.0), ..Default::default() };
        assert_eq!(graphic_link_width(&unknown), None);
        assert_eq!(device_link_width(&unknown), None);
        assert_eq!(link_speed(&unknown), None);
    }
}
