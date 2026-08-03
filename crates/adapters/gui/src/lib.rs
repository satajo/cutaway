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

mod bundle;
mod camera;
mod canvas;
mod continuity;
mod detail;
mod focus;
mod glyph;
mod inspector;
mod label;
mod layout;
mod minimap;
mod palette;
mod routing;
mod summary;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use cutaway_architecture::{ArchitectureGraph, Element, ElementId, Relation, RelationKind};
use cutaway_lenses::{BoundaryView, Cut, Detail, boundary_view, self_leaf_frame};
use cutaway_planning::ports::plan_store::PlanStore;
use cutaway_planning::{GroupStanding, Note, Plan, ProposedChange, Subject};
use eframe::egui::{self, Rect};

use crate::camera::Camera;
use crate::canvas::{CanvasAction, Content, EdgeStatus, EdgeVisual};
use crate::continuity::Piece;
use crate::label::Labels;
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

/// What a selection asks the camera to show, in world coordinates: the box
/// of a boundary, and both ends of a connection together, because a
/// connection is the run between them and one end alone says nothing about
/// where the other lies. An end the picture has no box for is left out;
/// nothing the picture draws at all answers None.
fn subject_of(layout: &Layout, selection: &Selection) -> Option<Rect> {
    let boxed = |id| layout.rects.get(id).copied();
    match selection {
        Selection::Node(id) => boxed(id),
        Selection::Edge(relation) => match (boxed(&relation.from), boxed(&relation.to)) {
            (Some(from), Some(to)) => Some(from.union(to)),
            (Some(one), None) | (None, Some(one)) => Some(one),
            (None, None) => None,
        },
    }
}

