//! GUI adapter: the eframe/egui desktop shell of Cutaway.
//!
//! The GUI drives the application core and knows nothing about where
//! architectures or plans come from: the composition root hands it a
//! [`ProjectOpener`] and every opened project carries its own
//! [`PlanStore`]. The boundary canvas shows the architecture at package or
//! module level; the user severs, draws, and annotates connections, and
//! every markup lands in the project's plan immediately.

mod canvas;
mod layout;

use std::collections::BTreeSet;
use std::path::Path;

use cutaway_architecture::{ArchitectureGraph, ElementId, ElementKind, Relation, RelationKind};
use cutaway_lenses::{BoundaryView, boundary_view};
use cutaway_redlining::ports::plan_store::PlanStore;
use cutaway_redlining::{Note, Plan, ProposedChange, Subject};
use eframe::egui::{self, Rect};

use crate::canvas::{CanvasAction, EdgeStatus, EdgeVisual};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Packages,
    Modules,
}

impl Level {
    fn kinds(self) -> BTreeSet<ElementKind> {
        match self {
            Self::Packages => BTreeSet::from([ElementKind::Package]),
            Self::Modules => BTreeSet::from([ElementKind::Package, ElementKind::Module]),
        }
    }
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
    level: Level,
    view: Result<BoundaryView, String>,
    scene_rect: Rect,
    selection: Option<Selection>,
    note_draft: String,
    drawing: bool,
    draw_source: Option<ElementId>,
    status: Option<String>,
}

impl Session {
    fn open(project: OpenedProject) -> Self {
        let mut session = Self {
            graph: project.graph,
            plan: project.plan,
            store: project.store,
            level: Level::Packages,
            view: Err("not built yet".to_owned()),
            scene_rect: Rect::ZERO,
            selection: None,
            note_draft: String::new(),
            drawing: false,
            draw_source: None,
            status: None,
        };
        session.rebuild_view();
        session
    }

    fn rebuild_view(&mut self) {
        self.view =
            boundary_view(&self.graph, &self.level.kinds()).map_err(|error| error.to_string());
        self.selection = None;
        self.note_draft.clear();
        self.draw_source = None;
        self.scene_rect = Rect::ZERO;
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
    repository_path: String,
    session: Option<Result<Session, String>>,
}

impl CutawayApp {
    fn new(opener: ProjectOpener) -> Self {
        Self {
            opener,
            repository_path: String::new(),
            session: None,
        }
    }
}

impl eframe::App for CutawayApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Repository:");
                ui.text_edit_singleline(&mut self.repository_path);
                if ui.button("Open").clicked() {
                    self.session =
                        Some((self.opener)(Path::new(&self.repository_path)).map(Session::open));
                }
                if let Some(Ok(session)) = &mut self.session {
                    ui.separator();
                    let mut level = session.level;
                    ui.selectable_value(&mut level, Level::Packages, "Packages");
                    ui.selectable_value(&mut level, Level::Modules, "Modules");
                    if level != session.level {
                        session.level = level;
                        session.rebuild_view();
                    }
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
                        let computed = layout::compute(&view.graph);
                        let edges = session.edges();
                        let (selected_edge, selected_node) = match &session.selection {
                            Some(Selection::Edge(relation)) => (Some(relation), None),
                            Some(Selection::Node(id)) => (None, Some(id)),
                            None => (None, None),
                        };
                        let mut action = None;
                        let mut scene_rect = session.scene_rect;
                        egui::Scene::new()
                            .zoom_range(0.1..=4.0)
                            .show(ui, &mut scene_rect, |ui| {
                                action = canvas::show(
                                    ui,
                                    &view.graph,
                                    &computed,
                                    &edges,
                                    selected_edge,
                                    selected_node,
                                    session.draw_source.as_ref(),
                                );
                            });
                        session.scene_rect = scene_rect;
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
                if !view.unscoped.is_empty() {
                    ui.label(format!(
                        "{} dependencies fall outside every boundary.",
                        view.unscoped.len()
                    ));
                }
            }
            ui.separator();
            ui.label("Select a node or a connection to annotate it.");
            ui.label("Severed connections turn red, drawn ones green; the plan saves to .cutaway/redline.json in the repository.");
        }
        Some(Selection::Node(id)) => {
            let (name, kind) = session.graph.element(&id).map_or_else(
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
                boundary_name(&session.graph, &relation.from),
                boundary_name(&session.graph, &relation.to)
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
                            boundary_name(&session.graph, &concrete.from),
                            boundary_name(&session.graph, &concrete.to)
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

fn boundary_name(graph: &ArchitectureGraph, id: &ElementId) -> String {
    graph
        .element(id)
        .map_or_else(|| id.to_string(), |element| element.name.to_string())
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
