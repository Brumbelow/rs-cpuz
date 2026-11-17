//! Windows and DirectX versions, and the hardware errors Windows has logged.

mod whea;

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows::core::{Owned, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_SUCCESS, FILETIME, SYSTEMTIME};
use windows::Win32::System::Registry::{
    RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY,
    RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};
use windows::Win32::System::SystemInformation::{
    GetSystemDirectoryW, IMAGE_FILE_MACHINE, IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64,
    IMAGE_FILE_MACHINE_ARMNT, IMAGE_FILE_MACHINE_I386,
};
use windows::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};
use windows::Win32::System::Time::{
    FileTimeToSystemTime, SystemTimeToFileTime, SystemTimeToTzSpecificLocalTime, TIME_ZONE_INFORMATION,
};
use windows::Win32::System::WindowsProgramming::{GetComputerNameW, MAX_COMPUTERNAME_LENGTH};

use crate::model::OsInfo;

/// Registry paths below `HKEY_LOCAL_MACHINE`.
const VERSION_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
const SETUP_KEY: &str = r"SYSTEM\Setup";
/// Setup subkeys named `Source OS (Updated on ...)` keep the version values of a replaced system.
const SOURCE_OS_PREFIX: &str = "Source OS";
const INSTALL_DATE: &str = "InstallDate";

/// First Windows 11 build. Its registry `ProductName` still reads "Windows 10".
const WINDOWS_11_BUILD: u32 = 22000;

/// Longest registry key name, in UTF-16 units, plus the terminating NUL.
const KEY_NAME_CAPACITY: usize = 256;

/// Seconds between the FILETIME epoch (1601) and the Unix epoch (1970).
const FILETIME_EPOCH_OFFSET: u64 = 11_644_473_600;
/// FILETIME ticks (100 ns) per second.
const TICKS_PER_SECOND: u64 = 10_000_000;
const SECONDS_PER_DAY: u64 = 86_400;

/// Runtime library that marks each Direct3D generation, newest first.
const DIRECTX_RUNTIMES: [(&str, &str); 4] =
    [("d3d12.dll", "12.0"), ("d3d11.dll", "11.0"), ("d3d10.dll", "10.0"), ("d3d9.dll", "9.0")];

/// Reads the Windows version, the original installation date, the computer name,
/// the installed DirectX generation and the number of logged hardware errors.
/// Anything unreadable stays empty.
pub fn detect() -> OsInfo {
    let version = Key::open(VERSION_KEY);
    let string = |value| version.as_ref().and_then(|key| key.string(value));
    let dword = |value| version.as_ref().and_then(|key| key.dword(value));

    let build = string("CurrentBuildNumber");
    let build_number = build.as_deref().and_then(|b| b.parse().ok());
    OsInfo {
        name: string("ProductName")
            .map(|product| os_name(&product, build_number, native_arch()))
            .unwrap_or_default(),
        build: build.map(|b| build_text(&b, dword("UBR"))).unwrap_or_default(),
        directx: system_directory().and_then(|dir| directx_version(|dll| dir.join(dll).exists())),
        computer_name: computer_name().unwrap_or_default(),
        install_date: first_install_time(dword(INSTALL_DATE))
            .map(|seconds| date_text(to_local(u64::from(seconds), None))),
        whea_errors: whea::error_count(),
    }
}

/// `Microsoft Windows 11 Home (x64)` - the product name with the Windows 11
/// correction and the native architecture applied.
fn os_name(product: &str, build: Option<u32>, arch: Option<&str>) -> String {
    let mut name = product.trim().to_string();
    if build.is_some_and(|b| b >= WINDOWS_11_BUILD) {
        name = name.replacen("Windows 10", "Windows 11", 1);
    }
    if !name.starts_with("Microsoft") {
        name.insert_str(0, "Microsoft ");
    }
    if let Some(arch) = arch {
        name.push_str(&format!(" ({arch})"));
    }
    name
}

/// `Build 26100.2314` - the build number and, when known, the update revision.
fn build_text(build: &str, revision: Option<u32>) -> String {
    match revision {
        Some(revision) => format!("Build {build}.{revision}"),
        None => format!("Build {build}"),
    }
}

/// The newest DirectX generation whose runtime library `present` reports.
fn directx_version(present: impl Fn(&str) -> bool) -> Option<String> {
    DIRECTX_RUNTIMES.iter().find(|(dll, _)| present(dll)).map(|(_, version)| version.to_string())
}

