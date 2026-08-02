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
//!
//! # A cut that is not uniform
//!
//! A [`Cut`] carries the detail of the picture as a whole together with an
//! override per boundary, so the reader opens the one package under study
//! down to its items while the rest of the project stays whole, and closes a
//! noisy package back into a single box while the rest stays open.
//!
//! An override on a boundary governs everything the boundary contains, never
//! the boundary itself: what a boundary is, its own surroundings decide.
//! The rules, in order:
//!
//! - The detail governing an element is the override of its nearest visible
//!   ancestor that carries one, and the global detail when no ancestor does.
//!   The nearest one wins, so an override inside another override refines it.
//! - An element is visible when the detail governing it covers its kind.
//! - An override on an invisible boundary is ignored.
//!
//! The last rule is what keeps a picture well formed. The kind levels nest -
//! every detail that shows modules shows packages, every detail that shows
//! items shows both - so a boundary hidden by the detail governing it hides
//! its whole subtree, and no box ever floats free of the frame the sources
//! put it in. Only an override below a hidden boundary could break that, by
//! making a descendant finer than the ancestor that should hold it; ignoring
//! it removes the one exception. A hidden boundary keeps its override for
//! the moment it becomes visible again.
//!
//! Overrides that name elements the graph does not hold are ignored. An
//! override is a reading preference rather than a claim about the sources,
//! and re-inspecting a repository routinely removes elements; rejecting a
//! stale id would leave the reader with no picture at all over a boundary
//! that no longer exists.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{
    ArchitectureGraph, Element, ElementId, ElementKind, ElementName, GraphError, Relation,
    RelationKind,
};

/// How deep into the containment hierarchy a boundary view reaches.
///
/// The order is the hierarchy: one detail is deeper than another when it is
/// greater, so opening a boundary to reach two elements at once asks for the
/// greater of the two details each of them needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Detail {
    Packages,
    Modules,
    Items,
}

impl Detail {
    /// Coarsest first; sliders and level pickers walk this order.
    pub const ALL: [Detail; 3] = [Detail::Packages, Detail::Modules, Detail::Items];

    /// One step further into the hierarchy; None at the deepest.
    pub fn deeper(self) -> Option<Self> {
        match self {
            Self::Packages => Some(Self::Modules),
            Self::Modules => Some(Self::Items),
            Self::Items => None,
        }
    }

    /// One step back toward the whole; None at the coarsest.
    pub fn shallower(self) -> Option<Self> {
        match self {
            Self::Packages => None,
            Self::Modules => Some(Self::Packages),
            Self::Items => Some(Self::Modules),
        }
    }

    /// The coarsest detail that shows a boundary of this kind, and None for
    /// a kind no detail ever shows. Opening a picture down to one element
    /// asks this of every step on the way to it: the boundary above the step
    /// must show at least this much for the step to appear at all.
    pub fn showing(kind: ElementKind) -> Option<Self> {
        Self::ALL.into_iter().find(|detail| detail.shows(kind))
    }

    /// Whether a boundary of this kind shows at this detail. The levels
    /// nest: a detail that shows one level shows every level above it, so a
    /// hidden boundary never holds a visible one.
    fn shows(self, kind: ElementKind) -> bool {
        match self {
            Self::Packages => kind == ElementKind::Package,
            Self::Modules => matches!(kind, ElementKind::Package | ElementKind::Module),
            Self::Items => matches!(
                kind,
                ElementKind::Package
                    | ElementKind::Module
                    | ElementKind::Function
                    | ElementKind::Type
            ),
        }
    }
}

/// Where the containment hierarchy is cut: one detail for the picture as a
/// whole, and a detail of its own for the inside of single boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cut {
    /// What everything follows while no override above it says otherwise.
    pub detail: Detail,
    /// Boundary -> the detail governing everything inside that boundary.
    pub overrides: BTreeMap<ElementId, Detail>,
}

impl Cut {
    /// One detail for the whole picture.
    pub fn uniform(detail: Detail) -> Self {
        Self {
            detail,
            overrides: BTreeMap::new(),
        }
    }

    /// Opens one boundary a step deeper than the detail governing its
    /// contents now. Answers whether the cut changed: a boundary that
    /// already shows its items, and one the view holds nothing for, stay as
    /// they are.
    pub fn expand(&mut self, view: &BoundaryView, boundary: &ElementId) -> bool {
        self.step(view, boundary, Detail::deeper)
    }

