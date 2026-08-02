//! The inspector panel: what the selection is, and where to go from it.
//!
//! Every list the panel shows is also a way to travel. A row names a
//! boundary or a connection and selects it, so reading the architecture and
//! moving through it are one act. A row selects through `Session::select`
//! and `Session::reveal`, exactly as a click on the canvas does, so the note
//! editor and the picture follow a row as they follow a click.
//!
//! The rows themselves come from pure functions over the boundary view and
//! the full graph. The panel only paints what they answer.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{ArchitectureGraph, ElementId, Relation};
use cutaway_lenses::{BoundaryView, is_self_leaf};
use eframe::egui;

use crate::canvas::{self, EdgeStatus};
use crate::focus;
use crate::label::{Labels, kind_name, kind_symbol};
use crate::{Scene, Selection, Session, detail_label};

/// How many rows one list shows before it names the rest. The cap is a
/// display limit and not a data limit: the count above every list still
/// speaks for all of it, and the canvas draws all of it.
const ROW_LIMIT: usize = 15;

pub(crate) fn show(ui: &mut egui::Ui, session: &mut Session) {
    egui::ScrollArea::vertical().show(ui, |ui| match session.selection.clone() {
        None => nothing_selected(ui, session),
        Some(Selection::Node(id)) => node(ui, session, &id),
        Some(Selection::Edge(relation)) => edge(ui, session, &relation),
    });
}

/// One row of a list: what it says, and what clicking it selects.
#[derive(Debug)]
struct Row {
    text: String,
    /// None when the row names something this view holds no boundary for.
    /// The row still reads; it just leads nowhere.
    target: Option<Selection>,
}

fn nothing_selected(ui: &mut egui::Ui, session: &mut Session) {
    ui.heading("Boundaries");
    let mut chosen = None;
    if let Ok(Scene { view, .. }) = &session.scene {
        let labels = Labels::of(&view.graph);
        ui.label(format!(
            "{} boundaries, {} connections.",
            view.graph.elements().count(),
            view.provenance.len()
        ));
        if !view.coarse.is_empty() {
            ui.separator();
            ui.label("Waiting at coarser detail:");
            ui.small(format!(
                "{} connections name a boundary with visible children as a whole. \
                 They show at a coarser detail.",
                view.coarse.len()
            ));
            chosen = list(ui, &coarse_rows(view, &labels)).or(chosen);
        }
        if !view.unscoped.is_empty() {
            ui.separator();
            ui.label("Outside every boundary:");
            ui.small(format!(
                "{} dependencies fall outside every boundary.",
                view.unscoped.len()
            ));
            chosen = list(ui, &unscoped_rows(view, &session.graph)).or(chosen);
        }
    }
    if let Some(target) = chosen {
        go_to(session, target);
    }
    ui.separator();
    help(ui);
}

fn help(ui: &mut egui::Ui) {
    ui.label(
        "Select a node or a connection to annotate it. Selecting a boundary keeps \
         everything inside it, and every dependency that crosses its border, at full \
         strength.",
    );
    ui.label("A box grows with the number of concepts inside it.");
    ui.label(
        "Double-click a boundary to open it one level deeper than the rest of the \
         picture; select it to expand or collapse it from here.",
    );
    ui.label(
        "Press ctrl+F or / to search every element by name, however the picture is \
         cut; enter opens the picture down to the one you choose.",
    );
    ui.label(
        "Severed connections turn red, drawn ones green; the plan saves to \
         cutaway.json in the repository.",
    );
    ui.label("Drag or scroll to pan, ctrl+scroll or pinch to zoom, double-click the background to refit.");
}

fn node(ui: &mut egui::Ui, session: &mut Session, id: &ElementId) {
    let Some(panel) = node_panel(session, id) else {
        return;
    };
    ui.heading(panel.heading);
    ui.small(panel.kind);
    // The id is the element's path through the sources: a reader knows the
    // boundary by it even where two short names read alike.
    ui.small(egui::RichText::new(id.as_str()).monospace());
    detail_controls(ui, session, id);
    note_editor(ui, session);
    let mut chosen = None;
    if !panel.contents.is_empty() {
        ui.separator();
        ui.label("Contains:");
        chosen = list(ui, &panel.contents).or(chosen);
    }
    ui.separator();
    ui.label("Connections:");
    if panel.connections.is_empty() {
        ui.label("Nothing crosses this boundary.");
    } else {
        chosen = list(ui, &panel.connections).or(chosen);
    }
    if let Some(target) = chosen {
        go_to(session, target);
    }
}

