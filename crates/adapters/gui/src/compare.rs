//! Compare mode: two versions of one project in a single picture.
//!
//! The reader names a before and an after version. The picture lays out the
//! union of the two architectures, so a boundary takes a box wherever
//! either version has something to say, and every box and every connection
//! then reads as what happened to it between the two.
//!
//! The paint is the same vocabulary the plan speaks in: green for what
//! arrives, red for what goes, amber for a boundary that stands in both
//! versions and changes inside itself. The mode is what says which story
//! the colours tell - in Explore they are what the reader intends, here
//! they are what the history records - so one visual language reads both
//! pictures.
//!
//! Nothing here edits. A comparison is a reading of what already happened,
//! and the plan stays out of it.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{ArchitectureGraph, ElementId, ElementKind, Relation, RelationKind};
use cutaway_comparison::{Comparison, ElementChange, Presence};
use cutaway_inspection::ports::project_history::{Version, VersionId};
use cutaway_lenses::{BoundaryView, Cut, boundary_view};
use eframe::egui::{self, Rect};

use crate::camera::{self, Camera};
use crate::canvas::{self, CanvasAction, Content, EdgeStatus, EdgeVisual, NodeStatus};
use crate::focus::{self, Containment};
use crate::label::{self, Labels, Renames};
use crate::palette::Palette;
use crate::{Scene, Selection, VersionInspector, glyph, layout, palette, subject_of, vocabulary};

/// How many characters of a version id name it in the interface: enough to
/// tell the versions of one project apart, and the same prefix the reader
/// reads in their own log.
const SHORT_ID: usize = 7;

/// The versions of the open project, and the way to read the architecture
/// of one.
///
/// The shell holds this beside the comparison rather than inside it: the
/// history answers for the whole project and outlives every pair the reader
/// puts against each other.
pub(crate) struct History {
    versions: Vec<Version>,
    inspect: VersionInspector,
}

impl History {
    pub(crate) fn new(versions: Vec<Version>, inspect: VersionInspector) -> Self {
        Self { versions, inspect }
    }
}

/// One comparison in front of the reader: the pair, the architectures
/// behind it, and everything that paints the picture of the difference.
pub(crate) struct CompareSession {
    before: VersionId,
    after: VersionId,
    /// The architecture of every version read so far. A version costs one
    /// inspection of its whole source tree, so a picker flipped back to a
    /// version already read answers from here.
    graphs: BTreeMap<VersionId, ArchitectureGraph>,
    comparison: Comparison,
    /// Where the picture cuts the union: the frontier of open boundaries
    /// and the vocabulary of kinds it renders. Element ids hold across
    /// versions, so a cut made against one pair still names boundaries of
    /// the next.
    cut: Cut,
    scene: Result<Scene, String>,
    /// How many scenes this comparison has built. Every rebuild raises it,
    /// and the number rides along in the scene it produced.
    generation: u64,
    strokes: canvas::Strokes,
    /// Contained-concept counts from the union; they size the boxes.
    weights: BTreeMap<ElementId, usize>,
    /// A comparison renames nothing: renaming is a plan's act, and the plan
    /// stays out of this picture. The labels and the layout still ask for
    /// renames, so the empty set stands here.
    renames: Renames,
    camera: Camera,
    /// The screen rectangle the canvas painted into last, recorded so a
    /// search can reveal what it found inside it.
    viewport: Rect,
    selection: Option<Selection>,
    status: Option<String>,
    /// Searching the whole union by name, whatever the picture shows.
    palette: Palette,
}

