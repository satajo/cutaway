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
//! without falling into a visible child - appears as a synthetic leaf inside
//! it, named after what the frame is by [`own_content_name`] (id
//! `<frame>#self`). A dependency that names a frame as a whole cannot attach
//! to any leaf; it is reported in [`BoundaryView::coarse`] and shows
//! rolled-up at a coarser detail.
//!
//! # A package filled by one frame
//!
//! A package whose whole visible content is one frame draws three boxes
//! where a single boundary is at stake. That frame is therefore merged into
//! the package: it leaves the picture, what it holds attaches to the
//! package, and its own code becomes the package's own content. A frame with
//! a visible boundary beside it stays, because there the frame tells one
//! part of the package from another, and a package holding one boundary that
//! frames nothing keeps that boundary, because a single box is what the
//! reader came for. Only a boundary directly inside a package merges, and
//! only one level deep: no chain of frames folds away.
//!
//! A merged frame is no boundary of the picture. It carries no detail of its
//! own, [`Cut::expand`] and [`Cut::collapse`] refuse it, an override naming
//! it opens nothing, and a dependency that names it as a whole waits in
//! [`BoundaryView::coarse`] under the package that absorbed it, exactly as a
//! dependency naming any frame with drawn children does.
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
//!
//! # A cut scoped to one boundary
//!
//! Every detail below packages puts the whole project's insides in front of
//! the reader at once. A [`Cut`] therefore carries a scope: the one boundary
//! the picture is about. The rules, in order:
//!
//! - The scoped boundary is the root frame of the picture and always
//!   visible. The frames that contain it are not: nothing is drawn around
//!   the root of a picture, and what holds the scope is said in words
//!   beside it.
//! - What the scope contains shows exactly as the unscoped cut - detail and
//!   overrides alike - would show it.
//! - A partner is an element outside the scope that a concrete dependency
//!   connects with one inside it, either way round, the dependencies naming
//!   a frame of the scope as a whole included. Each partner shows as the
//!   topmost boundary above it that does not contain the scope: the largest
//!   box disjoint from the scope, which is a sibling package for a partner
//!   in another package and a sibling boundary for one in the scope's own.
//!   Several partners behind one such box share it.
//! - Only the dependencies that reach into the scope enter the picture.
//!   What passes between two partners is about neither of them and leaves.
//! - A stub stands whole: [`Cut::expand`] refuses it and an override on it
//!   is ignored. Looking inside a partner means scoping to that partner.
//!
//! A scope naming an element the graph does not hold is ignored, exactly as
//! a stale override is, and for the same reason.

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
    /// The boundary the picture is about, and None for the whole project.
    /// A scope says where the reader stands rather than what the sources
    /// hold, so nothing outlives the reading that set it.
    pub scope: Option<ElementId>,
}

impl Cut {
    /// One detail for the whole picture.
    pub fn uniform(detail: Detail) -> Self {
        Self {
            detail,
            overrides: BTreeMap::new(),
            scope: None,
        }
    }

