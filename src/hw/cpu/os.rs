//! The Windows side of processor detection: the topology query, pinning a
//! thread to one logical processor at a time, and the registry values the
//! firmware and the kernel leave under `CentralProcessor\0`.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_LOCAL_MACHINE, REG_ROUTINE_FLAGS, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD,
};

use super::cpuid;
use super::topology::{self, OsTopology};
use crate::hw::processors::{self, Pin};
use crate::model::ThreadDump;

const CPU_KEY: PCWSTR = w!(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0");

/// Queries the processor topology of every processor group. Empty when Windows refuses.
pub fn topology() -> OsTopology {
    processors::relations().map(|buffer| topology::parse(&buffer)).unwrap_or_default()
}

/// Runs the CPUID enumeration on every logical processor the process may use.
/// A short-lived helper thread does the hopping between processors, so the
/// caller's affinity is left alone; should the helper not start, the calling
/// thread does it and gets its affinity back afterwards.
pub fn thread_dumps(topology: &OsTopology) -> Vec<ThreadDump> {
    let helper = std::thread::scope(|scope| {
        let spawned = std::thread::Builder::new().spawn_scoped(scope, || sample_pinned(topology));
        spawned.ok().and_then(|thread| thread.join().ok())
    });
    helper.unwrap_or_else(|| sample_pinned(topology))
}

/// Pins the calling thread to each logical processor in turn and dumps CPUID
/// there, restoring the thread's affinity before returning. Without a usable
/// topology the result is a single dump from wherever the thread happens to run.
fn sample_pinned(topology: &OsTopology) -> Vec<ThreadDump> {
    let mut pin = Pin::default();
    let mut dumps: Vec<ThreadDump> = topology
        .processors
        .iter()
        .zip(0u32..)
        .filter(|(processor, _)| pin.move_to(**processor))
        .map(|(_, index)| ThreadDump { processor: index, leaves: cpuid::enumerate(&mut cpuid::read) })
        .collect();
    drop(pin);
    if dumps.is_empty() {
        dumps.push(ThreadDump { processor: 0, leaves: cpuid::enumerate(&mut cpuid::read) });
    }
    dumps
}

/// The raw `Update Revision` value: the microcode revision as the kernel read it at boot.
pub fn update_revision() -> Option<Vec<u8>> {
    let mut data = [0u8; 8];
    let length = read_value(w!("Update Revision"), RRF_RT_REG_BINARY, &mut data)?;
    Some(data.get(..length)?.to_vec())
}

/// The `~MHz` value: the processor speed the kernel measured at boot.
pub fn firmware_mhz() -> Option<u32> {
    let mut data = [0u8; 4];
    let length = read_value(w!("~MHz"), RRF_RT_REG_DWORD, &mut data)?;
    (length == data.len()).then(|| u32::from_le_bytes(data)).filter(|&mhz| mhz != 0)
}

/// Reads a value of the first processor's registry key into `data`, returning its length.
fn read_value(name: PCWSTR, kind: REG_ROUTINE_FLAGS, data: &mut [u8]) -> Option<usize> {
    let mut length = u32::try_from(data.len()).ok()?;
    // SAFETY: the key and value names are NUL-terminated literals, `data` is writable for
    // `length` bytes and `length` outlives the call.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            CPU_KEY,
            name,
            kind,
            None,
            Some(data.as_mut_ptr().cast()),
            Some(&mut length),
        )
    };
    (status == ERROR_SUCCESS).then_some(length as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::SystemInformation::GROUP_AFFINITY;
    use windows::Win32::System::Threading::{GetCurrentThread, GetThreadGroupAffinity};

    fn current_affinity() -> (u16, usize) {
        let mut affinity = GROUP_AFFINITY::default();
        // SAFETY: the pseudo handle is always valid and `affinity` is a live out value.
        let read = unsafe { GetThreadGroupAffinity(GetCurrentThread(), &mut affinity) };
        assert!(read.as_bool());
        (affinity.Group, affinity.Mask)
    }

    #[test]
    fn topology_is_self_consistent() {
        let topology = topology();
        let count = topology.processors.len();
        assert!(count >= 1);
        assert!(topology.processors.windows(2).all(|pair| pair[0] < pair[1]));
        // Every logical processor belongs to exactly one core and one package.
        for members in [
            topology.cores.iter().flat_map(|c| c.processors.clone()).collect::<Vec<u32>>(),
            topology.packages.iter().flatten().copied().collect(),
        ] {
            let mut sorted = members;
            sorted.sort_unstable();
            assert_eq!(sorted, (0..count as u32).collect::<Vec<_>>());
        }
        assert!(topology.caches.iter().all(|c| !c.processors.is_empty() && c.size_kb > 0));
    }

    #[test]
    fn sampling_on_the_calling_thread_restores_its_affinity() {
        let before = current_affinity();
        let topology = topology();
        let dumps = sample_pinned(&topology);
        assert_eq!(current_affinity(), before);
        assert!(!dumps.is_empty() && dumps.len() <= topology.processors.len());
        assert!(dumps.windows(2).all(|pair| pair[0].processor < pair[1].processor));
        assert!(dumps.iter().all(|dump| dump.leaves.first().is_some_and(|l| (l.leaf, l.subleaf) == (0, 0))));
    }

    #[test]
    fn sampling_through_the_helper_leaves_the_caller_alone() {
        let before = current_affinity();
        let topology = topology();
        assert_eq!(thread_dumps(&topology).len(), sample_pinned(&topology).len());
        assert_eq!(current_affinity(), before);
        // No topology at all still yields one dump.
        assert_eq!(thread_dumps(&OsTopology::default()).len(), 1);
    }

    #[test]
    fn registry_values_are_plausible_when_present() {
        assert!(update_revision().map_or(true, |raw| raw.len() <= 8));
        assert!(firmware_mhz().map_or(true, |mhz| mhz > 0));
    }
}
