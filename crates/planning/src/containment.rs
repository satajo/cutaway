//! Walking the containment of an architecture.
//!
//! Several planning queries read a graph the same way: an entry names one
//! boundary and means everything the boundary holds. The walk climbs the
//! `Contains` relations from an element outward, so it answers "does this
//! lie inside that" for elements at any depth.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{ArchitectureGraph, ElementId, RelationKind};

/// The containment parent of every contained element of a graph. A walk
/// that climbs the containment of a whole graph asks this once instead of
/// searching the relations per step.
pub(crate) type Parents = BTreeMap<ElementId, ElementId>;

pub(crate) fn containment_parents(base: &ArchitectureGraph) -> Parents {
    base.relations()
        .filter(|relation| relation.kind == RelationKind::Contains)
        .map(|relation| (relation.to.clone(), relation.from.clone()))
        .collect()
}

/// Whether an element is the boundary itself or sits anywhere inside it.
pub(crate) fn lies_within(element: &ElementId, boundary: &ElementId, parents: &Parents) -> bool {
    // Containment is a tree in every graph a lens accepts, but a walk that
    // trusts that and meets a cycle never ends; the seen set bounds it.
    let mut seen = BTreeSet::new();
    let mut current = Some(element);
    while let Some(id) = current {
        if id == boundary {
            return true;
        }
        if !seen.insert(id.clone()) {
            return false;
        }
        current = parents.get(id);
    }
    false
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{Element, ElementKind, ElementName, Relation};

    use super::*;

    fn id(text: &str) -> ElementId {
        ElementId::new(text).unwrap()
    }

    /// package:a ⊃ a/lib ⊃ a/lib#type:X, and package:b beside it.
    fn base() -> ArchitectureGraph {
        let mut graph = ArchitectureGraph::new();
        for element in ["package:a", "package:b", "a/lib", "a/lib#type:X"] {
            graph
                .add_element(Element::of_kind(
                    id(element),
                    ElementKind::Module,
                    ElementName::new(element).unwrap(),
                ))
                .unwrap();
        }
        for (from, to) in [("package:a", "a/lib"), ("a/lib", "a/lib#type:X")] {
            graph
                .add_relation(Relation {
                    from: id(from),
                    to: id(to),
                    kind: RelationKind::Contains,
                })
                .unwrap();
        }
        graph
    }

    #[test]
    fn a_boundary_holds_everything_below_it_however_deep() {
        let parents = containment_parents(&base());
        assert!(lies_within(&id("a/lib#type:X"), &id("package:a"), &parents));
        assert!(lies_within(&id("a/lib"), &id("package:a"), &parents));
    }

    #[test]
    fn a_boundary_holds_itself() {
        let parents = containment_parents(&base());
        assert!(lies_within(&id("package:a"), &id("package:a"), &parents));
    }

    #[test]
    fn a_boundary_holds_nothing_beside_it() {
        let parents = containment_parents(&base());
        assert!(!lies_within(&id("package:b"), &id("package:a"), &parents));
        assert!(!lies_within(&id("package:a"), &id("a/lib"), &parents));
    }
}
