//! The devices on the USB ports.
//!
//! The device tree has their names and the hub and port each one is plugged into. The
//! class codes of a device are not in the tree, so the hub is asked for the device
//! descriptor of what its port holds. The hub driver answers that from what it read
//! when the device was plugged in; nothing is sent to the device.

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SPDRP_ADDRESS, SPDRP_DEVICEDESC, SPDRP_FRIENDLYNAME, SPDRP_HARDWAREID,
};
use windows::Win32::Devices::Usb::{GUID_DEVINTERFACE_USB_HUB, IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX};

use super::ioctl::DeviceHandle;
use super::setup::{Device, DeviceSet, InterfaceSet};
use crate::model::UsbDevice;

/// Name of the bus driver whose devices are listed.
const ENUMERATOR: &str = "USB";

/// Size of a `USB_NODE_CONNECTION_INFORMATION_EX` without its list of pipes. The
/// structure is packed: the port number, the device descriptor, five fields on the
/// state of the device and the status of the connection.
const CONNECTION_LEN: usize = 35;
const CONNECTION_DESCRIPTOR: usize = 4;
const CONNECTION_STATUS: usize = 31;
/// `DeviceConnected` in a `USB_CONNECTION_STATUS`.
const STATUS_CONNECTED: u32 = 1;

/// Length and type a USB device descriptor starts with.
const DEVICE_DESCRIPTOR_LEN: u8 = 18;
const DEVICE_DESCRIPTOR_TYPE: u8 = 1;

/// Hubs below one another that are followed. USB itself stops at five.
const MAX_HUB_DEPTH: usize = 8;

/// One entry the USB bus drivers made in the device tree.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Entry {
    /// Device instance id.
    id: String,
    /// Instance id of what the entry hangs off: for a device, its hub.
    parent: Option<String>,
    /// Number of the hub port a device is plugged into.
    port: Option<u32>,
    name: String,
    /// Vendor and product id. `None` for what is not a device of its own: a root hub,
    /// or one function of a composite device.
    ids: Option<(u16, u16)>,
}

/// What a hub knows of the device on one of its ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Connection {
    class: u8,
    subclass: u8,
    vendor_id: u16,
    product_id: u16,
}

/// Lists the connected USB devices, hub by hub in port order, each hub followed by
/// the devices plugged into it.
pub fn detect() -> Vec<UsbDevice> {
    let Some(set) = DeviceSet::of_enumerator(ENUMERATOR) else {
        return Vec::new();
    };
    let entries: Vec<Entry> = set.devices().filter_map(|device| read(&device)).collect();
    let hubs: Vec<(String, Option<DeviceHandle>)> = InterfaceSet::open(&GUID_DEVINTERFACE_USB_HUB)
        .map(|set| {
            set.interfaces()
                .filter_map(|(path, hub)| Some((hub.node().instance_id()?, DeviceHandle::open(&path))))
                .collect()
        })
        .unwrap_or_default();
    let hub_ids: Vec<&str> = hubs.iter().map(|(id, _)| id.as_str()).collect();

    arrange(&entries, &hub_ids)
        .into_iter()
        .filter_map(|(entry, hub)| {
            let entry = &entries[entry];
            let hub = hub.and_then(|hub| hubs[hub].1.as_ref());
            let connection = hub.zip(entry.port).and_then(|(hub, port)| {
                let answer = hub.control(
                    IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX,
                    &connection_request(port),
                    CONNECTION_LEN,
                )?;
                parse_connection(&answer)
            });
            describe(entry, connection)
        })
        .collect()
}

/// Reads one entry of the device tree.
fn read(device: &Device<'_>) -> Option<Entry> {
    let node = device.node();
    Some(Entry {
        id: node.instance_id()?,
        parent: node.parent().and_then(|parent| parent.instance_id()),
        port: device.dword(SPDRP_ADDRESS),
        name: [SPDRP_DEVICEDESC, SPDRP_FRIENDLYNAME]
            .into_iter()
            .find_map(|property| device.strings(property).into_iter().next())
            .unwrap_or_default(),
        ids: usb_ids(&device.strings(SPDRP_HARDWAREID)),
    })
}

/// Vendor and product id of a device, from a hardware id of the form
/// `USB\VID_1D6B&PID_0104` or `USB\VID_1D6B&PID_0104&REV_0100`. The ids of a function
/// of a composite device carry further fields and do not count.
fn usb_ids<S: AsRef<str>>(hardware_ids: &[S]) -> Option<(u16, u16)> {
    hardware_ids.iter().find_map(|id| {
        let (bus, fields) = id.as_ref().split_once('\\')?;
        if !bus.eq_ignore_ascii_case(ENUMERATOR) {
            return None;
        }
        let mut fields = fields.split('&');
        let vendor = hex_field(fields.next()?, "VID_")?;
        let product = hex_field(fields.next()?, "PID_")?;
        let rest_is_revision = match (fields.next(), fields.next()) {
            (None, _) => true,
            (Some(revision), None) => hex_field(revision, "REV_").is_some(),
            _ => false,
        };
        rest_is_revision.then_some((vendor, product))
    })
}