fn edge(ui: &mut egui::Ui, session: &mut Session, relation: &Relation) {
    let Some(panel) = edge_panel(session, relation) else {
        return;
    };
    ui.heading(panel.heading);
    match session.status_of(relation) {
        EdgeStatus::Existing => {
            ui.label("Existing dependency.");
            if ui.button("Sever").clicked() {
                session.sever(relation);
            }
        }
        EdgeStatus::Severed => {
            ui.colored_label(canvas::SEVERED, "Planned for removal.");
            if ui.button("Restore").clicked() {
                session.restore(relation);
            }
        }
        EdgeStatus::Drawn => {
            ui.colored_label(canvas::DRAWN, "Planned addition.");
            if ui.button("Erase").clicked() {
                session.sever(relation);
            }
        }
    }
    note_editor(ui, session);
    if panel.provenance.is_empty() {
        return;
    }
    ui.separator();
    ui.label(format!(
        "Stands for {} concrete dependencies:",
        panel.provenance.len()
    ));
    ui.small("Click a row to jump to its source.");
    if let Some(target) = list(ui, &panel.provenance) {
        go_to(session, target);
    }
}

/// Opens or closes this one boundary on top of the detail the rest of the
/// picture follows, so the project stays whole while the boundary under
/// study shows its parts. Double-clicking the boundary expands it too.
fn detail_controls(ui: &mut egui::Ui, session: &mut Session, id: &ElementId) {
    let Some(within) = session.detail_within(id) else {
        return;
    };
    ui.separator();
    ui.label(format!("Shows: {}", detail_label(within).to_lowercase()));
    ui.horizontal(|ui| {
        if ui
            .add_enabled(within.deeper().is_some(), egui::Button::new("Expand"))
            .clicked()
        {
            session.expand(id);
        }
        if ui
            .add_enabled(within.shallower().is_some(), egui::Button::new("Collapse"))
            .clicked()
        {
            session.collapse(id);
        }
    });
}

fn note_editor(ui: &mut egui::Ui, session: &mut Session) {
    ui.separator();
    ui.label("Note:");
    ui.text_edit_multiline(&mut session.note_draft);
    if ui.button("Save note").clicked() {
        session.save_note();
    }
}

/// Selects what a row names and brings it into view, exactly as a click on
/// the canvas would.
fn go_to(session: &mut Session, target: Selection) {
    let shown = revealed(&target).clone();
    session.select(Some(target));
    session.reveal(&shown);
}

/// What a selection asks the camera to show. A connection shows at the end
/// it leaves: that is where the reader's question started.
fn revealed(selection: &Selection) -> &ElementId {
    match selection {
        Selection::Node(id) => id,
        Selection::Edge(relation) => &relation.from,
    }
}

/// Paints one list of rows and answers with the selection a click asked
/// for. Rows past the cap collapse into a line that counts them.
fn list(ui: &mut egui::Ui, rows: &[Row]) -> Option<Selection> {
    let (shown, held_back) = capped(rows);
    let mut chosen = None;
    for row in shown {
        match &row.target {
            Some(target) => {
                if link(ui, &row.text).clicked() {
                    chosen = Some(target.clone());
                }
            }
            None => {
                ui.small(row.text.as_str());
            }
        }
    }
    if held_back > 0 {
        ui.small(format!("… and {held_back} more"));
    }
    chosen
}

/// The rows a list shows, and how many the cap holds back.
fn capped<T>(rows: &[T]) -> (&[T], usize) {
    let shown = rows.len().min(ROW_LIMIT);
    (&rows[..shown], rows.len() - shown)
}

