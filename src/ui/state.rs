//! What the main window currently shows: the selected tab, the selectors and the benchmark.
//!
//! Everything here is plain data so the renderer can draw the live window and
//! the snapshots from the same description.

use crate::bench::{Event, Phase, Reference};
use crate::model::{ClockSnapshot, CoreClock, GpuInfo, GpuPerfLevel, MemoryModule, SystemInfo};

use super::format;
use super::layout::Tab;

/// Something in the main window that reacts to the mouse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Tab(Tab),
    Minimize,
    CloseBox,
    /// The "Tools" drop-down button in the bottom row.
    Tools,
    /// The "Close" button in the bottom row.
    Close,
    /// An interactive control of the current tab page, by identifier.
    Control(u16),
}

/// The lists that open in a popup: the selectors and the two menus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListId {
    Socket,
    Slot,
    Adapter,
    PerfLevel,
    Threads,
    Reference,
    ToolsMenu,
    CoreMenu,
}

impl ListId {
    const ALL: [ListId; 8] = [
        ListId::Socket,
        ListId::Slot,
        ListId::Adapter,
        ListId::PerfLevel,
        ListId::Threads,
        ListId::Reference,
        ListId::ToolsMenu,
        ListId::CoreMenu,
    ];

    /// Stable number used to pass the list through a window message.
    pub fn code(self) -> usize {
        self as usize
    }

    pub fn from_code(code: usize) -> Option<ListId> {
        Self::ALL.get(code).copied()
    }
}

/// What an entry of the "Tools" menu does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolsEntry {
    SaveTxt,
    SaveHtml,
    Clocks,
    Timers,
    Sensors,
    RestartAsAdmin,
}

impl ToolsEntry {
    pub fn label(self) -> &'static str {
        match self {
            ToolsEntry::SaveTxt => "Save Report (.TXT)...",
            ToolsEntry::SaveHtml => "Save Report (.HTML)...",
            ToolsEntry::Clocks => "Clocks",
            ToolsEntry::Timers => "Timers",
            ToolsEntry::Sensors => "Sensors",
            ToolsEntry::RestartAsAdmin => "Restart as Administrator",
        }
    }
}

/// Entries of the "Tools" menu, in display order. `offer_admin` adds the entry that
/// restarts the program with administrator rights.
pub fn tools_menu(offer_admin: bool) -> Vec<ToolsEntry> {
    let mut entries = vec![
        ToolsEntry::SaveTxt,
        ToolsEntry::SaveHtml,
        ToolsEntry::Clocks,
        ToolsEntry::Timers,
        ToolsEntry::Sensors,
    ];
    if offer_admin {
        entries.push(ToolsEntry::RestartAsAdmin);
    }
    entries
}

/// First entry of the reference selector, shown while no reference is chosen.
pub const NO_REFERENCE: &str = "<Please Select>";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BenchMode {
    #[default]
    Idle,
    Benching,
    Stressing,
}

/// Fill level and caption of one benchmark bar.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BarView {
    /// Hidden bars are not drawn at all.
    pub visible: bool,
    /// Filled part, `0.0..=1.0`.
    pub fraction: f32,
    pub text: String,
    /// Reference bars use a neutral fill instead of the accent colour.
    pub reference: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BenchState {
    /// Threads used by the multi-thread run and the stress test.
    pub threads: u32,
    /// Number of logical processors, the upper end of the "Run on" selector.
    pub max_threads: u32,
    /// Threads the scores on display were measured with; the selector may have moved on since.
    pub run_threads: u32,
    /// The window shows the built-in sample system. The benchmark still measures the
    /// real processor, so its scores do not belong to the processor named on the page.
    pub sample: bool,
    pub mode: BenchMode,
    /// Phase in progress and its completion, `0.0..=1.0`.
    pub progress: Option<(Phase, f32)>,
    pub single: Option<f64>,
    pub multi: Option<f64>,
    /// Whether the reference bar of the single / multi group is shown.
    pub show_reference: [bool; 2],
    pub references: Vec<Reference>,
    pub reference: Option<usize>,
}

fn phase_index(phase: Phase) -> usize {
    match phase {
        Phase::Single => 0,
        Phase::Multi => 1,
    }
}

