//! The corner overview: where the reader stands in a picture too large to
//! see at once, and the way to travel elsewhere in it.
//!
//! The map draws the world's outer boxes at corner scale and marks the part
//! of the world the camera shows. Pointing at the map moves the camera
//! there; it never magnifies, so the reader keeps the detail they chose.
//! While the camera already shows the whole world the map stands down: it
//! answers "where am I", and "everywhere" needs no answer.
//!
//! Everything here is geometry over the laid-out view and the camera, so it
//! is unit-testable without a screen. The canvas paints the answer and wires
//! the pointer to it.

use cutaway_architecture::ElementId;
use eframe::egui::emath::TSTransform;
use eframe::egui::{Pos2, Rect, Vec2, vec2};

use crate::layout::Layout;

/// The largest box the map ever fills, before the canvas and the world cut
/// it down.
const MAX_SIZE: Vec2 = vec2(200.0, 150.0);
/// The distance the map keeps from the corner of the canvas.
const INSET: f32 = 16.0;
/// The largest share of the canvas the map covers in either direction: on a
/// narrow canvas a full-sized map would hide the picture it reports on.
const MAX_SHARE: f32 = 1.0 / 3.0;
/// The largest a map ever draws the world: one map point per four world
/// units. A world this bound reaches is one the camera has nearly caught
/// already, and blowing it up to fill the corner box would make the map
/// read as a second picture beside the picture.
const MAX_SCALE: f32 = 0.25;
/// The nesting levels that reach the map: the top-level frames and the one
/// level inside them. Deeper than that a box covers a fraction of a map
/// point in any picture large enough to need a map, so drawing it only
/// smears the outline that says where the reader stands.
const OUTER_LEVELS: usize = 2;

/// One frame's overview: where it sits on the screen, and how the world maps
/// onto it.
pub(crate) struct Minimap {
    /// The map's box on the screen.
    pub(crate) rect: Rect,
    /// World coordinates onto map coordinates.
    transform: TSTransform,
}

impl Minimap {
    /// The map the canvas shows this frame, and None while it has nothing to
    /// say: no picture, no canvas, or a camera that already holds the whole
    /// world.
    pub(crate) fn of(world: Rect, viewport: Rect, camera: TSTransform) -> Option<Self> {
        if !world.is_positive() || !viewport.is_positive() || !camera.is_valid() {
            return None;
        }
        if looked_at(viewport, camera).contains_rect(world) {
            return None;
        }
        let room = MAX_SIZE.min(viewport.size() * MAX_SHARE);
        let scale = (room.x / world.width())
            .min(room.y / world.height())
            .min(MAX_SCALE);
        let size = world.size() * scale;
        let rect = Rect::from_min_size(viewport.max - vec2(INSET, INSET) - size, size);
        if !rect.is_positive() {
            return None;
        }
        Some(Self {
            rect,
            transform: TSTransform::from_translation(
                rect.min.to_vec2() - world.min.to_vec2() * scale,
            ) * TSTransform::from_scaling(scale),
        })
    }

    /// The boxes the map draws, in screen coordinates, outermost first.
    pub(crate) fn boxes(&self, layout: &Layout) -> Vec<Rect> {
        shown(layout)
            .into_iter()
            .filter_map(|id| layout.rects.get(id))
            .map(|rect| self.transform.mul_rect(*rect))
            .collect()
    }

    /// The part of the world the camera shows, drawn on the map and cut to
    /// it: a camera that overshoots the picture still reports a rectangle
    /// the reader can find.
    pub(crate) fn looked_at(&self, viewport: Rect, camera: TSTransform) -> Rect {
        self.transform
            .mul_rect(looked_at(viewport, camera))
            .intersect(self.rect)
    }

