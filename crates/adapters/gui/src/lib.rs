//! GUI adapter: the eframe/egui desktop shell of Cutaway.
//!
//! The GUI drives the application core and knows nothing about where
//! architectures or plans come from: the composition root hands it a
//! [`ProjectOpener`] and every opened project carries its own
//! [`PlanStore`]. The boundary canvas shows the architecture at an
//! adjustable level of detail; the user severs, draws, and annotates
//! connections, and every markup lands in the project's plan immediately.
//!
//! The toolbar's three stops set the detail of the whole picture, and single
//! boundaries open or close on top of it: a double click on a boundary, or
//! the inspector's Expand and Collapse, moves that one boundary a step.
//! Choosing another stop drops those decisions, because a new whole is a new
//! question, and carries the selection across, because the subject of the
//! question stays the reader's.

mod canvas;
mod continuity;
mod detail;
mod focus;
mod inspector;
mod label;
mod layout;
mod palette;
mod routing;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use cutaway_architecture::{ArchitectureGraph, Element, ElementId, Relation, RelationKind};
use cutaway_lenses::{BoundaryView, Cut, Detail, boundary_view};
use cutaway_planning::ports::plan_store::PlanStore;
use cutaway_planning::{Note, Plan, ProposedChange, Subject};
use eframe::egui::{self, Rect, emath::TSTransform};

use crate::canvas::{CanvasAction, Content, EdgeStatus, EdgeVisual};
use crate::layout::Layout;
use crate::palette::Palette;

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

/// What a selection asks the camera to show. A connection shows at the end
/// it leaves: that is where the reader's question started.
pub(crate) fn revealed(selection: &Selection) -> &ElementId {
    match selection {
        Selection::Node(id) => id,
        Selection::Edge(relation) => &relation.from,
    }
}

/// A boundary view with the arrangement that paints it. The arrangement
/// follows from the view graph and the concept weights alone, so it is
/// computed once per rebuild instead of once per frame.
struct Scene {
    view: BoundaryView,
    layout: Layout,
}

struct Session {
    graph: ArchitectureGraph,
    plan: Plan,
    store: Box<dyn PlanStore>,
    /// Where the picture cuts the hierarchy: the detail of the whole, and
    /// the boundaries the reader opened or closed on top of it.
    cut: Cut,
    scene: Result<Scene, String>,
    /// Contained-concept counts from the full graph; they size the boxes.
    weights: BTreeMap<ElementId, usize>,
    /// World-to-screen camera; None until a frame fits the graph into view.
    camera: Option<TSTransform>,
    /// The screen rectangle the canvas painted into last. The canvas knows
    /// the viewport and the inspector does not, yet a selection made in the
    /// inspector must land inside it; the canvas therefore records the
    /// rectangle here every frame. The inspector paints before the canvas,
    /// so it reveals with the rectangle of the previous frame - the same
    /// one, unless the window resized in between.
    viewport: Rect,
    selection: Option<Selection>,
    note_draft: String,
    drawing: bool,
    draw_source: Option<ElementId>,
    status: Option<String>,
    /// Searching the whole architecture by name, whatever the picture shows.
    palette: Palette,
}

impl Session {
    fn open(project: OpenedProject) -> Self {
        let weights = layout::concept_weights(&project.graph);
        let mut session = Self {
            graph: project.graph,
            plan: project.plan,
            store: project.store,
            cut: Cut::uniform(Detail::Packages),
            scene: Err("not built yet".to_owned()),
            weights,
            camera: None,
            viewport: Rect::NOTHING,
            selection: None,
            note_draft: String::new(),
            drawing: false,
            draw_source: None,
            status: None,
            palette: Palette::default(),
        };
        session.rebuild_view();
        session
    }

    /// The element behind an id. The view graph answers first: it holds the
    /// synthetic `self` leaves the full graph never sees.
    fn element_of(&self, id: &ElementId) -> Option<&Element> {
        self.scene
            .as_ref()
            .ok()
            .and_then(|scene| scene.view.graph.element(id))
            .or_else(|| self.graph.element(id))
    }