impl BenchState {
    pub fn new(max_threads: u32, references: Vec<Reference>) -> Self {
        let max_threads = max_threads.max(1);
        BenchState {
            threads: max_threads,
            max_threads,
            run_threads: max_threads,
            sample: false,
            mode: BenchMode::Idle,
            progress: None,
            single: None,
            multi: None,
            show_reference: [true, true],
            references,
            reference: None,
        }
    }

    /// Clears the previous scores and enters the benchmarking state.
    pub fn begin_bench(&mut self) {
        self.mode = BenchMode::Benching;
        self.run_threads = self.threads;
        self.single = None;
        self.multi = None;
        self.progress = Some((Phase::Single, 0.0));
    }

    /// Returns to idle, dropping any unfinished progress.
    pub fn end(&mut self) {
        self.mode = BenchMode::Idle;
        self.progress = None;
    }

    /// Applies an event of the running benchmark. Events arriving while idle are ignored.
    pub fn apply(&mut self, event: Event) {
        if self.mode != BenchMode::Benching {
            return;
        }
        match event {
            Event::Progress(phase, fraction) => self.progress = Some((phase, fraction.clamp(0.0, 1.0))),
            Event::Score(Phase::Single, score) => self.single = Some(score),
            Event::Score(Phase::Multi, score) => self.multi = Some(score),
            Event::Finished | Event::Cancelled => self.end(),
        }
    }

    pub fn toggle_reference(&mut self, phase: Phase) {
        let shown = &mut self.show_reference[phase_index(phase)];
        *shown = !*shown;
    }

    pub fn reference_shown(&self, phase: Phase) -> bool {
        self.show_reference[phase_index(phase)]
    }

    fn score(&self, phase: Phase) -> Option<f64> {
        match phase {
            Phase::Single => self.single,
            Phase::Multi => self.multi,
        }
    }

    /// Score of the selected reference for `phase`, when its bar is shown.
    fn reference_score(&self, phase: Phase) -> Option<f64> {
        if !self.reference_shown(phase) {
            return None;
        }
        let reference = self.references.get(self.reference?)?;
        let score = match phase {
            Phase::Single => reference.single,
            Phase::Multi => reference.multi,
        };
        (score > 0.0).then_some(score)
    }

    /// Multi-thread to single-thread ratio once both scores exist.
    pub fn ratio(&self) -> Option<f64> {
        match (self.single, self.multi) {
            (Some(single), Some(multi)) if single > 0.0 => Some(multi / single),
            _ => None,
        }
    }

    /// Both scores are available, nothing is running and the scores are those of
    /// the processor on show, so they may be stored as a reference.
    pub fn can_save(&self) -> bool {
        !self.sample && self.mode == BenchMode::Idle && self.single.is_some() && self.multi.is_some()
    }

    /// The bar of this processor (`reference == false`) or of the reference for `phase`.
    /// Both bars of a group scale against the larger of the two scores.
    pub fn bar(&self, phase: Phase, reference: bool) -> BarView {
        let own = self.score(phase);
        let other = self.reference_score(phase);
        let full = own.unwrap_or(0.0).max(other.unwrap_or(0.0));
        let scaled = |score: f64| if full > 0.0 { (score / full) as f32 } else { 0.0 };
        if reference {
            return match other {
                Some(score) => {
                    BarView { visible: true, fraction: scaled(score), text: format::score(score), reference }
                }
                None => BarView { visible: self.reference_shown(phase), reference, ..BarView::default() },
            };
        }
        match (self.progress, own) {
            (Some((running, fraction)), _) if running == phase && self.mode == BenchMode::Benching => {
                BarView { visible: true, fraction, text: format::percent(fraction), reference }
            }
            (_, Some(score)) => {
                BarView { visible: true, fraction: scaled(score), text: format::score(score), reference }
            }
            _ => BarView { visible: true, reference, ..BarView::default() },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UiState {
    pub tab: Tab,
    /// Control under the mouse.
    pub hot: Option<Target>,
    /// Control the left button went down on.
    pub pressed: Option<Target>,
    /// List whose popup is open.
    pub open: Option<ListId>,
    /// Core whose clocks the CPU tab shows, as its index inside the package.
    pub core: u32,
    pub slot: usize,
    pub gpu: usize,
    pub perf_level: usize,
    /// The Tools menu offers to restart with administrator rights.
    pub offer_admin: bool,
    pub bench: BenchState,
}

impl UiState {
    /// Initial state: first populated memory slot, primary display adapter with its
    /// live clocks and the requested core.
    pub fn new(sys: &SystemInfo, core: Option<u32>, max_threads: u32, references: Vec<Reference>) -> Self {
        let gpu = sys.gpus.iter().position(|gpu| gpu.primary).unwrap_or(0);
        UiState {
            tab: Tab::Cpu,
            hot: None,
            pressed: None,
            open: None,
            core: core.unwrap_or(0),
            slot: sys.modules.iter().position(|module| module.populated).unwrap_or(0),
            gpu,
            perf_level: sys.gpus.get(gpu).map_or(0, GpuInfo::current_perf_level),
            offer_admin: false,
            bench: BenchState::new(max_threads, references),
        }
    }

    /// The same state without transient mouse feedback, for saved images.
    pub fn at_rest(&self) -> UiState {
        UiState { hot: None, pressed: None, open: None, ..self.clone() }
    }
}

/// Everything the renderer needs to draw the main window.
#[derive(Clone, Copy)]
pub struct View<'a> {
    pub sys: &'a SystemInfo,
    pub clocks: &'a ClockSnapshot,
    pub ui: &'a UiState,
}

impl<'a> View<'a> {
    /// Clocks of the selected core; the first core when the selection does not exist.
    pub fn core(&self) -> Option<&'a CoreClock> {
        self.clocks
            .cores
            .iter()
            .find(|clock| clock.core == self.ui.core)
            .or_else(|| self.clocks.cores.first())
    }

    pub fn module(&self) -> Option<&'a MemoryModule> {
        self.sys.modules.get(self.ui.slot)
    }

