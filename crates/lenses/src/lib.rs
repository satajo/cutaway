//! Lenses: views of an architecture cut along a frontier of open boundaries.
//!
//! The boundary lens draws the containment hierarchy as nested boxes, and a
//! [`Cut`] holds the two independent decisions that shape the picture:
//!
//! - The frontier - `open` - is the set of boundaries whose direct contents
//!   show. A boundary outside it stands as a single closed box, and opening
//!   a boundary reveals exactly one layer: its children arrive closed, and
//!   opening each of them is a step of its own.
//! - The vocabulary - `kinds` - is the set of element kinds the picture
//!   renders at all. A kind outside the vocabulary is transparent: its
//!   elements never draw, and what they contain hoists to the nearest
//!   rendered ancestor, so hiding modules pools their declarations directly
//!   in the package that holds them. The project root is transparent in the
//!   default vocabulary: the picture starts at the packages, with no box
//!   around the whole.
//!
//! Every dependency between elements rolls up to a dependency between the
//! boxes the picture draws. A rolled-up edge remembers the concrete
//! relations it stands for: severing the edge means severing exactly those.
//!
//! A dependency edge attaches each endpoint to the nearest rendered boundary
//! above it, leaf or frame alike. An edge that ends at a frame speaks about
//! the frame's own code or the frame as a whole; the edges into its parts
//! end at the parts. What passes between a boundary and its own contents
//! stays inside it: a relation whose two resolved boundaries stand in
//! containment - one holding the other - is the outer boundary's internal
//! wiring, not architecture, and it is dropped from the picture.
//!
//! # The frontier
//!
//! The rules, in order:
//!
//! - A boundary's contents show while the boundary is drawn and open.
//! - An open flag under a closed boundary stays latent: closing a boundary
//!   keeps the flags of everything inside it, so reopening it restores the
//!   reading the reader had built there.
//! - An open flag naming an element the graph does not hold is ignored. The
//!   frontier is a reading preference rather than a claim about the sources,
//!   and re-inspecting a repository routinely removes elements; rejecting a
//!   stale id would leave the reader with no picture at all.
//!
//! The latency rule is what keeps a picture well formed: nothing below a
//! closed or hidden boundary reaches the picture, so no box ever floats
//! free of the frame the sources put it in.
//!
//! # A cut scoped to one boundary
//!
//! An open frontier over the whole project puts everything in front of the
//! reader at once. A [`Cut`] therefore carries a scope: the one boundary
//! the picture is about. The rules, in order:
//!
//! - The scoped boundary is the root frame of the picture and always
//!   drawn, whatever the vocabulary says. The frames that contain it are
//!   not: nothing is drawn around the root of a picture, and what holds the
//!   scope is said in words beside it. Only the root's own flag governs its
//!   contents - the frontier above the root speaks about a picture the
//!   reader has left.
//! - What the scope contains shows exactly as the unscoped cut - frontier
//!   and vocabulary alike - would show it.
//! - A partner is an element outside the scope that a concrete dependency
//!   connects with one inside it, either way round, the dependencies naming
//!   a frame of the scope as a whole included. Each partner shows as the
//!   topmost boundary above it that does not contain the scope: the largest
//!   box disjoint from the scope, which is a sibling package for a partner
//!   in another package and a sibling boundary for one in the scope's own.
//!   Several partners behind one such box share it. A partner draws
//!   whatever the vocabulary says: the border of a scoped picture states
//!   who the scope talks to, and that answer must not vanish with a kind.
//! - Only the dependencies that reach into the scope enter the picture.
//!   What passes between two partners is about neither of them and leaves.
//! - A stub stands whole: every opening refuses it and its open flags are
//!   ignored. Looking inside a partner means scoping to that partner.
//!
//! A scope naming an element the graph does not hold is ignored, exactly as
//! a stale open flag is, and for the same reason.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{
    ArchitectureGraph, ElementId, ElementKind, GraphError, Relation, RelationKind,
};

/// Where the picture cuts the containment hierarchy: the frontier of open
/// boundaries, the vocabulary of kinds it renders, and the boundary the
/// picture is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cut {
    /// The frontier: boundaries whose direct contents show. Everything else
    /// stands as a single closed box. A flag under a closed boundary stays
    /// latent, and a flag naming no element is ignored.
    pub open: BTreeSet<ElementId>,
    /// The vocabulary: the element kinds the picture renders. A kind
    /// outside it is transparent - its elements never draw, and their
    /// contents hoist to the nearest rendered ancestor.
    pub kinds: BTreeSet<ElementKind>,
    /// The boundary the picture is about, and None for the whole project.
    /// A scope says where the reader stands rather than what the sources
    /// hold, so nothing outlives the reading that set it.
    pub scope: Option<ElementId>,
}

impl Default for Cut {
    fn default() -> Self {
        Self::whole()
    }
}

impl Cut {
    /// The whole project as closed boxes: nothing open, and every kind but
    /// the project root in the vocabulary. The project is the whole
    /// picture, so it stays transparent rather than boxing everything in
    /// one frame.
    #[must_use]
    pub fn whole() -> Self {
        Self {
            open: BTreeSet::new(),
            kinds: BTreeSet::from([
                ElementKind::Package,
                ElementKind::Directory,
                ElementKind::Module,
                ElementKind::File,
                ElementKind::Function,
                ElementKind::Type,
            ]),
            scope: None,
        }
    }

    /// Scopes the picture to one boundary, or - with None - back to the
    /// whole project. The frontier and the vocabulary stand either way: a
    /// reader who scopes the picture keeps the cut they were reading it at.
    pub fn focus(&mut self, scope: Option<ElementId>) {
        self.scope = scope;
    }

    /// Opens one boundary, revealing exactly one layer: its direct
    /// contents, closed. Answers whether the cut changed: a boundary
    /// already open, a stub, and one holding nothing the vocabulary shows
    /// stay as they are.
    pub fn expand(&mut self, view: &BoundaryView, boundary: &ElementId) -> bool {
        if !view.openable.contains(boundary) {
            return false;
        }
        self.open.insert(boundary.clone());
        true
    }

    /// Closes one boundary back into a single box. The flags inside it stay
    /// latent, so reopening it restores the reading made there. Answers
    /// whether the cut changed, as [`Cut::expand`] does.
    pub fn collapse(&mut self, view: &BoundaryView, boundary: &ElementId) -> bool {
        if !view.open.contains(boundary) {
            return false;
        }
        self.open.remove(boundary);
        true
    }

