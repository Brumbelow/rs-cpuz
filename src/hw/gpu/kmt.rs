//! What the graphics kernel of Windows knows about an adapter.
//!
//! The `D3DKMT` entry points of gdi32 are ordinary user-mode calls that the operating
//! system answers itself. They report what is in effect for an adapter rather than what
//! its driver package was built for.

use std::mem;
use std::ptr;

use windows::Wdk::Graphics::Direct3D::{
    D3DKMTCloseAdapter, D3DKMTOpenAdapterFromLuid, D3DKMTQueryAdapterInfo, D3DKMT_ADAPTERADDRESS,
    D3DKMT_ADAPTER_PERFDATA, D3DKMT_ADAPTER_PERFDATACAPS, D3DKMT_CLOSEADAPTER, D3DKMT_NODEMETADATA,
    D3DKMT_NODE_PERFDATA, D3DKMT_OPENADAPTERFROMLUID, D3DKMT_QUERYADAPTERINFO, DXGK_ENGINE_TYPE_3D,
    KMTQAITYPE_ADAPTERADDRESS, KMTQAITYPE_ADAPTERPERFDATA, KMTQAITYPE_ADAPTERPERFDATA_CAPS,
    KMTQAITYPE_DRIVERVERSION, KMTQAITYPE_NODEMETADATA, KMTQAITYPE_NODEPERFDATA, KMTQUERYADAPTERINFOTYPE,
};
use windows::Win32::Foundation::LUID;

/// The first driver model the graphics kernel numbers this way, WDDM 1.0.
const WDDM_1_0: u32 = 1000;

/// Highest device and function number on a PCI bus.
const MAX_PCI_DEVICE: u32 = 31;
const MAX_PCI_FUNCTION: u32 = 7;

/// Engines (nodes) looked at when searching for the 3D engine.
const MAX_NODES: u32 = 64;
/// Temperatures above this, in tenths of a degree Celsius, are no measurement.
const MAX_DECI_CELSIUS: u32 = 1_500;

/// The kernel's view of one adapter. A field is `None` when the kernel did not answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Details {
    /// The driver model the adapter runs under: the WDDM version times 1000, so 2700 is WDDM 2.7.
    pub wddm_version: Option<u32>,
    /// PCI bus, device and function of the adapter.
    pub address: Option<(u8, u8, u8)>,
}

/// What the driver reports of an adapter's clocks, temperature and fan through the
/// graphics kernel (WDDM 2.4 and later, the figures Task Manager shows). Each value is
/// left out when the driver does not fill it in or fills in something impossible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Performance {
    /// Clock of the 3D engine now, and the highest it may reach, in Hz.
    pub core_hz: Option<u64>,
    pub max_core_hz: Option<u64>,
    /// Memory clock now, and the highest it may reach, in Hz.
    pub memory_hz: Option<u64>,
    pub max_memory_hz: Option<u64>,
    /// Temperature in tenths of a degree Celsius.
    pub deci_celsius: Option<u32>,
    /// Fan speed; only reported when the driver states the fan's top speed.
    pub fan_rpm: Option<u32>,
}

impl Performance {
    /// Builds the readings from the adapter's performance data (memory clock, highest
    /// memory clock, temperature, fan), its capabilities (top fan speed, temperature
    /// limit) and the 3D engine's clock and highest clock.
    fn decode(
        adapter: Option<(u64, u64, u32, u32)>,
        caps: Option<(u32, u32)>,
        engine: Option<(u64, u64)>,
    ) -> Self {
        let set = |hz: u64| (hz != 0).then_some(hz);
        let (memory, max_memory, deci_celsius, fan) = adapter.unwrap_or_default();
        let (max_fan, _) = caps.unwrap_or_default();
        let (core, max_core) = engine.unwrap_or_default();
        let at_most = |now: u64, max: u64| set(now).filter(|&now| max == 0 || now <= max);
        Performance {
            core_hz: at_most(core, max_core),
            max_core_hz: set(max_core),
            memory_hz: at_most(memory, max_memory),
            max_memory_hz: set(max_memory),
            deci_celsius: (deci_celsius != 0 && deci_celsius <= MAX_DECI_CELSIUS).then_some(deci_celsius),
            fan_rpm: (max_fan != 0 && fan <= max_fan).then_some(fan),
        }
    }
}

