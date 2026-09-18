//! The cross-program invocation graph.
//!
//! [`crate::executor::CpiCall`] is the wire form — flat, recorded per
//! invocation, serialized into every replay record and therefore frozen. This
//! module is a *derived view* over it, not a replacement: changing the wire
//! form would invalidate every existing bundle for no gain.
//!
//! What the view adds is the parent relationship the flat list only implies,
//! and typed access to a node's identity. What it deliberately does not do is
//! collapse anything. An existing invariant holds here unchanged: **an
//! invocation's shape is program, depth, owning instruction, discriminant,
//! account count and data length — all six.** Comparing `program@depth` alone
//! once reported a candidate that changed which instruction it called, and how
//! many accounts it passed, as identical.

use crate::executor::CpiCall;

/// An invocation's full identity: program, depth, owning instruction,
/// discriminant, account count, data length.
///
/// Named rather than spelled out at each use so the six parts stay six parts.
/// Comparing `program@depth` alone once reported a candidate that changed which
/// instruction it called, and how many accounts it passed, as identical.
pub type InvocationShape<'a> = (&'a str, u8, u8, Option<u8>, u8, u32);

/// One invocation, with its position in the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpiNode {
    /// Index into [`CpiGraph::nodes`], stable and deterministic.
    pub index: usize,
    /// Parent invocation, or `None` for one invoked directly by a top-level
    /// instruction.
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub program: String,
    /// 1 is a top-level instruction; 2 is its direct CPI.
    pub depth: u8,
    /// Which top-level instruction this sits under.
    pub instruction_index: u8,
    /// The instruction's leading discriminant byte, where it has data.
    pub discriminant: Option<u8>,
    pub account_count: u8,
    pub data_len: u32,
}

impl CpiNode {
    /// The six-part identity an invocation is compared by.
    ///
    /// Any comparison that uses fewer parts is the defect this tuple exists to
    /// prevent. Returned as a tuple so a caller cannot accidentally compare a
    /// subset.
    pub fn shape(&self) -> InvocationShape<'_> {
        (
            self.program.as_str(),
            self.depth,
            self.instruction_index,
            self.discriminant,
            self.account_count,
            self.data_len,
        )
    }

    pub fn origin(&self) -> super::InvocationOrigin {
        super::InvocationOrigin {
            instruction_index: self.instruction_index,
            depth: self.depth,
        }
    }
}

/// Every invocation one execution made, as a tree.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CpiGraph {
    pub nodes: Vec<CpiNode>,
}

impl CpiGraph {
    /// Build the tree from the recorded flat list.
    ///
    /// Parenting rule: an invocation's parent is the most recent earlier
    /// invocation under the *same top-level instruction* at exactly one less
    /// depth. Scoping to the owning instruction is what stops a sibling
    /// instruction's invocation from adopting it — the defect that turns a
    /// two-instruction transaction's graph into a plausible-looking lie.
    ///
    /// Input order is the recorded execution order, and the output preserves
    /// it, so the graph is deterministic for a given execution.
    pub fn build(calls: &[CpiCall]) -> Self {
        let mut nodes: Vec<CpiNode> = Vec::with_capacity(calls.len());
        for (index, call) in calls.iter().enumerate() {
            let parent = nodes
                .iter()
                .rev()
                .find(|candidate| {
                    candidate.instruction_index == call.outer_index
                        && candidate.depth + 1 == call.stack_height
                })
                .map(|candidate| candidate.index);
            nodes.push(CpiNode {
                index,
                parent,
                children: Vec::new(),
                program: call.program.clone(),
                depth: call.stack_height,
                instruction_index: call.outer_index,
                discriminant: call.discriminant,
                account_count: call.account_count,
                data_len: call.data_len,
            });
        }
        for index in 0..nodes.len() {
            if let Some(parent) = nodes[index].parent {
                nodes[parent].children.push(index);
            }
        }
        Self { nodes }
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Invocations with no parent: those a top-level instruction made directly.
    pub fn roots(&self) -> impl Iterator<Item = &CpiNode> {
        self.nodes.iter().filter(|node| node.parent.is_none())
    }

    pub fn children_of(&self, index: usize) -> impl Iterator<Item = &CpiNode> {
        self.nodes
            .get(index)
            .into_iter()
            .flat_map(|node| node.children.iter())
            .filter_map(|child| self.nodes.get(*child))
    }

    /// Every distinct program this execution reached.
    pub fn programs(&self) -> std::collections::BTreeSet<&str> {
        self.nodes
            .iter()
            .map(|node| node.program.as_str())
            .collect()
    }

    /// The full six-part shape of every invocation, in execution order.
    ///
    /// This is the comparison a differential gate makes. Two executions whose
    /// shape lists are equal made the same calls, in the same order, with the
    /// same arguments' sizes.
    pub fn shapes(&self) -> Vec<InvocationShape<'_>> {
        self.nodes.iter().map(CpiNode::shape).collect()
    }

