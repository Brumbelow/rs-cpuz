//! System time sources: their frequencies and a stopwatch that compares them.

pub mod tsc;

use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::SystemInformation::{GetSystemTimePreciseAsFileTime, GetTickCount64};

use crate::model::TimerFrequency;

/// Number of time sources compared by the Timers tool.
pub const TIMER_COUNT: usize = 4;

/// Display names of the time sources, in the order [`TimerSet::elapsed`] reports them.
const NAMES: [&str; TIMER_COUNT] = ["Perf timer", "Sys timer", "TSC", "System time"];

const PERF: usize = 0;
const SYS: usize = 1;
const TSC: usize = 2;
const SYSTEM_TIME: usize = 3;

/// Length of one `FILETIME` unit in seconds.
const HUNDRED_NS: f64 = 1e-7;

/// Counts per second of `GetTickCount64`. It counts milliseconds, however
/// coarse the steps are in which the system clock tick moves it forward.
const TICK_COUNT_HZ: f64 = 1_000.0;

/// Frequencies of the system time sources, for the report.
///
/// Only what user mode can observe is listed, each as the counts per second
/// of its counter: the performance counter (`Perf timer`), the millisecond
/// count of `GetTickCount64` (`Sys timer`) and the measured time-stamp counter
/// (`TSC`). The first call in a process takes [`tsc::MIN_WINDOW`] to measure
/// the latter.
pub fn frequencies() -> Vec<TimerFrequency> {
    let sources = [
        (PERF, perf_frequency().map(|hz| hz as f64)),
        (SYS, Some(TICK_COUNT_HZ)),
        (TSC, tsc::frequency_hz()),
    ];
    sources
        .into_iter()
        .filter_map(|(source, hz)| Some(TimerFrequency { name: NAMES[source].to_string(), hz: hz? }))
        .collect()
}

/// Current value of the performance counter.
fn perf_counter() -> Option<i64> {
    let mut count = 0i64;
    // SAFETY: `count` is a live local the call writes to.
    unsafe { QueryPerformanceCounter(&mut count) }.ok()?;
    Some(count)
}

/// Ticks per second of the performance counter.
fn perf_frequency() -> Option<i64> {
    let mut hz = 0i64;
    // SAFETY: `hz` is a live local the call writes to.
    unsafe { QueryPerformanceFrequency(&mut hz) }.ok()?;
    (hz > 0).then_some(hz)
}

/// A set of free-running counters read together, and the length of one count of each.
trait TickSource {
    fn ticks(&self) -> [u64; TIMER_COUNT];

    /// Seconds per count as known right now; zero for a source that is not available.
    fn seconds_per_tick(&self) -> [f64; TIMER_COUNT];
}

/// The four Windows time sources.
struct SystemTicks;

impl TickSource for SystemTicks {
    fn ticks(&self) -> [u64; TIMER_COUNT] {
        // SAFETY: neither call takes arguments or has preconditions.
        let (uptime_ms, now) = unsafe { (GetTickCount64(), GetSystemTimePreciseAsFileTime()) };
        let mut ticks = [0; TIMER_COUNT];
        ticks[PERF] = perf_counter().map_or(0, |count| count as u64);
        ticks[SYS] = uptime_ms;
        ticks[TSC] = tsc::read();
        ticks[SYSTEM_TIME] = (u64::from(now.dwHighDateTime) << 32) | u64::from(now.dwLowDateTime);
        ticks
    }

    /// The period of the time-stamp counter is its latest measurement, which
    /// gets more precise the longer the process has been running.
    fn seconds_per_tick(&self) -> [f64; TIMER_COUNT] {
        let period = |hz: Option<f64>| hz.map_or(0.0, |hz| 1.0 / hz);
        let mut seconds_per_tick = [0.0; TIMER_COUNT];
        seconds_per_tick[PERF] = period(perf_frequency().map(|hz| hz as f64));
        seconds_per_tick[SYS] = 1.0 / TICK_COUNT_HZ;
        seconds_per_tick[TSC] = period(tsc::frequency_hz());
        seconds_per_tick[SYSTEM_TIME] = HUNDRED_NS;
        seconds_per_tick
    }
}

/// One stretch of a [`Stopwatch`] between a start and the next stop.
#[derive(Debug, Clone, Copy)]
struct Run {
    /// Counter values at the start.
    started: [u64; TIMER_COUNT],
    /// Seconds per count as the source stated them at the start. The run keeps
    /// them, so a source that refines a period does not rescale time already shown.
    scale: [f64; TIMER_COUNT],
}

