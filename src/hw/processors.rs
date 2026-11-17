//! Logical processors as Windows presents them: how one is identified, how a
//! thread is kept on one, the processor groups they sit in and the relation
//! records that say how they hang together. Processor detection, the clock
//! monitor and the benchmark all start from here.

use std::mem::{offset_of, size_of};

use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, HANDLE};
use windows::Win32::System::SystemInformation::{
    CpuSetInformation, GetLogicalProcessorInformationEx, GetSystemCpuSetInformation, RelationAll,
    GROUP_AFFINITY, SYSTEM_CPU_SET_INFORMATION, SYSTEM_CPU_SET_INFORMATION_0_0,
    SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
};
use windows::Win32::System::Threading::{
    GetActiveProcessorCount, GetActiveProcessorGroupCount, GetCurrentThread, SetThreadGroupAffinity,
};

/// A logical processor, identified the way the affinity APIs do: by its
/// processor group and its number inside the group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcId {
    pub group: u16,
    pub number: u8,
}

/// Keeps the calling thread on one logical processor at a time. Dropping it on that
/// thread gives the thread back the affinity it had before the first move.
#[derive(Default)]
pub struct Pin {
    original: Option<GROUP_AFFINITY>,
}

impl Pin {
    /// Binds the calling thread to `processor`. If that fails the thread simply keeps
    /// running where it is, which [`Pin::is_pinned`] tells.
    pub fn to(processor: ProcId) -> Pin {
        let mut pin = Pin::default();
        pin.move_to(processor);
        pin
    }

    /// Moves the calling thread to `processor`. `false` when the process is not allowed
    /// to run there; the thread then stays where it was.
    pub fn move_to(&mut self, processor: ProcId) -> bool {
        let Some(mask) = 1usize.checked_shl(u32::from(processor.number)) else { return false };
        let target = GROUP_AFFINITY { Mask: mask, Group: processor.group, ..Default::default() };
        let mut previous = GROUP_AFFINITY::default();
        // SAFETY: the pseudo handle of the current thread is always valid and both
        // pointers reference live `GROUP_AFFINITY` values.
        let moved =
            unsafe { SetThreadGroupAffinity(GetCurrentThread(), &target, Some(&mut previous)) }.as_bool();
        if moved && self.original.is_none() {
            // Only the first move sees the affinity the thread came with. Its reserved
            // fields have to be zero when it is handed back.
            self.original =
                Some(GROUP_AFFINITY { Mask: previous.Mask, Group: previous.Group, ..Default::default() });
        }
        moved
    }

    /// Whether a move succeeded, so that the thread runs where it was last sent.
    pub fn is_pinned(&self) -> bool {
        self.original.is_some()
    }
}

impl Drop for Pin {
    fn drop(&mut self) {
        if let Some(original) = self.original {
            // SAFETY: as in `Pin::move_to`; `original` is the affinity Windows reported for this thread.
            // A failure leaves the thread pinned to the last processor, which nothing can act on.
            let _ = unsafe { SetThreadGroupAffinity(GetCurrentThread(), &original, None) };
        }
    }
}

/// Number of active logical processors in each processor group.
pub fn group_sizes() -> Vec<u32> {
    // SAFETY: both functions only read system information; an invalid group number yields zero.
    unsafe { (0..GetActiveProcessorGroupCount()).map(|group| GetActiveProcessorCount(group)).collect() }
}

/// Fetches the `RelationAll` buffer of `GetLogicalProcessorInformationEx`, to
/// be split with [`records`]. The required size is learnt from a first call;
/// the loop covers the topology growing in between (processor hot-add).
/// `None` when Windows refuses.
pub fn relations() -> Option<Vec<u8>> {
    let mut length = 0u32;
    for _ in 0..4 {
        // u64 storage keeps the buffer aligned for the structures Windows writes into it.
        let mut buffer = vec![0u64; (length as usize).div_ceil(8)];
        let pointer = (!buffer.is_empty())
            .then(|| buffer.as_mut_ptr().cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>());
        // SAFETY: `pointer`, when given, addresses at least `length` writable bytes owned by
        // `buffer`, and `length` outlives the call.
        let result = unsafe { GetLogicalProcessorInformationEx(RelationAll, pointer, &mut length) };
        match result {
            Ok(()) => {
                let bytes = buffer.iter().flat_map(|word| word.to_ne_bytes());
                return Some(bytes.take(length as usize).collect());
            }
            Err(error) if error.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult() => {}
            Err(_) => return None,
        }
    }
    None
}