    /// Deepest invocation depth reached, or 0 for none.
    pub fn max_depth(&self) -> u8 {
        self.nodes.iter().map(|node| node.depth).max().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(program: &str, depth: u8, outer: u8, discriminant: u8, accounts: u8) -> CpiCall {
        CpiCall {
            program: program.into(),
            stack_height: depth,
            outer_index: outer,
            account_count: accounts,
            data_len: 9,
            discriminant: Some(discriminant),
        }
    }

    #[test]
    fn no_invocations_is_an_empty_graph_not_a_missing_one() {
        let graph = CpiGraph::build(&[]);
        assert!(graph.is_empty());
        assert_eq!(graph.len(), 0);
        assert_eq!(graph.max_depth(), 0);
        assert!(graph.programs().is_empty());
    }

    #[test]
    fn one_invocation_is_a_root() {
        let graph = CpiGraph::build(&[call("token", 2, 0, 7, 3)]);
        assert_eq!(graph.len(), 1);
        assert_eq!(graph.roots().count(), 1);
        assert_eq!(graph.nodes[0].parent, None);
        assert_eq!(graph.max_depth(), 2);
    }

    #[test]
    fn a_nested_invocation_is_attached_to_its_caller() {
        let graph = CpiGraph::build(&[call("pool", 2, 0, 14, 10), call("token", 3, 0, 7, 4)]);
        assert_eq!(graph.nodes[1].parent, Some(0));
        assert_eq!(graph.nodes[0].children, vec![1]);
        assert_eq!(graph.children_of(0).count(), 1);
        assert_eq!(graph.max_depth(), 3);
    }

    #[test]
    fn siblings_share_a_parent_and_keep_their_order() {
        let graph = CpiGraph::build(&[
            call("pool", 2, 0, 14, 10),
            call("system", 3, 0, 2, 2),
            call("token", 3, 0, 7, 4),
        ]);
        assert_eq!(graph.nodes[1].parent, Some(0));
        assert_eq!(graph.nodes[2].parent, Some(0));
        assert_eq!(graph.nodes[0].children, vec![1, 2]);
        // Order is execution order, and it is part of the evidence.
        assert_eq!(
            graph
                .children_of(0)
                .map(|n| n.program.as_str())
                .collect::<Vec<_>>(),
            vec!["system", "token"]
        );
    }

    /// The defect this scoping prevents: an invocation under instruction 1
    /// must not adopt a parent from instruction 0.
    #[test]
    fn a_child_never_attaches_across_top_level_instructions() {
        let graph = CpiGraph::build(&[
            call("pool", 2, 0, 14, 10),
            // A different top-level instruction. Depth 3 here has no parent in
            // instruction 0, however recent that invocation was.
            call("token", 3, 1, 7, 4),
        ]);
        assert_eq!(
            graph.nodes[1].parent, None,
            "instruction 1's invocation must not adopt instruction 0's frame"
        );
        assert_eq!(graph.nodes[0].children, Vec::<usize>::new());
        assert_eq!(graph.roots().count(), 2);
    }

    #[test]
    fn an_unknown_program_is_kept_with_its_raw_identity() {
        let graph = CpiGraph::build(&[call("SomeUnknownProgram1111111", 2, 0, 200, 6)]);
        assert_eq!(graph.nodes[0].program, "SomeUnknownProgram1111111");
        assert_eq!(graph.nodes[0].discriminant, Some(200));
        assert_eq!(graph.nodes[0].account_count, 6);
        assert_eq!(graph.nodes[0].data_len, 9);
    }

    /// The invariant that fixes the shape tuple at six parts.
    #[test]
    fn two_invocations_differing_only_in_arguments_have_different_shapes() {
        let base = CpiGraph::build(&[call("token", 2, 0, 7, 3)]);
        for changed in [
            call("token", 2, 0, 3, 3),  // a different instruction
            call("token", 2, 0, 7, 9),  // a different account count
            call("token", 3, 0, 7, 3),  // a different depth
            call("token", 2, 1, 7, 3),  // a different owning instruction
            call("system", 2, 0, 7, 3), // a different program
        ] {
            assert_ne!(
                base.shapes(),
                CpiGraph::build(std::slice::from_ref(&changed)).shapes(),
                "{changed:?} must not compare equal"
            );
        }
        // And the data length, the sixth part.
        let mut longer = call("token", 2, 0, 7, 3);
        longer.data_len = 10;
        assert_ne!(base.shapes(), CpiGraph::build(&[longer]).shapes());
    }

    #[test]
    fn the_graph_is_deterministic_for_a_given_execution() {
        let calls = vec![
            call("pool", 2, 0, 14, 10),
            call("system", 3, 0, 2, 2),
            call("token", 3, 0, 7, 4),
        ];
        assert_eq!(CpiGraph::build(&calls), CpiGraph::build(&calls));
    }

    #[test]
    fn raw_payload_size_and_discriminant_survive_the_view() {
        let mut raw = call("token", 2, 0, 7, 4);
        raw.data_len = 1_234;
        let graph = CpiGraph::build(&[raw]);
        assert_eq!(graph.nodes[0].data_len, 1_234);
        assert_eq!(graph.nodes[0].shape().5, 1_234);
        assert_eq!(
            graph.nodes[0].origin(),
            crate::evidence::InvocationOrigin {
                instruction_index: 0,
                depth: 2
            }
        );
    }

    #[test]
    fn programs_are_reported_as_a_set_without_collapsing_the_nodes() {
        let graph = CpiGraph::build(&[
            call("token", 2, 0, 7, 3),
            call("token", 2, 0, 3, 4),
            call("system", 2, 0, 2, 2),
        ]);
        assert_eq!(
            graph.programs().into_iter().collect::<Vec<_>>(),
            vec!["system", "token"]
        );
        // Two distinct token invocations remain two nodes.
        assert_eq!(graph.len(), 3);
        assert_eq!(graph.shapes().len(), 3);
    }
}