/// What the toolbar says when a connection carried into a new detail only in
/// part: the new picture draws the dependencies behind it as several
/// connections, and the selection follows the largest of them. Naming the
/// connection the reader asked about tells them which question the picture
/// is still answering.
fn following_a_piece(from: &str, to: &str, piece: &Piece) -> String {
    format!(
        "Following {from} {} {to}: largest piece at this detail, {} of {} dependencies.",
        glyph::OUTWARD,
        piece.carried,
        piece.whole
    )
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
    /// Where the picture stands in front of the reader, and where it is
    /// travelling.
    camera: Camera,
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
        // A stored plan may predate the concrete-relation contract:
        // normalization re-anchors it to this graph before anything reads
        // or extends it, so every markup matches by provenance from here on.
        let plan = project.plan.normalized(&project.graph);
        let mut session = Self {
            graph: project.graph,
            plan,
            store: project.store,
            cut: Cut::uniform(Detail::Packages),
            scene: Err("not built yet".to_owned()),
            weights,
            camera: Camera::default(),
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
    ///
    /// The lens views the architecture with the plan's added dependencies
    /// drawn in, so a drawn connection rolls up and reattaches at every cut
    /// exactly as the concrete ones do. Augmenting the graph before lensing,
    /// rather than mapping planned endpoints through the view here, keeps
    /// the lens pure and this shell thin, and planned elements will later
    /// enter the picture the same way.
    fn rebuild_view(&mut self) {
        let viewed = self.plan.with_planned_dependencies(&self.graph);
        self.scene = boundary_view(&viewed, &self.cut)
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
    /// another connection, and the camera travels to where it reappeared.
    /// Nothing selected leaves the camera where it is, unless the new picture
    /// no longer meets it at all.
    ///
    /// A connection can reappear as several, and then the selection follows
    /// the largest piece alone; the toolbar says so, because the reader
    /// asked about the whole.
    fn recut(&mut self, detail: Detail) {
        if self.cut.detail == detail {
            return;
        }
        // A new detail is a new question, and the answers to the last one no
        // longer stand.
        self.status = None;
        let before = self
            .selection
            .take()
            .and_then(|selection| Some((self.scene.as_ref().ok()?.view.clone(), selection)));
        self.cut = Cut::uniform(detail);
        self.rebuild_view();
        let carried = before.and_then(|(before, selection)| {
            let after = self.scene.as_ref().ok()?;
            let carried = continuity::translated(&self.graph, &before, &after.view, &selection)?;
            let note = carried.piece.as_ref().and_then(|piece| {
                let Selection::Edge(relation) = &selection else {
                    return None;
                };
                let labels = Labels::of(&before.graph);
                Some(following_a_piece(
                    &labels.qualified(&relation.from),
                    &labels.qualified(&relation.to),
                    piece,
                ))
            });
            Some((carried.selection, note))
        });
        if let Some((selection, note)) = carried {
            self.status = note;
            self.select(Some(selection.clone()));
            self.reveal(&selection);
        } else {
            // Each detail lays the picture out anew, so the old camera
            // coordinates point at arbitrary new content: without a subject
            // to follow, only a fresh fit shows something meaningful.
            self.select(None);
            self.camera.forget();
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
        match &self.scene {
            Ok(scene) => edge_visuals(&scene.view, &self.plan),
            Err(_) => Vec::new(),
        }
    }

    /// Every concrete dependency behind one rendered connection; empty
    /// while the picture draws no such connection.
    fn concrete_behind(&self, relation: &Relation) -> BTreeSet<Relation> {
        self.scene
            .as_ref()
            .map(|scene| scene.view.concrete_behind(relation))
            .unwrap_or_default()
    }

    /// How the plan stands toward a rendered connection, derived from the
    /// concrete dependencies behind it rather than from the connection's
    /// own name: the name changes with every cut, the dependencies do not.
    fn status_of(&self, relation: &Relation) -> EdgeStatus {
        status_of_group(&self.plan, &self.concrete_behind(relation))
    }

    fn current_note_text(&self, selection: &Selection) -> String {
        let note = match selection {
            Selection::Node(id) => self.plan.annotation_of(&Subject::Element(real_id(id))),
            Selection::Edge(relation) => {
                let concrete = self.concrete_behind(relation);
                note_behind(
                    &self.plan,
                    status_of_group(&self.plan, &concrete),
                    &concrete,
                )
            }
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

    /// Travels until the whole subject of a selection stands in front of the
    /// reader: a selection made beside the picture, or carried into a
    /// picture cut at another detail, must be findable in it. A subject
    /// already in comfortable view moves nothing.
    fn reveal(&mut self, selection: &Selection) {
        let (Some(at), Ok(scene)) = (self.camera.now(), &self.scene) else {
            // No camera yet means no frame has fitted the picture, and the
            // fit that comes shows everything anyway.
            return;
        };
        let Some(subject) = subject_of(&scene.layout, selection) else {
            return;
        };
        if let Some(moved) = camera::revealing(self.viewport, at, subject) {
            self.camera.fly(moved);
        }
    }

    /// Puts the whole picture back in front of the reader, exactly as a
    /// double click on the background does: the same fit, travelled to over
    /// the same flight, so the two ways of asking answer alike.
    fn refit(&mut self) {
        let (Some(_), Ok(scene)) = (self.camera.now(), &self.scene) else {
            // No camera yet means no frame has fitted the picture, and the
            // fit that comes shows everything anyway.
            return;
        };
        let world = canvas::world_bounds(&scene.layout);
        self.camera.fly(camera::fit(world, self.viewport));
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
        let selection = Selection::Node(found);
        self.select(Some(selection.clone()));
        self.reveal(&selection);
    }

    fn save_plan(&mut self) {
        self.status = self
            .store
            .save(&self.plan)
            .err()
            .map(|error| error.to_string());
    }

    /// Writes the note draft onto whatever anchors the selection: an
    /// element's annotation, or - for a connection - the concrete
    /// dependencies behind it. A note on a rolled-up connection lands on
    /// every concrete dependency it stands for, so the note is findable at
    /// whatever cut shows any of them.
    fn save_note(&mut self) {
        let Some(selection) = self.selection.clone() else {
            return;
        };
        // An emptied note field clears the note.
        let note = Note::new(self.note_draft.clone()).ok();
        let result = match &selection {
            Selection::Node(id) => {
                let subject = Subject::Element(real_id(id));
                match &note {
                    Some(note) => self.plan.annotate(subject, note.clone()),
                    None => self.plan.clear_annotation(&subject),
                }
                Ok(())
            }
            Selection::Edge(relation) => {
                let concrete = self.concrete_behind(relation);
                let status = status_of_group(&self.plan, &concrete);
                let mut result = Ok(());
                for concrete in &concrete {
                    match status {
                        // The additions folded into an existing run stand
                        // aside: an annotation talks about what exists.
                        EdgeStatus::Existing | EdgeStatus::PartiallySevered { .. } => {
                            if self.plan.plans_addition_of(concrete) {
                                continue;
                            }
                            let subject = Subject::Relation(concrete.clone());
                            match &note {
                                Some(note) => self.plan.annotate(subject, note.clone()),
                                None => self.plan.clear_annotation(&subject),
                            }
                        }
                        EdgeStatus::Severed => {
                            if !self.plan.plans_removal_of(concrete) {
                                continue;
                            }
                            result = result.and(self.plan.explain(
                                &ProposedChange::RemoveRelation(concrete.clone()),
                                note.clone(),
                            ));
                        }
                        EdgeStatus::Drawn => {
                            result = result.and(self.plan.explain(
                                &ProposedChange::AddRelation(concrete.clone()),
                                note.clone(),
                            ));
                        }
                    }
                }
                result
            }
        };
        match result {
            Ok(()) => self.save_plan(),
            Err(error) => self.status = Some(error.to_string()),
        }
    }

    /// Severs a rendered connection: proposes the removal of every concrete
    /// dependency behind it, the ones waiting at a coarser detail included,
    /// so the mark holds at whatever cut those dependencies reattach. On a
    /// partly severed connection this completes the removal of the rest; on
    /// a drawn one it erases the additions instead.
    fn sever(&mut self, relation: &Relation) {
        let concrete = self.concrete_behind(relation);
        if concrete.is_empty() {
            self.status = Some("nothing stands behind that connection".to_owned());
            return;
        }
        if matches!(self.plan.standing_of(&concrete), GroupStanding::Added) {
            let mut result = Ok(());
            for concrete in &concrete {
                result = result.and(
                    self.plan
                        .retract(&ProposedChange::AddRelation(concrete.clone())),
                );
            }
            match result {
                Ok(()) => self.save_plan(),
                Err(error) => self.status = Some(error.to_string()),
            }
            // The additions left the viewed graph, so the picture sheds the
            // connection, and with it the selection.
            self.rebuild_view();
            self.select(None);
            return;
        }
        let mut result = Ok(());
        for concrete in concrete {
            if self.plan.plans_addition_of(&concrete) || self.plan.plans_removal_of(&concrete) {
                continue;
            }
            result = result.and(self.plan.propose(ProposedChange::RemoveRelation(concrete)));
        }
        match result {
            Ok(()) => self.save_plan(),
            Err(error) => self.status = Some(error.to_string()),
        }
    }

    /// Withdraws every planned removal behind a rendered connection.
    fn restore(&mut self, relation: &Relation) {
        let planned: Vec<Relation> = self
            .concrete_behind(relation)
            .into_iter()
            .filter(|concrete| self.plan.plans_removal_of(concrete))
            .collect();
        if planned.is_empty() {
            self.status = Some("no removal is planned there".to_owned());
            return;
        }
        let mut result = Ok(());
        for concrete in planned {
            result = result.and(self.plan.retract(&ProposedChange::RemoveRelation(concrete)));
        }
        match result {
            Ok(()) => self.save_plan(),
            Err(error) => self.status = Some(error.to_string()),
        }
    }

    fn draw_edge(&mut self, from: ElementId, to: ElementId) {
        let picked = Relation {
            from,
            to,
            kind: RelationKind::DependsOn,
        };
        if edge_exists(&self.scene, &picked) {
            self.status = Some("that dependency already exists".to_owned());
            return;
        }
        // A pick on a frame's own-content box means the frame: the leaf is
        // the lens's invention, and the plan records only elements the
        // sources can hold.
        let relation = frame_pair(&picked);
        if relation.from == relation.to {
            self.status = Some("a boundary cannot depend on itself".to_owned());
            return;
        }
        if self.graph.relations().any(|r| *r == relation) {
            self.status = Some("that dependency already exists".to_owned());
            return;
        }
        match self
            .plan
            .propose(ProposedChange::AddRelation(relation.clone()))
        {
            Ok(()) => {
                self.save_plan();
                // The addition enters the viewed graph, and the lens rolls
                // it up like any other dependency; the selection follows the
                // connection that carries it here.
                self.rebuild_view();
                let rendered = self.rendered_for(&relation);
                self.select(rendered.map(Selection::Edge));
            }
            Err(error) => self.status = Some(error.to_string()),
        }
    }

    /// The rendered connection carrying one concrete relation at this cut:
    /// the rolled-up edge whose provenance holds it, or the coarse pair it
    /// waits under. None while the relation is interior to one boundary.
    fn rendered_for(&self, concrete: &Relation) -> Option<Relation> {
        let scene = self.scene.as_ref().ok()?;
        scene
            .view
            .provenance
            .iter()
            .chain(scene.view.coarse.iter())
            .find(|(_, behind)| behind.contains(concrete))
            .map(|(edge, _)| edge.clone())
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

/// The element an id truly names: itself, or - for a frame's own-content
/// leaf - the frame. Nothing the plan records may carry a self-leaf id,
/// because no source graph holds one.
fn real_id(id: &ElementId) -> ElementId {
    self_leaf_frame(id).unwrap_or_else(|| id.clone())
}

/// A rendered edge named by the boundaries the self leaves stand for.
fn frame_pair(relation: &Relation) -> Relation {
    Relation {
        from: real_id(&relation.from),
        to: real_id(&relation.to),
        kind: relation.kind,
    }
}

/// Every connection one frame draws, with what the plan says about each.
///
/// The status of a connection derives from the concrete dependencies behind
/// it, never from the connection's own name: the name changes with every
/// cut, the dependencies do not, so a planned removal keeps its mark when a
/// boundary opens and the same dependencies reattach elsewhere.
fn edge_visuals(view: &BoundaryView, plan: &Plan) -> Vec<EdgeVisual> {
    let mut edges = Vec::new();
    let mut rendered_pairs = BTreeSet::new();
    for relation in view.provenance.keys() {
        rendered_pairs.insert(frame_pair(relation));
        let concrete = view.concrete_behind(relation);
        let status = status_of_group(plan, &concrete);
        edges.push(EdgeVisual {
            relation: relation.clone(),
            status,
            annotated: note_behind(plan, status, &concrete).is_some(),
            weight: weight_of(plan, status, &concrete),
        });
    }
    // A planned addition naming an open frame as a whole would wait at a
    // coarser detail, as the architecture's own whole-frame dependencies
    // do; but a drawn dependency is the plan speaking, and the plan speaks
    // at every cut, so it attaches to the frame's border instead. A pair a
    // rendered edge already answers for stays out, or it would draw twice.
    for (pair, concrete) in &view.coarse {
        if matches!(plan.standing_of(concrete), GroupStanding::Added)
            && !rendered_pairs.contains(pair)
        {
            edges.push(EdgeVisual {
                relation: pair.clone(),
                status: EdgeStatus::Drawn,
                annotated: note_behind(plan, EdgeStatus::Drawn, concrete).is_some(),
                weight: concrete.len(),
            });
        }
    }
    edges
}

fn status_of_group(plan: &Plan, concrete: &BTreeSet<Relation>) -> EdgeStatus {
    match plan.standing_of(concrete) {
        GroupStanding::Untouched => EdgeStatus::Existing,
        GroupStanding::Added => EdgeStatus::Drawn,
        GroupStanding::Removed => EdgeStatus::Severed,
        GroupStanding::PartlyRemoved { removed, of } => EdgeStatus::PartiallySevered {
            severed: removed,
            total: of,
        },
    }
}

/// How many concrete dependencies one rendered edge stands for: what the
/// architecture carries behind it, or - drawn - what the plan proposes. An
/// edge with nothing recorded behind it still stands for the one dependency
/// it is.
fn weight_of(plan: &Plan, status: EdgeStatus, concrete: &BTreeSet<Relation>) -> usize {
    let counted = match status {
        EdgeStatus::Drawn => concrete.len(),
        _ => concrete
            .iter()
            .filter(|relation| !plan.plans_addition_of(relation))
            .count(),
    };
    counted.max(1)
}

/// The note a rendered edge shows, read from the concrete dependencies
/// behind it: the plan's rationale where the connection is planned, the
/// annotation where it merely exists. Several dependencies can carry notes;
/// the first in relation order answers, and saving a note writes them all
/// alike again.
fn note_behind<'p>(
    plan: &'p Plan,
    status: EdgeStatus,
    concrete: &BTreeSet<Relation>,
) -> Option<&'p Note> {
    concrete.iter().find_map(|relation| match status {
        EdgeStatus::Drawn => plan.note_of(&ProposedChange::AddRelation(relation.clone())),
        EdgeStatus::Severed => plan.note_of(&ProposedChange::RemoveRelation(relation.clone())),
        EdgeStatus::Existing | EdgeStatus::PartiallySevered { .. } => {
            if plan.plans_addition_of(relation) {
                None
            } else {
                plan.annotation_of(&Subject::Relation(relation.clone()))
            }
        }
    })
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
                    .add_enabled(
                        !picking,
                        egui::Button::new(format!("Open repository{}", glyph::ELLIPSIS)),
                    )
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
                    if ui
                        .button("Fit (Home)")
                        .on_hover_text(
                            "Bring the whole picture back into the canvas. \
                             Home, or a double click on the background.",
                        )
                        .clicked()
                    {
                        session.refit();
                    }
                    if let Some(status) = &session.status {
                        ui.separator();
                        ui.colored_label(ui.visuals().warn_fg_color, status);
                    }
                }
            });
        });

        // Nothing opened yet leaves the inspector nothing to inspect, and an
        // empty panel beside an empty canvas reads as a broken window: the
        // invitation in the middle carries the whole of it instead.
        if let Some(session) = &mut self.session {
            egui::Panel::right("inspector")
                .default_size(280.0)
                .show(ui, |ui| match session {
                    Err(reason) => {
                        ui.colored_label(ui.visuals().error_fg_color, reason.as_str());
                    }
                    Ok(session) => inspector::show(ui, session),
                });
        }

        let mut asked_to_open = false;
        egui::CentralPanel::default().show(ui, |ui| match &mut self.session {
            Some(Ok(session)) => picture(ui, session),
            // A repository that failed to open leaves the invitation
            // standing: the reader reads the reason beside it and picks
            // another one from here.
            None | Some(Err(_)) => asked_to_open = invitation(ui, self.picker.is_some()),
        });
        if asked_to_open {
            self.pick_repository(ui.ctx().clone());
        }

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
            // The keys reach the picture only after the palette had every
            // one of this frame: an open palette answers to keys of its own,
            // and a digit typed into its field is part of a name.
            if !session.palette.is_open() {
                if let Some(detail) = detail::requested(ui.ctx()) {
                    session.recut(detail);
                }
                if camera::refit_requested(ui.ctx()) {
                    session.refit();
                }
            }
        }
    }
}

