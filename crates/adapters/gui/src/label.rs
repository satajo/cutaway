//! What a box on the canvas says.
//!
//! A box names one boundary. A kind glyph in front of a leaf says what the
//! boundary is, so a function never reads like a type; a frame needs no
//! glyph, because its form already says that it holds other boundaries. The
//! name drops the path the frame around it already spells, so a module
//! inside its parent module reads as its own segment alone. The inspector
//! still names every boundary in full; only the picture shortens.
//!
//! A boundary may carry two readings of itself - the module `element` is
//! the file `element.rs` - and the vocabulary of the picture decides which
//! of them speaks: the language's reading while its kind is rendered, the
//! tree's otherwise. The kind glyph says the reading the boundary speaks
//! under and says nothing else. The name follows that reading too, until a
//! boundary beside it says the very same name: then the name alone borrows
//! the other reading's - the module `index` reads as `a/index.ts` - while
//! the glyph goes on saying what the boundary is.
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

use cutaway_architecture::{ArchitectureGraph, Element, ElementId, ElementKind, ElementName};
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

/// The reading a picture gives one node: the aspect its vocabulary renders,
/// and - where it renders neither - what the node reads as by default.
///
/// A node no reading of which the vocabulary renders is transparent and
/// draws no box at all, so nothing on the canvas asks this of one. A list
/// beside the picture does: the search reaches every element of the
/// architecture, drawn or not, and an answer without a name would name
/// nothing.
pub(crate) fn spoken<'a>(
    element: &'a Element,
    vocabulary: &BTreeSet<ElementKind>,
) -> (ElementKind, &'a ElementName) {
    element
        .speaks_as(vocabulary)
        .unwrap_or_else(|| (element.primary_kind(), element.primary_name()))
}

/// Every reading of one boundary, for the line under a panel's heading:
/// what a language read there, what the tree holds there, or both where the
/// boundary is one thing read two ways.
///
/// A reading whose name the heading already spells says its kind alone -
/// repeating the heading tells the reader nothing. The other says its name
/// too, because that name is the half of the boundary the heading left out:
/// `Module element · File` under the heading `element.rs`.
pub(crate) fn readings(element: &Element, heading: &str) -> String {
    let named = |kind: ElementKind, name: &ElementName| {
        if name.as_str() == heading {
            kind_name(kind).to_owned()
        } else {
            format!("{} {name}", kind_name(kind))
        }
    };
    let semantic = element
        .semantic_aspect()
        .map(|aspect| named(aspect.kind.into(), &aspect.name));
    let substrate = element
        .substrate_aspect()
        .map(|aspect| named(aspect.kind.into(), &aspect.name));
    [semantic, substrate]
        .into_iter()
        .flatten()
        .collect::<Vec<String>>()
        .join(glyph::READING_STEP)
}

/// The names one node falls back through while a boundary beside it says
/// the same: the aspect the vocabulary speaks, then the place in the tree
/// the node stands at, and last the id.
///
/// The place carries the segments of every directory that dissolved into
/// it - `a/index.ts` beside `b/index.ts` - so two places in one frame never
/// read alike. The id can be shared by nothing at all, which is why the
/// ladder ends there.
fn ladder<'a>(element: &'a Element, vocabulary: &BTreeSet<ElementKind>) -> Vec<&'a str> {
    let spoken = spoken(element, vocabulary).1.as_str();
    let place = element
        .substrate_aspect()
        .map(|aspect| aspect.name.as_str())
        .filter(|place| *place != spoken);
    [Some(spoken), place, Some(element.id.as_str())]
        .into_iter()
        .flatten()
        .collect()
}

/// Where a name is written, and therefore how far down its ladder it may
/// fall to stand apart from a namesake beside it.
#[derive(Clone, Copy)]
enum Voice {
    /// On a box of the picture. A box stops at the place in the tree: an id
    /// is a path a reader knows no boundary by, and a box wide enough for
    /// one crowds out the picture around it. Two namesakes a box cannot
    /// tell apart therefore stay namesakes on the canvas.
    Painted,
    /// In a text beside the picture, where the reader selects one boundary
    /// and must be able to say which: the whole ladder stands, id included.
    Written,
}