impl CompareSession {
    /// Compares the two newest versions of a project. Refused only where
    /// the history lists no version at all: without one there is nothing to
    /// name in a picker, let alone a pair.
    pub(crate) fn build(history: &History) -> Result<Self, String> {
        let (before, after) = default_pair(&history.versions)
            .ok_or_else(|| "this project lists no version to compare".to_owned())?;
        let mut session = Self {
            before,
            after,
            graphs: BTreeMap::new(),
            // An empty comparison stands until the pair below is read; a
            // version costs an inspection, and one code path reads them.
            comparison: Comparison::between(&ArchitectureGraph::new(), &ArchitectureGraph::new()),
            cut: Cut::whole(),
            scene: Err("not built yet".to_owned()),
            generation: 0,
            strokes: canvas::Strokes::default(),
            weights: BTreeMap::new(),
            renames: Renames::default(),
            camera: Camera::default(),
            viewport: Rect::NOTHING,
            selection: None,
            status: None,
            palette: Palette::default(),
        };
        session.retarget(history);
        Ok(session)
    }

    /// Whether the search palette holds the keyboard.
    pub(crate) fn searching(&self) -> bool {
        self.palette.is_open()
    }

    /// Reads the pair the pickers name and paints it, keeping whatever the
    /// reader opened: element ids derive from the sources alone, so the
    /// boundaries opened against one pair name the same boundaries of the
    /// next.
    fn retarget(&mut self, history: &History) {
        let before = self.before.clone();
        let after = self.after.clone();
        self.status = (before == after).then(|| {
            "Both pickers name one version; the picture shows it against itself.".to_owned()
        });
        if let Err(reason) = self
            .read(history, &before)
            .and_then(|()| self.read(history, &after))
        {
            self.scene = Err(reason);
            self.selection = None;
            return;
        }
        self.comparison = Comparison::between(&self.graphs[&before], &self.graphs[&after]);
        self.weights = layout::concept_weights(self.comparison.union());
        // The pair was named to see what changed between it, so the
        // boundaries hiding a change open by themselves. The reader closes
        // what they do not care for, and the Focus changes button reopens.
        self.cut
            .open
            .extend(boundaries_hiding_changes(&self.comparison));
        self.repaint();
        // A new pair lays the world out anew, so the old camera coordinates
        // point at arbitrary content: only a fresh fit shows the picture the
        // reader asked for.
        self.refit();
    }

    /// Reads the architecture of one version, unless it is already read.
    fn read(&mut self, history: &History, id: &VersionId) -> Result<(), String> {
        if self.graphs.contains_key(id) {
            return Ok(());
        }
        let graph = (history.inspect)(id)?;
        self.graphs.insert(id.clone(), graph);
        Ok(())
    }

    /// Paints the union at the current cut, leaving the camera where it is.
    /// Whatever the new picture no longer shows drops out of the selection.
    fn repaint(&mut self) {
        self.generation += 1;
        let generation = self.generation;
        let comparison = &self.comparison;
        let weights = &self.weights;
        let renames = &self.renames;
        let vocabulary = &self.cut.kinds;
        self.scene = boundary_view(comparison.union(), &self.cut)
            .map_err(|error| error.to_string())
            .map(|view| {
                let layout = layout::compute(&view.graph, weights, vocabulary, renames);
                Scene {
                    generation,
                    containment: Containment::of(&view.graph),
                    edges: edge_visuals(&view, comparison),
                    nodes: node_statuses(&view.graph, comparison),
                    world: canvas::world_bounds(&layout),
                    view,
                    layout,
                }
            });
        let dropped = self
            .selection
            .as_ref()
            .is_some_and(|selection| !self.shows_selection(selection));
        if dropped {
            self.selection = None;
        }
    }

    fn shows(&self, id: &ElementId) -> bool {
        self.scene
            .as_ref()
            .is_ok_and(|scene| scene.view.graph.element(id).is_some())
    }

    fn shows_selection(&self, selection: &Selection) -> bool {
        match selection {
            Selection::Node(id) => self.shows(id),
            Selection::Edge(relation) => self.shows(&relation.from) && self.shows(&relation.to),
        }
    }