    /// Scopes the picture to one boundary, or - with None - back to the
    /// whole project. The detail and the overrides stand either way: a
    /// reader who scopes the picture keeps the cut they were reading it at.
    pub fn focus(&mut self, scope: Option<ElementId>) {
        self.scope = scope;
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
        // A stub stands for a partner of the scope, whole and closed. The
        // picture is about the scope, so looking inside a partner means
        // scoping to that partner instead.
        if view.stubs.contains(boundary) {
            return false;
        }
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
    /// The partners standing at the border of a scoped picture, each whole
    /// and closed. They open to nothing: a reader looks inside one by
    /// scoping the picture to it. Empty while the cut scopes to nothing.
    pub stubs: BTreeSet<ElementId>,
}

impl BoundaryView {
    /// Every concrete relation one rendered edge answers for: what the lens
    /// rolled into it, together with the dependencies between the same pair
    /// of boundaries that name the target frame as a whole and wait in
    /// [`BoundaryView::coarse`]. Acting on the rendered edge - severing it,
    /// annotating it - means acting on all of them: the reader addresses the
    /// connection between two boundaries, not one attachment of it.
    ///
    /// The self leaves stand for their frames here, so an edge into a
    /// frame's own content also answers for the dependencies on the frame
    /// as a whole. Answers empty for an edge this view does not draw.
    #[must_use]
    pub fn concrete_behind(&self, edge: &Relation) -> BTreeSet<Relation> {
        let mut concrete = self.provenance.get(edge).cloned().unwrap_or_default();
        let frame_pair = Relation {
            from: self_leaf_frame(&edge.from).unwrap_or_else(|| edge.from.clone()),
            to: self_leaf_frame(&edge.to).unwrap_or_else(|| edge.to.clone()),
            kind: edge.kind,
        };
        if let Some(coarse) = self.coarse.get(&frame_pair) {
            concrete.extend(coarse.iter().cloned());
        }
        concrete
    }
}

/// Cuts `graph` where the cut asks for.
pub fn boundary_view(graph: &ArchitectureGraph, cut: &Cut) -> Result<BoundaryView, LensError> {
    let parents = containment_parents(graph)?;
    let contexts = contexts(graph, &parents, cut);
    let scoped = Scoped::of(graph, &parents, cut.scope.as_ref());
    let shown = |element: &Element| {
        contexts
            .get(&element.id)
            .is_some_and(|detail| detail.shows(element.kind))
    };
    let standing: BTreeSet<ElementId> = graph
        .elements()
        .filter(|element| match &scoped {
            None => shown(element),
            Some(scoped) => scoped.draws(&element.id, shown(element)),
        })
        .map(|element| element.id.clone())
        .collect();
    // A frame that fills its package alone leaves the picture before
    // anything reads the visible set, so what it holds nests under the
    // package and its own code rolls up to the package's own content.
    let merged = merged_into_packages(graph, &parents, &standing);
    let visible: BTreeSet<ElementId> = standing.difference(&merged).cloned().collect();
    let detail_within: BTreeMap<ElementId, Detail> = visible
        .iter()
        .map(|id| {
            let within = match &scoped {
                Some(scoped) if scoped.stubs.contains(id) => closed(graph, cut, id),
                _ => within(graph, cut, &contexts, id),
            };
            (id.clone(), within)
        })
        .collect();

    let boundary_of = |id: &ElementId| -> Option<ElementId> {
        // Outside the scope only the stubs stand, and every element behind
        // one attaches to the stub that stands for it.
        if let Some(scoped) = &scoped
            && !scoped.holds(id)
        {
            return scoped.stub_of(&parents, id);
        }
        // The scoped boundary is always visible, so a climb that starts
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
    let frames: BTreeSet<ElementId> = nesting.iter().map(|r| r.from.clone()).collect();

    // A scoped picture is about the scope, so the dependencies that reach
    // neither into nor out of it never enter the roll-up at all.
    let concrete = graph.relations().filter(|relation| {
        scoped
            .as_ref()
            .is_none_or(|scoped| scoped.touches(relation))
    });
    let RolledUp {
        provenance,
        coarse,
        unscoped,
        self_leaves,
    } = roll_up(concrete, &frames, &merged, boundary_of);

    for frame in &self_leaves {
        let kind = view
            .element(frame)
            .expect("self leaves grow only on view elements")
            .kind;
        view.add_element(Element {
            id: self_leaf_id(frame),
            name: ElementName::new(own_content_name(kind)).expect("the name is never empty"),
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
        stubs: scoped.map(|scoped| scoped.stubs).unwrap_or_default(),
    })
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
    /// scoped boundary itself stands whatever it says, and outside the scope
    /// the stubs stand alone.
    fn draws(&self, id: &ElementId, shown: bool) -> bool {
        *id == self.root || self.stubs.contains(id) || (shown && self.holds(id))
    }

    /// Whether one dependency reaches into the scope.
    fn touches(&self, relation: &Relation) -> bool {
        self.holds(&relation.from) || self.holds(&relation.to)
    }
}

/// The frames the picture merges into the package that holds them.
///
/// A package whose whole visible content is one frame draws three boxes
/// where a single boundary is at stake: the package, the frame, and what the
/// frame holds. The frame therefore leaves the visible set, so everything it
/// holds attaches to the package and its own code becomes the package's own
/// content. A frame with a visible boundary beside it stays: there the frame
/// tells one part of the package from another.
///
/// Only a boundary directly inside a package merges, and only into that
/// package. One containment level, so no chain of frames folds away and the
/// reader keeps every level the sources put between the package and its
/// content.
fn merged_into_packages(
    graph: &ArchitectureGraph,
    parents: &BTreeMap<ElementId, ElementId>,
    visible: &BTreeSet<ElementId>,
) -> BTreeSet<ElementId> {
    let mut children: BTreeMap<&ElementId, usize> = BTreeMap::new();
    for id in visible {
        if let Some(parent) = parents.get(id) {
            *children.entry(parent).or_default() += 1;
        }
    }
    let fills_a_package = |id: &ElementId| {
        let Some(package) = parents.get(id) else {
            return false;
        };
        // The package must stand in this picture. A picture scoped to the
        // frame draws no package around it, and a boundary merges into
        // nothing.
        visible.contains(package)
            && graph
                .element(package)
                .is_some_and(|element| element.kind == ElementKind::Package)
            && children.get(package) == Some(&1)
    };
    visible
        .iter()
        .filter(|id| children.contains_key(*id) && fills_a_package(id))
        .cloned()
        .collect()
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

/// The detail a boundary frozen at the border of a scope reads at: the
/// coarsest one that shows a boundary of its kind, which is the detail that
/// draws it as the single box it is.
fn closed(graph: &ArchitectureGraph, cut: &Cut, id: &ElementId) -> Detail {
    graph
        .element(id)
        .and_then(|element| Detail::showing(element.kind))
        .unwrap_or(cut.detail)
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

/// Rolls every concrete dependency up to the boundaries that carry it. The
/// relations are what the picture is drawn from: a scoped picture hands over
/// the ones that reach into its scope, and every other picture all of them.
/// `merged` names the frames the picture folded into their packages; a
/// dependency on one of those names a frame as surely as a dependency on a
/// frame the picture draws.
fn roll_up<'r>(
    relations: impl Iterator<Item = &'r Relation>,
    frames: &BTreeSet<ElementId>,
    merged: &BTreeSet<ElementId>,
    boundary_of: impl Fn(&ElementId) -> Option<ElementId>,
) -> RolledUp {
    let mut rolled = RolledUp {
        provenance: BTreeMap::new(),
        coarse: BTreeMap::new(),
        unscoped: BTreeSet::new(),
        self_leaves: BTreeSet::new(),
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
        if from == to {
            continue;
        }
        // A concrete relation that names a frame as its target depends on
        // the frame as a whole: no leaf can carry it at this detail. A frame
        // merged into its package answers the same way, because its contents
        // stand spread inside the package.
        let whole_frame =
            (frames.contains(&to) && relation.to == to) || merged.contains(&relation.to);
        if whole_frame {
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

/// What a frame's own-content leaf is called: the code the boundary of that
/// kind declares itself. The name says what the box holds rather than that
/// the lens made it, because a reader who meets the box asks the first
/// question and not the second.
#[must_use]
pub fn own_content_name(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::Project => "project code",
        ElementKind::Package => "package code",
        ElementKind::Module => "module code",
        ElementKind::Function => "function code",
        ElementKind::Type => "type code",
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
    self_leaf_frame(id).is_some()
}

/// The frame whose own content a self leaf names, and None for an id the
/// sources declare. A plan must never record a self leaf - the id exists in
/// no source graph - so whatever acts on a self leaf acts on this frame.
pub fn self_leaf_frame(id: &ElementId) -> Option<ElementId> {
    id.as_str()
        .strip_suffix(SELF_LEAF_MARK)
        .and_then(|frame| ElementId::new(frame).ok())
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
            scope: None,
        }
    }

    fn scoped<const N: usize>(detail: Detail, scope: &str, overrides: [(&str, Detail); N]) -> Cut {
        let mut cut = cut(detail, overrides);
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
    /// they stand beside the root modules so that each package holds more
    /// than one boundary, which is what keeps those root modules boundaries
    /// of their own.
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
        assert_eq!(
            self_leaf.name.as_str(),
            own_content_name(ElementKind::Module)
        );
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
                element.name.as_str() == own_content_name(element.kind),
                "{}",
                element.id
            );
        }
    }

    #[test]
    fn a_frames_own_content_is_named_after_what_the_frame_is() {
        let mut graph = fixture();
        graph
            .add_relation(relation("a/lib", "b/lib", RelationKind::DependsOn))
            .unwrap();
        let view = boundary_view(&graph, &Cut::uniform(Detail::Modules)).unwrap();
        assert_eq!(
            view.graph.element(&id("a/lib#self")).unwrap().name.as_str(),
            "module code"
        );

        let filled = boundary_view(&filled_package(), &Cut::uniform(Detail::Modules)).unwrap();
        assert_eq!(
            filled
                .graph
                .element(&id("package:one#self"))
                .unwrap()
                .name
                .as_str(),
            "package code",
            "the leaf says what kind of boundary declares the code it carries"
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

    /// project ⊃ {package:one ⊃ root ⊃ root/deep, package:two ⊃ two/lib},
    /// where root is the whole visible content of package:one at modules
    /// detail. root's own code depends on two/lib, and two/lib depends on
    /// root as a whole.
    fn filled_package() -> ArchitectureGraph {
        let mut graph = ArchitectureGraph::new();
        for (id, kind) in [
            ("project", ElementKind::Project),
            ("package:one", ElementKind::Package),
            ("package:two", ElementKind::Package),
            ("root", ElementKind::Module),
            ("root/deep", ElementKind::Module),
            ("two/lib", ElementKind::Module),
        ] {
            graph.add_element(element(id, kind)).unwrap();
        }
        for (from, to) in [
            ("project", "package:one"),
            ("project", "package:two"),
            ("package:one", "root"),
            ("root", "root/deep"),
            ("package:two", "two/lib"),
        ] {
            graph
                .add_relation(relation(from, to, RelationKind::Contains))
                .unwrap();
        }
        for (from, to) in [("root", "two/lib"), ("two/lib", "root")] {
            graph
                .add_relation(relation(from, to, RelationKind::DependsOn))
                .unwrap();
        }
        graph
    }

    #[test]
    fn a_frame_that_fills_its_package_alone_leaves_the_picture() {
        let view = boundary_view(&filled_package(), &Cut::uniform(Detail::Modules)).unwrap();

        assert!(view.graph.element(&id("root")).is_none());
        assert!(
            view.graph
                .relations()
                .any(|r| *r == relation("package:one", "root/deep", RelationKind::Contains)),
            "what the merged frame held stands directly inside the package"
        );
    }

    #[test]
    fn a_frame_with_a_boundary_beside_it_keeps_its_own_box() {
        let view = boundary_view(&fixture(), &Cut::uniform(Detail::Modules)).unwrap();

        assert!(view.graph.element(&id("a/lib")).is_some());
        assert!(
            view.graph
                .relations()
                .any(|r| *r == relation("a/lib", "a/util", RelationKind::Contains)),
            "a frame that tells one part of its package from another stands"
        );
    }

    #[test]
    fn a_package_holding_one_boundary_with_nothing_inside_it_keeps_that_boundary() {
        let view = boundary_view(&filled_package(), &Cut::uniform(Detail::Modules)).unwrap();

        assert!(
            view.graph.element(&id("two/lib")).is_some(),
            "only a frame merges: a boundary the picture draws as one box is \
             what the reader came for"
        );
    }

    #[test]
    fn the_own_code_of_a_merged_frame_becomes_the_packages_own_content() {
        let view = boundary_view(&filled_package(), &Cut::uniform(Detail::Modules)).unwrap();

        let rolled = relation("package:one#self", "two/lib", RelationKind::DependsOn);
        assert_eq!(
            view.provenance[&rolled],
            BTreeSet::from([relation("root", "two/lib", RelationKind::DependsOn)])
        );
    }

    #[test]
    fn a_dependency_naming_a_merged_frame_as_a_whole_waits_at_a_coarser_detail() {
        let view = boundary_view(&filled_package(), &Cut::uniform(Detail::Modules)).unwrap();

        assert_eq!(
            view.coarse[&relation("two/lib", "package:one", RelationKind::DependsOn)],
            BTreeSet::from([relation("two/lib", "root", RelationKind::DependsOn)]),
            "the merged frame's contents stand inside the package, so no leaf \
             carries a dependency on all of them"
        );
        assert!(
            view.provenance
                .keys()
                .all(|edge| edge.to != id("package:one#self")),
            "a dependency on the whole frame is not a dependency on its own code"
        );
    }

    #[test]
    fn a_merged_frame_is_no_boundary_the_reader_opens_or_closes() {
        let graph = filled_package();
        let mut cut = Cut::uniform(Detail::Modules);
        let view = boundary_view(&graph, &cut).unwrap();

        assert!(!view.detail_within.contains_key(&id("root")));
        assert!(!cut.expand(&view, &id("root")));
        assert!(!cut.collapse(&view, &id("root")));
        assert_eq!(cut, Cut::uniform(Detail::Modules));
    }

    #[test]
    fn an_override_on_a_merged_frame_leaves_the_package_merged() {
        let view = boundary_view(
            &filled_package(),
            &cut(Detail::Modules, [("root", Detail::Items)]),
        )
        .unwrap();

        assert!(view.graph.element(&id("root")).is_none());
        assert!(
            view.graph
                .relations()
                .any(|r| *r == relation("package:one", "root/deep", RelationKind::Contains)),
            "a boundary the picture does not draw is no boundary to open"
        );
    }

    #[test]
    fn a_picture_scoped_to_a_package_merges_the_frame_that_fills_it() {
        let view = boundary_view(
            &filled_package(),
            &scoped(Detail::Modules, "package:one", []),
        )
        .unwrap();

        assert_eq!(
            drawn(&view),
            ids([
                "package:one",
                "package:one#self",
                "root/deep",
                "package:two"
            ])
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
    fn a_self_leaf_id_names_the_frame_it_grew_on() {
        assert_eq!(self_leaf_frame(&id("a/lib#self")), Some(id("a/lib")));
        assert_eq!(self_leaf_frame(&id("a/lib")), None);
        assert_eq!(
            self_leaf_frame(&id("a/lib#type:X")),
            None,
            "an item id ends in #<kind>:<name>, never in the self mark"
        );
    }

    #[test]
    fn the_concrete_relations_behind_an_edge_are_its_provenance() {
        let view = boundary_view(&fixture(), &Cut::uniform(Detail::Packages)).unwrap();
        assert_eq!(
            view.concrete_behind(&relation("package:a", "package:b", RelationKind::DependsOn)),
            BTreeSet::from([relation("a/util", "b/lib", RelationKind::DependsOn)])
        );
    }

    #[test]
    fn an_edge_into_a_frames_own_content_also_answers_for_the_frame_as_a_whole() {
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
        let view = boundary_view(&graph, &Cut::uniform(Detail::Modules)).unwrap();

        let edge = relation("a/util", "b/lib#self", RelationKind::DependsOn);
        assert_eq!(
            view.concrete_behind(&edge),
            BTreeSet::from([
                relation("a/util", "b/lib", RelationKind::DependsOn),
                relation("a/util", "b/lib#function:go", RelationKind::DependsOn),
            ]),
            "the waiting whole-frame dependency joins the rendered edge's answer"
        );
    }

    #[test]
    fn an_edge_the_view_does_not_draw_answers_for_nothing() {
        let view = boundary_view(&fixture(), &Cut::uniform(Detail::Packages)).unwrap();
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
        let view = boundary_view(&neighbourhood(), &scoped(Detail::Modules, "a/util", [])).unwrap();

        assert_eq!(
            drawn(&view),
            ids(["a/util", "a/other", "package:b", "package:c"])
        );
        assert_eq!(view.stubs, ids(["a/other", "package:b", "package:c"]));
    }

    #[test]
    fn a_scoped_boundary_stands_at_the_root_of_the_picture() {
        let view = boundary_view(&neighbourhood(), &scoped(Detail::Modules, "a/util", [])).unwrap();

        assert!(
            view.graph
                .relations()
                .all(|r| r.kind != RelationKind::Contains || r.to != id("a/util")),
            "no frame is drawn around the boundary the picture is about"
        );
    }

    #[test]
    fn a_partner_stands_as_the_largest_boundary_that_leaves_the_scope_out() {
        let view = boundary_view(&neighbourhood(), &scoped(Detail::Modules, "a/util", [])).unwrap();

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
        let view = boundary_view(&neighbourhood(), &scoped(Detail::Modules, "a/util", [])).unwrap();

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
        let view = boundary_view(&neighbourhood(), &scoped(Detail::Modules, "a/util", [])).unwrap();

        assert!(!view.provenance.contains_key(&relation(
            "package:b",
            "package:c",
            RelationKind::DependsOn
        )));
        assert!(view.coarse.is_empty());
        assert!(
            view.unscoped.is_empty(),
            "a dependency the picture is not about is dropped, not reported"
        );
    }

    #[test]
    fn a_partner_stays_closed_however_the_reader_asks_to_open_it() {
        let graph = neighbourhood();
        let mut cut = scoped(Detail::Modules, "a/util", []);
        let view = boundary_view(&graph, &cut).unwrap();

        assert!(!cut.expand(&view, &id("package:b")));
        assert!(!cut.collapse(&view, &id("package:b")));
        assert_eq!(cut, scoped(Detail::Modules, "a/util", []));
    }

    #[test]
    fn an_override_that_would_open_a_partner_is_ignored_while_the_scope_stands() {
        let view = boundary_view(
            &neighbourhood(),
            &scoped(Detail::Modules, "a/util", [("package:b", Detail::Items)]),
        )
        .unwrap();

        assert!(view.graph.element(&id("b/lib")).is_none());
        assert_eq!(
            view.detail_within[&id("package:b")],
            Detail::Packages,
            "a stub stands whole, at the detail that shows a package as one box"
        );
    }

    #[test]
    fn a_boundary_inside_the_scope_opens_as_it_would_without_a_scope() {
        let graph = neighbourhood();
        let mut cut = scoped(Detail::Packages, "package:a", []);

        let view = boundary_view(&graph, &cut).unwrap();
        assert_eq!(drawn(&view), ids(["package:a", "package:b", "package:c"]));

        assert!(cut.expand(&view, &id("package:a")));
        let view = boundary_view(&graph, &cut).unwrap();
        assert!(view.graph.element(&id("a/lib")).is_some());
        assert!(view.graph.element(&id("a/other")).is_some());
    }

    #[test]
    fn the_scoped_frames_own_content_answers_for_its_own_dependencies() {
        let mut graph = neighbourhood();
        graph
            .add_relation(relation("a/lib", "b/lib", RelationKind::DependsOn))
            .unwrap();
        let view = boundary_view(&graph, &scoped(Detail::Modules, "package:a", [])).unwrap();

        assert_eq!(
            view.provenance[&relation("a/lib#self", "package:b", RelationKind::DependsOn)],
            BTreeSet::from([relation("a/lib", "b/lib", RelationKind::DependsOn)])
        );
    }

    #[test]
    fn a_partner_naming_the_scoped_frame_as_a_whole_stands_at_its_border() {
        let mut graph = neighbourhood();
        graph
            .add_relation(relation("c/lib", "package:a", RelationKind::DependsOn))
            .unwrap();
        let view = boundary_view(&graph, &scoped(Detail::Modules, "package:a", [])).unwrap();

        assert!(view.graph.element(&id("package:c")).is_some());
        assert_eq!(
            view.coarse[&relation("package:c", "package:a", RelationKind::DependsOn)],
            BTreeSet::from([relation("c/lib", "package:a", RelationKind::DependsOn)]),
            "a dependency naming the scoped frame as a whole waits at a coarser \
             detail, exactly as it does without a scope"
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
        let view = boundary_view(&graph, &scoped(Detail::Items, "a/util#function:go", [])).unwrap();

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
        let plain = boundary_view(&fixture(), &Cut::uniform(Detail::Packages)).unwrap();
        let stale =
            boundary_view(&fixture(), &scoped(Detail::Packages, "package:gone", [])).unwrap();
        assert_eq!(plain, stale);
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
