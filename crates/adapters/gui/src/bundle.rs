//! What several dependency edges draw as one stroke.
//!
//! Once a summary block stands for a whole subtree, every edge that reached
//! anything inside it reaches the block instead. Dozens of them then run
//! between the same two blocks, each still a line of its own, and the
//! picture answers a question about two boundaries with a pile of parallel
//! straws. One stroke per pair of boxes says the same thing and says how
//! much of it there is: the stroke carries the weight of everything it
//! draws, so its width already reads as the traffic.
//!
//! Only what the architecture carries and the plan leaves untouched
//! bundles. A severed, partly severed, or drawn edge is the plan speaking:
//! it is rare, it is the reason the reader opened the picture, and a line
//! nobody can pick out is a line nobody can answer for. Such an edge
//! therefore keeps a stroke of its own however much runs beside it.
//!
//! The bundling is pure - the edges, the substitution the summary made, and
//! the weights go in and the draw list comes out - so it is unit-testable
//! without a screen.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use cutaway_architecture::ElementId;

use crate::canvas::{EdgeStatus, EdgeVisual};
use crate::focus::Strength;

/// One stroke of the picture and the edges it draws.
pub(crate) struct Bundle {
    /// The boxes the stroke runs between: the endpoints of its edges, with
    /// the summary block in place of whatever the picture hides.
    pub(crate) from: ElementId,
    pub(crate) to: ElementId,
    /// The edges drawn as this one stroke, in the order they arrived. Never
    /// empty.
    pub(crate) members: Vec<usize>,
    /// The member the stroke answers for: it gives the stroke its status and
    /// its name, and a click on the stroke selects it. The heaviest member
    /// is the one the reader most likely means, and it is the one the panel
    /// beside the picture already names for a partner reached several ways.
    pub(crate) lead: usize,
    /// The concrete dependencies all the members stand for together.
    pub(crate) weight: usize,
}

impl Bundle {
    /// Whether the stroke draws more than the one edge it answers for.
    pub(crate) fn merged(&self) -> bool {
        self.members.len() > 1
    }

    /// How strongly the stroke paints: the strongest strength any member
    /// would paint at. The stroke is the only mark those edges have left in
    /// the picture, so it fades only once everything it draws has faded.
    pub(crate) fn strength(&self, of: impl Fn(usize) -> Strength) -> Strength {
        self.members
            .iter()
            .copied()
            .map(of)
            .max()
            .unwrap_or(Strength::Focused)
    }

    /// Whether any member of the stroke satisfies a question about an edge.
    pub(crate) fn any(&self, is: impl Fn(usize) -> bool) -> bool {
        self.members.iter().copied().any(is)
    }
}

