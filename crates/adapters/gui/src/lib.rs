//! GUI adapter: the eframe/egui desktop shell of Cutaway.
//!
//! The GUI drives the application core and knows nothing about where
//! architectures or plans come from: the composition root hands it a
//! [`ProjectOpener`] and every opened project carries its own
//! [`PlanStore`]. The boundary canvas shows the architecture at an
//! adjustable level of detail; the user severs, draws, and annotates
//! connections, marks whole boundaries for removal, and plans new ones, and
//! every markup lands in the project's plan immediately.
//!
//! What the plan adds stands in the picture beside what the sources declare:
//! the lens views the architecture with the plan's own additions drawn in,
//! so a planned boundary takes a box, receives connections, and rolls up
//! exactly as a real one does. What the plan removes stays in the picture
//! too, marked red: the reader must see what is going. What the plan
//! modifies - renamed, split, merged, reworked - stays where it is and turns
//! blue: a modification states intent for whoever implements the plan and
//! redraws nothing, so only the mark can say it.
//!
//! The toolbar's three stops set the detail of the whole picture, and single
//! boundaries open or close on top of it: a double click on a boundary, or
//! the inspector's Expand and Collapse, moves that one boundary a step.
//! Choosing another stop drops those decisions, because a new whole is a new
//! question, and carries the selection across, because the subject of the
//! question stays the reader's.
//!
//! Focusing scopes the picture to one boundary: it becomes the whole
//! picture, its dependency partners stand at the border as single closed
//! boxes, and the rest of the project leaves. The scope outlives every stop
//! and every expansion - those say how the boundary is read, not which
//! boundary is read - and the toolbar names it until the reader shows
//! everything again.

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

use cutaway_architecture::{
    ArchitectureGraph, Element, ElementId, ElementKind, ElementName, Relation, RelationKind,
};
use cutaway_lenses::{BoundaryView, Cut, Detail, boundary_view};
use cutaway_planning::ports::plan_store::PlanStore;
use cutaway_planning::{
    GroupStanding, Modification, ModificationKind, Note, Plan, ProposedChange, SplitParts, Subject,
    addition_of_element,
};
use eframe::egui::{self, Rect};

use crate::camera::Camera;
use crate::canvas::{CanvasAction, Content, EdgeStatus, EdgeVisual, NodeStatus};
use crate::continuity::Piece;
use crate::label::{Labels, Renames};
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
    /// The architecture the picture shows: the sources' graph with the
    /// plan's own additions drawn into it. The lens reads this, and so does
    /// every question about where a planned element sits.
    viewed: ArchitectureGraph,
    plan: Plan,
    store: Box<dyn PlanStore>,
    /// Where the picture cuts the hierarchy: the detail of the whole, and
    /// the boundaries the reader opened or closed on top of it.
    cut: Cut,
    /// The new names the plan gives elements. Both the arrangement and the
    /// paint read the labels, so the renames resolve once per rebuild.
    renames: Renames,
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
    /// The element the reader is composing in the inspector's add field.
    addition: Addition,
    /// Which modification the reader is composing in the inspector, and the
    /// text it takes. The field outlives one frame, so a reader who asks for
    /// a split keeps typing into a split.
    modifying: Modifying,
    modification_draft: String,
    /// The element waiting for the box it folds into: a merge names a second
    /// element, and the reader picks it on the canvas.
    merging: Option<ElementId>,
    drawing: bool,
    draw_source: Option<ElementId>,
    status: Option<String>,
    /// Searching the whole architecture by name, whatever the picture shows.
    palette: Palette,
}

/// What the reader is composing in the inspector's add field: the name, and
/// the kind of boundary it will be. The kind outlives one frame, so a reader
/// who picks "type" and types a name still adds a type.
#[derive(Debug, Clone)]
struct Addition {
    name: String,
    kind: ElementKind,
}

impl Default for Addition {
    fn default() -> Self {
        Self {
            name: String::new(),
            // A module is what fits inside most boundaries; a panel that
            // cannot hold a module offers its own kinds and picks the first.
            kind: ElementKind::Module,
        }
    }
}

