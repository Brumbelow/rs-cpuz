//! Drives and the volumes on them.
//!
//! Every present disk is opened through its device interface without read or write
//! access, which takes no elevation, and asked the property queries Windows documents
//! for storage devices. The device tree leads from a disk to the controller it is
//! attached to, and the mounted volumes say which disks they lie on.

use windows::core::HSTRING;
use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
};
use windows::Win32::System::Diagnostics::Debug::{
    SetThreadErrorMode, SEM_FAILCRITICALERRORS, THREAD_ERROR_MODE,
};
use windows::Win32::System::Ioctl::{
    AtaDataTypeIdentify, NVMeDataTypeIdentify, NVMeDataTypeLogPage, PropertyStandardQuery, ProtocolTypeAta,
    ProtocolTypeNvme, StorageAdapterProtocolSpecificProperty, StorageDeviceProperty,
    StorageDeviceProtocolSpecificProperty, StorageDeviceSeekPenaltyProperty, GUID_DEVINTERFACE_DISK,
    IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_STORAGE_GET_DEVICE_NUMBER, IOCTL_STORAGE_PREDICT_FAILURE,
    IOCTL_STORAGE_QUERY_PROPERTY, STORAGE_PROPERTY_ID,
};
use windows::Win32::System::WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE};

use super::ioctl::DeviceHandle;
use super::pci;
use super::setup::{InterfaceSet, Node};
use crate::model::{DriveController, DriveInfo, PciDevice, VolumeInfo};

/// Bus types (`STORAGE_BUS_TYPE`) whose drives answer queries of their own protocol.
const BUS_ATA: u32 = 3;
const BUS_SATA: u32 = 11;
const BUS_NVME: u32 = 17;

/// `FILE_DEVICE_DISK`, the device type of a disk in a `STORAGE_DEVICE_NUMBER`.
const DEVICE_TYPE_DISK: u32 = 7;

/// The feature a drive with failure prediction is listed with.
const FEATURE_SMART: &str = "SMART";

/// Size of the fixed part of a `STORAGE_DEVICE_DESCRIPTOR`; its strings follow.
const DEVICE_DESCRIPTOR_LEN: usize = 36;
/// Room for a device descriptor with its strings and raw properties.
const DEVICE_DESCRIPTOR_CAPACITY: usize = 4096;
/// Size of a `DEVICE_SEEK_PENALTY_DESCRIPTOR`.
const SEEK_PENALTY_LEN: usize = 12;
/// Size of a `STORAGE_DEVICE_NUMBER`.
const DEVICE_NUMBER_LEN: usize = 12;
/// Room for a `DISK_GEOMETRY_EX` with the partition and detection data that trail it.
const GEOMETRY_CAPACITY: usize = 256;
/// Offset of the disk size in a `DISK_GEOMETRY_EX`, right after the `DISK_GEOMETRY`.
const GEOMETRY_DISK_SIZE: usize = 24;
/// Size of a `STORAGE_PREDICT_FAILURE`.
const PREDICT_FAILURE_LEN: usize = 516;

/// Layout of a `VOLUME_DISK_EXTENTS`: a count, then one `DISK_EXTENT` per extent
/// that starts with the number of its disk.
const EXTENTS_HEADER_LEN: usize = 8;
const EXTENT_LEN: usize = 24;
/// More extents than a volume has in practice.
const MAX_EXTENTS: usize = 64;

/// Offset of the additional parameters in a `STORAGE_PROPERTY_QUERY`, and of the
/// protocol-specific part in the `STORAGE_PROTOCOL_DATA_DESCRIPTOR` that answers it.
const QUERY_HEADER_LEN: usize = 8;
/// Size of a `STORAGE_PROTOCOL_SPECIFIC_DATA`, and where it states the offset and the
/// length of the data that follows it.
const PROTOCOL_DATA_LEN: usize = 40;
const PROTOCOL_DATA_OFFSET_FIELD: usize = 16;
const PROTOCOL_DATA_LENGTH_FIELD: usize = 20;

/// `NVME_IDENTIFY_CNS_CONTROLLER` and the size of the structure it returns.
const NVME_IDENTIFY_CONTROLLER: u32 = 1;
const NVME_IDENTIFY_LEN: usize = 4096;
/// `NVME_LOG_PAGE_HEALTH_INFO`, the SMART log of an NVMe drive, and its size.
const NVME_LOG_HEALTH: u32 = 2;
const NVME_HEALTH_LEN: usize = 512;
/// The serial number field of the NVMe controller identification.
const NVME_SERIAL: std::ops::Range<usize> = 4..24;

