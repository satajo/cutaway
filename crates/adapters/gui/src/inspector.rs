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

use cutaway_architecture::{ArchitectureGraph, ElementId, ElementKind, ElementName, Relation};
use cutaway_lenses::{BoundaryView, Detail};
use cutaway_planning::ModificationKind;
use eframe::egui;

use crate::canvas::{self, EdgeStatus};
use crate::glyph;
use crate::label::{Labels, kind_name, kind_symbol};
use crate::{Modifying, Scene, Selection, Session, Standing, detail, focus};

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
    /// The glyph the row opens with, in an ink of its own. None where the
    /// row opens with nothing but its text.
    mark: Option<Mark>,
    text: String,
    /// None when the row names something this view holds no boundary for.
    /// The row still reads; it just leads nowhere.
    target: Option<Selection>,
}

impl Row {
    /// The whole row as one run of text, mark included.
    fn reading(&self) -> String {
        match self.mark {
            Some(mark) => format!("{} {}", mark.glyph, self.text),
            None => self.text.clone(),
        }
    }
}

/// A glyph that opens a row and carries a colour of its own, so the panel
/// and the canvas say the same thing about the same connection.
#[derive(Debug, Clone, Copy)]
struct Mark {
    glyph: &'static str,
    color: egui::Color32,
}

/// How loudly a list speaks.
///
/// A list that answers what the reader selected carries the panel and reads
/// as the way onward it is. A list that stands there whatever the reader
/// does, such as what falls outside every boundary, is background: dozens
/// of loud rows drown the panel they only annotate. Both stay clickable;
/// only their voice differs.
#[derive(Clone, Copy)]
enum Prominence {
    Primary,
    Quiet,
}

fn nothing_selected(ui: &mut egui::Ui, session: &mut Session) {
    ui.heading("Boundaries");
    let mut chosen = None;
    if let Ok(Scene { view, .. }) = &session.scene {
        ui.label(format!(
            "{} boundaries, {} connections.",
            view.graph.elements().count(),
            view.provenance.len()
        ));
        if !view.unscoped.is_empty() {
            ui.separator();
            ui.label("Outside every boundary:");
            ui.small(format!(
                "{} dependencies fall outside every boundary.",
                view.unscoped.len()
            ));
            chosen = list(
                ui,
                &unscoped_rows(view, &Labels::of(&session.graph)),
                Prominence::Quiet,
            )
            .or(chosen);
        }
    }
    if let Some(target) = chosen {
        go_to(session, &target);
    }
    // A package belongs to the project rather than to any boundary in the
    // picture, so the panel that stands for the picture as a whole is where
    // one is planned. It hangs under the project root exactly as an
    // inspected package does.
    let root = session.project_root();
    add_controls(ui, session, "Add a package:", root.as_ref(), None);
    ui.separator();
    help(ui);
}