/// Which modification the inspector is taking text for. A rename and a split
/// need a name and a list of names; a rework and a merge need neither, so
/// they act the moment the reader asks for them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Modifying {
    #[default]
    Nothing,
    Rename,
    Split,
}

impl Session {
    fn open(project: OpenedProject) -> Self {
        let weights = layout::concept_weights(&project.graph);
        // A stored plan may predate the concrete-relation contract:
        // normalization re-anchors it to this graph before anything reads
        // or extends it, so every markup matches by provenance from here on.
        let plan = project.plan.normalized(&project.graph);
        let mut session = Self {
            viewed: project.graph.clone(),
            graph: project.graph,
            plan,
            store: project.store,
            cut: Cut::uniform(Detail::Packages),
            renames: Renames::default(),
            scene: Err("not built yet".to_owned()),
            weights,
            camera: Camera::default(),
            viewport: Rect::NOTHING,
            selection: None,
            note_draft: String::new(),
            addition: Addition::default(),
            modifying: Modifying::default(),
            modification_draft: String::new(),
            merging: None,
            drawing: false,
            draw_source: None,
            status: None,
            palette: Palette::default(),
        };
        session.rebuild_view();
        session
    }

    /// The element behind an id, read from the architecture the picture
    /// shows: it holds every element a view can draw, so an element below
    /// the current detail - and a planned one - still has a name.
    fn element_of(&self, id: &ElementId) -> Option<&Element> {
        self.viewed.element(id)
    }