/// Size of the ATA `IDENTIFY DEVICE` data, 256 words.
const ATA_IDENTIFY_LEN: usize = 512;
/// The words of it that list the supported command sets.
const ATA_COMMAND_SETS_1: usize = 82;
const ATA_COMMAND_SETS_2: usize = 83;

/// Name of a storage bus as Windows numbers them (`STORAGE_BUS_TYPE`).
pub fn bus_name(code: u32) -> Option<&'static str> {
    Some(match code {
        1 => "SCSI",
        2 => "ATAPI",
        3 => "ATA",
        4 => "IEEE 1394",
        5 => "SSA",
        6 => "Fibre Channel",
        7 => "USB",
        8 => "RAID",
        9 => "iSCSI",
        10 => "SAS",
        11 => "SATA",
        12 => "SD",
        13 => "MMC",
        14 => "Virtual",
        15 => "File Backed Virtual",
        16 => "Storage Spaces",
        17 => "NVMe",
        18 => "SCM",
        19 => "UFS",
        _ => return None,
    })
}

/// Lists the drives in the order of their numbers. `pci` names the controllers; without
/// it the drives are listed without one.
pub fn detect(pci: &[PciDevice]) -> Vec<DriveInfo> {
    let Some(disks) = InterfaceSet::open(&GUID_DEVINTERFACE_DISK) else {
        return Vec::new();
    };
    let volumes = volumes();
    let mut drives: Vec<DriveInfo> = disks
        .interfaces()
        .filter_map(|(path, disk)| {
            let mut drive = query(&path)?;
            drive.controller = attachment(disk.node())
                .and_then(|(bus, address)| controller(pci, bus, address, drive.bus_type == Some(BUS_NVME)));
            drive.volumes = on_disk(&volumes, drive.number);
            Some(drive)
        })
        .collect();
    drives.sort_by_key(|drive| drive.number);
    drives
}

/// Asks one disk about itself. `None` when it cannot be opened or has no drive number.
fn query(path: &str) -> Option<DriveInfo> {
    let disk = DeviceHandle::open(path)?;
    let number = disk
        .control(IOCTL_STORAGE_GET_DEVICE_NUMBER, &[], DEVICE_NUMBER_LEN)
        .and_then(|answer| parse_device_number(&answer))?;
    let property = |id: STORAGE_PROPERTY_ID, capacity: usize| {
        disk.control(IOCTL_STORAGE_QUERY_PROPERTY, &property_query(id), capacity)
    };
    let descriptor = property(StorageDeviceProperty, DEVICE_DESCRIPTOR_CAPACITY)
        .and_then(|answer| parse_device_descriptor(&answer))
        .unwrap_or_default();
    // For an NVMe drive Windows passes off a generated identifier as the serial number;
    // the one the manufacturer gave it is in the controller identification.
    let nvme_serial = (descriptor.bus_type == Some(BUS_NVME))
        .then(|| {
            let identify = ProtocolQuery {
                property: StorageAdapterProtocolSpecificProperty,
                protocol: ProtocolTypeNvme.0 as u32,
                data_type: NVMeDataTypeIdentify.0 as u32,
                request: NVME_IDENTIFY_CONTROLLER,
                length: NVME_IDENTIFY_LEN,
            };
            identify.send(&disk).and_then(|data| text(data.get(NVME_SERIAL)?))
        })
        .flatten();
    let no_seek_penalty = property(StorageDeviceSeekPenaltyProperty, SEEK_PENALTY_LEN)
        .and_then(|answer| parse_seek_penalty(&answer))
        == Some(false);

    Some(DriveInfo {
        number,
        device_path: path.to_string(),
        name: drive_name(descriptor.vendor.as_deref(), descriptor.product.as_deref()),
        revision: descriptor.revision,
        serial: nvme_serial.or(descriptor.serial),
        capacity_bytes: disk
            .control(IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &[], GEOMETRY_CAPACITY)
            .and_then(|answer| parse_disk_size(&answer)),
        removable: descriptor.removable,
        solid_state: no_seek_penalty,
        bus_type: descriptor.bus_type,
        features: if predicts_failure(&disk, descriptor.bus_type) {
            vec![FEATURE_SMART.to_string()]
        } else {
            Vec::new()
        },
        controller: None,
        volumes: Vec::new(),
    })
}

