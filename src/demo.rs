//! A built-in sample system for `-demo`: lets the UI and reports be exercised
//! without touching the hardware, and gives tests a fully-populated model.

use crate::model::*;

pub fn system() -> SystemInfo {
    let cache = |level, kind, size_kb, count, ways, threads_sharing| CacheInfo {
        level,
        kind,
        size_kb,
        count,
        ways,
        line_size: 64,
        threads_sharing,
    };
    let logical = (0..16u32)
        .map(|i| LogicalProcessor {
            index: i,
            group: 0,
            number: i as u8,
            apic_id: i,
            package: 0,
            core: i / 2,
            smt: i % 2,
            kind: CoreKind::Uniform,
        })
        .collect();
    let cpu = CpuInfo {
        vendor: CpuVendor::Amd,
        vendor_string: "AuthenticAMD".into(),
        name: "AMD Ryzen 7 7700".into(),
        specification: "AMD Ryzen 7 7700 8-Core Processor".into(),
        codename: Some("Raphael".into()),
        package: Some("Socket AM5 (LGA1718)".into()),
        technology_nm: Some(5),
        max_tdp_w: Some(65.0),
        core_voltage: Some(1.016),
        family: 0xF,
        model: 0x1,
        stepping: 0x2,
        ext_family: 0x19,
        ext_model: 0x61,
        revision: Some("RPL-B2".into()),
        instructions: [
            "MMX (+)",
            "SSE",
            "SSE2",
            "SSE3",
            "SSSE3",
            "SSE4.1",
            "SSE4.2",
            "SSE4A",
            "x86-64",
            "AMD-V",
            "AES",
            "AVX",
            "AVX2",
            "AVX512 (DQ, BW, VL, CD, IFMA, VBMI, VBMI2, VNNI, BITALG, VPOPCNTDQ, BF16)",
            "FMA3",
            "SHA",
        ]
        .map(String::from)
        .to_vec(),
        caches: vec![
            cache(1, CacheKind::Data, 32, 8, 8, 2),
            cache(1, CacheKind::Instruction, 32, 8, 8, 2),
            cache(2, CacheKind::Unified, 1024, 8, 8, 2),
            cache(3, CacheKind::Unified, 32 * 1024, 1, 16, 16),
        ],
        cores: 8,
        threads: 16,
        max_cores: Some(8),
        max_threads: Some(16),
        preferred_cores: vec![0, 1],
        sockets: 1,
        core_sets: vec![CoreSet { kind: CoreKind::Uniform, cores: 8, threads: 16 }],
        logical,
        microcode: Some(0x0A60_1203),
        nominal_mhz: Some(3800.0),
        max_cpuid_leaf: 0x0D,
        max_ext_cpuid_leaf: 0x8000_0027,
        thread_dumps: (0..16)
            .map(|apic_id| ThreadDump { processor: apic_id, leaves: cpuid(apic_id) })
            .collect(),
    };

    let mainboard = MainboardInfo {
        manufacturer: "Example Computer Inc.".into(),
        model: "B650 EXAMPLE WIFI".into(),
        version: "1.0".into(),
        bus_specs: Some("PCI-Express 5.0 (32.0 GT/s)".into()),
        chipset: Some(BridgeInfo { vendor: "AMD".into(), model: "Ryzen SOC".into(), revision: "00".into() }),
        southbridge: Some(BridgeInfo { vendor: "AMD".into(), model: "B650".into(), revision: "51".into() }),
        lpcio: Some(BridgeInfo {
            vendor: "Nuvoton".into(),
            model: "NCT6687".into(),
            revision: String::new(),
        }),
        bios: BiosInfo {
            brand: "American Megatrends International LLC.".into(),
            version: "2.A0 - AMD AGESA ComboAM5PI 1.0.8.0".into(),
            date: "09/21/2023".into(),
            uefi: Some(true),
        },
        graphic_interface: Some(GraphicInterface {
            bus: "PCI-Express 4.0".into(),
            link: PcieLink {
                width: Some(8),
                max_width: Some(8),
                speed_gts: Some(16.0),
                max_speed_gts: Some(16.0),
            },
        }),
    };

    let memory = MemoryInfo {
        kind: Some("DDR5".into()),
        size_mb: 32 * 1024,
        channels: Some("4 x 32-bit".into()),
        dc_mode: None,
        controller_mhz: Some(2600.1),
        dram_mhz: Some(2600.1),
        max_dram_mhz: Some(2800.0),
        fsb_dram: Some("1:26".into()),
        timings: MemoryTimings {
            cas_latency: Some(42.0),
            trcd: Some(42),
            trp: Some(42),
            tras: Some(84),
            trc: Some(125),
            command_rate: Some("1T".into()),
        },
    };

    let profile = |name: &str, mhz: f64, cl: f32, trcd, trp, tras, trc| TimingProfile {
        name: name.into(),
        frequency_mhz: Some(mhz),
        cas_latency: Some(cl),
        trcd: Some(trcd),
        trp: Some(trp),
        tras: Some(tras),
        trc: Some(trc),
        command_rate: None,
        voltage: Some(1.1),
    };
    let module = |slot: u32, locator: &str, serial: &str| MemoryModule {
        slot: format!("Slot #{slot}"),
        locator: locator.into(),
        populated: true,
        kind: Some("DDR5".into()),
        form_factor: Some("UDIMM".into()),
        size_mb: 16 * 1024,
        max_bandwidth: Some("DDR5-5600 (2800 MHz)".into()),
        manufacturer: Some("Example Memory".into()),
        dram_manufacturer: Some("Example Memory".into()),
        part_number: Some("EX16G56C46U5".into()),
        serial_number: Some(serial.into()),
        week_year: Some((39, 25)),
        spd_ext: Some("EXPO, XMP".into()),
        correction: Some("None".into()),
        registered: Some(false),
        buffered: Some(false),
        ranks: Some(1),
        configured_speed: Some(5200),
        voltage: Some(1.1),
        profiles: vec![
            profile("JEDEC #8", 2633.0, 42.0, 43, 43, 85, 127),
            profile("JEDEC #9", 2800.0, 46.0, 45, 45, 90, 135),
            profile("XMP-5600", 2800.0, 46.0, 45, 45, 90, 135),
            profile("EXPO-5200", 2600.0, 42.0, 42, 42, 84, 125),
        ],
    };
    let empty = |slot: u32, locator: &str| MemoryModule {
        slot: format!("Slot #{slot}"),
        locator: locator.into(),
        ..Default::default()
    };
    let modules = vec![
        empty(1, "DIMMA1"),
        module(2, "DIMMA2", "00000001"),
        empty(3, "DIMMB1"),
        module(4, "DIMMB2", "00000002"),
    ];

    let gpus = vec![
        GpuInfo {
            name: "NVIDIA GeForce RTX 4060".into(),
            board_manufacturer: Some("Example Computer Inc.".into()),
            codename: Some("AD107".into()),
            revision: Some("A1".into()),
            technology_nm: Some(5),
            tdp_w: Some(115.0),
            rops: Some(48),
            tmus: Some(96),
            cores: Some(3072),
            perf_levels: vec![GpuPerfLevel {
                name: "Current".into(),
                core_mhz: Some(285.0),
                shader_mhz: None,
                memory_mhz: Some(5001.0),
            }],
            memory_mb: Some(8188),
            memory_type: Some("GDDR6".into()),
            memory_vendor: Some("Samsung".into()),
            memory_bus_width: Some(128),
            vendor_id: 0x10DE,
            device_id: 0x2882,
            subsys_vendor_id: 0x1462,
            subsys_id: 0x5161,
            revision_id: 0xA1,
            location: Some((1, 0, 0)),
            link: Some(PcieLink {
                width: Some(8),
                max_width: Some(8),
                speed_gts: Some(16.0),
                max_speed_gts: Some(16.0),
            }),
            driver_version: Some("32.0.15.6094".into()),
            driver_date: Some("08/14/2024".into()),
            wddm_model: Some("3.2".into()),
            primary: true,
        },
        GpuInfo {
            name: "AMD Radeon(TM) Graphics".into(),
            board_manufacturer: Some("Example Computer Inc.".into()),
            codename: Some("Raphael".into()),
            revision: Some("C5".into()),
            technology_nm: Some(6),
            cores: Some(128),
            memory_mb: Some(512),
            vendor_id: 0x1002,
            device_id: 0x164E,
            subsys_vendor_id: 0x1462,
            subsys_id: 0x7D78,
            revision_id: 0xC5,
            location: Some((16, 0, 0)),
            driver_version: Some("31.0.14046.0".into()),
            driver_date: Some("03/29/2023".into()),
            wddm_model: Some("3.1".into()),
            ..Default::default()
        },
    ];

    let pci = vec![
        PciDevice {
            vendor_id: 0x1022,
            device_id: 0x14D8,
            subsys_vendor_id: 0x1462,
            subsys_id: 0x7D78,
            base_class: 0x06,
            description: "PCI standard host CPU bridge".into(),
            ..Default::default()
        },
        PciDevice {
            bus: 1,
            vendor_id: 0x10DE,
            device_id: 0x2882,
            subsys_vendor_id: 0x1462,
            subsys_id: 0x5161,
            revision: 0xA1,
            base_class: 0x03,
            description: "NVIDIA GeForce RTX 4060".into(),
            link: Some(PcieLink {
                width: Some(8),
                max_width: Some(8),
                speed_gts: Some(16.0),
                max_speed_gts: Some(16.0),
            }),
            ..Default::default()
        },
    ];

    let monitors = vec![
        MonitorInfo {
            model: Some("EX2700Q".into()),
            id: Some("EXM2700".into()),
            serial: Some("EXAMPLE000001".into()),
            week: Some(12),
            year: Some(2024),
            size_inches: Some(27.2),
            max_resolution: Some(DisplayMode { width: 3840, height: 2160, refresh_hz: 59 }),
            horizontal_khz: Some((30, 222)),
            vertical_hz: Some((40, 144)),
            max_pixel_clock_mhz: Some(600),
            gamma: Some(2.2),
            video_input: Some("Display Port".into()),
        },
        MonitorInfo {
            model: Some("EX2400".into()),
            id: Some("EXM2400".into()),
            year: Some(2019),
            size_inches: Some(23.8),
            max_resolution: Some(DisplayMode { width: 1920, height: 1080, refresh_hz: 60 }),
            gamma: Some(2.2),
            video_input: Some("HDMI".into()),
            ..Default::default()
        },
    ];

    const GIB: u64 = 1 << 30;
    let drives = vec![
        DriveInfo {
            number: 0,
            device_path: r"\\?\scsi#disk&ven_nvme&prod_example_ssd_1tb#4&1a2b3c4d&0&000000#{53f56307-b6bf-11d0-94f2-00a0c91efb8b}"
                .into(),
            name: "Example SSD 1TB".into(),
            revision: Some("EX1.0".into()),
            serial: Some("EX0000000001".into()),
            capacity_bytes: Some(1_000_204_886_016),
            removable: false,
            solid_state: true,
            bus_type: Some(17),
            features: vec!["SMART".into()],
            controller: Some(DriveController {
                name: "NVM Express (NVMe) Controller".into(),
                bus: 2,
                device: 0,
                function: 0,
                link: Some(PcieLink {
                    width: Some(4),
                    max_width: Some(4),
                    speed_gts: Some(16.0),
                    max_speed_gts: Some(16.0),
                }),
            }),
            volumes: vec![VolumeInfo {
                mount: "c:\\".into(),
                size_bytes: 999_116_767_232,
                free_bytes: 414_633_458_401,
            }],
        },
        DriveInfo {
            number: 1,
            device_path: r"\\?\scsi#disk&ven_example&prod_hdd_4tb#4&5e6f7a8b&0&010000#{53f56307-b6bf-11d0-94f2-00a0c91efb8b}"
                .into(),
            name: "Example HDD 4TB".into(),
            revision: Some("EX2.0".into()),
            serial: Some("EX0000000002".into()),
            capacity_bytes: Some(4_000_787_030_016),
            removable: false,
            solid_state: false,
            bus_type: Some(11),
            features: vec!["SMART".into()],
            controller: Some(DriveController {
                name: "Serial ATA Controller - AHCI".into(),
                bus: 3,
                device: 0,
                function: 1,
                link: None,
            }),
            volumes: vec![
                VolumeInfo { mount: "d:\\".into(), size_bytes: 1863 * GIB, free_bytes: 1863 * GIB / 4 * 3 },
                VolumeInfo { mount: "e:\\".into(), size_bytes: 1863 * GIB, free_bytes: 1863 * GIB },
            ],
        },
    ];

    let usb_device = |name: &str, class, product_id| UsbDevice {
        name: name.into(),
        class: Some(class),
        vendor_id: 0x1D6B,
        product_id,
    };
    let usb = vec![
        usb_device("Generic USB Hub", (0x09, 0x00), 0x0002),
        usb_device("USB Input Device", (0x00, 0x00), 0x0101),
        usb_device("USB Composite Device", (0xEF, 0x02), 0x0104),
    ];

    SystemInfo {
        cpu,
        mainboard,
        memory,
        modules,
        gpus,
        graphic_apis: vec!["D3D".into(), "NVIDIA NVAPI".into()],
        monitors,
        drives,
        usb,
        os: OsInfo {
            name: "Microsoft Windows 11 Home (x64)".into(),
            build: "Build 26100.2314".into(),
            directx: Some("12.0".into()),
            computer_name: "EXAMPLE-PC".into(),
            install_date: Some("11/14/2023".into()),
            whea_errors: Some(0),
        },
        pci,
        smbios: None,
        timers: vec![
            TimerFrequency { name: "ACPI timer".into(), hz: 3_579_545.0 },
            TimerFrequency { name: "Perf timer".into(), hz: 10_000_000.0 },
            TimerFrequency { name: "Sys timer".into(), hz: 1_000.0 },
        ],
    }
}

