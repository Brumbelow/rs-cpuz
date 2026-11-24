//! The PawnIO driver, for the figures only ring 0 can read.
//!
//! PawnIO (https://pawnio.eu) is a signed kernel driver that runs small signed modules
//! and nothing else; the user installs it separately. rs-cpuz embeds three of its
//! modules (see `assets/pawnio`) and uses them only to read: model-specific registers
//! for temperatures, voltage and multipliers, the memory controller's registers for the
//! timings, and the SMBus for the memory modules' SPD data. The modules themselves would
//! also write; the wrappers here are the only way rs-cpuz talks to them, and each one
//! checks what it is asked to read against a fixed list before the request is sent.
//! Nothing outside this module can run a module function: a request is a [`Call`], which
//! only the wrappers build.
//!
//! The driver is used only when the program runs elevated (its device admits
//! administrators only), the settings allow it, and `main` switched it on: tests and the
//! sample system never do.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::w;
use windows::Win32::Foundation::ERROR_ACCESS_DENIED;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

use super::ioctl::DeviceHandle;

/// The driver's device.
const DEVICE: &str = r"\\?\GLOBALROOT\Device\PawnIO";
/// Loads a module into the handle it is sent to: the input is the signed module image.
const IOCTL_LOAD_BINARY: u32 = 0xA1B2_2084;
/// Runs a function of the loaded module: a 32-byte NUL-padded name, then 64-bit arguments in;
/// 64-bit results out.
const IOCTL_EXECUTE: u32 = 0xA1B2_2104;
/// Length of the function name field, terminating NUL included.
const NAME_FIELD: usize = 32;

/// Size of the memory controller window the module maps on Tiger Lake and later.
pub const MCHBAR_WINDOW: u32 = 0x2_0000;

/// The model-specific registers rs-cpuz reads: performance status (core voltage),
/// thermal status (core temperature), temperature target (TjMax), package thermal status,
/// platform information (ratios), turbo ratio limits and uncore performance status.
const MSR_READS: [u32; 7] = [0x198, 0x19C, 0x1A2, 0x1B1, 0xCE, 0x1AD, 0x621];

/// The SMBus addresses of SPD devices (EEPROMs and DDR5 hubs), the only ones read.
const SPD_ADDRESSES: std::ops::RangeInclusive<i64> = 0x50..=0x57;
/// SMBus protocol code of a byte-data transfer, and the direction flag of a read.
const SMBUS_BYTE_DATA: i64 = 2;
const SMBUS_READ: i64 = 1;

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Lets detection use the driver. `main` calls this for an elevated run that the settings
/// allow it for.
pub fn enable() {
    ENABLED.store(true, Ordering::Release);
}

/// Whether detection may use the driver.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
}

/// Where the driver stands, as the About tab and the reports say it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// In use; the installed version.
    Active(String),
    /// Running, but none of its modules serves this machine, so nothing was read through it.
    Unused(String),
    NotInstalled,
    /// Installed, but its device does not open: the driver is stopped or blocked, or waits
    /// for a restart.
    NotRunning(String),
    /// Installed, but the program runs without administrator rights.
    NeedsAdministrator,
    /// Turned off in the settings, or not offered (the sample system, command-line runs
    /// without elevation).
    Disabled,
}

impl Status {
    pub fn text(&self) -> String {
        match self {
            Status::Active(version) => format!("PawnIO: {version}"),
            Status::Unused(version) => format!("PawnIO: {version} (nothing to read on this processor)"),
            Status::NotInstalled => "PawnIO: not installed".into(),
            Status::NotRunning(version) => format!("PawnIO: {version} installed, not running"),
            Status::NeedsAdministrator => "PawnIO: needs administrator".into(),
            Status::Disabled => "PawnIO: disabled".into(),
        }
    }
}

/// Where the driver stands for this run: `wanted` is whether the settings allow it, `used`
/// whether one of its modules served detection.
pub fn status(wanted: bool, used: bool) -> Status {
    if !wanted {
        return Status::Disabled;
    }
    let version = || installed_version().unwrap_or_else(|| "installed".into());
    match DeviceHandle::open_read_write(DEVICE) {
        Ok(_) if enabled() && used => Status::Active(version()),
        Ok(_) if enabled() => Status::Unused(version()),
        Ok(_) => Status::Disabled,
        Err(error) if error.code() == ERROR_ACCESS_DENIED.to_hresult() => Status::NeedsAdministrator,
        Err(_) => installed_version().map_or(Status::NotInstalled, Status::NotRunning),
    }
}