    /// Carries out what a key or a toolbar button asked of the picture.
    fn obey(&mut self, request: vocabulary::Request) {
        match request {
            vocabulary::Request::Toggle(kind) => self.toggle_kind(kind),
            vocabulary::Request::OpenLayer => self.open_layer(),
            vocabulary::Request::CloseLayer => self.close_layer(),
        }
    }

    /// Adds one kind to the picture's vocabulary, or takes it out.
    fn toggle_kind(&mut self, kind: ElementKind) {
        if !self.cut.kinds.remove(&kind) {
            self.cut.kinds.insert(kind);
        }
        self.repaint();
    }

    /// Opens every boundary the picture offers to open.
    fn open_layer(&mut self) {
        let Ok(scene) = &self.scene else {
            return;
        };
        if self.cut.expand_frontier(&scene.view) {
            self.repaint();
        }
    }

    /// Closes the innermost open boundaries.
    fn close_layer(&mut self) {
        let Ok(scene) = &self.scene else {
            return;
        };
        if self.cut.collapse_frontier(&scene.view) {
            self.repaint();
        }
    }

    /// Opens the boundary, revealing one layer of its contents.
    fn expand(&mut self, id: &ElementId) {
        let Ok(scene) = &self.scene else {
            return;
        };
        if self.cut.expand(&scene.view, id) {
            self.repaint();
        }
    }

    /// Opens every boundary that hides a change, so each arrival and each
    /// departure shows at its own level rather than as amber on an
    /// ancestor. The vocabulary stands: a change of a hidden kind still
    /// reads on the nearest rendered boundary, because hiding the kind was
    /// the reader's own decision.
    fn focus_changes(&mut self) {
        let hiding = boundaries_hiding_changes(&self.comparison);
        let already = self.cut.open.len();
        self.cut.open.extend(hiding);
        if self.cut.open.len() > already {
            self.repaint();
        }
    }

    /// Opens the boundary and every boundary beneath it, down to the
    /// deepest frame. The walk reaches past what the picture draws, so it
    /// reads the union the lens cuts rather than the view cut from it.
    fn expand_fully(&mut self, id: &ElementId) {
        let Ok(scene) = &self.scene else {
            return;
        };
        if self
            .cut
            .expand_fully(&scene.view, self.comparison.union(), id)
        {
            self.repaint();
        }
    }

    /// Travels until the whole subject of a selection stands in front of
    /// the reader.
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

    /// Puts the whole picture back in front of the reader.
    fn refit(&mut self) {
        let (Some(_), Ok(scene)) = (self.camera.now(), &self.scene) else {
            return;
        };
        let world = scene.world;
        self.camera.fly(camera::fit(world, self.viewport));
    }

    /// Puts one element of the union in front of the reader: the
    /// boundaries above it open, its kind joins the vocabulary where the
    /// picture left it out, and the element becomes the selection the
    /// camera moves to. Where even that leaves it hidden, the nearest
    /// boundary above it answers instead.
    fn locate(&mut self, target: &ElementId) {
        if !self.shows(target) {
            for boundary in palette::boundaries_revealing(self.comparison.union(), target) {
                self.cut.open.insert(boundary);
            }
            // A kind joins the vocabulary only for an element no reading of
            // which the picture renders: one it already speaks for needs no
            // widening, and widening would change what everything else
            // draws as.
            if let Some(element) = self.comparison.union().element(target)
                && element.speaks_as(&self.cut.kinds).is_none()
            {
                self.cut.kinds.insert(element.primary_kind());
            }
            self.repaint();
        }
        let Ok(scene) = &self.scene else {
            return;
        };
        let found = focus::boundary_in_view(&scene.view.graph, self.comparison.union(), target);
        let Some(found) = found else {
            return;
        };
        let selection = Selection::Node(found);
        self.selection = Some(selection.clone());
        self.reveal(&selection);
    }