fn help(ui: &mut egui::Ui) {
    ui.label(
        "Select a node or a connection to annotate it. Selecting a boundary keeps \
         everything inside it, and every dependency that crosses its border, at full \
         strength.",
    );
    ui.label(
        "The connections the selection depends on turn blue, and the ones that depend \
         on it turn violet; the panel marks them the same way. Both paint over the \
         picture while the selection stands.",
    );
    ui.label("A box grows with the number of concepts inside it.");
    ui.label(
        "A connection that ends at a boundary's border speaks about the boundary's own \
         code or the boundary as a whole; the connections into its parts end at the \
         parts. What passes between a boundary and its own contents stays inside it.",
    );
    ui.label(
        "Double-click a boundary to open it one level deeper than the rest of the \
         picture; select it to expand or collapse it from here.",
    );
    ui.label(
        "Keys 1, 2 and 3 cut the whole picture at packages, modules or items; \
         whatever you selected carries over to the new one.",
    );
    ui.label(
        "Press F to focus the picture on the selected boundary; Escape shows \
         everything again.",
    );
    ui.label(
        "Press ctrl+F or / to search every element by name, however the picture is \
         cut; enter opens the picture down to the one you choose.",
    );
    ui.label(
        "Severed connections turn red, drawn ones green; the plan saves to \
         cutaway.json in the repository. A connection with only part of its \
         concrete dependencies severed keeps its color and carries a red \
         mark by its arrowhead.",
    );
    ui.label(
        "A boundary planned for removal turns red with everything inside it, and \
         a boundary that exists only in the plan turns green. Select a boundary \
         to plan its removal, or to plan a new one inside it.",
    );
    ui.label(
        "A boundary that stays and changes turns amber: rename, split, merge or \
         rework it from its panel. A modification states intent for whoever \
         implements the plan and redraws nothing, so its note carries the rest \
         of the story.",
    );
    ui.label(
        "Scroll or pinch to zoom, drag to pan; shift+scroll pans too. Press Home, \
         click Fit, or double-click the background to bring the whole picture back.",
    );
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
    plan_controls(ui, session, id);
    modify_controls(ui, session, id);
    detail_controls(ui, session, id);
    note_editor(ui, session);
    add_controls(
        ui,
        session,
        "Add inside:",
        Some(id),
        Some(panel.element_kind),
    );
    let mut chosen = None;
    if !panel.contents.is_empty() {
        ui.separator();
        ui.label("Contains:");
        chosen = list(ui, &panel.contents, Prominence::Primary).or(chosen);
    }
    ui.separator();
    ui.label("Connections:");
    if panel.connections.is_empty() {
        ui.label("Nothing crosses this boundary.");
    } else {
        chosen = list(ui, &panel.connections, Prominence::Primary).or(chosen);
    }
    if let Some(target) = chosen {
        go_to(session, &target);
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
        EdgeStatus::PartiallySevered { severed, total } => {
            ui.colored_label(
                canvas::SEVERED,
                format!("{severed} of {total} concrete dependencies severed."),
            );
            ui.horizontal(|ui| {
                if ui
                    .button("Sever")
                    .on_hover_text("Mark the remaining dependencies for removal too.")
                    .clicked()
                {
                    session.sever(relation);
                }
                if ui
                    .button("Restore")
                    .on_hover_text("Withdraw every planned removal behind this connection.")
                    .clicked()
                {
                    session.restore(relation);
                }
            });
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
    if let Some(target) = list(ui, &panel.provenance, Prominence::Primary) {
        go_to(session, &target);
    }
}

/// What the plan does to this boundary, and the one act that changes it.
///
/// A removal takes everything inside the boundary with it, so an element
/// below one names the boundary that takes it and restores that boundary
/// instead of itself: the entry the plan carries sits at the root of the
/// subtree, and that is what a restore withdraws.
fn plan_controls(ui: &mut egui::Ui, session: &mut Session, id: &ElementId) {
    ui.separator();
    match session.standing(id) {
        Standing::Existing => {
            if ui
                .button("Plan removal")
                .on_hover_text(
                    "Mark this boundary and everything inside it for removal, \
                     and sever every dependency that crosses its border.",
                )
                .clicked()
            {
                session.plan_removal(id);
            }
        }
        Standing::Removed { root } => {
            let line = if root == *id {
                "Planned for removal.".to_owned()
            } else {
                format!(
                    "Planned for removal: goes with {}.",
                    Labels::of(&session.viewed).qualified(&root)
                )
            };
            ui.colored_label(canvas::SEVERED, line);
            if ui.button("Restore").clicked() {
                session.restore_element(&root);
            }
        }
        Standing::Added => {
            ui.colored_label(canvas::DRAWN, "Planned addition.");
            let inside = session.planned_inside(id);
            let erase = if inside == 0 {
                "Erase".to_owned()
            } else {
                format!("Erase (takes {inside} inside)")
            };
            if ui.button(erase).clicked() {
                session.erase_element(id);
            }
        }
    }
}

/// What the plan changes about a boundary that stays where it is, and the
/// acts that state it.
///
/// A modification is offered on an element the sources declare alone: an
/// element that exists only in the plan is edited as the addition it is, and
/// one on its way out is not renamed but removed. A modification already
/// planned reads back whatever the element's standing became, so a reader
/// who marks a modified boundary for removal still sees - and can withdraw -
/// what they stated before.
fn modify_controls(ui: &mut egui::Ui, session: &mut Session, id: &ElementId) {
    let subject = id.clone();
    let planned = session.plan.modification_of(&subject).map(|modification| {
        planned_modification(
            &modification.kind,
            &Labels::renaming(&session.viewed, &session.renames),
        )
    });
    if let Some(line) = planned {
        ui.separator();
        ui.colored_label(canvas::MODIFIED, line);
        if ui
            .button("Discard")
            .on_hover_text("Leave this element exactly as the sources have it.")
            .clicked()
        {
            session.discard_modification(&subject);
        }
        return;
    }
    if !matches!(session.standing(id), Standing::Existing) {
        return;
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Modify:");
        if ui
            .small_button("Rename")
            .on_hover_text("The element keeps everything but its name.")
            .clicked()
        {
            session.modifying = Modifying::Rename;
            session.modification_draft.clear();
        }
        if ui
            .small_button("Split")
            .on_hover_text("The element becomes several, named here.")
            .clicked()
        {
            session.modifying = Modifying::Split;
            session.modification_draft.clear();
        }
        if ui
            .small_button("Merge")
            .on_hover_text("The element folds into another; pick it on the canvas.")
            .clicked()
        {
            session.begin_merge(id);
        }
        if ui
            .small_button("Rework")
            .on_hover_text("The insides change and the picture does not. Say how in the note.")
            .clicked()
        {
            session.propose_modification(id, ModificationKind::Rework);
        }
    });
    match session.modifying {
        Modifying::Nothing => {}
        Modifying::Rename => modification_field(ui, session, id, "New name:", Session::plan_rename),
        Modifying::Split => modification_field(
            ui,
            session,
            id,
            "Becomes, comma separated:",
            Session::plan_split,
        ),
    }
}

/// What the plan states about one element that stays, in one line.
fn planned_modification(kind: &ModificationKind, labels: &Labels<'_>) -> String {
    match kind {
        ModificationKind::Rename { to } => format!("Planned rename to {to}."),
        ModificationKind::Split { into } => format!(
            "Planned split into {}.",
            into.names()
                .iter()
                .map(ElementName::as_str)
                .collect::<Vec<&str>>()
                .join(", ")
        ),
        ModificationKind::Merge { with } => {
            format!("Planned merge into {}.", labels.qualified(with))
        }
        ModificationKind::Rework => "Planned rework.".to_owned(),
    }
}

/// The field a modification takes its text from. The text goes through the
/// same act whether the reader finishes it with the key that finishes every
/// other field or with the button beside it.
fn modification_field(
    ui: &mut egui::Ui,
    session: &mut Session,
    id: &ElementId,
    prompt: &str,
    plan: fn(&mut Session, &ElementId),
) {
    ui.label(prompt);
    let mut asked = false;
    ui.horizontal(|ui| {
        let field = ui.text_edit_singleline(&mut session.modification_draft);
        asked = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        asked |= ui.button("Plan").clicked();
    });
    if asked {
        plan(session, id);
    }
}

/// The kinds of boundary a reader may plan inside one of this kind: the
/// project takes packages, a package takes modules, a module takes further
/// modules and the items declared in it. An item holds nothing the picture
/// draws, so nothing is planned inside one.
fn addable_kinds(parent: Option<ElementKind>) -> &'static [ElementKind] {
    match parent {
        None | Some(ElementKind::Project) => &[ElementKind::Package],
        Some(ElementKind::Package) => &[ElementKind::Module],
        Some(ElementKind::Module) => &[
            ElementKind::Module,
            ElementKind::Type,
            ElementKind::Function,
        ],
        Some(ElementKind::Function | ElementKind::Type) => &[],
    }
}