/// The four hexadecimal digits that follow `prefix` in a hardware id field.
fn hex_field(field: &str, prefix: &str) -> Option<u16> {
    let digits = field
        .get(prefix.len()..)
        .filter(|_| field.get(..prefix.len()).is_some_and(|start| start.eq_ignore_ascii_case(prefix)))?;
    (digits.len() == 4 && digits.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| u16::from_str_radix(digits, 16).ok())
        .flatten()
}

/// Puts the devices in the order of the ports they are plugged into, hub by hub, with
/// the devices on a hub right after the hub. `hubs` holds the instance ids of the hubs;
/// the result pairs indices into `entries` with the index of each device's hub. Devices
/// whose hub is not among `hubs` follow at the end, in the order they were found.
fn arrange(entries: &[Entry], hubs: &[&str]) -> Vec<(usize, Option<usize>)> {
    let hub_of = |entry: &Entry| {
        let parent = entry.parent.as_deref()?;
        hubs.iter().position(|hub| hub.eq_ignore_ascii_case(parent))
    };
    let mut placed = vec![false; entries.len()];
    let mut order = Vec::new();
    for (index, hub) in hubs.iter().enumerate() {
        // A hub that is a device on another hub is reached through that one.
        let plugged_in = entries.iter().any(|entry| {
            entry.ids.is_some() && entry.id.eq_ignore_ascii_case(hub) && hub_of(entry).is_some()
        });
        if !plugged_in {
            descend(entries, hubs, index, MAX_HUB_DEPTH, &mut placed, &mut order);
        }
    }
    for (index, entry) in entries.iter().enumerate() {
        if entry.ids.is_some() && !placed[index] {
            order.push((index, hub_of(entry)));
        }
    }
    order
}

/// Appends the devices on hub `hub` in port order, following hubs among them at once.
fn descend(
    entries: &[Entry],
    hubs: &[&str],
    hub: usize,
    depth: usize,
    placed: &mut [bool],
    order: &mut Vec<(usize, Option<usize>)>,
) {
    if depth == 0 {
        return;
    }
    let mut ports: Vec<usize> = (0..entries.len())
        .filter(|&index| {
            let entry = &entries[index];
            entry.ids.is_some()
                && !placed[index]
                && entry.parent.as_deref().is_some_and(|parent| parent.eq_ignore_ascii_case(hubs[hub]))
        })
        .collect();
    ports.sort_by_key(|&index| entries[index].port);
    for index in ports {
        placed[index] = true;
        order.push((index, Some(hub)));
        if let Some(below) = hubs.iter().position(|id| id.eq_ignore_ascii_case(&entries[index].id)) {
            descend(entries, hubs, below, depth - 1, placed, order);
        }
    }
}

/// The request for the connection on a hub port: a `USB_NODE_CONNECTION_INFORMATION_EX`
/// that is blank but for the port number.
fn connection_request(port: u32) -> [u8; CONNECTION_LEN] {
    let mut request = [0u8; CONNECTION_LEN];
    request[..4].copy_from_slice(&port.to_le_bytes());
    request
}

/// Decodes the answer to a connection request. `None` unless a device is connected
/// and the hub holds its device descriptor.
fn parse_connection(bytes: &[u8]) -> Option<Connection> {
    let info: &[u8; CONNECTION_LEN] = bytes.get(..CONNECTION_LEN)?.try_into().ok()?;
    let status = u32::from_le_bytes(info[CONNECTION_STATUS..].try_into().ok()?);
    let descriptor = &info[CONNECTION_DESCRIPTOR..];
    if status != STATUS_CONNECTED
        || descriptor[0] != DEVICE_DESCRIPTOR_LEN
        || descriptor[1] != DEVICE_DESCRIPTOR_TYPE
    {
        return None;
    }
    Some(Connection {
        class: descriptor[4],
        subclass: descriptor[5],
        vendor_id: u16::from_le_bytes([descriptor[8], descriptor[9]]),
        product_id: u16::from_le_bytes([descriptor[10], descriptor[11]]),
    })
}

