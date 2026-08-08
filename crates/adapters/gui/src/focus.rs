//! What a selection puts in focus.
//!
//! Selecting a boundary asks "what does this depend on". A frame carries the
//! edges of its own code, and its parts carry theirs: the whole answer lies
//! in the subtree. A selection therefore focuses a whole subtree - the
//! selected boundary and its descendants - together with every edge that
//! touches the subtree and the partners at the far end of those edges. The
//! frames around the focused elements stay readable as context, so a
//! highlighted boundary still says which boundary it sits in.
//!
//! The computation is pure view logic: it reads the view graph and the drawn
//! edges and answers with a strength per element and per edge. The canvas
//! only applies the answer.
//!
//! Every walk here reads containment, so the containment queries the rest of
//! the shell needs - what a boundary holds, what holds it, and which
//! boundary of a picture answers for an element below its detail - live here
//! too, and answer for the full graph as readily as for a view.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{ArchitectureGraph, ElementId, Relation, RelationKind};

/// How strongly one element or edge paints, ordered weakest to strongest: a
/// box that stands for several elements paints as the strongest of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Strength {
    /// Everything the selection does not reach.
    Faded,
    /// The frames around focused elements: readable, but not a subject.
    Context,
    /// The selection and what it reaches: full color.
    Focused,
}

/// Which way one lit edge runs about the selected boundary. A selection asks
/// two questions at once - what does this need, and who needs this - and the
/// answer is only readable while the two stay apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    /// Leaves the selection: something the selection depends on.
    Outgoing,
    /// Arrives at the selection: something that depends on the selection.
    Incoming,
    /// Both ends lie inside the selection: its own internal wiring, which
    /// answers neither question.
    Internal,
}

/// What the user selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Selected<'a> {
    Node(&'a ElementId),
    Edge(&'a Relation),
}

/// The strengths one selection produces, and the way its lit edges run.
pub(crate) struct Focus<'a> {
    subjects: BTreeSet<&'a ElementId>,
    context: BTreeSet<&'a ElementId>,
    /// Every lit edge. A selected boundary also says which way each of them
    /// runs about it; a selected connection is a single edge and stands for
    /// no boundary, so it says nothing.
    lit: BTreeMap<&'a Relation, Option<Direction>>,
}

impl Focus<'_> {
    pub(crate) fn element(&self, id: &ElementId) -> Strength {
        if self.subjects.contains(id) {
            Strength::Focused
        } else if self.context.contains(id) {
            Strength::Context
        } else {
            Strength::Faded
        }
    }

    pub(crate) fn edge(&self, relation: &Relation) -> Strength {
        if self.lit.contains_key(relation) {
            Strength::Focused
        } else {
            Strength::Faded
        }
    }

    /// Which way this edge runs about the selection, and None wherever the
    /// selection says nothing about it: an edge it does not light, or any
    /// edge at all while a connection rather than a boundary is selected.
    pub(crate) fn direction(&self, relation: &Relation) -> Option<Direction> {
        self.lit.get(relation).copied().flatten()
    }
}

/// The focus of one selection over a view, given the edges the canvas draws
/// and the containment of that view.
pub(crate) fn focus_of<'a>(
    containment: &'a Containment,
    edges: impl IntoIterator<Item = &'a Relation>,
    selected: Selected<'a>,
) -> Focus<'a> {
    let mut subjects = BTreeSet::new();
    let mut lit = BTreeMap::new();
    match selected {
        Selected::Node(id) => {
            let inside = containment.subtree(id);
            for edge in edges {
                // The subtree holds everything below the selection, so an
                // endpoint the picture hides inside a summary block answers
                // the same as the block that stands for it: both lie inside
                // the selection or both lie outside it.
                let direction = match (inside.contains(&edge.from), inside.contains(&edge.to)) {
                    (true, true) => Direction::Internal,
                    (true, false) => Direction::Outgoing,
                    (false, true) => Direction::Incoming,
                    (false, false) => continue,
                };
                lit.insert(edge, Some(direction));
                subjects.insert(&edge.from);
                subjects.insert(&edge.to);
            }
            subjects.extend(inside);
        }
        Selected::Edge(relation) => {
            lit.insert(relation, None);
            subjects.insert(&relation.from);
            subjects.insert(&relation.to);
        }
    }
    let context = containment.ancestors_of(&subjects);
    Focus {
        subjects,
        context,
        lit,
    }
}