    /// Where the camera stands after the reader points at a place on the
    /// map: that world point in the middle of the canvas, at the
    /// magnification the reader already chose. A pointer that has left the
    /// map travels to the map's edge, so a drag keeps its grip.
    pub(crate) fn travel(&self, pointer: Pos2, viewport: Rect, camera: TSTransform) -> TSTransform {
        let target = self.transform.inverse().mul_pos(self.rect.clamp(pointer));
        TSTransform::from_translation(
            viewport.center().to_vec2() - target.to_vec2() * camera.scaling,
        ) * TSTransform::from_scaling(camera.scaling)
    }
}

/// The world rectangle the camera shows in the canvas.
fn looked_at(viewport: Rect, camera: TSTransform) -> Rect {
    camera.inverse().mul_rect(viewport)
}

/// The boundaries the map draws: the frames of the outer levels, and every
/// leaf that no deeper frame holds.
///
/// A leaf carries no nesting depth of its own, and the layout packs every
/// child inside the box of the frame that holds it, so the boxes say what
/// the depth would: a leaf sitting inside a frame past the outer levels is a
/// leaf the map leaves out.
fn shown(layout: &Layout) -> Vec<&ElementId> {
    let buried: Vec<Rect> = layout
        .containers
        .iter()
        .filter(|frame| frame.depth >= OUTER_LEVELS - 1)
        .filter_map(|frame| layout.rects.get(&frame.id).copied())
        .collect();
    let frames = layout
        .containers
        .iter()
        .filter(|frame| frame.depth < OUTER_LEVELS)
        .map(|frame| &frame.id);
    let leaves = layout.leaves.iter().filter(|id| {
        layout
            .rects
            .get(*id)
            .is_some_and(|rect| !buried.iter().any(|frame| frame.contains(rect.center())))
    });
    frames.chain(leaves).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use eframe::egui::pos2;

    use super::*;
    use crate::layout::Frame;

    fn id(text: &str) -> ElementId {
        ElementId::new(text).unwrap()
    }

    fn viewport() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))
    }

    fn world() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(2000.0, 1000.0))
    }

    /// A camera that shows the canvas-sized top left corner of the world:
    /// far too little of it for the map to stand down.
    fn corner() -> TSTransform {
        TSTransform::IDENTITY
    }

    fn map() -> Minimap {
        Minimap::of(world(), viewport(), corner()).unwrap()
    }

    #[test]
    fn the_map_mirrors_the_world_at_its_corner_scale() {
        let map = map();

        assert_eq!(
            map.rect,
            Rect::from_min_size(pos2(584.0, 484.0), vec2(200.0, 100.0)),
            "the map sits inset in the bottom right corner of the canvas"
        );
        assert_eq!(
            map.transform.mul_rect(world()),
            map.rect,
            "the world fills the map it scaled to"
        );
    }

    #[test]
    fn a_world_smaller_than_the_corner_box_keeps_a_smaller_map() {
        let small = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 80.0));
        // Magnified twentyfold, a hundred world units cover more than the
        // canvas, so even this world outgrows the camera.
        let map = Minimap::of(small, viewport(), TSTransform::from_scaling(20.0)).unwrap();

        assert_eq!(
            map.rect.size(),
            vec2(25.0, 20.0),
            "a small world draws at one map point per four world units, not blown up to the box"
        );
    }

    #[test]
    fn the_map_hides_when_everything_is_already_visible() {
        let pulled_back = TSTransform::from_scaling(0.2);
        assert!(Minimap::of(world(), viewport(), pulled_back).is_none());
    }

    #[test]
    fn no_map_stands_before_the_canvas_has_a_picture() {
        assert!(Minimap::of(Rect::NOTHING, viewport(), corner()).is_none());
        assert!(Minimap::of(world(), Rect::NOTHING, corner()).is_none());
    }

    #[test]
    fn the_marked_rectangle_covers_what_the_camera_shows() {
        let map = map();

        assert_eq!(
            map.looked_at(viewport(), corner()),
            Rect::from_min_size(map.rect.min, vec2(80.0, 60.0)),
            "the canvas-sized corner of the world marks the same corner of the map"
        );
    }

    #[test]
    fn the_marked_rectangle_stops_at_the_edge_of_the_map() {
        let map = map();
        let past_the_corner = TSTransform::from_translation(vec2(500.0, 300.0));

        let marked = map.looked_at(viewport(), past_the_corner);
        assert!(
            map.rect.contains_rect(marked),
            "a camera that overshoots the picture still marks a rectangle inside the map"
        );
        assert_eq!(
            marked,
            Rect::from_min_max(pos2(584.0, 484.0), pos2(614.0, 514.0))
        );
    }

    #[test]
    fn a_click_on_the_map_centers_the_camera_there() {
        let map = map();

        let travelled = map.travel(map.rect.center(), viewport(), corner());
        assert_eq!(
            travelled.mul_pos(pos2(1000.0, 500.0)),
            viewport().center(),
            "the world point under the pointer lands in the middle of the canvas"
        );
    }

    #[test]
    fn travelling_leaves_the_magnification_alone() {
        let map = map();
        let magnified = TSTransform::from_scaling(3.0);

        let travelled = map.travel(map.rect.left_top(), viewport(), magnified);
        assert!((travelled.scaling - 3.0).abs() < f32::EPSILON);
        assert_eq!(travelled.mul_pos(pos2(0.0, 0.0)), viewport().center());
    }

    #[test]
    fn a_drag_past_the_map_travels_no_further_than_its_edge() {
        let map = map();

        let travelled = map.travel(pos2(4000.0, 4000.0), viewport(), corner());
        assert_eq!(
            travelled.mul_pos(pos2(2000.0, 1000.0)),
            viewport().center(),
            "the far corner of the world is as far as the map reaches"
        );
    }

    /// A frame around a frame around a leaf, with a leaf beside each of them
    /// and one more leaf on the canvas itself.
    fn nested() -> Layout {
        Layout {
            rects: BTreeMap::from([
                (
                    id("package:a"),
                    Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0)),
                ),
                (
                    id("a/inner"),
                    Rect::from_min_size(pos2(20.0, 40.0), vec2(180.0, 160.0)),
                ),
                (
                    id("a/inner/deep"),
                    Rect::from_min_size(pos2(30.0, 60.0), vec2(120.0, 90.0)),
                ),
                (
                    id("a/leaf"),
                    Rect::from_min_size(pos2(220.0, 60.0), vec2(80.0, 30.0)),
                ),
                (
                    id("a/inner/leaf"),
                    Rect::from_min_size(pos2(30.0, 160.0), vec2(70.0, 30.0)),
                ),
                (
                    id("a/inner/deep/leaf"),
                    Rect::from_min_size(pos2(40.0, 80.0), vec2(60.0, 30.0)),
                ),
                (
                    id("package:b"),
                    Rect::from_min_size(pos2(500.0, 0.0), vec2(100.0, 30.0)),
                ),
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
                Frame {
                    id: id("a/inner/deep"),
                    depth: 2,
                },
            ],
            leaves: vec![
                id("a/leaf"),
                id("a/inner/leaf"),
                id("a/inner/deep/leaf"),
                id("package:b"),
            ],
        }
    }

    #[test]
    fn only_the_outer_two_levels_reach_the_map() {
        let layout = nested();
        let drawn: Vec<&str> = shown(&layout).into_iter().map(ElementId::as_str).collect();

        assert_eq!(
            drawn,
            vec!["package:a", "a/inner", "a/leaf", "package:b"],
            "the top-level frames, the level inside them, and the leaves beside them"
        );
    }

    #[test]
    fn every_box_the_map_draws_lands_on_the_map() {
        let layout = nested();
        let map = map();

        for boundary in map.boxes(&layout) {
            assert!(map.rect.contains_rect(boundary));
        }
    }
}
