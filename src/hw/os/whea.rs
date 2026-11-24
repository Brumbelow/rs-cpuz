//! Hardware errors on record with the Windows Hardware Error Architecture (WHEA).
//!
//! Windows writes an event to its System log for every hardware error the
//! platform reports: a warning for one that was corrected, an error for the
//! others. That log is readable without elevation, so the count comes from a
//! plain event query.

use windows::core::{w, Owned, PCWSTR};
use windows::Win32::Foundation::ERROR_NO_MORE_ITEMS;
use windows::Win32::System::EventLog::{EvtNext, EvtQuery, EvtQueryChannelPath, EVT_HANDLE};

const CHANNEL: PCWSTR = w!("System");
/// The events of the WHEA logger that report an error: level 1 is critical, 2 error, 3 warning.
const ERROR_EVENTS: PCWSTR =
    w!("*[System[Provider[@Name='Microsoft-Windows-WHEA-Logger'] and (Level=1 or Level=2 or Level=3)]]");
/// Events fetched per call.
const BATCH: usize = 64;
/// Milliseconds the event log service gets to hand over one batch.
const TIMEOUT_MS: u32 = 2_000;
/// Longest time the whole count may take; a log full of corrected errors can hold
/// many thousands of events.
const TOTAL_TIME: std::time::Duration = std::time::Duration::from_secs(5);

/// What one fetch from an event query came back with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fetch {
    Events(u32),
    End,
    Failed,
}

/// Counts the hardware errors in the System event log. `None` when the log cannot be read.
pub fn error_count() -> Option<u32> {
    // SAFETY: the channel and the query are NUL-terminated literals, and the handle
    // returned is owned by `query`, which closes it exactly once.
    let query = unsafe {
        EvtQuery(EVT_HANDLE::default(), CHANNEL, ERROR_EVENTS, EvtQueryChannelPath.0)
            .map(|handle| Owned::new(handle))
    }
    .ok()?;
    let started = std::time::Instant::now();
    count_until(|| fetch(*query), || started.elapsed() > TOTAL_TIME)
}

/// Adds up the batches `next` returns until the query runs dry. `None` if a fetch fails.
#[cfg(test)]
fn count(next: impl FnMut() -> Fetch) -> Option<u32> {
    count_until(next, || false)
}

/// Adds up the batches `next` returns until the query runs dry. `None` if a fetch fails
/// or `out_of_time` says so before the end: a partial count is not the count.
fn count_until(mut next: impl FnMut() -> Fetch, mut out_of_time: impl FnMut() -> bool) -> Option<u32> {
    let mut total = 0u32;
    loop {
        if out_of_time() {
            return None;
        }
        match next() {
            // A batch without events would never end the loop on its own.
            Fetch::End | Fetch::Events(0) => return Some(total),
            Fetch::Events(events) => total = total.saturating_add(events),
            Fetch::Failed => return None,
        }
    }
}

/// Takes the next batch of events off `query` and releases them again.
fn fetch(query: EVT_HANDLE) -> Fetch {
    let mut events = [0isize; BATCH];
    let mut returned = 0u32;
    // SAFETY: `query` is an open query handle, and the call writes at most `events.len()`
    // handles and the number of them it wrote.
    let fetched = unsafe { EvtNext(query, &mut events, TIMEOUT_MS, 0, &mut returned) };
    match fetched {
        Ok(()) => {
            for &event in events.iter().take(returned as usize) {
                // SAFETY: the call above handed this event handle over; it is closed once, here.
                drop(unsafe { Owned::new(EVT_HANDLE(event)) });
            }
            Fetch::Events(returned)
        }
        Err(error) if error.code() == ERROR_NO_MORE_ITEMS.to_hresult() => Fetch::End,
        Err(_) => Fetch::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Replays `script`, then reports the end of the query.
    fn replay(script: &[Fetch]) -> impl FnMut() -> Fetch + '_ {
        let mut steps = script.iter().copied();
        move || steps.next().unwrap_or(Fetch::End)
    }

    #[test]
    fn adds_up_the_batches() {
        assert_eq!(count(replay(&[])), Some(0));
        assert_eq!(count(replay(&[Fetch::Events(64), Fetch::Events(64), Fetch::Events(3)])), Some(131));
        assert_eq!(count(replay(&[Fetch::Events(7), Fetch::End, Fetch::Events(9)])), Some(7));
    }

    #[test]
    fn an_empty_batch_ends_the_count() {
        assert_eq!(count(replay(&[Fetch::Events(5), Fetch::Events(0), Fetch::Events(5)])), Some(5));
    }

    #[test]
    fn a_log_that_takes_too_long_leaves_the_count_unknown() {
        // An endless stream of full batches, read until the time is up.
        let mut checks = 0;
        let out_of_time = || {
            checks += 1;
            checks > 100
        };
        assert_eq!(count_until(|| Fetch::Events(64), out_of_time), None);
        assert_eq!(count_until(replay(&[Fetch::Events(3)]), || false), Some(3));
    }

    #[test]
    fn a_failed_fetch_leaves_the_count_unknown() {
        assert_eq!(count(replay(&[Fetch::Failed])), None);
        assert_eq!(count(replay(&[Fetch::Events(12), Fetch::Failed])), None);
    }

    #[test]
    fn huge_logs_saturate() {
        assert_eq!(count(replay(&[Fetch::Events(u32::MAX), Fetch::Events(1)])), Some(u32::MAX));
    }

    #[test]
    fn a_closed_query_fails_instead_of_ending() {
        assert_eq!(fetch(EVT_HANDLE::default()), Fetch::Failed);
    }
}
