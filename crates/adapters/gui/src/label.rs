//! What a box on the canvas says.
//!
//! A box names one boundary. A kind glyph in front of a leaf says what the
//! boundary is, so a function never reads like a type; a frame needs no
//! glyph, because its form already says that it holds other boundaries. The
//! name drops the path the frame around it already spells, so a module
//! inside its parent module reads as its own segment alone. The inspector
//! still names every boundary in full; only the picture shortens.
//!
//! A plan renames elements, and a name is what the reader recognises a
//! boundary by: a renamed boundary therefore reads "old → new" wherever its
//! name paints, in a box and in the panel alike. The rename enters here,
//! once, rather than at each of the places that write a name.
//!
//! Layout measures the label the canvas paints, so both ask this module and
//! no box is ever too narrow for its text.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{ArchitectureGraph, ElementId, ElementKind};
use cutaway_planning::{ModificationKind, Plan};

use crate::focus::Containment;
use crate::glyph;

/// The text of one box: the name, and the kind glyph in front of it when
/// the glyph tells the reader something.
pub(crate) struct Label {
    pub(crate) glyph: Option<&'static str>,
    pub(crate) name: String,
}

impl Label {
    /// Glyph and name as one string: the extent a box must offer its label.
    pub(crate) fn text(&self) -> String {
        match self.glyph {
            Some(glyph) => format!("{glyph} {}", self.name),
            None => self.name.clone(),
        }
    }
}

/// The new names a plan gives elements, by the element each renames.
#[derive(Debug, Default)]
pub(crate) struct Renames(BTreeMap<ElementId, String>);

/// No rename at all: what labels read by when nothing feeds them a plan.
static NONE: Renames = Renames(BTreeMap::new());

impl Renames {
    pub(crate) fn of(plan: &Plan) -> Self {
        Self(
            plan.modifications()
                .filter_map(|modification| match &modification.kind {
                    ModificationKind::Rename { to } => {
                        Some((modification.subject.clone(), to.to_string()))
                    }
                    _ => None,
                })
                .collect(),
        )
    }

    fn new_name(&self, id: &ElementId) -> Option<&str> {
        self.0.get(id).map(String::as_str)
    }
}

/// The labels of one view. A name reads against the frame around it, so the
/// containment resolves once and every box then answers directly.
pub(crate) struct Labels<'a> {
    view: &'a ArchitectureGraph,
    containment: Cow<'a, Containment>,
    renames: &'a Renames,
}

impl<'a> Labels<'a> {
    /// The labels of a view read as the sources name it.
    pub(crate) fn of(view: &'a ArchitectureGraph) -> Self {
        Self::renaming(view, &NONE)
    }

    /// The labels of a view with a plan's renames written into them.
    pub(crate) fn renaming(view: &'a ArchitectureGraph, renames: &'a Renames) -> Self {
        Self {
            view,
            containment: Cow::Owned(Containment::of(view)),
            renames,
        }
    }

    /// The same labels over a containment the caller already resolved for
    /// this very view. A picture asks for its labels every frame and its
    /// containment stands between rebuilds, so the labels borrow it instead
    /// of walking the view again.
    pub(crate) fn over(
        view: &'a ArchitectureGraph,
        containment: &'a Containment,
        renames: &'a Renames,
    ) -> Self {
        Self {
            view,
            containment: Cow::Borrowed(containment),
            renames,
        }
    }

    pub(crate) fn label(&self, id: &ElementId) -> Label {
        Label {
            glyph: self.glyph(id),
            name: self.name(id),
        }
    }

    /// The name a text beside the picture gives a boundary: the whole name,
    /// nothing shortened, because only a box is short of room.
    pub(crate) fn qualified(&self, id: &ElementId) -> String {
        self.full_name(id)
    }

