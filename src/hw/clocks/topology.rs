//! Where the logical processors sit - processor groups and NUMA nodes - and
//! how the instances of the processor performance counters map onto them.

use std::mem::offset_of;

use windows::Win32::System::SystemInformation::{
    RelationNumaNode, RelationNumaNodeEx, NUMA_NODE_RELATIONSHIP,
};

use crate::hw::processors::{
    group_masks, group_sizes, records, relations, u16_at, u32_at, GroupMask, ProcId,
};

/// How the instances of the "Processor Information" counters are named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Naming {
    /// `node,index`: NUMA node and position among the node's processors.
    NodeIndex,
    /// `group,number`: processor group and number inside it.
    GroupNumber,
}

/// Processor groups and NUMA nodes of the machine.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Layout {
    /// Active processors in each processor group.
    pub groups: Vec<u32>,
    /// NUMA nodes and their processors, ascending.
    pub nodes: Vec<(u32, Vec<ProcId>)>,
}

impl Layout {
    /// Queries the layout. Parts Windows does not report stay empty.
    pub fn detect() -> Layout {
        let nodes = relations().map(|buffer| numa_nodes(&buffer)).unwrap_or_default();
        Layout { groups: group_sizes(), nodes }
    }

    /// Every active logical processor, group by group.
    pub fn processors(&self) -> impl Iterator<Item = ProcId> + '_ {
        (0u16..).zip(&self.groups).flat_map(|(group, &count)| {
            (0..count.min(256)).map(move |number| ProcId { group, number: number as u8 })
        })
    }

    /// Picks the naming scheme that explains every instance. On most machines
    /// both do and agree; where they differ `node,index` is tried first.
    pub fn naming(&self, instances: &[(u32, u32)]) -> Option<Naming> {
        [Naming::NodeIndex, Naming::GroupNumber]
            .into_iter()
            .find(|&naming| instances.iter().all(|&instance| self.processor(naming, instance).is_some()))
    }

    /// The processor a counter instance stands for under `naming`.
    pub fn processor(&self, naming: Naming, (first, second): (u32, u32)) -> Option<ProcId> {
        match naming {
            Naming::NodeIndex => {
                let (_, processors) = self.nodes.iter().find(|(node, _)| *node == first)?;
                processors.get(usize::try_from(second).ok()?).copied()
            }
            Naming::GroupNumber => {
                let count = *self.groups.get(usize::try_from(first).ok()?)?;
                let id = ProcId { group: u16::try_from(first).ok()?, number: u8::try_from(second).ok()? };
                (second < count).then_some(id)
            }
        }
    }
}

/// NUMA nodes by number, read from a relations buffer. The records of a node
/// that spans several processor groups are merged; a malformed one is left out.
fn numa_nodes(buffer: &[u8]) -> Vec<(u32, Vec<ProcId>)> {
    let node_records = records(buffer)
        .filter(|(relationship, _)| [RelationNumaNode.0, RelationNumaNodeEx.0].contains(relationship))
        .filter_map(|(_, body)| node_record(body));
    let mut nodes: Vec<(u32, Vec<ProcId>)> = Vec::new();
    for (number, members) in node_records {
        match nodes.iter_mut().find(|(node, _)| *node == number) {
            Some((_, processors)) => processors.extend(members),
            None => nodes.push((number, members)),
        }
    }
    for (_, processors) in &mut nodes {
        processors.sort_unstable();
        processors.dedup();
    }
    nodes.sort_unstable_by_key(|(node, _)| *node);
    nodes
}