/// The field that plans a new boundary: its kind, its name, and the act that
/// puts it in the plan. `parent` is the boundary the new one sits in, and
/// None plans at the root of the project.
///
/// The name goes through [`cutaway_architecture::ElementName`] like every
/// other name in the architecture, so an unusable one is refused with the
/// reason the toolbar shows rather than half-planned.
fn add_controls(
    ui: &mut egui::Ui,
    session: &mut Session,
    heading: &str,
    parent: Option<&ElementId>,
    parent_kind: Option<ElementKind>,
) {
    let kinds = addable_kinds(parent_kind);
    let Some(&first) = kinds.first() else {
        return;
    };
    if !kinds.contains(&session.addition.kind) {
        session.addition.kind = first;
    }
    ui.separator();
    ui.label(heading);
    if kinds.len() > 1 {
        ui.horizontal(|ui| {
            for kind in kinds {
                if ui
                    .selectable_label(session.addition.kind == *kind, kind_name(*kind))
                    .clicked()
                {
                    session.addition.kind = *kind;
                }
            }
        });
    }
    let mut asked = false;
    ui.horizontal(|ui| {
        let field = ui.text_edit_singleline(&mut session.addition.name);
        // A name is finished by the key that finishes every other field, or
        // by the button beside it.
        asked = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        asked |= ui.button("Add").clicked();
    });
    if asked {
        session.add_element(parent, session.addition.kind);
    }
}

