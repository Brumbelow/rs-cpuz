//! Per-processor performance from the `% Processor Performance` counter.
//!
//! The counter is the kernel's own bookkeeping of the frequency a processor
//! delivered while it was not idle, relative to its nominal frequency; it is
//! what Task Manager turns into "Speed" and it exceeds 100 % under boost. It is
//! an average-with-base counter: a reading is the change of the raw value
//! divided by the change of its base between two collections.
//!
//! The raw values are used rather than PDH's formatted ones because both
//! halves need care. The base is a 32-bit quantity that wraps around, and the
//! two are not captured at exactly the same instant, which shows in the ratio
//! unless the base has advanced far enough. A processor that has not got there
//! yet keeps its baseline, so its next reading simply covers a longer time.

use std::collections::HashMap;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetRawCounterArrayW, PdhOpenQueryW,
    PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_MORE_DATA, PDH_RAW_COUNTER_ITEM_W,
};

/// English counter path; `PdhAddEnglishCounterW` resolves it on any display language.
const COUNTER_PATH: PCWSTR = w!(r"\Processor Information(*)\% Processor Performance");

/// Readings outside this range (percent of nominal) are measurement glitches.
const PLAUSIBLE_PERCENT: std::ops::RangeInclusive<f64> = 1.0..=1000.0;

/// The base of the counter is a 32-bit value.
const BASE_WRAP: i64 = 1 << 32;

/// Advance of the base a reading must cover. The base counts busy time, about
/// 625 000 per second, and value and base were seen up to some 65 counts
/// apart, so this is 4 ms of work and keeps that error below 3 %.
const MIN_BASE_DELTA: i64 = 2_500;

/// Longest instance name that is looked at, in UTF-16 units.
const MAX_NAME: usize = 32;

/// A counter instance, named `first,second` (see `topology::Naming`).
pub type Instance = (u32, u32);

/// Raw counter of one instance at one collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RawSample {
    value: i64,
    base: i64,
}

/// An open PDH query on the performance counter of every logical processor.
pub struct PerfQuery {
    query: isize,
    counter: isize,
    /// Receives the counter array; 64-bit words keep the items aligned.
    buffer: Vec<u64>,
    /// Raw counters each instance's next reading is measured from.
    baseline: HashMap<Instance, RawSample>,
}

impl PerfQuery {
    /// Opens the query and takes the baseline collection. `None` when the
    /// performance counters are unavailable or disabled.
    pub fn open() -> Option<PerfQuery> {
        let mut query = 0isize;
        // SAFETY: `query` is a live local that receives the handle; a null source means live data.
        if unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) } != ERROR_SUCCESS.0 {
            return None;
        }
        // From here on dropping `this` closes the query.
        let mut this = PerfQuery { query, counter: 0, buffer: Vec::new(), baseline: HashMap::new() };
        // SAFETY: the query handle is open, the path is a NUL-terminated literal and
        // `counter` is a live field that receives the handle.
        if unsafe { PdhAddEnglishCounterW(this.query, COUNTER_PATH, 0, &mut this.counter) } != ERROR_SUCCESS.0
        {
            return None;
        }
        this.baseline = this.read()?;
        Some(this)
    }

    /// Collects the counters and returns the performance, in percent of
    /// nominal, of every instance that has done enough since its last reading.
    /// `None` when the collection fails.
    pub fn collect(&mut self) -> Option<Vec<(Instance, f64)>> {
        let current = self.read()?;
        Some(advance(&mut self.baseline, &current))
    }

    /// Every instance of the counter, busy or not.
    pub fn instances(&self) -> Vec<Instance> {
        self.baseline.keys().copied().collect()
    }

    fn read(&mut self) -> Option<HashMap<Instance, RawSample>> {
        // SAFETY: the query handle stays open for the lifetime of `self`.
        if unsafe { PdhCollectQueryData(self.query) } != ERROR_SUCCESS.0 {
            return None;
        }
        let count = self.fetch()?;
        let bytes = self.buffer.len() * 8;
        if count.checked_mul(std::mem::size_of::<PDH_RAW_COUNTER_ITEM_W>())? > bytes {
            return None;
        }
        // SAFETY: PDH wrote `count` items at the start of the buffer (checked to fit above);
        // the buffer is 8-byte aligned and outlives the slice.
        let items = unsafe {
            std::slice::from_raw_parts(self.buffer.as_ptr().cast::<PDH_RAW_COUNTER_ITEM_W>(), count)
        };
        let samples = items
            .iter()
            .filter(|item| matches!(item.RawValue.CStatus, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA))
            .filter_map(|item| {
                let instance = parse_instance(&self.name(item.szName.0)?)?;
                Some((
                    instance,
                    RawSample { value: item.RawValue.FirstValue, base: item.RawValue.SecondValue },
                ))
            })
            .collect();
        Some(samples)
    }

    /// Fills the buffer with the raw counter array and returns the item count.
    fn fetch(&mut self) -> Option<usize> {
        // Usually the buffer of the previous call fits. If it does not, the size PDH
        // reports is only dependable for a call without a buffer, hence up to three rounds.
        for _ in 0..3 {
            let mut size = u32::try_from(self.buffer.len() * 8).ok()?;
            let mut count = 0u32;
            let items = (!self.buffer.is_empty()).then(|| self.buffer.as_mut_ptr().cast());
            // SAFETY: `items` is null or points to `size` writable bytes; the counter
            // handle belongs to the open query.
            let status = unsafe { PdhGetRawCounterArrayW(self.counter, &mut size, &mut count, items) };
            match status {
                s if s == ERROR_SUCCESS.0 => return Some(count as usize),
                PDH_MORE_DATA if self.buffer.is_empty() => self.buffer.resize((size as usize).div_ceil(8), 0),
                PDH_MORE_DATA => self.buffer.clear(),
                _ => return None,
            }
        }
        None
    }

    /// Copies an instance name, which PDH stores inside the buffer and points
    /// to. The pointer is only used to find the place; `None` when it leads elsewhere.
    fn name(&self, pointer: *const u16) -> Option<Vec<u16>> {
        let offset = (pointer as usize).checked_sub(self.buffer.as_ptr() as usize)?;
        if offset % 2 != 0 {
            return None;
        }
        // Every 64-bit word of the buffer holds four UTF-16 units in memory order.
        let unit = |index: usize| {
            let bytes = self.buffer.get(index / 4)?.to_ne_bytes();
            Some(u16::from_ne_bytes([bytes[index % 4 * 2], bytes[index % 4 * 2 + 1]]))
        };
        let first = offset / 2;
        Some((first..first + MAX_NAME).map_while(unit).take_while(|&unit| unit != 0).collect())
    }
}

