//! Device sets of SetupAPI and the device tree of the configuration manager.
//!
//! This is the information Device Manager shows. Reading it opens no device and
//! calls no driver.

use std::mem::size_of;
use std::ptr;

use windows::core::{Owned, GUID, HSTRING, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_Registry_PropertyW, CM_Get_Device_IDW, CM_Get_Parent, SetupDiEnumDeviceInfo,
    SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
    SetupDiGetDevicePropertyW, SetupDiGetDeviceRegistryPropertyW, SetupDiOpenDevRegKey, CM_DRP_ADDRESS,
    CM_DRP_BUSNUMBER, CM_DRP_ENUMERATOR_NAME, CR_SUCCESS, DICS_FLAG_GLOBAL, DIGCF_ALLCLASSES,
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, DIREG_DEV, HDEVINFO, SETUP_DI_GET_CLASS_DEVS_FLAGS,
    SETUP_DI_REGISTRY_PROPERTY, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA,
};
use windows::Win32::Devices::Properties::{DEVPROPKEY, DEVPROPTYPE, DEVPROP_TYPE_UINT32};
use windows::Win32::Foundation::{ERROR_SUCCESS, HWND};
use windows::Win32::System::Registry::{
    RegQueryValueExW, HKEY, KEY_READ, REG_DWORD, REG_MULTI_SZ, REG_SZ, REG_VALUE_TYPE,
};

/// Longest device instance id, in UTF-16 units, plus the terminating NUL.
const INSTANCE_ID_CAPACITY: usize = 201;

/// A device interface path longer than this many bytes is not believed.
const MAX_INTERFACE_DETAIL: u32 = 64 * 1024;

/// Levels of the device tree walked upwards before giving up.
const MAX_TREE_DEPTH: usize = 32;

/// A set of present devices, destroyed on drop.
pub struct DeviceSet(Owned<HDEVINFO>);

impl DeviceSet {
    /// The present devices a bus driver enumerated, e.g. those of `PCI` or `USB`.
    pub fn of_enumerator(enumerator: &str) -> Option<Self> {
        Self::open(None, Some(enumerator), DIGCF_PRESENT | DIGCF_ALLCLASSES)
    }

    /// The present devices of a device setup class.
    pub fn of_class(class: &GUID) -> Option<Self> {
        Self::open(Some(class), None, DIGCF_PRESENT)
    }

    fn open(
        guid: Option<&GUID>,
        enumerator: Option<&str>,
        flags: SETUP_DI_GET_CLASS_DEVS_FLAGS,
    ) -> Option<Self> {
        let enumerator = enumerator.map(HSTRING::from);
        let name = enumerator.as_ref().map_or(PCWSTR::null(), |name| PCWSTR(name.as_ptr()));
        // SAFETY: the GUID and the NUL-terminated enumerator name outlive the call, and the
        // returned set is owned by this value, which destroys it exactly once on drop.
        unsafe {
            let set = SetupDiGetClassDevsW(guid.map(ptr::from_ref), name, HWND::default(), flags);
            set.ok().map(|set| DeviceSet(Owned::new(set)))
        }
    }

    /// Walks the members of the set; ends at the first index SetupAPI rejects.
    pub fn devices(&self) -> impl Iterator<Item = Device<'_>> {
        (0u32..).map_while(move |index| {
            let mut data =
                SP_DEVINFO_DATA { cbSize: size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
            // SAFETY: the set is open for the lifetime of `self` and `data` is a properly
            // sized, writable SP_DEVINFO_DATA.
            unsafe { SetupDiEnumDeviceInfo(*self.0, index, &mut data) }
                .ok()
                .map(|()| Device { set: self, data })
        })
    }
}

/// The present device interfaces of one interface class.
pub struct InterfaceSet {
    set: DeviceSet,
    class: GUID,
}

impl InterfaceSet {
    pub fn open(class: &GUID) -> Option<Self> {
        let set = DeviceSet::open(Some(class), None, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE)?;
        Some(InterfaceSet { set, class: *class })
    }

