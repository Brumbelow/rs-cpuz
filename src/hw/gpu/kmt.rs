//! What the graphics kernel of Windows knows about an adapter.
//!
//! The `D3DKMT` entry points of gdi32 are ordinary user-mode calls that the operating
//! system answers itself. They report what is in effect for an adapter rather than what
//! its driver package was built for.

use std::mem;
use std::ptr;

use windows::Wdk::Graphics::Direct3D::{
    D3DKMTCloseAdapter, D3DKMTOpenAdapterFromLuid, D3DKMTQueryAdapterInfo, D3DKMT_ADAPTERADDRESS,
    D3DKMT_CLOSEADAPTER, D3DKMT_OPENADAPTERFROMLUID, D3DKMT_QUERYADAPTERINFO, KMTQAITYPE_ADAPTERADDRESS,
    KMTQAITYPE_DRIVERVERSION, KMTQUERYADAPTERINFOTYPE,
};
use windows::Win32::Foundation::LUID;

/// The first driver model the graphics kernel numbers this way, WDDM 1.0.
const WDDM_1_0: u32 = 1000;

/// Highest device and function number on a PCI bus.
const MAX_PCI_DEVICE: u32 = 31;
const MAX_PCI_FUNCTION: u32 = 7;

/// The kernel's view of one adapter. A field is `None` when the kernel did not answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Details {
    /// The driver model the adapter runs under: the WDDM version times 1000, so 2700 is WDDM 2.7.
    pub wddm_version: Option<u32>,
    /// PCI bus, device and function of the adapter.
    pub address: Option<(u8, u8, u8)>,
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
        let mut data = T::default();
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
    fn unknown_adapters_yield_nothing() {
        assert_eq!(query(LUID { LowPart: u32::MAX, HighPart: i32::MAX }), Details::default());
    }
}
