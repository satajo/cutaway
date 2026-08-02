//! GUI adapter: the eframe/egui desktop shell of Cutaway.
//!
//! The GUI drives the application core and knows nothing about where
//! architectures or plans come from: the composition root hands it a
//! [`ProjectOpener`] and every opened project carries its own
//! [`PlanStore`]. The boundary canvas shows the architecture at an
//! adjustable level of detail; the user severs, draws, and annotates
//! connections, and every markup lands in the project's plan immediately.

mod canvas;
mod layout;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use cutaway_architecture::{
    ArchitectureGraph, Element, ElementId, ElementKind, Relation, RelationKind,
};
use cutaway_lenses::{BoundaryView, Detail, boundary_view};
use cutaway_planning::ports::plan_store::PlanStore;
use cutaway_planning::{Note, Plan, ProposedChange, Subject};
use eframe::egui::{self, emath::TSTransform};

use crate::canvas::{CanvasAction, Content, EdgeStatus, EdgeVisual};

/// Everything the GUI needs about one opened project. The composition root
/// builds this from the real adapters.
pub struct OpenedProject {
    pub graph: ArchitectureGraph,
    pub plan: Plan,
    pub store: Box<dyn PlanStore>,
}

/// Opens the project at a path. Failures arrive as human-readable text: the
/// GUI displays them, it never reacts to individual causes.
pub type ProjectOpener = Box<dyn Fn(&Path) -> Result<OpenedProject, String>>;

pub fn run(opener: ProjectOpener) -> Result<(), StartupError> {
    eframe::run_native(
        "Cutaway",
        eframe::NativeOptions::default(),
        Box::new(move |_context| Ok(Box::new(CutawayApp::new(opener)))),
    )
    .map_err(|source| StartupError::Gui {
        reason: source.to_string(),
    })
}

#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    #[error("cannot start the GUI: {reason}")]
    Gui { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    Node(ElementId),
    Edge(Relation),
}

struct Session {
    graph: ArchitectureGraph,
    plan: Plan,
    store: Box<dyn PlanStore>,
    detail: Detail,
    view: Result<BoundaryView, String>,
    /// Contained-concept counts from the full graph; they size the boxes.
    weights: BTreeMap<ElementId, usize>,
    /// World-to-screen camera; None until a frame fits the graph into view.
    camera: Option<TSTransform>,
    selection: Option<Selection>,
    note_draft: String,
    drawing: bool,
    draw_source: Option<ElementId>,
    status: Option<String>,
}

impl Session {
    fn open(project: OpenedProject) -> Self {
        let weights = layout::concept_weights(&project.graph);
        let mut session = Self {
            graph: project.graph,
            plan: project.plan,
            store: project.store,
            detail: Detail::Packages,
            view: Err("not built yet".to_owned()),
            weights,
            camera: None,
            selection: None,
            note_draft: String::new(),
            drawing: false,
            draw_source: None,
            status: None,
        };
        session.rebuild_view();
        session
    }

    /// The element behind an id. The view graph answers first: it holds the
    /// synthetic `self` leaves the full graph never sees.
    fn element_of(&self, id: &ElementId) -> Option<&Element> {
        self.view
            .as_ref()
            .ok()
            .and_then(|view| view.graph.element(id))
            .or_else(|| self.graph.element(id))
    }

    fn rebuild_view(&mut self) {
        self.view = boundary_view(&self.graph, self.detail).map_err(|error| error.to_string());
        self.selection = None;
        self.note_draft.clear();
        self.draw_source = None;
        self.camera = None;
    }

    fn edges(&self) -> Vec<EdgeVisual> {
        let Ok(view) = &self.view else {
            return Vec::new();
        };
        let mut edges = Vec::new();
        for relation in view.graph.relations() {
            if relation.kind != RelationKind::DependsOn {
                continue;
            }
            let severed = self.plan.plans_removal_of(relation);
            edges.push(EdgeVisual {
                relation: relation.clone(),
                status: if severed {
                    EdgeStatus::Severed
                } else {
                    EdgeStatus::Existing
                },
                annotated: self.note_for(relation, severed).is_some(),
            });
        }
        for planned in self.plan.changes() {
            if let ProposedChange::AddRelation(relation) = &planned.change {
                let visible = relation.kind == RelationKind::DependsOn
                    && view.graph.element(&relation.from).is_some()
                    && view.graph.element(&relation.to).is_some()
                    && !view.graph.relations().any(|r| r == relation);
                if visible {
                    edges.push(EdgeVisual {
                        relation: relation.clone(),
                        status: EdgeStatus::Drawn,
                        annotated: planned.note.is_some(),
                    });
                }
            }
        }
        edges
    }

