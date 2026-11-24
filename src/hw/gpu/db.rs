//! Static knowledge about graphics chips, keyed by PCI ids.
//!
//! Device-id ranges follow the public PCI ID assignments; process nodes and the
//! per-product figures come from publicly documented specifications. An entry only
//! carries what holds for every product behind its ids - anything that varies between
//! board variants stays `None`.

use std::ops::RangeInclusive;

pub const NVIDIA: u16 = 0x10DE;
pub const AMD: u16 = 0x1002;
pub const INTEL: u16 = 0x8086;

/// A graphics chip (one die), usually shared by several products.
#[derive(Debug, PartialEq, Eq)]
pub struct Chip {
    vendor: u16,
    devices: RangeInclusive<u16>,
    /// PCI revisions the entry is limited to; empty means any.
    revisions: &'static [u8],
    pub codename: &'static str,
    pub technology_nm: Option<u32>,
    /// Set when every product built on the chip uses the same memory type.
    pub memory_type: Option<&'static str>,
}

impl Chip {
    const fn new(vendor: u16, devices: RangeInclusive<u16>, codename: &'static str) -> Self {
        Chip { vendor, devices, revisions: &[], codename, technology_nm: None, memory_type: None }
    }

    const fn nm(mut self, nm: u32) -> Self {
        self.technology_nm = Some(nm);
        self
    }

    const fn memory(mut self, kind: &'static str) -> Self {
        self.memory_type = Some(kind);
        self
    }

    const fn revisions(mut self, revisions: &'static [u8]) -> Self {
        self.revisions = revisions;
        self
    }

    fn matches(&self, vendor: u16, device: u16, revision: u8) -> bool {
        self.vendor == vendor
            && self.devices.contains(&device)
            && (self.revisions.is_empty() || self.revisions.contains(&revision))
    }
}

const fn nvidia(devices: RangeInclusive<u16>, codename: &'static str) -> Chip {
    Chip::new(NVIDIA, devices, codename)
}

const fn amd(devices: RangeInclusive<u16>, codename: &'static str) -> Chip {
    Chip::new(AMD, devices, codename)
}

const fn intel(devices: RangeInclusive<u16>, codename: &'static str) -> Chip {
    Chip::new(INTEL, devices, codename)
}