    /// Closes one boundary a step back toward a single box. Answers whether
    /// the cut changed, as [`Cut::expand`] does.
    pub fn collapse(&mut self, view: &BoundaryView, boundary: &ElementId) -> bool {
        self.step(view, boundary, Detail::shallower)
    }

    fn step(
        &mut self,
        view: &BoundaryView,
        boundary: &ElementId,
        step: fn(Detail) -> Option<Detail>,
    ) -> bool {
        let Some(detail) = view.detail_within.get(boundary).copied().and_then(step) else {
            return false;
        };
        self.overrides.insert(boundary.clone(), detail);
        true
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
    /// Boundary -> the detail governing what it shows inside it. Expanding
    /// the boundary deepens this detail and collapsing it coarsens it. The
    /// synthetic `self` leaves hold nothing, so they appear here not at all.
    pub detail_within: BTreeMap<ElementId, Detail>,
}

/// Cuts `graph` where the cut asks for.
pub fn boundary_view(graph: &ArchitectureGraph, cut: &Cut) -> Result<BoundaryView, LensError> {
    let parents = containment_parents(graph)?;
    let contexts = contexts(graph, &parents, cut);
    let visible: BTreeSet<ElementId> = graph
        .elements()
        .filter(|element| {
            contexts
                .get(&element.id)
                .is_some_and(|detail| detail.shows(element.kind))
        })
        .map(|element| element.id.clone())
        .collect();
    let detail_within: BTreeMap<ElementId, Detail> = visible
        .iter()
        .map(|id| (id.clone(), within(graph, cut, &contexts, id)))
        .collect();

    let boundary_of = |id: &ElementId| -> Option<ElementId> {
        let mut current = Some(id.clone());
        while let Some(id) = current {
            if visible.contains(&id) {
                return Some(id);
            }
            current = parents.get(&id).cloned();
        }
        None
    };

    let mut view = ArchitectureGraph::new();
    for element in graph.elements() {
        if visible.contains(&element.id) {
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

    let RolledUp {
        provenance,
        coarse,
        unscoped,
        self_leaves,
    } = roll_up(graph, &frames, boundary_of);

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
        detail_within,
    })
}

/// Every dependency of one cut, gathered from the concrete relations.
struct RolledUp {
    provenance: BTreeMap<Relation, BTreeSet<Relation>>,
    coarse: BTreeMap<Relation, BTreeSet<Relation>>,
    unscoped: BTreeSet<Relation>,
    /// The frames whose own content answers a dependency, and which
    /// therefore grow a `self` leaf to carry it.
    self_leaves: BTreeSet<ElementId>,
}

/// Rolls every concrete dependency up to the boundaries that carry it.
fn roll_up(
    graph: &ArchitectureGraph,
    frames: &BTreeSet<ElementId>,
    boundary_of: impl Fn(&ElementId) -> Option<ElementId>,
) -> RolledUp {
    let mut rolled = RolledUp {
        provenance: BTreeMap::new(),
        coarse: BTreeMap::new(),
        unscoped: BTreeSet::new(),
        self_leaves: BTreeSet::new(),
    };
    for relation in graph.relations() {
        if relation.kind != RelationKind::DependsOn {
            continue;
        }
        let (Some(from), Some(to)) = (boundary_of(&relation.from), boundary_of(&relation.to))
        else {
            rolled.unscoped.insert(relation.clone());
            continue;
        };
        if from == to {
            continue;
        }
        // A concrete relation that names a frame as its target depends on
        // the frame as a whole: no leaf can carry it at this detail.
        if frames.contains(&to) && relation.to == to {
            rolled
                .coarse
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
        let from = attached(from, frames, &mut rolled.self_leaves);
        let to = attached(to, frames, &mut rolled.self_leaves);
        rolled
            .provenance
            .entry(Relation {
                from,
                to,
                kind: RelationKind::DependsOn,
            })
            .or_default()
            .insert(relation.clone());
    }
    rolled
}

/// The detail governing each element of the graph, resolved from the
/// outermost element inward: every element follows the detail governing the
/// inside of its containment parent.
fn contexts(
    graph: &ArchitectureGraph,
    parents: &BTreeMap<ElementId, ElementId>,
    cut: &Cut,
) -> BTreeMap<ElementId, Detail> {
    let mut contexts: BTreeMap<ElementId, Detail> = BTreeMap::new();
    for element in graph.elements() {
        let mut ancestry = Vec::new();
        let mut current = Some(element.id.clone());
        while let Some(id) = current {
            if contexts.contains_key(&id) {
                break;
            }
            current = parents.get(&id).cloned();
            ancestry.push(id);
        }
        // Outermost first: a context follows from the one enclosing it.
        for id in ancestry.into_iter().rev() {
            let context = match parents.get(&id) {
                None => cut.detail,
                Some(parent) => within(graph, cut, &contexts, parent),
            };
            contexts.insert(id, context);
        }
    }
    contexts
}

/// The detail governing everything inside a boundary: the boundary's own
/// override where the cut holds one and the boundary is visible, else the
/// detail governing the boundary itself.
///
/// An override on an invisible boundary is ignored. Honoring it would show
/// elements below a boundary the cut hides, and those have no visible frame
/// to sit in.
fn within(
    graph: &ArchitectureGraph,
    cut: &Cut,
    contexts: &BTreeMap<ElementId, Detail>,
    boundary: &ElementId,
) -> Detail {
    let context = contexts.get(boundary).copied().unwrap_or(cut.detail);
    let visible = graph
        .element(boundary)
        .is_some_and(|element| context.shows(element.kind));
    match cut.overrides.get(boundary) {
        Some(detail) if visible => *detail,
        _ => context,
    }
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

/// What a frame's id gains to name the leaf that carries the frame's own
/// content. No source element ever ends this way: an item id ends in
/// `#<kind>:<name>`.
const SELF_LEAF_MARK: &str = "#self";

/// Whether the id names a frame's own content rather than a boundary the
/// sources declare. A view holds these synthetic leaves beside the real
/// boundaries, and a reader deserves to see the difference.
pub fn is_self_leaf(id: &ElementId) -> bool {
    id.as_str().ends_with(SELF_LEAF_MARK)
}

fn self_leaf_id(frame: &ElementId) -> ElementId {
    ElementId::new(format!("{frame}{SELF_LEAF_MARK}")).expect("the id extends a non-empty id")
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

    fn id(text: &str) -> ElementId {
        ElementId::new(text).unwrap()
    }

    fn cut<const N: usize>(detail: Detail, overrides: [(&str, Detail); N]) -> Cut {
        Cut {
            detail,
            overrides: overrides
                .into_iter()
                .map(|(boundary, within)| (id(boundary), within))
                .collect(),
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
        let view = boundary_view(&fixture(), &Cut::uniform(Detail::Packages)).unwrap();
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
        let view = boundary_view(&graph, &Cut::uniform(Detail::Packages)).unwrap();
        assert_eq!(
            view.provenance.len(),
            1,
            "only the cross-package edge remains"
        );
    }

    #[test]
    fn boundaries_nest_under_their_nearest_enclosing_boundary() {
        let view = boundary_view(&fixture(), &Cut::uniform(Detail::Modules)).unwrap();
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
        let view = boundary_view(&graph, &Cut::uniform(Detail::Modules)).unwrap();

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
    fn a_frames_own_content_is_recognisable_from_its_id_alone() {
        let mut graph = fixture();
        graph
            .add_relation(relation("a/lib", "b/lib", RelationKind::DependsOn))
            .unwrap();
        let view = boundary_view(&graph, &Cut::uniform(Detail::Modules)).unwrap();

        for element in view.graph.elements() {
            assert_eq!(
                is_self_leaf(&element.id),
                element.name.as_str() == "self",
                "{}",
                element.id
            );
        }
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
        let view = boundary_view(&graph, &Cut::uniform(Detail::Modules)).unwrap();

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
        let view = boundary_view(&graph, &Cut::uniform(Detail::Modules)).unwrap();

        let hidden = relation("package:a", "package:b", RelationKind::DependsOn);
        assert!(view.graph.relations().all(|r| *r != hidden));
        assert_eq!(
            view.coarse[&hidden],
            BTreeSet::from([relation("package:a", "package:b", RelationKind::DependsOn)])
        );

        let coarser = boundary_view(&graph, &Cut::uniform(Detail::Packages)).unwrap();
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
        let view = boundary_view(&graph, &Cut::uniform(Detail::Modules)).unwrap();

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
        let view = boundary_view(&graph, &Cut::uniform(Detail::Items)).unwrap();

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
        let view = boundary_view(&graph, &Cut::uniform(Detail::Packages)).unwrap();
        assert_eq!(
            view.unscoped,
            BTreeSet::from([relation("stray", "b/lib", RelationKind::DependsOn)])
        );
    }

    /// The fixture with one function declared inside each module of
    /// package:a.
    fn fixture_with_items() -> ArchitectureGraph {
        let mut graph = fixture();
        for (module, item) in [
            ("a/lib", "a/lib#function:near"),
            ("a/util", "a/util#function:go"),
        ] {
            graph
                .add_element(element(item, ElementKind::Function))
                .unwrap();
            graph
                .add_relation(relation(module, item, RelationKind::Contains))
                .unwrap();
        }
        graph
    }

    #[test]
    fn an_override_opens_one_boundary_deeper_than_the_view() {
        let view = boundary_view(
            &fixture(),
            &cut(Detail::Packages, [("package:a", Detail::Modules)]),
        )
        .unwrap();

        assert!(view.graph.element(&id("a/lib")).is_some());
        assert!(view.graph.element(&id("a/util")).is_some());
        assert!(
            view.graph.element(&id("b/lib")).is_none(),
            "the package beside it stays whole"
        );
    }

    #[test]
    fn an_override_collapses_one_boundary_below_the_view() {
        let view = boundary_view(
            &fixture(),
            &cut(Detail::Modules, [("package:a", Detail::Packages)]),
        )
        .unwrap();

        assert!(view.graph.element(&id("a/lib")).is_none());
        assert!(view.graph.element(&id("a/util")).is_none());
        assert!(
            view.graph.element(&id("b/lib")).is_some(),
            "the package beside it keeps its modules"
        );
        let rolled = relation("package:a", "b/lib", RelationKind::DependsOn);
        assert!(view.graph.relations().any(|r| *r == rolled));
    }

    #[test]
    fn a_deeper_override_wins_inside_a_shallower_one() {
        let view = boundary_view(
            &fixture_with_items(),
            &cut(
                Detail::Packages,
                [("package:a", Detail::Modules), ("a/util", Detail::Items)],
            ),
        )
        .unwrap();

        assert!(
            view.graph.element(&id("a/util#function:go")).is_some(),
            "the nearest override governs a/util's contents"
        );
        assert!(
            view.graph.element(&id("a/lib#function:near")).is_none(),
            "the shallower override still governs everything else inside package:a"
        );
    }

    #[test]
    fn an_override_inside_a_collapsed_boundary_leaves_its_contents_hidden() {
        let view = boundary_view(
            &fixture_with_items(),
            &cut(
                Detail::Items,
                [("package:a", Detail::Packages), ("a/lib", Detail::Items)],
            ),
        )
        .unwrap();

        assert!(view.graph.element(&id("a/lib")).is_none());
        assert!(
            view.graph.element(&id("a/util")).is_none(),
            "nothing below a hidden boundary reaches the picture"
        );
        assert!(view.graph.element(&id("a/lib#function:near")).is_none());
    }

    #[test]
    fn edges_reattach_to_the_boundaries_the_overrides_expose() {
        let mut graph = fixture();
        graph
            .add_element(element("package:c", ElementKind::Package))
            .unwrap();
        graph
            .add_element(element("c/util", ElementKind::Module))
            .unwrap();
        for (from, to) in [("project", "package:c"), ("package:c", "c/util")] {
            graph
                .add_relation(relation(from, to, RelationKind::Contains))
                .unwrap();
        }
        graph
            .add_relation(relation("c/util", "b/lib", RelationKind::DependsOn))
            .unwrap();

        let view = boundary_view(
            &graph,
            &cut(Detail::Packages, [("package:a", Detail::Modules)]),
        )
        .unwrap();

        let exposed = relation("a/util", "package:b", RelationKind::DependsOn);
        assert_eq!(
            view.provenance[&exposed],
            BTreeSet::from([relation("a/util", "b/lib", RelationKind::DependsOn)]),
            "the expanded package answers with the module the dependency leaves"
        );
        let rolled = relation("package:c", "package:b", RelationKind::DependsOn);
        assert_eq!(
            view.provenance[&rolled],
            BTreeSet::from([relation("c/util", "b/lib", RelationKind::DependsOn)]),
            "the same dependency stays rolled up in the package beside it"
        );
    }

    #[test]
    fn a_dependency_on_a_collapsed_boundary_draws_again() {
        let mut graph = fixture();
        graph
            .add_relation(relation("package:a", "package:b", RelationKind::DependsOn))
            .unwrap();
        let edge = relation("package:a", "package:b", RelationKind::DependsOn);

        let open = boundary_view(&graph, &Cut::uniform(Detail::Modules)).unwrap();
        assert!(
            open.coarse.contains_key(&edge),
            "package:b shows its modules, so no leaf can carry the edge"
        );

        let collapsed = boundary_view(
            &graph,
            &cut(Detail::Modules, [("package:b", Detail::Packages)]),
        )
        .unwrap();
        assert!(collapsed.coarse.is_empty());
        let drawn = relation("package:a#self", "package:b", RelationKind::DependsOn);
        assert!(
            collapsed.graph.relations().any(|r| *r == drawn),
            "a collapsed package is a leaf, and a leaf carries the edge"
        );
    }

    #[test]
    fn expanding_a_boundary_opens_it_one_detail_step_at_a_time() {
        let graph = fixture_with_items();
        let mut cut = Cut::uniform(Detail::Packages);

        let view = boundary_view(&graph, &cut).unwrap();
        assert!(cut.expand(&view, &id("package:a")));
        let view = boundary_view(&graph, &cut).unwrap();
        assert!(view.graph.element(&id("a/util")).is_some());
        assert!(view.graph.element(&id("a/util#function:go")).is_none());

        assert!(cut.expand(&view, &id("package:a")));
        let view = boundary_view(&graph, &cut).unwrap();
        assert!(view.graph.element(&id("a/util#function:go")).is_some());
        assert!(
            !cut.expand(&view, &id("package:a")),
            "the items are the deepest a boundary opens"
        );
    }

    #[test]
    fn a_boundary_showing_a_single_box_cannot_collapse_further() {
        let graph = fixture();
        let mut cut = Cut::uniform(Detail::Packages);
        let view = boundary_view(&graph, &cut).unwrap();
        assert!(!cut.collapse(&view, &id("package:a")));
        assert_eq!(cut, Cut::uniform(Detail::Packages));
    }

    #[test]
    fn an_override_naming_no_element_changes_nothing() {
        let plain = boundary_view(&fixture(), &Cut::uniform(Detail::Packages)).unwrap();
        let stale = boundary_view(
            &fixture(),
            &cut(Detail::Packages, [("package:gone", Detail::Items)]),
        )
        .unwrap();
        assert_eq!(plain, stale);
    }

    #[test]
    fn the_coarsest_detail_showing_a_kind_is_the_first_one_that_shows_it() {
        assert_eq!(
            Detail::showing(ElementKind::Package),
            Some(Detail::Packages)
        );
        assert_eq!(Detail::showing(ElementKind::Module), Some(Detail::Modules));
        assert_eq!(Detail::showing(ElementKind::Function), Some(Detail::Items));
        assert_eq!(Detail::showing(ElementKind::Type), Some(Detail::Items));
    }

    #[test]
    fn a_project_is_the_whole_picture_and_so_shows_at_no_detail() {
        assert_eq!(Detail::showing(ElementKind::Project), None);
    }

    #[test]
    fn a_detail_reaching_deeper_into_the_hierarchy_is_the_greater_one() {
        assert!(Detail::Items > Detail::Modules);
        assert!(Detail::Modules > Detail::Packages);
    }

    #[test]
    fn an_element_with_two_containers_is_rejected() {
        let mut graph = fixture();
        graph
            .add_relation(relation("package:b", "a/util", RelationKind::Contains))
            .unwrap();
        assert!(matches!(
            boundary_view(&graph, &Cut::uniform(Detail::Packages)),
            Err(LensError::AmbiguousContainment { .. })
        ));
    }
}
