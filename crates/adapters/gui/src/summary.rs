//! What the magnification has shrunk past reading, and what stands for it.
//!
//! Pulled far back over a large project, a deep frame arrives on screen as a
//! sliver: its children cover a pixel or two each, their names vanished long
//! before that, and none of it says anything - yet every box of it still
//! paints and still answers the pointer. A frame none of whose children can
//! say anything renders as one solid block instead. The block carries the
//! frame's name and how much it holds, and nothing inside it paints at all.
//! A frame with even one readable part stays open, and the parts that
//! cannot read yet blur block by block on their own, so detail arrives
//! outermost first as the camera closes in.
//!
//! The decision belongs to the magnification, so it is made anew whenever
//! the camera changes it and never in between. It is pure geometry and
//! containment over the laid-out view, so it is unit-testable without a
//! screen. Everything downstream reads the answer: the canvas paints and
//! hit-tests a block instead of its contents, and the routing lands an edge
//! on the block that stands for a hidden endpoint.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::ElementId;

use crate::canvas::{HAIRLINE, LABEL_SIZE, LEGIBLE_FONT};
use crate::focus::{Containment, Strength};
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
pub(crate) fn summarize(containment: &Containment, layout: &Layout, zoom: f32) -> Summary {
    let mut summary = Summary::default();
    for frame in &layout.containers {
        if summary.hides(&frame.id) || !blurs(&frame.id, containment, layout, zoom) {
            continue;
        }
        let covers: BTreeSet<ElementId> = containment
            .subtree(&frame.id)
            .into_iter()
            .cloned()
            .collect();
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

/// Whether every child of a frame arrives too small to read: the clearest
/// of them decides, because a frame reveals its parts as soon as any of
/// them can say something. A child measures by its smaller side - a label
/// is a line of text, so a box squeezed flat and a box squeezed narrow
/// read equally little, however far the other side stretches. A frame of
/// one huge sub-frame and a few small leaves opens early to show that
/// structure - the small leaves paint tiny beside it, and any child frame
/// whose own contents stay illegible blurs into a block of its own.
fn blurs(frame: &ElementId, containment: &Containment, layout: &Layout, zoom: f32) -> bool {
    let clearest = containment
        .children(frame)
        .iter()
        .filter_map(|child| layout.rects.get(child))
        .map(|rect| rect.height().min(rect.width()) * zoom)
        .fold(f32::NEG_INFINITY, f32::max);
    clearest.is_finite() && clearest < LEGIBLE_BOX
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{
        ArchitectureGraph, Element, ElementKind, ElementName, Relation, RelationKind,
    };
    use eframe::egui::{Rect, Vec2, pos2, vec2};

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
                fingerprint: None,
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

    fn placed(rects: &[(&str, Vec2)]) -> BTreeMap<ElementId, Rect> {
        rects
            .iter()
            .enumerate()
            .map(|(index, (name, size))| {
                let top = 10000.0 * f32::from(u16::try_from(index).unwrap());
                (id(name), Rect::from_min_size(pos2(0.0, top), *size))
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
            rects: placed(&[
                ("package:a", vec2(100.0, 300.0)),
                ("a/one", vec2(100.0, child)),
                ("a/two", vec2(100.0, child)),
            ]),
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
                ("package:a", vec2(200.0, 200.0)),
                ("a/inner", vec2(100.0, 100.0)),
                ("a/inner/one", vec2(100.0, 10.0)),
                ("a/inner/two", vec2(100.0, 10.0)),
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
        let summary = summarize(&Containment::of(&view), &layout, 0.2);

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
        let summary = summarize(&Containment::of(&view), &layout, 1.0);

        assert!(summary.block(&id("package:a")).is_none());
        assert!(!summary.hides(&id("a/one")));
    }

    #[test]
    fn a_sliver_too_narrow_to_name_blurs_however_tall_it_stands() {
        let mut graph = ArchitectureGraph::new();
        for element in ["package:a", "a/one", "a/two"] {
            add(&mut graph, element);
        }
        contain(&mut graph, "package:a", "a/one");
        contain(&mut graph, "package:a", "a/two");
        let layout = Layout {
            rects: placed(&[
                ("package:a", vec2(30.0, 700.0)),
                ("a/one", vec2(8.0, 300.0)),
                ("a/two", vec2(8.0, 300.0)),
            ]),
            containers: vec![Frame {
                id: id("package:a"),
                depth: 0,
            }],
            leaves: vec![id("a/one"), id("a/two")],
        };
        let summary = summarize(&Containment::of(&graph), &layout, 1.0);

        assert!(
            summary.block(&id("package:a")).is_some(),
            "a name is a line of text, so a box squeezed narrow says nothing"
        );
    }

    /// A frame shaped like a real package: one huge sub-frame beside a few
    /// small leaves. The sub-frame's height dwarfs the leaves'.
    fn lopsided() -> (ArchitectureGraph, Layout) {
        let mut graph = ArchitectureGraph::new();
        for element in ["package:a", "a/src", "a/src/one", "a/src/two", "a/config"] {
            add(&mut graph, element);
        }
        contain(&mut graph, "package:a", "a/src");
        contain(&mut graph, "package:a", "a/config");
        contain(&mut graph, "a/src", "a/src/one");
        contain(&mut graph, "a/src", "a/src/two");
        let layout = Layout {
            rects: placed(&[
                ("package:a", vec2(9000.0, 9000.0)),
                ("a/src", vec2(8000.0, 8000.0)),
                ("a/src/one", vec2(100.0, NODE_HEIGHT)),
                ("a/src/two", vec2(100.0, NODE_HEIGHT)),
                ("a/config", vec2(100.0, NODE_HEIGHT)),
            ]),
            containers: vec![
                Frame {
                    id: id("package:a"),
                    depth: 0,
                },
                Frame {
                    id: id("a/src"),
                    depth: 1,
                },
            ],
            leaves: vec![id("a/src/one"), id("a/src/two"), id("a/config")],
        };
        (graph, layout)
    }

    #[test]
    fn a_small_leaf_does_not_keep_a_frames_readable_structure_hidden() {
        let (view, layout) = lopsided();
        // At this magnification the config leaf is far from legible, but the
        // src frame covers most of the screen: showing it says something.
        let summary = summarize(&Containment::of(&view), &layout, 0.05);

        assert!(
            summary.block(&id("package:a")).is_none(),
            "a frame with a readable part shows its parts"
        );
        assert!(
            summary.block(&id("a/src")).is_some(),
            "the sub-frame's own children are all illegible, so it blurs"
        );
        assert!(!summary.hides(&id("a/config")));
    }

    #[test]
    fn a_summarized_frame_absorbs_the_frames_inside_it() {
        let (view, layout) = nested();
        let summary = summarize(&Containment::of(&view), &layout, 0.05);

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
        let summary = summarize(&Containment::of(&view), &layout, 0.5);

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
        let summary = summarize(&Containment::of(&view), &layout, 0.2);
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