/// A stopwatch over all sources of a [`TickSource`]. Stopping keeps the elapsed
/// time and starting again continues from it.
struct Stopwatch<S> {
    source: S,
    /// Seconds accumulated by earlier runs.
    banked: [f64; TIMER_COUNT],
    /// The run in progress.
    run: Option<Run>,
}

impl<S: TickSource> Stopwatch<S> {
    fn new(source: S) -> Self {
        Stopwatch { source, banked: [0.0; TIMER_COUNT], run: None }
    }

    fn is_running(&self) -> bool {
        self.run.is_some()
    }

    fn start(&mut self) {
        if self.run.is_none() {
            // The periods come first: getting them may take a moment and the run starts after it.
            let scale = self.source.seconds_per_tick();
            self.run = Some(Run { started: self.source.ticks(), scale });
        }
    }

    fn stop(&mut self) {
        self.banked = self.elapsed();
        self.run = None;
    }

    fn reset(&mut self) {
        self.banked = [0.0; TIMER_COUNT];
        self.run = None;
    }

    fn elapsed(&self) -> [f64; TIMER_COUNT] {
        let Some(Run { started, scale }) = self.run else { return self.banked };
        let now = self.source.ticks();
        // A source may step backwards (the system time when it is adjusted), hence the signed difference.
        std::array::from_fn(|i| self.banked[i] + now[i].wrapping_sub(started[i]) as i64 as f64 * scale[i])
    }
}

/// Runs several independent time sources side by side so drift between them is visible.
pub struct TimerSet {
    watch: Stopwatch<SystemTicks>,
}

impl TimerSet {
    /// Creates a stopped set.
    pub fn new() -> Self {
        TimerSet { watch: Stopwatch::new(SystemTicks) }
    }

    /// Short names of the time sources, in display order.
    pub fn names(&self) -> [&'static str; TIMER_COUNT] {
        NAMES
    }

    pub fn is_running(&self) -> bool {
        self.watch.is_running()
    }

    /// Starts the timers, or resumes them after [`TimerSet::stop`]. Does nothing while they run.
    ///
    /// Each run uses the frequency of the time-stamp counter as measured by then
    /// and keeps it until it stops. If nothing has measured the counter yet, this
    /// takes [`tsc::MIN_WINDOW`].
    pub fn start(&mut self) {
        self.watch.start();
    }

    /// Freezes the timers at their current values.
    pub fn stop(&mut self) {
        self.watch.stop();
    }

    /// Stops the timers and clears them.
    pub fn reset(&mut self) {
        self.watch.reset();
    }

    /// Seconds elapsed on each time source since `start`, not counting the time spent stopped.
    /// A source that is not available stays at zero.
    pub fn elapsed(&self) -> [f64; TIMER_COUNT] {
        self.watch.elapsed()
    }
}

impl Default for TimerSet {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// Counters that only move when the test says so.
    struct FakeTicks {
        now: Cell<[u64; TIMER_COUNT]>,
        scale: Cell<[f64; TIMER_COUNT]>,
    }

    impl FakeTicks {
        const SCALE: [f64; TIMER_COUNT] = [1e-7, 1e-3, 0.0, 1e-7];

        /// Moves every counter forward by `ms` milliseconds (the third one is unavailable and free-runs).
        fn advance(&self, ms: u64) {
            let mut now = self.now.get();
            now[PERF] += ms * 10_000;
            now[SYS] += ms;
            now[TSC] += ms * 3_800_000;
            now[SYSTEM_TIME] = now[SYSTEM_TIME].wrapping_add(ms * 10_000);
            self.now.set(now);
        }
    }

    impl TickSource for FakeTicks {
        fn ticks(&self) -> [u64; TIMER_COUNT] {
            self.now.get()
        }

        fn seconds_per_tick(&self) -> [f64; TIMER_COUNT] {
            self.scale.get()
        }
    }

    fn watch() -> Stopwatch<FakeTicks> {
        Stopwatch::new(FakeTicks {
            now: Cell::new([77, 1_000, 123_456_789, u64::MAX - 5_000]),
            scale: Cell::new(FakeTicks::SCALE),
        })
    }

    fn assert_secs(actual: [f64; TIMER_COUNT], expected: [f64; TIMER_COUNT]) {
        for (a, e) in actual.iter().zip(expected) {
            assert!((a - e).abs() < 1e-9, "{actual:?} != {expected:?}");
        }
    }

    #[test]
    fn idle_until_started() {
        let w = watch();
        assert!(!w.is_running());
        w.source.advance(500);
        assert_secs(w.elapsed(), [0.0; TIMER_COUNT]);
    }