    /// Opens one boundary and everything beneath it, down to the deepest
    /// frame. `graph` is the architecture the view was cut from: the walk
    /// must reach what the picture does not draw yet. Answers whether the
    /// picture changes; a stub refuses as it refuses every opening.
    pub fn expand_fully(
        &mut self,
        view: &BoundaryView,
        graph: &ArchitectureGraph,
        boundary: &ElementId,
    ) -> bool {
        if view.stubs.contains(boundary) {
            return false;
        }
        let children = contained_children(graph);
        let mut subtree = BTreeSet::from([boundary.clone()]);
        let mut walking = vec![boundary.clone()];
        while let Some(frame) = walking.pop() {
            for child in children.get(&frame).into_iter().flatten() {
                if subtree.insert((*child).clone()) {
                    walking.push((*child).clone());
                }
            }
        }
        if !view.openable.iter().any(|id| subtree.contains(id)) {
            return false;
        }
        // A flag on a childless element would open nothing, ever; only the
        // frames of the subtree join the frontier.
        self.open
            .extend(subtree.into_iter().filter(|id| children.contains_key(id)));
        true
    }

    /// Opens every boundary the picture offers to open: the whole frontier
    /// steps one layer deeper. Answers whether the cut changed.
    pub fn expand_frontier(&mut self, view: &BoundaryView) -> bool {
        if view.openable.is_empty() {
            return false;
        }
        self.open.extend(view.openable.iter().cloned());
        true
    }

    /// Closes the innermost open boundaries - the ones holding no open
    /// boundary of their own - so the frontier steps one layer back.
    /// Answers whether the cut changed.
    pub fn collapse_frontier(&mut self, view: &BoundaryView) -> bool {
        let parents = rendered_parents(&view.graph);
        let mut holding_open: BTreeSet<ElementId> = BTreeSet::new();
        for id in &view.open {
            let mut current = parents.get(id);
            while let Some(parent) = current {
                if view.open.contains(parent) {
                    holding_open.insert(parent.clone());
                }
                current = parents.get(parent);
            }
        }
        let innermost: Vec<ElementId> = view
            .open
            .iter()
            .filter(|id| !holding_open.contains(*id))
            .cloned()
            .collect();
        if innermost.is_empty() {
            return false;
        }
        for id in &innermost {
            self.open.remove(id);
        }
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
    /// An edge that ends at a frame speaks about the frame's own code or the
    /// frame as a whole; the edges into its parts end at the parts.
    pub provenance: BTreeMap<Relation, BTreeSet<Relation>>,
    /// Concrete `DependsOn` relations with an endpoint outside every
    /// rendered boundary; they appear in no rolled-up edge.
    pub unscoped: BTreeSet<Relation>,
    /// The boundaries standing open: drawn, on the cut's frontier, and not
    /// a stub, so the picture shows their direct contents.
    pub open: BTreeSet<ElementId>,
    /// The boundaries the reader can open: drawn, closed, not a stub, and
    /// holding something the vocabulary would show.
    pub openable: BTreeSet<ElementId>,
    /// The partners standing at the border of a scoped picture, each whole
    /// and closed. They open to nothing: a reader looks inside one by
    /// scoping the picture to it. Empty while the cut scopes to nothing.
    pub stubs: BTreeSet<ElementId>,
}

impl BoundaryView {
    /// Every concrete relation one rendered edge answers for: what the lens
    /// rolled into it. Acting on the rendered edge - severing it, annotating
    /// it - means acting on all of them: the reader addresses the connection
    /// between two boundaries, not one attachment of it. Answers empty for
    /// an edge this view does not draw.
    #[must_use]
    pub fn concrete_behind(&self, edge: &Relation) -> BTreeSet<Relation> {
        self.provenance.get(edge).cloned().unwrap_or_default()
    }
}

/// Cuts `graph` where the cut asks for.
pub fn boundary_view(graph: &ArchitectureGraph, cut: &Cut) -> Result<BoundaryView, LensError> {
    let parents = containment_parents(graph)?;
    let scoped = Scoped::of(graph, &parents, cut.scope.as_ref());
    let root = scoped.as_ref().map(|scoped| scoped.root.clone());

    let mut contents_memo = BTreeMap::new();
    let mut visible = BTreeSet::new();
    for element in graph.elements() {
        let shown = cut.kinds.contains(&element.kind)
            && parents.get(&element.id).is_none_or(|parent| {
                shows_contents(
                    graph,
                    &parents,
                    cut,
                    root.as_ref(),
                    &mut contents_memo,
                    parent,
                )
            });
        let drawn = match &scoped {
            None => shown,
            Some(scoped) => scoped.draws(&element.id, shown),
        };
        if drawn {
            visible.insert(element.id.clone());
        }
    }

    let children = contained_children(graph);
    let stubs = scoped
        .as_ref()
        .map(|scoped| scoped.stubs.clone())
        .unwrap_or_default();
    let mut open = BTreeSet::new();
    let mut openable = BTreeSet::new();
    for id in &visible {
        if stubs.contains(id) {
            continue;
        }
        if cut.open.contains(id) {
            open.insert(id.clone());
        } else if reveals(graph, &children, &cut.kinds, id) {
            openable.insert(id.clone());
        }
    }

    let boundary_of = |id: &ElementId| -> Option<ElementId> {
        // Outside the scope only the stubs stand, and every element behind
        // one attaches to the stub that stands for it.
        if let Some(scoped) = &scoped
            && !scoped.holds(id)
        {
            return scoped.stub_of(&parents, id);
        }
        // The scoped boundary is always drawn, so a climb that starts
        // inside the scope never leaves it.
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
    // A scoped picture is about the scope, so the dependencies that reach
    // neither into nor out of it never enter the roll-up at all.
    let concrete = graph.relations().filter(|relation| {
        scoped
            .as_ref()
            .is_none_or(|scoped| scoped.touches(relation))
    });
    let RolledUp {
        provenance,
        unscoped,
    } = roll_up(concrete, &parents, boundary_of);

    for relation in nesting.into_iter().chain(provenance.keys().cloned()) {
        view.add_relation(relation)?;
    }

    Ok(BoundaryView {
        graph: view,
        provenance,
        unscoped,
        open,
        openable,
        stubs,
    })
}

/// Whether a boundary's direct contents show: the boundary sits on the
/// cut's frontier and its own place is exposed. A transparent boundary -
/// one whose kind the vocabulary leaves out - passes the question up: it
/// draws no box, so its contents stand wherever it stands. The scoped root
/// bottoms the walk: it is the root frame of the picture, so only its own
/// flag speaks and the frontier above it stays out of the answer.
fn shows_contents(
    graph: &ArchitectureGraph,
    parents: &BTreeMap<ElementId, ElementId>,
    cut: &Cut,
    root: Option<&ElementId>,
    memo: &mut BTreeMap<ElementId, bool>,
    id: &ElementId,
) -> bool {
    if let Some(answer) = memo.get(id) {
        return *answer;
    }
    let answer = if root == Some(id) {
        cut.open.contains(id)
    } else {
        let above = parents
            .get(id)
            .is_none_or(|parent| shows_contents(graph, parents, cut, root, memo, parent));
        let rendered = graph
            .element(id)
            .is_some_and(|element| cut.kinds.contains(&element.kind));
        if rendered {
            above && cut.open.contains(id)
        } else {
            above
        }
    };
    memo.insert(id.clone(), answer);
    answer
}

/// Whether opening a boundary would put anything new in the picture: it
/// holds an element of a rendered kind, directly or behind transparent
/// intermediates alone. What sits behind a rendered child stays out of the
/// answer - that child arrives closed, and opening it is a step of its own.
fn reveals(
    graph: &ArchitectureGraph,
    children: &BTreeMap<&ElementId, Vec<&ElementId>>,
    kinds: &BTreeSet<ElementKind>,
    id: &ElementId,
) -> bool {
    children.get(id).into_iter().flatten().any(|child| {
        let rendered = graph
            .element(child)
            .is_some_and(|element| kinds.contains(&element.kind));
        rendered || reveals(graph, children, kinds, child)
    })
}

/// The direct contents of every containing element.
fn contained_children(graph: &ArchitectureGraph) -> BTreeMap<&ElementId, Vec<&ElementId>> {
    let mut children: BTreeMap<&ElementId, Vec<&ElementId>> = BTreeMap::new();
    for relation in graph.relations() {
        if relation.kind == RelationKind::Contains {
            children
                .entry(&relation.from)
                .or_default()
                .push(&relation.to);
        }
    }
    children
}

/// The containment parent of every contained element of a rendered view.
/// The view nests as a tree by construction, so nothing is validated again.
fn rendered_parents(graph: &ArchitectureGraph) -> BTreeMap<ElementId, ElementId> {
    graph
        .relations()
        .filter(|relation| relation.kind == RelationKind::Contains)
        .map(|relation| (relation.to.clone(), relation.from.clone()))
        .collect()
}

/// The picture one scope narrows to: the scoped boundary with everything
/// inside it, and one closed stub per partner at its border.
struct Scoped {
    /// The scoped boundary, which is the root frame of the picture.
    root: ElementId,
    /// The root and everything containment-wise inside it.
    inside: BTreeSet<ElementId>,
    /// The boundaries that contain the root. The picture draws none of them:
    /// they hold the root, so no box disjoint from it can stand for them.
    above: BTreeSet<ElementId>,
    stubs: BTreeSet<ElementId>,
}

impl Scoped {
    /// What a cut scopes the picture to, and None while it scopes to nothing
    /// or names an element this graph does not hold.
    fn of(
        graph: &ArchitectureGraph,
        parents: &BTreeMap<ElementId, ElementId>,
        scope: Option<&ElementId>,
    ) -> Option<Self> {
        let root = scope?.clone();
        graph.element(&root)?;
        let above = ancestors(parents, &root);
        let inside = graph
            .elements()
            .map(|element| &element.id)
            .filter(|id| **id == root || ancestors(parents, id).contains(&root))
            .cloned()
            .collect();
        let mut scoped = Self {
            root,
            inside,
            above,
            stubs: BTreeSet::new(),
        };
        scoped.stubs = graph
            .relations()
            .filter(|relation| relation.kind == RelationKind::DependsOn)
            .filter_map(|relation| scoped.partner(relation))
            .filter_map(|partner| scoped.stub_of(parents, partner))
            .collect();
        Some(scoped)
    }

    /// Whether an element stands inside the scope.
    fn holds(&self, id: &ElementId) -> bool {
        self.inside.contains(id)
    }

    /// The element one dependency reaches outside the scope while its other
    /// end stands inside it. A dependency wholly inside the scope, and one
    /// wholly outside it, answer None.
    fn partner<'r>(&self, relation: &'r Relation) -> Option<&'r ElementId> {
        match (self.holds(&relation.from), self.holds(&relation.to)) {
            (true, false) => Some(&relation.to),
            (false, true) => Some(&relation.from),
            _ => None,
        }
    }