/// Asks the graphics kernel for the clocks, temperature and fan speed of the adapter
/// DXGI identifies by `luid`.
pub fn performance(luid: LUID) -> Performance {
    let Some(adapter) = Handle::open(luid) else {
        return Performance::default();
    };
    // SAFETY: the kernel answers these requests with the plain structures named; the
    // first physical adapter (index 0) is the one asked about.
    let (data, caps) = unsafe {
        (
            adapter.info::<D3DKMT_ADAPTER_PERFDATA>(KMTQAITYPE_ADAPTERPERFDATA),
            adapter.info::<D3DKMT_ADAPTER_PERFDATACAPS>(KMTQAITYPE_ADAPTERPERFDATA_CAPS),
        )
    };
    let engine = (0..MAX_NODES).map_while(|node| adapter.node_type(node).map(|kind| (node, kind))).find_map(
        |(node, kind)| {
            if kind != DXGK_ENGINE_TYPE_3D.0 {
                return None;
            }
            let request = D3DKMT_NODE_PERFDATA { NodeOrdinal: node, ..Default::default() };
            // SAFETY: the node performance data is a plain structure; the ordinal is its input.
            let data = unsafe { adapter.info_with(KMTQAITYPE_NODEPERFDATA, request) }?;
            Some((data.Frequency, data.MaxFrequency))
        },
    );
    Performance::decode(
        data.map(|data| (data.MemoryFrequency, data.MaxMemoryFrequency, data.Temperature, data.FanRPM)),
        caps.map(|caps| (caps.MaxFanRPM, caps.TemperatureMax)),
        engine,
    )
}

/// Asks the graphics kernel about the adapter DXGI identifies by `luid`.
pub fn query(luid: LUID) -> Details {
    let Some(adapter) = Handle::open(luid) else {
        return Details::default();
    };
    // SAFETY: the kernel answers these two requests with a 32-bit number and a
    // `D3DKMT_ADAPTERADDRESS`, both plain data.
    let (version, address) = unsafe {
        (
            adapter.info::<u32>(KMTQAITYPE_DRIVERVERSION),
            adapter.info::<D3DKMT_ADAPTERADDRESS>(KMTQAITYPE_ADAPTERADDRESS),
        )
    };
    Details {
        wddm_version: version.filter(|&version| version >= WDDM_1_0),
        address: address.and_then(|at| pci_address(at.BusNumber, at.DeviceNumber, at.FunctionNumber)),
    }
}

/// The address as PCI numbers it. For an adapter that is not a PCI device the kernel
/// reports success with every field at its maximum, so the ranges are what tells.
fn pci_address(bus: u32, device: u32, function: u32) -> Option<(u8, u8, u8)> {
    if device > MAX_PCI_DEVICE || function > MAX_PCI_FUNCTION {
        return None;
    }
    Some((u8::try_from(bus).ok()?, device as u8, function as u8))
}

/// An adapter opened in the graphics kernel; closed again on drop.
struct Handle(u32);

impl Handle {
    fn open(luid: LUID) -> Option<Handle> {
        let mut request = D3DKMT_OPENADAPTERFROMLUID { AdapterLuid: luid, hAdapter: 0 };
        // SAFETY: `request` is a live structure of the type the call fills in.
        let status = unsafe { D3DKMTOpenAdapterFromLuid(&mut request) };
        status.is_ok().then_some(Handle(request.hAdapter))
    }

    /// Reads one fixed-size piece of information about the adapter.
    ///
    /// # Safety
    /// `T` must be the plain-data type the kernel returns for `kind`, valid for any bit pattern.
    unsafe fn info<T: Copy + Default>(&self, kind: KMTQUERYADAPTERINFOTYPE) -> Option<T> {
        // SAFETY: as for this function.
        unsafe { self.info_with(kind, T::default()) }
    }

