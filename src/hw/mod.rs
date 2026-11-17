//! Hardware detection. Everything here runs in user mode without elevation.

pub mod board;
pub mod clocks;
pub mod cpu;
pub mod gpu;
mod ioctl;
pub mod memory;
pub mod monitor;
pub mod os;
pub mod pci;
pub mod processors;
mod setup;
pub mod smbios;
pub mod storage;
pub mod timers;
pub mod usb;

use crate::config::Config;
use crate::model::SystemInfo;

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

/// Runs every detection backend, honouring the switches in `cfg`.
pub fn detect(cfg: &Config, progress: &mut dyn FnMut(Stage)) -> SystemInfo {
    progress(Stage::Processors);
    let cpu = cpu::detect();

    progress(Stage::Chipset);
    let smbios = if cfg.dmi { smbios::read() } else { None };
    let pci = if cfg.pci { pci::enumerate() } else { Vec::new() };
    let mainboard = board::detect(smbios.as_ref(), &pci, &cpu);

    progress(Stage::Memory);
    let (memory, modules) = memory::detect(smbios.as_ref());

    progress(Stage::Storage);
    let drives = storage::detect(&pci);
    let usb = usb::detect();

    progress(Stage::Graphics);
    let (gpus, graphic_apis) = if cfg.display { gpu::detect(&pci) } else { Default::default() };
    let monitors = if cfg.display { monitor::detect() } else { Vec::new() };

    progress(Stage::Software);
    let os = os::detect();
    let timers = timers::frequencies();

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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