/// Splits a relations buffer into its variable-length records, each given as
/// its relationship kind and the structure that follows the record header.
/// The walk ends at the first record whose size cannot be right.
pub fn records(buffer: &[u8]) -> impl Iterator<Item = (i32, &[u8])> {
    const SIZE: usize = offset_of!(SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX, Size);
    const BODY: usize = offset_of!(SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX, Anonymous);

    let mut rest = buffer;
    std::iter::from_fn(move || {
        let (relationship, size) = (u32_at(rest, 0)?, u32_at(rest, SIZE)? as usize);
        if size < BODY || size > rest.len() {
            return None;
        }
        let (record, tail) = rest.split_at(size);
        rest = tail;
        Some((relationship as i32, &record[BODY..]))
    })
}

/// The scheduling class of every logical processor Windows lists for the
/// process: given the choice, the scheduler puts a thread on a processor of a
/// higher class. Empty when Windows does not say.
pub fn scheduling_classes() -> Vec<(ProcId, u8)> {
    let mut length = 0u32;
    // SAFETY: without a buffer the call only reports the size it needs through `length`.
    let _ = unsafe { GetSystemCpuSetInformation(None, 0, &mut length, HANDLE::default(), 0) };
    // u64 storage keeps the buffer aligned for the structures Windows writes into it.
    let mut buffer = vec![0u64; (length as usize).div_ceil(8)];
    if buffer.is_empty() {
        return Vec::new();
    }
    let pointer = buffer.as_mut_ptr().cast::<SYSTEM_CPU_SET_INFORMATION>();
    // SAFETY: `pointer` addresses at least `length` writable bytes owned by `buffer`, and
    // `length` outlives the call. The null handle stands for the calling process.
    let filled =
        unsafe { GetSystemCpuSetInformation(Some(pointer), length, &mut length, HANDLE::default(), 0) };
    if !filled.as_bool() {
        return Vec::new();
    }
    let bytes: Vec<u8> = buffer.iter().flat_map(|word| word.to_ne_bytes()).take(length as usize).collect();
    parse_cpu_sets(&bytes)
}

/// Reads the processor and the scheduling class out of every CPU set record of a
/// `GetSystemCpuSetInformation` buffer. The walk ends at the first record whose
/// size cannot be right.
fn parse_cpu_sets(buffer: &[u8]) -> Vec<(ProcId, u8)> {
    const SIZE: usize = offset_of!(SYSTEM_CPU_SET_INFORMATION, Size);
    const TYPE: usize = offset_of!(SYSTEM_CPU_SET_INFORMATION, Type);
    const BODY: usize = offset_of!(SYSTEM_CPU_SET_INFORMATION, Anonymous);
    const GROUP: usize = BODY + offset_of!(SYSTEM_CPU_SET_INFORMATION_0_0, Group);
    const NUMBER: usize = BODY + offset_of!(SYSTEM_CPU_SET_INFORMATION_0_0, LogicalProcessorIndex);
    const CLASS: usize = BODY + offset_of!(SYSTEM_CPU_SET_INFORMATION_0_0, Anonymous2);

    let mut sets = Vec::new();
    let mut rest = buffer;
    while let (Some(size), Some(kind)) = (u32_at(rest, SIZE), u32_at(rest, TYPE)) {
        let size = size as usize;
        if size <= CLASS || size > rest.len() {
            break;
        }
        let (record, tail) = rest.split_at(size);
        rest = tail;
        if kind as i32 == CpuSetInformation.0 {
            if let Some(group) = u16_at(record, GROUP) {
                sets.push((ProcId { group, number: record[NUMBER] }, record[CLASS]));
            }
        }
    }
    sets
}

/// A set of logical processors inside one processor group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupMask {
    pub group: u16,
    pub mask: u64,
}

impl GroupMask {
    /// The processors of the set, lowest number first.
    pub fn processors(self) -> impl Iterator<Item = ProcId> {
        (0..u64::BITS as u8)
            .filter(move |n| self.mask >> n & 1 != 0)
            .map(move |number| ProcId { group: self.group, number })
    }
}

/// Reads `count` consecutive `GROUP_AFFINITY` entries of a record body,
/// starting at `at`. `None` when the body is too short to hold them all.
pub fn group_masks(body: &[u8], at: usize, count: u16) -> Option<Vec<GroupMask>> {
    (0..usize::from(count))
        .map(|i| {
            let entry = body.get(at + i * size_of::<GROUP_AFFINITY>()..)?;
            let mask = entry.get(offset_of!(GROUP_AFFINITY, Mask)..)?.get(..size_of::<usize>())?;
            let mut wide = [0u8; 8];
            wide[..mask.len()].copy_from_slice(mask);
            Some(GroupMask {
                group: u16_at(entry, offset_of!(GROUP_AFFINITY, Group))?,
                mask: u64::from_le_bytes(wide),
            })
        })
        .collect()
}

