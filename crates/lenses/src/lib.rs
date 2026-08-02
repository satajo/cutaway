//! Lenses: views of an architecture at an adjustable level of detail.
//!
//! The boundary lens cuts the containment hierarchy at a [`Detail`] level:
//! packages, the modules within them, or the individual items within the
//! modules. Every element maps to its nearest enclosing boundary, and every
//! dependency between elements rolls up to a dependency between boundaries.
//! A rolled-up edge remembers the concrete relations it stands for: severing
//! the edge means severing exactly those.
//!
//! Dependency edges attach only to boundaries without visible children, so
//! one picture never mixes two detail levels. A boundary that contains other
//! boundaries is a frame. Its own content - everything that rolls up to it
//! without falling into a visible child - appears as a synthetic `self` leaf
//! inside it (id `<frame>#self`). A dependency that names a frame as a whole
//! cannot attach to any leaf; it is reported in [`BoundaryView::coarse`] and
//! shows rolled-up at a coarser detail.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{
    ArchitectureGraph, Element, ElementId, ElementKind, ElementName, GraphError, Relation,
    RelationKind,
};

/// How deep into the containment hierarchy a boundary view reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    Packages,
    Modules,
    Items,
}

impl Detail {
    /// Coarsest first; sliders and level pickers walk this order.
    pub const ALL: [Detail; 3] = [Detail::Packages, Detail::Modules, Detail::Items];

    fn kinds(self) -> BTreeSet<ElementKind> {
        match self {
            Self::Packages => BTreeSet::from([ElementKind::Package]),
            Self::Modules => BTreeSet::from([ElementKind::Package, ElementKind::Module]),
            Self::Items => BTreeSet::from([
                ElementKind::Package,
                ElementKind::Module,
                ElementKind::Function,
                ElementKind::Type,
            ]),
        }
    }
}

/// An architecture viewed at boundary level.
///
/// `graph` holds the boundary elements, the `Contains` nesting between them,
/// and the rolled-up `DependsOn` edges. It is a plain [`ArchitectureGraph`],
/// so planning and comparison apply to it unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryView {
    pub graph: ArchitectureGraph,
    /// Rolled-up `DependsOn` edge -> the concrete relations it aggregates.
    pub provenance: BTreeMap<Relation, BTreeSet<Relation>>,
    /// Rolled-up edges that name a frame as a whole; drawing them beside the
    /// frame's children would mix two detail levels, so they stay off this
    /// view and show at a coarser detail. Keyed by the frame-level edge.
    pub coarse: BTreeMap<Relation, BTreeSet<Relation>>,
    /// Concrete `DependsOn` relations with an endpoint outside every
    /// boundary; they appear in no rolled-up edge.
    pub unscoped: BTreeSet<Relation>,
}

/// Cuts `graph` at the boundaries of the chosen detail level.
pub fn boundary_view(graph: &ArchitectureGraph, detail: Detail) -> Result<BoundaryView, LensError> {
    let boundary_kinds = detail.kinds();
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
    let frames: BTreeSet<ElementId> = nesting.iter().map(|r| r.from.clone()).collect();

    let mut provenance: BTreeMap<Relation, BTreeSet<Relation>> = BTreeMap::new();
    let mut coarse: BTreeMap<Relation, BTreeSet<Relation>> = BTreeMap::new();
    let mut unscoped = BTreeSet::new();
    let mut self_leaves: BTreeSet<ElementId> = BTreeSet::new();
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
        // A concrete relation that names a frame as its target depends on
        // the frame as a whole: no leaf can carry it at this detail.
        if frames.contains(&to) && relation.to == to {
            coarse
                .entry(Relation {
                    from,
                    to,
                    kind: RelationKind::DependsOn,
                })
                .or_default()
                .insert(relation.clone());
            continue;
        }
        // A concrete relation always originates in real content - source
        // text or a manifest - so a frame endpoint here means the frame's
        // own content, never the frame as a whole.
        let from = attached(from, &frames, &mut self_leaves);
        let to = attached(to, &frames, &mut self_leaves);
        provenance
            .entry(Relation {
                from,
                to,
                kind: RelationKind::DependsOn,
            })
            .or_default()
            .insert(relation.clone());
    }

    for frame in &self_leaves {
        let kind = view
            .element(frame)
            .expect("self leaves grow only on view elements")
            .kind;
        view.add_element(Element {
            id: self_leaf_id(frame),
            name: ElementName::new("self").expect("the self leaf name is never empty"),
            kind,
        })?;
        nesting.insert(Relation {
            from: frame.clone(),
            to: self_leaf_id(frame),
            kind: RelationKind::Contains,
        });
    }

    for relation in nesting.into_iter().chain(provenance.keys().cloned()) {
        view.add_relation(relation)?;
    }

    Ok(BoundaryView {
        graph: view,
        provenance,
        coarse,
        unscoped,
    })
}

/// Where a rolled-up endpoint attaches: the boundary itself when it is a
/// leaf, its `self` leaf when it is a frame.
fn attached(
    boundary: ElementId,
    frames: &BTreeSet<ElementId>,
    self_leaves: &mut BTreeSet<ElementId>,
) -> ElementId {
    if frames.contains(&boundary) {
        self_leaves.insert(boundary.clone());
        self_leaf_id(&boundary)
    } else {
        boundary
    }
}