    fn status_of(&self, relation: &Relation) -> EdgeStatus {
        if self.plan.plans_removal_of(relation) {
            EdgeStatus::Severed
        } else if self.plan.plans_addition_of(relation) {
            EdgeStatus::Drawn
        } else {
            EdgeStatus::Existing
        }
    }

    fn note_for(&self, relation: &Relation, severed: bool) -> Option<&Note> {
        if severed {
            self.plan
                .note_of(&ProposedChange::RemoveRelation(relation.clone()))
        } else {
            self.plan
                .annotation_of(&Subject::Relation(relation.clone()))
        }
    }

    fn current_note_text(&self, selection: &Selection) -> String {
        let note = match selection {
            Selection::Node(id) => self.plan.annotation_of(&Subject::Element(id.clone())),
            Selection::Edge(relation) => match self.status_of(relation) {
                EdgeStatus::Existing => self
                    .plan
                    .annotation_of(&Subject::Relation(relation.clone())),
                EdgeStatus::Severed => self
                    .plan
                    .note_of(&ProposedChange::RemoveRelation(relation.clone())),
                EdgeStatus::Drawn => self
                    .plan
                    .note_of(&ProposedChange::AddRelation(relation.clone())),
            },
        };
        note.map(|note| note.as_str().to_owned())
            .unwrap_or_default()
    }

    fn select(&mut self, selection: Option<Selection>) {
        self.note_draft = selection
            .as_ref()
            .map(|s| self.current_note_text(s))
            .unwrap_or_default();
        self.selection = selection;
    }

    fn save_plan(&mut self) {
        self.status = self
            .store
            .save(&self.plan)
            .err()
            .map(|error| error.to_string());
    }

    fn save_note(&mut self) {
        let Some(selection) = self.selection.clone() else {
            return;
        };
        // An emptied note field clears the note.
        let note = Note::new(self.note_draft.clone()).ok();
        let result = match &selection {
            Selection::Node(id) => {
                let subject = Subject::Element(id.clone());
                match &note {
                    Some(note) => self.plan.annotate(subject, note.clone()),
                    None => self.plan.clear_annotation(&subject),
                }
                Ok(())
            }
            Selection::Edge(relation) => match self.status_of(relation) {
                EdgeStatus::Existing => {
                    let subject = Subject::Relation(relation.clone());
                    match &note {
                        Some(note) => self.plan.annotate(subject, note.clone()),
                        None => self.plan.clear_annotation(&subject),
                    }
                    Ok(())
                }
                EdgeStatus::Severed => self
                    .plan
                    .explain(&ProposedChange::RemoveRelation(relation.clone()), note),
                EdgeStatus::Drawn => self
                    .plan
                    .explain(&ProposedChange::AddRelation(relation.clone()), note),
            },
        };
        match result {
            Ok(()) => self.save_plan(),
            Err(error) => self.status = Some(error.to_string()),
        }
    }

    fn sever(&mut self, relation: &Relation) {
        // A drawn edge is erased, an existing one is marked for removal.
        let result = if self.plan.plans_addition_of(relation) {
            self.plan
                .retract(&ProposedChange::AddRelation(relation.clone()))
        } else {
            self.plan
                .propose(ProposedChange::RemoveRelation(relation.clone()))
        };
        match result {
            Ok(()) => self.save_plan(),
            Err(error) => self.status = Some(error.to_string()),
        }
        if self.plan.plans_addition_of(relation) || !edge_exists(&self.view, relation) {
            self.select(None);
        }
    }

    fn restore(&mut self, relation: &Relation) {
        match self
            .plan
            .retract(&ProposedChange::RemoveRelation(relation.clone()))
        {
            Ok(()) => self.save_plan(),
            Err(error) => self.status = Some(error.to_string()),
        }
    }

    fn draw_edge(&mut self, from: ElementId, to: ElementId) {
        let relation = Relation {
            from,
            to,
            kind: RelationKind::DependsOn,
        };
        if edge_exists(&self.view, &relation) {
            self.status = Some("that dependency already exists".to_owned());
            return;
        }
        match self
            .plan
            .propose(ProposedChange::AddRelation(relation.clone()))
        {
            Ok(()) => {
                self.save_plan();
                self.select(Some(Selection::Edge(relation)));
            }
            Err(error) => self.status = Some(error.to_string()),
        }
    }