/// Every boundary a namesake in its own frame pushes off the name its
/// reading gives, with the rung of its ladder that stands apart. A boundary
/// no namesake stands beside is absent, so the map is empty for most views.
///
/// One pass over the view answers for every label of it. A picture asks for
/// its labels once per painted frame, and a box that read its neighbours
/// itself would ask the same question once per box.
fn told_apart(
    view: &ArchitectureGraph,
    containment: &Containment,
    vocabulary: &BTreeSet<ElementKind>,
) -> BTreeMap<ElementId, usize> {
    let mut apart = BTreeMap::new();
    for frame in containment.frames() {
        let ladders: Vec<(&ElementId, Vec<&str>)> = containment
            .children(frame)
            .iter()
            .filter_map(|id| Some((id, ladder(view.element(id)?, vocabulary))))
            .collect();
        for (id, mine) in &ladders {
            let mut rung = mine.len() - 1;
            for (step, name) in mine.iter().enumerate() {
                let shared = ladders
                    .iter()
                    .any(|(other, theirs)| *other != *id && theirs.get(step) == Some(name));
                if !shared {
                    rung = step;
                    break;
                }
            }
            if rung > 0 {
                apart.insert((*id).clone(), rung);
            }
        }
    }
    apart
}

/// The labels of one view. A name reads against the frame around it, so the
/// containment resolves once and every box then answers directly.
pub(crate) struct Labels<'a> {
    view: &'a ArchitectureGraph,
    containment: Cow<'a, Containment>,
    /// The kinds the picture renders, which is what decides the reading
    /// every name, glyph and kind of these labels speaks.
    vocabulary: &'a BTreeSet<ElementKind>,
    renames: &'a Renames,
    /// How far down its ladder each boundary a namesake stands beside had
    /// to go, resolved once for the whole view.
    apart: BTreeMap<ElementId, usize>,
}

impl<'a> Labels<'a> {
    /// The labels of a view read as the sources name it.
    pub(crate) fn of(view: &'a ArchitectureGraph, vocabulary: &'a BTreeSet<ElementKind>) -> Self {
        Self::renaming(view, vocabulary, &NONE)
    }

    /// The labels of a view with a plan's renames written into them.
    pub(crate) fn renaming(
        view: &'a ArchitectureGraph,
        vocabulary: &'a BTreeSet<ElementKind>,
        renames: &'a Renames,
    ) -> Self {
        let containment = Containment::of(view);
        Self {
            apart: told_apart(view, &containment, vocabulary),
            view,
            containment: Cow::Owned(containment),
            vocabulary,
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
        vocabulary: &'a BTreeSet<ElementKind>,
        renames: &'a Renames,
    ) -> Self {
        Self {
            view,
            containment: Cow::Borrowed(containment),
            vocabulary,
            renames,
            apart: told_apart(view, containment, vocabulary),
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
                .is_some_and(|element| element.primary_kind() == ElementKind::Project);
            if !project {
                names.push(self.full_name(frame));
            }
            current = self.containment.parent(frame);
        }
        names
    }

    /// The kind mark a box carries: the mark of the reading the boundary
    /// speaks under, whatever name it had to borrow to stand apart from a
    /// namesake. A frame carries none: the box around its children already
    /// says what it is.
    fn glyph(&self, id: &ElementId) -> Option<&'static str> {
        if self.containment.is_frame(id) {
            return None;
        }
        self.view
            .element(id)
            .map(|element| kind_symbol(spoken(element, self.vocabulary).0))
    }

    fn name(&self, id: &ElementId) -> String {
        let painted = self.renamed(id, self.told(id, Voice::Painted));
        match self.containment.parent(id) {
            // The path a box drops is the one the sources spell; a frame's
            // new name is a name nothing below it carries yet.
            Some(frame) => contextual(&painted, &self.told(frame, Voice::Painted)).to_owned(),
            None => painted,
        }
    }

