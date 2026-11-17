//! CPU benchmark and stress test.
//!
//! A score is the number of work units (see `workload`) completed per second,
//! scaled so that one core of a current desktop processor lands in the
//! hundreds. The single-thread phase runs one worker for a fixed wall time; the
//! multi-thread phase runs several for the same time and adds up their rates.

mod affinity;
mod references;
mod workload;

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::hw::processors;
use crate::model::CpuInfo;
pub use references::{load_references, save_reference};
use workload::Workload;

/// Version of the benchmark workload. Scores are only comparable within one version.
pub const VERSION: &str = "1.0";

/// Wall time of each benchmark phase.
const PHASE_DURATION: Duration = Duration::from_secs(3);

/// Pause between two progress events: about 15 of them per second.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(66);

/// Score earned by completing one work unit per second.
const SCORE_PER_UNIT_RATE: f64 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Single,
    Multi,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// Progress of the current phase, `0.0..=1.0`.
    Progress(Phase, f32),
    /// Final score of a phase.
    Score(Phase, f64),
    Finished,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BenchResult {
    pub single: f64,
    pub multi: f64,
    pub threads: u32,
}

impl BenchResult {
    /// Multi-thread to single-thread score ratio.
    pub fn ratio(&self) -> f64 {
        if self.single > 0.0 {
            self.multi / self.single
        } else {
            0.0
        }
    }
}

/// A stored result to compare against.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Reference {
    pub name: String,
    pub single: f64,
    pub multi: f64,
}

/// Number of threads a run on this machine may use.
///
/// `cpu` is the detected processor. It is `None` when the model at hand describes
/// another machine, as the sample system does: a run always measures the real
/// processor, so the operating system's count is taken then, as it is when
/// detection found no logical processors.
pub fn thread_limit(cpu: Option<&CpuInfo>) -> u32 {
    let os =
        thread::available_parallelism().map_or(1, |count| u32::try_from(count.get()).unwrap_or(u32::MAX));
    cpu.map_or(os, |cpu| logical_processors(cpu, os))
}

/// Logical processors of `cpu`, or `fallback` when detection could not tell.
fn logical_processors(cpu: &CpuInfo, fallback: u32) -> u32 {
    let detected = u32::try_from(cpu.logical.len()).unwrap_or(u32::MAX);
    [detected, cpu.threads.saturating_mul(cpu.sockets.max(1)), fallback]
        .into_iter()
        .find(|&count| count > 0)
        .unwrap_or(1)
}

/// A benchmark or stress run on background threads. Dropping it cancels the run and joins the threads.
pub struct Job {
    cancel: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Job {
    /// Starts the single-thread then the `threads`-thread benchmark.
    ///
    /// `on_event` is called in order from one background thread: progress
    /// events and a score for each phase, then `Finished` - or `Cancelled` as
    /// soon as the job is cancelled. It must not wait for the thread that owns
    /// the `Job`, which may itself be waiting for the run to stop.
    ///
    /// A run that cannot start all of its worker threads ends with `Cancelled`
    /// as well, without a score for that phase. Should even the background
    /// thread fail to start, `Cancelled` is reported from the calling thread
    /// before this function returns.
    pub fn bench(threads: u32, on_event: impl Fn(Event) + Send + Sync + 'static) -> Job {
        Job::bench_for(PHASE_DURATION, threads, on_event)
    }

    fn bench_for(phase: Duration, threads: u32, on_event: impl Fn(Event) + Send + Sync + 'static) -> Job {
        let cancel = Arc::new(AtomicBool::new(false));
        let on_event = Arc::new(on_event);
        let (stop, events) = (Arc::clone(&cancel), Arc::clone(&on_event));
        let coordinator = thread::Builder::new().name("bench".into()).spawn(move || {
            let result = run(phase, threads, &stop, &*events);
            events(if result.is_some() { Event::Finished } else { Event::Cancelled });
        });
        let threads = match coordinator {
            Ok(handle) => vec![handle],
            Err(_) => {
                on_event(Event::Cancelled);
                Vec::new()
            }
        };
        Job { cancel, threads }
    }

    /// Loads `threads` threads until cancelled. A thread that cannot be started
    /// is left out; `None` when not a single one could be.
    pub fn stress(threads: u32) -> Option<Job> {
        let cancel = Arc::new(AtomicBool::new(false));
        let groups: Arc<[u32]> = processors::group_sizes().into();
        let threads: Vec<_> = (0..threads.max(1))
            .filter_map(|worker| {
                let (stop, groups) = (Arc::clone(&cancel), Arc::clone(&groups));
                let load = move || {
                    affinity::assign_worker(worker, &groups);
                    work(Duration::MAX, &stop);
                };
                thread::Builder::new().name("stress".into()).spawn(load).ok()
            })
            .collect();
        (!threads.is_empty()).then(|| Job { cancel, threads })
    }

    /// Asks the run to stop. Returns at once; the threads wind down within milliseconds.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        for thread in &self.threads {
            thread.thread().unpark();
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
        let current = thread::current().id();
        for thread in self.threads.drain(..) {
            // A job dropped from inside its own event callback cannot wait for itself.
            if thread.thread().id() != current {
                let _ = thread.join();
            }
        }
    }
}