    /// The stub one element outside the scope stands as: the topmost
    /// boundary above it - itself, where nothing is above it - that does not
    /// contain the scope. That is the largest box disjoint from the scope.
    /// A boundary that contains the scope answers None: every box that would
    /// stand for it would hold the picture's own root.
    fn stub_of(
        &self,
        parents: &BTreeMap<ElementId, ElementId>,
        id: &ElementId,
    ) -> Option<ElementId> {
        if self.above.contains(id) {
            return None;
        }
        let mut stub = id;
        while let Some(parent) = parents.get(stub) {
            if self.above.contains(parent) {
                break;
            }
            stub = parent;
        }
        Some(stub.clone())
    }

    /// Whether the picture draws one element. `shown` is what the cut alone
    /// says about it: inside the scope that answer stands unchanged, the
    /// scoped boundary and the stubs stand whatever it says, and outside
    /// the scope the stubs stand alone.
    fn draws(&self, id: &ElementId, shown: bool) -> bool {
        *id == self.root || self.stubs.contains(id) || (shown && self.holds(id))
    }

    /// Whether one dependency reaches into the scope.
    fn touches(&self, relation: &Relation) -> bool {
        self.holds(&relation.from) || self.holds(&relation.to)
    }
}

/// The boundaries above an element, nearest first.
fn ancestors(parents: &BTreeMap<ElementId, ElementId>, id: &ElementId) -> BTreeSet<ElementId> {
    let mut above = BTreeSet::new();
    let mut current = parents.get(id);
    while let Some(parent) = current {
        if !above.insert(parent.clone()) {
            break;
        }
        current = parents.get(parent);
    }
    above
}

/// Every dependency of one cut, gathered from the concrete relations.
struct RolledUp {
    provenance: BTreeMap<Relation, BTreeSet<Relation>>,
    unscoped: BTreeSet<Relation>,
}

