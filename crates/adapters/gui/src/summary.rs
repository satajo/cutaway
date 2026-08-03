//! What the magnification has shrunk past reading, and what stands for it.
//!
//! Pulled far back over a large project, a deep frame arrives on screen as a
//! sliver: its children cover a pixel or two each, their names vanished long
//! before that, and none of it says anything - yet every box of it still
//! paints and still answers the pointer. Such a frame renders as one solid
//! block instead. The block carries the frame's name and how much it holds,
//! and nothing inside it paints at all.
//!
//! The decision belongs to the camera, so it is made anew every frame. It is
//! also pure geometry and containment over the laid-out view, so it is
//! unit-testable without a screen. Everything downstream reads the answer:
//! the canvas paints and hit-tests a block instead of its contents, and the
//! routing lands an edge on the block that stands for a hidden endpoint.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{ArchitectureGraph, ElementId};

use crate::canvas::{HAIRLINE, LABEL_SIZE, LEGIBLE_FONT};
use crate::focus::{Strength, children_of};
use crate::layout::{Layout, NODE_HEIGHT};

/// The screen height a box needs before its contents are worth painting.
///
/// A label paints while `LABEL_SIZE * zoom >= LEGIBLE_FONT`, so the smallest
/// magnification that still carries a name is `LEGIBLE_FONT / LABEL_SIZE`.
/// The shortest box a leaf ever gets is `NODE_HEIGHT` world units tall,
/// which at that magnification covers `NODE_HEIGHT * LEGIBLE_FONT /
/// LABEL_SIZE` = 9.2 screen pixels; the border such a box strokes above and
/// below its content never draws thinner than `HAIRLINE` and adds two more.
/// Below the sum, 10.7 pixels, a child box is chrome around a name nobody
/// can read.
const LEGIBLE_BOX: f32 = NODE_HEIGHT * LEGIBLE_FONT / LABEL_SIZE + 2.0 * HAIRLINE;

/// One frame the canvas paints as a solid block instead of as a frame around
/// its parts.
pub(crate) struct Block {
    /// The boundaries the block stands for: the frame itself and everything
    /// below it.
    covers: BTreeSet<ElementId>,
    /// How many boundaries the block hides.
    pub(crate) inside: usize,
}

impl Block {
    /// How strongly the block paints: the strongest strength anything it
    /// stands for would paint at. The block is the only mark those elements
    /// have left in the picture, so whatever is in focus inside it is in
    /// focus as the block.
    pub(crate) fn strength(&self, of: impl Fn(&ElementId) -> Strength) -> Strength {
        self.covers
            .iter()
            .map(of)
            .max()
            .unwrap_or(Strength::Focused)
    }
}

/// The frames one magnification summarizes, and what each of them hides.
#[derive(Default)]
pub(crate) struct Summary {
    blocks: BTreeMap<ElementId, Block>,
    stands_for: BTreeMap<ElementId, ElementId>,
}

impl Summary {
    /// The block a frame paints as; None while the frame paints around its
    /// parts.
    pub(crate) fn block(&self, id: &ElementId) -> Option<&Block> {
        self.blocks.get(id)
    }

    /// Whether a block stands for this element. Such an element paints
    /// nothing and answers no pointer: the block is there in its place.
    pub(crate) fn hides(&self, id: &ElementId) -> bool {
        self.stands_for.contains_key(id)
    }

    /// Hidden element -> the block that stands for it. An edge that ends in
    /// something hidden lands on that block instead.
    pub(crate) fn stands_for(&self) -> &BTreeMap<ElementId, ElementId> {
        &self.stands_for
    }
}

/// The frames of a laid-out view too small at this magnification to show
/// what they hold. The walk runs outermost first, so the first frame that
/// summarizes absorbs everything below it and no frame inside a block is
/// weighed on its own.
pub(crate) fn summarize(view: &ArchitectureGraph, layout: &Layout, zoom: f32) -> Summary {
    let children = children_of(view);
    let mut summary = Summary::default();
    for frame in &layout.containers {
        if summary.hides(&frame.id) || !blurs(&frame.id, &children, layout, zoom) {
            continue;
        }
        let covers = subtree(&frame.id, &children);
        for hidden in &covers {
            if *hidden != frame.id {
                summary.stands_for.insert(hidden.clone(), frame.id.clone());
            }
        }
        let inside = covers.iter().filter(|id| **id != frame.id).count();
        summary
            .blocks
            .insert(frame.id.clone(), Block { covers, inside });
    }
    summary
}

/// Whether a frame's children arrive too small to read: the shortest of them
/// decides, because a frame shows its parts only as well as it shows its
/// least readable one.
fn blurs(
    frame: &ElementId,
    children: &BTreeMap<&ElementId, Vec<&ElementId>>,
    layout: &Layout,
    zoom: f32,
) -> bool {
    let shortest = children
        .get(frame)
        .into_iter()
        .flatten()
        .filter_map(|child| layout.rects.get(*child))
        .map(|rect| rect.height() * zoom)
        .fold(f32::INFINITY, f32::min);
    shortest.is_finite() && shortest < LEGIBLE_BOX
}

