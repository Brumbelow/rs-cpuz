//! Processor frequencies as the power manager reports them.
//!
//! `CallNtPowerInformation(ProcessorInformation)` is the fallback when the
//! performance counters are unavailable. On current hardware its "current"
//! frequency rarely moves and never exceeds the nominal one, so it is no
//! substitute for them.

use windows::Win32::Foundation::STATUS_SUCCESS;
use windows::Win32::System::Power::{
    CallNtPowerInformation, ProcessorInformation, PROCESSOR_POWER_INFORMATION,
};
use windows::Win32::System::Threading::{
    GetActiveProcessorCount, GetCurrentProcessorNumberEx, ALL_PROCESSOR_GROUPS,
};

use crate::hw::processors::ProcId;

/// The power manager's view of one logical processor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessorPower {
    pub id: ProcId,
    /// Nominal frequency.
    pub max_mhz: u32,
    pub current_mhz: u32,
}

/// Queries the processors of the calling thread's processor group (the call
/// does not reach other groups). Empty when the query fails.
pub fn processors() -> Vec<ProcessorPower> {
    // SAFETY: neither call has preconditions.
    let (total, group) =
        unsafe { (GetActiveProcessorCount(ALL_PROCESSOR_GROUPS), GetCurrentProcessorNumberEx().Group) };
    let mut raw = vec![PROCESSOR_POWER_INFORMATION::default(); total as usize];
    let Ok(bytes) = u32::try_from(std::mem::size_of_val(raw.as_slice())) else { return Vec::new() };
    // SAFETY: the output buffer is `bytes` long and holds plain integers; no input buffer is passed.
    let status = unsafe {
        CallNtPowerInformation(ProcessorInformation, None, 0, Some(raw.as_mut_ptr().cast()), bytes)
    };
    if status != STATUS_SUCCESS {
        return Vec::new();
    }
    from_raw(group, &raw)
}

/// Nominal frequency in MHz: the highest one reported for any processor.
pub fn nominal_mhz() -> Option<f64> {
    processors().iter().map(|p| p.max_mhz).max().map(f64::from)
}

/// Keeps the entries the call filled in; the rest of an oversized buffer stays zero.
fn from_raw(group: u16, raw: &[PROCESSOR_POWER_INFORMATION]) -> Vec<ProcessorPower> {
    raw.iter()
        .filter(|info| info.MaxMhz > 0)
        .filter_map(|info| {
            let id = ProcId { group, number: u8::try_from(info.Number).ok()? };
            Some(ProcessorPower { id, max_mhz: info.MaxMhz, current_mhz: info.CurrentMhz })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(number: u32, max: u32, current: u32) -> PROCESSOR_POWER_INFORMATION {
        PROCESSOR_POWER_INFORMATION { Number: number, MaxMhz: max, CurrentMhz: current, ..Default::default() }
    }

    #[test]
    fn maps_filled_entries_to_the_group() {
        let raw = [info(0, 3600, 3600), info(1, 3600, 1200), info(0, 0, 0), info(300, 3600, 3600)];
        assert_eq!(
            from_raw(2, &raw),
            vec![
                ProcessorPower { id: ProcId { group: 2, number: 0 }, max_mhz: 3600, current_mhz: 3600 },
                ProcessorPower { id: ProcId { group: 2, number: 1 }, max_mhz: 3600, current_mhz: 1200 },
            ]
        );
    }

    #[test]
    fn query_never_reports_empty_entries() {
        assert!(processors().iter().all(|p| p.max_mhz > 0));
    }
}
