//! Live NVIDIA details through NVAPI, the user-mode library the display driver installs.
//!
//! Only entry points and structure layouts published in NVIDIA's open NVAPI SDK
//! (`nvapi.h`, `nvapi_lite_common.h`, `nvapi_interface.h`) are used. The library is
//! loaded from the system directory only and released again once the query is done;
//! when it is missing or a call fails, the values it would have provided stay unknown.

use std::ffi::c_void;
use std::mem;
use std::ptr;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use windows::core::{s, w, Owned, PCWSTR};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};

use super::db;
use super::dxgi::Adapter;
use crate::model::GpuPerfLevel;

#[cfg(target_pointer_width = "64")]
const LIBRARY: PCWSTR = w!("nvapi64.dll");
#[cfg(not(target_pointer_width = "64"))]
const LIBRARY: PCWSTR = w!("nvapi.dll");

/// Interface ids from `nvapi_interface.h`.
mod id {
    pub const INITIALIZE: u32 = 0x0150_E828;
    pub const UNLOAD: u32 = 0xD22B_DD7E;
    pub const ENUM_PHYSICAL_GPUS: u32 = 0xE5AC_921F;
    pub const GPU_GET_PCI_IDENTIFIERS: u32 = 0x2DDF_B66E;
    pub const GPU_GET_BUS_ID: u32 = 0x1BE0_B8E5;
    pub const GPU_GET_GPU_CORE_COUNT: u32 = 0xC702_6A87;
    pub const GPU_GET_RAM_BUS_WIDTH: u32 = 0x7975_C581;
    pub const GPU_GET_ALL_CLOCK_FREQUENCIES: u32 = 0xDCB6_16C3;
}

/// How long to wait for the driver before giving up on the live values.
const QUERY_TIMEOUT: Duration = Duration::from_secs(2);

const NVAPI_OK: Status = 0;
const NVAPI_INCOMPATIBLE_STRUCT_VERSION: Status = -9;
const MAX_PHYSICAL_GPUS: usize = 64;
const MAX_GPU_PUBLIC_CLOCKS: usize = 32;

/// Indices into the clock domains (`NV_GPU_PUBLIC_CLOCK_ID`).
const CLOCK_GRAPHICS: usize = 0;
const CLOCK_MEMORY: usize = 4;
const CLOCK_PROCESSOR: usize = 7;

/// Clock types (`NV_GPU_CLOCK_FREQUENCIES_CLOCK_TYPE`) and the performance level each one
/// describes, in the order the levels are listed: what the card is specified for, then
/// what it is running at.
const PERF_LEVELS: [(u32, &str); 3] = [(1, "Base"), (2, "Boost"), (0, GpuPerfLevel::CURRENT)];

/// `NV_GPU_CLOCK_FREQUENCIES_VER_3` and `_VER_2`, newest first; both use the V2 layout.
const CLOCK_FREQUENCIES_VERSIONS: [u32; 2] =
    [struct_version::<ClockFrequencies>(3), struct_version::<ClockFrequencies>(2)];

type Status = i32;
/// `NvPhysicalGpuHandle`, an opaque pointer.
type Handle = *mut c_void;
type QueryInterface = unsafe extern "C" fn(u32) -> *mut c_void;
type Initialize = unsafe extern "C" fn() -> Status;
type Unload = unsafe extern "C" fn() -> Status;
type EnumPhysicalGpus = unsafe extern "C" fn(*mut Handle, *mut u32) -> Status;
type GetPciIdentifiers = unsafe extern "C" fn(Handle, *mut u32, *mut u32, *mut u32, *mut u32) -> Status;
/// Shape shared by the GPU queries that return a single `NvU32`.
type GetNumber = unsafe extern "C" fn(Handle, *mut u32) -> Status;
type GetAllClockFrequencies = unsafe extern "C" fn(Handle, *mut ClockFrequencies) -> Status;

/// `MAKE_NVAPI_VERSION`: the structure size in the low word, its revision above.
const fn struct_version<T>(revision: u32) -> u32 {
    mem::size_of::<T>() as u32 | (revision << 16)
}

