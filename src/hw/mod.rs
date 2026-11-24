//! Hardware detection. Everything here runs in user mode without elevation.

pub mod board;
pub mod clocks;
pub mod cpu;
pub mod gpu;
mod ioctl;
pub mod memory;
pub mod monitor;
pub mod os;
pub mod pawnio;
pub mod pci;
pub mod processors;
pub mod ring0;
pub mod sensors;
mod setup;
pub mod smbios;
pub mod storage;
pub mod timers;
pub mod usb;

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::config::Config;
use crate::crash;
use crate::elevation::{self, Elevation};
use crate::model::{AccessInfo, SystemInfo};

/// Longest wait for a detection stage that talks to device drivers.
const STAGE_DEADLINE: Duration = Duration::from_secs(8);

/// Detection stages, reported to the splash screen as they start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Processors,
    Chipset,
    Memory,
    Storage,
    Graphics,
    Software,
    Done,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::Processors => "Processors",
            Stage::Chipset => "Chipset",
            Stage::Memory => "Memory",
            Stage::Storage => "Storage",
            Stage::Graphics => "Graphics",
            Stage::Software => "Software",
            Stage::Done => "Detection complete",
        }
    }

    /// Completion percentage when this stage starts.
    pub fn percent(self) -> u32 {
        match self {
            Stage::Processors => 0,
            Stage::Chipset => 35,
            Stage::Memory => 55,
            Stage::Storage => 65,
            Stage::Graphics => 75,
            Stage::Software => 90,
            Stage::Done => 100,
        }
    }
}

/// Runs every detection backend, honouring the switches in `cfg`. A backend that fails
/// or hangs costs only its own part of the result; `SystemInfo::notes` says which.
pub fn detect(cfg: &Config, progress: &mut dyn FnMut(Stage)) -> SystemInfo {
    let mut notes = Vec::new();
    // Power readings average over the time between two collections: the counters start
    // here and are read when detection is done.
    let display = cfg.display;
    let sensor_monitor = bounded("Sensors", STAGE_DEADLINE, &mut notes, move || {
        Some(sensors::SensorMonitor::new(&[], display))
    });
    progress(Stage::Processors);
    let mut cpu = guarded_stage("Processor", &mut notes, cpu::detect);

    progress(Stage::Chipset);
    let smbios = if cfg.dmi { guarded_stage("Firmware tables", &mut notes, smbios::read) } else { None };
    let pci =
        if cfg.pci { bounded("PCI devices", STAGE_DEADLINE, &mut notes, pci::enumerate) } else { Vec::new() };
    let mainboard = guarded_stage("Mainboard", &mut notes, || board::detect(smbios.as_ref(), &pci, &cpu));

    progress(Stage::Memory);
    let (mut memory, mut modules) = guarded_stage("Memory", &mut notes, || memory::detect(smbios.as_ref()));
    // What only ring 0 can read, when the PawnIO driver may be used, and where the driver
    // stands. Asking about it opens its device, a driver request like the readings.
    let (ring0, driver) = {
        let (processor, kind, wanted) = (cpu.clone(), memory.kind.clone(), cfg.pawnio);
        bounded("Ring 0", STAGE_DEADLINE, &mut notes, move || {
            let readings = if pawnio::enabled() {
                ring0::read(&processor, kind.as_deref())
            } else {
                ring0::Readings::default()
            };
            let driver = pawnio::status(wanted, readings.used).text();
            (readings, driver)
        })
    };
    cpu.core_voltage = ring0.core_voltage.map(|volts| volts as f32);
    cpu.ratio_range = ring0.ratio_range;
    memory.controller_mhz = ring0.uncore_mhz.or(memory.controller_mhz);
    if let Some(timings) = ring0.timings {
        memory.timings = timings;
    }
    if let Some(sub_channels) = ring0.sub_channels {
        memory.channels = Some(format!("{sub_channels} x 32-bit"));
    }
    memory::spd::apply(&mut modules, &ring0.spd);

    progress(Stage::Storage);
    let drives = {
        let pci = pci.clone();
        bounded("Drives", STAGE_DEADLINE, &mut notes, move || storage::detect(&pci))
    };
    let usb = bounded("USB devices", STAGE_DEADLINE, &mut notes, usb::detect);

    progress(Stage::Graphics);
    let (gpus, graphic_apis) = if cfg.display {
        let pci = pci.clone();
        bounded("Graphics", STAGE_DEADLINE, &mut notes, move || gpu::detect(&pci))
    } else {
        Default::default()
    };
    let monitors = if cfg.display {
        bounded("Monitors", STAGE_DEADLINE, &mut notes, monitor::detect)
    } else {
        Vec::new()
    };

    progress(Stage::Software);
    let os = bounded("Operating system", STAGE_DEADLINE, &mut notes, os::detect);
    let timers = guarded_stage("Timers", &mut notes, timers::frequencies);
    let mut readings = bounded("Sensors", STAGE_DEADLINE, &mut notes, move || {
        sensor_monitor.map(|mut monitor| monitor.sample()).unwrap_or_default()
    });
    readings.extend(ring0.sensors);
    readings.extend(sensors::drive_sensors(&drives));
    let access = AccessInfo { administrator: elevation::current() == Elevation::Elevated, driver };

    progress(Stage::Done);
    SystemInfo {
        cpu,
        mainboard,
        memory,
        modules,
        gpus,
        graphic_apis,
        monitors,
        drives,
        usb,
        os,
        pci,
        smbios,
        timers,
        sensors: readings,
        notes,
        access,
    }
}