/// Opens or closes this one boundary on top of the detail the rest of the
/// picture follows, so the project stays whole while the boundary under
/// study shows its parts. Double-clicking the boundary expands it too.
///
/// Scoping the picture to the boundary stands beside them: it is the other
/// way of reading one boundary closely, and the only way of reading a
/// partner standing closed at the border of a scope.
fn detail_controls(ui: &mut egui::Ui, session: &mut Session, id: &ElementId) {
    let (Some(within), Ok(Scene { view, .. })) = (session.detail_within(id), &session.scene) else {
        return;
    };
    let stub = view.stubs.contains(id);
    let line = if stub {
        "Stands whole at the border of the focus.".to_owned()
    } else {
        inside(&view.graph, &session.graph, id, within)
    };
    ui.separator();
    ui.label(line);
    ui.horizontal(|ui| {
        let expand = ui.add_enabled(
            !stub && within.deeper().is_some(),
            egui::Button::new("Expand"),
        );
        // A partner is closed because the picture is about the scope, not
        // because it holds nothing; the button says which of the two it is.
        let expand = if stub {
            expand.on_disabled_hover_text("Focus this boundary to look inside it.")
        } else {
            expand
        };
        if expand.clicked() {
            session.expand(id);
        }
        if ui
            .add_enabled(
                !stub && within.shallower().is_some(),
                egui::Button::new("Collapse"),
            )
            .clicked()
        {
            session.collapse(id);
        }
        if ui
            .button("Focus (F)")
            .on_hover_text(
                "Show this boundary and its dependency partners alone. \
                 Esc shows everything again.",
            )
            .clicked()
        {
            session.focus(id);
        }
    });
}