    fn handle(&mut self, ui: &egui::Ui, action: CanvasAction) {
        match action {
            CanvasAction::Node(id) => self.selection = Some(Selection::Node(id)),
            // Opening a boundary answers the question the reader just asked
            // of it, so the boundary stays selected under the new picture.
            CanvasAction::Expand(id) => {
                self.selection = Some(Selection::Node(id.clone()));
                if ui.input(|input| input.modifiers.shift) {
                    self.expand_fully(&id);
                } else {
                    self.expand(&id);
                }
            }
            CanvasAction::Edge(relation) => self.selection = Some(Selection::Edge(relation)),
            CanvasAction::Background => self.selection = None,
        }
    }
}

/// The pair a comparison opens on: the newest version against the one
/// before it. A project with a single version compares it with itself, and
/// the picture then says nothing changed - which is the truth about it.
/// None while the history lists no version at all.
fn default_pair(versions: &[Version]) -> Option<(VersionId, VersionId)> {
    let after = versions.first()?;
    let before = versions.get(1).unwrap_or(after);
    Some((before.id.clone(), after.id.clone()))
}

/// Every boundary standing above a change: the frames to open so each
/// arrival, each departure, and each element changed inside shows at its
/// own level. A changed dependency counts through both of its endpoints -
/// a connection drawn between two surviving boundaries is as much a change
/// as a boundary of its own.
fn boundaries_hiding_changes(comparison: &Comparison) -> BTreeSet<ElementId> {
    let containment = Containment::of(comparison.union());
    let delta = comparison.delta();
    let changed = delta
        .added_elements
        .iter()
        .chain(&delta.removed_elements)
        .chain(&delta.changed_elements)
        .map(|element| &element.id)
        .chain(
            delta
                .added_relations
                .iter()
                .chain(&delta.removed_relations)
                .filter(|relation| relation.kind == RelationKind::DependsOn)
                .flat_map(|relation| [&relation.from, &relation.to]),
        );
    let mut above = BTreeSet::new();
    for id in changed {
        let mut current = id;
        while let Some(frame) = containment.parent(current) {
            // A frame already collected brought its own ancestors with it,
            // and the same check bounds the walk should containment cycle.
            if !above.insert(frame.clone()) {
                break;
            }
            current = frame;
        }
    }
    above
}

/// How each box of the picture reads. A box neither version changed stays
/// out of the answer, so the canvas paints it as the architecture has it.
fn node_statuses(
    view: &ArchitectureGraph,
    comparison: &Comparison,
) -> BTreeMap<ElementId, NodeStatus> {
    let rendered: BTreeSet<ElementId> = view.elements().map(|element| element.id.clone()).collect();
    comparison
        .readings_at(&rendered)
        .into_iter()
        .map(|(id, change)| (id, paint_of(change)))
        .collect()
}

/// What one reading of a boundary paints as. The comparison's three
/// readings and the plan's three marks say the same three things - this
/// arrives, this goes, this stands and changes - so they wear one paint.
fn paint_of(change: ElementChange) -> NodeStatus {
    match change {
        ElementChange::Added => NodeStatus::Added,
        ElementChange::Removed => NodeStatus::Removed,
        ElementChange::Modified => NodeStatus::Modified,
    }
}

/// Every connection one frame draws, with what the two versions say about
/// each. A comparison carries no notes, so nothing is annotated.
fn edge_visuals(view: &BoundaryView, comparison: &Comparison) -> Vec<EdgeVisual> {
    view.provenance
        .keys()
        .map(|relation| {
            let concrete = view.concrete_behind(relation);
            EdgeVisual {
                relation: relation.clone(),
                status: status_of_group(comparison, &concrete),
                annotated: false,
                weight: concrete.len().max(1),
            }
        })
        .collect()
}

fn status_of_group(comparison: &Comparison, concrete: &BTreeSet<Relation>) -> EdgeStatus {
    let presences: Vec<Presence> = concrete
        .iter()
        .filter_map(|relation| comparison.presence_of_relation(relation))
        .collect();
    reading_of(&presences)
}