    /// Walks the interfaces: the path each one is opened by, and the device that exposes it.
    /// An interface whose details cannot be read is skipped.
    pub fn interfaces(&self) -> impl Iterator<Item = (String, Device<'_>)> {
        (0u32..)
            .map_while(move |index| {
                let mut data = SP_DEVICE_INTERFACE_DATA {
                    cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                    ..Default::default()
                };
                // SAFETY: the set is open for the lifetime of `self`, the class GUID lives as
                // long and `data` is a properly sized, writable SP_DEVICE_INTERFACE_DATA.
                unsafe { SetupDiEnumDeviceInterfaces(*self.set.0, None, &self.class, index, &mut data) }
                    .ok()
                    .map(|()| self.detail(&data))
            })
            .flatten()
    }

    /// The path of one interface and its device.
    fn detail(&self, interface: &SP_DEVICE_INTERFACE_DATA) -> Option<(String, Device<'_>)> {
        let mut required = 0u32;
        // SAFETY: without a buffer the call only reports the size the details take. It
        // fails by design, so its result says nothing.
        let _ = unsafe {
            SetupDiGetDeviceInterfaceDetailW(*self.set.0, interface, None, 0, Some(&mut required), None)
        };
        let header = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
        if required < header || required > MAX_INTERFACE_DETAIL {
            return None;
        }
        // The details are a 32-bit size followed by the path, so a buffer of 32-bit
        // words is aligned for them and the path starts at the second word.
        let mut detail = vec![0u32; required.div_ceil(4) as usize];
        detail[0] = header;
        let mut device =
            SP_DEVINFO_DATA { cbSize: size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
        // SAFETY: `detail` is writable for the `required` bytes stated and starts with the
        // size SetupAPI checks; `device` is a properly sized SP_DEVINFO_DATA.
        unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                *self.set.0,
                interface,
                Some(detail.as_mut_ptr().cast()),
                required,
                None,
                Some(&mut device),
            )
        }
        .ok()?;
        let units: Vec<u16> = detail[1..]
            .iter()
            .flat_map(|word| [*word as u16, (*word >> 16) as u16])
            .take_while(|&unit| unit != 0)
            .collect();
        let path = String::from_utf16_lossy(&units);
        (!path.is_empty()).then_some((path, Device { set: &self.set, data: device }))
    }
}

/// One member of a [`DeviceSet`].
pub struct Device<'a> {
    set: &'a DeviceSet,
    data: SP_DEVINFO_DATA,
}