/// Searched top to bottom, so entries narrowed to a revision or a single id come
/// before the range they carve out of.
static CHIPS: &[Chip] = &[
    // NVIDIA Pascal and Volta.
    nvidia(0x15F0..=0x15FF, "GP100").nm(16),
    nvidia(0x1B00..=0x1B7F, "GP102").nm(16),
    nvidia(0x1B80..=0x1BFF, "GP104").nm(16),
    nvidia(0x1C00..=0x1C7F, "GP106").nm(16),
    nvidia(0x1C80..=0x1CFF, "GP107").nm(14),
    nvidia(0x1D00..=0x1D7F, "GP108").nm(14),
    nvidia(0x1D80..=0x1DFF, "GV100").nm(12),
    // NVIDIA Turing.
    nvidia(0x1E00..=0x1E7F, "TU102").nm(12),
    nvidia(0x1E80..=0x1EFF, "TU104").nm(12),
    nvidia(0x1F00..=0x1F7F, "TU106").nm(12),
    nvidia(0x1F80..=0x1FFF, "TU117").nm(12),
    nvidia(0x2180..=0x21FF, "TU116").nm(12),
    // NVIDIA Ampere.
    nvidia(0x2200..=0x223F, "GA102").nm(8),
    nvidia(0x2400..=0x247F, "GA103").nm(8),
    nvidia(0x2480..=0x24FF, "GA104").nm(8),
    nvidia(0x2500..=0x257F, "GA106").nm(8),
    nvidia(0x2580..=0x25FF, "GA107").nm(8),
    // NVIDIA Ada Lovelace and Blackwell (TSMC 4N, a 5 nm class process).
    nvidia(0x2680..=0x26FF, "AD102").nm(5),
    nvidia(0x2700..=0x277F, "AD103").nm(5),
    nvidia(0x2780..=0x27FF, "AD104").nm(5),
    nvidia(0x2800..=0x287F, "AD106").nm(5),
    nvidia(0x2880..=0x28FF, "AD107").nm(5),
    nvidia(0x2B80..=0x2BFF, "GB202").nm(5),
    nvidia(0x2C00..=0x2C7F, "GB203").nm(5),
    nvidia(0x2D00..=0x2D7F, "GB206").nm(5),
    nvidia(0x2D80..=0x2DFF, "GB207").nm(5),
    nvidia(0x2F00..=0x2F7F, "GB205").nm(5),
    // AMD Polaris. The Radeon RX 400 and 500 cards share one device id per die and
    // differ by revision.
    amd(0x67DF..=0x67DF, "Polaris 30").nm(12).memory("GDDR5").revisions(&[0xE1]),
    amd(0x67DF..=0x67DF, "Polaris 20").nm(14).memory("GDDR5").revisions(&[0xE7, 0xEF]),
    amd(0x67DF..=0x67DF, "Polaris 10").nm(14).memory("GDDR5").revisions(&[0xC7, 0xCF]),
    amd(0x6FDF..=0x6FDF, "Polaris 20").nm(14).memory("GDDR5"),
    amd(0x67C0..=0x67DF, "Ellesmere").nm(14).memory("GDDR5"),
    amd(0x67E0..=0x67FF, "Baffin").nm(14),
    amd(0x6980..=0x699F, "Polaris 12").nm(14),
    amd(0x694C..=0x694F, "Polaris 22").nm(14),
    // AMD Vega.
    amd(0x6860..=0x687F, "Vega 10").nm(14).memory("HBM2"),
    amd(0x66A0..=0x66AF, "Vega 20").nm(7).memory("HBM2"),
    // AMD Navi.
    amd(0x7310..=0x731B, "Navi 10").nm(7).memory("GDDR6"),
    amd(0x731F..=0x731F, "Navi 10").nm(7).memory("GDDR6"),
    amd(0x7340..=0x734F, "Navi 14").nm(7).memory("GDDR6"),
    amd(0x7360..=0x7362, "Navi 12").nm(7),
    amd(0x73A0..=0x73BF, "Navi 21").nm(7).memory("GDDR6"),
    amd(0x73C0..=0x73DF, "Navi 22").nm(7).memory("GDDR6"),
    amd(0x73F0..=0x73F0, "Navi 33").nm(6).memory("GDDR6"),
    amd(0x73E0..=0x73FF, "Navi 23").nm(7).memory("GDDR6"),
    amd(0x7420..=0x743F, "Navi 24").nm(6).memory("GDDR6"),
    amd(0x7440..=0x745F, "Navi 31").nm(5).memory("GDDR6"),
    amd(0x7460..=0x747F, "Navi 32").nm(5).memory("GDDR6"),
    amd(0x7480..=0x749F, "Navi 33").nm(6).memory("GDDR6"),
    amd(0x7550..=0x7551, "Navi 48").nm(4).memory("GDDR6"),
    amd(0x7590..=0x7590, "Navi 44").nm(4).memory("GDDR6"),
    // AMD processor graphics. 0x15D8 covers both the 12 nm Picasso and the 14 nm Raven 2.
    amd(0x15DD..=0x15DD, "Raven Ridge").nm(14),
    amd(0x15D8..=0x15D8, "Picasso / Raven 2"),
    amd(0x1636..=0x1636, "Renoir").nm(7),
    amd(0x164C..=0x164C, "Lucienne").nm(7),
    amd(0x1638..=0x1638, "Cezanne").nm(7),
    amd(0x15E7..=0x15E7, "Barcelo").nm(7),
    amd(0x163F..=0x163F, "Van Gogh").nm(7),
    amd(0x164D..=0x164D, "Rembrandt").nm(6),
    amd(0x1681..=0x1681, "Rembrandt").nm(6),
    amd(0x1506..=0x1506, "Mendocino").nm(6),
    amd(0x164E..=0x164E, "Raphael").nm(6),
    amd(0x13C0..=0x13C0, "Granite Ridge").nm(6),
    amd(0x15BF..=0x15BF, "Phoenix").nm(4),
    amd(0x15C8..=0x15C8, "Phoenix 2").nm(4),
    amd(0x1900..=0x1901, "Hawk Point").nm(4),
    amd(0x150E..=0x150E, "Strix Point").nm(4),
    amd(0x1114..=0x1114, "Krackan Point").nm(4),
    amd(0x1586..=0x1586, "Strix Halo").nm(4),
    // Intel processor graphics. "Intel 7" is the renamed 10 nm Enhanced SuperFin process.
    intel(0x1902..=0x193D, "Skylake").nm(14),
    intel(0x5902..=0x5927, "Kaby Lake").nm(14),
    intel(0x3E90..=0x3E9C, "Coffee Lake").nm(14),
    intel(0x3EA0..=0x3EA1, "Whiskey Lake").nm(14),
    intel(0x3EA5..=0x3EA9, "Coffee Lake").nm(14),
    intel(0x9B21..=0x9BE6, "Comet Lake").nm(14),
    intel(0x8A51..=0x8A5C, "Ice Lake").nm(10),
    intel(0x9A40..=0x9A78, "Tiger Lake").nm(10),
    intel(0x4C8A..=0x4C9A, "Rocket Lake").nm(14),
    intel(0x4626..=0x462A, "Alder Lake").nm(10),
    intel(0x4680..=0x46D4, "Alder Lake").nm(10),
    intel(0xA720..=0xA7AD, "Raptor Lake").nm(10),
    // Meteor Lake and Arrow Lake ids interleave. The Meteor Lake graphics tile is TSMC N5;
    // Lunar Lake puts the GPU on its TSMC N3B compute tile.
    intel(0x7D40..=0x7D40, "Meteor Lake").nm(5),
    intel(0x7D41..=0x7D41, "Arrow Lake"),
    intel(0x7D45..=0x7D45, "Meteor Lake").nm(5),
    intel(0x7D51..=0x7D51, "Arrow Lake"),
    intel(0x7D55..=0x7D55, "Meteor Lake").nm(5),
    intel(0x7D60..=0x7D60, "Meteor Lake").nm(5),
    intel(0x7D67..=0x7D67, "Arrow Lake"),
    intel(0x7DD1..=0x7DD1, "Arrow Lake"),
    intel(0x7DD5..=0x7DD5, "Meteor Lake").nm(5),
    intel(0x6420..=0x6420, "Lunar Lake").nm(3),
    intel(0x64A0..=0x64A0, "Lunar Lake").nm(3),
    intel(0x64B0..=0x64B0, "Lunar Lake").nm(3),
    intel(0xB080..=0xB0FF, "Panther Lake"),
    // Intel discrete graphics.
    intel(0x4905..=0x4909, "DG1").nm(10),
    intel(0x5690..=0x56BF, "Alchemist").nm(6),
    intel(0xE202..=0xE216, "Battlemage").nm(5),
    intel(0xE220..=0xE223, "Battlemage"),
];