/// What a rendered connection says, read from where each dependency behind
/// it stands across the two versions.
///
/// A connection whose every dependency arrives is itself an arrival, and
/// one whose every dependency goes is itself a departure. A mix in which
/// anything goes wears the partial mark - the same mark a partly severed
/// connection wears in a plan, because both say "part of this leaves".
/// Everything else stands: dependencies added to a connection that already
/// ran do not change that it ran.
fn reading_of(presences: &[Presence]) -> EdgeStatus {
    let counted = |wanted: Presence| {
        presences
            .iter()
            .filter(|presence| **presence == wanted)
            .count()
    };
    let leaving = counted(Presence::OnlyBefore);
    let arriving = counted(Presence::OnlyAfter);
    let total = presences.len();
    if total > 0 && arriving == total {
        EdgeStatus::Drawn
    } else if total > 0 && leaving == total {
        EdgeStatus::Severed
    } else if leaving > 0 {
        EdgeStatus::PartiallySevered {
            severed: leaving,
            total,
        }
    } else {
        EdgeStatus::Existing
    }
}

/// What the toolbar offers about a comparison: which two versions it is
/// about, where the picture cuts them, the ways to move through it, and
/// what the pair itself is worth saying.
pub(crate) fn tools(ui: &mut egui::Ui, history: &History, session: &mut CompareSession) {
    ui.separator();
    let mut named_another = picker(ui, "Before", &history.versions, &mut session.before);
    named_another |= picker(ui, "After", &history.versions, &mut session.after);
    if named_another {
        session.retarget(history);
    }
    ui.separator();
    ui.label("Show");
    // Presence comes from the union, so a kind either version holds counts.
    let present = vocabulary::present_kinds(session.comparison.union());
    if let Some(kind) = vocabulary::chips(ui, &session.cut.kinds, &present) {
        session.toggle_kind(kind);
    }
    if let Some(request) = vocabulary::layer_buttons(ui) {
        session.obey(request);
    }
    if ui
        .add_enabled(
            !session.comparison.delta().is_empty(),
            egui::Button::new("Focus changes"),
        )
        .on_hover_text(
            "Open every boundary that hides a change, so each arrival, \
             each departure, and each change inside shows at its own level.",
        )
        .on_disabled_hover_text("These versions hold no change to focus")
        .clicked()
    {
        session.focus_changes();
        session.refit();
    }
    let open = session
        .scene
        .as_ref()
        .map(|scene| scene.view.open.len())
        .unwrap_or_default();
    if let Some(standing) = vocabulary::standing(open) {
        ui.label(egui::RichText::new(standing).weak().small());
    }
    ui.separator();
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

/// One version picker. Answers true where the reader named another version.
fn picker(ui: &mut egui::Ui, label: &str, versions: &[Version], chosen: &mut VersionId) -> bool {
    let mut named_another = false;
    egui::ComboBox::from_label(label)
        .selected_text(
            versions
                .iter()
                .find(|version| version.id == *chosen)
                .map_or_else(|| short(chosen), entry),
        )
        .show_ui(ui, |ui| {
            for version in versions {
                let picked = ui
                    .selectable_value(chosen, version.id.clone(), entry(version))
                    // The short id names the version everywhere; the whole
                    // one is what a reader pastes into their own tools.
                    .on_hover_text(version.id.as_str());
                named_another |= picked.changed();
            }
        });
    named_another
}

/// How one version reads in a picker: the short id the reader knows from
/// their own log, and what the version says about itself.
fn entry(version: &Version) -> String {
    let short = short(&version.id);
    if version.summary.is_empty() {
        short
    } else {
        format!("{short}  {}", version.summary)
    }
}

fn short(id: &VersionId) -> String {
    id.as_str().chars().take(SHORT_ID).collect()
}

/// The panel beside the comparison: what changed, about the whole picture
/// or about the one thing the reader selected. It only reads - a
/// comparison states what happened, and nothing here can answer back.
pub(crate) fn panel(ui: &mut egui::Ui, session: &CompareSession) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.heading(format!(
            "From {} to {}",
            short(&session.before),
            short(&session.after)
        ));
        match &session.selection {
            None => whole_story(ui, session),
            Some(Selection::Node(id)) => boundary(ui, session, id),
            Some(Selection::Edge(relation)) => connection(ui, session, relation),
        }
    });
}