impl Device<'_> {
    /// The device's place in the device tree.
    pub fn node(&self) -> Node {
        Node(self.data.DevInst)
    }

    /// A string or string-list registry property; empty when missing.
    pub fn strings(&self, property: SETUP_DI_REGISTRY_PROPERTY) -> Vec<String> {
        match self.registry(property) {
            Some((REG_SZ | REG_MULTI_SZ, bytes)) => wide_strings(&bytes),
            _ => Vec::new(),
        }
    }

    pub fn dword(&self, property: SETUP_DI_REGISTRY_PROPERTY) -> Option<u32> {
        match self.registry(property)? {
            (REG_DWORD, bytes) => Some(u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?)),
            _ => None,
        }
    }

    /// Reads a registry property as its value type and raw bytes.
    fn registry(&self, property: SETUP_DI_REGISTRY_PROPERTY) -> Option<(REG_VALUE_TYPE, Vec<u8>)> {
        let mut buffer = vec![0u8; 1024];
        // One retry covers a value that did not fit the first buffer.
        for _ in 0..2 {
            let (mut kind, mut required) = (0u32, 0u32);
            // SAFETY: the set and device data are valid, and SetupAPI writes at most
            // `buffer.len()` bytes plus the two out values.
            let read = unsafe {
                SetupDiGetDeviceRegistryPropertyW(
                    *self.set.0,
                    &self.data,
                    property,
                    Some(&mut kind),
                    Some(&mut buffer),
                    Some(&mut required),
                )
            };
            let required = required as usize;
            if read.is_ok() {
                buffer.truncate(required.min(buffer.len()));
                return Some((REG_VALUE_TYPE(kind), buffer));
            }
            buffer.resize(grown_size(required, buffer.len())?, 0);
        }
        None
    }

    /// Reads a 32-bit unsigned device property; `None` when missing or of another type.
    pub fn property(&self, key: &DEVPROPKEY) -> Option<u32> {
        let mut kind = DEVPROPTYPE::default();
        let mut value = [0u8; 4];
        // SAFETY: the set and device data are valid, `key` and `kind` outlive the call and
        // SetupAPI writes at most the four bytes of `value`.
        unsafe {
            SetupDiGetDevicePropertyW(*self.set.0, &self.data, key, &mut kind, Some(&mut value), None, 0)
        }
        .ok()?;
        (kind == DEVPROP_TYPE_UINT32).then(|| u32::from_le_bytes(value))
    }

    /// Reads a value the device's driver keeps under the `Device Parameters` key of the
    /// device. `None` when it is missing, empty or larger than `limit` bytes.
    pub fn parameter(&self, name: &str, limit: u32) -> Option<Vec<u8>> {
        // SAFETY: the set and device data are valid. The key is opened for reading and
        // is ours to close, which `Owned` does.
        let key: Owned<HKEY> = unsafe {
            SetupDiOpenDevRegKey(*self.set.0, &self.data, DICS_FLAG_GLOBAL.0, 0, DIREG_DEV, KEY_READ.0)
                .ok()
                .map(|key| Owned::new(key))?
        };
        let name = HSTRING::from(name);
        let mut size = 0u32;
        // SAFETY: without a data buffer the call only reports the size of the value.
        let status = unsafe { RegQueryValueExW(*key, &name, None, None, None, Some(&mut size)) };
        if status != ERROR_SUCCESS || size == 0 || size > limit {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        // SAFETY: `data` is `size` bytes long, which is the capacity passed in `size`.
        let status =
            unsafe { RegQueryValueExW(*key, &name, None, None, Some(data.as_mut_ptr()), Some(&mut size)) };
        if status != ERROR_SUCCESS {
            return None;
        }
        data.truncate(size as usize);
        Some(data)
    }
}

/// A node of the device tree, as the configuration manager numbers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Node(u32);

impl Node {
    /// The node this one hangs off; `None` at the root of the tree.
    pub fn parent(self) -> Option<Node> {
        let mut parent = 0u32;
        // SAFETY: `parent` is a live out-parameter.
        let status = unsafe { CM_Get_Parent(&mut parent, self.0, 0) };
        (status == CR_SUCCESS).then_some(Node(parent))
    }

    /// The nodes above this one, nearest first.
    pub fn ancestors(self) -> impl Iterator<Item = Node> {
        std::iter::successors(self.parent(), |node| node.parent()).take(MAX_TREE_DEPTH)
    }

    /// The device instance id, which names the node across device sets.
    pub fn instance_id(self) -> Option<String> {
        let mut buffer = [0u16; INSTANCE_ID_CAPACITY];
        // SAFETY: the call writes at most `buffer.len()` UTF-16 units into `buffer`.
        let status = unsafe { CM_Get_Device_IDW(self.0, &mut buffer, 0) };
        if status != CR_SUCCESS {
            return None;
        }
        let end = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
        (end > 0).then(|| String::from_utf16_lossy(&buffer[..end]))
    }

    /// PCI bus number and address (`device << 16 | function`) of a node the PCI bus
    /// driver enumerated; `None` for a device of another bus.
    pub fn pci_address(self) -> Option<(u32, u32)> {
        let (kind, name) = self.registry(CM_DRP_ENUMERATOR_NAME)?;
        if kind != REG_SZ || wide_strings(&name) != ["PCI"] {
            return None;
        }
        Some((self.dword(CM_DRP_BUSNUMBER)?, self.dword(CM_DRP_ADDRESS)?))
    }