    /// The whole name of a boundary, the plan's rename included: a renamed
    /// boundary reads "old → new", because the reader knows it by the name
    /// the sources carry and must see what it becomes.
    fn full_name(&self, id: &ElementId) -> String {
        self.renamed(id, self.source_name(id))
    }

    /// A name with what the plan makes of it written after it.
    fn renamed(&self, id: &ElementId, name: String) -> String {
        match self.renames.new_name(id) {
            Some(new) => format!("{name} {} {new}", glyph::BECOMES),
            None => name,
        }
    }

    /// The name the sources give a boundary as a text beside the picture
    /// writes it, whatever the plan says of it: the reading the vocabulary
    /// speaks, and the place the boundary stands at wherever a boundary
    /// beside it speaks the very same name.
    ///
    /// TypeScript names a module after the stem of its file, so
    /// `src/a/index.ts` and `src/b/index.ts` both speak as `index`, and the
    /// single-child directories around them dissolve, which puts the two in
    /// one frame. Two entries reading `index` name neither, so each falls
    /// back to what the tree calls it - `a/index.ts` beside `b/index.ts`.
    /// Where even the places read alike the id stands: it is the one thing
    /// no two boundaries share, and a reader who has to pick one of them
    /// needs something that tells them apart.
    pub(crate) fn source_name(&self, id: &ElementId) -> String {
        self.told(id, Voice::Written)
    }

    /// The name one boundary reads under where it is written.
    fn told(&self, id: &ElementId, voice: Voice) -> String {
        let Some(element) = self.view.element(id) else {
            return id.to_string();
        };
        let rungs = ladder(element, self.vocabulary);
        let deepest = match voice {
            // The id is the last rung of every ladder, and a box paints no
            // id, so a painted name stops one short of the end.
            Voice::Painted => rungs.len().saturating_sub(2),
            Voice::Written => rungs.len() - 1,
        };
        rungs[self.apart.get(id).copied().unwrap_or_default().min(deepest)].to_owned()
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
/// add. What it resolves is namesakes of different frames: two boundaries
/// of one frame already read apart, because a name a boundary beside it
/// says too gives way to the place that boundary stands at, or to its id.
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
        ElementKind::File => glyph::FILE,
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
        ElementKind::File => "File",
        ElementKind::Function => "Function",
        ElementKind::Type => "Type",
    }
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{
        Element, ElementName, Relation, RelationKind, Semantic, SemanticKind, Substrate,
        SubstrateKind,
    };

    use super::*;

    fn id(text: &str) -> ElementId {
        ElementId::new(text).unwrap()
    }

    fn name(text: &str) -> ElementName {
        ElementName::new(text).unwrap()
    }

    /// The vocabulary a reader who has hidden nothing looks at. Labels
    /// borrow the vocabulary they speak under, so the set stands for the
    /// whole run rather than per call.
    static EVERYTHING: std::sync::LazyLock<BTreeSet<ElementKind>> =
        std::sync::LazyLock::new(|| cutaway_lenses::Cut::whole().kinds);

