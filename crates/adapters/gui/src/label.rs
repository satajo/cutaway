//! What a box on the canvas says.
//!
//! A box names one boundary. A kind glyph in front of a leaf says what the
//! boundary is, so a function never reads like a type; a frame needs no
//! glyph, because its form already says that it holds other boundaries. The
//! name drops the path the frame around it already spells, so a module
//! inside its parent module reads as its own segment alone. The inspector
//! still names every boundary in full; only the picture shortens.
//!
//! Layout measures the label the canvas paints, so both ask this module and
//! no box is ever too narrow for its text.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{ArchitectureGraph, ElementId, ElementKind, RelationKind};
use cutaway_lenses::is_self_leaf;

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

/// The labels of one view. A name reads against the frame around it, so the
/// containment resolves once and every box then answers directly.
pub(crate) struct Labels<'a> {
    view: &'a ArchitectureGraph,
    frame_of: BTreeMap<&'a ElementId, &'a ElementId>,
    frames: BTreeSet<&'a ElementId>,
}

impl<'a> Labels<'a> {
    pub(crate) fn of(view: &'a ArchitectureGraph) -> Self {
        let mut frame_of = BTreeMap::new();
        let mut frames = BTreeSet::new();
        for relation in view.relations() {
            if relation.kind == RelationKind::Contains {
                frame_of.insert(&relation.to, &relation.from);
                frames.insert(&relation.from);
            }
        }
        Self {
            view,
            frame_of,
            frames,
        }
    }

    pub(crate) fn label(&self, id: &ElementId) -> Label {
        Label {
            glyph: self.glyph(id),
            name: self.name(id),
        }
    }

    /// The name a text beside the picture gives a boundary: the whole name,
    /// nothing shortened, because only a box is short of room. A frame's own
    /// content answers as the frame it belongs to, which is the boundary the
    /// reader knows.
    pub(crate) fn qualified(&self, id: &ElementId) -> String {
        match self.frame_of.get(id) {
            Some(frame) if is_self_leaf(id) => {
                format!("{} (own content)", self.full_name(frame))
            }
            _ => self.full_name(id),
        }
    }

    /// The kind mark a box carries. A frame carries none - the box around
    /// its children already says what it is - and neither does a frame's own
    /// content, whose kind is the frame's own.
    fn glyph(&self, id: &ElementId) -> Option<&'static str> {
        if self.frames.contains(id) || is_self_leaf(id) {
            return None;
        }
        self.view
            .element(id)
            .map(|element| kind_symbol(element.kind))
    }

    fn name(&self, id: &ElementId) -> String {
        let full = self.full_name(id);
        match self.frame_of.get(id) {
            Some(frame) => contextual(&full, &self.full_name(frame)).to_owned(),
            None => full,
        }
    }

    fn full_name(&self, id: &ElementId) -> String {
        self.view
            .element(id)
            .map_or_else(|| id.to_string(), |element| element.name.to_string())
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
        ElementKind::Project => "◈",
        ElementKind::Package => "▣",
        ElementKind::Module => "▤",
        ElementKind::Function => "ƒ",
        ElementKind::Type => "T",
    }
}

/// What the glyph stands for, in words. A picture has room for a mark
/// alone; a panel beside it has room to say the kind outright.
pub(crate) fn kind_name(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::Project => "Project",
        ElementKind::Package => "Package",
        ElementKind::Module => "Module",
        ElementKind::Function => "Function",
        ElementKind::Type => "Type",
    }
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{Element, ElementName, Relation};

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
        add(
            &mut graph,
            "core/ports.rs#self",
            "self",
            ElementKind::Module,
        );
        contain(&mut graph, "package:core", "core/ports.rs");
        contain(&mut graph, "core/ports.rs", "core/ports/source_analyzer.rs");
        contain(&mut graph, "core/ports.rs", "core/ports.rs#self");
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

    #[test]
    fn a_frames_own_content_shows_no_kind_at_all() {
        let view = view();
        let labels = Labels::of(&view);
        let own = labels.label(&id("core/ports.rs#self"));
        assert_eq!(own.glyph, None);
        assert_eq!(own.name, "self");
        assert_eq!(own.text(), "self");
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

    #[test]
    fn a_frames_own_content_is_named_after_the_frame_it_belongs_to() {
        let view = view();
        let labels = Labels::of(&view);
        assert_eq!(
            labels.qualified(&id("core/ports.rs#self")),
            "ports (own content)"
        );
    }

    #[test]
    fn a_measured_label_carries_the_glyph_the_canvas_paints() {
        let view = view();
        let labels = Labels::of(&view);
        let leaf = labels.label(&id("core/ports/source_analyzer.rs#function:analyze"));
        assert_eq!(leaf.text(), "ƒ analyze");
    }
}