/// The report entry for a device. A descriptor with other ids than the device tree has
/// belongs to something else, so its class codes are not used.
fn describe(entry: &Entry, connection: Option<Connection>) -> Option<UsbDevice> {
    let (vendor_id, product_id) = entry.ids?;
    let class = connection
        .filter(|found| (found.vendor_id, found.product_id) == (vendor_id, product_id))
        .map(|found| (found.class, found.subclass));
    Some(UsbDevice { name: entry.name.clone(), class, vendor_id, product_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, parent: &str, port: u32, ids: Option<(u16, u16)>) -> Entry {
        Entry { id: id.into(), parent: Some(parent.into()), port: Some(port), name: id.into(), ids }
    }

    /// The answer of a hub for a port: `status` and a device descriptor with the given
    /// class, subclass, vendor id and product id.
    fn connection(status: u32, class: u8, subclass: u8, vendor_id: u16, product_id: u16) -> Vec<u8> {
        let mut info = vec![0u8; CONNECTION_LEN];
        info[0] = 3;
        let descriptor = &mut info[CONNECTION_DESCRIPTOR..];
        descriptor[0] = DEVICE_DESCRIPTOR_LEN;
        descriptor[1] = DEVICE_DESCRIPTOR_TYPE;
        descriptor[4] = class;
        descriptor[5] = subclass;
        descriptor[8..10].copy_from_slice(&vendor_id.to_le_bytes());
        descriptor[10..12].copy_from_slice(&product_id.to_le_bytes());
        info[CONNECTION_STATUS..].copy_from_slice(&status.to_le_bytes());
        info
    }

    #[test]
    fn reads_ids_of_whole_devices_only() {
        assert_eq!(
            usb_ids(&["USB\\VID_1D6B&PID_0104&REV_0100", "USB\\VID_1D6B&PID_0104"]),
            Some((0x1D6B, 0x0104))
        );
        assert_eq!(usb_ids(&["usb\\vid_1d6b&pid_0104"]), Some((0x1D6B, 0x0104)));
        // One function of a composite device, and a child a function driver made up.
        assert_eq!(usb_ids(&["USB\\VID_1D6B&PID_0104&REV_0100&MI_02", "USB\\VID_1D6B&PID_0104&MI_02"]), None);
        assert_eq!(usb_ids(&["USB\\VID_1D6B&PID_0104&EXAMPLE"]), None);
        assert_eq!(usb_ids(&["USB\\ROOT_HUB30&VID1D6B&PID0003&REV0100", "USB\\ROOT_HUB30"]), None);
        assert_eq!(usb_ids(&["USB\\VID_1D6&PID_0104", "USB\\VID_1D6BX&PID_0104", "USB\\VID_1D6B"]), None);
        assert_eq!(usb_ids(&["HID\\VID_1D6B&PID_0104", "PCI\\VEN_1022&DEV_15B8"]), None);
        assert_eq!(usb_ids::<&str>(&[]), None);
    }

    #[test]
    fn decodes_connections() {
        assert_eq!(
            parse_connection(&connection(STATUS_CONNECTED, 0xEF, 0x02, 0x1D6B, 0x0104)),
            Some(Connection { class: 0xEF, subclass: 0x02, vendor_id: 0x1D6B, product_id: 0x0104 })
        );
        // A hub appends the pipes of the device; they are of no interest.
        let mut with_pipes = connection(STATUS_CONNECTED, 0x09, 0x00, 0x1D6B, 0x0002);
        with_pipes.extend([0xAA; 24]);
        assert_eq!(parse_connection(&with_pipes).map(|found| found.class), Some(0x09));

        // An empty port, and a device that failed to enumerate.
        assert_eq!(parse_connection(&connection(0, 0, 0, 0, 0)), None);
        assert_eq!(parse_connection(&connection(2, 0xEF, 0x02, 0x1D6B, 0x0104)), None);
        // Connected, but the descriptor was never read.
        assert_eq!(parse_connection(&connection_request(3)), None);
        let mut blank = connection(STATUS_CONNECTED, 0, 0, 0, 0);
        blank[CONNECTION_DESCRIPTOR] = 0;
        assert_eq!(parse_connection(&blank), None);
        assert_eq!(parse_connection(&connection(STATUS_CONNECTED, 0xEF, 0x02, 0x1D6B, 0x0104)[..34]), None);
        assert_eq!(parse_connection(&[]), None);
    }

    #[test]
    fn asks_for_a_port_by_number() {
        let request = connection_request(0x0102);
        assert_eq!(request[..4], [0x02, 0x01, 0x00, 0x00]);
        assert!(request[4..].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn lists_devices_hub_by_hub() {
        let keyboard = Some((0x1D6B, 0x0101));
        let entries = [
            entry("USB\\VID_1D6B&PID_0105\\A", "USB\\ROOT_HUB30\\2", 2, Some((0x1D6B, 0x0105))),
            entry("USB\\ROOT_HUB30\\1", "PCI\\VEN_1022&DEV_15B8\\0", 0, None),
            entry("USB\\VID_1D6B&PID_0104\\A", "USB\\ROOT_HUB30\\1", 9, Some((0x1D6B, 0x0104))),
            // A hub on port 4 of the first root hub with two devices of its own.
            entry("USB\\VID_1D6B&PID_0102\\B", "USB\\VID_1D6B&PID_0002\\A", 3, Some((0x1D6B, 0x0102))),
            entry("USB\\VID_1D6B&PID_0002\\A", "USB\\ROOT_HUB30\\1", 4, Some((0x1D6B, 0x0002))),
            entry("USB\\VID_1D6B&PID_0101\\C", "usb\\vid_1d6b&pid_0002\\a", 1, keyboard),
            // A function of the composite device on port 9 is not a device.
            entry("USB\\VID_1D6B&PID_0104&MI_00\\B", "USB\\VID_1D6B&PID_0104\\A", 0, None),
            entry("USB\\ROOT_HUB30\\2", "PCI\\VEN_1022&DEV_15B7\\0", 0, None),
            entry("USB\\VID_1D6B&PID_0103\\A", "USB\\ROOT_HUB30\\1", 2, Some((0x1D6B, 0x0103))),
        ];
        let hubs = ["USB\\VID_1D6B&PID_0002\\A", "USB\\ROOT_HUB30\\1", "USB\\ROOT_HUB30\\2"];
        let listed: Vec<(u16, Option<usize>)> = arrange(&entries, &hubs)
            .into_iter()
            .map(|(index, hub)| (entries[index].ids.unwrap().1, hub))
            .collect();
        assert_eq!(
            listed,
            [
                (0x0103, Some(1)),
                (0x0002, Some(1)),
                (0x0101, Some(0)),
                (0x0102, Some(0)),
                (0x0104, Some(1)),
                (0x0105, Some(2)),
            ]
        );
    }

    #[test]
    fn keeps_devices_without_a_known_hub() {
        let entries = [
            entry("USB\\VID_1D6B&PID_0101\\A", "USB\\ROOT_HUB30\\9", 1, Some((0x1D6B, 0x0101))),
            entry("USB\\VID_1D6B&PID_0102\\A", "USB\\ROOT_HUB30\\1", 1, Some((0x1D6B, 0x0102))),
            Entry { ids: Some((0x1D6B, 0x0103)), ..Default::default() },
        ];
        assert_eq!(arrange(&entries, &["USB\\ROOT_HUB30\\1"]), [(1, Some(0)), (0, None), (2, None)]);
        assert_eq!(arrange(&entries, &[]), [(0, None), (1, None), (2, None)]);
        assert!(arrange(&[], &["USB\\ROOT_HUB30\\1"]).is_empty());
    }

    #[test]
    fn a_loop_in_the_tree_ends() {
        // Two hubs that claim to be plugged into one another are not reachable from a
        // root hub, and following them stops.
        let entries = [
            entry("USB\\VID_1D6B&PID_0002\\A", "USB\\VID_1D6B&PID_0002\\B", 1, Some((0x1D6B, 0x0002))),
            entry("USB\\VID_1D6B&PID_0002\\B", "USB\\VID_1D6B&PID_0002\\A", 1, Some((0x1D6B, 0x0002))),
        ];
        let hubs = ["USB\\VID_1D6B&PID_0002\\A", "USB\\VID_1D6B&PID_0002\\B"];
        assert_eq!(arrange(&entries, &hubs), [(0, Some(1)), (1, Some(0))]);
        let mut placed = [false; 2];
        let mut order = Vec::new();
        descend(&entries, &hubs, 0, MAX_HUB_DEPTH, &mut placed, &mut order);
        assert_eq!(order, [(1, Some(0)), (0, Some(1))]);
    }

    #[test]
    fn describes_devices() {
        let device =
            Entry { name: "USB Composite Device".into(), ids: Some((0x1D6B, 0x0104)), ..Default::default() };
        let found = |vendor_id, product_id| Connection { class: 0xEF, subclass: 0x02, vendor_id, product_id };
        assert_eq!(
            describe(&device, Some(found(0x1D6B, 0x0104))),
            Some(UsbDevice {
                name: "USB Composite Device".into(),
                class: Some((0xEF, 0x02)),
                vendor_id: 0x1D6B,
                product_id: 0x0104,
            })
        );
        // The port holds something else, or the hub did not answer.
        assert_eq!(describe(&device, Some(found(0x1D6B, 0x0105))).unwrap().class, None);
        assert_eq!(describe(&device, None).unwrap().class, None);
        assert_eq!(describe(&Entry::default(), Some(found(0, 0))), None);
    }

    #[test]
    fn detection_is_self_consistent() {
        for device in detect() {
            assert_ne!((device.vendor_id, device.product_id), (0, 0));
        }
    }
}