/// The strokes that draw a frame's edges, in the order the edges arrive.
///
/// `stands_for` names, for every element the picture does not paint, the box
/// that stands for it - the same substitution the routing lands the edge
/// on. Two edges share a stroke once that substitution leaves them running
/// between the same two boxes and both belong to the architecture as it
/// stands. An empty map therefore bundles nothing: without a block to
/// gather them, no two edges can share a pair of endpoints.
pub(crate) fn bundles(
    edges: &[EdgeVisual],
    stands_for: &BTreeMap<ElementId, ElementId>,
) -> Vec<Bundle> {
    let attached = |id: &ElementId| stands_for.get(id).unwrap_or(id).clone();
    let mut bundles: Vec<Bundle> = Vec::new();
    let mut strokes: BTreeMap<(ElementId, ElementId), usize> = BTreeMap::new();
    for (index, edge) in edges.iter().enumerate() {
        let from = attached(&edge.relation.from);
        let to = attached(&edge.relation.to);
        let alone = Bundle {
            from: from.clone(),
            to: to.clone(),
            members: vec![index],
            lead: index,
            weight: edge.weight,
        };
        // An edge no block gathered still runs between the boxes it names,
        // and the plan's own edges never hide in a crowd.
        let gathered = edge.status == EdgeStatus::Existing
            && (from != edge.relation.from || to != edge.relation.to);
        if !gathered {
            bundles.push(alone);
            continue;
        }
        match strokes.entry((from, to)) {
            Entry::Vacant(slot) => {
                slot.insert(bundles.len());
                bundles.push(alone);
            }
            Entry::Occupied(slot) => {
                let stroke = &mut bundles[*slot.get()];
                stroke.members.push(index);
                stroke.weight += edge.weight;
                if edge.weight > edges[stroke.lead].weight {
                    stroke.lead = index;
                }
            }
        }
    }
    bundles
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{Relation, RelationKind};

    use super::*;

    fn id(text: &str) -> ElementId {
        ElementId::new(text).unwrap()
    }

    fn edge(from: &str, to: &str, weight: usize, status: EdgeStatus) -> EdgeVisual {
        EdgeVisual {
            relation: Relation {
                from: id(from),
                to: id(to),
                kind: RelationKind::DependsOn,
            },
            status,
            annotated: false,
            weight,
        }
    }

    fn existing(from: &str, to: &str, weight: usize) -> EdgeVisual {
        edge(from, to, weight, EdgeStatus::Existing)
    }

    /// Two blocks, each standing for two of the boxes the edges name.
    fn blocked() -> BTreeMap<ElementId, ElementId> {
        BTreeMap::from([
            (id("a/one"), id("package:a")),
            (id("a/two"), id("package:a")),
            (id("b/one"), id("package:b")),
            (id("b/two"), id("package:b")),
        ])
    }

    fn relations(bundle: &Bundle, edges: &[EdgeVisual]) -> Vec<Relation> {
        bundle
            .members
            .iter()
            .map(|member| edges[*member].relation.clone())
            .collect()
    }

    #[test]
    fn edges_between_the_same_blocks_merge_into_one() {
        let edges = [
            existing("a/one", "b/one", 3),
            existing("a/two", "b/one", 1),
            existing("a/two", "b/two", 5),
        ];

        let bundles = bundles(&edges, &blocked());
        assert_eq!(bundles.len(), 1, "one stroke joins the two blocks");
        assert_eq!(bundles[0].from, id("package:a"));
        assert_eq!(bundles[0].to, id("package:b"));
        assert_eq!(bundles[0].members, vec![0, 1, 2]);
        assert_eq!(
            bundles[0].weight, 9,
            "the stroke stands for everything it draws"
        );
    }

    #[test]
    fn a_merged_edge_answers_for_its_heaviest_member() {
        let edges = [
            existing("a/one", "b/one", 3),
            existing("a/two", "b/two", 5),
            existing("a/two", "b/one", 1),
        ];

        let bundles = bundles(&edges, &blocked());
        assert_eq!(bundles[0].lead, 1);
        assert!(bundles[0].merged());
    }

    #[test]
    fn a_severed_edge_never_disappears_into_a_merge() {
        let edges = [
            existing("a/one", "b/one", 3),
            edge("a/two", "b/two", 4, EdgeStatus::Severed),
            existing("a/two", "b/one", 1),
        ];

        let bundles = bundles(&edges, &blocked());
        assert_eq!(bundles.len(), 2);
        assert_eq!(
            relations(&bundles[1], &edges),
            vec![edges[1].relation.clone()],
            "the planned removal keeps a stroke of its own"
        );
        assert_eq!(relations(&bundles[0], &edges).len(), 2);
        assert_eq!(
            bundles[0].weight, 4,
            "a stroke counts what it draws and nothing beside it"
        );
    }

    #[test]
    fn a_drawn_edge_never_disappears_into_a_merge() {
        let edges = [
            existing("a/one", "b/one", 3),
            edge("a/two", "b/two", 1, EdgeStatus::Drawn),
        ];

        let bundles = bundles(&edges, &blocked());
        assert_eq!(bundles.len(), 2);
        assert_eq!(
            relations(&bundles[1], &edges),
            vec![edges[1].relation.clone()]
        );
    }

    #[test]
    fn edges_no_block_gathered_keep_their_own_strokes() {
        let edges = [existing("a/one", "b/one", 3), existing("a/two", "b/two", 1)];

        let bundles = bundles(&edges, &BTreeMap::new());
        assert_eq!(
            bundles.len(),
            2,
            "a picture that paints every box merges nothing"
        );
        assert!(!bundles[0].merged());
        assert_eq!(bundles[0].from, id("a/one"));
    }

    #[test]
    fn edges_reaching_different_blocks_keep_their_own_strokes() {
        let mut stands_for = blocked();
        stands_for.insert(id("c/one"), id("package:c"));
        let edges = [existing("a/one", "b/one", 3), existing("a/two", "c/one", 1)];

        let bundles = bundles(&edges, &stands_for);
        assert_eq!(bundles.len(), 2);
        assert_eq!(bundles[1].to, id("package:c"));
    }

    #[test]
    fn a_merged_edge_lights_while_any_member_lights() {
        let edges = [
            existing("a/one", "b/one", 3),
            existing("a/two", "b/two", 1),
            existing("a/two", "b/one", 1),
        ];

        let bundles = bundles(&edges, &blocked());
        let only_the_second = |member: usize| {
            if member == 1 {
                Strength::Focused
            } else {
                Strength::Faded
            }
        };
        assert_eq!(bundles[0].strength(only_the_second), Strength::Focused);
        assert_eq!(bundles[0].strength(|_| Strength::Faded), Strength::Faded);
    }
}