    pub fn gpu(&self) -> Option<&'a GpuInfo> {
        self.sys.gpus.get(self.ui.gpu)
    }

    pub fn perf_level(&self) -> Option<&'a GpuPerfLevel> {
        self.gpu()?.perf_levels.get(self.ui.perf_level)
    }

    /// Entries of a selector or menu.
    pub fn list_items(&self, list: ListId) -> Vec<String> {
        match list {
            // Every page describes the first package, so that is the only entry on offer.
            ListId::Socket => vec!["Socket #1".to_string()],
            ListId::Slot => self
                .sys
                .modules
                .iter()
                .enumerate()
                .map(|(i, m)| if m.slot.is_empty() { format!("Slot #{}", i + 1) } else { m.slot.clone() })
                .collect(),
            ListId::Adapter => self.sys.gpus.iter().map(|gpu| gpu.name.clone()).collect(),
            ListId::PerfLevel => self
                .gpu()
                .map(|gpu| gpu.perf_levels.iter().map(|l| l.name.clone()).collect())
                .unwrap_or_default(),
            ListId::Threads => (1..=self.ui.bench.max_threads).map(format::thread_count).collect(),
            ListId::Reference => std::iter::once(NO_REFERENCE.to_string())
                .chain(self.ui.bench.references.iter().map(|r| r.name.clone()))
                .collect(),
            ListId::ToolsMenu => {
                tools_menu(self.ui.offer_admin).into_iter().map(|entry| entry.label().to_string()).collect()
            }
            ListId::CoreMenu => {
                self.clocks.cores.iter().map(|clock| format::core_label(clock.kind, clock.core)).collect()
            }
        }
    }

    /// Index of the current entry of a list; menus without a current entry return `None`.
    pub fn list_selection(&self, list: ListId) -> Option<usize> {
        let ui = self.ui;
        let within = |index: usize, len: usize| (index < len).then_some(index);
        match list {
            ListId::Socket => Some(0),
            ListId::Slot => within(ui.slot, self.sys.modules.len()),
            ListId::Adapter => within(ui.gpu, self.sys.gpus.len()),
            ListId::PerfLevel => within(ui.perf_level, self.gpu().map_or(0, |gpu| gpu.perf_levels.len())),
            ListId::Threads => Some(ui.bench.threads.clamp(1, ui.bench.max_threads) as usize - 1),
            ListId::Reference => Some(ui.bench.reference.map_or(0, |index| index + 1)),
            ListId::ToolsMenu => None,
            ListId::CoreMenu => {
                let core = self.core()?.core;
                self.clocks.cores.iter().position(|clock| clock.core == core)
            }
        }
    }

    /// Text shown in the closed selector.
    pub fn list_text(&self, list: ListId) -> String {
        let index = self.list_selection(list);
        index.and_then(|index| self.list_items(list).into_iter().nth(index)).unwrap_or_default()
    }
}