    fn handle(&mut self, action: CanvasAction) {
        self.status = None;
        match action {
            CanvasAction::Node(id) => {
                if self.drawing {
                    match self.draw_source.take() {
                        None => self.draw_source = Some(id),
                        Some(source) if source == id => {}
                        Some(source) => {
                            self.drawing = false;
                            self.draw_edge(source, id);
                        }
                    }
                } else {
                    self.select(Some(Selection::Node(id)));
                }
            }
            CanvasAction::Edge(relation) => {
                self.select(Some(Selection::Edge(relation)));
            }
            CanvasAction::Background => {
                self.draw_source = None;
                self.select(None);
            }
        }
    }
}

fn edge_exists(view: &Result<BoundaryView, String>, relation: &Relation) -> bool {
    view.as_ref()
        .is_ok_and(|view| view.graph.relations().any(|r| r == relation))
}

struct CutawayApp {
    opener: ProjectOpener,
    repository: Option<PathBuf>,
    /// Delivers the folder chosen in the system picker; Some while a picker
    /// dialog is open.
    picker: Option<mpsc::Receiver<Option<PathBuf>>>,
    session: Option<Result<Session, String>>,
}

impl CutawayApp {
    fn new(opener: ProjectOpener) -> Self {
        Self {
            opener,
            repository: None,
            picker: None,
            session: None,
        }
    }

    /// Opens the system folder picker on a helper thread: the dialog blocks
    /// until the user chooses, and the GUI thread must keep painting.
    fn pick_repository(&mut self, context: egui::Context) {
        let (sender, receiver) = mpsc::channel();
        let start_in = self.repository.clone();
        std::thread::spawn(move || {
            let mut dialog = rfd::AsyncFileDialog::new().set_title("Open a git repository");
            if let Some(directory) = start_in {
                dialog = dialog.set_directory(directory);
            }
            let choice = pollster::block_on(dialog.pick_folder());
            let _ = sender.send(choice.map(|folder| folder.path().to_path_buf()));
            context.request_repaint();
        });
        self.picker = Some(receiver);
    }

    fn receive_picked_repository(&mut self) {
        let Some(receiver) = self.picker.take() else {
            return;
        };
        match receiver.try_recv() {
            Ok(Some(path)) => {
                self.session = Some((self.opener)(&path).map(Session::open));
                self.repository = Some(path);
            }
            Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {}
            Err(mpsc::TryRecvError::Empty) => self.picker = Some(receiver),
        }
    }
}

impl eframe::App for CutawayApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.receive_picked_repository();
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                let picking = self.picker.is_some();
                if ui
                    .add_enabled(!picking, egui::Button::new("Open repository…"))
                    .clicked()
                {
                    self.pick_repository(ui.ctx().clone());
                }
                if let Some(repository) = &self.repository {
                    ui.label(repository.display().to_string());
                }
                if let Some(Ok(session)) = &mut self.session {
                    ui.separator();
                    ui.label("Detail");
                    let mut position = Detail::ALL
                        .iter()
                        .position(|detail| *detail == session.detail)
                        .unwrap_or(0);
                    let slider = egui::Slider::new(&mut position, 0..=Detail::ALL.len() - 1)
                        .show_value(false)
                        .step_by(1.0);
                    if ui.add(slider).changed() {
                        session.detail = Detail::ALL[position];
                        session.rebuild_view();
                    }
                    ui.label(detail_label(session.detail));
                    ui.separator();
                    let label = if session.drawing {
                        match &session.draw_source {
                            None => "Drawing: pick the dependent",
                            Some(_) => "Drawing: pick the dependency",
                        }
                    } else {
                        "Draw dependency"
                    };
                    if ui.selectable_label(session.drawing, label).clicked() {
                        session.drawing = !session.drawing;
                        session.draw_source = None;
                    }
                    if let Some(status) = &session.status {
                        ui.separator();
                        ui.colored_label(ui.visuals().warn_fg_color, status);
                    }
                }
            });
        });

        egui::Panel::right("inspector")
            .default_size(280.0)
            .show(ui, |ui| match &mut self.session {
                None => {
                    ui.heading("Cutaway");
                    ui.label(
                        "Open a git repository to see its architecture as \
                         boundaries and the dependencies that cross them.",
                    );
                }
                Some(Err(reason)) => {
                    ui.colored_label(ui.visuals().error_fg_color, reason.as_str());
                }
                Some(Ok(session)) => inspector(ui, session),
            });

        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(Ok(session)) = &mut self.session {
                match &session.view {
                    Err(reason) => {
                        ui.colored_label(ui.visuals().error_fg_color, reason.as_str());
                    }
                    Ok(view) => {
                        let computed = layout::compute(&view.graph, &session.weights);
                        let edges = session.edges();
                        let (selected_edge, selected_node) = match &session.selection {
                            Some(Selection::Edge(relation)) => (Some(relation), None),
                            Some(Selection::Node(id)) => (None, Some(id)),
                            None => (None, None),
                        };
                        let action = canvas::show(
                            ui,
                            &Content {
                                view: &view.graph,
                                layout: &computed,
                                edges: &edges,
                                selected_edge,
                                selected_node,
                                draw_source: session.draw_source.as_ref(),
                            },
                            &mut session.camera,
                        );
                        if let Some(action) = action {
                            session.handle(action);
                        }
                    }
                }
            }
        });
    }
}