    /// Paints the cut anew, leaving the camera where it is: opening or
    /// closing one boundary changes what the picture holds, not what the
    /// reader looks at. Whatever the new cut no longer shows is dropped.
    ///
    /// The lens views the architecture with the plan's own additions drawn
    /// in - planned elements, their containment, and drawn dependencies - so
    /// what the plan adds rolls up and reattaches at every cut exactly as
    /// the concrete architecture does. Augmenting the graph before lensing,
    /// rather than mapping planned parts through the view here, keeps the
    /// lens pure and this shell thin.
    fn rebuild_view(&mut self) {
        self.viewed = self.plan.viewed_architecture(&self.graph);
        // A renamed box says what it becomes, so the arrangement must give
        // it room for the longer text: the renames resolve before the
        // layout that measures them.
        self.renames = Renames::of(&self.plan);
        self.scene = boundary_view(&self.viewed, &self.cut)
            .map_err(|error| error.to_string())
            .map(|view| {
                let layout = layout::compute(&view.graph, &self.weights, &self.renames);
                Scene { view, layout }
            });
        if let Some(source) = &self.draw_source
            && !self.shows(source)
        {
            self.draw_source = None;
        }
        if let Some(subject) = &self.merging
            && !self.shows(subject)
        {
            self.merging = None;
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
        // The scope survives the new whole: a detail says how the reader
        // reads a boundary, never which boundary they are reading.
        let scope = self.cut.scope.clone();
        self.cut = Cut::uniform(detail);
        self.cut.focus(scope);
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

    /// Scopes the picture to one boundary: it becomes the whole picture, its
    /// dependency partners stand at the border as single closed boxes, and
    /// everything else leaves.
    fn focus(&mut self, id: &ElementId) {
        self.scoped_to(Some(id.clone()));
    }

    /// Puts the whole project back in the picture.
    fn unfocus(&mut self) {
        self.scoped_to(None);
    }

    /// Moves the picture to another scope. The selection carries over where
    /// the new picture still holds it, and the camera fits the new picture
    /// whole: another scope lays the world out anew, so the old coordinates
    /// point at arbitrary content.
    fn scoped_to(&mut self, scope: Option<ElementId>) {
        if self.cut.scope == scope {
            return;
        }
        self.status = None;
        self.cut.focus(scope);
        self.rebuild_view();
        self.refit();
    }

    /// Whether one element of the architecture stands inside the scope the
    /// picture holds, and true while the picture holds no scope: without a
    /// scope every element is in the picture's reach.
    fn within_scope(&self, id: &ElementId) -> bool {
        let Some(scope) = &self.cut.scope else {
            return true;
        };
        focus::subtree_of(&self.viewed, scope).contains(id)
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

    /// The note the editor shows for a selection. An element the plan
    /// modifies answers with the modification's own note: a rework is
    /// described by its note and nothing else, so the one note field beside
    /// the element must be that note rather than a second remark about an
    /// element the plan is already changing.
    fn current_note_text(&self, selection: &Selection) -> String {
        let note = match selection {
            Selection::Node(id) => match self.plan.modification_of(id) {
                Some(modification) => modification.note.as_ref(),
                None => self.plan.annotation_of(&Subject::Element(id.clone())),
            },
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
        // A half-written modification belongs to the element it was started
        // on; the next subject is a new question.
        self.modifying = Modifying::Nothing;
        self.modification_draft.clear();
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
        // A search reaches past the picture, and past its scope with it. An
        // element outside the scope is answered by showing everything again:
        // a scoped picture has no box to put it in.
        if !self.within_scope(target) {
            self.unfocus();
        }
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
                let id = id.clone();
                if let Some(modification) = self.plan.modification_of(&id) {
                    let described = Modification {
                        note: note.clone(),
                        ..modification.clone()
                    };
                    self.plan.plan_modification(described);
                    Ok(())
                } else {
                    let subject = Subject::Element(id);
                    match &note {
                        Some(note) => self.plan.annotate(subject, note.clone()),
                        None => self.plan.clear_annotation(&subject),
                    }
                    Ok(())
                }
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
    /// dependency behind it, so the mark holds at whatever cut those
    /// dependencies reattach. On a
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

    fn standing(&self, id: &ElementId) -> Standing {
        standing_of(&self.plan, &self.viewed, id)
    }

    /// What the plan does to each box the picture draws. A box the plan
    /// leaves alone stays out of the answer.
    fn node_statuses(&self) -> BTreeMap<ElementId, NodeStatus> {
        match &self.scene {
            Ok(scene) => node_statuses(&scene.view.graph, &self.viewed, &self.plan),
            Err(_) => BTreeMap::new(),
        }
    }

    /// Plans the removal of one element: the element itself, and with it
    /// everything it contains. The couplings that cross the border of what
    /// it holds are severed in the same act, because the element cannot
    /// leave while they stand.
    fn plan_removal(&mut self, id: &ElementId) {
        let mut result = Ok(());
        for change in self.plan.removal_of_element(id, &self.graph) {
            result = result.and(self.plan.propose(change));
        }
        match result {
            Ok(()) => self.save_plan(),
            Err(error) => self.status = Some(error.to_string()),
        }
    }

    /// Withdraws the planned removal of one element: the entry on the
    /// element, and every severing planned with it.
    fn restore_element(&mut self, id: &ElementId) {
        let planned = self.plan.planned_removal_of_element(id, &self.graph);
        if planned.is_empty() {
            self.status = Some("no removal is planned there".to_owned());
            return;
        }
        let mut result = Ok(());
        for change in planned {
            result = result.and(self.plan.retract(&change));
        }
        match result {
            Ok(()) => self.save_plan(),
            Err(error) => self.status = Some(error.to_string()),
        }
    }

    /// Plans a new element inside a boundary, or - with no boundary - at the
    /// root of the project. The element enters the picture at once: the
    /// viewed architecture carries the plan's additions, so the reader draws
    /// dependencies to it and annotates it exactly as they would a boundary
    /// the sources declare.
    fn add_element(&mut self, parent: Option<&ElementId>, kind: ElementKind) {
        let name = match ElementName::new(self.addition.name.trim()) {
            Ok(name) => name,
            Err(error) => {
                self.status = Some(error.to_string());
                return;
            }
        };
        let changes = match addition_of_element(parent, kind, &name) {
            Ok(changes) => changes,
            Err(error) => {
                self.status = Some(error.to_string());
                return;
            }
        };
        let Some(ProposedChange::AddElement(element)) = changes.first() else {
            return;
        };
        let added = element.id.clone();
        if self.viewed.element(&added).is_some() {
            self.status = Some(format!("{added} is already there"));
            return;
        }
        let mut result = Ok(());
        for change in changes {
            result = result.and(self.plan.propose(change));
        }
        match result {
            Ok(()) => {
                self.addition.name.clear();
                self.save_plan();
                self.rebuild_view();
                if self.shows(&added) {
                    let selection = Selection::Node(added);
                    self.select(Some(selection.clone()));
                    self.reveal(&selection);
                }
            }
            Err(error) => self.status = Some(error.to_string()),
        }
    }

    /// Erases a planned element: the element, every planned element inside
    /// it, their containment, and every dependency drawn to or from any of
    /// them. A note on an erased element goes with it - the element it
    /// explains is gone.
    fn erase_element(&mut self, id: &ElementId) {
        let planned = self.plan.planned_addition_of_element(id, &self.viewed);
        if planned.is_empty() {
            self.status = Some("no addition is planned there".to_owned());
            return;
        }
        let mut result = Ok(());
        for change in planned {
            if let ProposedChange::AddElement(element) = &change {
                self.plan
                    .clear_annotation(&Subject::Element(element.id.clone()));
            }
            result = result.and(self.plan.retract(&change));
        }
        match result {
            Ok(()) => self.save_plan(),
            Err(error) => self.status = Some(error.to_string()),
        }
        // The elements left the viewed graph, so the picture sheds their
        // boxes, and with them the selection.
        self.rebuild_view();
        self.select(None);
    }

    /// States that one element changes while staying where it is. Only an
    /// element the sources declare may be modified: an element that exists
    /// only in the plan carries whatever name and place the reader gave it,
    /// so the addition itself is what they edit.
    fn propose_modification(&mut self, subject: &ElementId, kind: ModificationKind) {
        let subject = subject.clone();
        if self.graph.element(&subject).is_none() {
            self.status = Some(format!(
                "{subject} exists only in the plan; change the planned element itself"
            ));
            return;
        }
        // A modification arriving on an element replaces whatever it
        // carried, so its old note explained a change that no longer stands.
        self.plan.plan_modification(Modification {
            subject,
            kind,
            note: None,
        });
        self.modifying = Modifying::Nothing;
        self.modification_draft.clear();
        self.save_plan();
        // A renamed box carries longer text and needs a wider box.
        self.rebuild_view();
        if let Some(selection) = self.selection.clone() {
            self.select(Some(selection));
        }
    }

    fn plan_rename(&mut self, subject: &ElementId) {
        match ElementName::new(self.modification_draft.trim()) {
            Ok(to) => self.propose_modification(subject, ModificationKind::Rename { to }),
            Err(error) => self.status = Some(error.to_string()),
        }
    }

    /// States the elements one boundary splits into, read from a
    /// comma-separated list: the reader names the parts in one field, and
    /// each name goes through [`ElementName`] like every other name in the
    /// architecture.
    fn plan_split(&mut self, subject: &ElementId) {
        let named: Result<Vec<ElementName>, _> = self
            .modification_draft
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(ElementName::new)
            .collect();
        let parts = match named
            .map_err(|error| error.to_string())
            .and_then(|names| SplitParts::new(names).map_err(|error| error.to_string()))
        {
            Ok(parts) => parts,
            Err(reason) => {
                self.status = Some(reason);
                return;
            }
        };
        self.propose_modification(subject, ModificationKind::Split { into: parts });
    }

    /// Asks the reader for the element this one folds into. The pick happens
    /// on the canvas, exactly as drawing a dependency does: a merge names
    /// two boundaries, and the picture is where the second one is found.
    fn begin_merge(&mut self, subject: &ElementId) {
        let subject = subject.clone();
        if self.graph.element(&subject).is_none() {
            self.status = Some(format!(
                "{subject} exists only in the plan; change the planned element itself"
            ));
            return;
        }
        self.drawing = false;
        self.draw_source = None;
        self.merging = Some(subject);
    }

    fn complete_merge(&mut self, target: &ElementId) {
        let Some(subject) = self.merging.clone() else {
            return;
        };
        let target = target.clone();
        if target == subject {
            self.status = Some("an element cannot merge into itself".to_owned());
            return;
        }
        if self.graph.element(&target).is_none() {
            self.status = Some(format!(
                "{target} exists only in the plan; nothing can fold into it yet"
            ));
            return;
        }
        self.merging = None;
        self.propose_modification(&subject, ModificationKind::Merge { with: target });
    }

    fn discard_modification(&mut self, subject: &ElementId) {
        let subject = subject.clone();
        if self.plan.modification_of(&subject).is_none() {
            self.status = Some("no modification is planned there".to_owned());
            return;
        }
        self.plan.discard_modification(&subject);
        self.modifying = Modifying::Nothing;
        self.modification_draft.clear();
        self.save_plan();
        self.rebuild_view();
        if let Some(selection) = self.selection.clone() {
            self.select(Some(selection));
        }
    }

    /// How many planned elements sit inside a planned one. Erasing the
    /// boundary erases them with it, and the button says so beforehand.
    fn planned_inside(&self, id: &ElementId) -> usize {
        self.plan
            .planned_addition_of_element(id, &self.viewed)
            .iter()
            .filter(|change| matches!(change, ProposedChange::AddElement(_)))
            .count()
            .saturating_sub(1)
    }

    /// The element every package hangs under, where the architecture names
    /// one. A planned package joins the project exactly as the inspected
    /// ones do.
    fn project_root(&self) -> Option<ElementId> {
        self.graph
            .elements()
            .find(|element| element.kind == ElementKind::Project)
            .map(|element| element.id.clone())
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
    /// the rolled-up edge whose provenance holds it. None while the relation
    /// is interior to one boundary.
    fn rendered_for(&self, concrete: &Relation) -> Option<Relation> {
        let scene = self.scene.as_ref().ok()?;
        scene
            .view
            .provenance
            .iter()
            .find(|(_, behind)| behind.contains(concrete))
            .map(|(edge, _)| edge.clone())
    }

    fn handle(&mut self, action: CanvasAction) {
        self.status = None;
        match action {
            CanvasAction::Node(id) => {
                if self.merging.is_some() {
                    self.complete_merge(&id);
                } else if self.drawing {
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
                if !self.drawing && self.merging.is_none() {
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

/// How the plan stands toward one element of the picture.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Standing {
    /// The architecture holds it and the plan leaves it alone.
    Existing,
    /// Planned for removal, by the entry on this element or on a boundary
    /// above it: a removal takes everything the element holds with it, so
    /// the root of the removal is what a restore acts on.
    Removed { root: ElementId },
    /// The element exists only in the plan.
    Added,
}

/// How the plan stands toward one box of the picture.
///
/// A removal reaches everything inside the boundary it names, so the answer
/// follows the containment of the viewed architecture rather than the plan's
/// entries alone: one entry marks a whole subtree, and nothing inside it
/// needs an entry of its own.
fn standing_of(plan: &Plan, viewed: &ArchitectureGraph, id: &ElementId) -> Standing {
    if let Some(root) = plan.removal_root_of(id, viewed) {
        return Standing::Removed { root };
    }
    if plan.plans_addition_of_element(id) {
        return Standing::Added;
    }
    Standing::Existing
}

/// What the plan does to each box of one picture. A box the plan leaves
/// alone stays out of the answer, so the canvas paints it as it always did.
///
/// What is going and what is arriving answer before what merely changes: an
/// element on its way out is not renamed, it is removed, and that is the
/// story the reader must read off the box.
fn node_statuses(
    view: &ArchitectureGraph,
    viewed: &ArchitectureGraph,
    plan: &Plan,
) -> BTreeMap<ElementId, NodeStatus> {
    view.elements()
        .filter_map(|element| {
            let status = match standing_of(plan, viewed, &element.id) {
                Standing::Removed { .. } => NodeStatus::Removed,
                Standing::Added => NodeStatus::Added,
                Standing::Existing => {
                    plan.modification_of(&element.id)?;
                    NodeStatus::Modified
                }
            };
            Some((element.id.clone(), status))
        })
        .collect()
}

/// Every connection one frame draws, with what the plan says about each.
///
/// The status of a connection derives from the concrete dependencies behind
/// it, never from the connection's own name: the name changes with every
/// cut, the dependencies do not, so a planned removal keeps its mark when a
/// boundary opens and the same dependencies reattach elsewhere.
fn edge_visuals(view: &BoundaryView, plan: &Plan) -> Vec<EdgeVisual> {
    view.provenance
        .keys()
        .map(|relation| {
            let concrete = view.concrete_behind(relation);
            let status = status_of_group(plan, &concrete);
            EdgeVisual {
                relation: relation.clone(),
                status,
                annotated: note_behind(plan, status, &concrete).is_some(),
                weight: weight_of(plan, status, &concrete),
            }
        })
        .collect()
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
                    project_tools(ui, session);
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
                if focus_requested(ui.ctx())
                    && let Some(Selection::Node(id)) = session.selection.clone()
                {
                    session.focus(&id);
                }
                // A mode that waits for a click on the picture must be
                // leavable without one, and Escape is the key that leaves.
                // With no mode waiting, Escape leaves the scope instead: it
                // is the one key that steps back out of wherever the reader
                // went, and only one thing is ever there to step out of.
                if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                    let waiting = session.drawing
                        || session.draw_source.is_some()
                        || session.merging.is_some();
                    session.drawing = false;
                    session.draw_source = None;
                    session.merging = None;
                    if !waiting {
                        session.unfocus();
                    }
                }
            }
        }
    }
}

/// Whether this frame's keys ask to scope the picture to the selection.
///
/// F is the first letter of what it does, and it commands only while no text
/// field holds the keyboard: in a note or a name, F is a letter. The search
/// takes ctrl+F before this reads anything, so the two never contend.
fn focus_requested(ctx: &egui::Context) -> bool {
    if ctx.text_edit_focused() {
        return false;
    }
    ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F))
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

/// What the toolbar offers about an open project: where the picture cuts,
/// the modes that wait for a click on it, the ways to move through it, and
/// what the last act answered.
///
/// A mode that waits for a click names itself here and nowhere else, so a
/// reader whose next click means something unusual reads that in one place -
/// and leaves the mode by clicking the same label again.
fn project_tools(ui: &mut egui::Ui, session: &mut Session) {
    ui.separator();
    ui.label("Detail");
    if let Some(chosen) = detail::stops(ui, session.cut.detail) {
        session.recut(chosen);
    }
    // What the reader opened or closed by hand departs from the stop beside
    // it, so the stop alone would misname the picture.
    if let Some(departures) = detail::departures(&session.cut) {
        ui.label(egui::RichText::new(departures).weak().small());
    }
    // A scoped picture leaves the rest of the project out, and a reader who
    // forgets that reads a whole project into one package. The scope
    // therefore names itself beside the stops, where the picture's own terms
    // stand, together with the way back out.
    if let Some(scope) = session.cut.scope.clone() {
        ui.separator();
        let name = Labels::renaming(&session.viewed, &session.renames).qualified(&scope);
        ui.label(format!("Focused on {name}"));
        if ui
            .button("Show everything")
            .on_hover_text("Put the whole project back in the picture. Esc.")
            .clicked()
        {
            session.unfocus();
        }
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
        session.merging = None;
    }
    // A merge is asked for on the panel and answered on the canvas, so the
    // toolbar says what the next click means.
    if let Some(subject) = session.merging.clone() {
        let folds = Labels::renaming(&session.viewed, &session.renames).qualified(&subject);
        if ui
            .selectable_label(true, "Merging: pick the element to merge into")
            .on_hover_text(format!(
                "{folds} folds into the element you pick. Esc cancels."
            ))
            .clicked()
        {
            session.merging = None;
        }
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
    let nodes = session.node_statuses();
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
            nodes: &nodes,
            renames: &session.renames,
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
        let view = boundary_view(&plan.viewed_architecture(graph), cut).unwrap();
        edge_visuals(&view, plan)
    }

    fn statuses(
        graph: &ArchitectureGraph,
        plan: &Plan,
        cut: &Cut,
    ) -> BTreeMap<ElementId, NodeStatus> {
        let viewed = plan.viewed_architecture(graph);
        let view = boundary_view(&viewed, cut).unwrap();
        node_statuses(&view.graph, &viewed, plan)
    }

    fn removing_package_a() -> Plan {
        let mut plan = Plan::new();
        for change in Plan::new().removal_of_element(&id("package:a"), &two_packages()) {
            plan.propose(change).unwrap();
        }
        plan
    }

    #[test]
    fn a_boundary_planned_for_removal_marks_everything_inside_it() {
        let statuses = statuses(
            &two_packages(),
            &removing_package_a(),
            &Cut::uniform(Detail::Modules),
        );
        for inside in ["package:a", "a/one", "a/two"] {
            assert_eq!(
                statuses.get(&id(inside)),
                Some(&NodeStatus::Removed),
                "{inside} goes with the package that holds it"
            );
        }
        assert_eq!(statuses.get(&id("package:b")), None);
        assert_eq!(statuses.get(&id("b/one")), None);
    }

    fn modifying(plan: &mut Plan, subject: &str, kind: ModificationKind) {
        plan.plan_modification(Modification {
            subject: id(subject),
            kind,
            note: None,
        });
    }

    #[test]
    fn a_boundary_the_plan_modifies_is_marked_where_it_stands() {
        let mut plan = Plan::new();
        modifying(
            &mut plan,
            "package:a",
            ModificationKind::Rename {
                to: ElementName::new("engine").unwrap(),
            },
        );

        let statuses = statuses(&two_packages(), &plan, &Cut::uniform(Detail::Modules));
        assert_eq!(statuses.get(&id("package:a")), Some(&NodeStatus::Modified));
        assert_eq!(
            statuses.get(&id("a/one")),
            None,
            "a modification speaks about the element it names and nothing inside it"
        );
    }

    #[test]
    fn a_boundary_on_its_way_out_reads_as_removed_however_it_was_modified() {
        let mut plan = removing_package_a();
        modifying(&mut plan, "package:a", ModificationKind::Rework);

        let statuses = statuses(&two_packages(), &plan, &Cut::uniform(Detail::Modules));
        assert_eq!(
            statuses.get(&id("package:a")),
            Some(&NodeStatus::Removed),
            "what is going is the story, not what it would have become"
        );
    }

    fn planning_a_module(parent: &str, name: &str) -> Plan {
        let mut plan = Plan::new();
        for change in addition_of_element(
            Some(&id(parent)),
            ElementKind::Module,
            &ElementName::new(name).unwrap(),
        )
        .unwrap()
        {
            plan.propose(change).unwrap();
        }
        plan
    }

    #[test]
    fn a_planned_element_stands_in_the_picture_marked_as_planned() {
        let plan = planning_a_module("package:a", "wiring");
        let statuses = statuses(&two_packages(), &plan, &Cut::uniform(Detail::Modules));
        assert_eq!(
            statuses.get(&id("package:a/wiring")),
            Some(&NodeStatus::Added)
        );
    }

    #[test]
    fn a_dependency_drawn_to_a_planned_element_draws_at_every_cut() {
        let mut plan = planning_a_module("package:a", "wiring");
        plan.propose(ProposedChange::AddRelation(depends(
            "b/one",
            "package:a/wiring",
        )))
        .unwrap();
        let graph = two_packages();

        let fine = visuals(&graph, &plan, &Cut::uniform(Detail::Modules));
        assert!(
            fine.iter()
                .any(|edge| edge.relation == depends("b/one", "package:a/wiring")
                    && edge.status == EdgeStatus::Drawn),
            "the planned element is in the viewed graph, so the lens draws to it: {fine:?}"
        );

        let coarse = visuals(&graph, &plan, &Cut::uniform(Detail::Packages));
        assert!(
            coarse
                .iter()
                .any(|edge| edge.relation == depends("package:b", "package:a")
                    && edge.status == EdgeStatus::Drawn),
            "and rolls it up to the packages exactly as a concrete one: {coarse:?}"
        );
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
            scope: None,
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
            "the whole-frame addition attaches to the frame's border like any dependency"
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