    /// Like [`Handle::info`], for a request whose structure carries input fields; `data`
    /// comes with them filled in.
    ///
    /// # Safety
    /// As for [`Handle::info`].
    unsafe fn info_with<T: Copy>(&self, kind: KMTQUERYADAPTERINFOTYPE, mut data: T) -> Option<T> {
        let mut request = D3DKMT_QUERYADAPTERINFO {
            hAdapter: self.0,
            Type: kind,
            pPrivateDriverData: ptr::from_mut(&mut data).cast(),
            PrivateDriverDataSize: mem::size_of::<T>() as u32,
        };
        // SAFETY: the handle is open and the kernel writes at most the stated size to
        // `data`, which outlives the call.
        let status = unsafe { D3DKMTQueryAdapterInfo(&mut request) };
        status.is_ok().then_some(data)
    }
}

impl Handle {
    /// The engine type of node `node` of the first physical adapter; `None` past the last node.
    fn node_type(&self, node: u32) -> Option<i32> {
        let request = D3DKMT_NODEMETADATA { NodeOrdinalAndAdapterIndex: node, ..Default::default() };
        // SAFETY: the node metadata is a plain structure; the ordinal is its input.
        let metadata = unsafe { self.info_with(KMTQAITYPE_NODEMETADATA, request) }?;
        // The structure is packed: copy the field out before looking at it.
        let data = metadata.NodeData;
        let kind = data.EngineType;
        Some(kind.0)
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let request = D3DKMT_CLOSEADAPTER { hAdapter: self.0 };
        // SAFETY: the handle came from a successful open and is closed exactly once.
        // Nothing can be done about a failure.
        let _ = unsafe { D3DKMTCloseAdapter(&request) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_pci_addresses_only() {
        assert_eq!(pci_address(1, 0, 0), Some((1, 0, 0)));
        assert_eq!(pci_address(255, 31, 7), Some((255, 31, 7)));
        // What the kernel reports for an adapter that is not on the PCI bus.
        assert_eq!(pci_address(0xFFFF_FFFF, 0xFFFF, 0xFFFF), None);
        assert_eq!(pci_address(256, 0, 0), None);
        assert_eq!(pci_address(0, 32, 0), None);
        assert_eq!(pci_address(0, 0, 8), None);
    }

    #[test]
    fn reads_what_a_discrete_card_reports() {
        // The values an RTX 4060 Ti gave at idle: memory and 3D engine clocks in Hz, the
        // temperature in tenths of a degree, and a fan that stands still.
        let performance = Performance::decode(
            Some((405_000_000, 9_001_000_000, 364, 0)),
            Some((3_800, 900)),
            Some((210_000_000, 3_105_000_000)),
        );
        assert_eq!(performance.core_hz, Some(210_000_000));
        assert_eq!(performance.max_core_hz, Some(3_105_000_000));
        assert_eq!(performance.memory_hz, Some(405_000_000));
        assert_eq!(performance.max_memory_hz, Some(9_001_000_000));
        assert_eq!(performance.deci_celsius, Some(364));
        // The driver states a top fan speed, so the standing fan is a reading of zero.
        assert_eq!(performance.fan_rpm, Some(0));
    }

    #[test]
    fn leaves_out_what_a_driver_gets_wrong_or_does_not_fill() {
        // The values the UHD Graphics 770 driver gave: nothing for the adapter, and a 3D
        // engine "running" far above its own maximum.
        let performance =
            Performance::decode(Some((0, 0, 0, 0)), Some((0, 0)), Some((8_550_000_000, 1_600_000_000)));
        assert_eq!(performance.core_hz, None);
        assert_eq!(performance.max_core_hz, Some(1_600_000_000));
        assert_eq!((performance.memory_hz, performance.max_memory_hz), (None, None));
        assert_eq!((performance.deci_celsius, performance.fan_rpm), (None, None));
        assert_eq!(Performance::decode(None, None, None), Performance::default());
    }

    #[test]
    fn unknown_adapters_yield_nothing() {
        assert_eq!(query(LUID { LowPart: u32::MAX, HighPart: i32::MAX }), Details::default());
    }
}