/// Rolls every concrete dependency up to the boundaries that carry it. The
/// relations are what the picture is drawn from: a scoped picture hands over
/// the ones that reach into its scope, and every other picture all of them.
///
/// Each endpoint resolves to the nearest rendered boundary above it, leaf or
/// frame alike. A relation whose two resolved boundaries stand in
/// containment - the same boundary, or one holding the other - is the outer
/// boundary's internal wiring: a frame talking to its own part is
/// implementation, not architecture, so the relation is dropped.
fn roll_up<'r>(
    relations: impl Iterator<Item = &'r Relation>,
    parents: &BTreeMap<ElementId, ElementId>,
    boundary_of: impl Fn(&ElementId) -> Option<ElementId>,
) -> RolledUp {
    let mut rolled = RolledUp {
        provenance: BTreeMap::new(),
        unscoped: BTreeSet::new(),
    };
    for relation in relations {
        if relation.kind != RelationKind::DependsOn {
            continue;
        }
        let (Some(from), Some(to)) = (boundary_of(&relation.from), boundary_of(&relation.to))
        else {
            rolled.unscoped.insert(relation.clone());
            continue;
        };
        if from == to
            || ancestors(parents, &from).contains(&to)
            || ancestors(parents, &to).contains(&from)
        {
            continue;
        }
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
            fingerprint: None,
        }
    }

    fn id(text: &str) -> ElementId {
        ElementId::new(text).unwrap()
    }

    /// A cut with the named boundaries open and the default vocabulary.
    fn opened<const N: usize>(open: [&str; N]) -> Cut {
        let mut cut = Cut::whole();
        cut.open = open.into_iter().map(id).collect();
        cut
    }

    /// A cut scoped to one boundary, with the named boundaries open.
    fn scoped<const N: usize>(scope: &str, open: [&str; N]) -> Cut {
        let mut cut = opened(open);
        cut.focus(Some(id(scope)));
        cut
    }

    /// Every boundary one view draws.
    fn drawn(view: &BoundaryView) -> BTreeSet<ElementId> {
        view.graph
            .elements()
            .map(|element| element.id.clone())
            .collect()
    }

    fn ids<const N: usize>(texts: [&str; N]) -> BTreeSet<ElementId> {
        texts.into_iter().map(id).collect()
    }

    fn relation(from: &str, to: &str, kind: RelationKind) -> Relation {
        Relation {
            from: ElementId::new(from).unwrap(),
            to: ElementId::new(to).unwrap(),
            kind,
        }
    }

    /// project ⊃ {package:a ⊃ {a/lib ⊃ a/util, a/beside},
    ///            package:b ⊃ {b/lib, b/beside}}, with a/util depending on
    /// b/lib. The two `beside` modules hold nothing and depend on nothing:
    /// they stand beside the deeper modules as plain structure.
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
            .add_element(element("a/beside", ElementKind::Module))
            .unwrap();
        graph
            .add_element(element("b/lib", ElementKind::Module))
            .unwrap();
        graph
            .add_element(element("b/beside", ElementKind::Module))
            .unwrap();
        for (from, to) in [
            ("project", "package:a"),
            ("project", "package:b"),
            ("package:a", "a/lib"),
            ("package:a", "a/beside"),
            ("a/lib", "a/util"),
            ("package:b", "b/lib"),
            ("package:b", "b/beside"),
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
        let view = boundary_view(&fixture(), &Cut::whole()).unwrap();
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
        let view = boundary_view(&graph, &Cut::whole()).unwrap();
        assert_eq!(
            view.provenance.len(),
            1,
            "only the cross-package edge remains"
        );
    }

    #[test]
    fn the_project_stands_transparent_at_the_root() {
        let view = boundary_view(&fixture(), &Cut::whole()).unwrap();
        assert_eq!(drawn(&view), ids(["package:a", "package:b"]));
        assert!(
            view.graph
                .relations()
                .all(|r| r.kind != RelationKind::Contains),
            "no box stands around the packages: the project is the picture itself"
        );
    }

    #[test]
    fn boundaries_nest_under_their_nearest_enclosing_boundary() {
        let view = boundary_view(&fixture(), &opened(["package:a", "a/lib"])).unwrap();
        let nested = relation("a/lib", "a/util", RelationKind::Contains);
        let across = relation("package:a", "a/lib", RelationKind::Contains);
        assert!(view.graph.relations().any(|r| *r == nested));
        assert!(view.graph.relations().any(|r| *r == across));
    }

    #[test]
    fn a_frames_own_dependencies_attach_to_the_frame_itself() {
        let mut graph = fixture();
        graph
            .add_relation(relation("a/lib", "b/lib", RelationKind::DependsOn))
            .unwrap();
        let view = boundary_view(&graph, &opened(["package:a", "a/lib", "package:b"])).unwrap();

        let rolled = relation("a/lib", "b/lib", RelationKind::DependsOn);
        assert!(view.graph.relations().any(|r| *r == rolled));
        assert_eq!(
            view.provenance[&rolled],
            BTreeSet::from([relation("a/lib", "b/lib", RelationKind::DependsOn)])
        );
    }

    #[test]
    fn a_view_holds_no_elements_beyond_the_sources() {
        let mut graph = fixture();
        graph
            .add_relation(relation("a/lib", "b/lib", RelationKind::DependsOn))
            .unwrap();
        let view = boundary_view(&graph, &opened(["package:a", "a/lib", "package:b"])).unwrap();

        for element in view.graph.elements() {
            assert!(
                graph.element(&element.id).is_some(),
                "{} exists in no source graph",
                element.id
            );
        }
    }

    #[test]
    fn a_dependency_from_a_frame_to_its_own_part_stays_inside_it() {
        let mut graph = fixture();
        graph
            .add_relation(relation("a/lib", "a/util", RelationKind::DependsOn))
            .unwrap();
        let view = boundary_view(&graph, &opened(["package:a", "a/lib"])).unwrap();

        assert!(
            !view
                .provenance
                .contains_key(&relation("a/lib", "a/util", RelationKind::DependsOn)),
            "a frame talking to its own part is implementation, not architecture"
        );
    }

    #[test]
    fn a_dependency_from_a_part_to_its_own_frame_stays_inside_it() {
        let mut graph = fixture();
        graph
            .add_relation(relation("a/util", "a/lib", RelationKind::DependsOn))
            .unwrap();
        let view = boundary_view(&graph, &opened(["package:a", "a/lib"])).unwrap();

        assert!(!view.provenance.contains_key(&relation(
            "a/util",
            "a/lib",
            RelationKind::DependsOn
        )));
    }

    #[test]
    fn a_dependency_naming_a_frame_as_a_whole_attaches_to_that_frame() {
        let mut graph = fixture();
        graph
            .add_relation(relation("package:a", "package:b", RelationKind::DependsOn))
            .unwrap();
        let view = boundary_view(&graph, &opened(["package:a", "package:b"])).unwrap();

        let rolled = relation("package:a", "package:b", RelationKind::DependsOn);
        assert!(
            view.graph.relations().any(|r| *r == rolled),
            "the frame takes the attachment like any other boundary"
        );
        assert_eq!(
            view.provenance[&rolled],
            BTreeSet::from([relation("package:a", "package:b", RelationKind::DependsOn)])
        );
    }

    #[test]
    fn a_dependency_into_a_closed_frames_content_attaches_to_the_frame() {
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
        let view = boundary_view(&graph, &opened(["package:a", "a/lib", "package:b"])).unwrap();

        let rolled = relation("a/util", "b/lib", RelationKind::DependsOn);
        assert!(view.graph.relations().any(|r| *r == rolled));
        assert_eq!(
            view.provenance[&rolled],
            BTreeSet::from([
                relation("a/util", "b/lib", RelationKind::DependsOn),
                relation("a/util", "b/lib#function:go", RelationKind::DependsOn),
            ]),
            "the dependency into the frame's content and the fixture's own \
             dependency on the frame share the one edge into it"
        );
    }

    #[test]
    fn opening_a_module_shows_the_items_it_declares() {
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
        let view = boundary_view(
            &graph,
            &opened(["package:a", "a/lib", "package:b", "b/lib"]),
        )
        .unwrap();

        assert!(
            view.graph
                .element(&ElementId::new("b/lib#type:Thing").unwrap())
                .is_some()
        );
        let edge = relation("a/util", "b/lib#type:Thing", RelationKind::DependsOn);
        assert!(view.graph.relations().any(|r| *r == edge));
    }

    #[test]
    fn a_dependency_outside_every_rendered_boundary_is_reported_as_unscoped() {
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
        let mut cut = Cut::whole();
        // Without modules in the vocabulary the stray module has no rendered
        // boundary above it at all; the packaged ones climb to their
        // packages.
        cut.kinds.remove(&ElementKind::Module);
        let view = boundary_view(&graph, &cut).unwrap();
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
    fn expanding_a_boundary_opens_exactly_one_layer() {
        let graph = fixture_with_items();
        let mut cut = Cut::whole();

        let view = boundary_view(&graph, &cut).unwrap();
        assert!(cut.expand(&view, &id("package:a")));
        let view = boundary_view(&graph, &cut).unwrap();
        assert!(view.graph.element(&id("a/lib")).is_some());
        assert!(
            view.graph.element(&id("a/util")).is_none(),
            "the children arrive closed; opening each is a step of its own"
        );
        assert!(
            view.graph.element(&id("b/lib")).is_none(),
            "the package beside it stays whole"
        );

        assert!(cut.expand(&view, &id("a/lib")));
        let view = boundary_view(&graph, &cut).unwrap();
        assert!(view.graph.element(&id("a/util")).is_some());
        assert!(view.graph.element(&id("a/lib#function:near")).is_some());
        assert!(
            !cut.expand(&view, &id("a/lib")),
            "an open boundary opens no further"
        );
    }

    #[test]
    fn a_closed_boundary_stands_as_a_single_box_beside_open_ones() {
        let view = boundary_view(&fixture(), &opened(["package:b"])).unwrap();

        assert!(view.graph.element(&id("a/lib")).is_none());
        assert!(
            view.graph.element(&id("b/lib")).is_some(),
            "the package beside it keeps its contents"
        );
        let rolled = relation("package:a", "b/lib", RelationKind::DependsOn);
        assert!(view.graph.relations().any(|r| *r == rolled));
    }

    #[test]
    fn an_open_flag_under_a_closed_boundary_leaves_its_contents_hidden() {
        let view = boundary_view(&fixture_with_items(), &opened(["a/lib"])).unwrap();

        assert!(view.graph.element(&id("a/lib")).is_none());
        assert!(
            view.graph.element(&id("a/util")).is_none(),
            "nothing below a closed boundary reaches the picture"
        );
        assert!(view.graph.element(&id("a/lib#function:near")).is_none());
    }

    #[test]
    fn reopening_a_boundary_restores_the_openings_made_inside_it() {
        let graph = fixture();
        let mut cut = Cut::whole();

        let view = boundary_view(&graph, &cut).unwrap();
        assert!(cut.expand(&view, &id("package:a")));
        let view = boundary_view(&graph, &cut).unwrap();
        assert!(cut.expand(&view, &id("a/lib")));

        let view = boundary_view(&graph, &cut).unwrap();
        assert!(cut.collapse(&view, &id("package:a")));
        let view = boundary_view(&graph, &cut).unwrap();
        assert!(view.graph.element(&id("a/lib")).is_none());

        assert!(cut.expand(&view, &id("package:a")));
        let view = boundary_view(&graph, &cut).unwrap();
        assert!(
            view.graph.element(&id("a/util")).is_some(),
            "the flag on a/lib stayed latent through the closing"
        );
    }

    #[test]
    fn edges_reattach_to_the_boundaries_the_reader_opens() {
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

        let view = boundary_view(&graph, &opened(["package:a", "a/lib"])).unwrap();

        let exposed = relation("a/util", "package:b", RelationKind::DependsOn);
        assert_eq!(
            view.provenance[&exposed],
            BTreeSet::from([relation("a/util", "b/lib", RelationKind::DependsOn)]),
            "the opened package answers with the module the dependency leaves"
        );
        let rolled = relation("package:c", "package:b", RelationKind::DependsOn);
        assert_eq!(
            view.provenance[&rolled],
            BTreeSet::from([relation("c/util", "b/lib", RelationKind::DependsOn)]),
            "the same dependency stays rolled up in the package beside it"
        );
    }

    #[test]
    fn a_dependency_on_a_boundary_holds_whether_the_boundary_is_open_or_collapsed() {
        let mut graph = fixture();
        graph
            .add_relation(relation("package:a", "package:b", RelationKind::DependsOn))
            .unwrap();
        let edge = relation("package:a", "package:b", RelationKind::DependsOn);

        let open = boundary_view(&graph, &opened(["package:a", "package:b"])).unwrap();
        assert!(
            open.graph.relations().any(|r| *r == edge),
            "an open frame takes the attachment on its border"
        );

        let collapsed = boundary_view(&graph, &opened(["package:a"])).unwrap();
        assert!(
            collapsed.graph.relations().any(|r| *r == edge),
            "a collapsed package is a single box, and the same edge holds"
        );
    }

    #[test]
    fn hiding_functions_reattaches_their_connections_to_the_module() {
        let mut graph = fixture();
        graph
            .add_element(element("b/lib#function:go", ElementKind::Function))
            .unwrap();
        graph
            .add_relation(relation(
                "b/lib",
                "b/lib#function:go",
                RelationKind::Contains,
            ))
            .unwrap();
        graph
            .add_relation(relation(
                "a/util",
                "b/lib#function:go",
                RelationKind::DependsOn,
            ))
            .unwrap();
        let mut cut = opened(["package:a", "a/lib", "package:b", "b/lib"]);
        cut.kinds.remove(&ElementKind::Function);
        let view = boundary_view(&graph, &cut).unwrap();

        assert!(view.graph.element(&id("b/lib#function:go")).is_none());
        let edge = relation("a/util", "b/lib", RelationKind::DependsOn);
        assert!(
            view.provenance[&edge].contains(&relation(
                "a/util",
                "b/lib#function:go",
                RelationKind::DependsOn
            )),
            "the dependency into the hidden function speaks through its module"
        );
    }

    #[test]
    fn hiding_modules_pools_their_types_in_the_package() {
        let mut graph = fixture();
        graph
            .add_element(element("a/lib#type:One", ElementKind::Type))
            .unwrap();
        graph
            .add_element(element("a/beside#type:Two", ElementKind::Type))
            .unwrap();
        for (module, item) in [
            ("a/lib", "a/lib#type:One"),
            ("a/beside", "a/beside#type:Two"),
        ] {
            graph
                .add_relation(relation(module, item, RelationKind::Contains))
                .unwrap();
        }
        graph
            .add_relation(relation(
                "a/lib#type:One",
                "a/beside#type:Two",
                RelationKind::DependsOn,
            ))
            .unwrap();
        let mut cut = opened(["package:a"]);
        cut.kinds.remove(&ElementKind::Module);
        let view = boundary_view(&graph, &cut).unwrap();

        for item in ["a/lib#type:One", "a/beside#type:Two"] {
            assert!(view.graph.element(&id(item)).is_some());
            let hoisted = relation("package:a", item, RelationKind::Contains);
            assert!(
                view.graph.relations().any(|r| *r == hoisted),
                "{item} hoists past its transparent module into the package"
            );
        }
        let edge = relation(
            "a/lib#type:One",
            "a/beside#type:Two",
            RelationKind::DependsOn,
        );
        assert!(
            view.graph.relations().any(|r| *r == edge),
            "what passed between sibling modules now passes between their types"
        );
    }

    /// The fixture with a directory inside package:a grouping two modules:
    /// package:a ⊃ a/dir ⊃ {a/dir/one, a/dir/two}.
    fn fixture_with_a_directory() -> ArchitectureGraph {
        let mut graph = fixture();
        graph
            .add_element(element("a/dir", ElementKind::Directory))
            .unwrap();
        for module in ["a/dir/one", "a/dir/two"] {
            graph
                .add_element(element(module, ElementKind::Module))
                .unwrap();
        }
        for (from, to) in [
            ("package:a", "a/dir"),
            ("a/dir", "a/dir/one"),
            ("a/dir", "a/dir/two"),
        ] {
            graph
                .add_relation(relation(from, to, RelationKind::Contains))
                .unwrap();
        }
        graph
    }

    #[test]
    fn hiding_directories_pools_their_modules_in_the_package() {
        let graph = fixture_with_a_directory();
        let mut cut = opened(["package:a"]);
        cut.kinds.remove(&ElementKind::Directory);
        let view = boundary_view(&graph, &cut).unwrap();

        assert!(view.graph.element(&id("a/dir")).is_none());
        for module in ["a/dir/one", "a/dir/two"] {
            let hoisted = relation("package:a", module, RelationKind::Contains);
            assert!(
                view.graph.relations().any(|r| *r == hoisted),
                "{module} hoists past its transparent directory into the package"
            );
        }
    }

    #[test]
    fn a_directory_opens_one_layer_like_any_other_boundary() {
        let graph = fixture_with_a_directory();
        let mut cut = opened(["package:a"]);
        let view = boundary_view(&graph, &cut).unwrap();

        assert!(view.graph.element(&id("a/dir")).is_some());
        assert!(
            view.graph.element(&id("a/dir/one")).is_none(),
            "a directory arrives closed, as every boundary does"
        );

        assert!(cut.expand(&view, &id("a/dir")));
        let view = boundary_view(&graph, &cut).unwrap();
        assert!(view.graph.element(&id("a/dir/one")).is_some());
        assert!(view.graph.element(&id("a/dir/two")).is_some());
    }

    /// The fixture with a method inside a type: `a/lib` ⊃ `a/lib#type:Config`
    /// ⊃ `a/lib#function:Config::load`, the method depending on `b/lib`.
    fn fixture_with_a_method() -> ArchitectureGraph {
        let mut graph = fixture();
        graph
            .add_element(element("a/lib#type:Config", ElementKind::Type))
            .unwrap();
        graph
            .add_element(element(
                "a/lib#function:Config::load",
                ElementKind::Function,
            ))
            .unwrap();
        for (from, to, kind) in [
            ("a/lib", "a/lib#type:Config", RelationKind::Contains),
            (
                "a/lib#type:Config",
                "a/lib#function:Config::load",
                RelationKind::Contains,
            ),
            (
                "a/lib#function:Config::load",
                "b/lib",
                RelationKind::DependsOn,
            ),
        ] {
            graph.add_relation(relation(from, to, kind)).unwrap();
        }
        graph
    }

    #[test]
    fn opening_a_type_reveals_its_methods_one_layer_down() {
        let graph = fixture_with_a_method();
        let mut cut = opened(["package:a", "a/lib"]);
        let view = boundary_view(&graph, &cut).unwrap();
        assert!(view.graph.element(&id("a/lib#type:Config")).is_some());
        assert!(
            view.graph
                .element(&id("a/lib#function:Config::load"))
                .is_none(),
            "a type with methods arrives closed, as every boundary does"
        );

        assert!(cut.expand(&view, &id("a/lib#type:Config")));
        let view = boundary_view(&graph, &cut).unwrap();
        assert!(
            view.graph
                .element(&id("a/lib#function:Config::load"))
                .is_some()
        );
    }

    #[test]
    fn hiding_functions_pools_a_methods_connections_in_its_type() {
        let graph = fixture_with_a_method();
        let mut cut = opened(["package:a", "a/lib", "a/lib#type:Config", "package:b"]);
        cut.kinds.remove(&ElementKind::Function);
        let view = boundary_view(&graph, &cut).unwrap();

        assert!(
            view.graph
                .element(&id("a/lib#function:Config::load"))
                .is_none()
        );
        let rolled = relation("a/lib#type:Config", "b/lib", RelationKind::DependsOn);
        assert!(
            view.graph.relations().any(|r| *r == rolled),
            "the hidden method hands its connection to the type that holds it"
        );
        assert_eq!(
            view.provenance[&rolled],
            BTreeSet::from([relation(
                "a/lib#function:Config::load",
                "b/lib",
                RelationKind::DependsOn
            )])
        );
    }

    #[test]
    fn a_boundary_holding_nothing_the_vocabulary_shows_cannot_open() {
        let graph = fixture_with_items();
        let mut cut = opened(["package:a", "a/lib"]);
        cut.kinds.remove(&ElementKind::Function);
        let view = boundary_view(&graph, &cut).unwrap();

        assert!(
            !view.openable.contains(&id("a/util")),
            "a/util holds one function, and functions are outside the vocabulary"
        );
        assert!(!cut.expand(&view, &id("a/util")));

        cut.kinds.insert(ElementKind::Function);
        let view = boundary_view(&graph, &cut).unwrap();
        assert!(view.openable.contains(&id("a/util")));
    }

    #[test]
    fn a_boundary_showing_a_single_box_cannot_collapse_further() {
        let graph = fixture();
        let mut cut = Cut::whole();
        let view = boundary_view(&graph, &cut).unwrap();
        assert!(!cut.collapse(&view, &id("package:a")));
        assert_eq!(cut, Cut::whole());
    }

    #[test]
    fn expand_fully_opens_a_whole_subtree() {
        let graph = fixture_with_items();
        let mut cut = Cut::whole();
        let view = boundary_view(&graph, &cut).unwrap();

        assert!(cut.expand_fully(&view, &graph, &id("package:a")));
        let view = boundary_view(&graph, &cut).unwrap();
        for inside in [
            "a/lib",
            "a/util",
            "a/lib#function:near",
            "a/util#function:go",
        ] {
            assert!(
                view.graph.element(&id(inside)).is_some(),
                "{inside} stands open with everything above it"
            );
        }
        assert!(
            view.graph.element(&id("b/lib")).is_none(),
            "the package beside it stays whole"
        );
        assert!(
            !cut.expand_fully(&view, &graph, &id("package:a")),
            "a subtree already open opens no further"
        );
    }

    #[test]
    fn the_frontier_steps_one_layer_at_a_time() {
        let graph = fixture();
        let mut cut = Cut::whole();

        let view = boundary_view(&graph, &cut).unwrap();
        assert!(cut.expand_frontier(&view));
        let view = boundary_view(&graph, &cut).unwrap();
        assert!(view.graph.element(&id("a/lib")).is_some());
        assert!(
            view.graph.element(&id("a/util")).is_none(),
            "one step opens the packages, not what their modules hold"
        );

        assert!(cut.expand_frontier(&view));
        let view = boundary_view(&graph, &cut).unwrap();
        assert!(view.graph.element(&id("a/util")).is_some());
        assert!(
            !cut.expand_frontier(&view),
            "a picture with nothing left to open steps nowhere"
        );

        assert!(cut.collapse_frontier(&view));
        let view = boundary_view(&graph, &cut).unwrap();
        assert!(
            view.graph.element(&id("a/util")).is_none(),
            "the innermost open boundary closed first"
        );
        assert!(view.graph.element(&id("a/lib")).is_some());
    }

    #[test]
    fn an_open_id_naming_no_element_changes_nothing() {
        let plain = boundary_view(&fixture(), &Cut::whole()).unwrap();
        let stale = boundary_view(&fixture(), &opened(["package:gone"])).unwrap();
        assert_eq!(plain, stale);
    }

    #[test]
    fn the_concrete_relations_behind_an_edge_are_its_provenance() {
        let view = boundary_view(&fixture(), &Cut::whole()).unwrap();
        assert_eq!(
            view.concrete_behind(&relation("package:a", "package:b", RelationKind::DependsOn)),
            BTreeSet::from([relation("a/util", "b/lib", RelationKind::DependsOn)])
        );
    }

    #[test]
    fn an_edge_into_a_frame_answers_for_its_own_code_and_the_frame_as_a_whole() {
        let mut graph = fixture();
        graph
            .add_element(element("b/util", ElementKind::Module))
            .unwrap();
        graph
            .add_relation(relation("b/lib", "b/util", RelationKind::Contains))
            .unwrap();
        // a/util reaches b/lib as a whole (the fixture dependency) and an
        // item of b/lib's own content.
        graph
            .add_element(element("b/lib#function:go", ElementKind::Function))
            .unwrap();
        graph
            .add_relation(relation(
                "b/lib",
                "b/lib#function:go",
                RelationKind::Contains,
            ))
            .unwrap();
        graph
            .add_relation(relation(
                "a/util",
                "b/lib#function:go",
                RelationKind::DependsOn,
            ))
            .unwrap();
        let view = boundary_view(&graph, &opened(["package:a", "a/lib", "package:b"])).unwrap();

        let edge = relation("a/util", "b/lib", RelationKind::DependsOn);
        assert_eq!(
            view.concrete_behind(&edge),
            BTreeSet::from([
                relation("a/util", "b/lib", RelationKind::DependsOn),
                relation("a/util", "b/lib#function:go", RelationKind::DependsOn),
            ]),
            "the whole-frame dependency and the one into the frame's content \
             answer as the one connection they draw as"
        );
    }

    #[test]
    fn an_edge_the_view_does_not_draw_answers_for_nothing() {
        let view = boundary_view(&fixture(), &Cut::whole()).unwrap();
        assert!(
            view.concrete_behind(&relation("package:b", "package:a", RelationKind::DependsOn))
                .is_empty()
        );
    }

    /// The fixture with a package beside the other two, a module beside
    /// a/lib, and a module inside each of the far ones:
    ///
    /// project ⊃ {package:a ⊃ {a/lib ⊃ a/util, a/other ⊃ a/other/deep},
    ///            package:b ⊃ b/lib ⊃ b/deep,
    ///            package:c ⊃ c/lib}
    ///
    /// a/util reaches b/lib, b/deep and a/other/deep; c/lib reaches a/util;
    /// b/lib reaches c/lib.
    fn neighbourhood() -> ArchitectureGraph {
        let mut graph = fixture();
        graph
            .add_element(element("package:c", ElementKind::Package))
            .unwrap();
        for module in ["a/other", "a/other/deep", "b/deep", "c/lib"] {
            graph
                .add_element(element(module, ElementKind::Module))
                .unwrap();
        }
        for (from, to) in [
            ("package:a", "a/other"),
            ("a/other", "a/other/deep"),
            ("b/lib", "b/deep"),
            ("project", "package:c"),
            ("package:c", "c/lib"),
        ] {
            graph
                .add_relation(relation(from, to, RelationKind::Contains))
                .unwrap();
        }
        for (from, to) in [
            ("a/util", "b/deep"),
            ("a/util", "a/other/deep"),
            ("c/lib", "a/util"),
            ("b/lib", "c/lib"),
        ] {
            graph
                .add_relation(relation(from, to, RelationKind::DependsOn))
                .unwrap();
        }
        graph
    }

    #[test]
    fn a_scoped_picture_holds_the_scope_the_partners_at_its_border_and_nothing_else() {
        let view = boundary_view(&neighbourhood(), &scoped("a/util", [])).unwrap();

        assert_eq!(
            drawn(&view),
            ids(["a/util", "a/other", "package:b", "package:c"])
        );
        assert_eq!(view.stubs, ids(["a/other", "package:b", "package:c"]));
    }

    #[test]
    fn a_scoped_boundary_stands_at_the_root_of_the_picture() {
        let view = boundary_view(&neighbourhood(), &scoped("a/util", [])).unwrap();

        assert!(
            view.graph
                .relations()
                .all(|r| r.kind != RelationKind::Contains || r.to != id("a/util")),
            "no frame is drawn around the boundary the picture is about"
        );
    }

    #[test]
    fn a_partner_stands_as_the_largest_boundary_that_leaves_the_scope_out() {
        let view = boundary_view(&neighbourhood(), &scoped("a/util", [])).unwrap();

        assert!(
            view.graph.element(&id("b/deep")).is_none(),
            "a partner in another package stands as that package"
        );
        assert!(
            view.graph.element(&id("a/other/deep")).is_none(),
            "a partner in the scope's own package stands as the boundary beside the scope"
        );
        assert!(
            view.graph
                .relations()
                .all(|r| r.kind != RelationKind::Contains),
            "every partner is one closed box, so the picture nests nothing"
        );
    }

    #[test]
    fn every_dependency_to_one_partner_gathers_on_the_stub_that_stands_for_it() {
        let view = boundary_view(&neighbourhood(), &scoped("a/util", [])).unwrap();

        assert_eq!(
            view.provenance[&relation("a/util", "package:b", RelationKind::DependsOn)],
            BTreeSet::from([
                relation("a/util", "b/deep", RelationKind::DependsOn),
                relation("a/util", "b/lib", RelationKind::DependsOn),
            ])
        );
        assert_eq!(
            view.provenance[&relation("package:c", "a/util", RelationKind::DependsOn)],
            BTreeSet::from([relation("c/lib", "a/util", RelationKind::DependsOn)]),
            "a dependency reaching into the scope stands as readily as one leaving it"
        );
    }

    #[test]
    fn a_dependency_between_two_partners_leaves_a_scoped_picture() {
        let view = boundary_view(&neighbourhood(), &scoped("a/util", [])).unwrap();

        assert!(!view.provenance.contains_key(&relation(
            "package:b",
            "package:c",
            RelationKind::DependsOn
        )));
        assert!(
            view.unscoped.is_empty(),
            "a dependency the picture is not about is dropped, not reported"
        );
    }

    #[test]
    fn a_partner_stays_closed_however_the_reader_asks_to_open_it() {
        let graph = neighbourhood();
        let mut cut = scoped("a/util", []);
        let view = boundary_view(&graph, &cut).unwrap();

        assert!(!cut.expand(&view, &id("package:b")));
        assert!(!cut.collapse(&view, &id("package:b")));
        assert!(!cut.expand_fully(&view, &graph, &id("package:b")));
        assert_eq!(cut, scoped("a/util", []));
    }

    #[test]
    fn an_open_flag_on_a_partner_is_ignored_while_the_scope_stands() {
        let view = boundary_view(&neighbourhood(), &scoped("a/util", ["package:b"])).unwrap();

        assert!(view.graph.element(&id("b/lib")).is_none());
        assert!(
            !view.open.contains(&id("package:b")) && !view.openable.contains(&id("package:b")),
            "a stub stands whole, and offers nothing to open"
        );
    }

    #[test]
    fn a_boundary_inside_the_scope_opens_as_it_would_without_a_scope() {
        let graph = neighbourhood();
        let mut cut = scoped("package:a", []);

        let view = boundary_view(&graph, &cut).unwrap();
        assert_eq!(drawn(&view), ids(["package:a", "package:b", "package:c"]));

        assert!(cut.expand(&view, &id("package:a")));
        let view = boundary_view(&graph, &cut).unwrap();
        assert!(view.graph.element(&id("a/lib")).is_some());
        assert!(view.graph.element(&id("a/other")).is_some());
    }

    #[test]
    fn the_scoped_frames_own_code_answers_from_the_frame_itself() {
        let mut graph = neighbourhood();
        graph
            .add_relation(relation("a/lib", "b/lib", RelationKind::DependsOn))
            .unwrap();
        let view = boundary_view(&graph, &scoped("package:a", ["package:a", "a/lib"])).unwrap();

        assert_eq!(
            view.provenance[&relation("a/lib", "package:b", RelationKind::DependsOn)],
            BTreeSet::from([relation("a/lib", "b/lib", RelationKind::DependsOn)])
        );
    }

    #[test]
    fn a_partner_naming_the_scoped_frame_as_a_whole_stands_at_its_border() {
        let mut graph = neighbourhood();
        graph
            .add_relation(relation("c/lib", "package:a", RelationKind::DependsOn))
            .unwrap();
        let view = boundary_view(&graph, &scoped("package:a", ["package:a", "a/lib"])).unwrap();

        assert!(view.graph.element(&id("package:c")).is_some());
        assert_eq!(
            view.provenance[&relation("package:c", "package:a", RelationKind::DependsOn)],
            BTreeSet::from([relation("c/lib", "package:a", RelationKind::DependsOn)]),
            "a dependency naming the scoped frame as a whole attaches to the \
             frame's border, exactly as it does without a scope"
        );
    }

    #[test]
    fn a_scope_on_a_single_item_shows_that_item_among_its_partners() {
        let mut graph = neighbourhood();
        graph
            .add_element(element("a/util#function:go", ElementKind::Function))
            .unwrap();
        graph
            .add_relation(relation(
                "a/util",
                "a/util#function:go",
                RelationKind::Contains,
            ))
            .unwrap();
        graph
            .add_relation(relation(
                "a/util#function:go",
                "b/deep",
                RelationKind::DependsOn,
            ))
            .unwrap();
        let view = boundary_view(&graph, &scoped("a/util#function:go", [])).unwrap();

        assert_eq!(drawn(&view), ids(["a/util#function:go", "package:b"]));
        assert_eq!(
            view.provenance[&relation("a/util#function:go", "package:b", RelationKind::DependsOn)],
            BTreeSet::from([relation(
                "a/util#function:go",
                "b/deep",
                RelationKind::DependsOn
            )])
        );
    }

    #[test]
    fn a_scope_naming_no_element_leaves_the_whole_picture_standing() {
        let plain = boundary_view(&fixture(), &Cut::whole()).unwrap();
        let stale = boundary_view(&fixture(), &scoped("package:gone", [])).unwrap();
        assert_eq!(plain, stale);
    }

    #[test]
    fn an_element_with_two_containers_is_rejected() {
        let mut graph = fixture();
        graph
            .add_relation(relation("package:b", "a/util", RelationKind::Contains))
            .unwrap();
        assert!(matches!(
            boundary_view(&graph, &Cut::whole()),
            Err(LensError::AmbiguousContainment { .. })
        ));
    }
}
