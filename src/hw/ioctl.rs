//! Device handles for information requests.

use windows::core::{Owned, HSTRING};
use windows::Win32::Foundation::{BOOL, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Power::GetDevicePowerState;
use windows::Win32::System::IO::DeviceIoControl;

/// A device opened for control requests only, closed on drop.
pub struct DeviceHandle(Owned<HANDLE>);

impl DeviceHandle {
    /// Opens a device without asking for read or write access. The requests sent here
    /// are the ones Windows answers on such a handle, so no elevation is involved, and
    /// nothing on the device can be read or changed through it.
    pub fn open(path: &str) -> Option<DeviceHandle> {
        // SAFETY: the path is NUL-terminated and outlives the call. On success the handle
        // is ours to close, which `Owned` does.
        unsafe {
            CreateFileW(
                &HSTRING::from(path),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                HANDLE::default(),
            )
            .ok()
            .map(|handle| DeviceHandle(Owned::new(handle)))
        }
    }

    /// Opens a device for reading and writing, as a driver that takes requests through
    /// its own control codes wants it. The error tells a missing device from a refusal.
    pub fn open_read_write(path: &str) -> windows::core::Result<DeviceHandle> {
        // SAFETY: as in `open`.
        unsafe {
            CreateFileW(
                &HSTRING::from(path),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                HANDLE::default(),
            )
            .map(|handle| DeviceHandle(Owned::new(handle)))
        }
    }

    /// Whether the device is in its working power state; `None` when Windows cannot tell.
    /// Asking does not wake a device that sleeps.
    pub fn is_awake(&self) -> Option<bool> {
        let mut on = BOOL::default();
        // SAFETY: the handle is open and `on` is a live BOOL the call writes to.
        unsafe { GetDevicePowerState(*self.0, &mut on) }.as_bool().then_some(on.as_bool())
    }

    /// Sends a control request with `input` and returns what the driver answered, at
    /// most `capacity` bytes. `None` when the request fails.
    pub fn control(&self, code: u32, input: &[u8], capacity: usize) -> Option<Vec<u8>> {
        let mut output = vec![0u8; capacity];
        let mut returned = 0u32;
        // SAFETY: the handle is open, both buffers are valid for the lengths passed along
        // with them and `returned` is a live out-parameter. Without an OVERLAPPED the call
        // completes before it returns.
        unsafe {
            DeviceIoControl(
                *self.0,
                code,
                Some(input.as_ptr().cast()),
                u32::try_from(input.len()).ok()?,
                Some(output.as_mut_ptr().cast()),
                u32::try_from(output.len()).ok()?,
                Some(&mut returned),
                None,
            )
        }
        .ok()?;
        output.truncate((returned as usize).min(capacity));
        Some(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_devices_do_not_open() {
        assert!(DeviceHandle::open(r"\\.\rs-cpuz-test-missing").is_none());
        assert!(DeviceHandle::open("").is_none());
    }

    #[test]
    fn failed_requests_yield_nothing() {
        // The null device is always there and knows no control requests.
        let null = DeviceHandle::open(r"\\.\NUL").expect("the null device");
        assert_eq!(null.control(0x0022_2000, &[], 16), None);
        assert_eq!(null.control(0x0022_2000, &[1, 2, 3, 4], 0), None);
    }
}