    /// Paints the cut anew, leaving the camera where it is: opening or
    /// closing one boundary changes what the picture holds, not what the
    /// reader looks at. Whatever the new cut no longer shows is dropped.
    fn rebuild_view(&mut self) {
        self.scene = boundary_view(&self.graph, &self.cut)
            .map_err(|error| error.to_string())
            .map(|view| {
                let layout = layout::compute(&view.graph, &self.weights);
                Scene { view, layout }
            });
        if let Some(source) = &self.draw_source
            && !self.shows(source)
        {
            self.draw_source = None;
        }
        let dropped = self
            .selection
            .as_ref()
            .is_some_and(|selection| !self.shows_selection(selection));
        if dropped {
            self.select(None);
        }
    }

    /// Cuts the whole picture at one detail again, dropping every boundary
    /// the reader opened or closed: those decisions answered the detail they
    /// were made in.
    ///
    /// The subject of the reading is not dropped with them. Element ids hold
    /// across details, so whatever stood selected reappears as another box or
    /// another connection, and the camera moves to where it reappeared.
    /// Nothing selected leaves the camera where it is, unless the new picture
    /// no longer meets it at all.
    fn recut(&mut self, detail: Detail) {
        if self.cut.detail == detail {
            return;
        }
        let before = self
            .selection
            .take()
            .and_then(|selection| Some((self.scene.as_ref().ok()?.view.clone(), selection)));
        self.cut = Cut::uniform(detail);
        self.rebuild_view();
        let carried = before.and_then(|(before, selection)| {
            let after = self.scene.as_ref().ok()?;
            continuity::translated(&self.graph, &before, &after.view, &selection)
        });
        if let Some(selection) = carried {
            let shown = revealed(&selection).clone();
            self.select(Some(selection));
            self.reveal(&shown);
        } else {
            self.select(None);
            self.keep_or_refit();
        }
    }

    /// Drops the camera wherever the new picture no longer meets what the
    /// camera looks at; the canvas then fits the picture into view again.
    fn keep_or_refit(&mut self) {
        let Some(camera) = self.camera else {
            return;
        };
        let held = self.scene.as_ref().is_ok_and(|scene| {
            continuity::camera_holds(camera, self.viewport, canvas::world_bounds(&scene.layout))
        });
        if !held {
            self.camera = None;
        }
    }

    /// Opens the boundary one detail step deeper than what it shows now.
    fn expand(&mut self, id: &ElementId) {
        self.step_detail(id, Cut::expand);
    }

    /// Closes the boundary one detail step back toward a single box.
    fn collapse(&mut self, id: &ElementId) {
        self.step_detail(id, Cut::collapse);
    }

    fn step_detail(
        &mut self,
        id: &ElementId,
        step: fn(&mut Cut, &BoundaryView, &ElementId) -> bool,
    ) {
        let Ok(scene) = &self.scene else {
            return;
        };
        if step(&mut self.cut, &scene.view, id) {
            self.rebuild_view();
        }
    }

    /// The detail governing what a boundary shows inside it; None while the
    /// picture holds no such boundary.
    fn detail_within(&self, id: &ElementId) -> Option<Detail> {
        self.scene
            .as_ref()
            .ok()
            .and_then(|scene| scene.view.detail_within.get(id).copied())
    }

    fn shows(&self, id: &ElementId) -> bool {
        self.scene
            .as_ref()
            .is_ok_and(|scene| scene.view.graph.element(id).is_some())
    }

    fn shows_selection(&self, selection: &Selection) -> bool {
        match selection {
            Selection::Node(id) => self.shows(id),
            // A connection reads only where both of its ends do, planned
            // ones included.
            Selection::Edge(relation) => self.shows(&relation.from) && self.shows(&relation.to),
        }
    }