/// One clickable row. A boundary name can be long, and a wrapped row reads
/// as two entries, so the row truncates instead and shows its whole text
/// when the pointer rests on it.
fn link(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let color = ui.visuals().hyperlink_color;
    ui.add(
        egui::Label::new(egui::RichText::new(text).color(color))
            .truncate()
            .sense(egui::Sense::click()),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// What the panel says about a selected boundary.
struct NodePanel {
    heading: String,
    kind: &'static str,
    contents: Vec<Row>,
    connections: Vec<Row>,
}

fn node_panel(session: &Session, id: &ElementId) -> Option<NodePanel> {
    let Ok(Scene { view, .. }) = &session.scene else {
        return None;
    };
    let labels = Labels::of(&view.graph);
    let element = session.element_of(id)?;
    Some(NodePanel {
        heading: format!("{} {}", kind_symbol(element.kind), labels.qualified(id)),
        kind: kind_name(element.kind),
        contents: contents_rows(&view.graph, &labels, id),
        connections: connection_rows(view, &labels, id),
    })
}

/// What the panel says about a selected connection.
struct EdgePanel {
    heading: String,
    provenance: Vec<Row>,
}

fn edge_panel(session: &Session, relation: &Relation) -> Option<EdgePanel> {
    let Ok(Scene { view, .. }) = &session.scene else {
        return None;
    };
    let labels = Labels::of(&view.graph);
    Some(EdgePanel {
        heading: format!(
            "{} → {}",
            labels.qualified(&relation.from),
            labels.qualified(&relation.to)
        ),
        provenance: provenance_rows(view, &session.graph, relation),
    })
}

/// The boundaries a frame directly holds.
fn contents_rows(view: &ArchitectureGraph, labels: &Labels<'_>, id: &ElementId) -> Vec<Row> {
    focus::contents_of(view, id)
        .into_iter()
        .map(|child| Row {
            // A frame's own content needs no name here: the frame it
            // belongs to is the heading right above the list.
            text: if is_self_leaf(child) {
                "own content".to_owned()
            } else {
                labels.label(child).text()
            },
            target: Some(Selection::Node(child.clone())),
        })
        .collect()
}

/// What a boundary connects to: the drawn edges that touch it or - for a
/// frame, which carries no edge of its own - the edges that cross its
/// border. Outgoing first, heaviest first.
fn connection_rows(view: &BoundaryView, labels: &Labels<'_>, id: &ElementId) -> Vec<Row> {
    let crossing = crossings(view, &focus::subtree_of(&view.graph, id));
    let row = |arrow: &str, crossing: &Crossing<'_>| Row {
        text: format!(
            "{arrow} {} ({})",
            labels.qualified(crossing.partner),
            crossing.concrete
        ),
        target: Some(Selection::Edge(crossing.edge.clone())),
    };
    crossing
        .outward
        .iter()
        .map(|crossing| row("→", crossing))
        .chain(crossing.inward.iter().map(|crossing| row("←", crossing)))
        .collect()
}

/// The rolled-up edges this view holds back. Each names a frame as a whole,
/// so a row selects that frame.
fn coarse_rows(view: &BoundaryView, labels: &Labels<'_>) -> Vec<Row> {
    let mut ranked: Vec<(&Relation, usize)> = view
        .coarse
        .iter()
        .map(|(edge, concrete)| (edge, concrete.len()))
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    ranked
        .into_iter()
        .map(|(edge, concrete)| Row {
            text: format!(
                "{} → {}, {concrete} concrete",
                labels.qualified(&edge.from),
                labels.qualified(&edge.to)
            ),
            target: Some(Selection::Node(edge.to.clone())),
        })
        .collect()
}

/// The dependencies with an endpoint no boundary holds. They lead nowhere:
/// the view has no boundary to select for them.
fn unscoped_rows(view: &BoundaryView, graph: &ArchitectureGraph) -> Vec<Row> {
    view.unscoped
        .iter()
        .map(|relation| Row {
            text: format!(
                "{} → {}",
                name_of(graph, &relation.from),
                name_of(graph, &relation.to)
            ),
            target: None,
        })
        .collect()
}

/// The concrete dependencies behind one rolled-up edge, each leading to the
/// boundary its source shows up as at this detail.
fn provenance_rows(
    view: &BoundaryView,
    graph: &ArchitectureGraph,
    relation: &Relation,
) -> Vec<Row> {
    view.provenance
        .get(relation)
        .into_iter()
        .flatten()
        .map(|concrete| Row {
            text: format!(
                "{} → {}",
                name_of(graph, &concrete.from),
                name_of(graph, &concrete.to)
            ),
            target: focus::boundary_in_view(&view.graph, graph, &concrete.from)
                .map(Selection::Node),
        })
        .collect()
}

/// A concrete element as its sources name it. Rows about concrete
/// dependencies speak of elements below the detail the view cuts at, and
/// only the full graph knows those.
fn name_of(graph: &ArchitectureGraph, id: &ElementId) -> String {
    graph
        .element(id)
        .map_or_else(|| id.to_string(), |element| element.name.to_string())
}

/// The dependencies that cross one boundary's border, gathered by the
/// partner on the far side, heaviest first.
struct Crossings<'a> {
    outward: Vec<Crossing<'a>>,
    inward: Vec<Crossing<'a>>,
}