/// The chip behind a PCI vendor / device / revision triple.
pub fn chip(vendor: u16, device: u16, revision: u8) -> Option<&'static Chip> {
    CHIPS.iter().find(|chip| chip.matches(vendor, device, revision))
}

/// Fixed characteristics of one product, identified by a single device id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Model {
    vendor: u16,
    device: u16,
    /// Shader processors, CUDA cores or execution units.
    pub cores: Option<u32>,
    pub rops: Option<u32>,
    pub tmus: Option<u32>,
    /// Board power of the reference design in watts.
    pub tdp_w: Option<u16>,
    pub memory_type: Option<&'static str>,
    pub memory_bus_width: Option<u32>,
}

impl Model {
    const fn new(vendor: u16, device: u16) -> Self {
        Model {
            vendor,
            device,
            cores: None,
            rops: None,
            tmus: None,
            tdp_w: None,
            memory_type: None,
            memory_bus_width: None,
        }
    }

    const fn cores(mut self, cores: u32) -> Self {
        self.cores = Some(cores);
        self
    }

    const fn rops(mut self, rops: u32) -> Self {
        self.rops = Some(rops);
        self
    }

    const fn tmus(mut self, tmus: u32) -> Self {
        self.tmus = Some(tmus);
        self
    }

    const fn tdp(mut self, watts: u16) -> Self {
        self.tdp_w = Some(watts);
        self
    }