/// Whether the drive keeps the health records known as SMART. An NVMe drive is asked
/// for its health log and an ATA drive for the command sets it supports; any drive may
/// answer the failure prediction request of Windows instead.
fn predicts_failure(disk: &DeviceHandle, bus_type: Option<u32>) -> bool {
    let by_protocol = match bus_type {
        Some(BUS_NVME) => ProtocolQuery {
            property: StorageDeviceProtocolSpecificProperty,
            protocol: ProtocolTypeNvme.0 as u32,
            data_type: NVMeDataTypeLogPage.0 as u32,
            request: NVME_LOG_HEALTH,
            length: NVME_HEALTH_LEN,
        }
        .send(disk)
        .is_some_and(|log| log.len() == NVME_HEALTH_LEN),
        Some(BUS_ATA | BUS_SATA) => ProtocolQuery {
            property: StorageDeviceProtocolSpecificProperty,
            protocol: ProtocolTypeAta.0 as u32,
            data_type: AtaDataTypeIdentify.0 as u32,
            request: 0,
            length: ATA_IDENTIFY_LEN,
        }
        .send(disk)
        .is_some_and(|identify| ata_supports_smart(&identify) == Some(true)),
        _ => false,
    };
    by_protocol || disk.control(IOCTL_STORAGE_PREDICT_FAILURE, &[], PREDICT_FAILURE_LEN).is_some()
}

/// The request for one standard property: a `STORAGE_PROPERTY_QUERY` without parameters.
fn property_query(property: STORAGE_PROPERTY_ID) -> [u8; 12] {
    let mut query = [0u8; 12];
    query[..4].copy_from_slice(&property.0.to_le_bytes());
    query[4..8].copy_from_slice(&PropertyStandardQuery.0.to_le_bytes());
    query
}

/// A request for data in the drive's own protocol, such as an identification or a log page.
#[derive(Debug, Clone, Copy)]
struct ProtocolQuery {
    property: STORAGE_PROPERTY_ID,
    /// A `STORAGE_PROTOCOL_TYPE`.
    protocol: u32,
    /// What to fetch, in the numbering of the protocol's data types.
    data_type: u32,
    /// Which one of them, e.g. the number of a log page.
    request: u32,
    /// Size of the data in bytes.
    length: usize,
}

impl ProtocolQuery {
    /// The request as Windows takes it: a `STORAGE_PROPERTY_QUERY` whose parameters are a
    /// `STORAGE_PROTOCOL_SPECIFIC_DATA`, followed by room for the data. The answer
    /// comes back in the same layout.
    fn encode(&self) -> Vec<u8> {
        let mut query = vec![0u8; QUERY_HEADER_LEN + PROTOCOL_DATA_LEN + self.length];
        let fields = [
            self.property.0 as u32,
            PropertyStandardQuery.0 as u32,
            self.protocol,
            self.data_type,
            self.request,
            // The request sub-value, unused by the requests made here.
            0,
            // Where the data starts, counted from the protocol-specific part.
            PROTOCOL_DATA_LEN as u32,
            self.length as u32,
        ];
        for (slot, field) in query.chunks_exact_mut(4).zip(fields) {
            slot.copy_from_slice(&field.to_le_bytes());
        }
        query
    }

    /// Sends the request and returns the data the drive answered with.
    fn send(&self, disk: &DeviceHandle) -> Option<Vec<u8>> {
        let query = self.encode();
        let answer = disk.control(IOCTL_STORAGE_QUERY_PROPERTY, &query, query.len())?;
        protocol_data(&answer).map(<[u8]>::to_vec)
    }
}

/// The data in the answer to a [`ProtocolQuery`], at the place and of the length the
/// answer itself states.
fn protocol_data(answer: &[u8]) -> Option<&[u8]> {
    let specific = answer.get(QUERY_HEADER_LEN..)?;
    let offset = u32_at(specific, PROTOCOL_DATA_OFFSET_FIELD)? as usize;
    let length = u32_at(specific, PROTOCOL_DATA_LENGTH_FIELD)? as usize;
    specific.get(offset..)?.get(..length)
}

/// What a `STORAGE_DEVICE_DESCRIPTOR` says about a drive.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct DeviceDescriptor {
    vendor: Option<String>,
    product: Option<String>,
    revision: Option<String>,
    serial: Option<String>,
    bus_type: Option<u32>,
    removable: bool,
}

/// Decodes a `STORAGE_DEVICE_DESCRIPTOR`: fixed fields, then the strings they point to.
fn parse_device_descriptor(bytes: &[u8]) -> Option<DeviceDescriptor> {
    if bytes.len() < DEVICE_DESCRIPTOR_LEN {
        return None;
    }
    // A string is given as its offset from the start of the descriptor; a drive without
    // it leaves the offset at zero.
    let string = |field: usize| {
        let offset = u32_at(bytes, field)? as usize;
        let tail = bytes.get(offset..).filter(|_| offset >= DEVICE_DESCRIPTOR_LEN)?;
        text(tail)
    };
    Some(DeviceDescriptor {
        vendor: string(12),
        product: string(16),
        revision: string(20),
        serial: string(24),
        bus_type: u32_at(bytes, 28),
        removable: bytes[10] != 0,
    })
}