/// Runs the whole benchmark on the calling thread. Fails when a phase could
/// not run all of its worker threads.
pub fn run_blocking(threads: u32) -> io::Result<BenchResult> {
    run(PHASE_DURATION, threads, &AtomicBool::new(false), &|_| {})
        .ok_or_else(|| io::Error::other("the benchmark could not run all of its threads"))
}

/// Runs both phases, each for `phase`, reporting to `on_event`. `None` when
/// cancelled, or when a phase could not run all of its workers.
fn run(phase: Duration, threads: u32, cancel: &AtomicBool, on_event: &dyn Fn(Event)) -> Option<BenchResult> {
    let threads = threads.max(1);
    let single = run_phase(Phase::Single, worker_threads(1), phase, cancel, on_event)?;
    let multi = run_phase(Phase::Multi, worker_threads(threads), phase, cancel, on_event)?;
    Some(BenchResult { single, multi, threads })
}

/// The threads for `count` benchmark workers, not yet started.
fn worker_threads(count: u32) -> impl Iterator<Item = thread::Builder> {
    (0..count).map(|_| thread::Builder::new().name("bench worker".into()))
}

/// Runs one worker on each of `threads` for `duration` while reporting
/// progress from the calling thread, then reports and returns the score.
/// `None` when cancelled.
///
/// A score stands for all of the workers. When one of them cannot be started
/// the run is cancelled on the spot, and when one panics the phase ends
/// without a score.
fn run_phase(
    phase: Phase,
    threads: impl IntoIterator<Item = thread::Builder>,
    duration: Duration,
    cancel: &AtomicBool,
    on_event: &dyn Fn(Event),
) -> Option<f64> {
    let groups = processors::group_sizes();
    let rate = thread::scope(|scope| {
        let mut workers = Vec::new();
        for (worker, thread) in (0..).zip(threads) {
            let groups = &groups;
            let measure = move || {
                affinity::assign_worker(worker, groups);
                work(duration, cancel)
            };
            match thread.spawn_scoped(scope, measure) {
                Ok(handle) => workers.push(handle),
                Err(_) => {
                    // Cancelling stops the workers that did start, right away.
                    cancel.store(true, Ordering::SeqCst);
                    break;
                }
            }
        }

        let start = Instant::now();
        while !cancel.load(Ordering::Relaxed) {
            let elapsed = start.elapsed();
            on_event(Event::Progress(phase, progress(elapsed, duration)));
            if elapsed >= duration {
                break;
            }
            // `Job::cancel` unparks this thread, so a cancelled run does not sleep on.
            thread::park_timeout(PROGRESS_INTERVAL.min(duration - elapsed));
        }
        // Every worker is joined here, also after one that panicked: the scope
        // would raise the panic of a worker it is left to join by itself again.
        let rates: Vec<_> = workers.into_iter().map(|worker| worker.join().ok()).collect();
        rates.into_iter().sum::<Option<f64>>()
    });

    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let score = rate? * SCORE_PER_UNIT_RATE;
    on_event(Event::Score(phase, score));
    Some(score)
}

/// Completes work units until `limit` has passed or `stop` is set. Returns the rate in units per second.
fn work(limit: Duration, stop: &AtomicBool) -> f64 {
    let mut workload = Workload::new();
    let mut units = 0u64;
    let start = Instant::now();
    loop {
        workload.run_unit();
        units += 1;
        let elapsed = start.elapsed();
        if elapsed >= limit || stop.load(Ordering::Relaxed) {
            return units as f64 / elapsed.as_secs_f64();
        }
    }
}

