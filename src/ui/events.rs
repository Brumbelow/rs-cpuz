//! Messages from the worker threads to the window thread.
//!
//! Worker threads never touch UI state: they queue an [`Event`] and post one
//! wake-up message to whichever window currently listens. The window drains
//! the queue from its message handler.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Foundation::HWND;

use crate::bench;
use crate::config::Config;
use crate::demo;
use crate::hw::sensors::SensorMonitor;
use crate::hw::{self, clocks::ClockMonitor, Stage};
use crate::hw::{pawnio, ring0};
use crate::model::{ClockSnapshot, Sensor, SystemInfo};

use super::win;

/// Time between two clock samples.
const CLOCK_INTERVAL: Duration = Duration::from_secs(1);

pub enum Event {
    /// Detection entered a new stage.
    Stage(Stage),
    /// Detection finished; comes with the first clock sample.
    Detected(Box<SystemInfo>, ClockSnapshot),
    /// A fresh clock sample.
    Clocks(ClockSnapshot),
    /// A fresh reading of the sensors, sent while the window asks for them.
    Sensors(Vec<Sensor>),
    /// An event of the benchmark run with the given serial number.
    Bench(u32, bench::Event),
    /// The detection and clock thread ended, normally or not.
    WorkerExited,
}

/// The sending half: cheap to clone and usable from any thread.
#[derive(Clone)]
pub struct Mailbox {
    queue: Sender<Event>,
    /// Window to wake, stored as an integer so it can cross threads; 0 for none.
    target: Arc<AtomicIsize>,
    /// A wake-up message is already on its way.
    pending: Arc<AtomicBool>,
    /// The window shows the sensors, so the worker reads them along with the clocks.
    sensors: Arc<AtomicBool>,
}

/// The receiving half, owned by the window thread.
pub struct Inbox {
    queue: Receiver<Event>,
    pending: Arc<AtomicBool>,
}

/// Creates a connected mailbox and inbox with no window listening yet.
pub fn channel() -> (Mailbox, Inbox) {
    let (sender, receiver) = mpsc::channel();
    let pending = Arc::new(AtomicBool::new(false));
    let mailbox = Mailbox {
        queue: sender,
        target: Arc::new(AtomicIsize::new(0)),
        pending: pending.clone(),
        sensors: Arc::new(AtomicBool::new(false)),
    };
    (mailbox, Inbox { queue: receiver, pending })
}

impl Mailbox {
    /// Queues an event and wakes the listening window unless a wake-up is already pending.
    pub fn post(&self, event: Event) {
        if self.queue.send(event).is_ok() {
            self.wake();
        }
    }

    fn wake(&self) {
        let target = self.target.load(Ordering::Acquire);
        if target != 0 && !self.pending.swap(true, Ordering::AcqRel) {
            win::post(win::hwnd_from(target), win::WM_UI_EVENT, 0, 0);
        }
    }

    /// Asks the worker to read the sensors with every clock sample, or to stop.
    pub fn want_sensors(&self, wanted: bool) {
        self.sensors.store(wanted, Ordering::Release);
    }

    /// Whether the window wants sensor readings.
    pub fn sensors_wanted(&self) -> bool {
        self.sensors.load(Ordering::Acquire)
    }

    /// Makes `window` the one that is woken for new events; `None` stops the wake-ups.
    /// Events queued in the meantime are announced to the new window right away.
    pub fn listen(&self, window: Option<HWND>) {
        self.target.store(window.map_or(0, |hwnd| hwnd.0 as isize), Ordering::Release);
        self.pending.store(false, Ordering::Release);
        self.wake();
    }
}

impl Inbox {
    /// Takes every queued event. Call this for each wake-up message.
    pub fn drain(&self) -> Vec<Event> {
        self.pending.store(false, Ordering::Release);
        self.queue.try_iter().collect()
    }

    /// Blocks until the next event; `None` once every mailbox is gone.
    pub fn wait(&self) -> Option<Event> {
        self.queue.recv().ok()
    }
}

/// Longest wait for the worker when the window closes; a thread stuck in a driver call
/// is left behind, and ends with the process.
const STOP_WAIT: Duration = Duration::from_secs(2);

/// The thread that detects the hardware and then samples the clocks until stopped.
pub struct Worker {
    stop: Sender<()>,
    thread: JoinHandle<()>,
    /// Disconnects when the thread ends.
    finished: Receiver<()>,
}

/// Posts [`Event::WorkerExited`] when the worker thread ends, even by a panic in a backend.
struct ExitNotice(Mailbox);

impl Drop for ExitNotice {
    fn drop(&mut self) {
        self.0.post(Event::WorkerExited);
    }
}

impl Worker {
    /// Starts the worker; `None` when the system refuses to create a thread.
    pub fn spawn(cfg: Config, demo: bool, mailbox: Mailbox) -> Option<Worker> {
        let (stop, stopped) = mpsc::channel();
        let (done, finished) = mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("detect-and-clocks".into())
            .spawn(move || {
                let _done = done;
                work(&cfg, demo, &mailbox, &stopped);
            })
            .ok()?;
        Some(Worker { stop, thread, finished })
    }