/// The drive number in a `STORAGE_DEVICE_NUMBER`, when the device is a disk.
fn parse_device_number(bytes: &[u8]) -> Option<u32> {
    (u32_at(bytes, 0)? == DEVICE_TYPE_DISK).then(|| u32_at(bytes, 4)).flatten()
}

/// The capacity in bytes from a `DISK_GEOMETRY_EX`.
fn parse_disk_size(bytes: &[u8]) -> Option<u64> {
    let size = i64::from_le_bytes(bytes.get(GEOMETRY_DISK_SIZE..GEOMETRY_DISK_SIZE + 8)?.try_into().ok()?);
    u64::try_from(size).ok().filter(|&size| size > 0)
}

/// Whether a `DEVICE_SEEK_PENALTY_DESCRIPTOR` says the drive has to seek.
fn parse_seek_penalty(bytes: &[u8]) -> Option<bool> {
    bytes.get(8).map(|&incurs| incurs != 0)
}

/// The numbers of the disks a `VOLUME_DISK_EXTENTS` lists, each once.
fn parse_disk_extents(bytes: &[u8]) -> Vec<u32> {
    let count = u32_at(bytes, 0).map_or(0, |count| (count as usize).min(MAX_EXTENTS));
    let mut disks = Vec::new();
    for extent in 0..count {
        match u32_at(bytes, EXTENTS_HEADER_LEN + extent * EXTENT_LEN) {
            Some(disk) if !disks.contains(&disk) => disks.push(disk),
            Some(_) => {}
            None => break,
        }
    }
    disks
}

/// Whether ATA `IDENTIFY DEVICE` data lists the SMART feature set. `None` when the
/// data is cut short or the words in question are not filled in.
fn ata_supports_smart(identify: &[u8]) -> Option<bool> {
    let word =
        |index: usize| Some(u16::from_le_bytes(identify.get(index * 2..index * 2 + 2)?.try_into().ok()?));
    // Bit 14 set and bit 15 clear in the second word mark both words as valid.
    (word(ATA_COMMAND_SETS_2)? & 0xC000 == 0x4000).then_some(word(ATA_COMMAND_SETS_1)? & 0x0001 != 0)
}

/// `Example Flash Drive`: the product, led by the vendor unless it is part of the
/// product already or only names the bus, as the identifiers of NVMe and ATA drives do.
fn drive_name(vendor: Option<&str>, product: Option<&str>) -> String {
    let product = product.unwrap_or_default();
    match vendor {
        Some(vendor)
            if !["ATA", "NVMe"].iter().any(|bus| vendor.eq_ignore_ascii_case(bus))
                && !product.to_ascii_lowercase().starts_with(&vendor.to_ascii_lowercase()) =>
        {
            format!("{vendor} {product}").trim().to_string()
        }
        _ => product.to_string(),
    }
}

/// Bus number and address (`device << 16 | function`) of the PCI device a disk is
/// attached to: the nearest device above it in the tree that sits on the PCI bus.
fn attachment(disk: Node) -> Option<(u32, u32)> {
    disk.ancestors().find_map(Node::pci_address)
}

/// Describes the controller at a PCI location from the PCI device list. `own_link` says
/// that the controller is the drive's own, so that its link is the drive's as well.
fn controller(pci: &[PciDevice], bus: u32, address: u32, own_link: bool) -> Option<DriveController> {
    let location = ((bus & 0xFF) as u8, u8::try_from(address >> 16).ok()?, (address & 0xFF) as u8);
    let device = pci.iter().find(|device| (device.bus, device.device, device.function) == location)?;
    let name =
        pci::class_name(device.base_class, device.sub_class, device.prog_if).unwrap_or(&device.description);
    (!name.is_empty()).then(|| DriveController {
        name: name.to_string(),
        bus: device.bus,
        device: device.device,
        function: device.function,
        link: device.link.filter(|_| own_link),
    })
}

/// A mounted volume and the numbers of the disks it has extents on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Volume {
    disks: Vec<u32>,
    info: VolumeInfo,
}

/// The volumes that lie on the disk with the given number.
fn on_disk(volumes: &[Volume], disk: u32) -> Vec<VolumeInfo> {
    volumes.iter().filter(|volume| volume.disks.contains(&disk)).map(|volume| volume.info.clone()).collect()
}