    /// The names the rows of one list carry. Rows stand side by side, so
    /// two boundaries that read alike read as one entry: the list asks for
    /// all of its names at once and gets them told apart.
    pub(crate) fn distinct<'b>(&self, ids: impl IntoIterator<Item = &'b ElementId>) -> Distinct {
        // The same boundary may fill several rows of one list; it is one
        // name, and a name never has to stand apart from itself.
        let subjects: BTreeSet<&ElementId> = ids.into_iter().collect();
        let entries: Vec<Entry> = subjects
            .into_iter()
            .map(|id| Entry {
                id: id.clone(),
                name: self.qualified(id),
                above: self.above(id),
            })
            .collect();
        Distinct {
            names: distinguish(&entries),
        }
    }

    /// The boundaries above the one a name speaks of, nearest first. The
    /// project holds the whole picture and therefore tells nothing apart;
    /// it stays out.
    fn above(&self, id: &ElementId) -> Vec<String> {
        let mut names = Vec::new();
        // Containment is a tree, but a walk that trusts that and meets a
        // cycle never ends; the seen set bounds it.
        let mut seen = BTreeSet::new();
        let mut current = self.containment.parent(id);
        while let Some(frame) = current {
            if !seen.insert(frame) {
                break;
            }
            let project = self
                .view
                .element(frame)
                .is_some_and(|element| element.kind == ElementKind::Project);
            if !project {
                names.push(self.full_name(frame));
            }
            current = self.containment.parent(frame);
        }
        names
    }

    /// The kind mark a box carries. A frame carries none: the box around
    /// its children already says what it is.
    fn glyph(&self, id: &ElementId) -> Option<&'static str> {
        if self.containment.is_frame(id) {
            return None;
        }
        self.view
            .element(id)
            .map(|element| kind_symbol(element.kind))
    }

    fn name(&self, id: &ElementId) -> String {
        let full = self.full_name(id);
        match self.containment.parent(id) {
            // The path a box drops is the one the sources spell; a frame's
            // new name is a name nothing below it carries yet.
            Some(frame) => contextual(&full, &self.source_name(frame)).to_owned(),
            None => full,
        }
    }

    /// The whole name of a boundary, the plan's rename included: a renamed
    /// boundary reads "old → new", because the reader knows it by the name
    /// the sources carry and must see what it becomes.
    fn full_name(&self, id: &ElementId) -> String {
        let name = self.source_name(id);
        match self.renames.new_name(id) {
            Some(new) => format!("{name} {} {new}", glyph::BECOMES),
            None => name,
        }
    }

    /// The name the sources give a boundary, whatever the plan says of it.
    fn source_name(&self, id: &ElementId) -> String {
        self.view
            .element(id)
            .map_or_else(|| id.to_string(), |element| element.name.to_string())
    }
}

/// The names of one list, each standing apart from the rest of that list.
///
/// A panel lists rows about boundaries from all over the picture, where two
/// short names collide freely: three crates each hold a module named
/// `crate`, and three rows reading `crate` name nothing. A name another row
/// of the same list shares therefore gains the boundaries above it, one
/// step outward at a time, until the rows read apart. A name no other row
/// shares stays as short as it was.
pub(crate) struct Distinct {
    names: BTreeMap<ElementId, String>,
}

impl Distinct {
    /// The name one row of the list carries. An id the list was not built
    /// from answers with its own text, so a row always reads.
    pub(crate) fn name<'a>(&'a self, id: &'a ElementId) -> &'a str {
        self.names
            .get(id)
            .map_or_else(|| id.as_str(), String::as_str)
    }
}

/// One subject of a list: the name it carries while nothing collides with
/// it, and the boundaries above it, nearest first, to fall back on.
struct Entry {
    id: ElementId,
    name: String,
    above: Vec<String>,
}

impl Entry {
    /// The name behind its nearest `steps` boundaries, outermost first, as
    /// a path through the picture reads.
    fn text(&self, steps: usize) -> String {
        self.above[..steps]
            .iter()
            .rev()
            .map(String::as_str)
            .chain([self.name.as_str()])
            .collect::<Vec<&str>>()
            .join(glyph::CONTAINER_STEP)
    }
}

/// Lengthens every name another entry shares by one boundary at a time,
/// and stops when the names stand apart or no entry has a boundary left to
/// add. Two boundaries of one name in one frame cannot be told apart at
/// all; they keep the shortest name that says everything known about them.
fn distinguish(entries: &[Entry]) -> BTreeMap<ElementId, String> {
    let mut steps = vec![0usize; entries.len()];
    loop {
        let mut shared: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, entry) in entries.iter().enumerate() {
            shared
                .entry(entry.text(steps[index]))
                .or_default()
                .push(index);
        }
        let mut grew = false;
        for group in shared.values().filter(|group| group.len() > 1) {
            for index in group {
                if steps[*index] < entries[*index].above.len() {
                    steps[*index] += 1;
                    grew = true;
                }
            }
        }
        if !grew {
            return shared
                .into_iter()
                .flat_map(|(text, group)| {
                    group
                        .into_iter()
                        .map(move |index| (entries[index].id.clone(), text.clone()))
                })
                .collect();
        }
    }
}