    fn everything() -> &'static BTreeSet<ElementKind> {
        &EVERYTHING
    }

    fn add(graph: &mut ArchitectureGraph, id_text: &str, name: &str, kind: SemanticKind) {
        graph
            .add_element(Element::semantic(
                id(id_text),
                kind,
                ElementName::new(name).unwrap(),
            ))
            .unwrap();
    }

    /// A place in the tree no language read a boundary out of.
    fn add_directory(graph: &mut ArchitectureGraph, id_text: &str, name: &str) {
        graph
            .add_element(Element::substrate(
                id(id_text),
                SubstrateKind::Directory,
                ElementName::new(name).unwrap(),
                None,
            ))
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
        add(&mut graph, "package:core", "core", SemanticKind::Package);
        add(&mut graph, "core/ports.rs", "ports", SemanticKind::Module);
        add(
            &mut graph,
            "core/ports/source_analyzer.rs",
            "ports::source_analyzer",
            SemanticKind::Module,
        );
        add(
            &mut graph,
            "core/ports/source_analyzer.rs#function:analyze",
            "ports::source_analyzer::analyze",
            SemanticKind::Function,
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
        let labels = Labels::of(&view, everything());
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
        add(&mut view, "package:source", "source", SemanticKind::Package);
        add(
            &mut view,
            "source/git.rs",
            "source_git",
            SemanticKind::Module,
        );
        contain(&mut view, "package:source", "source/git.rs");

        let labels = Labels::of(&view, everything());
        assert_eq!(labels.label(&id("source/git.rs")).name, "source_git");
    }

    #[test]
    fn a_leaf_shows_its_kind_and_a_frame_does_not() {
        let view = view();
        let labels = Labels::of(&view, everything());
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
        let labels = Labels::renaming(&view, everything(), &renames);
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
        let labels = Labels::renaming(&view, everything(), &renames);
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
        let labels = Labels::of(&view, everything());
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
            add(&mut graph, &package_id, package, SemanticKind::Package);
            add(&mut graph, root, "crate", SemanticKind::Module);
            contain(&mut graph, &package_id, root);
        }
        add(&mut graph, "gui/label.rs", "label", SemanticKind::Module);
        contain(&mut graph, "package:cutaway-gui", "gui/label.rs");
        graph
    }

    #[test]
    fn a_name_repeated_in_a_list_gains_its_container() {
        let view = crates();
        let labels = Labels::of(&view, everything());
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
        let labels = Labels::of(&view, everything());
        let listed = [id("gui/lib.rs"), id("lenses/lib.rs"), id("gui/label.rs")];
        let names = labels.distinct(&listed);
        assert_eq!(names.name(&listed[2]), "label");
    }

    /// The module `element`, read out of the file `element.rs`, inside a
    /// package that occupies the directory `crates/architecture`.
    fn fused() -> ArchitectureGraph {
        let mut graph = ArchitectureGraph::new();
        fuse(
            &mut graph,
            "package:cutaway-architecture",
            (SemanticKind::Package, "cutaway-architecture"),
            (SubstrateKind::Directory, "crates/architecture"),
        );
        fuse(
            &mut graph,
            "crates/architecture/src/element.rs",
            (SemanticKind::Module, "element"),
            (SubstrateKind::File, "element.rs"),
        );
        contain(
            &mut graph,
            "package:cutaway-architecture",
            "crates/architecture/src/element.rs",
        );
        graph
    }

    fn fuse(
        graph: &mut ArchitectureGraph,
        id_text: &str,
        semantic: (SemanticKind, &str),
        substrate: (SubstrateKind, &str),
    ) {
        graph
            .add_element(Element::fused(
                id(id_text),
                Semantic {
                    kind: semantic.0,
                    name: name(semantic.1),
                },
                Substrate {
                    kind: substrate.0,
                    name: name(substrate.1),
                },
                None,
            ))
            .unwrap();
    }

    #[test]
    fn a_fused_node_speaks_the_name_its_vocabulary_admits() {
        let view = fused();
        let element = id("crates/architecture/src/element.rs");

        assert_eq!(
            Labels::of(&view, everything()).qualified(&element),
            "element",
            "with both readings drawn the language's reading leads"
        );
        assert_eq!(
            Labels::of(
                &view,
                &BTreeSet::from([ElementKind::Package, ElementKind::File])
            )
            .qualified(&element),
            "element.rs",
            "with modules hidden the same boundary is the file it is"
        );
    }

    #[test]
    fn a_fused_leaf_wears_the_mark_of_the_reading_it_speaks() {
        let view = fused();
        let element = id("crates/architecture/src/element.rs");

        assert_eq!(
            Labels::of(&view, everything()).label(&element).glyph,
            Some(kind_symbol(ElementKind::Module))
        );
        assert_eq!(
            Labels::of(
                &view,
                &BTreeSet::from([ElementKind::Package, ElementKind::File])
            )
            .label(&element)
            .glyph,
            Some(kind_symbol(ElementKind::File)),
            "a name the tree gives never wears a language's mark"
        );
    }

    #[test]
    fn a_boundary_of_two_readings_names_both_and_repeats_no_name() {
        let element = fused()
            .element(&id("crates/architecture/src/element.rs"))
            .unwrap()
            .clone();

        assert_eq!(
            readings(&element, "element"),
            format!("Module{}File element.rs", glyph::READING_STEP),
            "the heading spells the module's name, so the file adds the half it left out"
        );
        assert_eq!(
            readings(&element, "element.rs"),
            format!("Module element{}File", glyph::READING_STEP),
            "the same boundary read the other way round"
        );
    }

    #[test]
    fn a_boundary_of_one_reading_names_that_one() {
        let file = Element::substrate(
            id("README.md"),
            SubstrateKind::File,
            name("README.md"),
            None,
        );
        assert_eq!(readings(&file, "README.md"), "File");
    }

    #[test]
    fn a_node_the_vocabulary_speaks_no_reading_of_keeps_its_default_voice() {
        let view = fused();
        assert_eq!(
            Labels::of(&view, &BTreeSet::from([ElementKind::Package]))
                .qualified(&id("crates/architecture/src/element.rs")),
            "element",
            "a boundary the picture draws no box for is still named in a list beside it"
        );
    }

    /// Two TypeScript modules named after the stem of their file, standing
    /// in one directory because the single-child directories around them
    /// dissolved into their names.
    fn namesakes() -> ArchitectureGraph {
        let mut graph = ArchitectureGraph::new();
        add_directory(&mut graph, "app/src", "src");
        for place in ["a", "b"] {
            let path = format!("app/src/{place}/index.ts");
            fuse(
                &mut graph,
                &path,
                (SemanticKind::Module, "index"),
                (SubstrateKind::File, &format!("{place}/index.ts")),
            );
            contain(&mut graph, "app/src", &path);
        }
        graph
    }

    #[test]
    fn colliding_spoken_names_fall_back_to_the_place_in_the_tree() {
        let view = namesakes();
        let labels = Labels::of(&view, everything());

        assert_eq!(
            labels.label(&id("app/src/a/index.ts")).name,
            "a/index.ts",
            "two boxes reading index name neither"
        );
        assert_eq!(labels.label(&id("app/src/b/index.ts")).name, "b/index.ts");
    }

    #[test]
    fn a_name_no_boundary_beside_it_repeats_stays_the_one_the_vocabulary_speaks() {
        let mut view = namesakes();
        add_directory(&mut view, "app/src/only", "only");
        fuse(
            &mut view,
            "app/src/only/index.ts",
            (SemanticKind::Module, "alone"),
            (SubstrateKind::File, "index.ts"),
        );
        contain(&mut view, "app/src/only", "app/src/only/index.ts");

        assert_eq!(
            Labels::of(&view, everything())
                .label(&id("app/src/only/index.ts"))
                .name,
            "alone",
            "a namesake in another frame collides with nothing"
        );
    }

    #[test]
    fn namesakes_with_no_place_stay_namesakes_on_a_box_and_read_apart_beside_it() {
        let mut view = ArchitectureGraph::new();
        add(&mut view, "package:app", "app", SemanticKind::Package);
        for path in ["app/one.ts#type:Entry", "app/two.ts#type:Entry"] {
            add(&mut view, path, "Entry", SemanticKind::Type);
            contain(&mut view, "package:app", path);
        }

        let labels = Labels::of(&view, everything());
        for path in ["app/one.ts#type:Entry", "app/two.ts#type:Entry"] {
            assert_eq!(
                labels.label(&id(path)).name,
                "Entry",
                "a box paints no id: the picture has no room for a path, \
                 and no reader knows the boundary by one"
            );
        }
        assert_eq!(
            labels.qualified(&id("app/one.ts#type:Entry")),
            "app/one.ts#type:Entry",
            "a row the reader picks one of must say which one it is"
        );
        assert_eq!(
            labels.qualified(&id("app/two.ts#type:Entry")),
            "app/two.ts#type:Entry"
        );
    }

    #[test]
    fn a_measured_label_carries_the_glyph_the_canvas_paints() {
        let view = view();
        let labels = Labels::of(&view, everything());
        let leaf = labels.label(&id("core/ports/source_analyzer.rs#function:analyze"));
        assert_eq!(leaf.text(), format!("{} analyze", glyph::FUNCTION));
    }
}
