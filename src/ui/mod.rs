//! The application window: splash, tabs and tool dialogs.
//!
//! Everything is drawn with GDI into an off-screen surface from layout tables
//! in dialog units ([`layout`]); the live windows and [`snapshot`] share the
//! same drawing code.

mod bmp;
mod content;
mod dialogs;
mod events;
mod format;
mod gdi;
mod geom;
mod layout;
mod popup;
mod render;
mod splash;
mod state;
mod theme;
mod tools;
mod widgets;
mod win;
mod window;

use std::io;
use std::path::{Path, PathBuf};

use crate::bench;
use crate::config::{self, Config};
use crate::model::{ClockSnapshot, SystemInfo};

use events::Worker;
use gdi::{Fonts, Painter, Surface};
use geom::{Scale, BASE_DPI};
use layout::Tab;
use state::{UiState, View};
use theme::Theme;
use tools::{ClocksTool, SensorsTool, TimersTool};

/// File that holds the saved benchmark references, in the data folder (see [`config::data_dir`]).
const REFERENCE_FILE: &str = "rs-cpuz-bench.txt";

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Core whose clocks are shown on the CPU tab at startup.
    pub core: Option<u32>,
    /// Show the built-in sample system instead of probing the hardware.
    pub demo: bool,
}

/// Shows the splash screen, runs detection and then the main window. Returns the process exit code.
pub fn run(cfg: &Config, opts: &Options) -> i32 {
    let icons = win::Icons::create();
    let registered = splash::register(&icons)
        && window::register(&icons)
        && popup::register()
        && tools::register::<ClocksTool>()
        && tools::register::<TimersTool>()
        && tools::register::<SensorsTool>();
    if !registered {
        dialogs::error(None, "The application windows could not be set up.");
        return 1;
    }

    let (mailbox, inbox) = events::channel();
    let Some(worker) = Worker::spawn(cfg.clone(), opts.demo, mailbox.clone()) else {
        dialogs::error(None, "The hardware detection thread could not be started.");
        return 1;
    };
    let Some((sys, clocks)) = splash::run(cfg, &mailbox, &inbox) else {
        // The splash was closed while detection was still running.
        worker.abandon();
        return 0;
    };
    let code = window::run(cfg, opts, sys, clocks, mailbox, inbox);
    worker.stop();
    code
}

/// Path of the benchmark reference file the next save goes to.
fn reference_path() -> PathBuf {
    config::data_dir().join(REFERENCE_FILE)
}

/// The saved benchmark references, from the executable's folder and from
/// `%APPDATA%\rs-cpuz` alike.
fn saved_references() -> Vec<bench::Reference> {
    merge_references(&config::search_dirs(), &config::data_dir())
}

/// The references in the reference files of `dirs`, in order; for a name saved in more
/// than one, the copy in `preferred` (the folder saves go to) wins.
fn merge_references(dirs: &[PathBuf], preferred: &Path) -> Vec<bench::Reference> {
    let mut merged: Vec<bench::Reference> = Vec::new();
    for dir in dirs {
        for reference in bench::load_references(&dir.join(REFERENCE_FILE)) {
            match merged.iter_mut().find(|known| known.name == reference.name) {
                Some(known) if dir == preferred => *known = reference,
                Some(_) => {}
                None => merged.push(reference),
            }
        }
    }
    merged
}

/// The initial window state for a detected system, its benchmark limited to `threads` threads.
fn initial_state(sys: &SystemInfo, core: Option<u32>, threads: u32) -> UiState {
    UiState::new(sys, core, threads, saved_references())
}

/// Draws images off screen at 96 DPI and saves them as `.bmp` files in one directory.
struct Camera<'a> {
    fonts: Fonts,
    theme: Theme,
    dir: &'a Path,
    written: Vec<PathBuf>,
}

impl Camera<'_> {
    /// Draws a `size` image with `draw` and writes it to `name` in the directory.
    fn shoot(&mut self, name: &str, size: (i32, i32), draw: impl FnOnce(&Painter)) -> io::Result<()> {
        let surface = Surface::new(size.0, size.1)
            .ok_or_else(|| io::Error::other("could not create an off-screen drawing surface"))?;
        draw(&Painter {
            hdc: surface.dc(),
            scale: Scale::new(BASE_DPI),
            fonts: &self.fonts,
            theme: &self.theme,
        });
        let path = self.dir.join(name);
        bmp::save(&surface, &path)?;
        self.written.push(path);
        Ok(())
    }
}