/// The volumes with a drive letter on fixed and removable drives, in letter order.
fn volumes() -> Vec<Volume> {
    let _quiet = QuietErrors::begin();
    // SAFETY: the call takes no arguments and returns a bit per drive letter.
    let letters = unsafe { GetLogicalDrives() };
    ('a'..='z')
        .zip(0u32..)
        .filter(|(_, bit)| letters & (1 << bit) != 0)
        .filter_map(|(letter, _)| volume(letter))
        .collect()
}

/// Sizes one lettered volume and finds its disks. `None` for network, optical and RAM
/// drives, and for a drive without a medium.
fn volume(letter: char) -> Option<Volume> {
    let root = format!("{letter}:\\");
    let root_wide = HSTRING::from(root.as_str());
    // SAFETY: the root path is NUL-terminated and outlives the call.
    let kind = unsafe { GetDriveTypeW(&root_wide) };
    if kind != DRIVE_FIXED && kind != DRIVE_REMOVABLE {
        return None;
    }
    let device = DeviceHandle::open(&format!(r"\\.\{letter}:"))?;
    // The volume manager answers this for every volume on a disk; a floppy drive, which
    // numbers its media in a series of its own, is not among them.
    let disks = device
        .control(IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, &[], EXTENTS_HEADER_LEN + MAX_EXTENTS * EXTENT_LEN)
        .map(|answer| parse_disk_extents(&answer))
        .filter(|disks| !disks.is_empty())?;
    let (mut size, mut free) = (0u64, 0u64);
    // SAFETY: the root path is NUL-terminated and both outputs are live for the call.
    unsafe { GetDiskFreeSpaceExW(&root_wide, None, Some(&mut size), Some(&mut free)) }.ok()?;
    let info = VolumeInfo { mount: root, size_bytes: size, free_bytes: free };
    (size > 0).then_some(Volume { disks, info })
}

/// Keeps Windows from putting up a dialog when a drive without a medium is examined.
/// The error mode the thread had returns on drop.
struct QuietErrors(Option<THREAD_ERROR_MODE>);

impl QuietErrors {
    fn begin() -> Self {
        let mut previous = THREAD_ERROR_MODE(0);
        // SAFETY: `previous` is a live out-parameter; the call only changes a setting of this thread.
        let changed = unsafe { SetThreadErrorMode(SEM_FAILCRITICALERRORS, Some(&mut previous)) };
        QuietErrors(changed.is_ok().then_some(previous))
    }
}

impl Drop for QuietErrors {
    fn drop(&mut self) {
        if let Some(previous) = self.0 {
            // SAFETY: restores the mode `begin` replaced. Nothing can be done about a failure.
            let _ = unsafe { SetThreadErrorMode(previous, None) };
        }
    }
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?))
}

