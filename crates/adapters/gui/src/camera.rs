//! Where the picture stands in front of the reader, and how it gets there.
//!
//! The camera moves in two ways. The reader's hand moves it directly - a
//! drag, a scroll, a pinch, a grip on the map - and there it must answer the
//! hand exactly, frame by frame; anything smoothed would feel like a loose
//! wheel. Everything else moves it in one leap: a refit, or a reveal that
//! follows a selection into a picture cut at another detail. A leap lands
//! the reader somewhere they never saw the way to, so those moves travel
//! over [`TRAVEL`] seconds and the reader keeps their bearings.
//!
//! Where a reveal sends the camera is decided by [`revealing`] alone, out of
//! the viewport, the camera, and the subject's box in the world. That
//! decision is geometry, so it is answered here and proved without a screen.

use eframe::egui::emath::TSTransform;
use eframe::egui::{self, Pos2, Rect, Response, Ui, Vec2};

/// The magnification the reader is held between. Past the near bound a box
/// is all that fits on the canvas; past the far one the whole picture is
/// texture.
pub(crate) const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 6.0;
/// The closest the camera ever places something by itself. A fit that
/// magnified further would blow a small picture up into a poster and say
/// nothing more than 1:1 already does.
const FIT_ZOOM: f32 = 1.25;
/// The space kept between what the camera places and the edge of the canvas,
/// so nothing the camera brings lands under the window's own chrome.
const MARGIN: f32 = 32.0;
/// The share of the canvas a subject must cover before it counts as seen at
/// all. Two per mille of the canvas is a few dozen square points: a speck
/// the reader would have to hunt for, which is exactly what a reveal exists
/// to spare them.
const LEGIBLE_SHARE: f32 = 0.002;
/// The surroundings a reveal shows around its subject, in shares of the
/// subject's size on each side. A subject alone in the frame answers "here
/// it is" and nothing else; a quarter of its size on every side says what it
/// sits among.
const CONTEXT_RING: f32 = 0.25;
/// The largest share of either side of the canvas a revealed subject covers.
/// Past this the subject is not shown in context, it is wallpaper.
const MAX_SUBJECT_SHARE: f32 = 0.6;
/// How long the camera takes to travel from one stand to another. Long
/// enough that the eye can follow the picture across, short enough that the
/// reader never waits for it.
const TRAVEL: f32 = 0.25;
/// The magnification a point of wheel travel is worth, as an exponent. This
/// is the speed egui itself gives ctrl+scroll, so one notch of the wheel
/// magnifies by the same step whichever way the reader asks for it.
const WHEEL_ZOOM_SPEED: f32 = 1.0 / 200.0;

/// Where the picture stands, and where it is travelling.
///
/// The camera holds nothing until a frame fits the whole picture into the
/// canvas: the world exists before the canvas that shows it.
#[derive(Default)]
pub(crate) struct Camera {
    at: Option<TSTransform>,
    flight: Option<Flight>,
}

/// One journey of the camera: where it left, where it is bound, and whether
/// its clock has started. The clock starts on the first frame that draws the
/// flight, because a reveal is decided beside the canvas, where no frame
/// time is at hand.
struct Flight {
    from: TSTransform,
    to: TSTransform,
    launched: bool,
}

impl Camera {
    /// Where the camera stands right now, and None until a frame has fitted
    /// the picture.
    pub(crate) fn now(&self) -> Option<TSTransform> {
        self.at
    }

    /// Forgets where the camera stood, so the next frame fits the whole
    /// picture again. A picture laid out anew leaves the old coordinates
    /// pointing at arbitrary content, and only a fresh fit shows something
    /// meaningful.
    pub(crate) fn forget(&mut self) {
        self.at = None;
        self.flight = None;
    }

    /// Puts the camera exactly here and ends any travel: the reader's hand
    /// outranks the picture's own movement.
    pub(crate) fn hold(&mut self, at: TSTransform) -> TSTransform {
        self.at = Some(at);
        self.flight = None;
        at
    }