/// `x64`, `ARM64`, `x86` or `ARM` for a PE machine type.
fn arch_label(machine: IMAGE_FILE_MACHINE) -> Option<&'static str> {
    match machine {
        IMAGE_FILE_MACHINE_AMD64 => Some("x64"),
        IMAGE_FILE_MACHINE_ARM64 => Some("ARM64"),
        IMAGE_FILE_MACHINE_I386 => Some("x86"),
        IMAGE_FILE_MACHINE_ARMNT => Some("ARM"),
        _ => None,
    }
}

/// The earliest timestamp that is set (non-zero).
fn oldest(times: impl IntoIterator<Item = u32>) -> Option<u32> {
    times.into_iter().filter(|&seconds| seconds != 0).min()
}

/// `MM/DD/YYYY` for a count of seconds since 1970-01-01.
fn date_text(seconds: u64) -> String {
    let (year, month, day) = civil_from_days(seconds / SECONDS_PER_DAY);
    format!("{month:02}/{day:02}/{year:04}")
}

/// Converts days since 1970-01-01 to a (year, month, day) Gregorian date.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    // Count from 0000-03-01 so the leap day is the last day of a 400-year era.
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted % 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

/// Unix time of the first Windows installation, given the running system's `InstallDate`.
/// A feature update resets that value and archives the replaced system's values under
/// the setup key, so the original date is the oldest of the current and archived ones.
fn first_install_time(current: Option<u32>) -> Option<u32> {
    let archived = Key::open(SETUP_KEY)
        .map(|setup| setup.subkey_names())
        .unwrap_or_default()
        .into_iter()
        .filter(|name| name.starts_with(SOURCE_OS_PREFIX))
        .filter_map(|name| Key::open(&format!(r"{SETUP_KEY}\{name}"))?.dword(INSTALL_DATE));
    oldest(current.into_iter().chain(archived))
}

/// Shifts a UTC Unix timestamp to local time in `zone` (the system's own when `None`),
/// with the daylight-saving offset of that date rather than today's. Falls back to UTC
/// if the conversion fails.
fn to_local(unix_seconds: u64, zone: Option<&TIME_ZONE_INFORMATION>) -> u64 {
    let ticks = (unix_seconds + FILETIME_EPOCH_OFFSET) * TICKS_PER_SECOND;
    let utc = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let mut universal = SYSTEMTIME::default();
    let mut zoned = SYSTEMTIME::default();
    let mut local = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    // SAFETY: every pointer refers to a live value owned by this frame or borrowed through `zone`.
    let converted = unsafe {
        FileTimeToSystemTime(&utc, &mut universal)
            .and_then(|()| {
                SystemTimeToTzSpecificLocalTime(zone.map(std::ptr::from_ref), &universal, &mut zoned)
            })
            .and_then(|()| SystemTimeToFileTime(&zoned, &mut local))
    };
    if converted.is_err() {
        return unix_seconds;
    }
    let ticks = (u64::from(local.dwHighDateTime) << 32) | u64::from(local.dwLowDateTime);
    (ticks / TICKS_PER_SECOND).saturating_sub(FILETIME_EPOCH_OFFSET)
}

/// A registry key below `HKEY_LOCAL_MACHINE`, open for reading in the 64-bit view.
struct Key(Owned<HKEY>);

