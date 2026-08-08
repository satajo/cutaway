//! How the reader sets what the picture speaks about and how deep its
//! frontier reaches.
//!
//! The vocabulary is a row of chips, one per element kind, each answering to
//! a digit; the frontier steps through two buttons, answering to plus and
//! minus. The line beside them counts the open boundaries, so the reader
//! sees how far into the tree the picture reaches without moving anything.

use std::collections::BTreeSet;

use cutaway_architecture::{ArchitectureGraph, Element, ElementKind};
use eframe::egui;

/// The kinds the picture can speak about, with the label and the digit each
/// answers to. Coarsest first, so the digits read down the hierarchy.
const KINDS: [(ElementKind, &str, egui::Key); 6] = [
    (ElementKind::Package, "Packages", egui::Key::Num1),
    (ElementKind::Directory, "Directories", egui::Key::Num2),
    (ElementKind::Module, "Modules", egui::Key::Num3),
    (ElementKind::File, "Files", egui::Key::Num4),
    (ElementKind::Type, "Types", egui::Key::Num5),
    (ElementKind::Function, "Functions", egui::Key::Num6),
];

/// What a key or a toolbar button asks of the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Request {
    /// Add one kind to the vocabulary, or take it out.
    Toggle(ElementKind),
    /// Open every boundary the picture offers to open.
    OpenLayer,
    /// Close the innermost open boundaries.
    CloseLayer,
}

/// The kinds the architecture holds, over every reading of every element of
/// the graph: a module read out of a file answers to both, and either chip
/// acts on it. A control over a kind outside this set would toggle an empty
/// set and change nothing visible, so the chips and the digits gate
/// themselves on it.
pub(crate) fn present_kinds(graph: &ArchitectureGraph) -> BTreeSet<ElementKind> {
    graph.elements().flat_map(Element::kinds).collect()
}

/// The vocabulary of the whole picture, as one chip per kind. A chip whose
/// kind the architecture does not hold sits disabled, still showing its
/// state: it keeps the row's shape and says why it cannot act. Answers with
/// the kind the reader toggled, and with None every other frame.
pub(crate) fn chips(
    ui: &mut egui::Ui,
    kinds: &BTreeSet<ElementKind>,
    present: &BTreeSet<ElementKind>,
) -> Option<ElementKind> {
    let mut toggled = None;
    ui.scope(|ui| {
        // The chips sit against each other, so the row reads as one control
        // rather than as a handful of buttons.
        ui.spacing_mut().item_spacing.x = 1.0;
        for (position, (kind, label, _)) in KINDS.into_iter().enumerate() {
            let clicked = ui
                .add_enabled(
                    present.contains(&kind),
                    egui::Button::selectable(kinds.contains(&kind), label),
                )
                .on_hover_text(format!(
                    "Show {} in the picture ({})",
                    label.to_lowercase(),
                    position + 1
                ))
                // "The architecture" rather than "this project": a Rust
                // repository plainly holds filesystem directories, yet its
                // architecture holds no directory elements - the sentence
                // must not read as false about the files on disk.
                .on_disabled_hover_text(format!(
                    "The architecture holds no {} to show or hide",
                    label.to_lowercase()
                ))
                .clicked();
            if clicked {
                toggled = Some(kind);
            }
        }
    });
    toggled
}

/// The frontier buttons: the whole picture one layer deeper, or one layer
/// back. Answers with what the reader clicked, and with None every other
/// frame.
pub(crate) fn layer_buttons(ui: &mut egui::Ui) -> Option<Request> {
    let mut asked = None;
    if ui
        .button("Open a layer")
        .on_hover_text("Open every boundary the picture offers to open (+).")
        .clicked()
    {
        asked = Some(Request::OpenLayer);
    }
    if ui
        .button("Close a layer")
        .on_hover_text("Close the innermost open boundaries (-).")
        .clicked()
    {
        asked = Some(Request::CloseLayer);
    }
    asked
}

/// What this frame's keys ask of the picture, and None while they ask
/// nothing.
///
/// A digit typed into a note or a search is a digit rather than a command,
/// so the shell asks this only while no text field holds the keyboard and no
/// palette is open. The palette takes its own keys before this runs and
/// never asks for a digit, so the two never contend.
///
/// A digit answering to a kind the architecture does not hold asks nothing,
/// for the same reason its chip sits disabled: toggling an empty set changes
/// nothing visible and would read as a fault.
pub(crate) fn requested(ctx: &egui::Context, present: &BTreeSet<ElementKind>) -> Option<Request> {
    if ctx.text_edit_focused() {
        return None;
    }
    ctx.input_mut(|input| {
        KINDS
            .into_iter()
            .filter(|(kind, _, _)| present.contains(kind))
            .find(|(_, _, key)| input.consume_key(egui::Modifiers::NONE, *key))
            .map(|(kind, _, _)| Request::Toggle(kind))
            .or_else(|| {
                input
                    .consume_key(egui::Modifiers::NONE, egui::Key::Plus)
                    .then_some(Request::OpenLayer)
            })
            .or_else(|| {
                input
                    .consume_key(egui::Modifiers::NONE, egui::Key::Minus)
                    .then_some(Request::CloseLayer)
            })
    })
}

/// How far the frontier reaches, in one line. None while nothing is open: a
/// picture of closed boxes says so itself.
pub(crate) fn standing(open: usize) -> Option<String> {
    match open {
        0 => None,
        1 => Some("1 boundary open".to_owned()),
        several => Some(format!("{several} boundaries open")),
    }
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{Element, ElementId, ElementName};

    use super::*;

    fn holding(kinds: &[ElementKind]) -> ArchitectureGraph {
        let mut graph = ArchitectureGraph::new();
        for (position, kind) in kinds.iter().enumerate() {
            let name = format!("element-{position}");
            graph
                .add_element(Element::of_kind(
                    ElementId::new(&name).unwrap(),
                    *kind,
                    ElementName::new(&name).unwrap(),
                ))
                .unwrap();
        }
        graph
    }

    #[test]
    fn an_empty_architecture_presents_no_kinds() {
        assert_eq!(present_kinds(&holding(&[])), BTreeSet::new());
    }

    #[test]
    fn an_architecture_presents_exactly_the_kinds_its_elements_carry() {
        let graph = holding(&[
            ElementKind::Package,
            ElementKind::Module,
            ElementKind::Module,
        ]);
        assert_eq!(
            present_kinds(&graph),
            BTreeSet::from([ElementKind::Package, ElementKind::Module])
        );
    }

    #[test]
    fn a_picture_of_closed_boxes_counts_nothing() {
        assert_eq!(standing(0), None);
    }

    #[test]
    fn one_open_boundary_counts_in_the_singular() {
        assert_eq!(standing(1), Some("1 boundary open".to_owned()));
    }

    #[test]
    fn several_open_boundaries_count_in_the_plural() {
        assert_eq!(standing(3), Some("3 boundaries open".to_owned()));
    }
}