    const fn memory(mut self, kind: &'static str) -> Self {
        self.memory_type = Some(kind);
        self
    }

    const fn bus(mut self, bits: u32) -> Self {
        self.memory_bus_width = Some(bits);
        self
    }
}

const fn geforce(device: u16) -> Model {
    Model::new(NVIDIA, device)
}

const fn radeon(device: u16) -> Model {
    Model::new(AMD, device)
}

const fn arc(device: u16) -> Model {
    Model::new(INTEL, device)
}

/// Processor graphics with `eus` execution units.
const fn intel_igpu(device: u16, eus: u32) -> Model {
    Model::new(INTEL, device).cores(eus)
}

/// Texture-unit counts are listed for Pascal only, where NVIDIA documents eight per
/// shader cluster; later NVIDIA designs derive theirs from the core count (see
/// [`texture_units`]). Rows without a power figure have reference and partner designs
/// that differ, and rows without a memory type shipped with more than one.
static MODELS: &[Model] = &[
    // GeForce GTX 10.
    geforce(0x1B06).cores(3584).rops(88).tmus(224).tdp(250).memory("GDDR5X").bus(352), // GTX 1080 Ti
    geforce(0x1B80).cores(2560).rops(64).tmus(160).tdp(180).memory("GDDR5X").bus(256), // GTX 1080
    geforce(0x1B82).cores(2432).rops(64).tmus(152).tdp(180).memory("GDDR5").bus(256),  // GTX 1070 Ti
    geforce(0x1B81).cores(1920).rops(64).tmus(120).tdp(150).memory("GDDR5").bus(256),  // GTX 1070
    geforce(0x1C03).cores(1280).rops(48).tmus(80).tdp(120).memory("GDDR5").bus(192),   // GTX 1060 6GB
    geforce(0x1C02).cores(1152).rops(48).tmus(72).tdp(120).memory("GDDR5").bus(192),   // GTX 1060 3GB
    geforce(0x1C82).cores(768).rops(32).tmus(48).tdp(75).memory("GDDR5").bus(128),     // GTX 1050 Ti
    geforce(0x1C81).cores(640).rops(32).tmus(40).tdp(75).memory("GDDR5").bus(128),     // GTX 1050
    geforce(0x1D01).cores(384).rops(16).tmus(24).bus(64),                              // GT 1030
    // GeForce GTX 16.
    geforce(0x2182).cores(1536).rops(48).tdp(120).memory("GDDR6").bus(192), // GTX 1660 Ti
    geforce(0x21C4).cores(1408).rops(48).tdp(125).memory("GDDR6").bus(192), // GTX 1660 SUPER
    geforce(0x2184).cores(1408).rops(48).tdp(120).memory("GDDR5").bus(192), // GTX 1660
    geforce(0x2187).cores(1280).tdp(100).memory("GDDR6").bus(128),          // GTX 1650 SUPER
    geforce(0x1F82).cores(896).rops(32).tdp(75).bus(128),                   // GTX 1650
    // GeForce RTX 20.
    geforce(0x1E04).cores(4352).rops(88).memory("GDDR6").bus(352), // RTX 2080 Ti
    geforce(0x1E07).cores(4352).rops(88).memory("GDDR6").bus(352), // RTX 2080 Ti
    geforce(0x1E81).cores(3072).rops(64).tdp(250).memory("GDDR6").bus(256), // RTX 2080 SUPER
    geforce(0x1E82).cores(2944).rops(64).memory("GDDR6").bus(256), // RTX 2080
    geforce(0x1E87).cores(2944).rops(64).memory("GDDR6").bus(256), // RTX 2080
    geforce(0x1E84).cores(2560).rops(64).tdp(215).memory("GDDR6").bus(256), // RTX 2070 SUPER
    geforce(0x1F02).cores(2304).rops(64).memory("GDDR6").bus(256), // RTX 2070
    geforce(0x1F07).cores(2304).rops(64).memory("GDDR6").bus(256), // RTX 2070
    geforce(0x1F06).cores(2176).rops(64).tdp(175).memory("GDDR6").bus(256), // RTX 2060 SUPER
    geforce(0x1F08).cores(1920).rops(48).tdp(160).memory("GDDR6").bus(192), // RTX 2060
    // GeForce RTX 30.
    geforce(0x2203).cores(10752).rops(112).tdp(450).memory("GDDR6X").bus(384), // RTX 3090 Ti
    geforce(0x2204).cores(10496).rops(112).tdp(350).memory("GDDR6X").bus(384), // RTX 3090
    geforce(0x2208).cores(10240).rops(112).tdp(350).memory("GDDR6X").bus(384), // RTX 3080 Ti
    geforce(0x220A).cores(8960).rops(96).tdp(350).memory("GDDR6X").bus(384),   // RTX 3080 12GB
    geforce(0x2206).cores(8704).rops(96).tdp(320).memory("GDDR6X").bus(320),   // RTX 3080
    geforce(0x2216).cores(8704).rops(96).tdp(320).memory("GDDR6X").bus(320),   // RTX 3080 LHR
    geforce(0x2482).cores(6144).rops(96).tdp(290).memory("GDDR6X").bus(256),   // RTX 3070 Ti
    geforce(0x2484).cores(5888).rops(96).tdp(220).memory("GDDR6").bus(256),    // RTX 3070
    geforce(0x2488).cores(5888).rops(96).tdp(220).memory("GDDR6").bus(256),    // RTX 3070 LHR
    geforce(0x2486).cores(4864).rops(80).tdp(200).memory("GDDR6").bus(256),    // RTX 3060 Ti
    geforce(0x2489).cores(4864).rops(80).tdp(200).memory("GDDR6").bus(256),    // RTX 3060 Ti LHR
    geforce(0x2503).cores(3584).rops(48).tdp(170).memory("GDDR6").bus(192),    // RTX 3060
    geforce(0x2504).cores(3584).rops(48).tdp(170).memory("GDDR6").bus(192),    // RTX 3060 LHR
    geforce(0x2507).cores(2560).tdp(130).memory("GDDR6").bus(128),             // RTX 3050
    // GeForce RTX 40.
    geforce(0x2684).cores(16384).rops(176).tdp(450).memory("GDDR6X").bus(384), // RTX 4090
    geforce(0x2702).cores(10240).rops(112).tdp(320).memory("GDDR6X").bus(256), // RTX 4080 SUPER
    geforce(0x2703).cores(10240).rops(112).tdp(320).memory("GDDR6X").bus(256), // RTX 4080 SUPER
    geforce(0x2704).cores(9728).rops(112).tdp(320).memory("GDDR6X").bus(256),  // RTX 4080
    geforce(0x2705).cores(8448).tdp(285).memory("GDDR6X").bus(256),            // RTX 4070 Ti SUPER
    geforce(0x2782).cores(7680).rops(80).tdp(285).memory("GDDR6X").bus(192),   // RTX 4070 Ti
    geforce(0x2783).cores(7168).rops(80).tdp(220).memory("GDDR6X").bus(192),   // RTX 4070 SUPER
    geforce(0x2786).cores(5888).rops(64).tdp(200).bus(192),                    // RTX 4070
    geforce(0x2803).cores(4352).rops(48).tdp(160).memory("GDDR6").bus(128),    // RTX 4060 Ti
    geforce(0x2805).cores(4352).rops(48).tdp(165).memory("GDDR6").bus(128),    // RTX 4060 Ti 16GB
    geforce(0x2882).cores(3072).rops(48).tdp(115).memory("GDDR6").bus(128),    // RTX 4060
    // GeForce RTX 50.
    geforce(0x2B85).cores(21760).tdp(575).memory("GDDR7").bus(512), // RTX 5090
    geforce(0x2C02).cores(10752).tdp(360).memory("GDDR7").bus(256), // RTX 5080
    geforce(0x2C05).cores(8960).tdp(300).memory("GDDR7").bus(256),  // RTX 5070 Ti
    geforce(0x2F04).cores(6144).tdp(250).memory("GDDR7").bus(192),  // RTX 5070
    geforce(0x2D04).cores(4608).tdp(180).memory("GDDR7").bus(128),  // RTX 5060 Ti
    geforce(0x2D05).cores(3840).tdp(145).memory("GDDR7").bus(128),  // RTX 5060
    geforce(0x2D83).cores(2560).tdp(130).memory("GDDR6").bus(128),  // RTX 5050
    // AMD processor graphics.
    radeon(0x164E).cores(128).rops(4), // Raphael, two RDNA 2 compute units
    // Intel Arc.
    arc(0x56A0).cores(512).tdp(225).memory("GDDR6").bus(256), // Arc A770
    arc(0x56A1).cores(448).tdp(225).memory("GDDR6").bus(256), // Arc A750
    arc(0x56A2).cores(384).memory("GDDR6").bus(256),          // Arc A580
    arc(0x56A5).cores(128).tdp(75).memory("GDDR6").bus(96),   // Arc A380
    arc(0x56A6).cores(96).tdp(75).memory("GDDR6").bus(64),    // Arc A310
    arc(0xE20B).tdp(190).memory("GDDR6").bus(192),            // Arc B580
    arc(0xE20C).tdp(150).memory("GDDR6").bus(160),            // Arc B570
    // Intel desktop processor graphics (Xe-LP, where the model number fixes the unit count).
    intel_igpu(0x4C8A, 32), // UHD Graphics 750
    intel_igpu(0x4C8B, 24), // UHD Graphics 730
    intel_igpu(0x4680, 32), // UHD Graphics 770
    intel_igpu(0x4690, 32), // UHD Graphics 770
    intel_igpu(0x4682, 24), // UHD Graphics 730
    intel_igpu(0x4692, 24), // UHD Graphics 730
    intel_igpu(0x4693, 16), // UHD Graphics 710
    intel_igpu(0xA780, 32), // UHD Graphics 770
];