/// A boundary and everything below it. Containment is a tree wherever a view
/// exists at all, but a walk that trusts that and meets a cycle never ends;
/// the set of what the walk already holds bounds it.
fn subtree(
    root: &ElementId,
    children: &BTreeMap<&ElementId, Vec<&ElementId>>,
) -> BTreeSet<ElementId> {
    let mut inside = BTreeSet::from([root.clone()]);
    let mut queue = vec![root.clone()];
    while let Some(current) = queue.pop() {
        for child in children.get(&current).into_iter().flatten() {
            if inside.insert((*child).clone()) {
                queue.push((*child).clone());
            }
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{Element, ElementKind, ElementName, Relation, RelationKind};
    use eframe::egui::{Rect, pos2, vec2};

    use super::*;
    use crate::layout::Frame;

    fn id(text: &str) -> ElementId {
        ElementId::new(text).unwrap()
    }

    fn add(graph: &mut ArchitectureGraph, id_text: &str) {
        graph
            .add_element(Element {
                id: id(id_text),
                name: ElementName::new(id_text).unwrap(),
                kind: ElementKind::Module,
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

    fn placed(rects: &[(&str, f32)]) -> BTreeMap<ElementId, Rect> {
        rects
            .iter()
            .enumerate()
            .map(|(index, (name, height))| {
                let top = 1000.0 * f32::from(u16::try_from(index).unwrap());
                (
                    id(name),
                    Rect::from_min_size(pos2(0.0, top), vec2(100.0, *height)),
                )
            })
            .collect()
    }

    /// One frame around two leaves, every leaf box `child` world units tall.
    fn frame_of_leaves(child: f32) -> (ArchitectureGraph, Layout) {
        let mut graph = ArchitectureGraph::new();
        for element in ["package:a", "a/one", "a/two"] {
            add(&mut graph, element);
        }
        contain(&mut graph, "package:a", "a/one");
        contain(&mut graph, "package:a", "a/two");
        let layout = Layout {
            rects: placed(&[("package:a", 300.0), ("a/one", child), ("a/two", child)]),
            containers: vec![Frame {
                id: id("package:a"),
                depth: 0,
            }],
            leaves: vec![id("a/one"), id("a/two")],
        };
        (graph, layout)
    }

    /// A frame around a frame around two leaves: the outer frame's one child
    /// is a hundred units tall, the inner frame's children ten.
    fn nested() -> (ArchitectureGraph, Layout) {
        let mut graph = ArchitectureGraph::new();
        for element in ["package:a", "a/inner", "a/inner/one", "a/inner/two"] {
            add(&mut graph, element);
        }
        contain(&mut graph, "package:a", "a/inner");
        contain(&mut graph, "a/inner", "a/inner/one");
        contain(&mut graph, "a/inner", "a/inner/two");
        let layout = Layout {
            rects: placed(&[
                ("package:a", 200.0),
                ("a/inner", 100.0),
                ("a/inner/one", 10.0),
                ("a/inner/two", 10.0),
            ]),
            containers: vec![
                Frame {
                    id: id("package:a"),
                    depth: 0,
                },
                Frame {
                    id: id("a/inner"),
                    depth: 1,
                },
            ],
            leaves: vec![id("a/inner/one"), id("a/inner/two")],
        };
        (graph, layout)
    }

    #[test]
    fn a_frame_whose_children_would_blur_summarizes() {
        let (view, layout) = frame_of_leaves(NODE_HEIGHT);
        let summary = summarize(&view, &layout, 0.2);

        assert!(summary.block(&id("package:a")).is_some());
        assert!(summary.hides(&id("a/one")));
        assert!(summary.hides(&id("a/two")));
        assert_eq!(
            summary.stands_for().get(&id("a/one")),
            Some(&id("package:a")),
            "the block stands for what it hides"
        );
        assert_eq!(
            summary.block(&id("package:a")).unwrap().inside,
            2,
            "the block counts every boundary it hides"
        );
    }

    #[test]
    fn a_frame_whose_children_still_read_keeps_showing_them() {
        let (view, layout) = frame_of_leaves(NODE_HEIGHT);
        let summary = summarize(&view, &layout, 1.0);

        assert!(summary.block(&id("package:a")).is_none());
        assert!(!summary.hides(&id("a/one")));
    }

    #[test]
    fn a_summarized_frame_absorbs_the_frames_inside_it() {
        let (view, layout) = nested();
        let summary = summarize(&view, &layout, 0.05);

        assert!(summary.block(&id("package:a")).is_some());
        assert!(
            summary.block(&id("a/inner")).is_none(),
            "a frame a block already stands for summarizes nothing of its own"
        );
        for hidden in ["a/inner", "a/inner/one", "a/inner/two"] {
            assert_eq!(
                summary.stands_for().get(&id(hidden)),
                Some(&id("package:a")),
                "{hidden} answers as the outermost block"
            );
        }
    }

    #[test]
    fn only_the_frames_whose_own_detail_stopped_reading_summarize() {
        let (view, layout) = nested();
        let summary = summarize(&view, &layout, 0.5);

        assert!(
            summary.block(&id("package:a")).is_none(),
            "a frame whose one child still reads keeps showing it"
        );
        assert!(summary.block(&id("a/inner")).is_some());
        assert!(summary.hides(&id("a/inner/one")));
    }

    #[test]
    fn a_summarized_frame_carries_the_strength_of_what_it_hides() {
        let (view, layout) = frame_of_leaves(NODE_HEIGHT);
        let summary = summarize(&view, &layout, 0.2);
        let block = summary.block(&id("package:a")).unwrap();

        let focused_leaf = |element: &ElementId| {
            if *element == id("a/two") {
                Strength::Focused
            } else {
                Strength::Faded
            }
        };
        assert_eq!(block.strength(focused_leaf), Strength::Focused);
        assert_eq!(
            block.strength(|_| Strength::Context),
            Strength::Context,
            "a block of context alone stays context"
        );
        assert_eq!(block.strength(|_| Strength::Faded), Strength::Faded);
    }
}