fn inspector(ui: &mut egui::Ui, session: &mut Session) {
    match session.selection.clone() {
        None => {
            ui.heading("Boundaries");
            if let Ok(view) = &session.view {
                ui.label(format!(
                    "{} boundaries, {} connections.",
                    view.graph.elements().count(),
                    view.provenance.len()
                ));
                if !view.coarse.is_empty() {
                    ui.label(format!(
                        "{} connections name a boundary with visible children as a \
                         whole; they show at a coarser detail.",
                        view.coarse.len()
                    ));
                }
                if !view.unscoped.is_empty() {
                    ui.label(format!(
                        "{} dependencies fall outside every boundary.",
                        view.unscoped.len()
                    ));
                }
            }
            ui.separator();
            ui.label("Select a node or a connection to annotate it. Selecting a node fades everything it does not touch.");
            ui.label("Severed connections turn red, drawn ones green; the plan saves to cutaway.json in the repository.");
            ui.label("Drag or scroll to pan, ctrl+scroll or pinch to zoom, double-click the background to refit.");
        }
        Some(Selection::Node(id)) => {
            let (name, kind) = session.element_of(&id).map_or_else(
                || (id.to_string(), String::new()),
                |element| {
                    (
                        element.name.to_string(),
                        format!("{} ", kind_symbol(element.kind)),
                    )
                },
            );
            ui.heading(format!("{kind}{name}"));
            note_editor(ui, session);
        }
        Some(Selection::Edge(relation)) => {
            ui.heading(format!(
                "{} → {}",
                element_name(session, &relation.from),
                element_name(session, &relation.to)
            ));
            match session.status_of(&relation) {
                EdgeStatus::Existing => {
                    ui.label("Existing dependency.");
                    if ui.button("Sever").clicked() {
                        session.sever(&relation);
                    }
                }
                EdgeStatus::Severed => {
                    ui.colored_label(canvas::SEVERED, "Planned for removal.");
                    if ui.button("Restore").clicked() {
                        session.restore(&relation);
                    }
                }
                EdgeStatus::Drawn => {
                    ui.colored_label(canvas::DRAWN, "Planned addition.");
                    if ui.button("Erase").clicked() {
                        session.sever(&relation);
                    }
                }
            }
            note_editor(ui, session);
            if let Ok(view) = &session.view
                && let Some(underlying) = view.provenance.get(&relation)
            {
                ui.separator();
                ui.label(format!(
                    "Stands for {} concrete dependencies:",
                    underlying.len()
                ));
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for concrete in underlying {
                        ui.small(format!(
                            "{} → {}",
                            element_name(session, &concrete.from),
                            element_name(session, &concrete.to)
                        ));
                    }
                });
            }
        }
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

fn element_name(session: &Session, id: &ElementId) -> String {
    session
        .element_of(id)
        .map_or_else(|| id.to_string(), |element| element.name.to_string())
}

fn detail_label(detail: Detail) -> &'static str {
    match detail {
        Detail::Packages => "Packages",
        Detail::Modules => "Modules",
        Detail::Items => "Items",
    }
}

fn kind_symbol(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::Project => "◈",
        ElementKind::Package => "▣",
        ElementKind::Module => "▤",
        ElementKind::Function => "ƒ",
        ElementKind::Type => "T",
    }
}