    /// Stops the clock sampling and waits a little while for the thread to end.
    pub fn stop(self) {
        drop(self.stop);
        wait_for(self.thread, &self.finished, STOP_WAIT);
    }

    /// Lets the thread end on its own; used when detection is still running at exit.
    pub fn abandon(self) {
        drop(self.stop);
    }
}

/// Joins `thread` if it ends within `wait`, `finished` disconnecting when it does.
/// Returns whether it ended; one that did not is left to the end of the process.
fn wait_for(thread: JoinHandle<()>, finished: &Receiver<()>, wait: Duration) -> bool {
    match finished.recv_timeout(wait) {
        Err(RecvTimeoutError::Timeout) => false,
        _ => {
            let _ = thread.join();
            true
        }
    }
}

fn work(cfg: &Config, demo: bool, mailbox: &Mailbox, stopped: &Receiver<()>) {
    let _notice = ExitNotice(mailbox.clone());
    let (sys, mut monitor) = if demo {
        (demo::system(), None)
    } else {
        let sys = hw::detect(cfg, &mut |stage| mailbox.post(Event::Stage(stage)));
        let monitor = ClockMonitor::new(&sys.cpu, sys.smbios.as_ref());
        (sys, Some(monitor))
    };
    let mut sample = move || monitor.as_mut().map_or_else(demo::clocks, ClockMonitor::sample);
    // The sensors are opened when the window first asks for them; the sample system has none.
    let drives = (!demo).then(|| sys.drives.clone());
    let processor = (!demo).then(|| sys.cpu.clone());
    let mut sensors: Option<SensorMonitor> = None;
    // The registers only the driver reads, when it may be used.
    let mut registers: Option<Option<ring0::Live>> = None;
    mailbox.post(Event::Detected(Box::new(sys), sample()));
    if demo {
        // Detection and the clock monitor measure the time-stamp counter as they go. A demo
        // run has neither, so the measurement starts here instead of on the window thread
        // when the Timers tool asks for it.
        let _ = hw::timers::tsc::frequency_hz();
    }
    while stopped.recv_timeout(CLOCK_INTERVAL) == Err(RecvTimeoutError::Timeout) {
        mailbox.post(Event::Clocks(sample()));
        if let (Some(drives), true) = (&drives, mailbox.sensors_wanted()) {
            let monitor = sensors.get_or_insert_with(|| SensorMonitor::new(drives, cfg.display));
            let mut readings = monitor.sample();
            let live = registers.get_or_insert_with(|| {
                processor.as_ref().filter(|_| pawnio::enabled()).and_then(ring0::Live::open)
            });
            if let Some(live) = live {
                readings.extend(live.sample());
            }
            mailbox.post(Event::Sensors(readings));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_queue_without_a_listener() {
        let (mailbox, inbox) = channel();
        assert!(inbox.drain().is_empty());
        mailbox.post(Event::Stage(Stage::Memory));
        mailbox.post(Event::Bench(3, bench::Event::Finished));
        let events = inbox.drain();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], Event::Stage(Stage::Memory)));
        assert!(matches!(events[1], Event::Bench(3, bench::Event::Finished)));
        assert!(inbox.drain().is_empty());
    }

    #[test]
    fn the_window_tells_the_worker_when_it_wants_sensors() {
        let (mailbox, _inbox) = channel();
        let worker_side = mailbox.clone();
        assert!(!worker_side.sensors_wanted());
        mailbox.want_sensors(true);
        assert!(worker_side.sensors_wanted());
        mailbox.want_sensors(false);
        assert!(!worker_side.sensors_wanted());
    }

    #[test]
    fn stopping_does_not_wait_for_a_stuck_thread() {
        // A thread stuck in a driver call: it holds its end of the channel for seconds.
        let (done, finished) = mpsc::channel::<()>();
        let thread = std::thread::spawn(move || {
            let _done = done;
            std::thread::sleep(Duration::from_secs(3));
        });
        let started = std::time::Instant::now();
        assert!(!wait_for(thread, &finished, Duration::from_millis(100)));
        assert!(started.elapsed() < Duration::from_secs(2));
        // A thread that ends in time is joined.
        let (done, finished) = mpsc::channel::<()>();
        let thread = std::thread::spawn(move || drop(done));
        assert!(wait_for(thread, &finished, Duration::from_secs(5)));
    }

    #[test]
    fn demo_worker_delivers_the_sample_system_and_stops() {
        let (mailbox, inbox) = channel();
        let worker = Worker::spawn(Config::default(), true, mailbox).expect("worker thread");
        let detected = loop {
            match inbox.queue.recv_timeout(Duration::from_secs(10)) {
                Ok(Event::Detected(sys, clocks)) => break Some((sys, clocks)),
                Ok(_) => continue,
                Err(_) => break None,
            }
        };
        let (sys, clocks) = detected.expect("the worker reports the detected system");
        assert_eq!(sys.cpu.name, demo::system().cpu.name);
        assert_eq!(clocks, demo::clocks());
        worker.stop();
        assert!(inbox.drain().iter().any(|event| matches!(event, Event::WorkerExited)));
        // The Timers tool must not be the first to measure the time-stamp counter.
        assert!(hw::timers::tsc::is_measured());
    }
}