/// The version the PawnIO installer registered, e.g. `2.2.0`.
fn installed_version() -> Option<String> {
    let mut buffer = [0u16; 64];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: the key and value names are NUL-terminated literals, and `buffer` holds
    // `size` bytes for the call to write to.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO"),
            w!("DisplayVersion"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return None;
    }
    let units = &buffer[..(size as usize / 2).min(buffer.len())];
    let text = String::from_utf16_lossy(units).trim_end_matches('\0').trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// A request to run a module function: its name, its arguments, and how many results it
/// gives. The fields are private, so only the read wrappers here can build one.
pub struct Call<'a> {
    function: &'a str,
    input: &'a [i64],
    outputs: usize,
}

/// Runs a function of a loaded module. Implemented by [`Module`]; tests record the calls.
pub trait Transport {
    /// Runs `call` and returns its results; `None` when the driver refused.
    fn execute(&self, call: Call) -> Option<Vec<i64>>;
}

/// The modules rs-cpuz embeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    IntelMsr,
    IntelMchbar,
    SmbusI801,
}

impl Kind {
    fn image(self) -> &'static [u8] {
        match self {
            Kind::IntelMsr => include_bytes!("../../assets/pawnio/IntelMSR.bin"),
            Kind::IntelMchbar => include_bytes!("../../assets/pawnio/IntelMCHBAR.bin"),
            Kind::SmbusI801 => include_bytes!("../../assets/pawnio/SmbusI801.bin"),
        }
    }
}

/// A module loaded into the driver on a handle of its own; unloaded when dropped.
pub struct Module(DeviceHandle);

impl Module {
    /// Opens the driver and loads `kind` into it. `None` when the driver is off, missing,
    /// refuses the program, or does not take the module (another processor, chipset or
    /// generation than the module serves).
    fn open(kind: Kind) -> Option<Module> {
        if !enabled() {
            return None;
        }
        let device = DeviceHandle::open_read_write(DEVICE).ok()?;
        device.control(IOCTL_LOAD_BINARY, kind.image(), 0)?;
        Some(Module(device))
    }
}

impl Transport for Module {
    fn execute(&self, call: Call) -> Option<Vec<i64>> {
        let request = encode_call(call.function, call.input)?;
        let answer = self.0.control(IOCTL_EXECUTE, &request, call.outputs * 8)?;
        let results = decode_outputs(&answer);
        (results.len() == call.outputs).then_some(results)
    }
}

/// The input of an execute request: the function name, NUL-padded to 32 bytes, followed
/// by the arguments. `None` for a name that does not fit.
fn encode_call(function: &str, input: &[i64]) -> Option<Vec<u8>> {
    let name = function.as_bytes();
    if name.len() >= NAME_FIELD {
        return None;
    }
    let mut call = vec![0u8; NAME_FIELD];
    call[..name.len()].copy_from_slice(name);
    call.extend(input.iter().flat_map(|value| value.to_le_bytes()));
    Some(call)
}

/// The 64-bit results in an execute answer.
fn decode_outputs(bytes: &[u8]) -> Vec<i64> {
    bytes.chunks_exact(8).map(|chunk| i64::from_le_bytes(chunk.try_into().unwrap_or_default())).collect()
}

/// Model-specific register reads through the `IntelMSR` module. A read runs on the
/// processor the calling thread runs on.
pub struct Msr<T: Transport>(T);

impl Msr<Module> {
    /// Loads the module; `None` when the driver is off or the processor is not one it serves.
    pub fn open() -> Option<Msr<Module>> {
        Module::open(Kind::IntelMsr).map(Msr)
    }
}

impl<T: Transport> Msr<T> {
    /// The value of register `msr`, when it is one rs-cpuz reads.
    pub fn read(&self, msr: u32) -> Option<u64> {
        if !MSR_READS.contains(&msr) {
            return None;
        }
        let call = Call { function: "ioctl_read_msr", input: &[i64::from(msr)], outputs: 1 };
        self.0.execute(call).map(|values| values[0] as u64)
    }
}

/// Memory controller register reads through the `IntelMCHBAR` module.
pub struct Mchbar<T: Transport>(T);

impl Mchbar<Module> {
    /// Loads the module; `None` when the driver is off or the memory controller is not one
    /// it serves.
    pub fn open() -> Option<Mchbar<Module>> {
        Module::open(Kind::IntelMchbar).map(Mchbar)
    }
}

impl<T: Transport> Mchbar<T> {
    /// The 32-bit register at `offset` into the controller's window.
    pub fn read(&self, offset: u32) -> Option<u32> {
        if offset % 4 != 0 || offset >= MCHBAR_WINDOW {
            return None;
        }
        let call = Call { function: "ioctl_read_dword", input: &[i64::from(offset)], outputs: 1 };
        self.0.execute(call).map(|values| values[0] as u32)
    }
}

/// SMBus reads through the `SmbusI801` module, limited to SPD devices.
pub struct Smbus<T: Transport>(T);

