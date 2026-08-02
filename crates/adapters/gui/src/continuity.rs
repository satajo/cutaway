//! What a change of the whole picture's detail carries across.
//!
//! A new detail is a new picture, not a new question. The reader who studies
//! one boundary at packages studies the same boundary at modules; it merely
//! appears as another box, and the connection between two boundaries appears
//! under another name. Element ids hold across details by construction, and
//! a rolled-up connection remembers the concrete relations behind it, so
//! both subjects can be followed from one cut into the next.
//!
//! This module answers where a selection reappears. Every answer is a pure
//! function of the two views and the full graph; the shell only applies
//! them.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{ArchitectureGraph, ElementId, Relation};
use cutaway_lenses::{BoundaryView, is_self_leaf};

use crate::Selection;
use crate::focus;

/// What became of one selection: where it reappears, and what it lost on the
/// way.
pub(crate) struct Carried {
    pub(crate) selection: Selection,
    /// Set only where a rolled-up connection split: the new picture draws
    /// the concrete dependencies behind the old one as several connections,
    /// and the selection follows the largest of them alone.
    pub(crate) piece: Option<Piece>,
}

/// How much of a split connection came along.
pub(crate) struct Piece {
    /// The concrete dependencies the new connection stands for.
    pub(crate) carried: usize,
    /// The concrete dependencies the old one stood for.
    pub(crate) whole: usize,
}

/// Where one selection made in `before` reappears in `after`. None when the
/// new picture holds nothing the selection could stand for.
pub(crate) fn translated(
    graph: &ArchitectureGraph,
    before: &BoundaryView,
    after: &BoundaryView,
    selection: &Selection,
) -> Option<Carried> {
    match selection {
        Selection::Node(id) => node_in(graph, before, after, id).map(|id| Carried {
            selection: Selection::Node(id),
            piece: None,
        }),
        Selection::Edge(relation) => edge_in(graph, before, after, relation),
    }
}

/// Where a selected boundary reappears. The rules, in order:
///
/// - A frame's own content is no boundary of its own, so it answers as the
///   frame it belongs to: that frame's place in the new picture, then the
///   own content of that place where the new picture grew one, and the place
///   itself where it did not.
/// - A boundary the new picture holds answers as itself. The kind levels
///   nest, so a finer cut keeps everything a coarser one showed.
/// - A boundary the new picture hides answers as the nearest boundary above
///   it, which is where its content rolled up to.
/// - A boundary no visible boundary holds answers nothing.
fn node_in(
    graph: &ArchitectureGraph,
    before: &BoundaryView,
    after: &BoundaryView,
    id: &ElementId,
) -> Option<ElementId> {
    if is_self_leaf(id) {
        let frame = focus::frame_of(&before.graph, id)?;
        let landed = focus::boundary_in_view(&after.graph, graph, frame)?;
        return Some(own_content_of(&after.graph, landed));
    }
    focus::boundary_in_view(&after.graph, graph, id)
}

/// The box carrying a boundary's own content: the boundary itself while it
/// is a leaf, and the own-content leaf inside it while it is a frame that
/// grew one. A frame whose content answers no dependency grows none, and
/// then the frame itself is the nearest the picture comes.
fn own_content_of(after: &ArchitectureGraph, boundary: ElementId) -> ElementId {
    focus::contents_of(after, &boundary)
        .into_iter()
        .find(|child| is_self_leaf(child))
        .cloned()
        .unwrap_or(boundary)
}

/// Where a selected connection reappears.
///
/// A rolled-up connection is named by the boundaries it joins, and those
/// change with the detail; the concrete dependencies behind it do not. The
/// connection therefore answers as the connection of the new picture
/// standing for most of the same concrete dependencies.
///
/// A connection with nothing concrete behind it is a planned one, and it
/// answers as itself wherever the new picture still holds both of its ends.
/// Where neither answers - a connection the new picture rolls into a
/// boundary's inside, or one held back at a coarser detail - the boundary it
/// departs answers, so the reader keeps a subject rather than an empty
/// panel.
fn edge_in(
    graph: &ArchitectureGraph,
    before: &BoundaryView,
    after: &BoundaryView,
    relation: &Relation,
) -> Option<Carried> {
    let carried = match before.provenance.get(relation) {
        Some(concrete) => most_alike(&after.provenance, concrete).map(|(edge, shared)| Carried {
            selection: Selection::Edge(edge.clone()),
            // A connection that came along whole says nothing; only a
            // reader following one piece of it needs telling.
            piece: (shared < concrete.len()).then_some(Piece {
                carried: shared,
                whole: concrete.len(),
            }),
        }),
        None => (holds(after, &relation.from) && holds(after, &relation.to)).then(|| Carried {
            selection: Selection::Edge(relation.clone()),
            piece: None,
        }),
    };
    carried.or_else(|| {
        node_in(graph, before, after, &relation.from).map(|id| Carried {
            selection: Selection::Node(id),
            piece: None,
        })
    })
}

