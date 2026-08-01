//! Lenses: views of an architecture at a chosen abstraction level.
//!
//! The boundary lens cuts the containment hierarchy at a set of element
//! kinds. Every element maps to its nearest enclosing boundary, and every
//! dependency between elements rolls up to a dependency between their
//! boundaries. A rolled-up edge remembers the concrete relations it stands
//! for: severing the edge means severing exactly those.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{
    ArchitectureGraph, ElementId, ElementKind, GraphError, Relation, RelationKind,
};

/// An architecture viewed at boundary level.
///
/// `graph` holds the boundary elements, the `Contains` nesting between them,
/// and the rolled-up `DependsOn` edges. It is a plain [`ArchitectureGraph`],
/// so redlining and comparison apply to it unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryView {
    pub graph: ArchitectureGraph,
    /// Rolled-up `DependsOn` edge -> the concrete relations it aggregates.
    pub provenance: BTreeMap<Relation, BTreeSet<Relation>>,
    /// Concrete `DependsOn` relations with an endpoint outside every
    /// boundary; they appear in no rolled-up edge.
    pub unscoped: BTreeSet<Relation>,
}

/// Cuts `graph` at the elements whose kind is in `boundary_kinds`.
pub fn boundary_view(
    graph: &ArchitectureGraph,
    boundary_kinds: &BTreeSet<ElementKind>,
) -> Result<BoundaryView, LensError> {
    let parents = containment_parents(graph)?;

    let boundary_of = |id: &ElementId| -> Option<ElementId> {
        let mut current = Some(id.clone());
        while let Some(id) = current {
            let element = graph.element(&id)?;
            if boundary_kinds.contains(&element.kind) {
                return Some(id);
            }
            current = parents.get(&id).cloned();
        }
        None
    };

    let mut view = ArchitectureGraph::new();
    for element in graph.elements() {
        if boundary_kinds.contains(&element.kind) {
            view.add_element(element.clone())
                .expect("source elements are unique");
        }
    }

    // Nesting between boundaries: each boundary attaches to the nearest
    // boundary strictly above it.
    let mut nesting = BTreeSet::new();
    for element in view.elements() {
        let above = parents.get(&element.id).and_then(&boundary_of);
        if let Some(above) = above {
            nesting.insert(Relation {
                from: above,
                to: element.id.clone(),
                kind: RelationKind::Contains,
            });
        }
    }

    let mut provenance: BTreeMap<Relation, BTreeSet<Relation>> = BTreeMap::new();
    let mut unscoped = BTreeSet::new();
    for relation in graph.relations() {
        if relation.kind != RelationKind::DependsOn {
            continue;
        }
        let (Some(from), Some(to)) = (boundary_of(&relation.from), boundary_of(&relation.to))
        else {
            unscoped.insert(relation.clone());
            continue;
        };
        if from == to {
            continue;
        }
        provenance
            .entry(Relation {
                from,
                to,
                kind: RelationKind::DependsOn,
            })
            .or_default()
            .insert(relation.clone());
    }

    for relation in nesting.into_iter().chain(provenance.keys().cloned()) {
        view.add_relation(relation)?;
    }

    Ok(BoundaryView {
        graph: view,
        provenance,
        unscoped,
    })
}