impl Smbus<Module> {
    /// Loads the module; `None` when the driver is off or the chipset has no SMBus controller
    /// it serves.
    pub fn open() -> Option<Smbus<Module>> {
        Module::open(Kind::SmbusI801).map(Smbus)
    }
}

impl<T: Transport> Smbus<T> {
    /// The byte at `command` (register or offset) of the SPD device at `address`.
    pub fn read_byte(&self, address: u8, command: u8) -> Option<u8> {
        let address = i64::from(address);
        if !SPD_ADDRESSES.contains(&address) {
            return None;
        }
        let input = [address, SMBUS_READ, i64::from(command), SMBUS_BYTE_DATA];
        let values = self.0.execute(Call { function: "ioctl_smbus_xfer", input: &input, outputs: 1 })?;
        u8::try_from(values[0]).ok()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// A driver stand-in that records every call and answers with `answer`.
    #[derive(Default)]
    struct Recorder {
        calls: RefCell<Vec<(String, Vec<i64>)>>,
        answer: i64,
    }

    impl Transport for Recorder {
        fn execute(&self, call: Call) -> Option<Vec<i64>> {
            self.calls.borrow_mut().push((call.function.to_string(), call.input.to_vec()));
            Some(vec![self.answer; call.outputs])
        }
    }

    #[test]
    fn calls_are_laid_out_as_the_driver_reads_them() {
        let call = encode_call("ioctl_read_msr", &[0x19C]).unwrap();
        assert_eq!(call.len(), 32 + 8);
        assert_eq!(&call[..14], b"ioctl_read_msr");
        assert!(call[14..32].iter().all(|&byte| byte == 0));
        assert_eq!(&call[32..], &0x19C_i64.to_le_bytes());
        // A name needs its terminating NUL inside the 32 bytes.
        assert!(encode_call(&"x".repeat(32), &[]).is_none());
        assert_eq!(
            decode_outputs(&[1, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
            [1, -1]
        );
    }

    #[test]
    fn only_allowed_reads_reach_the_driver() {
        let msr = Msr(Recorder { answer: 7, ..Recorder::default() });
        assert_eq!(msr.read(0x19C), Some(7));
        // The overclocking mailbox and the power limit can be written through the module;
        // rs-cpuz never asks for anything outside its list of readings.
        assert_eq!(msr.read(0x150), None);
        assert_eq!(msr.read(0x610), None);

        let mchbar = Mchbar(Recorder { answer: 5, ..Recorder::default() });
        assert_eq!(mchbar.read(0xE070), Some(5));
        assert_eq!(mchbar.read(0xE071), None, "unaligned");
        assert_eq!(mchbar.read(MCHBAR_WINDOW), None, "outside the window");

        let smbus = Smbus(Recorder { answer: 0x51, ..Recorder::default() });
        assert_eq!(smbus.read_byte(0x50, 0x00), Some(0x51));
        assert_eq!(smbus.read_byte(0x57, 0x80), Some(0x51));
        // Page selects (0x36/0x37), the clock generator, anything that is no SPD.
        for address in [0x36, 0x37, 0x48, 0x58, 0x69] {
            assert_eq!(smbus.read_byte(address, 0), None, "{address:#x}");
        }

        let calls: Vec<(String, Vec<i64>)> =
            [msr.0.calls, mchbar.0.calls, smbus.0.calls].into_iter().flat_map(RefCell::into_inner).collect();
        assert_eq!(
            calls,
            [
                ("ioctl_read_msr".to_string(), vec![0x19C]),
                ("ioctl_read_dword".to_string(), vec![0xE070]),
                // Address, read, command, the byte-data protocol.
                ("ioctl_smbus_xfer".to_string(), vec![0x50, 1, 0x00, 2]),
                ("ioctl_smbus_xfer".to_string(), vec![0x57, 1, 0x80, 2]),
            ]
        );
    }

    #[test]
    fn smbus_answers_are_bytes() {
        let smbus = Smbus(Recorder { answer: 0x1FF, ..Recorder::default() });
        assert_eq!(smbus.read_byte(0x50, 0), None);
    }

    #[test]
    fn status_explains_itself() {
        assert_eq!(Status::Active("2.2.0".into()).text(), "PawnIO: 2.2.0");
        assert_eq!(
            Status::Unused("2.2.0".into()).text(),
            "PawnIO: 2.2.0 (nothing to read on this processor)"
        );
        assert_eq!(Status::NotInstalled.text(), "PawnIO: not installed");
        assert_eq!(Status::NotRunning("2.2.0".into()).text(), "PawnIO: 2.2.0 installed, not running");
        assert_eq!(Status::NeedsAdministrator.text(), "PawnIO: needs administrator");
        assert_eq!(Status::Disabled.text(), "PawnIO: disabled");
    }

    #[test]
    fn the_driver_is_off_unless_switched_on() {
        // Tests and the sample system never switch it on.
        assert!(!enabled());
    }
}