/// One partner beyond the border: how many concrete dependencies reach it,
/// and the rolled-up edge a row about it selects.
struct Crossing<'a> {
    partner: &'a ElementId,
    concrete: usize,
    /// The heaviest edge to the partner. A boundary can reach one partner
    /// through several of its parts, and a row selects a single edge: the
    /// heaviest is the one the canvas draws thickest.
    edge: &'a Relation,
}

/// Partner -> the concrete dependencies reaching it in total, what the
/// heaviest edge to it stands for, and that edge.
type Tally<'a> = BTreeMap<&'a ElementId, (usize, usize, &'a Relation)>;

fn crossings<'a>(view: &'a BoundaryView, inside: &BTreeSet<&ElementId>) -> Crossings<'a> {
    let mut outward = Tally::new();
    let mut inward = Tally::new();
    for (edge, concrete) in &view.provenance {
        let (side, partner) = match (inside.contains(&edge.from), inside.contains(&edge.to)) {
            (true, false) => (&mut outward, &edge.to),
            (false, true) => (&mut inward, &edge.from),
            _ => continue,
        };
        let entry = side.entry(partner).or_insert((0, 0, edge));
        entry.0 += concrete.len();
        if concrete.len() > entry.1 {
            entry.1 = concrete.len();
            entry.2 = edge;
        }
    }
    Crossings {
        outward: ranked(outward),
        inward: ranked(inward),
    }
}

