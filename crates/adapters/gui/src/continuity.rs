//! What a change of the whole picture's detail carries across.
//!
//! A new detail is a new picture, not a new question. The reader who studies
//! one boundary at packages studies the same boundary at modules; it merely
//! appears as another box, and the connection between two boundaries appears
//! under another name. Element ids hold across details by construction, and
//! a rolled-up connection remembers the concrete relations behind it, so
//! both subjects can be followed from one cut into the next.
//!
//! This module answers where a selection reappears, and whether the camera
//! still looks at the picture at all. Every answer is a pure function of the
//! two views and the full graph; the shell only applies them.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{ArchitectureGraph, ElementId, Relation};
use cutaway_lenses::{BoundaryView, is_self_leaf};
use eframe::egui::Rect;
use eframe::egui::emath::TSTransform;

use crate::Selection;
use crate::focus;

/// Where one selection made in `before` reappears in `after`. None when the
/// new picture holds nothing the selection could stand for.
pub(crate) fn translated(
    graph: &ArchitectureGraph,
    before: &BoundaryView,
    after: &BoundaryView,
    selection: &Selection,
) -> Option<Selection> {
    match selection {
        Selection::Node(id) => node_in(graph, before, after, id).map(Selection::Node),
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
) -> Option<Selection> {
    let carried = match before.provenance.get(relation) {
        Some(concrete) => most_alike(&after.provenance, concrete).cloned(),
        None => {
            (holds(after, &relation.from) && holds(after, &relation.to)).then(|| relation.clone())
        }
    };
    carried
        .map(Selection::Edge)
        .or_else(|| node_in(graph, before, after, &relation.from).map(Selection::Node))
}

fn holds(view: &BoundaryView, id: &ElementId) -> bool {
    view.graph.element(id).is_some()
}

/// The connection standing for most of the given concrete dependencies, and
/// None while no connection stands for any of them. Ties go to the heavier
/// connection - the one answering for more altogether - and an even tie to
/// the first in id order, so one graph always answers the same way.
fn most_alike<'a>(
    candidates: &'a BTreeMap<Relation, BTreeSet<Relation>>,
    concrete: &BTreeSet<Relation>,
) -> Option<&'a Relation> {
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
        .map(|(_, _, edge)| edge)
}

/// How much of the smaller of the two rectangles the camera and the picture
/// must share for the camera to have kept its subject.
const MEANINGFUL_OVERLAP: f32 = 0.1;

/// Whether the camera still looks at the picture.
///
/// A picture changes size with its detail: the items of a project dwarf the
/// packages of the same project. A camera left where it stood can therefore
/// end up over empty space, and a reader who asked for more detail would get
/// a blank canvas. The camera keeps its place while what it shows and what
/// the picture occupies still share a tenth of the smaller of the two, and
/// gives way to a fresh fit otherwise.
pub(crate) fn camera_holds(camera: TSTransform, viewport: Rect, world: Rect) -> bool {
    if !camera.is_valid() || !viewport.is_positive() || !world.is_positive() {
        return false;
    }
    let looked_at = camera.inverse().mul_rect(viewport);
    let shared = looked_at.intersect(world);
    if !shared.is_positive() {
        return false;
    }
    area(shared) >= MEANINGFUL_OVERLAP * area(looked_at).min(area(world))
}

fn area(rect: Rect) -> f32 {
    rect.width() * rect.height()
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{Element, ElementKind, ElementName, RelationKind};
    use cutaway_lenses::{Cut, Detail, boundary_view};
    use eframe::egui::{pos2, vec2};

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

    fn carried(from: Detail, to: Detail, selection: &Selection) -> Option<Selection> {
        let graph = graph();
        translated(&graph, &view(&graph, from), &view(&graph, to), selection)
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
            Some(&depends("c", "d")),
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
            Some(&depends("a", "b"))
        );
    }

    #[test]
    fn a_connection_sharing_nothing_answers_nothing() {
        let candidates = BTreeMap::from([(depends("a", "b"), behind([depends("x", "y")]))]);
        assert_eq!(most_alike(&candidates, &behind([depends("p", "q")])), None);
    }

    fn viewport() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0))
    }

    #[test]
    fn the_camera_refits_only_when_it_would_stare_at_nothing() {
        let camera = TSTransform::IDENTITY;
        let met = Rect::from_min_size(pos2(100.0, 100.0), vec2(600.0, 600.0));
        assert!(camera_holds(camera, viewport(), met));

        let far_away = Rect::from_min_size(pos2(5000.0, 5000.0), vec2(600.0, 600.0));
        assert!(!camera_holds(camera, viewport(), far_away));
    }

    #[test]
    fn a_camera_grazing_the_corner_of_a_picture_refits() {
        let grazed = Rect::from_min_size(pos2(390.0, 290.0), vec2(600.0, 600.0));
        assert!(!camera_holds(TSTransform::IDENTITY, viewport(), grazed));
    }

    #[test]
    fn a_camera_showing_the_whole_of_a_small_picture_keeps_its_place() {
        let small = Rect::from_min_size(pos2(150.0, 120.0), vec2(40.0, 30.0));
        assert!(camera_holds(TSTransform::IDENTITY, viewport(), small));
    }

    #[test]
    fn a_zoomed_camera_reads_the_world_it_magnifies() {
        // Twice magnified, the screen shows a quarter of the world it did.
        let camera = TSTransform::from_scaling(2.0);
        let inside = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 150.0));
        assert!(camera_holds(camera, viewport(), inside));

        let beyond = Rect::from_min_size(pos2(1000.0, 1000.0), vec2(200.0, 150.0));
        assert!(!camera_holds(camera, viewport(), beyond));
    }

    #[test]
    fn a_camera_without_a_viewport_or_a_picture_refits() {
        assert!(!camera_holds(
            TSTransform::IDENTITY,
            Rect::NOTHING,
            viewport()
        ));
        assert!(!camera_holds(
            TSTransform::IDENTITY,
            viewport(),
            Rect::NOTHING
        ));
    }
}