    /// Sends the camera to another stand, over time. A camera that stands
    /// nowhere yet has no journey to make: it simply arrives.
    pub(crate) fn fly(&mut self, to: TSTransform) {
        match self.at {
            None => self.at = Some(to),
            Some(from) => {
                self.flight = Some(Flight {
                    from,
                    to,
                    launched: false,
                });
            }
        }
    }

    /// Where the camera stands this frame: its travel carried one frame
    /// further, or a fresh fit of the whole world while it stands nowhere.
    pub(crate) fn advance(&mut self, ui: &Ui, world: Rect, viewport: Rect) -> TSTransform {
        let at = *self.at.get_or_insert_with(|| fit(world, viewport));
        let Some(flight) = &mut self.flight else {
            return at;
        };
        let context = ui.ctx();
        let clock = ui.id().with("camera-flight");
        if !flight.launched {
            flight.launched = true;
            // A journey measures its own time from zero, wherever the last
            // one left the clock. Asking for no animation time at all resets
            // it in place.
            context.animate_value_with_time(clock, 0.0, 0.0);
        }
        let progress = context.animate_value_with_time(clock, 1.0, TRAVEL);
        let at = flight.at(viewport, progress);
        if progress >= 1.0 {
            self.flight = None;
        }
        self.at = Some(at);
        at
    }
}

impl Flight {
    /// Where the camera stands after a share of the journey.
    ///
    /// Magnification steps geometrically, because zoom is multiplicative:
    /// halfway from a tenth to a whole is a third, not a twentieth over a
    /// half. The world point in the middle of the canvas travels evenly
    /// beside it, so the picture slides where it is going instead of
    /// swinging past it while the magnification catches up.
    fn at(&self, viewport: Rect, progress: f32) -> TSTransform {
        if !self.from.is_valid() || !self.to.is_valid() || !viewport.is_positive() {
            return self.to;
        }
        let eased = egui::emath::easing::cubic_in_out(progress.clamp(0.0, 1.0));
        let scaling = self.from.scaling * (self.to.scaling / self.from.scaling).powf(eased);
        let middle = |stand: TSTransform| stand.inverse().mul_pos(viewport.center());
        let looked_at = middle(self.from) + (middle(self.to) - middle(self.from)) * eased;
        centered_on(looked_at.to_vec2(), scaling, viewport)
    }
}

/// Applies drag-to-pan, scroll-or-pinch zoom about the pointer, and
/// shift-scroll pan, and answers with the camera the frame paints at.
///
/// The wheel magnifies and the hand pans, because the reader crosses this
/// picture far more often by getting closer to a boundary than by sliding
/// the page.
///
/// A double click on the background refits the whole picture. That one
/// travels rather than cuts: the reader's hand is not on the picture, and a
/// refit is the longest leap the canvas ever makes.
pub(crate) fn steer(
    ui: &Ui,
    background: &Response,
    viewport: Rect,
    world: Rect,
    camera: &mut Camera,
    current: TSTransform,
) -> TSTransform {
    if background.double_clicked() {
        camera.fly(fit(world, viewport));
        return current;
    }
    let mut steered = current;
    if background.dragged() {
        steered = shifted(steered, background.drag_delta());
    }
    // The wheel, the pinch, the modifier and the pointer are read from one
    // snapshot: a modifier let go between two reads would turn the reader's
    // pan into a zoom halfway through the gesture.
    let (pinch, scroll, pans, pointer) = ui.input(|input| {
        (
            input.zoom_delta(),
            input.smooth_scroll_delta,
            input.modifiers.shift,
            input.pointer.hover_pos(),
        )
    });
    if let Some(pointer) = pointer.filter(|position| viewport.contains(*position)) {
        // A pinch and a wheel that arrive in the same frame are one wish for
        // magnification, so they multiply into a single step instead of each
        // moving the camera in turn.
        let zoom = pinch * wheel_zoom(scroll, pans);
        if (zoom - 1.0).abs() > f32::EPSILON {
            steered = magnified(steered, pointer, zoom);
        } else {
            steered = shifted(steered, scroll);
        }
    }
    if steered == current {
        // Nothing was steered, so nothing outranks a travel in progress.
        return current;
    }
    camera.hold(steered)
}