    #[test]
    fn runs_freezes_and_resumes() {
        let mut w = watch();
        w.start();
        assert!(w.is_running());
        w.source.advance(250);
        assert_secs(w.elapsed(), [0.25, 0.25, 0.0, 0.25]);

        w.stop();
        assert!(!w.is_running());
        w.source.advance(10_000);
        assert_secs(w.elapsed(), [0.25, 0.25, 0.0, 0.25]);

        w.start();
        w.source.advance(750);
        assert_secs(w.elapsed(), [1.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn start_while_running_does_not_restart() {
        let mut w = watch();
        w.start();
        w.source.advance(100);
        w.start();
        w.source.advance(100);
        assert_secs(w.elapsed(), [0.2, 0.2, 0.0, 0.2]);
    }

    #[test]
    fn reset_stops_and_clears() {
        let mut w = watch();
        w.start();
        w.source.advance(100);
        w.reset();
        assert!(!w.is_running());
        w.source.advance(100);
        assert_secs(w.elapsed(), [0.0; TIMER_COUNT]);

        w.start();
        w.source.advance(40);
        w.stop();
        w.reset();
        assert_secs(w.elapsed(), [0.0; TIMER_COUNT]);
    }

    #[test]
    fn stop_without_start_keeps_zero() {
        let mut w = watch();
        w.stop();
        assert_secs(w.elapsed(), [0.0; TIMER_COUNT]);
    }

    #[test]
    fn a_source_stepping_backwards_gives_a_negative_time() {
        let mut w = watch();
        w.start();
        let mut now = w.source.now.get();
        now[SYSTEM_TIME] = now[SYSTEM_TIME].wrapping_sub(20_000);
        w.source.now.set(now);
        assert_secs(w.elapsed(), [0.0, 0.0, 0.0, -0.002]);
    }

    #[test]
    fn a_run_keeps_the_periods_it_started_with() {
        let mut w = watch();
        w.start();
        w.source.advance(100);
        // The source now takes a count of its second counter to last twice as long.
        let mut refined = FakeTicks::SCALE;
        refined[SYS] *= 2.0;
        w.source.scale.set(refined);
        w.source.advance(100);
        assert_secs(w.elapsed(), [0.2, 0.2, 0.0, 0.2]);

        w.stop();
        assert_secs(w.elapsed(), [0.2, 0.2, 0.0, 0.2]);
        w.start();
        w.source.advance(100);
        assert_secs(w.elapsed(), [0.3, 0.4, 0.0, 0.3]);
    }

    #[test]
    fn names_are_distinct_and_filled() {
        let names = TimerSet::new().names();
        for (i, name) in names.iter().enumerate() {
            assert!(!name.is_empty());
            assert!(!names[..i].contains(name));
        }
    }

    #[test]
    fn frequencies_are_named_and_positive() {
        let list = frequencies();
        assert!(list.iter().all(|t| NAMES.contains(&t.name.as_str()) && t.hz > 0.0));
        assert!(list.iter().any(|t| t.name == NAMES[PERF]));
    }

    #[test]
    fn sys_timer_is_reported_in_the_unit_it_is_counted_in() {
        let hz_of = |source: usize| frequencies().into_iter().find(|t| t.name == NAMES[source]).map(|t| t.hz);
        assert_eq!(hz_of(SYS), Some(1_000.0));
        // Every reported frequency is the reciprocal of the period the stopwatch runs that source at.
        let periods = SystemTicks.seconds_per_tick();
        assert_eq!(periods[SYS], 1e-3);
        assert_eq!(hz_of(PERF).map(|hz| 1.0 / hz), Some(periods[PERF]));
    }

    #[test]
    fn real_sources_advance_together() {
        let mut timers = TimerSet::new();
        assert_eq!(timers.elapsed(), [0.0; TIMER_COUNT]);
        timers.start();
        assert!(timers.is_running());
        std::thread::sleep(std::time::Duration::from_millis(60));
        timers.stop();
        assert!(!timers.is_running());
        let frozen = timers.elapsed();
        assert!(frozen[PERF] > 0.03 && frozen[PERF] < 5.0, "{frozen:?}");
        assert!((frozen[SYS] - frozen[PERF]).abs() < 0.05, "{frozen:?}");
        if tsc::frequency_hz().is_some() {
            assert!((frozen[TSC] / frozen[PERF] - 1.0).abs() < 0.2, "{frozen:?}");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(timers.elapsed(), frozen);
        timers.reset();
        assert_eq!(timers.elapsed(), [0.0; TIMER_COUNT]);
    }
}
