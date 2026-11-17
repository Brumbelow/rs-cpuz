//! The processor's time-stamp counter: reading it and measuring its frequency
//! against the performance counter.
//!
//! The first measurement is the classic one: a thread pinned to one processor
//! reads both counters, sleeps for a short window and reads them again. It runs
//! on a helper thread so that the caller keeps its affinity. After that every
//! request adds one more pair of readings and the frequency becomes the slope
//! over a window of a few seconds, which costs nothing and is far more precise.
//! Those later readings come from whichever processor the caller is on. The
//! counters of the processors are kept in step (Windows builds its own
//! performance counter on that) and what offset remains is negligible against
//! a window that long.

#[cfg(target_arch = "x86")]
use core::arch::x86::_rdtsc;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::_rdtsc;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use windows::Win32::System::Threading::GetCurrentProcessorNumberEx;

use super::{perf_counter, perf_frequency};
use crate::hw::cpu::cpuid;
use crate::hw::processors::{Pin, ProcId};

/// Shortest window a frequency is derived from. With the performance counter's
/// usual 100 ns resolution this bounds the error to a few parts per million.
pub const MIN_WINDOW: Duration = Duration::from_millis(50);

/// Once the window is this long its start slides forward, so the result follows
/// a clock that changes and never spans more than twice this time.
const SLIDE_SECS: f64 = 4.0;

/// Largest relative change between two results that is still taken as the same
/// clock. Anything bigger (a resume from sleep resets the counter) restarts the window.
const MAX_STEP: f64 = 0.01;

/// Tries for a pair whose two performance-counter readings coincide.
const PAIR_ATTEMPTS: usize = 4;

/// How often the first measurement sleeps for [`MIN_WINDOW`] before giving up
/// (a sleep can end early; one is normally enough).
const WAIT_ATTEMPTS: usize = 3;

/// Current value of the time-stamp counter.
pub fn read() -> u64 {
    // SAFETY: RDTSC only reads a counter; Windows never restricts it to ring 0.
    unsafe { _rdtsc() }
}

/// Whether the counter runs at a constant rate in every power state
/// (CPUID leaf 0x8000_0007, EDX bit 8).
pub fn is_invariant() -> bool {
    cpuid::read_supported(0x8000_0007).is_some_and(|leaf| cpuid::bit(leaf.edx, 8))
}

/// The time-stamp counter and the performance counter, read at the same moment.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Reading {
    tsc: u64,
    /// Performance-counter ticks; fractional because it is the middle of a bracket.
    qpc: f64,
}

/// Turns a stream of [`Reading`]s into the counter frequency.
#[derive(Debug, Clone)]
struct Calibrator {
    qpc_hz: f64,
    anchor: Reading,
    next_anchor: Option<Reading>,
    hz: Option<f64>,
    /// The window was restarted; its first result replaces `hz` without being compared to it.
    restarted: bool,
}

impl Calibrator {
    fn new(qpc_hz: f64, first: Reading) -> Self {
        Calibrator { qpc_hz, anchor: first, next_anchor: None, hz: None, restarted: false }
    }

    /// Feeds a new reading and returns the frequency in Hz, once a window of
    /// [`MIN_WINDOW`] has been seen. After a discontinuity the last good value
    /// is kept until a fresh window is complete.
    fn update(&mut self, now: Reading) -> Option<f64> {
        let span = self.seconds(self.anchor, now);
        // Either counter running backwards means the window no longer describes one clock.
        let Some(ticks) = now.tsc.checked_sub(self.anchor.tsc).filter(|_| span >= 0.0) else {
            self.restart(now);
            return self.hz;
        };
        if span < MIN_WINDOW.as_secs_f64() {
            return self.hz;
        }
        let rate = ticks as f64 / span;
        let jumped = self.hz.is_some_and(|hz| (rate / hz - 1.0).abs() > MAX_STEP);
        if jumped && !self.restarted {
            self.restart(now);
            return self.hz;
        }
        self.hz = Some(rate);
        self.restarted = false;
        self.slide(now, span);
        self.hz
    }