fn self_leaf_id(frame: &ElementId) -> ElementId {
    ElementId::new(format!("{frame}#self")).expect("the id extends a non-empty id")
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

    #[test]
    fn module_dependencies_roll_up_to_their_packages() {
        let view = boundary_view(&fixture(), Detail::Packages).unwrap();
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
        let view = boundary_view(&graph, Detail::Packages).unwrap();
        assert_eq!(
            view.provenance.len(),
            1,
            "only the cross-package edge remains"
        );
    }

    #[test]
    fn boundaries_nest_under_their_nearest_enclosing_boundary() {
        let view = boundary_view(&fixture(), Detail::Modules).unwrap();
        let nested = relation("a/lib", "a/util", RelationKind::Contains);
        let across = relation("package:a", "a/lib", RelationKind::Contains);
        assert!(view.graph.relations().any(|r| *r == nested));
        assert!(view.graph.relations().any(|r| *r == across));
    }

    #[test]
    fn a_frames_own_dependencies_attach_to_its_self_leaf() {
        let mut graph = fixture();
        graph
            .add_relation(relation("a/lib", "b/lib", RelationKind::DependsOn))
            .unwrap();
        let view = boundary_view(&graph, Detail::Modules).unwrap();

        let rolled = relation("a/lib#self", "b/lib", RelationKind::DependsOn);
        assert!(view.graph.relations().any(|r| *r == rolled));
        assert_eq!(
            view.provenance[&rolled],
            BTreeSet::from([relation("a/lib", "b/lib", RelationKind::DependsOn)])
        );
        let self_leaf = view
            .graph
            .element(&ElementId::new("a/lib#self").unwrap())
            .expect("the frame grows a self leaf");
        assert_eq!(self_leaf.name.as_str(), "self");
        assert!(
            view.graph
                .relations()
                .any(|r| *r == relation("a/lib", "a/lib#self", RelationKind::Contains))
        );
    }

    #[test]
    fn no_dependency_edge_touches_a_frame() {
        let mut graph = fixture();
        graph
            .add_relation(relation("a/lib", "b/lib", RelationKind::DependsOn))
            .unwrap();
        graph
            .add_relation(relation("package:a", "package:b", RelationKind::DependsOn))
            .unwrap();
        let view = boundary_view(&graph, Detail::Modules).unwrap();

        let frames: BTreeSet<ElementId> = view
            .graph
            .relations()
            .filter(|r| r.kind == RelationKind::Contains)
            .map(|r| r.from.clone())
            .collect();
        for edge in view.provenance.keys() {
            assert!(!frames.contains(&edge.from), "{edge:?} leaves a frame");
            assert!(!frames.contains(&edge.to), "{edge:?} enters a frame");
        }
    }

    #[test]
    fn a_dependency_naming_a_frame_as_a_whole_waits_at_a_coarser_detail() {
        let mut graph = fixture();
        graph
            .add_relation(relation("package:a", "package:b", RelationKind::DependsOn))
            .unwrap();
        let view = boundary_view(&graph, Detail::Modules).unwrap();

        let hidden = relation("package:a", "package:b", RelationKind::DependsOn);
        assert!(view.graph.relations().all(|r| *r != hidden));
        assert_eq!(
            view.coarse[&hidden],
            BTreeSet::from([relation("package:a", "package:b", RelationKind::DependsOn)])
        );

        let coarser = boundary_view(&graph, Detail::Packages).unwrap();
        assert!(coarser.graph.relations().any(|r| *r == hidden));
    }

    #[test]
    fn a_dependency_into_a_frames_direct_content_attaches_to_its_self_leaf() {
        let mut graph = fixture();
        graph
            .add_element(element("b/util", ElementKind::Module))
            .unwrap();
        graph
            .add_element(element("b/lib#function:go", ElementKind::Function))
            .unwrap();
        for (from, to) in [("b/lib", "b/util"), ("b/lib", "b/lib#function:go")] {
            graph
                .add_relation(relation(from, to, RelationKind::Contains))
                .unwrap();
        }
        graph
            .add_relation(relation(
                "a/util",
                "b/lib#function:go",
                RelationKind::DependsOn,
            ))
            .unwrap();
        let view = boundary_view(&graph, Detail::Modules).unwrap();

        let rolled = relation("a/util", "b/lib#self", RelationKind::DependsOn);
        assert!(view.graph.relations().any(|r| *r == rolled));
        assert_eq!(
            view.provenance[&rolled],
            BTreeSet::from([relation(
                "a/util",
                "b/lib#function:go",
                RelationKind::DependsOn
            )])
        );
    }

    #[test]
    fn items_detail_shows_individual_declarations() {
        let mut graph = fixture();
        graph
            .add_element(element("b/lib#type:Thing", ElementKind::Type))
            .unwrap();
        graph
            .add_relation(relation(
                "b/lib",
                "b/lib#type:Thing",
                RelationKind::Contains,
            ))
            .unwrap();
        graph
            .add_relation(relation(
                "a/util",
                "b/lib#type:Thing",
                RelationKind::DependsOn,
            ))
            .unwrap();
        let view = boundary_view(&graph, Detail::Items).unwrap();

        assert!(
            view.graph
                .element(&ElementId::new("b/lib#type:Thing").unwrap())
                .is_some()
        );
        let edge = relation("a/util", "b/lib#type:Thing", RelationKind::DependsOn);
        assert!(view.graph.relations().any(|r| *r == edge));
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
        let view = boundary_view(&graph, Detail::Packages).unwrap();
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
            boundary_view(&graph, Detail::Packages),
            Err(LensError::AmbiguousContainment { .. })
        ));
    }
}