fn holds(view: &BoundaryView, id: &ElementId) -> bool {
    view.graph.element(id).is_some()
}

/// The connection standing for most of the given concrete dependencies and
/// how many of them it stands for, and None while no connection stands for
/// any of them. Ties go to the heavier connection - the one answering for
/// more altogether - and an even tie to the first in id order, so one graph
/// always answers the same way.
fn most_alike<'a>(
    candidates: &'a BTreeMap<Relation, BTreeSet<Relation>>,
    concrete: &BTreeSet<Relation>,
) -> Option<(&'a Relation, usize)> {
    candidates
        .iter()
        .filter_map(|(edge, behind)| {
            let shared = behind.intersection(concrete).count();
            (shared > 0).then_some((shared, behind.len(), edge))
        })
        .max_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.cmp(&right.1))
                // The first in id order wins, so it must compare greatest.
                .then_with(|| right.2.cmp(left.2))
        })
        .map(|(shared, _, edge)| (edge, shared))
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{Element, ElementKind, ElementName, RelationKind};
    use cutaway_lenses::{Cut, Detail, boundary_view};

    use super::*;

    fn id(text: &str) -> ElementId {
        ElementId::new(text).unwrap()
    }

    fn add(graph: &mut ArchitectureGraph, id_text: &str, kind: ElementKind) {
        graph
            .add_element(Element {
                id: id(id_text),
                name: ElementName::new(id_text).unwrap(),
                kind,
            })
            .unwrap();
    }

    fn relate(graph: &mut ArchitectureGraph, from: &str, to: &str, kind: RelationKind) {
        graph
            .add_relation(Relation {
                from: id(from),
                to: id(to),
                kind,
            })
            .unwrap();
    }

    fn depends(from: &str, to: &str) -> Relation {
        Relation {
            from: id(from),
            to: id(to),
            kind: RelationKind::DependsOn,
        }
    }

    /// project ⊃ {package:a ⊃ {a/one ⊃ a/one#type:X, a/two},
    /// package:b ⊃ b/one, package:c ⊃ c/one ⊃ {c/one/inner, c/one#type:Y},
    /// stray}. Everything on the left reaches b/one, and a/one also reaches
    /// a/two beside it. a/one is a leaf at modules and a frame at items;
    /// c/one is a frame at both.
    fn graph() -> ArchitectureGraph {
        let mut graph = ArchitectureGraph::new();
        add(&mut graph, "project", ElementKind::Project);
        for package in ["package:a", "package:b", "package:c"] {
            add(&mut graph, package, ElementKind::Package);
        }
        for module in ["a/one", "a/two", "b/one", "c/one", "c/one/inner", "stray"] {
            add(&mut graph, module, ElementKind::Module);
        }
        add(&mut graph, "a/one#type:X", ElementKind::Type);
        add(&mut graph, "c/one#type:Y", ElementKind::Type);
        for (frame, inner) in [
            ("project", "package:a"),
            ("project", "package:b"),
            ("project", "package:c"),
            ("project", "stray"),
            ("package:a", "a/one"),
            ("package:a", "a/two"),
            ("package:b", "b/one"),
            ("package:c", "c/one"),
            ("a/one", "a/one#type:X"),
            ("c/one", "c/one/inner"),
            ("c/one", "c/one#type:Y"),
        ] {
            relate(&mut graph, frame, inner, RelationKind::Contains);
        }
        for (from, to) in [
            ("a/one", "b/one"),
            ("a/one#type:X", "b/one"),
            ("a/two", "b/one"),
            ("a/one", "a/two"),
            ("c/one", "b/one"),
            ("c/one#type:Y", "b/one"),
        ] {
            relate(&mut graph, from, to, RelationKind::DependsOn);
        }
        graph
    }

    fn view(graph: &ArchitectureGraph, detail: Detail) -> BoundaryView {
        boundary_view(graph, &Cut::uniform(detail)).unwrap()
    }

    fn crossing(from: Detail, to: Detail, selection: &Selection) -> Option<Carried> {
        let graph = graph();
        translated(&graph, &view(&graph, from), &view(&graph, to), selection)
    }

    fn carried(from: Detail, to: Detail, selection: &Selection) -> Option<Selection> {
        crossing(from, to, selection).map(|carried| carried.selection)
    }

    #[test]
    fn a_selected_boundary_survives_a_coarser_cut_as_its_ancestor() {
        assert_eq!(
            carried(
                Detail::Modules,
                Detail::Packages,
                &Selection::Node(id("a/one"))
            ),
            Some(Selection::Node(id("package:a")))
        );
    }

    #[test]
    fn a_selected_boundary_a_finer_cut_still_shows_stays_the_selection() {
        assert_eq!(
            carried(
                Detail::Packages,
                Detail::Modules,
                &Selection::Node(id("package:a"))
            ),
            Some(Selection::Node(id("package:a"))),
            "every detail showing modules shows the packages around them"
        );
    }

    #[test]
    fn a_boundary_that_opens_into_a_frame_stays_the_selection_itself() {
        assert_eq!(
            carried(
                Detail::Modules,
                Detail::Items,
                &Selection::Node(id("a/one"))
            ),
            Some(Selection::Node(id("a/one"))),
            "the reader selected the boundary, not what it holds"
        );
    }

    #[test]
    fn the_own_content_of_a_frame_follows_the_frame_it_belongs_to() {
        assert_eq!(
            carried(
                Detail::Items,
                Detail::Modules,
                &Selection::Node(id("a/one#self"))
            ),
            Some(Selection::Node(id("a/one"))),
            "the frame is a single box at modules, and it carries its own content"
        );
    }

    #[test]
    fn the_own_content_of_a_frame_stays_own_content_where_the_new_picture_holds_it() {
        assert_eq!(
            carried(
                Detail::Modules,
                Detail::Items,
                &Selection::Node(id("c/one#self"))
            ),
            Some(Selection::Node(id("c/one#self")))
        );
    }

    #[test]
    fn a_boundary_no_visible_boundary_holds_carries_nothing() {
        assert_eq!(
            carried(
                Detail::Modules,
                Detail::Packages,
                &Selection::Node(id("stray"))
            ),
            None,
            "a module outside every package rolls up to nothing a picture of packages holds"
        );
    }

    #[test]
    fn a_selected_connection_translates_through_its_concrete_relations() {
        assert_eq!(
            carried(
                Detail::Modules,
                Detail::Packages,
                &Selection::Edge(depends("a/one", "b/one"))
            ),
            Some(Selection::Edge(depends("package:a", "package:b")))
        );
    }

    #[test]
    fn a_connection_answers_with_the_finer_one_standing_for_most_of_it() {
        assert_eq!(
            carried(
                Detail::Packages,
                Detail::Modules,
                &Selection::Edge(depends("package:a", "package:b"))
            ),
            Some(Selection::Edge(depends("a/one", "b/one"))),
            "a/one carries two of the three concrete dependencies, a/two the third"
        );
    }

    #[test]
    fn a_partial_edge_translation_says_so() {
        let piece = crossing(
            Detail::Packages,
            Detail::Modules,
            &Selection::Edge(depends("package:a", "package:b")),
        )
        .expect("the connection reappears between the modules behind it")
        .piece
        .expect("one of the three concrete dependencies stayed behind");

        assert_eq!((piece.carried, piece.whole), (2, 3));
    }

    #[test]
    fn a_connection_that_came_along_whole_reports_no_piece() {
        let carried = crossing(
            Detail::Modules,
            Detail::Packages,
            &Selection::Edge(depends("a/one", "b/one")),
        )
        .expect("the connection reappears between the packages above it");

        assert!(
            carried.piece.is_none(),
            "a coarser picture gathers the whole connection, and gathers more besides"
        );
    }

    #[test]
    fn a_connection_the_coarser_picture_swallows_leaves_the_boundary_it_departs_selected() {
        assert_eq!(
            carried(
                Detail::Modules,
                Detail::Packages,
                &Selection::Edge(depends("a/one", "a/two"))
            ),
            Some(Selection::Node(id("package:a"))),
            "a dependency inside one package draws no edge at packages"
        );
    }

    #[test]
    fn a_connection_with_nothing_concrete_behind_it_survives_where_both_its_ends_do() {
        let planned = Selection::Edge(depends("b/one", "a/two"));
        assert_eq!(
            carried(Detail::Modules, Detail::Items, &planned),
            Some(planned.clone()),
            "a planned dependency stands for itself alone"
        );
    }

    fn behind<const N: usize>(concrete: [Relation; N]) -> BTreeSet<Relation> {
        concrete.into_iter().collect()
    }

    #[test]
    fn a_tie_between_connections_goes_to_the_heavier_one() {
        let candidates = BTreeMap::from([
            (depends("a", "b"), behind([depends("x", "y")])),
            (
                depends("c", "d"),
                behind([depends("x", "y"), depends("p", "q")]),
            ),
        ]);
        assert_eq!(
            most_alike(&candidates, &behind([depends("x", "y")])),
            Some((&depends("c", "d"), 1)),
            "both stand for the shared dependency, one stands for more besides"
        );
    }

    #[test]
    fn an_even_tie_between_connections_goes_to_the_first_in_order() {
        let candidates = BTreeMap::from([
            (depends("a", "b"), behind([depends("x", "y")])),
            (depends("c", "d"), behind([depends("x", "y")])),
        ]);
        assert_eq!(
            most_alike(&candidates, &behind([depends("x", "y")])),
            Some((&depends("a", "b"), 1))
        );
    }

    #[test]
    fn a_connection_sharing_nothing_answers_nothing() {
        let candidates = BTreeMap::from([(depends("a", "b"), behind([depends("x", "y")]))]);
        assert_eq!(most_alike(&candidates, &behind([depends("p", "q")])), None);
    }
}