/// One entry of `NV_GPU_CLOCK_FREQUENCIES::domain`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ClockDomain {
    /// Bit 0 is set when the domain exists on the GPU; the other bits are reserved.
    flags: u32,
    khz: u32,
}

/// `NV_GPU_CLOCK_FREQUENCIES_V2`.
#[repr(C)]
struct ClockFrequencies {
    version: u32,
    /// Bits 0-3 select the clock type; the other bits are reserved and stay zero.
    clock_type: u32,
    domains: [ClockDomain; MAX_GPU_PUBLIC_CLOCKS],
}

impl ClockFrequencies {
    fn request(version: u32, clock_type: u32) -> Self {
        ClockFrequencies {
            version,
            clock_type: clock_type & 0xF,
            domains: [ClockDomain::default(); MAX_GPU_PUBLIC_CLOCKS],
        }
    }

    /// Frequency of a clock domain in MHz, when the GPU has it.
    fn mhz(&self, domain: usize) -> Option<f64> {
        let domain = self.domains.get(domain)?;
        (domain.flags & 1 != 0 && domain.khz != 0).then(|| f64::from(domain.khz) / 1000.0)
    }

    /// The clocks as a performance level; `None` when no domain reported a frequency.
    fn perf_level(&self, name: &str) -> Option<GpuPerfLevel> {
        let level = GpuPerfLevel {
            name: name.to_string(),
            core_mhz: self.mhz(CLOCK_GRAPHICS),
            shader_mhz: self.mhz(CLOCK_PROCESSOR),
            memory_mhz: self.mhz(CLOCK_MEMORY),
        };
        (level.core_mhz.is_some() || level.shader_mhz.is_some() || level.memory_mhz.is_some())
            .then_some(level)
    }
}

/// What NVAPI reports for one physical GPU.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Gpu {
    pub device_id: u16,
    /// Subsystem id in the high word, subsystem vendor id in the low word.
    pub subsys: u32,
    pub revision: u8,
    /// PCI bus the GPU sits on.
    pub bus: Option<u32>,
    pub cores: Option<u32>,
    pub memory_bus_width: Option<u32>,
    /// Base, boost and current clocks, as far as the driver reports them.
    pub perf_levels: Vec<GpuPerfLevel>,
}

impl Gpu {
    /// Whether the PCI ids are those of `adapter`.
    pub fn describes(&self, adapter: &Adapter) -> bool {
        adapter.vendor_id == db::NVIDIA
            && self.device_id == adapter.device_id
            && self.subsys == adapter.subsys()
            && self.revision == adapter.revision_id
    }
}

/// Queries every GPU the NVIDIA driver manages. `None` when NVAPI is not available or
/// the driver does not answer in time.
pub fn query() -> Option<Vec<Gpu>> {
    // The calls end up in the driver, so they run on a helper thread that can be abandoned.
    let (sender, receiver) = mpsc::channel();
    let spawned = thread::Builder::new().name("nvapi".into()).spawn(move || {
        // Sending only fails when the wait below has already timed out.
        let _ = sender.send(Api::load().map(|api| api.gpus()));
    });
    spawned.ok()?;
    receiver.recv_timeout(QUERY_TIMEOUT).ok().flatten()
}

/// Index of the entry describing `adapter`, skipping entries already `taken` by an
/// identical card. An entry on the adapter's PCI bus is preferred; one known to sit on
/// a different bus is never chosen.
pub fn pick(gpus: &[Gpu], taken: &[bool], adapter: &Adapter, bus: Option<u8>) -> Option<usize> {
    let bus = bus.map(u32::from);
    let find = |fits_bus: &dyn Fn(&Gpu) -> bool| {
        gpus.iter().enumerate().position(|(index, gpu)| {
            !taken.get(index).copied().unwrap_or(false) && gpu.describes(adapter) && fits_bus(gpu)
        })
    };
    find(&|gpu| bus.is_some() && gpu.bus == bus).or_else(|| find(&|gpu| bus.is_none() || gpu.bus.is_none()))
}

