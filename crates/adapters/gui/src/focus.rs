//! What a selection puts in focus.
//!
//! Selecting a boundary asks "what does this depend on". Dependency edges
//! attach only to leaves, so a frame carries no edge of its own: its answer
//! lies in the edges of everything it contains. A selection therefore
//! focuses a whole subtree - the selected boundary and its descendants -
//! together with every edge that touches the subtree and the partners at the
//! far end of those edges. The frames around the focused elements stay
//! readable as context, so a highlighted boundary still says which boundary
//! it sits in.
//!
//! The computation is pure view logic: it reads the view graph and the drawn
//! edges and answers with a strength per element and per edge. The canvas
//! only applies the answer.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{ArchitectureGraph, ElementId, Relation, RelationKind};

/// How strongly one element or edge paints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Strength {
    /// The selection and what it reaches: full color.
    Focused,
    /// The frames around focused elements: readable, but not a subject.
    Context,
    /// Everything else.
    Faded,
}

/// What the user selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Selected<'a> {
    Node(&'a ElementId),
    Edge(&'a Relation),
}

/// The strengths one selection produces.
pub(crate) struct Focus<'a> {
    subjects: BTreeSet<&'a ElementId>,
    context: BTreeSet<&'a ElementId>,
    lit: BTreeSet<&'a Relation>,
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
        if self.lit.contains(relation) {
            Strength::Focused
        } else {
            Strength::Faded
        }
    }
}

/// The focus of one selection over `view`, given the edges the canvas draws.
pub(crate) fn focus_of<'a>(
    view: &'a ArchitectureGraph,
    edges: impl IntoIterator<Item = &'a Relation>,
    selected: Selected<'a>,
) -> Focus<'a> {
    let mut subjects = BTreeSet::new();
    let mut lit = BTreeSet::new();
    match selected {
        Selected::Node(id) => {
            let inside = subtree_of(view, id);
            for edge in edges {
                if !inside.contains(&edge.from) && !inside.contains(&edge.to) {
                    continue;
                }
                lit.insert(edge);
                subjects.insert(&edge.from);
                subjects.insert(&edge.to);
            }
            subjects.extend(inside);
        }
        Selected::Edge(relation) => {
            lit.insert(relation);
            subjects.insert(&relation.from);
            subjects.insert(&relation.to);
        }
    }
    let context = ancestors_of(view, &subjects);
    Focus {
        subjects,
        context,
        lit,
    }
}

/// A boundary and everything it contains, however deep.
pub(crate) fn subtree_of<'a>(
    view: &'a ArchitectureGraph,
    root: &'a ElementId,
) -> BTreeSet<&'a ElementId> {
    let children = children_of(view);
    let mut inside = BTreeSet::from([root]);
    let mut queue = vec![root];
    while let Some(current) = queue.pop() {
        for child in children.get(current).into_iter().flatten() {
            if inside.insert(*child) {
                queue.push(*child);
            }
        }
    }
    inside
}

/// The boundaries one boundary directly contains, in id order. A boundary
/// with contents paints as a frame, and no dependency edge touches a frame:
/// an empty answer names a leaf.
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

/// The containing boundaries above the given elements, excluding the
/// elements themselves.
fn ancestors_of<'a>(
    view: &'a ArchitectureGraph,
    of: &BTreeSet<&'a ElementId>,
) -> BTreeSet<&'a ElementId> {
    let mut parents: BTreeMap<&ElementId, &ElementId> = BTreeMap::new();
    for relation in view.relations() {
        if relation.kind == RelationKind::Contains {
            parents.insert(&relation.to, &relation.from);
        }
    }
    let mut context = BTreeSet::new();
    for id in of {
        // Containment of a view is a tree, but a walk that trusts that and
        // meets a cycle never ends; the seen set bounds every walk.
        let mut seen = BTreeSet::new();
        let mut current = parents.get(*id).copied();
        while let Some(parent) = current {
            if !seen.insert(parent) {
                break;
            }
            if !of.contains(parent) {
                context.insert(parent);
            }
            current = parents.get(parent).copied();
        }
    }
    context
}

fn children_of(view: &ArchitectureGraph) -> BTreeMap<&ElementId, Vec<&ElementId>> {
    let mut children: BTreeMap<&ElementId, Vec<&ElementId>> = BTreeMap::new();
    for relation in view.relations() {
        if relation.kind == RelationKind::Contains {
            children
                .entry(&relation.from)
                .or_default()
                .push(&relation.to);
        }
    }
    children
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
                .add_element(Element {
                    id: id(element),
                    name: ElementName::new(element).unwrap(),
                    kind,
                })
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
        let view = view();
        let edges = edges();
        let selected = id("package:a");
        let focus = focus_of(&view, &edges, Selected::Node(&selected));

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
        let view = view();
        let edges = edges();
        let frame = id("package:a");
        let focus = focus_of(&view, &edges, Selected::Node(&frame));
        assert_eq!(
            focus.element(&id("package:b")),
            Strength::Context,
            "the frame around the highlighted partner names it"
        );

        let leaf = id("a/one");
        let focus = focus_of(&view, &edges, Selected::Node(&leaf));
        assert_eq!(focus.element(&id("package:a")), Strength::Context);
    }

    #[test]
    fn edges_fully_outside_a_selected_frame_fade() {
        let view = view();
        let edges = edges();
        let selected = id("package:a");
        let focus = focus_of(&view, &edges, Selected::Node(&selected));

        assert_eq!(
            focus.edge(&depends("package:c", "package:d")),
            Strength::Faded
        );
        assert_eq!(focus.element(&id("package:c")), Strength::Faded);
        assert_eq!(focus.element(&id("package:d")), Strength::Faded);
    }

    #[test]
    fn selecting_a_leaf_lights_only_the_dependencies_that_touch_it() {
        let view = view();
        let edges = edges();
        let selected = id("a/one");
        let focus = focus_of(&view, &edges, Selected::Node(&selected));

        assert_eq!(focus.edge(&depends("a/one", "a/two")), Strength::Focused);
        assert_eq!(focus.element(&id("a/two")), Strength::Focused);
        assert_eq!(focus.edge(&depends("a/two", "b/one")), Strength::Faded);
        assert_eq!(focus.element(&id("b/one")), Strength::Faded);
    }

    #[test]
    fn selecting_a_connection_lights_its_endpoints_alone() {
        let view = view();
        let edges = edges();
        let selected = depends("a/two", "b/one");
        let focus = focus_of(&view, &edges, Selected::Edge(&selected));

        assert_eq!(focus.edge(&selected), Strength::Focused);
        assert_eq!(focus.edge(&depends("a/one", "a/two")), Strength::Faded);
        assert_eq!(focus.element(&id("a/two")), Strength::Focused);
        assert_eq!(focus.element(&id("b/one")), Strength::Focused);
        assert_eq!(focus.element(&id("a/one")), Strength::Faded);
        assert_eq!(focus.element(&id("package:a")), Strength::Context);
    }
}