fn whole_story(ui: &mut egui::Ui, session: &CompareSession) {
    let delta = session.comparison.delta();
    let dependencies = |relations: &[Relation]| {
        relations
            .iter()
            .filter(|relation| relation.kind == RelationKind::DependsOn)
            .count()
    };
    ui.label(format!(
        "{} boundaries arrive, {} leave, {} change inside; {} dependencies arrive, {} leave.",
        delta.added_elements.len(),
        delta.removed_elements.len(),
        delta.changed_elements.len(),
        dependencies(&delta.added_relations),
        dependencies(&delta.removed_relations),
    ));
    ui.separator();
    ui.label("Green arrives, red leaves, amber stands in both versions and changes inside.");
    ui.label("Select a boundary or a connection to read what happened to it.");
}

fn boundary(ui: &mut egui::Ui, session: &CompareSession, id: &ElementId) {
    let Ok(scene) = &session.scene else {
        return;
    };
    let labels = Labels::over(
        &scene.view.graph,
        &scene.containment,
        &session.cut.kinds,
        &session.renames,
    );
    ui.label(labels.qualified(id));
    if let Some(element) = scene.view.graph.element(id) {
        // Every reading of the boundary, the one the picture speaks it
        // under first. A boundary changes by either of them, so a panel
        // that named one alone would leave half the story out.
        ui.label(egui::RichText::new(label::readings(element, &labels.source_name(id))).weak());
    }
    ui.separator();
    ui.label(told(scene.nodes.get(id).copied()));
}

/// What one box says about itself, in words.
fn told(status: Option<NodeStatus>) -> &'static str {
    match status {
        Some(NodeStatus::Added) => "Arrives: only the newer version has it.",
        Some(NodeStatus::Removed) => "Leaves: only the older version has it.",
        Some(NodeStatus::Modified) => {
            // A boundary reads as two things at once, so either reading
            // changing is the boundary changing: a crate that moved keeps
            // its name and stands somewhere else in the tree.
            "Changes inside: it stands in both versions, and something beneath it moved - \
             or the boundary itself changed: its name, its kind, where it lies in the \
             tree, or its contents."
        }
        None => "Unchanged.",
    }
}

fn connection(ui: &mut egui::Ui, session: &CompareSession, relation: &Relation) {
    let Ok(scene) = &session.scene else {
        return;
    };
    let labels = Labels::over(
        &scene.view.graph,
        &scene.containment,
        &session.cut.kinds,
        &session.renames,
    );
    ui.label(format!(
        "{} {} {}",
        labels.qualified(&relation.from),
        glyph::OUTWARD,
        labels.qualified(&relation.to)
    ));
    ui.separator();
    let presences: Vec<Presence> = scene
        .view
        .concrete_behind(relation)
        .iter()
        .filter_map(|concrete| session.comparison.presence_of_relation(concrete))
        .collect();
    let counted = |wanted: Presence| {
        presences
            .iter()
            .filter(|presence| **presence == wanted)
            .count()
    };
    ui.label(format!(
        "{} dependencies arrive, {} leave, {} stay.",
        counted(Presence::OnlyAfter),
        counted(Presence::OnlyBefore),
        counted(Presence::InBoth),
    ));
}