/// Node number and processors of one NUMA node record.
fn node_record(body: &[u8]) -> Option<(u32, Vec<ProcId>)> {
    let number = u32_at(body, offset_of!(NUMA_NODE_RELATIONSHIP, NodeNumber))?;
    // Older systems leave the group count at zero and store exactly one mask.
    let count = u16_at(body, offset_of!(NUMA_NODE_RELATIONSHIP, GroupCount))?.max(1);
    let masks = group_masks(body, offset_of!(NUMA_NODE_RELATIONSHIP, Anonymous), count)?;
    Some((number, masks.into_iter().flat_map(GroupMask::processors).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::processors::fixtures::{put_masks, record};
    use windows::Win32::System::SystemInformation::{RelationCache, RelationProcessorCore};

    fn id(group: u16, number: u8) -> ProcId {
        ProcId { group, number }
    }

    /// A record laid out like a NUMA node's. `count_field` is what its group
    /// count says, whatever the number of masks that follow.
    fn node(relationship: i32, number: u32, count_field: u16, masks: &[(u16, u64)]) -> Vec<u8> {
        let mut body = Vec::new();
        put_masks(&mut body, offset_of!(NUMA_NODE_RELATIONSHIP, Anonymous), masks);
        let number_at = offset_of!(NUMA_NODE_RELATIONSHIP, NodeNumber);
        body[number_at..number_at + 4].copy_from_slice(&number.to_le_bytes());
        let count_at = offset_of!(NUMA_NODE_RELATIONSHIP, GroupCount);
        body[count_at..count_at + 2].copy_from_slice(&count_field.to_le_bytes());
        record(relationship, body)
    }

    #[test]
    fn reads_the_node_records_among_the_others() {
        let mut buffer = node(RelationProcessorCore.0, 7, 1, &[(0, 0b0011)]);
        buffer.extend(node(RelationNumaNode.0, 3, 1, &[(1, 0b0101)]));
        buffer.extend(node(RelationCache.0, 8, 1, &[(0, 0xFF)]));
        buffer.extend(node(RelationNumaNodeEx.0, 4, 2, &[(0, 0b1), (2, 0b10)]));
        assert_eq!(numa_nodes(&buffer), vec![(3, vec![id(1, 0), id(1, 2)]), (4, vec![id(0, 0), id(2, 1)])]);
        assert!(numa_nodes(&[]).is_empty());
    }

    #[test]
    fn zero_group_count_still_carries_one_mask() {
        let buffer = node(RelationNumaNode.0, 0, 0, &[(0, 0b1010)]);
        assert_eq!(numa_nodes(&buffer), vec![(0, vec![id(0, 1), id(0, 3)])]);
    }

    #[test]
    fn node_record_with_missing_masks_is_left_out() {
        // The first record announces two masks and holds one.
        let mut buffer = node(RelationNumaNode.0, 0, 2, &[(0, 0b01)]);
        buffer.extend(node(RelationNumaNode.0, 1, 1, &[(0, 0b10)]));
        assert_eq!(numa_nodes(&buffer), vec![(1, vec![id(0, 1)])]);
        // Cut short, the second record ends the walk; what came before it stands.
        let whole = node(RelationNumaNode.0, 0, 1, &[(0, 0b01)]);
        let mut buffer = whole.clone();
        buffer.extend(&whole[..whole.len() - 1]);
        assert_eq!(numa_nodes(&buffer), vec![(0, vec![id(0, 0)])]);
    }

    #[test]
    fn merges_node_records_across_groups() {
        let mut buffer = node(RelationNumaNode.0, 1, 0, &[(1, 0b11)]);
        buffer.extend(node(RelationNumaNode.0, 0, 2, &[(0, 0b11), (2, 0b1)]));
        buffer.extend(node(RelationNumaNode.0, 1, 0, &[(3, 0b1)]));
        assert_eq!(
            numa_nodes(&buffer),
            vec![(0, vec![id(0, 0), id(0, 1), id(2, 0)]), (1, vec![id(1, 0), id(1, 1), id(3, 0)])]
        );
    }

    #[test]
    fn enumerates_processors_group_by_group() {
        let layout = Layout { groups: vec![2, 1], nodes: Vec::new() };
        assert_eq!(layout.processors().collect::<Vec<_>>(), vec![id(0, 0), id(0, 1), id(1, 0)]);
        assert_eq!(Layout::default().processors().count(), 0);
    }

    fn two_nodes_one_group() -> Layout {
        Layout {
            groups: vec![8],
            nodes: vec![(0, (0..4).map(|n| id(0, n)).collect()), (1, (4..8).map(|n| id(0, n)).collect())],
        }
    }

    #[test]
    fn resolves_node_index_names() {
        let layout = two_nodes_one_group();
        let instances = [(0, 0), (0, 3), (1, 0), (1, 3)];
        assert_eq!(layout.naming(&instances), Some(Naming::NodeIndex));
        assert_eq!(layout.processor(Naming::NodeIndex, (1, 3)), Some(id(0, 7)));
        assert_eq!(layout.processor(Naming::NodeIndex, (1, 4)), None);
        assert_eq!(layout.processor(Naming::NodeIndex, (2, 0)), None);
    }

    #[test]
    fn falls_back_to_group_number_names() {
        let layout = two_nodes_one_group();
        let instances = [(0, 0), (0, 7)];
        assert_eq!(layout.naming(&instances), Some(Naming::GroupNumber));
        assert_eq!(layout.processor(Naming::GroupNumber, (0, 7)), Some(id(0, 7)));
        assert_eq!(layout.processor(Naming::GroupNumber, (0, 8)), None);
        assert_eq!(layout.processor(Naming::GroupNumber, (1, 0)), None);
    }

    #[test]
    fn unexplained_names_have_no_scheme() {
        let layout = two_nodes_one_group();
        assert_eq!(layout.naming(&[(0, 0), (5, 9)]), None);
        assert_eq!(Layout::default().naming(&[(0, 0)]), None);
    }

    #[test]
    fn single_node_agrees_under_both_schemes() {
        let layout = Layout { groups: vec![4], nodes: vec![(0, (0..4).map(|n| id(0, n)).collect())] };
        for number in 0..4u8 {
            let instance = (0, u32::from(number));
            assert_eq!(layout.processor(Naming::NodeIndex, instance), Some(id(0, number)));
            assert_eq!(layout.processor(Naming::GroupNumber, instance), Some(id(0, number)));
        }
    }

    #[test]
    fn detection_is_self_consistent() {
        let layout = Layout::detect();
        assert!(layout.processors().count() >= 1);
        // The nodes list their processors in order and no processor twice.
        let mut listed = 0;
        for (_, processors) in &layout.nodes {
            assert!(processors.windows(2).all(|pair| pair[0] < pair[1]));
            listed += processors.len();
        }
        let distinct: std::collections::HashSet<ProcId> =
            layout.nodes.iter().flat_map(|(_, processors)| processors.iter().copied()).collect();
        assert_eq!(distinct.len(), listed);
    }
}