    fn dword(self, property: u32) -> Option<u32> {
        match self.registry(property)? {
            (REG_DWORD, bytes) => Some(u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?)),
            _ => None,
        }
    }

    /// Reads a registry property of the node as its value type and raw bytes.
    fn registry(self, property: u32) -> Option<(REG_VALUE_TYPE, Vec<u8>)> {
        let mut buffer = vec![0u8; 256];
        let (mut kind, mut length) = (0u32, buffer.len() as u32);
        // SAFETY: the configuration manager writes at most `length` bytes into `buffer`
        // and both out values are live.
        let status = unsafe {
            CM_Get_DevNode_Registry_PropertyW(
                self.0,
                property,
                Some(&mut kind),
                Some(buffer.as_mut_ptr().cast()),
                &mut length,
                0,
            )
        };
        if status != CR_SUCCESS {
            return None;
        }
        buffer.truncate((length as usize).min(buffer.len()));
        Some((REG_VALUE_TYPE(kind), buffer))
    }
}

/// Splits little-endian UTF-16 text into its NUL-separated strings, dropping empty ones.
pub fn wide_strings(bytes: &[u8]) -> Vec<String> {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
    units
        .split(|&unit| unit == 0)
        .map(|text| String::from_utf16_lossy(text).trim().to_string())
        .filter(|text| !text.is_empty())
        .collect()
}

/// Largest device property read, in bytes; the longest real ones (hardware id lists)
/// take a few hundred.
const MAX_PROPERTY: usize = 64 * 1024;

/// The size to grow a property buffer of `current` bytes to after SetupAPI asked for
/// `required`: `None` when that would not grow it, or would exceed [`MAX_PROPERTY`].
fn grown_size(required: usize, current: usize) -> Option<usize> {
    (required > current && required <= MAX_PROPERTY).then_some(required)
}

#[cfg(test)]
mod tests {
    use windows::Win32::Devices::DeviceAndDriverInstallation::{GUID_DEVCLASS_COMPUTER, SPDRP_CLASS};

    use super::*;

    fn wide(strings: &[&str]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for s in strings {
            bytes.extend(s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
        }
        bytes.extend([0, 0]);
        bytes
    }

    #[test]
    fn splits_wide_string_lists() {
        assert_eq!(wide_strings(&wide(&["PCI\\CC_0604", " second "])), ["PCI\\CC_0604", "second"]);
        assert_eq!(wide_strings(&wide(&["Single"])[..12]), ["Single"]);
        assert!(wide_strings(&[]).is_empty());
        assert!(wide_strings(&[0, 0, 0]).is_empty());
    }

    #[test]
    fn walks_the_computer_device() {
        // Every Windows installation has a device of the Computer class near the root
        // of the device tree, well above any bus.
        let set = DeviceSet::of_class(&GUID_DEVCLASS_COMPUTER).expect("the Computer class");
        let computer = set.devices().next().expect("the computer device");
        assert_eq!(computer.strings(SPDRP_CLASS), ["Computer"]);
        assert_eq!(computer.dword(SPDRP_CLASS), None);
        assert!(computer.node().instance_id().is_some_and(|id| id.contains('\\')));
        assert_eq!(computer.node().pci_address(), None);
        assert_eq!(computer.parameter("rs-cpuz-test-missing", 64), None);
        let above: Vec<_> = computer.node().ancestors().collect();
        assert!(!above.is_empty() && above.len() < MAX_TREE_DEPTH);
        assert!(above.iter().all(|node| node.pci_address().is_none()));
        assert_eq!(above.last().and_then(|root| root.parent()), None);
    }

    #[test]
    fn unknown_sets_are_empty() {
        assert_eq!(DeviceSet::of_enumerator("RS-CPUZ-TEST").map_or(0, |set| set.devices().count()), 0);
        // A made-up interface class has no interfaces.
        let class = GUID::from_u128(0x7273_6370_757a_4000_8000_0000_0000_0001);
        assert_eq!(InterfaceSet::open(&class).map_or(0, |set| set.interfaces().count()), 0);
    }

    #[test]
    fn property_buffers_grow_within_bounds() {
        assert_eq!(grown_size(600, 256), Some(600));
        // An answer that would not grow the buffer, or would grow it without limit, ends the reading.
        assert_eq!(grown_size(200, 256), None);
        assert_eq!(grown_size(MAX_PROPERTY + 1, 256), None);
        assert_eq!(grown_size(usize::MAX, 256), None);
    }
}