/// The containment of one graph, resolved once: what each boundary directly
/// holds, and what holds it.
///
/// Every walk over containment - a subtree, a climb to the frames above, the
/// name a box reads against - otherwise scans the whole relation list per
/// question, and a picture asks these questions many times per frame. The
/// index owns its ids rather than borrowing the graph, so a caller can hold
/// it beside the graph it describes and answer from it for as long as that
/// graph stands.
#[derive(Debug, Clone, Default)]
pub(crate) struct Containment {
    children: BTreeMap<ElementId, Vec<ElementId>>,
    parents: BTreeMap<ElementId, ElementId>,
}

impl Containment {
    pub(crate) fn of(graph: &ArchitectureGraph) -> Self {
        let mut containment = Self::default();
        for relation in graph.relations() {
            if relation.kind == RelationKind::Contains {
                containment
                    .children
                    .entry(relation.from.clone())
                    .or_default()
                    .push(relation.to.clone());
                containment
                    .parents
                    .insert(relation.to.clone(), relation.from.clone());
            }
        }
        containment
    }

    /// The boundaries one boundary directly holds, in the order the graph
    /// declares them. A boundary that holds nothing is a leaf.
    pub(crate) fn children(&self, frame: &ElementId) -> &[ElementId] {
        self.children.get(frame).map_or(&[], Vec::as_slice)
    }

    /// The boundary that directly holds this one, if any.
    pub(crate) fn parent(&self, id: &ElementId) -> Option<&ElementId> {
        self.parents.get(id)
    }

    /// Whether this boundary holds anything, and therefore paints as a frame
    /// rather than as a box of its own.
    pub(crate) fn is_frame(&self, id: &ElementId) -> bool {
        self.children.contains_key(id)
    }

    /// A boundary and everything it contains, however deep.
    pub(crate) fn subtree<'a>(&'a self, root: &'a ElementId) -> BTreeSet<&'a ElementId> {
        let mut inside = BTreeSet::from([root]);
        let mut queue = vec![root];
        while let Some(current) = queue.pop() {
            for child in self.children(current) {
                if inside.insert(child) {
                    queue.push(child);
                }
            }
        }
        inside
    }

    /// The containing boundaries above the given elements, excluding the
    /// elements themselves.
    fn ancestors_of<'a>(&'a self, of: &BTreeSet<&'a ElementId>) -> BTreeSet<&'a ElementId> {
        let mut context = BTreeSet::new();
        for id in of {
            // Containment of a view is a tree, but a walk that trusts that
            // and meets a cycle never ends; the seen set bounds every walk.
            let mut seen = BTreeSet::new();
            let mut current = self.parent(id);
            while let Some(parent) = current {
                if !seen.insert(parent) {
                    break;
                }
                if !of.contains(parent) {
                    context.insert(parent);
                }
                current = self.parent(parent);
            }
        }
        context
    }
}

/// The boundaries one boundary directly contains, in id order. A boundary
/// with contents paints as a frame; an empty answer names a leaf.
pub(crate) fn contents_of<'a>(
    view: &'a ArchitectureGraph,
    frame: &ElementId,
) -> Vec<&'a ElementId> {
    view.relations()
        .filter(|relation| relation.kind == RelationKind::Contains && relation.from == *frame)
        .map(|relation| &relation.to)
        .collect()
}

/// The boundary that directly contains this one, if any.
pub(crate) fn frame_of<'a>(view: &'a ArchitectureGraph, id: &ElementId) -> Option<&'a ElementId> {
    view.relations()
        .find(|relation| relation.kind == RelationKind::Contains && relation.to == *id)
        .map(|relation| &relation.from)
}