fn ranked(tally: Tally<'_>) -> Vec<Crossing<'_>> {
    let mut ranked: Vec<Crossing<'_>> = tally
        .into_iter()
        .map(|(partner, (concrete, _, edge))| Crossing {
            partner,
            concrete,
            edge,
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.concrete
            .cmp(&a.concrete)
            .then_with(|| a.partner.cmp(b.partner))
    });
    ranked
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

    /// package:a ⊃ {a/one, a/two ⊃ a/two#type:X}, package:b ⊃ {b/one},
    /// package:c ⊃ {c/one}. a/two reaches b/one twice, once through the
    /// type inside it; a/one reaches b/one and c/one once each; b/one
    /// reaches back into a/one.
    fn graph() -> ArchitectureGraph {
        let mut graph = ArchitectureGraph::new();
        for package in ["package:a", "package:b", "package:c"] {
            add(&mut graph, package, ElementKind::Package);
        }
        for module in ["a/one", "a/two", "b/one", "c/one"] {
            add(&mut graph, module, ElementKind::Module);
        }
        add(&mut graph, "a/two#type:X", ElementKind::Type);
        for (frame, inner) in [
            ("package:a", "a/one"),
            ("package:a", "a/two"),
            ("package:b", "b/one"),
            ("package:c", "c/one"),
            ("a/two", "a/two#type:X"),
        ] {
            relate(&mut graph, frame, inner, RelationKind::Contains);
        }
        for (from, to) in [
            ("a/one", "b/one"),
            ("a/two", "b/one"),
            ("a/two#type:X", "b/one"),
            ("a/one", "c/one"),
            ("b/one", "a/one"),
        ] {
            relate(&mut graph, from, to, RelationKind::DependsOn);
        }
        graph
    }

    fn rows_of(view: &BoundaryView, id_text: &str) -> Vec<Row> {
        let labels = Labels::of(&view.graph);
        connection_rows(view, &labels, &id(id_text))
    }

    fn texts(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|row| row.text.as_str()).collect()
    }

    #[test]
    fn a_boundary_lists_what_it_depends_on_before_what_depends_on_it() {
        let view = boundary_view(&graph(), &Cut::uniform(Detail::Modules)).unwrap();
        assert_eq!(
            texts(&rows_of(&view, "package:a")),
            ["→ b/one (3)", "→ c/one (1)", "← b/one (1)"]
        );
    }

    #[test]
    fn a_connection_row_counts_every_concrete_dependency_behind_it() {
        let view = boundary_view(&graph(), &Cut::uniform(Detail::Modules)).unwrap();
        let rows = rows_of(&view, "a/two");
        assert_eq!(
            texts(&rows),
            ["→ b/one (2)"],
            "the type inside a/two counts toward the module that holds it"
        );
    }

    #[test]
    fn a_connection_row_selects_the_heaviest_edge_to_its_partner() {
        let view = boundary_view(&graph(), &Cut::uniform(Detail::Modules)).unwrap();
        let rows = rows_of(&view, "package:a");
        assert_eq!(
            rows[0].target,
            Some(Selection::Edge(depends("a/two", "b/one"))),
            "package:a reaches b/one through both of its modules"
        );
    }

    #[test]
    fn the_connections_of_a_leaf_are_the_edges_that_touch_it() {
        let view = boundary_view(&graph(), &Cut::uniform(Detail::Modules)).unwrap();
        let rows = rows_of(&view, "a/one");
        assert_eq!(texts(&rows), ["→ b/one (1)", "→ c/one (1)", "← b/one (1)"]);
        assert_eq!(
            rows[2].target,
            Some(Selection::Edge(depends("b/one", "a/one")))
        );
    }

    #[test]
    fn a_frame_lists_the_boundaries_it_directly_holds() {
        let view = boundary_view(&graph(), &Cut::uniform(Detail::Modules)).unwrap();
        let labels = Labels::of(&view.graph);
        let rows = contents_rows(&view.graph, &labels, &id("package:a"));
        assert_eq!(texts(&rows), ["▤ a/one", "▤ a/two"]);
        assert_eq!(rows[0].target, Some(Selection::Node(id("a/one"))));
    }

    #[test]
    fn hidden_connections_list_the_heaviest_first_and_lead_to_the_boundary_they_name() {
        let mut graph = graph();
        add(&mut graph, "c/one#type:Y", ElementKind::Type);
        relate(&mut graph, "c/one", "c/one#type:Y", RelationKind::Contains);
        for from in ["package:a", "c/one", "c/one#type:Y"] {
            relate(&mut graph, from, "package:b", RelationKind::DependsOn);
        }
        let view = boundary_view(&graph, &Cut::uniform(Detail::Modules)).unwrap();
        let labels = Labels::of(&view.graph);
        let rows = coarse_rows(&view, &labels);
        assert_eq!(
            texts(&rows),
            [
                "c/one → package:b, 2 concrete",
                "package:a → package:b, 1 concrete"
            ]
        );
        assert_eq!(rows[0].target, Some(Selection::Node(id("package:b"))));
    }

    #[test]
    fn a_dependency_outside_every_boundary_leads_nowhere() {
        let mut graph = graph();
        add(&mut graph, "project", ElementKind::Project);
        add(&mut graph, "stray", ElementKind::Module);
        relate(&mut graph, "project", "stray", RelationKind::Contains);
        relate(&mut graph, "stray", "b/one", RelationKind::DependsOn);
        let view = boundary_view(&graph, &Cut::uniform(Detail::Packages)).unwrap();
        let rows = unscoped_rows(&view, &graph);
        assert_eq!(texts(&rows), ["stray → b/one"]);
        assert_eq!(rows[0].target, None);
    }

    #[test]
    fn a_concrete_dependency_leads_to_the_boundary_its_source_shows_up_as() {
        let graph = graph();
        let view = boundary_view(&graph, &Cut::uniform(Detail::Packages)).unwrap();
        let rows = provenance_rows(&view, &graph, &depends("package:a", "package:b"));
        assert_eq!(
            texts(&rows),
            ["a/one → b/one", "a/two → b/one", "a/two#type:X → b/one"]
        );
        for row in &rows {
            assert_eq!(row.target, Some(Selection::Node(id("package:a"))));
        }
    }

    #[test]
    fn a_list_longer_than_the_cap_names_how_many_it_holds_back() {
        let rows: Vec<usize> = (0..ROW_LIMIT + 4).collect();
        let (shown, held_back) = capped(&rows);
        assert_eq!(shown.len(), ROW_LIMIT);
        assert_eq!(held_back, 4);

        let short: Vec<usize> = (0..3).collect();
        assert_eq!(capped(&short), (&short[..], 0));
    }
}