    fn edges(&self) -> Vec<EdgeVisual> {
        let Ok(scene) = &self.scene else {
            return Vec::new();
        };
        let view = &scene.view;
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
                weight: concrete_count(view, relation),
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
                        // A planned edge stands for itself alone: nothing
                        // concrete is behind it yet.
                        weight: 1,
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

    /// Moves the picture until the element is on screen, at the current
    /// magnification: a selection made beside the picture must be findable
    /// in it. An element already on screen moves nothing.
    fn reveal(&mut self, id: &ElementId) {
        let Some(camera) = self.camera else {
            return;
        };
        let Ok(scene) = &self.scene else {
            return;
        };
        let Some(rect) = scene.layout.rects.get(id) else {
            return;
        };
        let shift = canvas::reveal_shift(self.viewport, camera.mul_rect(*rect));
        if let (Some(shift), Some(camera)) = (shift, self.camera.as_mut()) {
            camera.translation += shift;
        }
    }

    /// Puts one element of the full architecture in front of the reader.
    ///
    /// A search reaches past the picture, so the element found is often
    /// finer than the detail the picture cuts at. The cut then opens down to
    /// it - one override per boundary on the way, exactly as expanding each
    /// of them by hand would - and the element itself becomes the selection
    /// the camera moves to. Where even that leaves it hidden, the nearest
    /// boundary above it answers instead, so the reader always lands
    /// somewhere the picture holds.
    fn locate(&mut self, target: &ElementId) {
        if !self.shows(target) {
            for (boundary, detail) in palette::overrides_revealing(&self.graph, target) {
                // A boundary the reader closed opens again: the search is
                // the later question, and the later question wins.
                self.cut
                    .overrides
                    .entry(boundary)
                    .and_modify(|open| *open = (*open).max(detail))
                    .or_insert(detail);
            }
            self.rebuild_view();
        }
        let Ok(scene) = &self.scene else {
            return;
        };
        let Some(found) = focus::boundary_in_view(&scene.view.graph, &self.graph, target) else {
            return;
        };
        self.select(Some(Selection::Node(found.clone())));
        self.reveal(&found);
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
        if self.plan.plans_addition_of(relation) || !edge_exists(&self.scene, relation) {
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
        if edge_exists(&self.scene, &relation) {
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
            // Opening a boundary answers the question the reader just asked
            // of it, so the boundary stays selected under the new picture.
            CanvasAction::Expand(id) => {
                if !self.drawing {
                    self.select(Some(Selection::Node(id.clone())));
                    self.expand(&id);
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

/// How many concrete dependencies one rolled-up edge stands for. An edge
/// the view rolled nothing into still stands for the one dependency it is.
fn concrete_count(view: &BoundaryView, relation: &Relation) -> usize {
    view.provenance
        .get(relation)
        .map_or(1, |concrete| concrete.len().max(1))
}

fn edge_exists(scene: &Result<Scene, String>, relation: &Relation) -> bool {
    scene
        .as_ref()
        .is_ok_and(|scene| scene.view.graph.relations().any(|r| r == relation))
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
                    if let Some(chosen) = detail::stops(ui, session.cut.detail) {
                        session.recut(chosen);
                    }
                    // What the reader opened or closed by hand departs from
                    // the stop beside it, so the stop alone would misname the
                    // picture.
                    if let Some(departures) = detail::departures(&session.cut) {
                        ui.label(egui::RichText::new(departures).weak().small());
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
                    if ui.button("Search (Ctrl+F)").clicked() {
                        session.palette.open();
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
                Some(Ok(session)) => inspector::show(ui, session),
            });

        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(Ok(session)) = &mut self.session {
                picture(ui, session);
            }
        });

        // The palette floats over everything and takes its keys before any
        // widget of the next frame reads them, so it paints after the panels
        // and beside the picture rather than inside it.
        if let Some(Ok(session)) = &mut self.session {
            let found = palette::show(
                ui.ctx(),
                &mut session.palette,
                &session.graph,
                session.viewport,
            );
            if let Some(target) = found {
                session.locate(&target);
            }
            // The digits reach the picture only after the palette had every
            // key of this frame: an open palette answers to keys of its own,
            // and a digit typed into its field is part of a name.
            if !session.palette.is_open()
                && let Some(detail) = detail::requested(ui.ctx())
            {
                session.recut(detail);
            }
        }
    }
}

/// The boundary canvas, and what a click on it does.
fn picture(ui: &mut egui::Ui, session: &mut Session) {
    session.viewport = ui.max_rect();
    let scene = match &session.scene {
        Err(reason) => {
            ui.colored_label(ui.visuals().error_fg_color, reason.as_str());
            return;
        }
        Ok(scene) => scene,
    };
    let edges = session.edges();
    let (selected_edge, selected_node) = match &session.selection {
        Some(Selection::Edge(relation)) => (Some(relation), None),
        Some(Selection::Node(id)) => (None, Some(id)),
        None => (None, None),
    };
    let action = canvas::show(
        ui,
        &Content {
            view: &scene.view.graph,
            layout: &scene.layout,
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