/// The boundary a concrete element shows up as in the view. The element
/// itself often sits below the detail the view cuts at, so the walk climbs
/// the containment of the full graph until it meets a boundary the view
/// holds. Answers None while no boundary above the element is visible.
pub(crate) fn boundary_in_view(
    view: &ArchitectureGraph,
    graph: &ArchitectureGraph,
    id: &ElementId,
) -> Option<ElementId> {
    // Containment is a tree wherever a view exists at all, but a walk that
    // trusts that and meets a cycle never ends; the seen set bounds it.
    let mut seen = BTreeSet::new();
    let mut current = Some(id);
    while let Some(id) = current {
        if !seen.insert(id) {
            return None;
        }
        if view.element(id).is_some() {
            return Some(id.clone());
        }
        current = frame_of(graph, id);
    }
    None
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{Element, ElementKind, ElementName};

    use super::*;

    fn id(text: &str) -> ElementId {
        ElementId::new(text).unwrap()
    }

    fn depends(from: &str, to: &str) -> Relation {
        Relation {
            from: id(from),
            to: id(to),
            kind: RelationKind::DependsOn,
        }
    }

    /// package:a ⊃ {a/one, a/two}, package:b ⊃ {b/one}, and the leaves
    /// package:c and package:d beside them.
    fn view() -> ArchitectureGraph {
        let mut graph = ArchitectureGraph::new();
        for (element, kind) in [
            ("package:a", ElementKind::Package),
            ("package:b", ElementKind::Package),
            ("package:c", ElementKind::Package),
            ("package:d", ElementKind::Package),
            ("a/one", ElementKind::Module),
            ("a/two", ElementKind::Module),
            ("b/one", ElementKind::Module),
        ] {
            graph
                .add_element(Element::of_kind(
                    id(element),
                    kind,
                    ElementName::new(element).unwrap(),
                ))
                .unwrap();
        }
        for (from, to) in [
            ("package:a", "a/one"),
            ("package:a", "a/two"),
            ("package:b", "b/one"),
        ] {
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

    /// One edge inside package:a, one crossing its border, one far outside.
    fn edges() -> Vec<Relation> {
        vec![
            depends("a/one", "a/two"),
            depends("a/two", "b/one"),
            depends("package:c", "package:d"),
        ]
    }

    #[test]
    fn selecting_a_frame_keeps_its_subtree_and_border_crossing_edges_lit() {
        let containment = Containment::of(&view());
        let edges = edges();
        let selected = id("package:a");
        let focus = focus_of(&containment, &edges, Selected::Node(&selected));

        for member in ["package:a", "a/one", "a/two"] {
            assert_eq!(focus.element(&id(member)), Strength::Focused, "{member}");
        }
        assert_eq!(focus.edge(&depends("a/one", "a/two")), Strength::Focused);
        assert_eq!(focus.edge(&depends("a/two", "b/one")), Strength::Focused);
        assert_eq!(
            focus.element(&id("b/one")),
            Strength::Focused,
            "the partner across the border answers the question"
        );
    }

    #[test]
    fn ancestors_of_focused_elements_stay_readable() {
        let containment = Containment::of(&view());
        let edges = edges();
        let frame = id("package:a");
        let focus = focus_of(&containment, &edges, Selected::Node(&frame));
        assert_eq!(
            focus.element(&id("package:b")),
            Strength::Context,
            "the frame around the highlighted partner names it"
        );

        let leaf = id("a/one");
        let focus = focus_of(&containment, &edges, Selected::Node(&leaf));
        assert_eq!(focus.element(&id("package:a")), Strength::Context);
    }

    #[test]
    fn edges_fully_outside_a_selected_frame_fade() {
        let containment = Containment::of(&view());
        let edges = edges();
        let selected = id("package:a");
        let focus = focus_of(&containment, &edges, Selected::Node(&selected));

        assert_eq!(
            focus.edge(&depends("package:c", "package:d")),
            Strength::Faded
        );
        assert_eq!(focus.element(&id("package:c")), Strength::Faded);
        assert_eq!(focus.element(&id("package:d")), Strength::Faded);
    }

    #[test]
    fn selecting_a_leaf_lights_only_the_dependencies_that_touch_it() {
        let containment = Containment::of(&view());
        let edges = edges();
        let selected = id("a/one");
        let focus = focus_of(&containment, &edges, Selected::Node(&selected));

        assert_eq!(focus.edge(&depends("a/one", "a/two")), Strength::Focused);
        assert_eq!(focus.element(&id("a/two")), Strength::Focused);
        assert_eq!(focus.edge(&depends("a/two", "b/one")), Strength::Faded);
        assert_eq!(focus.element(&id("b/one")), Strength::Faded);
    }

    #[test]
    fn a_selected_boundary_tells_its_dependencies_from_its_dependents() {
        let containment = Containment::of(&view());
        let edges = edges();
        let selected = id("package:a");
        let focus = focus_of(&containment, &edges, Selected::Node(&selected));

        assert_eq!(
            focus.direction(&depends("a/two", "b/one")),
            Some(Direction::Outgoing),
            "what the selection reaches is what it depends on"
        );
        assert_eq!(
            focus.direction(&depends("a/one", "a/two")),
            Some(Direction::Internal),
            "an edge with both ends inside answers neither question"
        );
        assert_eq!(
            focus.direction(&depends("package:c", "package:d")),
            None,
            "an edge the selection does not light runs no way about it"
        );
    }

    #[test]
    fn a_connection_reaching_the_selection_runs_inward() {
        let containment = Containment::of(&view());
        let edges = edges();
        let selected = id("package:b");
        let focus = focus_of(&containment, &edges, Selected::Node(&selected));

        assert_eq!(
            focus.direction(&depends("a/two", "b/one")),
            Some(Direction::Incoming),
            "the partner reaches into the selection through the part it holds"
        );
    }

    #[test]
    fn a_selected_connection_runs_no_way_about_itself() {
        let containment = Containment::of(&view());
        let edges = edges();
        let selected = depends("a/two", "b/one");
        let focus = focus_of(&containment, &edges, Selected::Edge(&selected));

        assert_eq!(focus.edge(&selected), Strength::Focused);
        assert_eq!(
            focus.direction(&selected),
            None,
            "a connection stands for no boundary, so nothing leaves or arrives"
        );
    }

    /// The view above, plus the type declared inside a/one. The full graph
    /// holds it; a picture cut at packages does not.
    fn graph() -> ArchitectureGraph {
        let mut graph = view();
        graph
            .add_element(Element::of_kind(
                id("a/one#type:X"),
                ElementKind::Type,
                ElementName::new("X").unwrap(),
            ))
            .unwrap();
        graph
            .add_relation(Relation {
                from: id("a/one"),
                to: id("a/one#type:X"),
                kind: RelationKind::Contains,
            })
            .unwrap();
        graph
    }

    /// A picture holding the packages alone, as a cut at packages leaves it.
    fn packages() -> ArchitectureGraph {
        let mut packages = ArchitectureGraph::new();
        for package in ["package:a", "package:b", "package:c", "package:d"] {
            packages
                .add_element(Element::of_kind(
                    id(package),
                    ElementKind::Package,
                    ElementName::new(package).unwrap(),
                ))
                .unwrap();
        }
        packages
    }

    #[test]
    fn an_element_the_picture_already_holds_maps_to_itself() {
        let graph = graph();
        assert_eq!(
            boundary_in_view(&graph, &graph, &id("a/two")),
            Some(id("a/two"))
        );
    }

    #[test]
    fn an_element_below_the_picture_maps_to_the_boundary_that_holds_it() {
        assert_eq!(
            boundary_in_view(&packages(), &graph(), &id("a/one#type:X")),
            Some(id("package:a")),
            "the walk climbs until a boundary of this picture answers"
        );
    }

    #[test]
    fn an_element_no_boundary_holds_maps_nowhere() {
        let mut graph = graph();
        graph
            .add_element(Element::of_kind(
                id("stray"),
                ElementKind::Module,
                ElementName::new("stray").unwrap(),
            ))
            .unwrap();
        assert_eq!(boundary_in_view(&packages(), &graph, &id("stray")), None);
    }

    #[test]
    fn selecting_a_connection_lights_its_endpoints_alone() {
        let containment = Containment::of(&view());
        let edges = edges();
        let selected = depends("a/two", "b/one");
        let focus = focus_of(&containment, &edges, Selected::Edge(&selected));

        assert_eq!(focus.edge(&selected), Strength::Focused);
        assert_eq!(focus.edge(&depends("a/one", "a/two")), Strength::Faded);
        assert_eq!(focus.element(&id("a/two")), Strength::Focused);
        assert_eq!(focus.element(&id("b/one")), Strength::Focused);
        assert_eq!(focus.element(&id("a/one")), Strength::Faded);
        assert_eq!(focus.element(&id("package:a")), Strength::Context);
    }
}