pub fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..)?.get(..2)?.try_into().ok()?))
}

pub fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..)?.get(..4)?.try_into().ok()?))
}

#[cfg(test)]
pub mod fixtures {
    //! Builders for relation records, shared by the tests of the modules that parse them.

    use super::*;

    const BODY: usize = offset_of!(SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX, Anonymous);
    const AFFINITY: usize = size_of::<GROUP_AFFINITY>();

    /// Serialises one record the way Windows lays it out.
    pub fn record(relationship: i32, body: Vec<u8>) -> Vec<u8> {
        let mut bytes = relationship.to_le_bytes().to_vec();
        bytes.extend(((BODY + body.len()) as u32).to_le_bytes());
        bytes.extend(body);
        bytes
    }

    /// Serialises the CPU set record of logical processor `number` in `group`.
    pub fn cpu_set(group: u16, number: u8, scheduling_class: u8) -> Vec<u8> {
        const BODY: usize = offset_of!(SYSTEM_CPU_SET_INFORMATION, Anonymous);
        let mut record = vec![0u8; size_of::<SYSTEM_CPU_SET_INFORMATION>()];
        let size = record.len() as u32;
        record[..4].copy_from_slice(&size.to_le_bytes());
        let group_at = BODY + offset_of!(SYSTEM_CPU_SET_INFORMATION_0_0, Group);
        record[group_at..group_at + 2].copy_from_slice(&group.to_le_bytes());
        record[BODY + offset_of!(SYSTEM_CPU_SET_INFORMATION_0_0, LogicalProcessorIndex)] = number;
        record[BODY + offset_of!(SYSTEM_CPU_SET_INFORMATION_0_0, Anonymous2)] = scheduling_class;
        record
    }