/// What the wheel asks the magnification to be multiplied by, and 1.0 where
/// it asks for a pan instead.
///
/// Magnification is multiplicative, so wheel travel enters it as an exponent:
/// equal travel in either direction magnifies and shrinks by the same step,
/// and travelling back undoes exactly what travelling out did.
///
/// Two deltas mean a pan and never a zoom. Shift is the reader saying so,
/// and egui has already laid that gesture on the horizontal axis by the time
/// it arrives here. A delta with no vertical part at all is a sideways
/// scroll from a trackpad or a tilting wheel, which carries nothing to
/// magnify by.
fn wheel_zoom(scroll: Vec2, pans: bool) -> f32 {
    if pans || scroll.y == 0.0 {
        return 1.0;
    }
    (scroll.y * WHEEL_ZOOM_SPEED).exp()
}

/// The camera magnified about one point on the screen, held between the
/// bounds the reader is allowed.
///
/// The world under that point stays under it: the picture grows around what
/// the reader is pointing at, so the boundary they are reaching for is the
/// one that fills the canvas.
fn magnified(camera: TSTransform, about: Pos2, zoom: f32) -> TSTransform {
    let allowed = (camera.scaling * zoom).clamp(MIN_ZOOM, MAX_ZOOM) / camera.scaling;
    TSTransform::from_translation(about.to_vec2())
        * TSTransform::from_scaling(allowed)
        * TSTransform::from_translation(-about.to_vec2())
        * camera
}

/// Whether this frame's keys ask for the whole picture again.
///
/// Home is the key that goes back to the beginning, and the beginning of a
/// picture is all of it. It commands only while no text field holds the
/// keyboard: in a note or a search field, Home moves the caret.
pub(crate) fn refit_requested(ctx: &egui::Context) -> bool {
    if ctx.text_edit_focused() {
        return false;
    }
    ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Home))
}

/// Where the camera must stand to put a subject in front of the reader, and
/// None while it already does.
///
/// | the subject at the current magnification      | the camera         |
/// | --------------------------------------------- | ------------------ |
/// | fits the canvas, reads, and is wholly inside   | stays              |
/// | fits the canvas and reads, but is cut off      | pans to the middle |
/// | overflows the canvas, or is too small to read  | refits around it   |
///
/// The middle row is the small hop the reader follows with their eye, and
/// it keeps the magnification they chose. The last row is the case a pan
/// cannot answer: a picture cut at another detail lays out a world of
/// another size, and panning an old magnification into it drops the reader
/// among boxes with no bearing on what they asked about.
pub(crate) fn revealing(viewport: Rect, camera: TSTransform, subject: Rect) -> Option<TSTransform> {
    let available = viewport.shrink(MARGIN);
    if !available.is_positive() || !subject.is_positive() || !camera.is_valid() {
        return None;
    }
    let on_screen = camera.mul_rect(subject);
    let fits = on_screen.width() <= available.width() && on_screen.height() <= available.height();
    let reads = on_screen.area() >= LEGIBLE_SHARE * viewport.area();
    if fits && reads {
        return (!available.contains_rect(on_screen))
            .then(|| shifted(camera, viewport.center() - on_screen.center()));
    }
    Some(around(subject, viewport))
}

/// The camera that shows one subject with room around it: as close as the
/// context ring, the canvas margin, and the subject's own share of the
/// canvas all allow, and never closer than a fit would come.
fn around(subject: Rect, viewport: Rect) -> TSTransform {
    let available = viewport.shrink(MARGIN).size();
    let with_context = subject.size() * (1.0 + 2.0 * CONTEXT_RING);
    let scaling = (available.x / with_context.x)
        .min(available.y / with_context.y)
        .min(viewport.width() * MAX_SUBJECT_SHARE / subject.width())
        .min(viewport.height() * MAX_SUBJECT_SHARE / subject.height())
        .clamp(MIN_ZOOM, FIT_ZOOM);
    centered_on(subject.center().to_vec2(), scaling, viewport)
}

/// The transform that centers the world bounds in the canvas, zoomed to fit.
pub(crate) fn fit(world: Rect, viewport: Rect) -> TSTransform {
    if !world.is_positive() || !viewport.is_positive() {
        return TSTransform::IDENTITY;
    }
    let available = viewport.shrink(MARGIN);
    let scaling = (available.width() / world.width())
        .min(available.height() / world.height())
        .clamp(MIN_ZOOM, FIT_ZOOM);
    centered_on(world.center().to_vec2(), scaling, viewport)
}