impl Key {
    fn open(path: &str) -> Option<Self> {
        let mut handle = HKEY::default();
        // SAFETY: `handle` is a live out-parameter that receives the opened key on success.
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                &HSTRING::from(path),
                0,
                KEY_READ | KEY_WOW64_64KEY,
                &mut handle,
            )
        };
        // SAFETY: on success `handle` is a key nothing else owns; dropping `Owned` closes it once.
        (status == ERROR_SUCCESS).then(|| Key(unsafe { Owned::new(handle) }))
    }

    /// Reads a string value. `None` when missing or empty.
    fn string(&self, value: &str) -> Option<String> {
        let mut buf = [0u16; 256];
        let mut size = std::mem::size_of_val(&buf) as u32;
        // SAFETY: `buf` is writable for `size` bytes, and RRF_RT_REG_SZ makes the call
        // NUL-terminate whatever it stores there.
        let status = unsafe {
            RegGetValueW(
                *self.0,
                PCWSTR::null(),
                &HSTRING::from(value),
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let text = String::from_utf16_lossy(&buf[..len]);
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_string())
    }

    /// Reads a DWORD value.
    fn dword(&self, value: &str) -> Option<u32> {
        let mut data = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: `data` is a writable DWORD and `size` holds its size in bytes.
        let status = unsafe {
            RegGetValueW(
                *self.0,
                PCWSTR::null(),
                &HSTRING::from(value),
                RRF_RT_REG_DWORD,
                None,
                Some(std::ptr::from_mut(&mut data).cast()),
                Some(&mut size),
            )
        };
        (status == ERROR_SUCCESS).then_some(data)
    }

    /// Names of the direct subkeys.
    fn subkey_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for index in 0..u32::MAX {
            let mut buf = [0u16; KEY_NAME_CAPACITY];
            let mut len = buf.len() as u32;
            // SAFETY: `buf` holds `len` UTF-16 units, and the class and timestamp outputs are optional.
            let status = unsafe {
                RegEnumKeyExW(
                    *self.0,
                    index,
                    PWSTR(buf.as_mut_ptr()),
                    &mut len,
                    None,
                    PWSTR::null(),
                    None,
                    None,
                )
            };
            // Any status but success ends the walk; the usual one is ERROR_NO_MORE_ITEMS.
            if status != ERROR_SUCCESS {
                break;
            }
            names.push(String::from_utf16_lossy(&buf[..(len as usize).min(buf.len())]));
        }
        names
    }
}

/// Architecture of the operating system itself, even when this process is emulated.
fn native_arch() -> Option<&'static str> {
    let mut process = IMAGE_FILE_MACHINE::default();
    let mut native = IMAGE_FILE_MACHINE::default();
    // SAFETY: both out-pointers refer to live locals; the pseudo handle needs no cleanup.
    unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, Some(&mut native)) }.ok()?;
    arch_label(native)
}

/// The NetBIOS name of this computer.
fn computer_name() -> Option<String> {
    let mut buf = [0u16; MAX_COMPUTERNAME_LENGTH as usize + 1];
    let mut len = buf.len() as u32;
    // SAFETY: `buf` holds `len` UTF-16 units; on success `len` is the count written.
    unsafe { GetComputerNameW(PWSTR(buf.as_mut_ptr()), &mut len) }.ok()?;
    let name = String::from_utf16_lossy(buf.get(..len as usize)?);
    (!name.is_empty()).then_some(name)
}