/// Device id from NVAPI's packed identifier (device id above the vendor id).
fn unpack_device_id(packed: u32) -> Option<u16> {
    (packed & 0xFFFF == u32::from(db::NVIDIA)).then_some((packed >> 16) as u16)
}

/// An initialised NVAPI session; dropping it unloads the API and frees the library.
struct Api {
    unload: Unload,
    enum_physical_gpus: EnumPhysicalGpus,
    pci_identifiers: GetPciIdentifiers,
    bus_id: Option<GetNumber>,
    core_count: Option<GetNumber>,
    ram_bus_width: Option<GetNumber>,
    clock_frequencies: Option<GetAllClockFrequencies>,
    _library: Owned<HMODULE>,
}

impl Api {
    fn load() -> Option<Api> {
        // SAFETY: the name is a NUL-terminated literal and the search is confined to the
        // system directory. The module handle is ours to free, which `Owned` does.
        let library = unsafe {
            LoadLibraryExW(LIBRARY, None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok().map(|module| Owned::new(module))
        }?;
        // SAFETY: `library` is loaded and the export name is a NUL-terminated literal.
        let export = unsafe { GetProcAddress(*library, s!("nvapi_QueryInterface")) }?;
        // SAFETY: the export is a C function taking an interface id and returning the
        // address of its implementation, or null.
        let query = unsafe { mem::transmute::<unsafe extern "system" fn() -> isize, QueryInterface>(export) };

        // SAFETY: each type is the prototype the SDK headers declare for its id.
        let (initialize, unload, enum_physical_gpus, pci_identifiers) = unsafe {
            (
                resolve::<Initialize>(query, id::INITIALIZE)?,
                resolve::<Unload>(query, id::UNLOAD)?,
                resolve::<EnumPhysicalGpus>(query, id::ENUM_PHYSICAL_GPUS)?,
                resolve::<GetPciIdentifiers>(query, id::GPU_GET_PCI_IDENTIFIERS)?,
            )
        };
        // SAFETY: `NvAPI_Initialize` takes no arguments. When it fails nothing was set
        // up, so returning early only frees the library.
        if unsafe { initialize() } != NVAPI_OK {
            return None;
        }
        // SAFETY: as for the required entry points above.
        Some(unsafe {
            Api {
                unload,
                enum_physical_gpus,
                pci_identifiers,
                bus_id: resolve(query, id::GPU_GET_BUS_ID),
                core_count: resolve(query, id::GPU_GET_GPU_CORE_COUNT),
                ram_bus_width: resolve(query, id::GPU_GET_RAM_BUS_WIDTH),
                clock_frequencies: resolve(query, id::GPU_GET_ALL_CLOCK_FREQUENCIES),
                _library: library,
            }
        })
    }

    fn gpus(&self) -> Vec<Gpu> {
        let mut handles: [Handle; MAX_PHYSICAL_GPUS] = [ptr::null_mut(); MAX_PHYSICAL_GPUS];
        let mut count = 0u32;
        // SAFETY: the array has the `NVAPI_MAX_PHYSICAL_GPUS` entries the call may fill.
        if unsafe { (self.enum_physical_gpus)(handles.as_mut_ptr(), &mut count) } != NVAPI_OK {
            return Vec::new();
        }
        handles.iter().take(count as usize).filter_map(|&handle| self.describe(handle)).collect()
    }

    fn describe(&self, handle: Handle) -> Option<Gpu> {
        let (mut device, mut subsys, mut revision, mut ext_device) = (0u32, 0u32, 0u32, 0u32);
        // SAFETY: `handle` was just returned by `NvAPI_EnumPhysicalGPUs` and the four
        // outputs are valid for writes.
        let status = unsafe {
            (self.pci_identifiers)(handle, &mut device, &mut subsys, &mut revision, &mut ext_device)
        };
        if status != NVAPI_OK {
            return None;
        }
        Some(Gpu {
            device_id: unpack_device_id(device)?,
            subsys,
            revision: (revision & 0xFF) as u8,
            bus: self.number(self.bus_id, handle),
            cores: self.number(self.core_count, handle).filter(|&cores| cores > 0),
            memory_bus_width: self.number(self.ram_bus_width, handle).filter(|&bits| bits > 0),
            perf_levels: self.perf_levels(handle),
        })
    }

    fn number(&self, get: Option<GetNumber>, handle: Handle) -> Option<u32> {
        let get = get?;
        let mut value = 0u32;
        // SAFETY: `get` is one of the single-number GPU queries, `handle` a live GPU
        // handle and `value` valid for writes.
        (unsafe { get(handle, &mut value) } == NVAPI_OK).then_some(value)
    }

    fn perf_levels(&self, handle: Handle) -> Vec<GpuPerfLevel> {
        PERF_LEVELS
            .iter()
            .filter_map(|&(clock_type, name)| self.clocks(handle, clock_type)?.perf_level(name))
            .collect()
    }

    fn clocks(&self, handle: Handle, clock_type: u32) -> Option<ClockFrequencies> {
        let get = self.clock_frequencies?;
        for version in CLOCK_FREQUENCIES_VERSIONS {
            let mut clocks = ClockFrequencies::request(version, clock_type);
            // SAFETY: `clocks` has the layout and version tag of `NV_GPU_CLOCK_FREQUENCIES`
            // and `handle` is a live GPU handle.
            match unsafe { get(handle, &mut clocks) } {
                NVAPI_OK => return Some(clocks),
                NVAPI_INCOMPATIBLE_STRUCT_VERSION => continue,
                _ => return None,
            }
        }
        None
    }
}

impl Drop for Api {
    fn drop(&mut self) {
        // SAFETY: pairs with the successful `NvAPI_Initialize` in `load`; the library
        // is still loaded because its handle is a field of `self`.
        unsafe { (self.unload)() };
    }
}

/// Looks up the implementation of an NVAPI interface.
///
/// # Safety
/// `query` must be the library's `nvapi_QueryInterface` export and `F` the
/// `extern "C"` function pointer type the SDK declares for `id`.
unsafe fn resolve<F: Copy>(query: QueryInterface, id: u32) -> Option<F> {
    // SAFETY: guaranteed by the caller.
    let pointer = unsafe { query(id) };
    if pointer.is_null() || mem::size_of::<F>() != mem::size_of::<*mut c_void>() {
        return None;
    }
    // SAFETY: `F` is a function pointer type of the same size as `pointer`, which is
    // the non-null address of a function with that signature.
    Some(unsafe { mem::transmute_copy::<*mut c_void, F>(&pointer) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clocks(domains: &[(usize, u32, u32)]) -> ClockFrequencies {
        let mut clocks = ClockFrequencies::request(CLOCK_FREQUENCIES_VERSIONS[0], 0);
        for &(index, flags, khz) in domains {
            clocks.domains[index] = ClockDomain { flags, khz };
        }
        clocks
    }

    #[test]
    fn structure_matches_the_sdk_layout() {
        assert_eq!(mem::size_of::<ClockDomain>(), 8);
        assert_eq!(mem::size_of::<ClockFrequencies>(), 264);
        assert_eq!(CLOCK_FREQUENCIES_VERSIONS, [0x0003_0108, 0x0002_0108]);
        let request = ClockFrequencies::request(CLOCK_FREQUENCIES_VERSIONS[1], 2);
        assert_eq!((request.version, request.clock_type), (0x0002_0108, 2));
        assert!(request.domains.iter().all(|domain| domain.flags == 0 && domain.khz == 0));
    }

    #[test]
    fn lists_the_live_clocks_after_the_specified_ones() {
        // `NV_GPU_CLOCK_FREQUENCIES_CURRENT_FREQ` is 0, `_BASE_CLOCK` 1 and `_BOOST_CLOCK` 2.
        assert_eq!(PERF_LEVELS, [(1, "Base"), (2, "Boost"), (0, "Current")]);
    }

    #[test]
    fn converts_present_domains_to_mhz() {
        let level = clocks(&[(CLOCK_GRAPHICS, 1, 285_000), (CLOCK_MEMORY, 1, 5_001_000)])
            .perf_level("Current")
            .unwrap();
        assert_eq!(level.name, "Current");
        assert_eq!(level.core_mhz, Some(285.0));
        assert_eq!(level.memory_mhz, Some(5001.0));
        assert_eq!(level.shader_mhz, None);

        // Reserved flag bits are ignored, the presence bit is not.
        let level =
            clocks(&[(CLOCK_PROCESSOR, 0xFFFF_FFFF, 1_200_500), (CLOCK_GRAPHICS, 0xFFFF_FFFE, 600_000)])
                .perf_level("Base")
                .unwrap();
        assert_eq!(level.shader_mhz, Some(1200.5));
        assert_eq!(level.core_mhz, None);
    }

    #[test]
    fn empty_clock_reports_are_dropped() {
        assert_eq!(clocks(&[]).perf_level("Boost"), None);
        assert_eq!(clocks(&[(CLOCK_GRAPHICS, 1, 0), (CLOCK_MEMORY, 0, 8_501_000)]).perf_level("Boost"), None);
        // Domains this crate does not show do not make a level on their own.
        assert_eq!(clocks(&[(8, 1, 540_000)]).perf_level("Current"), None);
    }

    #[test]
    fn unpacks_device_ids() {
        assert_eq!(unpack_device_id(0x2882_10DE), Some(0x2882));
        assert_eq!(unpack_device_id(0x2882_1002), None);
        assert_eq!(unpack_device_id(0), None);
    }

    fn gpu(device_id: u16, bus: Option<u32>) -> Gpu {
        Gpu { device_id, subsys: 0x5161_1462, revision: 0xA1, bus, ..Default::default() }
    }

    fn adapter(device_id: u16) -> Adapter {
        Adapter {
            vendor_id: db::NVIDIA,
            device_id,
            subsys_vendor_id: 0x1462,
            subsys_id: 0x5161,
            revision_id: 0xA1,
            ..Default::default()
        }
    }

    #[test]
    fn picks_by_ids_and_bus() {
        let gpus = [gpu(0x2684, Some(1)), gpu(0x2882, Some(3)), gpu(0x2882, Some(4)), gpu(0x2882, None)];
        let free = [false; 4];
        let card = adapter(0x2882);
        assert_eq!(pick(&gpus, &free, &card, None), Some(1));
        assert_eq!(pick(&gpus, &free, &card, Some(4)), Some(2));
        assert_eq!(pick(&gpus, &free, &card, Some(9)), Some(3));
        assert_eq!(pick(&gpus[..3], &free, &card, Some(9)), None);
        assert_eq!(pick(&gpus, &free, &adapter(0x2204), None), None);
        assert_eq!(pick(&gpus, &free, &Adapter { subsys_vendor_id: 0x1043, ..adapter(0x2882) }, None), None);
        assert_eq!(pick(&gpus, &free, &Adapter { revision_id: 0xA2, ..adapter(0x2882) }, None), None);
        assert_eq!(pick(&gpus, &free, &Adapter { vendor_id: db::AMD, ..adapter(0x2882) }, None), None);
        assert_eq!(pick(&[], &[], &card, None), None);
    }

    #[test]
    fn identical_cards_are_handed_out_once() {
        let gpus = [gpu(0x2882, None), gpu(0x2882, None)];
        let card = adapter(0x2882);
        let mut taken = [false; 2];
        let first = pick(&gpus, &taken, &card, None).unwrap();
        taken[first] = true;
        let second = pick(&gpus, &taken, &card, None).unwrap();
        taken[second] = true;
        assert_eq!((first, second), (0, 1));
        assert_eq!(pick(&gpus, &taken, &card, None), None);
    }
}