impl Drop for PerfQuery {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by `open` and is closed exactly once; this also releases the counter.
        unsafe { PdhCloseQuery(self.query) };
    }
}

/// Parses an instance name such as `0,15`. The `_Total` instances have no number and yield `None`.
fn parse_instance(name: &[u16]) -> Option<Instance> {
    let mut parts = name.split(|&unit| unit == u16::from(b','));
    let (first, second) = (number(parts.next()?)?, number(parts.next()?)?);
    parts.next().is_none().then_some((first, second))
}

fn number(digits: &[u16]) -> Option<u32> {
    if digits.is_empty() {
        return None;
    }
    digits.iter().try_fold(0u32, |value, &unit| {
        let digit = char::from_u32(u32::from(unit))?.to_digit(10)?;
        value.checked_mul(10)?.checked_add(digit)
    })
}

/// What a collection says about an instance, relative to its baseline.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Progress {
    /// Too little work since the baseline for a dependable ratio; keep waiting.
    Pending,
    /// Performance since the baseline, in percent of nominal.
    Reading(f64),
    /// The counters do not continue the baseline; start over from this collection.
    Restart,
}

/// Compares a collection with the baselines, returning the readings that are
/// due and moving the baseline of every instance that gave one, is new or was reset.
fn advance(
    baseline: &mut HashMap<Instance, RawSample>,
    current: &HashMap<Instance, RawSample>,
) -> Vec<(Instance, f64)> {
    let mut readings = Vec::new();
    for (&instance, &now) in current {
        let progress = baseline.get(&instance).map_or(Progress::Restart, |&then| progress(then, now));
        if let Progress::Reading(percent) = progress {
            readings.push((instance, percent));
        }
        if progress != Progress::Pending {
            baseline.insert(instance, now);
        }
    }
    readings
}

/// Change of the value over change of the base.
fn progress(baseline: RawSample, current: RawSample) -> Progress {
    let value = current.value.checked_sub(baseline.value).filter(|delta| *delta >= 0);
    let (Some(value), Some(base)) = (value, base_delta(baseline.base, current.base)) else {
        return Progress::Restart;
    };
    if base < MIN_BASE_DELTA {
        return Progress::Pending;
    }
    let percent = value as f64 / base as f64;
    if PLAUSIBLE_PERCENT.contains(&percent) {
        Progress::Reading(percent)
    } else {
        Progress::Restart
    }
}