/// The empty canvas, before an architecture stands on it. A dark expanse
/// with a small button in the corner says neither what the window is for nor
/// where to begin, so the middle of the canvas names the tool, says what it
/// does, and offers the one act that starts the work. Answers whether the
/// reader asked for the repository picker.
fn invitation(ui: &mut egui::Ui, picking: bool) -> bool {
    let mut asked = false;
    ui.vertical_centered(|ui| {
        // The invitation sits above the middle, where the eye lands before
        // it searches.
        ui.add_space(ui.available_height() * 0.3);
        ui.heading("Cutaway");
        ui.label(
            egui::RichText::new(
                "See a repository as boundaries and the dependencies that cross them.",
            )
            .weak(),
        );
        ui.add_space(ui.spacing().item_spacing.y * 2.0);
        asked = ui
            .add_enabled(
                !picking,
                egui::Button::new(format!("Open a repository{}", glyph::ELLIPSIS)),
            )
            .clicked();
    });
    asked
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

#[cfg(test)]
mod tests {
    use eframe::egui::{pos2, vec2};

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

    /// Two boxes in opposite corners of a world, and nothing else.
    fn layout() -> Layout {
        Layout {
            rects: BTreeMap::from([
                (
                    id("a"),
                    Rect::from_min_size(pos2(0.0, 0.0), vec2(40.0, 20.0)),
                ),
                (
                    id("b"),
                    Rect::from_min_size(pos2(600.0, 400.0), vec2(40.0, 20.0)),
                ),
            ]),
            containers: Vec::new(),
            leaves: vec![id("a"), id("b")],
        }
    }

    #[test]
    fn a_selected_boundary_asks_for_its_own_box() {
        assert_eq!(
            subject_of(&layout(), &Selection::Node(id("a"))),
            Some(layout().rects[&id("a")])
        );
    }

    #[test]
    fn a_selected_connection_asks_for_both_of_its_ends() {
        assert_eq!(
            subject_of(&layout(), &Selection::Edge(depends("a", "b"))),
            Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(640.0, 420.0))),
            "the run between the boxes is the subject, not either end of it"
        );
    }

    #[test]
    fn a_connection_with_one_end_in_the_picture_asks_for_that_end() {
        assert_eq!(
            subject_of(&layout(), &Selection::Edge(depends("a", "elsewhere"))),
            Some(layout().rects[&id("a")])
        );
    }

    #[test]
    fn nothing_the_picture_draws_asks_for_nothing() {
        assert_eq!(
            subject_of(&layout(), &Selection::Node(id("elsewhere"))),
            None
        );
    }

    fn add(graph: &mut ArchitectureGraph, id_text: &str, kind: cutaway_architecture::ElementKind) {
        graph
            .add_element(Element {
                id: id(id_text),
                name: cutaway_architecture::ElementName::new(id_text).unwrap(),
                kind,
            })
            .unwrap();
    }

    /// package:a ⊃ {a/one, a/two}, package:b ⊃ {b/one}, with both of a's
    /// modules depending on b/one.
    fn two_packages() -> ArchitectureGraph {
        use cutaway_architecture::ElementKind;
        let mut graph = ArchitectureGraph::new();
        add(&mut graph, "package:a", ElementKind::Package);
        add(&mut graph, "package:b", ElementKind::Package);
        add(&mut graph, "a/one", ElementKind::Module);
        add(&mut graph, "a/two", ElementKind::Module);
        add(&mut graph, "b/one", ElementKind::Module);
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
        for from in ["a/one", "a/two"] {
            graph.add_relation(depends(from, "b/one")).unwrap();
        }
        graph
    }

    fn visuals(graph: &ArchitectureGraph, plan: &Plan, cut: &Cut) -> Vec<EdgeVisual> {
        let view = boundary_view(&plan.with_planned_dependencies(graph), cut).unwrap();
        edge_visuals(&view, plan)
    }

    fn removing(relations: &[Relation]) -> Plan {
        let mut plan = Plan::new();
        for relation in relations {
            plan.propose(ProposedChange::RemoveRelation(relation.clone()))
                .unwrap();
        }
        plan
    }

    #[test]
    fn a_connection_with_every_dependency_severed_draws_as_severed() {
        let plan = removing(&[depends("a/one", "b/one"), depends("a/two", "b/one")]);
        let edges = visuals(&two_packages(), &plan, &Cut::uniform(Detail::Packages));
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].status, EdgeStatus::Severed);
        assert_eq!(edges[0].weight, 2);
    }

    #[test]
    fn a_connection_with_part_of_its_dependencies_severed_draws_the_partial_mark() {
        let plan = removing(&[depends("a/one", "b/one")]);
        let edges = visuals(&two_packages(), &plan, &Cut::uniform(Detail::Packages));
        assert_eq!(
            edges[0].status,
            EdgeStatus::PartiallySevered {
                severed: 1,
                total: 2
            }
        );
    }

    #[test]
    fn a_planned_removal_keeps_its_mark_when_the_target_boundary_opens() {
        let plan = removing(&[depends("a/one", "b/one"), depends("a/two", "b/one")]);
        let opened = Cut {
            detail: Detail::Packages,
            overrides: BTreeMap::from([(id("package:b"), Detail::Modules)]),
        };
        let edges = visuals(&two_packages(), &plan, &opened);
        assert_eq!(edges.len(), 1);
        assert_eq!(
            edges[0].relation,
            depends("package:a", "b/one"),
            "the same dependencies reattach to the module the boundary opened onto"
        );
        assert_eq!(edges[0].status, EdgeStatus::Severed);
    }

    #[test]
    fn a_drawn_dependency_draws_at_every_cut() {
        let mut graph = two_packages();
        for from in ["a/one", "a/two"] {
            graph.remove_relation(&depends(from, "b/one")).unwrap();
        }
        let mut plan = Plan::new();
        plan.propose(ProposedChange::AddRelation(depends("a/one", "b/one")))
            .unwrap();

        let coarse = visuals(&graph, &plan, &Cut::uniform(Detail::Packages));
        assert_eq!(coarse.len(), 1);
        assert_eq!(coarse[0].relation, depends("package:a", "package:b"));
        assert_eq!(coarse[0].status, EdgeStatus::Drawn);

        let fine = visuals(&graph, &plan, &Cut::uniform(Detail::Modules));
        assert_eq!(fine.len(), 1);
        assert_eq!(fine[0].relation, depends("a/one", "b/one"));
        assert_eq!(fine[0].status, EdgeStatus::Drawn);
    }

    #[test]
    fn a_drawn_dependency_naming_an_open_frame_still_draws() {
        let mut graph = two_packages();
        for from in ["a/one", "a/two"] {
            graph.remove_relation(&depends(from, "b/one")).unwrap();
        }
        let mut plan = Plan::new();
        plan.propose(ProposedChange::AddRelation(depends("a/one", "package:b")))
            .unwrap();

        let edges = visuals(&graph, &plan, &Cut::uniform(Detail::Modules));
        assert_eq!(
            edges.len(),
            1,
            "the whole-frame addition does not wait at a coarser detail"
        );
        assert_eq!(edges[0].relation, depends("a/one", "package:b"));
        assert_eq!(edges[0].status, EdgeStatus::Drawn);
    }

    #[test]
    fn a_note_about_a_split_connection_names_both_of_its_old_ends() {
        let note = following_a_piece(
            "cutaway-gui",
            "cutaway-architecture",
            &Piece {
                carried: 4,
                whole: 11,
            },
        );

        assert_eq!(
            note,
            format!(
                "Following cutaway-gui {} cutaway-architecture: largest piece at this \
                 detail, 4 of 11 dependencies.",
                glyph::OUTWARD
            )
        );
    }
}