impl UiState {
    /// Applies the choice of entry `index` in a selector. Menus are handled by the window.
    pub fn choose(&mut self, list: ListId, index: usize, sys: &SystemInfo, clocks: &ClockSnapshot) {
        match list {
            ListId::Slot if index < sys.modules.len() => self.slot = index,
            ListId::Adapter if index < sys.gpus.len() => {
                if self.gpu != index {
                    self.perf_level = sys.gpus[index].current_perf_level();
                }
                self.gpu = index;
            }
            ListId::PerfLevel => {
                let levels = sys.gpus.get(self.gpu).map_or(0, |gpu| gpu.perf_levels.len());
                if index < levels {
                    self.perf_level = index;
                }
            }
            ListId::Threads => {
                if let Ok(index) = u32::try_from(index) {
                    if index < self.bench.max_threads {
                        self.bench.threads = index + 1;
                    }
                }
            }
            ListId::Reference if index <= self.bench.references.len() => {
                self.bench.reference = index.checked_sub(1);
            }
            ListId::CoreMenu => {
                if let Some(clock) = clocks.cores.get(index) {
                    self.core = clock.core;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    fn state(sys: &SystemInfo) -> UiState {
        UiState::new(sys, None, sys.cpu.threads, Vec::new())
    }

    fn reference(name: &str, single: f64, multi: f64) -> Reference {
        Reference { name: name.into(), single, multi }
    }

    #[test]
    fn initial_selection_prefers_populated_slot_and_primary_adapter() {
        let sys = demo::system();
        let ui = state(&sys);
        assert_eq!(ui.tab, Tab::Cpu);
        assert_eq!(ui.slot, 1);
        assert_eq!(ui.gpu, 0);
        assert_eq!(ui.bench.threads, 16);
        assert_eq!(ui.bench.max_threads, 16);

        let empty = SystemInfo::default();
        let ui = state(&empty);
        assert_eq!((ui.slot, ui.gpu, ui.bench.threads), (0, 0, 1));
    }

    #[test]
    fn core_selection_follows_the_snapshot() {
        let sys = demo::system();
        let clocks = demo::clocks();
        let mut ui = state(&sys);
        ui.core = 3;
        let view = View { sys: &sys, clocks: &clocks, ui: &ui };
        assert_eq!(view.core().map(|c| c.core), Some(3));
        assert_eq!(view.list_selection(ListId::CoreMenu), Some(3));
        assert_eq!(view.list_items(ListId::CoreMenu).len(), clocks.cores.len());
        assert_eq!(view.list_items(ListId::CoreMenu)[3], "Core #3");

        ui.core = 99;
        let view = View { sys: &sys, clocks: &clocks, ui: &ui };
        assert_eq!(view.core().map(|c| c.core), Some(0));

        let none = ClockSnapshot::default();
        let view = View { sys: &sys, clocks: &none, ui: &ui };
        assert!(view.core().is_none());
        assert_eq!(view.list_selection(ListId::CoreMenu), None);
    }

    #[test]
    fn selector_lists() {
        let sys = demo::system();
        let clocks = demo::clocks();
        let ui = state(&sys);
        let view = View { sys: &sys, clocks: &clocks, ui: &ui };
        assert_eq!(view.list_items(ListId::Socket), ["Socket #1"]);
        assert_eq!(view.list_text(ListId::Socket), "Socket #1");
        assert_eq!(view.list_items(ListId::Slot), ["Slot #1", "Slot #2", "Slot #3", "Slot #4"]);
        assert_eq!(view.list_text(ListId::Slot), "Slot #2");
        assert_eq!(view.list_items(ListId::Adapter).len(), 2);
        assert_eq!(view.list_text(ListId::PerfLevel), "Current");
        assert_eq!(view.list_items(ListId::Threads).len(), 16);
        assert_eq!(view.list_text(ListId::Threads), "16 threads");
        assert_eq!(view.list_text(ListId::Reference), NO_REFERENCE);
        assert_eq!(view.list_selection(ListId::ToolsMenu), None);
        assert_eq!(
            view.list_items(ListId::ToolsMenu),
            ["Save Report (.TXT)...", "Save Report (.HTML)...", "Clocks", "Timers", "Sensors"]
        );
        assert_eq!(tools_menu(true).last(), Some(&ToolsEntry::RestartAsAdmin));
        assert_eq!(ToolsEntry::RestartAsAdmin.label(), "Restart as Administrator");
    }

    #[test]
    fn choices_are_range_checked() {
        let sys = demo::system();
        let clocks = demo::clocks();
        let mut ui = state(&sys);
        ui.bench.references = vec![reference("A", 1.0, 2.0)];

        ui.choose(ListId::Slot, 3, &sys, &clocks);
        assert_eq!(ui.slot, 3);
        ui.choose(ListId::Slot, 9, &sys, &clocks);
        assert_eq!(ui.slot, 3);

        ui.perf_level = 0;
        ui.choose(ListId::Adapter, 1, &sys, &clocks);
        assert_eq!((ui.gpu, ui.perf_level), (1, 0));
        ui.choose(ListId::PerfLevel, 0, &sys, &clocks);
        assert_eq!(ui.perf_level, 0);

        ui.choose(ListId::Threads, 3, &sys, &clocks);
        assert_eq!(ui.bench.threads, 4);
        ui.choose(ListId::Threads, 16, &sys, &clocks);
        assert_eq!(ui.bench.threads, 4);

        ui.choose(ListId::Reference, 1, &sys, &clocks);
        assert_eq!(ui.bench.reference, Some(0));
        ui.choose(ListId::Reference, 0, &sys, &clocks);
        assert_eq!(ui.bench.reference, None);
        ui.choose(ListId::Reference, 5, &sys, &clocks);
        assert_eq!(ui.bench.reference, None);

        ui.choose(ListId::CoreMenu, 5, &sys, &clocks);
        assert_eq!(ui.core, 5);
        ui.choose(ListId::CoreMenu, 50, &sys, &clocks);
        assert_eq!(ui.core, 5);
    }

    #[test]
    fn socket_selector_offers_the_package_on_show() {
        // The model describes the first package, however many the machine has.
        let mut sys = demo::system();
        sys.cpu.sockets = 2;
        let clocks = demo::clocks();
        let mut ui = state(&sys);
        let before = ui.clone();
        ui.choose(ListId::Socket, 1, &sys, &clocks);
        ui.choose(ListId::Socket, 0, &sys, &clocks);
        assert_eq!(ui, before);
        let view = View { sys: &sys, clocks: &clocks, ui: &ui };
        assert_eq!(view.list_items(ListId::Socket), ["Socket #1"]);
        assert_eq!(view.list_selection(ListId::Socket), Some(0));
    }

    #[test]
    fn scores_keep_the_thread_count_they_were_measured_with() {
        let sys = demo::system();
        let clocks = demo::clocks();
        let mut ui = state(&sys);
        assert_eq!(ui.bench.run_threads, 16);
        ui.choose(ListId::Threads, 3, &sys, &clocks);
        assert_eq!(ui.bench.run_threads, 16, "nothing has run on four threads yet");
        ui.bench.begin_bench();
        ui.bench.apply(Event::Score(Phase::Single, 500.0));
        ui.bench.apply(Event::Score(Phase::Multi, 1900.0));
        ui.bench.apply(Event::Finished);

        // The selector works again once the run is over; the scores stay four-thread scores.
        ui.choose(ListId::Threads, 15, &sys, &clocks);
        assert_eq!((ui.bench.threads, ui.bench.run_threads), (16, 4));
        assert!(ui.bench.can_save());
        ui.bench.begin_bench();
        assert_eq!(ui.bench.run_threads, 16);
    }

    #[test]
    fn sample_system_scores_cannot_be_saved() {
        let mut bench = BenchState::new(8, Vec::new());
        bench.single = Some(700.0);
        bench.multi = Some(5600.0);
        assert!(bench.can_save());
        bench.sample = true;
        assert!(!bench.can_save());
    }

    #[test]
    fn adapters_open_on_their_live_clocks() {
        let level = |name: &str, mhz: f64| GpuPerfLevel {
            name: name.into(),
            core_mhz: Some(mhz),
            ..Default::default()
        };
        let mut sys = demo::system();
        sys.gpus[0].perf_levels =
            vec![level("Base", 1830.0), level("Boost", 2460.0), level("Current", 285.0)];
        let clocks = demo::clocks();

        let mut ui = state(&sys);
        assert_eq!((ui.gpu, ui.perf_level), (0, 2));
        let view = View { sys: &sys, clocks: &clocks, ui: &ui };
        assert_eq!(view.list_items(ListId::PerfLevel), ["Base", "Boost", "Current"]);
        assert_eq!(view.list_text(ListId::PerfLevel), "Current");
        assert_eq!(view.perf_level().and_then(|level| level.core_mhz), Some(285.0));

        // Another level stays selected while the adapter does, and is given up with it.
        ui.choose(ListId::PerfLevel, 1, &sys, &clocks);
        ui.choose(ListId::Adapter, 0, &sys, &clocks);
        assert_eq!((ui.gpu, ui.perf_level), (0, 1));
        ui.choose(ListId::Adapter, 1, &sys, &clocks);
        assert_eq!((ui.gpu, ui.perf_level), (1, 0));
        ui.choose(ListId::Adapter, 0, &sys, &clocks);
        assert_eq!((ui.gpu, ui.perf_level), (0, 2));
    }

    #[test]
    fn list_codes_round_trip() {
        for list in ListId::ALL {
            assert_eq!(ListId::from_code(list.code()), Some(list));
        }
        assert_eq!(ListId::from_code(99), None);
    }

    #[test]
    fn bench_events_drive_the_state() {
        let mut bench = BenchState::new(8, Vec::new());
        bench.apply(Event::Score(Phase::Single, 5.0));
        assert_eq!(bench.single, None, "events are ignored while idle");

        bench.begin_bench();
        assert_eq!(bench.mode, BenchMode::Benching);
        bench.apply(Event::Progress(Phase::Single, 0.5));
        let bar = bench.bar(Phase::Single, false);
        assert_eq!((bar.fraction, bar.text.as_str()), (0.5, "50 %"));
        bench.apply(Event::Score(Phase::Single, 700.0));
        bench.apply(Event::Progress(Phase::Multi, 0.25));
        assert_eq!(bench.bar(Phase::Single, false).text, "700.0");
        assert_eq!(bench.bar(Phase::Multi, false).text, "25 %");
        assert!(!bench.can_save());
        bench.apply(Event::Score(Phase::Multi, 7000.0));
        bench.apply(Event::Finished);
        assert_eq!(bench.mode, BenchMode::Idle);
        assert_eq!(bench.progress, None);
        assert_eq!(bench.ratio(), Some(10.0));
        assert!(bench.can_save());

        bench.begin_bench();
        assert_eq!((bench.single, bench.multi), (None, None));
        bench.apply(Event::Cancelled);
        assert_eq!(bench.mode, BenchMode::Idle);
        assert_eq!(bench.ratio(), None);
    }

    #[test]
    fn bars_scale_against_the_larger_score() {
        let mut bench = BenchState::new(8, vec![reference("Ref", 400.0, 8000.0)]);
        bench.single = Some(800.0);
        bench.multi = Some(4000.0);

        let own = bench.bar(Phase::Single, false);
        assert_eq!((own.fraction, own.text.as_str()), (1.0, "800.0"));
        let other = bench.bar(Phase::Single, true);
        assert!(other.visible && other.text.is_empty(), "no reference selected: empty bar");

        bench.reference = Some(0);
        assert_eq!(bench.bar(Phase::Single, true).fraction, 0.5);
        assert_eq!(bench.bar(Phase::Multi, false).fraction, 0.5);
        let other = bench.bar(Phase::Multi, true);
        assert_eq!((other.fraction, other.text.as_str(), other.reference), (1.0, "8000.0", true));

        bench.toggle_reference(Phase::Multi);
        assert!(!bench.bar(Phase::Multi, true).visible);
        assert_eq!(bench.bar(Phase::Multi, false).fraction, 1.0);
        assert!(bench.bar(Phase::Single, true).visible);
    }

    #[test]
    fn rest_state_drops_mouse_feedback() {
        let sys = demo::system();
        let mut ui = state(&sys);
        ui.hot = Some(Target::Close);
        ui.pressed = Some(Target::Tab(Tab::Spd));
        ui.open = Some(ListId::Slot);
        ui.tab = Tab::Spd;
        let rest = ui.at_rest();
        assert_eq!((rest.hot, rest.pressed, rest.open, rest.tab), (None, None, None, Tab::Spd));
    }
}