/// The text of a fixed-width or NUL-terminated ASCII field, trimmed. Bytes outside
/// printable ASCII read as spaces. `None` when nothing is left.
fn text(field: &[u8]) -> Option<String> {
    let text: String = field
        .iter()
        .take_while(|&&byte| byte != 0)
        .map(|&byte| if byte.is_ascii_graphic() { char::from(byte) } else { ' ' })
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PcieLink;

    /// A device descriptor with the given strings placed after the fixed part. Each
    /// string is NUL-terminated; an absent one keeps a zero offset.
    fn device_descriptor(strings: [Option<&str>; 4], bus_type: u32, removable: bool) -> Vec<u8> {
        let mut bytes = vec![0u8; DEVICE_DESCRIPTOR_LEN];
        bytes[0..4].copy_from_slice(&40u32.to_le_bytes());
        bytes[10] = u8::from(removable);
        bytes[28..32].copy_from_slice(&bus_type.to_le_bytes());
        for (field, string) in [12, 16, 20, 24].into_iter().zip(strings) {
            if let Some(string) = string {
                let offset = bytes.len() as u32;
                bytes[field..field + 4].copy_from_slice(&offset.to_le_bytes());
                bytes.extend(string.bytes().chain([0]));
            }
        }
        let size = bytes.len() as u32;
        bytes[4..8].copy_from_slice(&size.to_le_bytes());
        bytes
    }

    /// The answer to a protocol query as a `STORAGE_PROTOCOL_DATA_DESCRIPTOR` lays it
    /// out: version and size, the protocol-specific part with protocol, data type,
    /// request value and sub-value, data offset and data length, then the data.
    fn protocol_answer(protocol: u32, data_type: u32, request: u32, data: &[u8]) -> Vec<u8> {
        let mut answer = Vec::new();
        for field in [48, 48, protocol, data_type, request, 0, 40, data.len() as u32, 0, 0, 0, 0] {
            answer.extend(field.to_le_bytes());
        }
        answer.extend(data);
        answer
    }

    /// ATA identification data with the two command set words set.
    fn ata_identify(command_sets_1: u16, command_sets_2: u16) -> Vec<u8> {
        let mut identify = vec![0u8; ATA_IDENTIFY_LEN];
        identify[ATA_COMMAND_SETS_1 * 2..][..2].copy_from_slice(&command_sets_1.to_le_bytes());
        identify[ATA_COMMAND_SETS_2 * 2..][..2].copy_from_slice(&command_sets_2.to_le_bytes());
        identify
    }

    fn nvme_controller() -> PciDevice {
        PciDevice {
            bus: 2,
            vendor_id: 0x1987,
            device_id: 0x5013,
            base_class: 0x01,
            sub_class: 0x08,
            prog_if: 0x02,
            description: "Standard NVM Express Controller".into(),
            link: Some(PcieLink {
                width: Some(4),
                max_width: Some(4),
                speed_gts: Some(16.0),
                max_speed_gts: Some(16.0),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn names_the_buses() {
        assert_eq!(bus_name(7), Some("USB"));
        assert_eq!(bus_name(11), Some("SATA"));
        assert_eq!(bus_name(17), Some("NVMe"));
        assert_eq!(bus_name(0), None);
        assert_eq!(bus_name(20), None);
        assert_eq!(
            (bus_name(BUS_ATA), bus_name(BUS_SATA), bus_name(BUS_NVME)),
            (Some("ATA"), Some("SATA"), Some("NVMe"))
        );
    }

    #[test]
    fn decodes_device_descriptors() {
        let nvme = device_descriptor(
            [None, Some("Example SSD 1TB"), Some("EX1.0 "), Some("0000_0000_0000_0001.")],
            17,
            false,
        );
        assert_eq!(
            parse_device_descriptor(&nvme),
            Some(DeviceDescriptor {
                vendor: None,
                product: Some("Example SSD 1TB".into()),
                revision: Some("EX1.0".into()),
                serial: Some("0000_0000_0000_0001.".into()),
                bus_type: Some(17),
                removable: false,
            })
        );
        let stick = device_descriptor(
            [Some("Example "), Some("Flash Drive     "), Some("1.00"), Some("  ")],
            7,
            true,
        );
        let stick = parse_device_descriptor(&stick).unwrap();
        assert_eq!(
            (stick.vendor.as_deref(), stick.product.as_deref()),
            (Some("Example"), Some("Flash Drive"))
        );
        assert_eq!((stick.serial, stick.bus_type, stick.removable), (None, Some(7), true));
    }

    #[test]
    fn descriptor_strings_stay_inside_the_data() {
        let mut bytes = device_descriptor([None, Some("Example Disk"), None, None], 11, false);
        // The driver's buffer ended in the middle of the product.
        let cut = parse_device_descriptor(&bytes[..DEVICE_DESCRIPTOR_LEN + 7]).unwrap();
        assert_eq!(cut.product.as_deref(), Some("Example"));
        // Offsets that point past the data or into the fixed part name no string.
        bytes[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[20..24].copy_from_slice(&0x1_0000u32.to_le_bytes());
        bytes[24..28].copy_from_slice(&16u32.to_le_bytes());
        let odd = parse_device_descriptor(&bytes).unwrap();
        assert_eq!((odd.vendor, odd.revision, odd.serial), (None, None, None));
        assert_eq!(odd.product.as_deref(), Some("Example Disk"));

        assert_eq!(parse_device_descriptor(&bytes[..DEVICE_DESCRIPTOR_LEN - 1]), None);
        assert_eq!(parse_device_descriptor(&[]), None);
    }

    #[test]
    fn reads_drive_numbers_of_disks_only() {
        let number = |kind: u32, number: u32| {
            let mut bytes = Vec::new();
            for field in [kind, number, 0] {
                bytes.extend(field.to_le_bytes());
            }
            bytes
        };
        assert_eq!(parse_device_number(&number(DEVICE_TYPE_DISK, 3)), Some(3));
        // An optical drive counts in a series of its own.
        assert_eq!(parse_device_number(&number(2, 0)), None);
        assert_eq!(parse_device_number(&number(DEVICE_TYPE_DISK, 3)[..7]), None);
        assert_eq!(parse_device_number(&[]), None);
    }

    #[test]
    fn reads_the_disk_size() {
        let mut geometry = vec![0u8; 40];
        geometry[GEOMETRY_DISK_SIZE..GEOMETRY_DISK_SIZE + 8]
            .copy_from_slice(&1_000_204_886_016i64.to_le_bytes());
        assert_eq!(parse_disk_size(&geometry), Some(1_000_204_886_016));
        assert_eq!(parse_disk_size(&geometry[..GEOMETRY_DISK_SIZE + 7]), None);
        // A card reader without a card, and a size no disk has.
        assert_eq!(parse_disk_size(&[0u8; 40]), None);
        geometry[GEOMETRY_DISK_SIZE..GEOMETRY_DISK_SIZE + 8].copy_from_slice(&(-1i64).to_le_bytes());
        assert_eq!(parse_disk_size(&geometry), None);
    }

    #[test]
    fn reads_the_seek_penalty() {
        assert_eq!(parse_seek_penalty(&[12, 0, 0, 0, 12, 0, 0, 0, 0, 0, 0, 0]), Some(false));
        assert_eq!(parse_seek_penalty(&[12, 0, 0, 0, 12, 0, 0, 0, 1, 0, 0, 0]), Some(true));
        assert_eq!(parse_seek_penalty(&[12, 0, 0, 0, 12, 0, 0, 0]), None);
    }

    #[test]
    fn lists_the_disks_of_a_volume() {
        let extents = |count: u32, disks: &[u32]| {
            let mut bytes = vec![0u8; EXTENTS_HEADER_LEN];
            bytes[..4].copy_from_slice(&count.to_le_bytes());
            for disk in disks {
                let mut extent = [0u8; EXTENT_LEN];
                extent[..4].copy_from_slice(&disk.to_le_bytes());
                bytes.extend(extent);
            }
            bytes
        };
        assert_eq!(parse_disk_extents(&extents(1, &[0])), [0]);
        // A volume spanning two disks, with two extents on the first.
        assert_eq!(parse_disk_extents(&extents(3, &[2, 2, 5])), [2, 5]);
        // A count beyond the data stops at the data.
        assert_eq!(parse_disk_extents(&extents(u32::MAX, &[1, 4])), [1, 4]);
        assert!(parse_disk_extents(&extents(0, &[7])).is_empty());
        assert!(parse_disk_extents(&[1, 0]).is_empty());
    }

    #[test]
    fn encodes_property_queries() {
        assert_eq!(property_query(StorageDeviceProperty), [0u8; 12]);
        assert_eq!(property_query(StorageDeviceSeekPenaltyProperty), [7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

        let identify = ProtocolQuery {
            property: StorageAdapterProtocolSpecificProperty,
            protocol: ProtocolTypeNvme.0 as u32,
            data_type: NVMeDataTypeIdentify.0 as u32,
            request: NVME_IDENTIFY_CONTROLLER,
            length: NVME_IDENTIFY_LEN,
        };
        let query = identify.encode();
        assert_eq!(query.len(), 8 + 40 + 4096);
        let fields: Vec<u32> = (0..12).map(|field| u32_at(&query, field * 4).unwrap()).collect();
        assert_eq!(fields, [49, 0, 3, 1, 1, 0, 40, 4096, 0, 0, 0, 0]);
        assert!(query[48..].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn finds_the_protocol_data() {
        let mut identify = vec![0u8; NVME_IDENTIFY_LEN];
        identify[NVME_SERIAL].copy_from_slice(b"EX0000000001        ");
        let answer = protocol_answer(3, 1, 1, &identify);
        assert_eq!(answer.len(), 48 + NVME_IDENTIFY_LEN);
        let data = protocol_data(&answer).unwrap();
        assert_eq!(data.len(), NVME_IDENTIFY_LEN);
        assert_eq!(text(&data[NVME_SERIAL]).as_deref(), Some("EX0000000001"));

        // A request comes back in the layout it was sent in.
        let request = ProtocolQuery {
            property: StorageDeviceProtocolSpecificProperty,
            protocol: 3,
            data_type: 2,
            request: NVME_LOG_HEALTH,
            length: NVME_HEALTH_LEN,
        };
        assert_eq!(protocol_data(&request.encode()).map(<[u8]>::len), Some(NVME_HEALTH_LEN));

        // The stated place and length have to lie inside the answer.
        assert_eq!(protocol_data(&answer[..answer.len() - 1]), None);
        assert_eq!(protocol_data(&answer[..28]), None);
        let mut far = answer.clone();
        far[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(protocol_data(&far), None);
        assert_eq!(protocol_data(&protocol_answer(3, 2, 2, &[])), Some(&[][..]));
    }

    #[test]
    fn reads_smart_support_from_ata_identification() {
        assert_eq!(ata_supports_smart(&ata_identify(0x746B, 0x7D01)), Some(true));
        assert_eq!(ata_supports_smart(&ata_identify(0x746A, 0x7D01)), Some(false));
        // Words that are not marked valid say nothing either way.
        assert_eq!(ata_supports_smart(&ata_identify(0x0001, 0x0000)), None);
        assert_eq!(ata_supports_smart(&ata_identify(0xFFFF, 0xFFFF)), None);
        assert_eq!(ata_supports_smart(&ata_identify(0x0001, 0x4000)[..166]), None);
        assert_eq!(ata_supports_smart(&[]), None);
    }

    #[test]
    fn composes_drive_names() {
        let name = drive_name;
        assert_eq!(name(None, Some("Example SSD 1TB")), "Example SSD 1TB");
        assert_eq!(name(Some("Example"), Some("Flash Drive")), "Example Flash Drive");
        // The vendor field of these only repeats the bus.
        assert_eq!(name(Some("NVMe"), Some("Example SSD 1TB")), "Example SSD 1TB");
        assert_eq!(name(Some("ATA"), Some("Example HDD 4TB")), "Example HDD 4TB");
        assert_eq!(name(Some("EXAMPLE"), Some("Example Portable")), "Example Portable");
        assert_eq!(name(Some("Example"), None), "Example");
        assert_eq!(name(None, None), "");
    }

    #[test]
    fn describes_the_controller_from_the_pci_list() {
        let ahci = PciDevice {
            device: 0x17,
            base_class: 0x01,
            sub_class: 0x06,
            prog_if: 0x01,
            link: Some(PcieLink { width: Some(1), ..Default::default() }),
            ..Default::default()
        };
        let unnamed = PciDevice { bus: 3, base_class: 0xFF, ..Default::default() };
        let named = PciDevice {
            bus: 4,
            base_class: 0xFF,
            description: "Example Storage Card".into(),
            ..Default::default()
        };
        let pci = [ahci, nvme_controller(), unnamed, named];

        let nvme = controller(&pci, 2, 0, true).unwrap();
        assert_eq!(nvme.name, "NVM Express (NVMe) Controller");
        assert_eq!((nvme.bus, nvme.device, nvme.function), (2, 0, 0));
        assert_eq!(nvme.link, pci[1].link);

        // A drive behind a SATA port does not share the controller's link.
        let sata = controller(&pci, 0, 0x0017_0000, false).unwrap();
        assert_eq!(sata.name, "Serial ATA Controller - AHCI");
        assert_eq!((sata.bus, sata.device, sata.function, sata.link), (0, 0x17, 0, None));

        // The bus number may carry a PCI segment above its low byte.
        assert_eq!(controller(&pci, 0x0001_0002, 0, true).map(|found| found.bus), Some(2));
        assert_eq!(
            controller(&pci, 4, 0, false).map(|found| found.name).as_deref(),
            Some("Example Storage Card")
        );
        assert_eq!(controller(&pci, 3, 0, false), None);
        assert_eq!(controller(&pci, 2, 1, true), None);
        assert_eq!(controller(&pci, 2, 0x0100_0000, true), None);
        assert_eq!(controller(&[], 2, 0, true), None);
    }

    #[test]
    fn assigns_volumes_to_their_disks() {
        let volume = |mount: &str, disks: &[u32]| Volume {
            disks: disks.to_vec(),
            info: VolumeInfo { mount: mount.into(), size_bytes: 1 << 30, free_bytes: 1 << 29 },
        };
        let volumes = [volume("c:\\", &[0]), volume("d:\\", &[1, 2]), volume("e:\\", &[0])];
        let mounts = |disk| on_disk(&volumes, disk).into_iter().map(|info| info.mount).collect::<Vec<_>>();
        assert_eq!(mounts(0), ["c:\\", "e:\\"]);
        assert_eq!(mounts(1), ["d:\\"]);
        assert_eq!(mounts(2), ["d:\\"]);
        assert!(mounts(3).is_empty());
    }

    #[test]
    fn trims_fixed_width_text() {
        assert_eq!(text(b"EX1.0   ").as_deref(), Some("EX1.0"));
        assert_eq!(text(b"  Example\0garbage").as_deref(), Some("Example"));
        assert_eq!(text(&[b'A', 0x07, 0xFF, b'B']).as_deref(), Some("A  B"));
        assert_eq!(text(b"        "), None);
        assert_eq!(text(&[]), None);
    }

    #[test]
    fn detection_is_self_consistent() {
        let drives = detect(&[]);
        assert!(drives.windows(2).all(|pair| pair[0].number < pair[1].number));
        for drive in &drives {
            assert!(!drive.device_path.is_empty());
            assert_eq!(drive.controller, None);
            assert!(drive.volumes.iter().all(|volume| volume.free_bytes <= volume.size_bytes));
        }
    }
}