/// Runs one detection stage on a thread of its own and waits up to `deadline` for it. A
/// stage that panics or does not answer in time (a driver that never completes a request)
/// gives its default, and `notes` says what is missing. A thread that hangs is left behind.
fn bounded<T: Default + Send + 'static>(
    what: &str,
    deadline: Duration,
    notes: &mut Vec<String>,
    work: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (sender, receiver) = mpsc::channel();
    let spawned = thread::Builder::new().name(format!("detect {what}")).spawn(move || {
        let _ = sender.send(crash::guarded(work));
    });
    let outcome = match spawned {
        Err(_) => Err("not read, no thread to read it on"),
        Ok(_) => match receiver.recv_timeout(deadline) {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) => Err("not read, the detection failed"),
            Err(_) => Err("not read, no answer in time"),
        },
    };
    outcome.unwrap_or_else(|why| {
        notes.push(format!("{what}: {why}"));
        T::default()
    })
}

/// Runs one detection stage on the calling thread. A stage that panics gives its default,
/// and `notes` says so.
fn guarded_stage<T: Default>(what: &str, notes: &mut Vec<String>, work: impl FnOnce() -> T) -> T {
    crash::guarded(work).unwrap_or_else(|_| {
        notes.push(format!("{what}: not read, the detection failed"));
        T::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stage_that_panics_costs_only_its_own_data() {
        let mut notes = Vec::new();
        let usb: Vec<u8> =
            bounded("USB devices", Duration::from_secs(5), &mut notes, || panic!("hub answered garbage"));
        assert!(usb.is_empty());
        let memory: Vec<u8> = guarded_stage("Memory", &mut notes, || panic!("odd SMBIOS table"));
        assert!(memory.is_empty());
        assert_eq!(
            notes,
            ["USB devices: not read, the detection failed", "Memory: not read, the detection failed"]
        );
        // Stages that do their work give its result and no note.
        assert_eq!(bounded("Drives", Duration::from_secs(5), &mut notes, || vec![1u8]), [1]);
        assert_eq!(guarded_stage("Timers", &mut notes, || 7), 7);
        assert_eq!(notes.len(), 2);
    }

    #[test]
    fn a_stage_that_hangs_is_left_behind() {
        let mut notes = Vec::new();
        let started = std::time::Instant::now();
        let drives: Vec<u8> = bounded("Drives", Duration::from_millis(100), &mut notes, || {
            std::thread::sleep(Duration::from_secs(3));
            vec![1]
        });
        assert!(drives.is_empty());
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(notes, ["Drives: not read, no answer in time"]);
    }

    #[test]
    fn stages_are_reported_in_order() {
        let cfg = Config { dmi: false, pci: false, display: false, ..Config::default() };
        let mut stages = Vec::new();
        let sys = detect(&cfg, &mut |stage| stages.push(stage));
        assert_eq!(
            stages,
            [
                Stage::Processors,
                Stage::Chipset,
                Stage::Memory,
                Stage::Storage,
                Stage::Graphics,
                Stage::Software,
                Stage::Done
            ]
        );
        assert!(stages.windows(2).all(|pair| pair[0].percent() < pair[1].percent()));
        assert!(stages.iter().all(|stage| !stage.label().is_empty()));

        // What the switches turn off stays out of the model, and so does what is built on it.
        assert!(sys.smbios.is_none() && sys.pci.is_empty());
        assert!(sys.gpus.is_empty() && sys.graphic_apis.is_empty() && sys.monitors.is_empty());
        assert!(sys.drives.iter().all(|drive| drive.controller.is_none()));
    }
}