/// Texture units of a product: the figure its row lists, or for NVIDIA's Turing, Ampere,
/// Ada and Blackwell chips four per streaming multiprocessor, worked out from the row's
/// core count (64 cores per multiprocessor on Turing, 128 since Ampere). A core count
/// that does not divide evenly is not one of those designs and gives `None`.
pub fn texture_units(chip: Option<&Chip>, model: Option<&Model>) -> Option<u32> {
    let model = model?;
    if model.tmus.is_some() {
        return model.tmus;
    }
    if model.vendor != NVIDIA {
        return None;
    }
    let cores_per_unit = match chip?.codename.get(..2)? {
        "TU" => 16,
        "GA" | "AD" | "GB" => 32,
        _ => return None,
    };
    let cores = model.cores?;
    (cores % cores_per_unit == 0).then_some(cores / cores_per_unit)
}

/// Product details for a PCI vendor / device pair.
pub fn model(vendor: u16, device: u16) -> Option<&'static Model> {
    MODELS.iter().find(|model| model.vendor == vendor && model.device == device)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codename(vendor: u16, device: u16, revision: u8) -> Option<&'static str> {
        chip(vendor, device, revision).map(|chip| chip.codename)
    }

    #[test]
    fn finds_nvidia_chips() {
        let ad107 = chip(NVIDIA, 0x2882, 0xA1).unwrap();
        assert_eq!((ad107.codename, ad107.technology_nm), ("AD107", Some(5)));
        assert_eq!(codename(NVIDIA, 0x1B80, 0xA1), Some("GP104"));
        assert_eq!(codename(NVIDIA, 0x1F82, 0xA1), Some("TU117"));
        assert_eq!(codename(NVIDIA, 0x2187, 0xA1), Some("TU116"));
        assert_eq!(codename(NVIDIA, 0x2204, 0xA1), Some("GA102"));
        assert_eq!(codename(NVIDIA, 0x28E0, 0xA1), Some("AD107"));
        assert_eq!(codename(NVIDIA, 0x2B85, 0xA1), Some("GB202"));
        assert_eq!(codename(NVIDIA, 0x2F04, 0xA1), Some("GB205"));
        assert_eq!(chip(NVIDIA, 0x1C82, 0xA1).unwrap().technology_nm, Some(14));
        assert_eq!(chip(NVIDIA, 0x1E04, 0xA1).unwrap().technology_nm, Some(12));
        assert_eq!(chip(NVIDIA, 0x2484, 0xA1).unwrap().technology_nm, Some(8));
    }

    #[test]
    fn finds_amd_chips() {
        let raphael = chip(AMD, 0x164E, 0xC5).unwrap();
        assert_eq!(
            (raphael.codename, raphael.technology_nm, raphael.memory_type),
            ("Raphael", Some(6), None)
        );
        assert_eq!(codename(AMD, 0x15BF, 0x00), Some("Phoenix"));
        assert_eq!(codename(AMD, 0x1681, 0x00), Some("Rembrandt"));
        assert_eq!(codename(AMD, 0x1638, 0x00), Some("Cezanne"));
        assert_eq!(codename(AMD, 0x73BF, 0xC1), Some("Navi 21"));
        assert_eq!(codename(AMD, 0x744C, 0xC8), Some("Navi 31"));
        assert_eq!(codename(AMD, 0x7550, 0xC0), Some("Navi 48"));
        assert_eq!(chip(AMD, 0x747E, 0x00).unwrap().memory_type, Some("GDDR6"));
        assert_eq!(chip(AMD, 0x15D8, 0xC8).unwrap().technology_nm, None);
    }

    #[test]
    fn narrow_entries_win_over_ranges() {
        assert_eq!(codename(AMD, 0x67DF, 0xC7), Some("Polaris 10"));
        assert_eq!(codename(AMD, 0x67DF, 0xE7), Some("Polaris 20"));
        let rx590 = chip(AMD, 0x67DF, 0xE1).unwrap();
        assert_eq!((rx590.codename, rx590.technology_nm), ("Polaris 30", Some(12)));
        assert_eq!(codename(AMD, 0x67DF, 0x00), Some("Ellesmere"));
        assert_eq!(codename(AMD, 0x67C4, 0xE7), Some("Ellesmere"));
        assert_eq!(codename(AMD, 0x73F0, 0x00), Some("Navi 33"));
        assert_eq!(codename(AMD, 0x73FF, 0x00), Some("Navi 23"));
        assert_eq!(codename(AMD, 0x731E, 0x00), None);
    }

    #[test]
    fn finds_intel_chips() {
        assert_eq!(codename(INTEL, 0x56A0, 0x08), Some("Alchemist"));
        assert_eq!(codename(INTEL, 0xE20B, 0x00), Some("Battlemage"));
        assert_eq!(codename(INTEL, 0xA780, 0x04), Some("Raptor Lake"));
        assert_eq!(codename(INTEL, 0x4680, 0x0C), Some("Alder Lake"));
        assert_eq!(codename(INTEL, 0x7D55, 0x08), Some("Meteor Lake"));
        assert_eq!(codename(INTEL, 0x7D51, 0x03), Some("Arrow Lake"));
        assert_eq!(chip(INTEL, 0x64A0, 0x04).unwrap().technology_nm, Some(3));
        assert_eq!(chip(INTEL, 0x9A49, 0x01).unwrap().technology_nm, Some(10));
    }

    #[test]
    fn unknown_devices_have_no_chip() {
        assert_eq!(chip(NVIDIA, 0x0001, 0), None);
        assert_eq!(chip(NVIDIA, 0x22BE, 0xA1), None);
        assert_eq!(chip(AMD, 0x2882, 0), None);
        assert_eq!(chip(0x1414, 0x008C, 0), None);
    }

    #[test]
    fn chip_table_is_well_formed() {
        for (index, entry) in CHIPS.iter().enumerate() {
            assert!(entry.devices.start() <= entry.devices.end(), "{entry:?}");
            assert!(!entry.codename.is_empty());
            // An unrestricted entry must not hide a later one completely.
            for later in &CHIPS[index + 1..] {
                let hidden = entry.vendor == later.vendor
                    && entry.revisions.is_empty()
                    && entry.devices.start() <= later.devices.start()
                    && entry.devices.end() >= later.devices.end();
                assert!(!hidden, "{later:?} is unreachable behind {entry:?}");
            }
        }
    }

    #[test]
    fn derives_texture_units_from_the_architecture() {
        let tmus = |device, revision| texture_units(chip(NVIDIA, device, revision), model(NVIDIA, device));
        // Ada and Ampere: four per 128-core streaming multiprocessor.
        assert_eq!(tmus(0x2803, 0xA1), Some(136)); // RTX 4060 Ti, AD106
        assert_eq!(tmus(0x2684, 0xA1), Some(512)); // RTX 4090, AD102
        assert_eq!(tmus(0x2206, 0xA1), Some(272)); // RTX 3080, GA102
        assert_eq!(tmus(0x2B85, 0xA1), Some(680)); // RTX 5090, GB202
                                                   // Turing: four per 64-core streaming multiprocessor.
        assert_eq!(tmus(0x1E04, 0xA1), Some(272)); // RTX 2080 Ti, TU102
        assert_eq!(tmus(0x1F82, 0xA1), Some(56)); // GTX 1650, TU117
                                                  // Pascal rows list theirs.
        assert_eq!(tmus(0x1B80, 0xA1), Some(160));
        // No chip, no model, or a vendor without the rule: unknown.
        assert_eq!(texture_units(None, model(NVIDIA, 0x2803)), None);
        assert_eq!(texture_units(chip(NVIDIA, 0x2803, 0xA1), None), None);
        assert_eq!(texture_units(chip(AMD, 0x744C, 0xC8), model(AMD, 0x744C)), None);
    }

    #[test]
    fn finds_models() {
        let rtx4060 = model(NVIDIA, 0x2882).unwrap();
        assert_eq!(rtx4060.cores, Some(3072));
        assert_eq!(rtx4060.rops, Some(48));
        assert_eq!(rtx4060.tmus, None);
        assert_eq!(rtx4060.tdp_w, Some(115));
        assert_eq!(rtx4060.memory_type, Some("GDDR6"));
        assert_eq!(rtx4060.memory_bus_width, Some(128));
        assert_eq!(model(NVIDIA, 0x1B80).unwrap().tmus, Some(160));
        assert_eq!(model(NVIDIA, 0x2786).unwrap().memory_type, None);
        assert_eq!(model(AMD, 0x164E).unwrap().cores, Some(128));
        assert_eq!(model(INTEL, 0x56A0).unwrap().memory_bus_width, Some(256));
        assert_eq!(model(INTEL, 0x4680).unwrap().cores, Some(32));
        assert_eq!(model(AMD, 0x2882), None);
        assert_eq!(model(NVIDIA, 0x28E0), None);
    }

    #[test]
    fn model_table_is_consistent() {
        for (index, entry) in MODELS.iter().enumerate() {
            let duplicate =
                MODELS[index + 1..].iter().any(|m| m.vendor == entry.vendor && m.device == entry.device);
            assert!(!duplicate, "{entry:?} is listed twice");
            // Every product belongs to a known chip, and they agree on the memory type.
            let chip = chip(entry.vendor, entry.device, 0).unwrap_or_else(|| panic!("no chip for {entry:?}"));
            if let (Some(a), Some(b)) = (chip.memory_type, entry.memory_type) {
                assert_eq!(a, b, "{entry:?}");
            }
            // Memory buses are built from 32-bit devices.
            assert_eq!(entry.memory_bus_width.unwrap_or(0) % 32, 0, "{entry:?}");
        }
    }
}