/// Advance of the 32-bit base, across at most one wrap-around. `None` when it ran backwards.
fn base_delta(baseline: i64, current: i64) -> Option<i64> {
    let in_range = |base: i64| (0..BASE_WRAP).contains(&base);
    let delta = current.checked_sub(baseline)?;
    let delta = if delta < 0 && in_range(baseline) && in_range(current) { delta + BASE_WRAP } else { delta };
    (delta >= 0).then_some(delta)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    fn raw(value: i64, base: i64) -> RawSample {
        RawSample { value, base }
    }

    fn reading(progress: Progress) -> f64 {
        match progress {
            Progress::Reading(percent) => percent,
            other => panic!("expected a reading, got {other:?}"),
        }
    }

    #[test]
    fn parses_instance_names() {
        assert_eq!(parse_instance(&wide("0,0")), Some((0, 0)));
        assert_eq!(parse_instance(&wide("0,15")), Some((0, 15)));
        assert_eq!(parse_instance(&wide("3,127")), Some((3, 127)));
    }

    #[test]
    fn rejects_totals_and_garbage() {
        let names = [
            "_Total",
            "0,_Total",
            "",
            ",",
            "0,",
            ",1",
            "0,1,2",
            "0;1",
            "a,b",
            "-1,0",
            "0, 1",
            "99999999999,0",
        ];
        for name in names {
            assert_eq!(parse_instance(&wide(name)), None, "{name}");
        }
    }

    #[test]
    fn reading_is_value_over_base() {
        let percent = reading(progress(raw(1_000_000_000, 2_665_917_395), raw(1_001_040_446, 2_665_925_990)));
        assert!((percent - 121.052).abs() < 0.001);
    }

    #[test]
    fn base_wraps_at_32_bits() {
        let percent = reading(progress(raw(1_000_000, BASE_WRAP - 6_000), raw(2_400_000, 4_000)));
        assert!((percent - 140.0).abs() < 1e-9);
    }

    #[test]
    fn wide_bases_do_not_wrap() {
        assert_eq!(progress(raw(0, BASE_WRAP + 5_000), raw(1_000_000, BASE_WRAP - 5_000)), Progress::Restart);
        let percent = reading(progress(raw(0, BASE_WRAP - 5_000), raw(1_000_000, BASE_WRAP + 5_000)));
        assert!((percent - 100.0).abs() < 1e-9);
    }

    #[test]
    fn little_work_keeps_waiting() {
        assert_eq!(progress(raw(500, 10), raw(500, 10)), Progress::Pending);
        assert_eq!(progress(raw(500, 10), raw(900, 10)), Progress::Pending);
        assert_eq!(progress(raw(0, 0), raw(257_000, MIN_BASE_DELTA - 1)), Progress::Pending);
        assert_eq!(progress(raw(0, 0), raw(335_000, MIN_BASE_DELTA)), Progress::Reading(134.0));
    }

    #[test]
    fn counters_running_backwards_restart() {
        assert_eq!(progress(raw(500, 10), raw(400, 9_000)), Progress::Restart);
        assert_eq!(progress(raw(i64::MIN, 10), raw(i64::MAX, 9_000)), Progress::Restart);
        assert_eq!(progress(raw(0, i64::MAX), raw(1_000_000, i64::MIN)), Progress::Restart);
    }

    #[test]
    fn implausible_ratios_restart() {
        assert_eq!(progress(raw(0, 0), raw(50, 10_000)), Progress::Restart);
        assert_eq!(progress(raw(0, 0), raw(50_000_000, 10_000)), Progress::Restart);
        assert_eq!(progress(raw(0, 0), raw(950_000, 10_000)), Progress::Reading(95.0));
    }

    #[test]
    fn baselines_move_only_when_a_reading_is_taken() {
        let mut baseline =
            HashMap::from([((0, 0), raw(0, 0)), ((0, 1), raw(0, 0)), ((0, 2), raw(9_000, 500))]);

        // Processor 0 is busy, 1 nearly idle, 2 had its counters reset and 3 is new.
        let first = HashMap::from([
            ((0, 0), raw(1_200_000, 10_000)),
            ((0, 1), raw(100_000, 1_000)),
            ((0, 2), raw(100, 20)),
            ((0, 3), raw(7_000, 70)),
        ]);
        assert_eq!(advance(&mut baseline, &first), vec![((0, 0), 120.0)]);
        assert_eq!(baseline[&(0, 0)], raw(1_200_000, 10_000));
        assert_eq!(baseline[&(0, 1)], raw(0, 0));
        assert_eq!(baseline[&(0, 2)], raw(100, 20));
        assert_eq!(baseline[&(0, 3)], raw(7_000, 70));

        // The idle processor's reading covers both intervals once it has done enough.
        let second = HashMap::from([((0, 0), raw(1_300_000, 11_000)), ((0, 1), raw(390_000, 3_000))]);
        assert_eq!(advance(&mut baseline, &second), vec![((0, 1), 130.0)]);
        assert_eq!(baseline[&(0, 0)], raw(1_200_000, 10_000));
        assert_eq!(baseline[&(0, 1)], raw(390_000, 3_000));
        assert_eq!(baseline.len(), 4);
    }
}
