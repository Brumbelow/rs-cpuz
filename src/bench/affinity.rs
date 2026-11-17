//! Spreads benchmark threads over processor groups.
//!
//! Systems with more than 64 logical processors split them into groups, and
//! before Windows 11 a thread only runs in the group it was started in unless
//! it is moved explicitly.

use windows::Win32::System::SystemInformation::GROUP_AFFINITY;
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadGroupAffinity};

/// Moves the calling thread to the processor group that worker number `worker`
/// belongs to, `groups` being the number of logical processors in each group.
/// Does nothing on a system with a single group.
pub fn assign_worker(worker: u32, groups: &[u32]) {
    if groups.len() < 2 {
        return;
    }
    let Some(group) = group_of(worker, groups) else { return };
    let affinity =
        GROUP_AFFINITY { Mask: processor_mask(groups[usize::from(group)]), Group: group, Reserved: [0; 3] };
    // SAFETY: the pseudo handle of the current thread is always valid and
    // `affinity` outlives the call. On failure the thread keeps its group.
    unsafe {
        let _ = SetThreadGroupAffinity(GetCurrentThread(), &affinity, None);
    }
}

/// The group for worker number `worker`: workers fill the groups in order, one
/// per logical processor, and start over once every processor has one.
fn group_of(worker: u32, groups: &[u32]) -> Option<u16> {
    let total: u64 = groups.iter().map(|&size| u64::from(size)).sum();
    let mut slot = u64::from(worker).checked_rem(total)?;
    for (group, &size) in groups.iter().enumerate() {
        if slot < u64::from(size) {
            return u16::try_from(group).ok();
        }
        slot -= u64::from(size);
    }
    None
}

/// Affinity mask selecting the first `count` processors of a group.
fn processor_mask(count: u32) -> usize {
    1usize.checked_shl(count).map_or(usize::MAX, |bit| bit - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workers_fill_groups_in_order() {
        let groups = [64, 32];
        assert_eq!(group_of(0, &groups), Some(0));
        assert_eq!(group_of(63, &groups), Some(0));
        assert_eq!(group_of(64, &groups), Some(1));
        assert_eq!(group_of(95, &groups), Some(1));
        // More workers than processors wrap around.
        assert_eq!(group_of(96, &groups), Some(0));
        assert_eq!(group_of(96 + 70, &groups), Some(1));
    }

    #[test]
    fn every_processor_gets_one_worker() {
        let groups = [64, 64, 48];
        let mut workers = [0u32; 3];
        for worker in 0..176 {
            workers[usize::from(group_of(worker, &groups).unwrap())] += 1;
        }
        assert_eq!(workers, groups);
    }

    #[test]
    fn no_group_without_processors() {
        assert_eq!(group_of(0, &[]), None);
        assert_eq!(group_of(5, &[0, 0]), None);
        assert_eq!(group_of(5, &[0, 4]), Some(1));
    }

    #[test]
    fn masks_cover_the_group() {
        assert_eq!(processor_mask(0), 0);
        assert_eq!(processor_mask(1), 1);
        assert_eq!(processor_mask(12), 0xFFF);
        assert_eq!(processor_mask(usize::BITS), usize::MAX);
        assert_eq!(processor_mask(200), usize::MAX);
    }

    #[test]
    fn assigning_a_worker_is_harmless_on_this_machine() {
        assign_worker(0, &crate::hw::processors::group_sizes());
    }
}