/// What a boundary shows inside itself, in one line.
///
/// The detail governing a boundary answers what its contents would stand at,
/// not whether the picture draws them: a package cut at packages "shows
/// packages" and draws nothing within, which tells the reader nothing.
/// Naming the detail is therefore reserved for a boundary the picture opens.
/// A closed one says it is closed, and one holding nothing anywhere says
/// that instead, because there is nothing for Expand to reach.
fn inside(
    view: &ArchitectureGraph,
    graph: &ArchitectureGraph,
    id: &ElementId,
    within: Detail,
) -> String {
    if !focus::contents_of(view, id).is_empty() {
        return format!("Shows: {}", detail::name(within).to_lowercase());
    }
    if focus::contents_of(graph, id).is_empty() {
        "Nothing inside.".to_owned()
    } else {
        "Contents hidden. Expand to open.".to_owned()
    }
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
fn go_to(session: &mut Session, target: &Selection) {
    session.select(Some(target.clone()));
    session.reveal(target);
}

/// Paints one list of rows and answers with the selection a click asked
/// for. Rows past the cap collapse into a line that counts them.
fn list(ui: &mut egui::Ui, rows: &[Row], prominence: Prominence) -> Option<Selection> {
    let (shown, held_back) = capped(rows);
    let mut chosen = None;
    for row in shown {
        match &row.target {
            Some(target) => {
                if link(ui, row, prominence).clicked() {
                    chosen = Some(target.clone());
                }
            }
            None => {
                ui.small(row.reading());
            }
        }
    }
    if held_back > 0 {
        ui.small(format!("{} and {held_back} more", glyph::ELLIPSIS));
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
///
/// A quiet row reads in the colour of an aside until the pointer reaches it,
/// and answers it with the link colour and an underline: the row must sit in
/// the background and still promise it leads somewhere.
///
/// A row's mark keeps its own ink through all of that: it says which way a
/// connection runs, and the answer does not change with the pointer.
fn link(ui: &mut egui::Ui, row: &Row, prominence: Prominence) -> egui::Response {
    let font = match prominence {
        Prominence::Primary => egui::TextStyle::Body,
        Prominence::Quiet => egui::TextStyle::Small,
    }
    .resolve(ui.style());
    let mut written = egui::text::LayoutJob::default();
    if let Some(mark) = row.mark {
        run(&mut written, mark.glyph, &font, mark.color);
        run(&mut written, " ", &font, egui::Color32::PLACEHOLDER);
    }
    run(&mut written, &row.text, &font, egui::Color32::PLACEHOLDER);
    // The row decides its colour from its own hover, so it is laid out and
    // sensed first and painted after, rather than added as a whole.
    let (position, galley, response) = egui::Label::new(written)
        .truncate()
        .sense(egui::Sense::click())
        .layout_in_ui(ui);
    let linked = ui.visuals().hyperlink_color;
    let (color, underline) = match prominence {
        Prominence::Primary => (linked, egui::Stroke::NONE),
        Prominence::Quiet if response.hovered() => (
            linked,
            egui::Stroke::new(ui.style().interact(&response).fg_stroke.width, linked),
        ),
        Prominence::Quiet => (ui.visuals().weak_text_color(), egui::Stroke::NONE),
    };
    let elided = galley.elided;
    ui.painter()
        .add(egui::epaint::TextShape::new(position, galley, color).with_underline(underline));
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if elided {
        response.on_hover_text(row.reading())
    } else {
        response
    }
}

/// Adds one run of a row's text. [`egui::Color32::PLACEHOLDER`] leaves the
/// colour to the paint, which is where a row learns whether the pointer
/// rests on it.
fn run(written: &mut egui::text::LayoutJob, text: &str, font: &egui::FontId, color: egui::Color32) {
    written.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: font.clone(),
            color,
            ..Default::default()
        },
    );
}

/// What the panel says about a selected boundary.
struct NodePanel {
    heading: String,
    kind: &'static str,
    /// What the boundary is, for the panel to decide what may be planned
    /// inside it.
    element_kind: ElementKind,
    contents: Vec<Row>,
    connections: Vec<Row>,
}

fn node_panel(session: &Session, id: &ElementId) -> Option<NodePanel> {
    let Ok(Scene { view, .. }) = &session.scene else {
        return None;
    };
    // The heading is where a reader reads what a boundary is called, so a
    // renamed one says what it becomes right there.
    let labels = Labels::renaming(&view.graph, &session.renames);
    let element = session.element_of(id)?;
    Some(NodePanel {
        heading: format!("{} {}", kind_symbol(element.kind), labels.qualified(id)),
        kind: kind_name(element.kind),
        element_kind: element.kind,
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
            "{} {} {}",
            labels.qualified(&relation.from),
            glyph::OUTWARD,
            labels.qualified(&relation.to)
        ),
        provenance: provenance_rows(
            view,
            &session.graph,
            &Labels::of(&session.graph),
            relation,
            &session.plan,
        ),
    })
}

/// The boundaries a frame directly holds.
fn contents_rows(view: &ArchitectureGraph, labels: &Labels<'_>, id: &ElementId) -> Vec<Row> {
    focus::contents_of(view, id)
        .into_iter()
        .map(|child| Row {
            mark: None,
            text: labels.label(child).text(),
            target: Some(Selection::Node(child.clone())),
        })
        .collect()
}