/// The CPUID leaves the reports interpret, as the sample processor's thread with
/// the given APIC ID answers them: identity, topology and caches.
fn cpuid(apic_id: u32) -> Vec<CpuidLeaf> {
    let leaf = |leaf, subleaf, eax, ebx, ecx, edx| CpuidLeaf { leaf, subleaf, eax, ebx, ecx, edx };
    vec![
        leaf(0x0000_0000, 0, 0x0000_000D, 0x6874_7541, 0x444D_4163, 0x6974_6E65),
        leaf(0x0000_0001, 0, 0x00A6_0F12, apic_id << 24 | 0x0010_0800, 0xFED8_320B, 0x178B_FBFF),
        // Two threads to a core, sixteen to the package.
        leaf(0x0000_000B, 0, 0x0000_0001, 0x0000_0002, 0x0000_0100, apic_id),
        leaf(0x0000_000B, 1, 0x0000_0004, 0x0000_0010, 0x0000_0201, apic_id),
        leaf(0x8000_0000, 0, 0x8000_0027, 0x6874_7541, 0x444D_4163, 0x6974_6E65),
        leaf(0x8000_0001, 0, 0x00A6_0F12, 0x0000_0000, 0x20C0_23F3, 0x2FD3_FBFF),
        leaf(0x8000_0008, 0, 0x0000_3030, 0x7912_D215, 0x0000_400F, 0x0001_0000),
        // 32 KB L1 data and instruction caches and 1 MB of L2 per core, 32 MB of L3 for all.
        leaf(0x8000_001D, 0, 0x0000_4121, 0x01C0_003F, 0x0000_003F, 0x0000_0000),
        leaf(0x8000_001D, 1, 0x0000_4122, 0x01C0_003F, 0x0000_003F, 0x0000_0000),
        leaf(0x8000_001D, 2, 0x0000_4143, 0x01C0_003F, 0x0000_07FF, 0x0000_0002),
        leaf(0x8000_001D, 3, 0x0003_C163, 0x03C0_003F, 0x0000_7FFF, 0x0000_0001),
        leaf(0x8000_001E, 0, apic_id, 0x0000_0100 | apic_id >> 1, 0x0000_0000, 0x0000_0000),
        // Core, core complex, die and socket: one complex on one die.
        leaf(0x8000_0026, 0, 0x0000_0001, 0x0000_0002, 0x0000_0100, apic_id),
        leaf(0x8000_0026, 1, 0x0000_0004, 0x0000_0010, 0x0000_0201, apic_id),
        leaf(0x8000_0026, 2, 0x0000_0004, 0x0000_0010, 0x0000_0302, apic_id),
        leaf(0x8000_0026, 3, 0x0000_0004, 0x0000_0010, 0x0000_0403, apic_id),
    ]
}

pub fn clocks() -> ClockSnapshot {
    let mhz = [5136.63, 5322.90, 5297.91, 5322.90, 4258.32, 5322.90, 5247.93, 5147.97];
    ClockSnapshot {
        cores: mhz
            .iter()
            .enumerate()
            .map(|(i, &mhz)| CoreClock {
                core: i as u32,
                kind: CoreKind::Uniform,
                mhz,
                multiplier: Some((mhz / 99.74 * 4.0_f64).round() / 4.0),
            })
            .collect(),
        bus_mhz: Some(99.74),
        multiplier_min: Some(4.0),
        multiplier_max: Some(53.5),
        rated_fsb_mhz: None,
    }
}