    /// Writes `(group, mask)` entries into `body` from offset `at` on, growing it as
    /// needed. There is room for one entry even when none is given.
    pub fn put_masks(body: &mut Vec<u8>, at: usize, masks: &[(u16, u64)]) {
        body.resize(at + masks.len().max(1) * AFFINITY, 0);
        for (i, (group, mask)) in masks.iter().enumerate() {
            let entry = at + i * AFFINITY;
            let mask_at = entry + offset_of!(GROUP_AFFINITY, Mask);
            body[mask_at..mask_at + size_of::<usize>()].copy_from_slice(&(*mask as usize).to_le_bytes());
            let group_at = entry + offset_of!(GROUP_AFFINITY, Group);
            body[group_at..group_at + 2].copy_from_slice(&group.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::System::Threading::{GetCurrentProcessorNumberEx, GetThreadGroupAffinity};

    use super::fixtures::{cpu_set, put_masks, record};
    use super::*;

    fn id(group: u16, number: u8) -> ProcId {
        ProcId { group, number }
    }

    /// Group and mask the calling thread is confined to.
    fn affinity() -> (u16, usize) {
        let mut affinity = GROUP_AFFINITY::default();
        // SAFETY: the pseudo handle is always valid and `affinity` is a live out value.
        let read = unsafe { GetThreadGroupAffinity(GetCurrentThread(), &mut affinity) };
        assert!(read.as_bool());
        (affinity.Group, affinity.Mask)
    }

    #[test]
    fn pin_moves_the_thread_and_puts_it_back() {
        let before = affinity();
        // SAFETY: the call only fills in the structure it returns.
        let here = unsafe { GetCurrentProcessorNumberEx() };
        let here = id(here.Group, here.Number);
        let nowhere = id(here.group, 200);
        {
            let mut pin = Pin::default();
            assert!(!pin.is_pinned());
            assert!(pin.move_to(here));
            assert!(pin.is_pinned());
            assert_eq!(affinity(), (here.group, 1 << here.number));
            // A failed move leaves the thread on the processor it was sent to before.
            assert!(!pin.move_to(nowhere));
            assert_eq!(affinity(), (here.group, 1 << here.number));
        }
        assert_eq!(affinity(), before);

        assert!(!Pin::to(nowhere).is_pinned());
        assert_eq!(affinity(), before);
    }

    #[test]
    fn walks_the_records_in_order() {
        let mut buffer = record(3, vec![0xAA; 4]);
        buffer.extend(record(0, Vec::new()));
        buffer.extend(record(1, vec![0xBB; 40]));
        let walked: Vec<(i32, Vec<u8>)> =
            records(&buffer).map(|(kind, body)| (kind, body.to_vec())).collect();
        assert_eq!(walked, [(3, vec![0xAA; 4]), (0, Vec::new()), (1, vec![0xBB; 40])]);
        assert_eq!(records(&[]).count(), 0);
    }

    #[test]
    fn stops_at_malformed_records() {
        let mut buffer = record(0, vec![1; 24]);
        let good = buffer.len();
        buffer.extend(record(0, vec![2; 24]));
        // A header cut short, then a record cut short.
        assert_eq!(records(&buffer[..good + 5]).count(), 1);
        assert_eq!(records(&buffer[..buffer.len() - 1]).count(), 1);
        // A record that claims to be shorter than its header ends the walk.
        buffer[good + 4..good + 8].copy_from_slice(&4u32.to_le_bytes());
        assert_eq!(records(&buffer).count(), 1);
    }

    #[test]
    fn reads_group_masks() {
        let mut body = Vec::new();
        put_masks(&mut body, 6, &[(0, 0b101), (3, 1 << 63)]);
        let masks = group_masks(&body, 6, 2).unwrap();
        assert_eq!(masks, [GroupMask { group: 0, mask: 0b101 }, GroupMask { group: 3, mask: 1 << 63 }]);
        assert_eq!(masks[0].processors().collect::<Vec<_>>(), [id(0, 0), id(0, 2)]);
        assert_eq!(masks[1].processors().collect::<Vec<_>>(), [id(3, 63)]);
        assert_eq!(group_masks(&body, 6, 0), Some(Vec::new()));
        // More entries than the body holds is a malformed record, not a shorter list.
        assert_eq!(group_masks(&body, 6, 3), None);
        let inside_second_group = 6 + size_of::<GROUP_AFFINITY>() + offset_of!(GROUP_AFFINITY, Group) + 1;
        assert_eq!(group_masks(&body[..inside_second_group], 6, 2), None);
    }

    #[test]
    fn reads_scheduling_classes() {
        let mut buffer = cpu_set(0, 0, 6);
        buffer.extend(cpu_set(0, 1, 6));
        buffer.extend(cpu_set(1, 63, 2));
        assert_eq!(parse_cpu_sets(&buffer), [(id(0, 0), 6), (id(0, 1), 6), (id(1, 63), 2)]);
        assert!(parse_cpu_sets(&[]).is_empty());

        // A record of a kind this module does not know is passed over.
        let type_at = offset_of!(SYSTEM_CPU_SET_INFORMATION, Type);
        let mut foreign = cpu_set(0, 2, 9);
        foreign[type_at..type_at + 4].copy_from_slice(&7u32.to_le_bytes());
        let mut mixed = foreign;
        mixed.extend(cpu_set(0, 3, 1));
        assert_eq!(parse_cpu_sets(&mixed), [(id(0, 3), 1)]);
    }

    #[test]
    fn stops_at_malformed_cpu_sets() {
        let good = cpu_set(0, 0, 4);
        let mut buffer = good.clone();
        buffer.extend(cpu_set(0, 1, 5));
        // A record cut short, and one that claims a size too small to hold a class.
        assert_eq!(parse_cpu_sets(&buffer[..buffer.len() - 1]), [(id(0, 0), 4)]);
        buffer[good.len()..good.len() + 4].copy_from_slice(&8u32.to_le_bytes());
        assert_eq!(parse_cpu_sets(&buffer), [(id(0, 0), 4)]);
        buffer[good.len()..good.len() + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(parse_cpu_sets(&buffer), [(id(0, 0), 4)]);
    }

    #[test]
    fn live_scheduling_classes_name_active_processors() {
        let groups = group_sizes();
        for (processor, _) in scheduling_classes() {
            let size = groups.get(usize::from(processor.group)).copied().unwrap_or(0);
            assert!(u32::from(processor.number) < size, "{processor:?}");
        }
    }

    #[test]
    fn reads_integers_within_bounds() {
        let bytes = [0x34, 0x12, 0x78, 0x56, 0xBC];
        assert_eq!(u16_at(&bytes, 0), Some(0x1234));
        assert_eq!(u16_at(&bytes, 3), Some(0xBC56));
        assert_eq!(u16_at(&bytes, 4), None);
        assert_eq!(u32_at(&bytes, 1), Some(0xBC56_7812));
        assert_eq!(u32_at(&bytes, 2), None);
        assert_eq!(u32_at(&bytes, 9), None);
    }

    #[test]
    fn live_query_describes_the_active_processors() {
        let groups = group_sizes();
        assert!(!groups.is_empty());
        assert!(groups.iter().sum::<u32>() >= 1);
        let buffer = relations().expect("Windows reports the processor relations");
        // The records make up the whole buffer.
        let header = offset_of!(SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX, Anonymous);
        let walked: usize = records(&buffer).map(|(_, body)| header + body.len()).sum();
        assert_eq!(walked, buffer.len());
    }
}