/// What a boundary connects to: the drawn edges that touch it - the edges
/// on the frame's own border included - and the edges that cross the border
/// of anything it holds. Outgoing first, heaviest first.
///
/// Each row opens with the mark the canvas paints that connection in while
/// the boundary stands selected, so a row and the line it names are read as
/// one answer.
fn connection_rows(view: &BoundaryView, labels: &Labels<'_>, id: &ElementId) -> Vec<Row> {
    let crossing = crossings(view, &focus::subtree_of(&view.graph, id));
    let names = labels.distinct(
        crossing
            .outward
            .iter()
            .chain(&crossing.inward)
            .map(|crossing| crossing.partner),
    );
    let row = |mark: Mark, crossing: &Crossing<'_>| Row {
        mark: Some(mark),
        text: format!("{} ({})", names.name(crossing.partner), crossing.concrete),
        target: Some(Selection::Edge(crossing.edge.clone())),
    };
    let depends_on = Mark {
        glyph: glyph::OUTWARD,
        color: canvas::DEPENDENCY,
    };
    let depended_on = Mark {
        glyph: glyph::INWARD,
        color: canvas::DEPENDENT,
    };
    crossing
        .outward
        .iter()
        .map(|crossing| row(depends_on, crossing))
        .chain(
            crossing
                .inward
                .iter()
                .map(|crossing| row(depended_on, crossing)),
        )
        .collect()
}

/// The dependencies with an endpoint no boundary holds. They lead nowhere:
/// the view has no boundary to select for them.
fn unscoped_rows(view: &BoundaryView, concrete: &Labels<'_>) -> Vec<Row> {
    let names = concrete.distinct(
        view.unscoped
            .iter()
            .flat_map(|relation| [&relation.from, &relation.to]),
    );
    view.unscoped
        .iter()
        .map(|relation| Row {
            mark: None,
            text: format!(
                "{} {} {}",
                names.name(&relation.from),
                glyph::OUTWARD,
                names.name(&relation.to)
            ),
            target: None,
        })
        .collect()
}