/// The containment parent of every contained element. Fails when containment
/// is not a tree: an element with two parents makes "nearest enclosing
/// boundary" ambiguous.
fn containment_parents(
    graph: &ArchitectureGraph,
) -> Result<BTreeMap<ElementId, ElementId>, LensError> {
    let mut parents = BTreeMap::new();
    for relation in graph.relations() {
        if relation.kind != RelationKind::Contains {
            continue;
        }
        if let Some(existing) = parents.insert(relation.to.clone(), relation.from.clone()) {
            return Err(LensError::AmbiguousContainment {
                element: relation.to.clone(),
                parents: [existing, relation.from.clone()],
            });
        }
    }
    // A cycle would make the ancestor walk endless; reject it outright.
    for start in parents.keys() {
        let mut seen = BTreeSet::from([start.clone()]);
        let mut current = parents.get(start);
        while let Some(parent) = current {
            if !seen.insert(parent.clone()) {
                return Err(LensError::CyclicContainment {
                    element: parent.clone(),
                });
            }
            current = parents.get(parent);
        }
    }
    Ok(parents)
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum LensError {
    #[error("element {element} is contained by both {} and {}", parents[0], parents[1])]
    AmbiguousContainment {
        element: ElementId,
        parents: [ElementId; 2],
    },
    #[error("containment of {element} is cyclic")]
    CyclicContainment { element: ElementId },
    #[error("the rolled-up view is inconsistent")]
    Inconsistent {
        #[from]
        source: GraphError,
    },
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{Element, ElementName};

    use super::*;

    fn element(id: &str, kind: ElementKind) -> Element {
        Element {
            id: ElementId::new(id).unwrap(),
            name: ElementName::new(id).unwrap(),
            kind,
        }
    }

    fn relation(from: &str, to: &str, kind: RelationKind) -> Relation {
        Relation {
            from: ElementId::new(from).unwrap(),
            to: ElementId::new(to).unwrap(),
            kind,
        }
    }

    /// project ⊃ {package:a ⊃ a/lib ⊃ a/util, package:b ⊃ b/lib}, with
    /// a/util depending on b/lib.
    fn fixture() -> ArchitectureGraph {
        let mut graph = ArchitectureGraph::new();
        graph
            .add_element(element("project", ElementKind::Project))
            .unwrap();
        graph
            .add_element(element("package:a", ElementKind::Package))
            .unwrap();
        graph
            .add_element(element("package:b", ElementKind::Package))
            .unwrap();
        graph
            .add_element(element("a/lib", ElementKind::Module))
            .unwrap();
        graph
            .add_element(element("a/util", ElementKind::Module))
            .unwrap();
        graph
            .add_element(element("b/lib", ElementKind::Module))
            .unwrap();
        for (from, to) in [
            ("project", "package:a"),
            ("project", "package:b"),
            ("package:a", "a/lib"),
            ("a/lib", "a/util"),
            ("package:b", "b/lib"),
        ] {
            graph
                .add_relation(relation(from, to, RelationKind::Contains))
                .unwrap();
        }
        graph
            .add_relation(relation("a/util", "b/lib", RelationKind::DependsOn))
            .unwrap();
        graph
    }

    fn kinds(kinds: &[ElementKind]) -> BTreeSet<ElementKind> {
        kinds.iter().copied().collect()
    }

    #[test]
    fn module_dependencies_roll_up_to_their_packages() {
        let view = boundary_view(&fixture(), &kinds(&[ElementKind::Package])).unwrap();
        let rolled = relation("package:a", "package:b", RelationKind::DependsOn);
        assert!(view.graph.relations().any(|r| *r == rolled));
        assert_eq!(
            view.provenance[&rolled],
            BTreeSet::from([relation("a/util", "b/lib", RelationKind::DependsOn)])
        );
    }

    #[test]
    fn dependencies_within_one_boundary_disappear_from_the_view() {
        let mut graph = fixture();
        graph
            .add_relation(relation("a/util", "a/lib", RelationKind::DependsOn))
            .unwrap();
        let view = boundary_view(&graph, &kinds(&[ElementKind::Package])).unwrap();
        assert_eq!(
            view.provenance.len(),
            1,
            "only the cross-package edge remains"
        );
    }

    #[test]
    fn boundaries_nest_under_their_nearest_enclosing_boundary() {
        let view = boundary_view(
            &fixture(),
            &kinds(&[ElementKind::Package, ElementKind::Module]),
        )
        .unwrap();
        let nested = relation("a/lib", "a/util", RelationKind::Contains);
        let across = relation("package:a", "a/lib", RelationKind::Contains);
        assert!(view.graph.relations().any(|r| *r == nested));
        assert!(view.graph.relations().any(|r| *r == across));
    }

    #[test]
    fn a_dependency_outside_every_boundary_is_reported_as_unscoped() {
        let mut graph = fixture();
        graph
            .add_element(element("stray", ElementKind::Module))
            .unwrap();
        graph
            .add_relation(relation("project", "stray", RelationKind::Contains))
            .unwrap();
        graph
            .add_relation(relation("stray", "b/lib", RelationKind::DependsOn))
            .unwrap();
        let view = boundary_view(&graph, &kinds(&[ElementKind::Package])).unwrap();
        assert_eq!(
            view.unscoped,
            BTreeSet::from([relation("stray", "b/lib", RelationKind::DependsOn)])
        );
    }

    #[test]
    fn an_element_with_two_containers_is_rejected() {
        let mut graph = fixture();
        graph
            .add_relation(relation("package:b", "a/util", RelationKind::Contains))
            .unwrap();
        assert!(matches!(
            boundary_view(&graph, &kinds(&[ElementKind::Package])),
            Err(LensError::AmbiguousContainment { .. })
        ));
    }
}