/// Fraction of `duration` that has `elapsed`, capped at 1.
fn progress(elapsed: Duration, duration: Duration) -> f32 {
    if duration.is_zero() {
        1.0
    } else {
        (elapsed.as_secs_f32() / duration.as_secs_f32()).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{self, Receiver};

    const SHORT: Duration = Duration::from_millis(50);
    /// Generous limit for anything that should happen "promptly", even on a loaded machine.
    const PATIENCE: Duration = Duration::from_secs(20);

    fn start(phase: Duration, threads: u32) -> (Job, Receiver<Event>) {
        let (sender, receiver) = mpsc::channel();
        let job = Job::bench_for(phase, threads, move |event| {
            let _ = sender.send(event);
        });
        (job, receiver)
    }

    /// Collects events up to and including the one that ends the run.
    fn collect(receiver: &Receiver<Event>) -> Vec<Event> {
        let mut events = Vec::new();
        loop {
            let event = receiver.recv_timeout(PATIENCE).expect("the job stopped reporting");
            events.push(event);
            if matches!(event, Event::Finished | Event::Cancelled) {
                return events;
            }
        }
    }

    /// Whether any thread of the job is still alive.
    fn is_running(job: &Job) -> bool {
        job.threads.iter().any(|thread| !thread.is_finished())
    }

    fn wait_until_stopped(job: &Job) {
        let deadline = Instant::now() + PATIENCE;
        while is_running(job) {
            assert!(Instant::now() < deadline, "the job kept running");
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn events_arrive_in_order_with_monotonic_progress() {
        let (job, receiver) = start(SHORT, 2);
        let events = collect(&receiver);
        wait_until_stopped(&job);
        assert_eq!(events.last(), Some(&Event::Finished));

        // Single-thread phase, then multi-thread phase, each closed by its score.
        let mut rest = events.as_slice();
        for phase in [Phase::Single, Phase::Multi] {
            let count =
                rest.iter().take_while(|event| matches!(event, Event::Progress(p, _) if *p == phase)).count();
            let (steps, tail) = rest.split_at(count);
            let fractions: Vec<f32> = steps
                .iter()
                .map(|event| match event {
                    Event::Progress(_, fraction) => *fraction,
                    other => panic!("unexpected {other:?}"),
                })
                .collect();
            assert!(fractions.len() >= 2, "{phase:?}: {fractions:?}");
            assert!(fractions.windows(2).all(|pair| pair[0] <= pair[1]), "{phase:?}: {fractions:?}");
            assert!((0.0..0.5).contains(&fractions[0]), "{phase:?}: {fractions:?}");
            assert_eq!(fractions.last(), Some(&1.0), "{phase:?}");
            assert!(
                matches!(tail.first(), Some(Event::Score(p, score)) if *p == phase && *score > 0.0),
                "{tail:?}"
            );
            rest = &tail[1..];
        }
        assert_eq!(rest, [Event::Finished]);
        assert!(receiver.try_recv().is_err(), "nothing may follow `Finished`");
    }

    #[test]
    fn cancel_stops_promptly_and_reports_it() {
        let (job, receiver) = start(Duration::from_secs(600), 2);
        assert!(is_running(&job));
        assert!(matches!(receiver.recv_timeout(PATIENCE), Ok(Event::Progress(Phase::Single, _))));

        let cancelled_at = Instant::now();
        job.cancel();
        let events = collect(&receiver);
        assert!(cancelled_at.elapsed() < PATIENCE);
        assert_eq!(events.last(), Some(&Event::Cancelled));
        assert!(events
            .iter()
            .all(|event| matches!(event, Event::Progress(Phase::Single, _) | Event::Cancelled)));
        wait_until_stopped(&job);

        // Cancelling again, or dropping the stopped job, changes nothing.
        job.cancel();
        drop(job);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn dropping_a_job_cancels_and_joins_it() {
        let (job, receiver) = start(Duration::from_secs(600), 1);
        let dropped_at = Instant::now();
        drop(job);
        assert!(dropped_at.elapsed() < PATIENCE);
        // The sender lives in the joined thread, so the channel is closed and complete by now.
        let events: Vec<Event> = receiver.try_iter().collect();
        assert_eq!(events.last(), Some(&Event::Cancelled));
        assert!(matches!(receiver.try_recv(), Err(mpsc::TryRecvError::Disconnected)));
    }

    #[test]
    fn job_may_be_dropped_from_its_own_callback() {
        let (sender, receiver) = mpsc::channel();
        let slot = Arc::new(std::sync::Mutex::new(None::<Job>));
        let held = Arc::clone(&slot);
        let job = Job::bench_for(Duration::from_secs(600), 1, move |event| {
            if let Ok(mut held) = held.lock() {
                drop(held.take());
            }
            let _ = sender.send(event);
        });
        *slot.lock().unwrap() = Some(job);
        let events = collect(&receiver);
        assert_eq!(events.last(), Some(&Event::Cancelled));
    }

    #[test]
    fn stress_runs_until_cancelled() {
        let job = Job::stress(2).expect("the stress threads start");
        thread::sleep(Duration::from_millis(20));
        assert_eq!(job.threads.iter().filter(|thread| !thread.is_finished()).count(), 2);
        job.cancel();
        wait_until_stopped(&job);
    }

    #[test]
    fn blocking_run_scores_both_phases() {
        let idle = AtomicBool::new(false);
        let result = run(SHORT, 3, &idle, &|_| {}).expect("not cancelled");
        assert!(result.single > 0.0 && result.single.is_finite());
        assert!(result.multi > 0.0 && result.multi.is_finite());
        assert_eq!(result.threads, 3);

        // Zero threads still means one worker.
        assert_eq!(run(SHORT, 0, &idle, &|_| {}).map(|r| r.threads), Some(1));
        // A run that is already cancelled yields nothing.
        assert_eq!(run(SHORT, 1, &AtomicBool::new(true), &|_| {}), None);
    }

    #[test]
    fn phase_without_all_of_its_workers_has_no_score() {
        // No process can reserve half of its address space for one more stack.
        let unstartable = || thread::Builder::new().stack_size(usize::MAX / 2);
        let cancel = AtomicBool::new(false);
        let events = std::cell::RefCell::new(Vec::new());
        let record = |event| events.borrow_mut().push(event);

        // The first worker starts and would stay busy for the whole phase.
        let started_at = Instant::now();
        let threads = [thread::Builder::new(), unstartable(), thread::Builder::new()];
        assert_eq!(run_phase(Phase::Multi, threads, Duration::from_secs(600), &cancel, &record), None);
        assert!(started_at.elapsed() < PATIENCE);
        assert!(cancel.load(Ordering::SeqCst), "the run counts as cancelled");

        // Nor is there a score of zero when no worker starts at all.
        let cancel = AtomicBool::new(false);
        assert_eq!(run_phase(Phase::Single, [unstartable()], SHORT, &cancel, &record), None);
        assert!(events.borrow().is_empty(), "{events:?}");
    }

    #[test]
    fn logical_processor_count_falls_back() {
        let mut cpu = CpuInfo::default();
        assert_eq!(logical_processors(&cpu, 12), 12);
        assert_eq!(logical_processors(&cpu, 0), 1);
        cpu.threads = 8;
        cpu.sockets = 2;
        assert_eq!(logical_processors(&cpu, 12), 16);
        cpu.logical = vec![Default::default(); 96];
        assert_eq!(logical_processors(&cpu, 12), 96);
    }

    #[test]
    fn thread_limit_is_that_of_the_machine_the_run_measures() {
        let os = thread::available_parallelism().map_or(1, |count| count.get() as u32);
        let sample = crate::demo::system().cpu;
        assert_eq!(thread_limit(Some(&sample)), 16);
        // No processor of this machine to go by, or one whose threads could not be counted.
        assert_eq!(thread_limit(None), os);
        assert_eq!(thread_limit(Some(&CpuInfo::default())), os);
    }

    #[test]
    fn ratio_compares_multi_to_single() {
        assert_eq!(BenchResult { single: 100.0, multi: 350.0, threads: 4 }.ratio(), 3.5);
        assert_eq!(BenchResult { single: 0.0, multi: 350.0, threads: 4 }.ratio(), 0.0);
        assert_eq!(BenchResult::default().ratio(), 0.0);
    }

    #[test]
    fn progress_is_a_capped_fraction() {
        let second = Duration::from_secs(1);
        assert_eq!(progress(Duration::ZERO, second), 0.0);
        assert_eq!(progress(Duration::from_millis(250), second), 0.25);
        assert_eq!(progress(Duration::from_secs(5), second), 1.0);
        assert_eq!(progress(Duration::ZERO, Duration::ZERO), 1.0);
    }
}