/// The camera that holds one world point in the middle of the canvas at a
/// given magnification.
fn centered_on(world: Vec2, scaling: f32, viewport: Rect) -> TSTransform {
    TSTransform::from_translation(viewport.center().to_vec2() - world * scaling)
        * TSTransform::from_scaling(scaling)
}

fn shifted(camera: TSTransform, by: Vec2) -> TSTransform {
    TSTransform {
        translation: camera.translation + by,
        ..camera
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::{pos2, vec2};

    use super::*;

    fn viewport() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0))
    }

    /// The canvas minus the margin every placement keeps.
    fn available() -> Rect {
        viewport().shrink(MARGIN)
    }

    /// Geometry answered in floating point is compared at the resolution a
    /// canvas has: a hundredth of a point is the same place.
    fn same(left: f32, right: f32) -> bool {
        (left - right).abs() < 0.01
    }

    fn same_stand(left: TSTransform, right: TSTransform) -> bool {
        same(left.scaling, right.scaling)
            && same(left.translation.x, right.translation.x)
            && same(left.translation.y, right.translation.y)
    }

    #[test]
    fn a_visible_subject_leaves_the_camera_alone() {
        let subject = Rect::from_center_size(pos2(200.0, 150.0), vec2(60.0, 30.0));
        assert_eq!(revealing(viewport(), TSTransform::IDENTITY, subject), None);
    }

    #[test]
    fn a_clipped_subject_pans_into_view() {
        let subject = Rect::from_center_size(pos2(390.0, 150.0), vec2(60.0, 30.0));

        let moved = revealing(viewport(), TSTransform::IDENTITY, subject)
            .expect("a subject hanging over the edge is not yet revealed");
        assert!(
            same(moved.scaling, 1.0),
            "a pan keeps the magnification the reader chose: {moved:?}"
        );
        let on_screen = moved.mul_rect(subject);
        assert!(
            same(on_screen.center().x, viewport().center().x)
                && same(on_screen.center().y, viewport().center().y),
            "{on_screen:?}"
        );
    }

    #[test]
    fn a_subject_lost_at_the_old_zoom_refits_around_it() {
        let pulled_back = TSTransform::from_scaling(0.05);
        let subject = Rect::from_center_size(pos2(2000.0, 1000.0), vec2(200.0, 100.0));

        let moved = revealing(viewport(), pulled_back, subject)
            .expect("a speck on the canvas is not revealed by standing still");
        assert!(
            moved.scaling > pulled_back.scaling,
            "a subject too small to read is met with magnification, not a pan: {moved:?}"
        );
        let on_screen = moved.mul_rect(subject);
        assert!(
            on_screen.area() >= LEGIBLE_SHARE * viewport().area(),
            "the reveal must leave the subject readable: {on_screen:?}"
        );
        assert!(
            available().contains_rect(on_screen),
            "a refit brings the whole subject inside the canvas: {on_screen:?}"
        );
    }

    #[test]
    fn a_revealed_subject_keeps_room_around_it() {
        let subject = Rect::from_center_size(pos2(500.0, 400.0), vec2(1000.0, 800.0));

        let moved = revealing(viewport(), TSTransform::IDENTITY, subject)
            .expect("a subject larger than the canvas is not revealed by standing still");
        let on_screen = moved.mul_rect(subject);
        assert!(
            available().contains_rect(on_screen),
            "the whole subject lands inside the canvas margin: {on_screen:?}"
        );
        assert!(
            on_screen.width() <= MAX_SUBJECT_SHARE * viewport().width()
                && on_screen.height() <= MAX_SUBJECT_SHARE * viewport().height(),
            "the subject is shown among its surroundings, not across them: {on_screen:?}"
        );
    }

    #[test]
    fn a_reveal_never_magnifies_past_a_fit() {
        let subject = Rect::from_center_size(pos2(0.0, 0.0), vec2(4.0, 3.0));

        let moved = revealing(viewport(), TSTransform::from_scaling(0.05), subject)
            .expect("a speck is not revealed by standing still");
        assert!(moved.scaling <= FIT_ZOOM, "{moved:?}");
    }

    #[test]
    fn nothing_moves_before_the_canvas_has_a_viewport() {
        let subject = Rect::from_center_size(pos2(900.0, 700.0), vec2(60.0, 30.0));
        assert_eq!(
            revealing(Rect::NOTHING, TSTransform::IDENTITY, subject),
            None
        );
    }

    #[test]
    fn magnifying_about_a_point_leaves_that_point_where_it_is() {
        let camera = centered_on(vec2(1000.0, 500.0), 0.7, viewport());
        let pointer = pos2(310.0, 90.0);
        let under_pointer = camera.inverse().mul_pos(pointer);

        let closer = magnified(camera, pointer, 1.4);
        assert!(same(closer.scaling, 0.98), "{closer:?}");
        let moved = closer.mul_pos(under_pointer);
        assert!(
            same(moved.x, pointer.x) && same(moved.y, pointer.y),
            "the world under the pointer must not slide out from under it: {moved:?}"
        );
    }

    #[test]
    fn magnification_stops_at_the_bounds_the_reader_is_held_between() {
        let pointer = pos2(120.0, 200.0);
        let farthest = centered_on(vec2(1000.0, 500.0), MIN_ZOOM, viewport());
        let nearest = centered_on(vec2(1000.0, 500.0), MAX_ZOOM, viewport());

        assert!(
            same_stand(magnified(farthest, pointer, 0.1), farthest),
            "a camera at the far bound stands still instead of drifting"
        );
        assert!(
            same_stand(magnified(nearest, pointer, 10.0), nearest),
            "a camera at the near bound stands still instead of drifting"
        );
        let leapt = magnified(farthest, pointer, 100.0);
        assert!(
            same(leapt.scaling, MAX_ZOOM),
            "one huge step lands on the bound, not past it: {leapt:?}"
        );
    }

    #[test]
    fn wheel_travel_back_undoes_the_magnification_it_made() {
        let out = wheel_zoom(vec2(0.0, 14.0), false);
        let back = wheel_zoom(vec2(0.0, -14.0), false);

        assert!(out > 1.0, "scrolling up comes closer: {out}");
        assert!(same(out * back, 1.0), "{out} against {back}");
    }

    #[test]
    fn a_sideways_scroll_pans_instead_of_magnifying() {
        assert!(same(wheel_zoom(vec2(-30.0, 0.0), false), 1.0));
        assert!(
            same(wheel_zoom(vec2(0.0, 14.0), true), 1.0),
            "shift asks the wheel to pan, whichever axis the delta arrives on"
        );
    }

    fn flight() -> Flight {
        Flight {
            from: TSTransform::from_scaling(0.1),
            to: centered_on(vec2(1000.0, 500.0), 0.9, viewport()),
            launched: false,
        }
    }

    #[test]
    fn a_flight_leaves_where_the_camera_stood_and_lands_where_it_was_sent() {
        let flight = flight();
        assert!(same_stand(flight.at(viewport(), 0.0), flight.from));
        assert!(same_stand(flight.at(viewport(), 1.0), flight.to));
    }

    #[test]
    fn a_flight_changes_magnification_geometrically() {
        let flight = flight();
        let halfway = flight.at(viewport(), 0.5).scaling;

        let even = (flight.from.scaling * flight.to.scaling).sqrt();
        assert!(
            (halfway - even).abs() < 0.001,
            "halfway in time is halfway in zoom, not in scale: {halfway} against {even}"
        );
    }

    #[test]
    fn a_flight_carries_the_middle_of_the_canvas_between_its_ends() {
        let flight = flight();
        let middle = |stand: TSTransform| stand.inverse().mul_pos(viewport().center());

        let travelled = middle(flight.at(viewport(), 0.5));
        let (left, right) = (middle(flight.from), middle(flight.to));
        assert!(
            travelled.x > left.x.min(right.x) && travelled.x < left.x.max(right.x),
            "the picture slides toward its destination instead of swinging past it: {travelled:?}"
        );
    }
}