/// Renders every tab to a `.bmp` file in `dir` without showing a window. Returns the files written.
///
/// The images are drawn at 96 DPI: one per tab, named after it, plus
/// `clocks.bmp`, `timers.bmp` and `sensors.bmp` for the tool windows. The CPU tab shows
/// the clocks of `core`, as the window does when it is started with that core.
pub fn snapshot(
    sys: &SystemInfo,
    clocks: &ClockSnapshot,
    cfg: &Config,
    core: Option<u32>,
    dir: &Path,
) -> io::Result<Vec<PathBuf>> {
    let scale = Scale::new(BASE_DPI);
    let mut camera =
        Camera { fonts: Fonts::new(cfg, scale), theme: Theme::new(cfg), dir, written: Vec::new() };

    // Nothing is run for a picture, so the Bench tab is that of the system on show.
    let mut ui = initial_state(sys, core, bench::thread_limit(Some(&sys.cpu)));
    for tab in Tab::ALL {
        ui.tab = tab;
        let view = View { sys, clocks, ui: &ui };
        camera.shoot(tab.file_name(), render::frame_size(scale), |p| render::main_window(p, &view))?;
    }
    let clocks_tool = ClocksTool::new(clocks);
    camera.shoot("clocks.bmp", tools::window_size::<ClocksTool>(scale), |p| {
        tools::draw(p, &clocks_tool, None, None);
    })?;
    let timers_tool = TimersTool::default();
    camera.shoot("timers.bmp", tools::window_size::<TimersTool>(scale), |p| {
        tools::draw(p, &timers_tool, None, None);
    })?;
    let sensors_tool = SensorsTool::new(&sys.sensors);
    camera.shoot("sensors.bmp", tools::window_size::<SensorsTool>(scale), |p| {
        tools::draw(p, &sensors_tool, None, None);
    })?;
    Ok(camera.written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    #[test]
    fn references_are_read_from_both_folders() {
        let root = std::env::temp_dir().join(format!("rs-cpuz-test-{}-references", std::process::id()));
        let (exe, app_data) = (root.join("exe"), root.join("appdata"));
        let save = |dir: &Path, name: &str, single: f64| {
            std::fs::create_dir_all(dir).unwrap();
            let reference = bench::Reference { name: name.into(), single, multi: single * 10.0 };
            bench::save_reference(&dir.join(REFERENCE_FILE), &reference).unwrap();
        };
        save(&exe, "Old PC", 300.0);
        save(&exe, "This PC", 400.0);
        save(&app_data, "This PC", 480.0);
        let merged = merge_references(&[exe.clone(), app_data.clone()], &app_data);
        let _ = std::fs::remove_dir_all(&root);
        let scores: Vec<(&str, f64)> = merged.iter().map(|r| (r.name.as_str(), r.single)).collect();
        assert_eq!(scores, [("Old PC", 300.0), ("This PC", 480.0)]);
    }

    #[test]
    fn snapshot_shows_the_requested_core() {
        let (sys, clocks, cfg) = (demo::system(), demo::clocks(), Config::default());
        let root = std::env::temp_dir().join(format!("rs-cpuz-test-{}-snapshot", std::process::id()));
        // The pictures of one run in the order they were written; `None` in a
        // session where GDI gives out no drawing surface.
        let shoot = |core: Option<u32>, name: &str| {
            let dir = root.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            let files = snapshot(&sys, &clocks, &cfg, core, &dir).ok()?;
            Some(files.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>())
        };
        let shots = (shoot(None, "default"), shoot(Some(3), "core3"));
        let _ = std::fs::remove_dir_all(&root);
        let (Some(default), Some(core3)) = shots else { return };

        assert_eq!(default.len(), Tab::ALL.len() + 3);
        assert_eq!(Tab::ALL[0], Tab::Cpu);
        // Only the CPU tab follows the core; the Clocks tool lists every core anyway.
        assert!(default[0] != core3[0], "the CPU tab ignores the core");
        assert!(default[1..] == core3[1..], "another picture depends on the core");
    }
}