/// The concrete dependencies behind one rolled-up edge, each leading to the
/// boundary its source shows up as at this detail. A dependency the plan
/// removes says so, which is how a partly severed connection lists which of
/// its dependencies are going.
///
/// A row names elements below the detail the view cuts at, so it reads them
/// through the labels of the full graph: only that graph holds them.
fn provenance_rows(
    view: &BoundaryView,
    graph: &ArchitectureGraph,
    concrete: &Labels<'_>,
    relation: &Relation,
    plan: &cutaway_planning::Plan,
) -> Vec<Row> {
    let behind = || view.provenance.get(relation).into_iter().flatten();
    let names = concrete.distinct(behind().flat_map(|edge| [&edge.from, &edge.to]));
    behind()
        .map(|edge| Row {
            mark: None,
            text: format!(
                "{}{} {} {}",
                if plan.plans_removal_of(edge) {
                    "(severed) "
                } else {
                    ""
                },
                names.name(&edge.from),
                glyph::OUTWARD,
                names.name(&edge.to)
            ),
            target: focus::boundary_in_view(&view.graph, graph, &edge.from).map(Selection::Node),
        })
        .collect()
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
        add_named(graph, id_text, id_text, kind);
    }

    fn add_named(graph: &mut ArchitectureGraph, id_text: &str, name: &str, kind: ElementKind) {
        graph
            .add_element(Element {
                id: id(id_text),
                name: ElementName::new(name).unwrap(),
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

    /// A row as the reader reads it: its mark and its text, in one run.
    fn texts(rows: &[Row]) -> Vec<String> {
        rows.iter().map(Row::reading).collect()
    }

    #[test]
    fn a_boundary_lists_what_it_depends_on_before_what_depends_on_it() {
        let view = boundary_view(&graph(), &Cut::uniform(Detail::Modules)).unwrap();
        assert_eq!(
            texts(&rows_of(&view, "package:a")),
            [
                format!("{} b/one (3)", glyph::OUTWARD),
                format!("{} c/one (1)", glyph::OUTWARD),
                format!("{} b/one (1)", glyph::INWARD)
            ]
        );
    }

    #[test]
    fn a_frames_own_connections_list_beside_the_ones_of_its_parts() {
        let mut graph = graph();
        relate(
            &mut graph,
            "package:a",
            "package:c",
            RelationKind::DependsOn,
        );
        let view = boundary_view(&graph, &Cut::uniform(Detail::Modules)).unwrap();
        let rows = rows_of(&view, "package:a");
        assert_eq!(
            texts(&rows),
            [
                format!("{} b/one (3)", glyph::OUTWARD),
                format!("{} c/one (1)", glyph::OUTWARD),
                format!("{} package:c (1)", glyph::OUTWARD),
                format!("{} b/one (1)", glyph::INWARD)
            ],
            "the edge attached to the frame itself is one row among the others"
        );
        assert_eq!(
            rows[2].target,
            Some(Selection::Edge(depends("package:a", "package:c")))
        );
    }

    #[test]
    fn a_connection_row_is_marked_in_the_color_the_canvas_paints_it_in() {
        let view = boundary_view(&graph(), &Cut::uniform(Detail::Modules)).unwrap();
        let marks: Vec<(&str, egui::Color32)> = rows_of(&view, "package:a")
            .iter()
            .map(|row| {
                let mark = row.mark.expect("a connection row carries a direction");
                (mark.glyph, mark.color)
            })
            .collect();
        assert_eq!(
            marks,
            [
                (glyph::OUTWARD, canvas::DEPENDENCY),
                (glyph::OUTWARD, canvas::DEPENDENCY),
                (glyph::INWARD, canvas::DEPENDENT)
            ],
            "a row says which way its connection runs in the colour and the glyph both"
        );
    }

    #[test]
    fn a_connection_row_counts_every_concrete_dependency_behind_it() {
        let view = boundary_view(&graph(), &Cut::uniform(Detail::Modules)).unwrap();
        let rows = rows_of(&view, "a/two");
        assert_eq!(
            texts(&rows),
            [format!("{} b/one (2)", glyph::OUTWARD)],
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
        assert_eq!(
            texts(&rows),
            [
                format!("{} b/one (1)", glyph::OUTWARD),
                format!("{} c/one (1)", glyph::OUTWARD),
                format!("{} b/one (1)", glyph::INWARD)
            ]
        );
        assert_eq!(
            rows[2].target,
            Some(Selection::Edge(depends("b/one", "a/one")))
        );
    }

    #[test]
    fn two_partners_of_one_name_carry_the_package_that_holds_them() {
        let mut graph = ArchitectureGraph::new();
        for package in ["app", "engine", "store"] {
            let package_id = format!("package:{package}");
            let root = format!("{package}/lib.rs");
            add_named(&mut graph, &package_id, package, ElementKind::Package);
            add_named(&mut graph, &root, "crate", ElementKind::Module);
            relate(&mut graph, &package_id, &root, RelationKind::Contains);
        }
        for partner in ["engine/lib.rs", "store/lib.rs"] {
            relate(&mut graph, "app/lib.rs", partner, RelationKind::DependsOn);
        }
        let view = boundary_view(&graph, &Cut::uniform(Detail::Modules)).unwrap();
        assert_eq!(
            texts(&rows_of(&view, "app/lib.rs")),
            [
                format!(
                    "{} {} (1)",
                    glyph::OUTWARD,
                    ["engine", "crate"].join(glyph::CONTAINER_STEP)
                ),
                format!(
                    "{} {} (1)",
                    glyph::OUTWARD,
                    ["store", "crate"].join(glyph::CONTAINER_STEP)
                )
            ]
        );
    }

    #[test]
    fn a_frame_lists_the_boundaries_it_directly_holds() {
        let view = boundary_view(&graph(), &Cut::uniform(Detail::Modules)).unwrap();
        let labels = Labels::of(&view.graph);
        let rows = contents_rows(&view.graph, &labels, &id("package:a"));
        assert_eq!(
            texts(&rows),
            [
                format!("{} a/one", glyph::MODULE),
                format!("{} a/two", glyph::MODULE)
            ]
        );
        assert_eq!(rows[0].target, Some(Selection::Node(id("a/one"))));
    }

    #[test]
    fn a_dependency_outside_every_boundary_leads_nowhere() {
        let mut graph = graph();
        add(&mut graph, "project", ElementKind::Project);
        add(&mut graph, "stray", ElementKind::Module);
        relate(&mut graph, "project", "stray", RelationKind::Contains);
        relate(&mut graph, "stray", "b/one", RelationKind::DependsOn);
        let view = boundary_view(&graph, &Cut::uniform(Detail::Packages)).unwrap();
        let rows = unscoped_rows(&view, &Labels::of(&graph));
        assert_eq!(texts(&rows), [format!("stray {} b/one", glyph::OUTWARD)]);
        assert_eq!(rows[0].target, None);
    }

    #[test]
    fn a_concrete_dependency_leads_to_the_boundary_its_source_shows_up_as() {
        let graph = graph();
        let view = boundary_view(&graph, &Cut::uniform(Detail::Packages)).unwrap();
        let rows = provenance_rows(
            &view,
            &graph,
            &Labels::of(&graph),
            &depends("package:a", "package:b"),
            &cutaway_planning::Plan::new(),
        );
        assert_eq!(
            texts(&rows),
            [
                format!("a/one {} b/one", glyph::OUTWARD),
                format!("a/two {} b/one", glyph::OUTWARD),
                format!("a/two#type:X {} b/one", glyph::OUTWARD)
            ]
        );
        for row in &rows {
            assert_eq!(row.target, Some(Selection::Node(id("package:a"))));
        }
    }

    #[test]
    fn a_partly_severed_connection_lists_which_dependencies_are_going() {
        let graph = graph();
        let view = boundary_view(&graph, &Cut::uniform(Detail::Packages)).unwrap();
        let mut plan = cutaway_planning::Plan::new();
        plan.propose(cutaway_planning::ProposedChange::RemoveRelation(depends(
            "a/one", "b/one",
        )))
        .unwrap();
        let rows = provenance_rows(
            &view,
            &graph,
            &Labels::of(&graph),
            &depends("package:a", "package:b"),
            &plan,
        );
        assert_eq!(
            texts(&rows),
            [
                format!("(severed) a/one {} b/one", glyph::OUTWARD),
                format!("a/two {} b/one", glyph::OUTWARD),
                format!("a/two#type:X {} b/one", glyph::OUTWARD)
            ]
        );
    }

    #[test]
    fn an_opened_boundary_names_the_detail_its_contents_stand_at() {
        let graph = graph();
        let view = boundary_view(&graph, &Cut::uniform(Detail::Modules)).unwrap();
        assert_eq!(
            inside(&view.graph, &graph, &id("package:a"), Detail::Modules),
            "Shows: modules"
        );
    }

    #[test]
    fn a_boundary_drawing_nothing_of_what_it_holds_says_it_is_closed() {
        let graph = graph();
        let view = boundary_view(&graph, &Cut::uniform(Detail::Packages)).unwrap();
        assert_eq!(
            inside(&view.graph, &graph, &id("package:a"), Detail::Packages),
            "Contents hidden. Expand to open.",
            "a package cut at packages holds modules the picture does not draw"
        );
    }

    #[test]
    fn a_boundary_holding_nothing_anywhere_says_so() {
        let graph = graph();
        let view = boundary_view(&graph, &Cut::uniform(Detail::Items)).unwrap();
        assert_eq!(
            inside(&view.graph, &graph, &id("a/two#type:X"), Detail::Items),
            "Nothing inside."
        );
    }

    #[test]
    fn a_planned_split_names_every_element_it_becomes() {
        let graph = graph();
        assert_eq!(
            planned_modification(
                &ModificationKind::Split {
                    into: cutaway_planning::SplitParts::new(vec![
                        ElementName::new("engine").unwrap(),
                        ElementName::new("transport").unwrap(),
                    ])
                    .unwrap(),
                },
                &Labels::of(&graph)
            ),
            "Planned split into engine, transport."
        );
    }

    #[test]
    fn a_planned_merge_names_the_boundary_it_folds_into() {
        let graph = graph();
        assert_eq!(
            planned_modification(
                &ModificationKind::Merge { with: id("b/one") },
                &Labels::of(&graph)
            ),
            "Planned merge into b/one."
        );
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