/// The part of a name its frame does not already spell. A name that carries
/// a path repeats the whole path, and the frame beside it repeats it again:
/// the box shows the segment below the frame instead. The stripping applies
/// at every level, because each frame drops its own frame's path in turn.
/// A name that merely starts like its frame keeps all of its text.
fn contextual<'a>(name: &'a str, frame: &str) -> &'a str {
    let Some(rest) = name.strip_prefix(frame) else {
        return name;
    };
    // Only punctuation joins a path to the segment below it, whatever the
    // source language writes: nothing else may fall away here.
    let segment = rest.trim_start_matches(|character: char| !name_character(character));
    if segment.is_empty() || segment.len() == rest.len() {
        return name;
    }
    segment
}

fn name_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

pub(crate) fn kind_symbol(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::Project => glyph::PROJECT,
        ElementKind::Package => glyph::PACKAGE,
        ElementKind::Directory => glyph::DIRECTORY,
        ElementKind::Module => glyph::MODULE,
        ElementKind::Function => glyph::FUNCTION,
        ElementKind::Type => glyph::TYPE,
    }
}

/// What the glyph stands for, in words. A picture has room for a mark
/// alone; a panel beside it has room to say the kind outright.
pub(crate) fn kind_name(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::Project => "Project",
        ElementKind::Package => "Package",
        ElementKind::Directory => "Directory",
        ElementKind::Module => "Module",
        ElementKind::Function => "Function",
        ElementKind::Type => "Type",
    }
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{Element, ElementName, Relation, RelationKind};

    use super::*;

    fn id(text: &str) -> ElementId {
        ElementId::new(text).unwrap()
    }

    fn add(graph: &mut ArchitectureGraph, id_text: &str, name: &str, kind: ElementKind) {
        graph
            .add_element(Element {
                id: id(id_text),
                name: ElementName::new(name).unwrap(),
                kind,
            })
            .unwrap();
    }

    fn contain(graph: &mut ArchitectureGraph, frame: &str, inner: &str) {
        graph
            .add_relation(Relation {
                from: id(frame),
                to: id(inner),
                kind: RelationKind::Contains,
            })
            .unwrap();
    }

    /// A module `ports` inside a package, holding the module
    /// `ports::source_analyzer` and a function of its own.
    fn view() -> ArchitectureGraph {
        let mut graph = ArchitectureGraph::new();
        add(&mut graph, "package:core", "core", ElementKind::Package);
        add(&mut graph, "core/ports.rs", "ports", ElementKind::Module);
        add(
            &mut graph,
            "core/ports/source_analyzer.rs",
            "ports::source_analyzer",
            ElementKind::Module,
        );
        add(
            &mut graph,
            "core/ports/source_analyzer.rs#function:analyze",
            "ports::source_analyzer::analyze",
            ElementKind::Function,
        );
        contain(&mut graph, "package:core", "core/ports.rs");
        contain(&mut graph, "core/ports.rs", "core/ports/source_analyzer.rs");
        contain(
            &mut graph,
            "core/ports/source_analyzer.rs",
            "core/ports/source_analyzer.rs#function:analyze",
        );
        graph
    }

    #[test]
    fn a_name_drops_the_path_the_frame_around_it_already_spells() {
        let view = view();
        let labels = Labels::of(&view);
        assert_eq!(
            labels.label(&id("core/ports/source_analyzer.rs")).name,
            "source_analyzer"
        );
        assert_eq!(
            labels
                .label(&id("core/ports/source_analyzer.rs#function:analyze"))
                .name,
            "analyze",
            "every level drops the path of the level above it"
        );
    }

    #[test]
    fn a_name_that_merely_starts_like_its_frame_stays_whole() {
        let mut view = ArchitectureGraph::new();
        add(&mut view, "package:source", "source", ElementKind::Package);
        add(
            &mut view,
            "source/git.rs",
            "source_git",
            ElementKind::Module,
        );
        contain(&mut view, "package:source", "source/git.rs");

        let labels = Labels::of(&view);
        assert_eq!(labels.label(&id("source/git.rs")).name, "source_git");
    }

    #[test]
    fn a_leaf_shows_its_kind_and_a_frame_does_not() {
        let view = view();
        let labels = Labels::of(&view);
        assert_eq!(
            labels
                .label(&id("core/ports/source_analyzer.rs#function:analyze"))
                .glyph,
            Some(kind_symbol(ElementKind::Function))
        );
        assert_eq!(labels.label(&id("core/ports.rs")).glyph, None);
        assert_eq!(labels.label(&id("package:core")).glyph, None);
    }

    fn renaming(subject: &str, to: &str) -> Renames {
        let mut plan = cutaway_planning::Plan::new();
        plan.plan_modification(cutaway_planning::Modification {
            subject: id(subject),
            kind: ModificationKind::Rename {
                to: ElementName::new(to).unwrap(),
            },
            note: None,
        });
        Renames::of(&plan)
    }

    #[test]
    fn a_renamed_boundary_reads_as_the_name_it_becomes() {
        let view = view();
        let renames = renaming("core/ports/source_analyzer.rs", "parsing");
        let labels = Labels::renaming(&view, &renames);
        assert_eq!(
            labels.label(&id("core/ports/source_analyzer.rs")).name,
            format!("source_analyzer {} parsing", glyph::BECOMES),
            "the box still drops the path its frame spells"
        );
        assert_eq!(
            labels.qualified(&id("core/ports/source_analyzer.rs")),
            format!("ports::source_analyzer {} parsing", glyph::BECOMES)
        );
    }

    #[test]
    fn a_renamed_frame_leaves_the_names_inside_it_as_short_as_they_were() {
        let view = view();
        let renames = renaming("core/ports.rs", "wiring");
        let labels = Labels::renaming(&view, &renames);
        assert_eq!(
            labels.label(&id("core/ports/source_analyzer.rs")).name,
            "source_analyzer",
            "the path a box drops is the one the sources spell"
        );
        assert_eq!(
            labels.qualified(&id("core/ports.rs")),
            format!("ports {} wiring", glyph::BECOMES),
            "a text beside the picture names the frame rename included"
        );
    }

    #[test]
    fn a_name_beside_the_picture_keeps_the_path_a_box_drops() {
        let view = view();
        let labels = Labels::of(&view);
        assert_eq!(
            labels.qualified(&id("core/ports/source_analyzer.rs")),
            "ports::source_analyzer"
        );
    }

    /// Two packages that each hold a module named `crate`, as two Rust crate
    /// roots appear side by side, and one module no other package repeats.
    fn crates() -> ArchitectureGraph {
        let mut graph = ArchitectureGraph::new();
        for (package, root) in [
            ("cutaway-gui", "gui/lib.rs"),
            ("cutaway-lenses", "lenses/lib.rs"),
        ] {
            let package_id = format!("package:{package}");
            add(&mut graph, &package_id, package, ElementKind::Package);
            add(&mut graph, root, "crate", ElementKind::Module);
            contain(&mut graph, &package_id, root);
        }
        add(&mut graph, "gui/label.rs", "label", ElementKind::Module);
        contain(&mut graph, "package:cutaway-gui", "gui/label.rs");
        graph
    }

    #[test]
    fn a_name_repeated_in_a_list_gains_its_container() {
        let view = crates();
        let labels = Labels::of(&view);
        let listed = [id("gui/lib.rs"), id("lenses/lib.rs")];
        let names = labels.distinct(&listed);
        assert_eq!(
            names.name(&listed[0]),
            ["cutaway-gui", "crate"].join(glyph::CONTAINER_STEP)
        );
        assert_eq!(
            names.name(&listed[1]),
            ["cutaway-lenses", "crate"].join(glyph::CONTAINER_STEP)
        );
    }

    #[test]
    fn a_unique_name_stays_short() {
        let view = crates();
        let labels = Labels::of(&view);
        let listed = [id("gui/lib.rs"), id("lenses/lib.rs"), id("gui/label.rs")];
        let names = labels.distinct(&listed);
        assert_eq!(names.name(&listed[2]), "label");
    }

    #[test]
    fn a_measured_label_carries_the_glyph_the_canvas_paints() {
        let view = view();
        let labels = Labels::of(&view);
        let leaf = labels.label(&id("core/ports/source_analyzer.rs#function:analyze"));
        assert_eq!(leaf.text(), format!("{} analyze", glyph::FUNCTION));
    }
}
