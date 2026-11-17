//! The connected displays, each described by the EDID it handed to Windows.
//!
//! Windows keeps the EDID it read from a display with the display's device, below the
//! `DISPLAY` enumerator of the registry. Going through the present devices of the
//! monitor class leaves out the displays that were connected at some earlier time.

mod edid;

use windows::Win32::Devices::DeviceAndDriverInstallation::GUID_DEVCLASS_MONITOR;

use super::setup::DeviceSet;
use crate::model::MonitorInfo;

/// Registry value under a display's `Device Parameters` key.
const EDID_VALUE: &str = "EDID";

/// An EDID has at most 256 blocks of 128 bytes.
const MAX_EDID_BYTES: u32 = 256 * 128;

/// Lists the connected displays that have an EDID, in the order Windows enumerates them.
pub fn detect() -> Vec<MonitorInfo> {
    let Some(monitors) = DeviceSet::of_class(&GUID_DEVCLASS_MONITOR) else {
        return Vec::new();
    };
    monitors
        .devices()
        .filter_map(|monitor| edid::parse(&monitor.parameter(EDID_VALUE, MAX_EDID_BYTES)?))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_is_self_consistent() {
        for monitor in detect() {
            assert!(monitor.year.is_some_and(|year| year >= 1990));
            assert!(monitor.id.iter().all(|id| id.len() == 7));
            assert!(monitor.video_input.is_some());
        }
    }
}