/// The Windows system directory (`System32`).
fn system_directory() -> Option<PathBuf> {
    let mut buf = [0u16; 260];
    // SAFETY: the call writes at most `buf.len()` UTF-16 units into `buf`.
    let len = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
    (len > 0 && len < buf.len()).then(|| PathBuf::from(OsString::from_wide(&buf[..len])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_windows_11_from_its_build_number() {
        assert_eq!(os_name("Windows 10 Home", Some(26100), Some("x64")), "Microsoft Windows 11 Home (x64)");
        assert_eq!(os_name("Windows 10 Pro", Some(22000), None), "Microsoft Windows 11 Pro");
        assert_eq!(
            os_name("Windows 10 Enterprise LTSC 2024", Some(26100), Some("ARM64")),
            "Microsoft Windows 11 Enterprise LTSC 2024 (ARM64)"
        );
    }

    #[test]
    fn keeps_other_product_names() {
        assert_eq!(os_name("Windows 10 Pro", Some(19045), Some("x64")), "Microsoft Windows 10 Pro (x64)");
        assert_eq!(os_name(" Windows 10 Home ", None, Some("x86")), "Microsoft Windows 10 Home (x86)");
        assert_eq!(
            os_name("Windows Server 2025 Datacenter", Some(26100), Some("x64")),
            "Microsoft Windows Server 2025 Datacenter (x64)"
        );
        assert_eq!(os_name("Microsoft Windows 8.1", Some(9600), None), "Microsoft Windows 8.1");
    }

    #[test]
    fn formats_build() {
        assert_eq!(build_text("26100", Some(2314)), "Build 26100.2314");
        assert_eq!(build_text("19045", None), "Build 19045");
    }

    #[test]
    fn picks_newest_directx_runtime() {
        assert_eq!(directx_version(|_| true).as_deref(), Some("12.0"));
        assert_eq!(directx_version(|dll| dll != "d3d12.dll").as_deref(), Some("11.0"));
        assert_eq!(directx_version(|dll| dll == "d3d10.dll" || dll == "d3d9.dll").as_deref(), Some("10.0"));
        assert_eq!(directx_version(|dll| dll == "d3d9.dll").as_deref(), Some("9.0"));
        assert_eq!(directx_version(|_| false), None);
    }

    #[test]
    fn labels_architectures() {
        assert_eq!(arch_label(IMAGE_FILE_MACHINE_AMD64), Some("x64"));
        assert_eq!(arch_label(IMAGE_FILE_MACHINE_ARM64), Some("ARM64"));
        assert_eq!(arch_label(IMAGE_FILE_MACHINE_I386), Some("x86"));
        assert_eq!(arch_label(IMAGE_FILE_MACHINE(0)), None);
    }

    #[test]
    fn picks_the_oldest_install_time() {
        assert_eq!(oldest([1_700_000_300, 1_600_000_200, 1_650_000_100]), Some(1_600_000_200));
        assert_eq!(oldest([0, 1_600_000_200]), Some(1_600_000_200));
        assert_eq!(oldest([0]), None);
        assert_eq!(oldest([]), None);
    }

    #[test]
    fn formats_dates() {
        assert_eq!(date_text(0), "01/01/1970");
        assert_eq!(date_text(86_399), "01/01/1970");
        assert_eq!(date_text(86_400), "01/02/1970");
        // 2000-02-29 and the day after: a leap day in a century year.
        assert_eq!(date_text(951_782_400), "02/29/2000");
        assert_eq!(date_text(951_868_800), "03/01/2000");
        // 2023-11-14 22:13:20 UTC.
        assert_eq!(date_text(1_700_000_000), "11/14/2023");
        assert_eq!(date_text(1_735_689_599), "12/31/2024");
        assert_eq!(date_text(u64::from(u32::MAX)), "02/07/2106");
    }

    #[test]
    fn local_time_uses_the_offset_of_the_converted_date() {
        // US Eastern rules: UTC-5, with daylight time from the second Sunday of March
        // to the first Sunday of November.
        let eastern = TIME_ZONE_INFORMATION {
            Bias: 300,
            StandardDate: SYSTEMTIME { wMonth: 11, wDay: 1, wHour: 2, ..Default::default() },
            DaylightDate: SYSTEMTIME { wMonth: 3, wDay: 2, wHour: 2, ..Default::default() },
            DaylightBias: -60,
            ..Default::default()
        };
        // 2024-07-04 04:30 UTC is half past midnight in daylight time.
        let summer = 1_720_067_400;
        assert_eq!(to_local(summer, Some(&eastern)), summer - 4 * 3600);
        assert_eq!(date_text(to_local(summer, Some(&eastern))), "07/04/2024");
        // 2024-01-01 04:30 UTC is still the evening before in standard time.
        let winter = 1_704_083_400;
        assert_eq!(to_local(winter, Some(&eastern)), winter - 5 * 3600);
        assert_eq!(date_text(to_local(winter, Some(&eastern))), "12/31/2023");

        let utc = TIME_ZONE_INFORMATION::default();
        assert_eq!(date_text(to_local(summer, Some(&utc))), "07/04/2024");
        assert_eq!(date_text(to_local(winter, Some(&utc))), "01/01/2024");
        // The system's own zone, whichever it is, lies within a day of UTC.
        assert!(to_local(summer, None).abs_diff(summer) < SECONDS_PER_DAY);
    }

    #[test]
    fn missing_registry_data_reads_as_none() {
        assert!(Key::open(r"SOFTWARE\rs-cpuz-test\Missing").is_none());
        let software = Key::open("SOFTWARE").expect("HKLM\\SOFTWARE exists on every Windows");
        assert_eq!(software.string("rs-cpuz-test-missing"), None);
        assert_eq!(software.dword("rs-cpuz-test-missing"), None);
        assert!(software.subkey_names().iter().any(|name| name.eq_ignore_ascii_case("Microsoft")));
    }

    #[test]
    fn detection_is_self_consistent() {
        let os = detect();
        assert!(os.name.is_empty() || os.name.starts_with("Microsoft "));
        assert!(os.build.is_empty() || os.build.starts_with("Build "));
        assert!(os.install_date.iter().all(|date| date.len() == 10));
    }
}