    fn seconds(&self, from: Reading, to: Reading) -> f64 {
        (to.qpc - from.qpc) / self.qpc_hz
    }

    fn restart(&mut self, now: Reading) {
        self.anchor = now;
        self.next_anchor = None;
        self.restarted = true;
    }

    /// Moves the window start forward so it stays between one and two [`SLIDE_SECS`] long.
    fn slide(&mut self, now: Reading, span: f64) {
        match self.next_anchor {
            Some(next) if self.seconds(next, now) >= SLIDE_SECS => {
                self.anchor = next;
                self.next_anchor = Some(now);
            }
            None if span >= SLIDE_SECS => self.next_anchor = Some(now),
            _ => {}
        }
    }
}

/// Reads both counters, keeping the attempt in which the performance counter
/// moved the least around the TSC read; a preemption in between shows as a wide gap.
fn read_pair() -> Option<Reading> {
    let mut best: Option<(i64, Reading)> = None;
    for _ in 0..PAIR_ATTEMPTS {
        let before = perf_counter()?;
        let tsc = read();
        let after = perf_counter()?;
        let spread = after - before;
        if best.map_or(true, |(least, _)| spread < least) {
            best = Some((spread, Reading { tsc, qpc: (before as f64 + after as f64) / 2.0 }));
        }
        if spread <= 0 {
            break;
        }
    }
    best.map(|(_, reading)| reading)
}

/// Measures over one window with the calling thread pinned to the processor it is on.
fn first_window(qpc_hz: f64) -> Option<Calibrator> {
    // SAFETY: the call only fills in the structure it returns.
    let here = unsafe { GetCurrentProcessorNumberEx() };
    let _pin = Pin::to(ProcId { group: here.Group, number: here.Number });
    let mut calibrator = Calibrator::new(qpc_hz, read_pair()?);
    for _ in 0..WAIT_ATTEMPTS {
        std::thread::sleep(MIN_WINDOW);
        if calibrator.update(read_pair()?).is_some() {
            break;
        }
    }
    Some(calibrator)
}

/// Holds the measurement once the first window has been taken.
static SHARED: OnceLock<Option<Mutex<Calibrator>>> = OnceLock::new();

/// The process-wide measurement shared by the clock monitor and the timers.
/// `None` when there is no performance counter to compare the counter with.
fn shared() -> Option<&'static Mutex<Calibrator>> {
    SHARED
        .get_or_init(|| {
            let qpc_hz = perf_frequency()? as f64;
            // The window pins its thread, so it gets one of its own.
            let calibrator = std::thread::scope(|scope| {
                let helper = std::thread::Builder::new().spawn_scoped(scope, || first_window(qpc_hz)).ok()?;
                helper.join().ok()?
            })?;
            Some(Mutex::new(calibrator))
        })
        .as_ref()
}

/// Measured frequency of the time-stamp counter in Hz, refined on every call.
///
/// The first call in a process blocks for [`MIN_WINDOW`]; later calls return at once.
/// `None` when there is no performance counter to compare it with.
pub fn frequency_hz() -> Option<f64> {
    let mut calibrator = shared()?.lock().unwrap_or_else(PoisonError::into_inner);
    let now = read_pair()?;
    calibrator.update(now)
}