/// The boundary canvas of a comparison, and what a click on it does.
pub(crate) fn picture(ui: &mut egui::Ui, session: &mut CompareSession) {
    session.viewport = ui.max_rect();
    let scene = match &session.scene {
        Err(reason) => {
            ui.colored_label(ui.visuals().error_fg_color, reason.as_str());
            return;
        }
        Ok(scene) => scene,
    };
    let (selected_edge, selected_node) = match &session.selection {
        Some(Selection::Edge(relation)) => (Some(relation), None),
        Some(Selection::Node(id)) => (None, Some(id)),
        None => (None, None),
    };
    let action = canvas::show(
        ui,
        &Content {
            generation: scene.generation,
            view: &scene.view.graph,
            layout: &scene.layout,
            containment: &scene.containment,
            world: scene.world,
            edges: &scene.edges,
            nodes: &scene.nodes,
            vocabulary: &session.cut.kinds,
            renames: &session.renames,
            selected_edge,
            selected_node,
            // Nothing is drawn in a comparison; there is no source to pick.
            draw_source: None,
        },
        &mut session.camera,
        &mut session.strokes,
    );
    if let Some(action) = action {
        session.handle(ui, action);
    }
}

/// The search and the keys that stand over the comparison. Escape is not
/// among them: nothing here waits for a click, and nothing scopes the
/// picture, so there is nothing to step out of.
pub(crate) fn overlay(ctx: &egui::Context, session: &mut CompareSession) {
    let found = palette::show(
        ctx,
        &mut session.palette,
        session.comparison.union(),
        &session.cut.kinds,
        session.viewport,
    );
    if let Some(target) = found {
        session.locate(&target);
    }
    // The keys reach the picture only after the palette had every one of
    // this frame: an open palette answers to keys of its own.
    if session.palette.is_open() {
        return;
    }
    if let Some(request) =
        vocabulary::requested(ctx, &vocabulary::present_kinds(session.comparison.union()))
    {
        session.obey(request);
    }
    if camera::refit_requested(ctx) {
        session.refit();
    }
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{Element, ElementName, SemanticKind};

    use super::*;

    fn wired(
        ids: &[&str],
        contains: &[(&str, &str)],
        depends: &[(&str, &str)],
    ) -> ArchitectureGraph {
        let mut graph = ArchitectureGraph::new();
        for id in ids {
            graph
                .add_element(Element::semantic(
                    ElementId::new(*id).unwrap(),
                    SemanticKind::Module,
                    ElementName::new(*id).unwrap(),
                ))
                .unwrap();
        }
        let relation = |from: &str, to: &str, kind| Relation {
            from: ElementId::new(from).unwrap(),
            to: ElementId::new(to).unwrap(),
            kind,
        };
        for (from, to) in contains {
            graph
                .add_relation(relation(from, to, RelationKind::Contains))
                .unwrap();
        }
        for (from, to) in depends {
            graph
                .add_relation(relation(from, to, RelationKind::DependsOn))
                .unwrap();
        }
        graph
    }

    fn opened(before: &ArchitectureGraph, after: &ArchitectureGraph) -> BTreeSet<ElementId> {
        boundaries_hiding_changes(&Comparison::between(before, after))
    }

    fn ids(ids: &[&str]) -> BTreeSet<ElementId> {
        ids.iter().map(|id| ElementId::new(*id).unwrap()).collect()
    }

    #[test]
    fn focusing_changes_opens_every_boundary_above_an_arrival() {
        let held = &[("root", "pkg"), ("pkg", "old")];
        let before = wired(&["root", "pkg", "old"], held, &[]);
        let after = wired(
            &["root", "pkg", "old", "new"],
            &[("root", "pkg"), ("pkg", "old"), ("pkg", "new")],
            &[],
        );
        assert_eq!(opened(&before, &after), ids(&["root", "pkg"]));
    }

    #[test]
    fn focusing_changes_opens_the_boundaries_that_held_a_departure() {
        let before = wired(
            &["root", "pkg", "old", "gone"],
            &[("root", "pkg"), ("pkg", "old"), ("pkg", "gone")],
            &[],
        );
        let after = wired(
            &["root", "pkg", "old"],
            &[("root", "pkg"), ("pkg", "old")],
            &[],
        );
        assert_eq!(opened(&before, &after), ids(&["root", "pkg"]));
    }

    #[test]
    fn focusing_changes_opens_the_boundaries_above_both_ends_of_a_drawn_dependency() {
        let elements = &["root", "a", "b", "a/x", "b/y"];
        let held = &[("root", "a"), ("root", "b"), ("a", "a/x"), ("b", "b/y")];
        let before = wired(elements, held, &[]);
        let after = wired(elements, held, &[("a/x", "b/y")]);
        assert_eq!(opened(&before, &after), ids(&["root", "a", "b"]));
    }

    #[test]
    fn focusing_changes_opens_nothing_where_nothing_changed() {
        let graph = wired(&["root", "pkg"], &[("root", "pkg")], &[]);
        assert!(opened(&graph, &graph).is_empty());
    }

    fn version(id: &str) -> Version {
        Version {
            id: VersionId::new(id).unwrap(),
            summary: format!("what {id} did"),
        }
    }

    fn pair(ids: &[&str]) -> Option<(String, String)> {
        let versions: Vec<Version> = ids.iter().map(|id| version(id)).collect();
        default_pair(&versions).map(|(before, after)| (before.to_string(), after.to_string()))
    }

    #[test]
    fn a_comparison_opens_on_the_newest_version_against_the_one_before_it() {
        assert_eq!(
            pair(&["third", "second", "first"]),
            Some(("second".to_owned(), "third".to_owned()))
        );
    }

    #[test]
    fn a_project_with_a_single_version_compares_it_with_itself() {
        assert_eq!(
            pair(&["only"]),
            Some(("only".to_owned(), "only".to_owned()))
        );
    }

    #[test]
    fn a_project_with_no_version_offers_no_pair() {
        assert_eq!(pair(&[]), None);
    }

    #[test]
    fn a_connection_whose_every_dependency_arrives_reads_as_arriving() {
        assert_eq!(
            reading_of(&[Presence::OnlyAfter, Presence::OnlyAfter]),
            EdgeStatus::Drawn
        );
    }

    #[test]
    fn a_connection_whose_every_dependency_leaves_reads_as_leaving() {
        assert_eq!(
            reading_of(&[Presence::OnlyBefore, Presence::OnlyBefore]),
            EdgeStatus::Severed
        );
    }

    #[test]
    fn a_connection_losing_part_of_its_dependencies_wears_the_partial_mark() {
        assert_eq!(
            reading_of(&[Presence::InBoth, Presence::OnlyBefore, Presence::OnlyAfter]),
            EdgeStatus::PartiallySevered {
                severed: 1,
                total: 3
            }
        );
    }

    #[test]
    fn a_connection_that_only_gains_dependencies_still_reads_as_standing() {
        assert_eq!(
            reading_of(&[Presence::InBoth, Presence::OnlyAfter]),
            EdgeStatus::Existing,
            "the connection already ran, and more traffic on it does not make it new"
        );
    }

    #[test]
    fn a_connection_neither_version_touched_reads_as_standing() {
        assert_eq!(
            reading_of(&[Presence::InBoth, Presence::InBoth]),
            EdgeStatus::Existing
        );
        assert_eq!(reading_of(&[]), EdgeStatus::Existing);
    }

    #[test]
    fn a_boundary_that_arrives_paints_as_an_addition() {
        assert_eq!(paint_of(ElementChange::Added), NodeStatus::Added);
    }

    #[test]
    fn a_boundary_that_leaves_paints_as_a_removal() {
        assert_eq!(paint_of(ElementChange::Removed), NodeStatus::Removed);
    }

    #[test]
    fn a_boundary_that_changes_inside_paints_as_a_modification() {
        assert_eq!(paint_of(ElementChange::Modified), NodeStatus::Modified);
    }
}