/// Whether the first measurement has been made, so that [`frequency_hz`] no longer blocks.
#[cfg(test)]
pub fn is_measured() -> bool {
    SHARED.get().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    const QPC_HZ: f64 = 10_000_000.0;

    /// A reading `secs` after the origin on a counter running at `hz` from `tsc0`.
    fn at(secs: f64, tsc0: u64, hz: f64) -> Reading {
        Reading { tsc: tsc0 + (secs * hz) as u64, qpc: 1_000.0 + secs * QPC_HZ }
    }

    fn close(a: f64, b: f64) -> bool {
        (a / b - 1.0).abs() < 1e-6
    }

    #[test]
    fn needs_a_minimum_window() {
        let mut c = Calibrator::new(QPC_HZ, at(0.0, 5_000, 3.8e9));
        assert_eq!(c.update(at(0.010, 5_000, 3.8e9)), None);
        assert_eq!(c.update(at(0.049, 5_000, 3.8e9)), None);
        assert!(close(c.update(at(0.050, 5_000, 3.8e9)).unwrap(), 3.8e9));
    }

    #[test]
    fn measures_the_slope() {
        let mut c = Calibrator::new(QPC_HZ, at(0.0, 0, 2_995_200_000.0));
        assert!(close(c.update(at(0.1, 0, 2_995_200_000.0)).unwrap(), 2_995_200_000.0));
        assert!(close(c.update(at(1.0, 0, 2_995_200_000.0)).unwrap(), 2_995_200_000.0));
    }

    #[test]
    fn window_slides_and_follows_a_small_change() {
        let mut c = Calibrator::new(QPC_HZ, at(0.0, 0, 3.0e9));
        for step in 1..=5 {
            c.update(at(f64::from(step), 0, 3.0e9));
        }
        // From t = 5 s the counter runs 0.5 % faster.
        let base = at(5.0, 0, 3.0e9);
        let faster = |secs: f64| Reading {
            tsc: base.tsc + ((secs - 5.0) * 3.015e9) as u64,
            qpc: base.qpc + (secs - 5.0) * QPC_HZ,
        };
        let mut last = 0.0;
        for step in 6..=20 {
            last = c.update(faster(f64::from(step))).unwrap();
        }
        assert!(close(last, 3.015e9), "{last}");
    }

    #[test]
    fn counter_reset_restarts_the_window() {
        let mut c = Calibrator::new(QPC_HZ, at(0.0, 1_000_000_000_000, 3.0e9));
        let good = c.update(at(1.0, 1_000_000_000_000, 3.0e9)).unwrap();
        // The counter starts again from a small value, as after a resume from sleep.
        assert_eq!(c.update(at(2.0, 0, 3.0e9)), Some(good));
        assert_eq!(c.update(at(2.01, 0, 3.0e9)), Some(good));
        assert!(close(c.update(at(2.5, 0, 3.0e9)).unwrap(), 3.0e9));
    }

    #[test]
    fn large_jump_is_adopted_after_a_fresh_window() {
        let mut c = Calibrator::new(QPC_HZ, at(0.0, 0, 3.0e9));
        let good = c.update(at(1.0, 0, 3.0e9)).unwrap();
        // One second of history is missing from the performance counter's point of view.
        let jumped = |secs: f64| Reading { tsc: at(secs, 0, 3.0e9).tsc, qpc: at(secs - 0.5, 0, 3.0e9).qpc };
        assert_eq!(c.update(jumped(2.0)), Some(good));
        assert!(close(c.update(jumped(3.0)).unwrap(), 3.0e9));
    }

    #[test]
    fn backwards_performance_counter_restarts_the_window() {
        let first = at(1.0, 0, 3.0e9);
        let mut c = Calibrator::new(QPC_HZ, first);
        assert_eq!(c.update(Reading { tsc: first.tsc + 1_000, qpc: first.qpc - 5.0 }), None);
        assert_eq!(c.update(Reading { tsc: first.tsc + 2_000, qpc: first.qpc - 5.0 }), None);
        let later = Reading { tsc: first.tsc + 1_000 + 300_000_000, qpc: first.qpc - 5.0 + 0.1 * QPC_HZ };
        assert!(close(c.update(later).unwrap(), 3.0e9));
    }

    #[test]
    fn frequency_is_stable_between_calls() {
        let (Some(first), Some(second)) = (frequency_hz(), frequency_hz()) else {
            assert!(perf_frequency().is_none());
            return;
        };
        assert!(first > 0.0);
        assert!((second / first - 1.0).abs() < 0.01, "{first} then {second}");
        assert!(is_measured());
    }
}
