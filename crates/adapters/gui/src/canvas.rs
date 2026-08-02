//! The boundary canvas: draws the laid-out view and reports what the user
//! clicked. All state changes happen in the app shell; the canvas only
//! renders, steers its camera, and hit-tests.
//!
//! The canvas maps world coordinates to the screen with its own pan/zoom
//! camera instead of egui's Scene. Scene scales already-rasterized glyphs,
//! which pixelates text; the camera instead picks the font size each frame,
//! so labels stay sharp at every magnification.

use cutaway_architecture::{ArchitectureGraph, ElementId, Relation};
use cutaway_lenses::is_self_leaf;
use eframe::egui::emath::TSTransform;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Response, Sense, Shape,
    Stroke, StrokeKind, Ui, pos2, vec2,
};

use crate::focus::{Focus, Selected, Strength, focus_of};
use crate::label::{Label, Labels};
use crate::layout::{HEADER, Layout};
use crate::routing::{self, Route, Scope};

/// How one dependency edge is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeStatus {
    /// Part of the current architecture: monochrome, solid.
    Existing,
    /// Planned for removal: red, dashed.
    Severed,
    /// Planned addition: green, dashed.
    Drawn,
}

#[derive(Debug, Clone)]
pub struct EdgeVisual {
    pub relation: Relation,
    pub status: EdgeStatus,
    pub annotated: bool,
    /// How many concrete dependencies this one edge stands for. A heavier
    /// edge draws thicker, so the picture says where the traffic is.
    pub weight: usize,
}

/// What the user clicked on the canvas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanvasAction {
    Node(ElementId),
    /// A double-clicked boundary: the reader asks to see inside it.
    Expand(ElementId),
    Edge(Relation),
    Background,
}

/// One frame's input to the canvas.
pub struct Content<'a> {
    pub view: &'a ArchitectureGraph,
    pub layout: &'a Layout,
    pub edges: &'a [EdgeVisual],
    pub selected_edge: Option<&'a Relation>,
    pub selected_node: Option<&'a ElementId>,
    pub draw_source: Option<&'a ElementId>,
}

pub const SEVERED: Color32 = Color32::from_rgb(205, 70, 60);
pub const DRAWN: Color32 = Color32::from_rgb(70, 165, 80);

const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 6.0;
/// Screen-pixel distance within which a pointer catches an edge.
const EDGE_REACH: f32 = 8.0;
/// Straight segments a curve flattens into for drawing and hit-testing.
const CURVE_SEGMENTS: u16 = 24;
/// Color strength left to everything outside the selection's neighborhood.
const FADE: f32 = 0.18;
/// Color strength left to the frames around the selection's neighborhood:
/// enough to read their names, little enough to stay background.
const CONTEXT: f32 = 0.55;
/// Color strength of a frame's own-content leaf while nothing touches it:
/// present beside the parts, never competing with them.
const OWN_CONTENT: f32 = 0.5;
/// Color strength of a kind glyph beside the name it marks.
const GLYPH: f32 = 0.55;
/// The gap between a kind glyph and its name, in font sizes.
const GLYPH_GAP: f32 = 0.3;
/// Opacity of the wash that tints one nesting level of frames.
const WASH: f32 = 0.06;
/// Stroke width of an edge that stands for a single concrete dependency.
const EDGE_WIDTH: f32 = 1.2;
/// Width added per square root of the further concrete dependencies a
/// rolled-up edge stands for.
const WEIGHT_WIDTH: f32 = 0.7;
/// The widest an edge draws, however much it stands for: past this the
/// stroke stops reading as a line and starts hiding the boxes.
const MAX_EDGE_WIDTH: f32 = 4.0;
/// Width added while the pointer catches an edge.
const HOVER_WIDTH: f32 = 0.7;
/// Width added while an edge is selected.
const SELECTED_WIDTH: f32 = 1.5;
/// Width and color strength left to an edge that stays inside one top-level
/// boundary: a boundary's internal wiring is not the picture's subject, and
/// a dense boundary would otherwise drown the crossings around it.
const INTRA_WIDTH: f32 = 0.75;
const INTRA: f32 = 0.6;

pub fn show(
    ui: &mut Ui,
    content: &Content<'_>,
    camera: &mut Option<TSTransform>,
) -> Option<CanvasAction> {
    let viewport = ui.max_rect();
    let world = world_bounds(content.layout);

    // The background sits below every node, so nodes win overlapping hits.
    let background = ui.interact(
        viewport,
        ui.id().with("canvas-background"),
        Sense::click_and_drag(),
    );
    let camera = {
        let current = camera.get_or_insert_with(|| fit(world, viewport));
        steer(ui, &background, viewport, world, current);
        *current
    };

    let touched = interact_nodes(ui, content.layout, camera, viewport);
    let hovered_node = touched.hovered;

    let routes = routing::routes(
        content.view,
        content.layout,
        content.edges.iter().map(|edge| &edge.relation),
    );
    let curves: Vec<Option<Vec<Pos2>>> = routes
        .iter()
        .map(|route| {
            route
                .as_ref()
                .map(|route| flattened(route.curve.map(|point| camera.mul_pos(point))))
        })
        .collect();
    let pointer = ui
        .input(|input| input.pointer.hover_pos())
        .filter(|position| viewport.contains(*position));
    let hovered_edge = match &hovered_node {
        Some(_) => None,
        None => pointer.and_then(|position| nearest_curve(&curves, position)),
    };
    let drawn = Drawn {
        routes,
        curves,
        hovered: hovered_edge,
    };
    if let Some(index) = drawn.hovered {
        ui.ctx()
            .output_mut(|output| output.cursor_icon = CursorIcon::PointingHand);
        describe_edge(ui, content, index);
    }

    paint(ui, content, camera, viewport, &drawn, hovered_node.as_ref());

    // A double click also reports as a click, and its first click already
    // selected the node; the deeper answer is the one that stands.
    if let Some(id) = touched.double_clicked {
        return Some(CanvasAction::Expand(id));
    }
    if let Some(id) = touched.clicked {
        return Some(CanvasAction::Node(id));
    }
    if background.clicked() {
        return background
            .interact_pointer_pos()
            .and_then(|position| nearest_curve(&drawn.curves, position))
            .map(|index| CanvasAction::Edge(content.edges[index].relation.clone()))
            .or(Some(CanvasAction::Background));
    }
    None
}

/// The edges of one frame: where each one runs, the flattened screen curve
/// that both paints and hit-tests it, and the one the pointer catches. All
/// three lists index alike with [`Content::edges`].
struct Drawn {
    routes: Vec<Option<Route>>,
    curves: Vec<Option<Vec<Pos2>>>,
    hovered: Option<usize>,
}

/// Names the edge under the pointer: which boundaries it joins, how many
/// concrete dependencies it stands for, and what the plan does to it. The
/// canvas hit-tests its own edges, so the tooltip follows the pointer
/// instead of a widget.
fn describe_edge(ui: &Ui, content: &Content<'_>, index: usize) {
    let Some(edge) = content.edges.get(index) else {
        return;
    };
    let labels = Labels::of(content.view);
    let joins = format!(
        "{} → {}",
        labels.qualified(&edge.relation.from),
        labels.qualified(&edge.relation.to)
    );
    // A planned addition stands for nothing concrete yet, so it counts
    // nothing; every edge the architecture already carries does.
    let stands_for = match (edge.status, edge.weight) {
        (EdgeStatus::Drawn, _) => None,
        (_, 1) => Some("1 concrete dependency".to_owned()),
        (_, many) => Some(format!("{many} concrete dependencies")),
    };
    egui::Tooltip::always_open(
        ui.ctx().clone(),
        ui.layer_id(),
        ui.id().with("edge-tooltip"),
        egui::PopupAnchor::Pointer,
    )
    .show(|ui| {
        ui.label(joins);
        if let Some(stands_for) = stands_for {
            ui.label(stands_for);
        }
        match edge.status {
            EdgeStatus::Existing => {}
            EdgeStatus::Severed => {
                ui.colored_label(SEVERED, "Planned for removal.");
            }
            EdgeStatus::Drawn => {
                ui.colored_label(DRAWN, "Planned addition.");
            }
        }
    });
}

/// Applies drag-to-pan, scroll-to-pan, and pinch-or-ctrl-scroll zoom about
/// the pointer. A double click on the background refits the whole graph.
fn steer(ui: &Ui, background: &Response, viewport: Rect, world: Rect, camera: &mut TSTransform) {
    if background.double_clicked() {
        *camera = fit(world, viewport);
        return;
    }
    if background.dragged() {
        camera.translation += background.drag_delta();
    }
    let Some(pointer) = ui
        .input(|input| input.pointer.hover_pos())
        .filter(|position| viewport.contains(*position))
    else {
        return;
    };
    let zoom = ui.input(egui::InputState::zoom_delta);
    if (zoom - 1.0).abs() > f32::EPSILON {
        let allowed = (camera.scaling * zoom).clamp(MIN_ZOOM, MAX_ZOOM) / camera.scaling;
        *camera = TSTransform::from_translation(pointer.to_vec2())
            * TSTransform::from_scaling(allowed)
            * TSTransform::from_translation(-pointer.to_vec2())
            * *camera;
    } else {
        camera.translation += ui.input(|input| input.smooth_scroll_delta);
    }
}

/// How far the camera must move to bring one box into the viewport, and
/// None while the box already reads there. A selection made beside the
/// picture must be findable in it, but a picture that jumps on every click
/// loses the reader, so the camera stays still whenever it can.
///
/// Presence is the center being on screen, not the whole box fitting: a
/// frame wider than the viewport never fits, and re-centering it on every
/// click would move the picture for nothing.
pub(crate) fn reveal_shift(viewport: Rect, on_screen: Rect) -> Option<egui::Vec2> {
    if !viewport.is_positive() || !on_screen.is_positive() {
        return None;
    }
    (!viewport.contains(on_screen.center())).then(|| viewport.center() - on_screen.center())
}

/// The transform that centers the world bounds in the viewport, zoomed to
/// fit but never past 1.25x.
fn fit(world: Rect, viewport: Rect) -> TSTransform {
    if !world.is_positive() || !viewport.is_positive() {
        return TSTransform::IDENTITY;
    }
    let available = viewport.shrink(32.0);
    let scale = (available.width() / world.width())
        .min(available.height() / world.height())
        .clamp(MIN_ZOOM, 1.25);
    TSTransform::from_translation(viewport.center().to_vec2() - world.center().to_vec2() * scale)
        * TSTransform::from_scaling(scale)
}

/// What the pointer did to the nodes this frame.
struct Touched {
    hovered: Option<ElementId>,
    clicked: Option<ElementId>,
    double_clicked: Option<ElementId>,
}

/// Registers a click-and-hover area for every node. Leaves register after
/// containers, so they win overlapping hits; a container answers only on
/// its header strip. The nodes register after the background, so a click
/// that lands on a node never reaches the background - which is what keeps
/// a double-clicked node out of the background's refit.
fn interact_nodes(ui: &mut Ui, layout: &Layout, camera: TSTransform, viewport: Rect) -> Touched {
    let mut targets: Vec<(&ElementId, Rect)> = layout
        .containers
        .iter()
        .map(|frame| (&frame.id, header_of(layout.rects[&frame.id])))
        .collect();
    targets.extend(layout.leaves.iter().map(|id| (id, layout.rects[id])));

    let mut touched = Touched {
        hovered: None,
        clicked: None,
        double_clicked: None,
    };
    for (id, rect) in targets {
        let on_screen = camera.mul_rect(rect).intersect(viewport);
        if !on_screen.is_positive() {
            continue;
        }
        let response = ui
            .interact(on_screen, ui.id().with(id.as_str()), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        if response.hovered() {
            touched.hovered = Some(id.clone());
        }
        if response.clicked() {
            touched.clicked = Some(id.clone());
        }
        if response.double_clicked() {
            touched.double_clicked = Some(id.clone());
        }
    }
    touched
}

/// The clickable strip along a container's top edge.
fn header_of(rect: Rect) -> Rect {
    Rect::from_min_size(rect.min, vec2(rect.width(), HEADER))
}

struct Paint<'a> {
    painter: egui::Painter,
    camera: TSTransform,
    base: Color32,
    fill: Color32,
    /// The interior tints of frames, by nesting depth parity.
    washes: [Color32; 2],
    labels: Labels<'a>,
    focus: Option<Focus<'a>>,
}

impl Paint<'_> {
    /// The label font at the current zoom; None when too small to read.
    fn font(&self) -> Option<FontId> {
        let size = 13.0 * self.camera.scaling;
        (size >= 4.0).then(|| FontId::proportional(size))
    }

    fn stroke_width(&self, width: f32) -> f32 {
        (width * self.camera.scaling).max(0.75)
    }

    fn element(&self, id: &ElementId) -> Strength {
        self.focus
            .as_ref()
            .map_or(Strength::Focused, |focus| focus.element(id))
    }

    fn edge(&self, relation: &Relation) -> Strength {
        self.focus
            .as_ref()
            .map_or(Strength::Focused, |focus| focus.edge(relation))
    }

    /// The interior of a frame at this nesting depth.
    fn wash(&self, depth: usize) -> Color32 {
        self.washes[depth % 2]
    }
}

/// The tints that separate one nesting level from the next: consecutive
/// depths wash in opposite directions, one toward the text and one toward
/// the deepest background, so no frame shares the shade of the frame around
/// it. Both directions come from the theme, so a light and a dark canvas
/// read alike, and both stay faint enough to leave the boxes and the edges
/// the picture.
fn washes(visuals: &egui::Visuals) -> [Color32; 2] {
    [
        visuals.text_color().gamma_multiply(WASH),
        visuals.extreme_bg_color.gamma_multiply(WASH),
    ]
}

fn shade(color: Color32, strength: Strength) -> Color32 {
    match strength {
        Strength::Focused => color,
        Strength::Context => color.gamma_multiply(CONTEXT),
        Strength::Faded => color.gamma_multiply(FADE),
    }
}

fn paint(
    ui: &Ui,
    content: &Content<'_>,
    camera: TSTransform,
    viewport: Rect,
    drawn: &Drawn,
    hovered_node: Option<&ElementId>,
) {
    let visuals = ui.visuals();
    let paint = Paint {
        painter: ui.painter().with_clip_rect(viewport),
        camera,
        base: visuals.text_color(),
        fill: visuals.extreme_bg_color,
        washes: washes(visuals),
        labels: Labels::of(content.view),
        focus: focus(content),
    };
    paint_containers(&paint, content, hovered_node);
    paint_edges(&paint, content, drawn);
    paint_leaves(&paint, content, hovered_node);
}

/// The selection's neighborhood. None when nothing is selected: then
/// nothing fades.
fn focus<'a>(content: &Content<'a>) -> Option<Focus<'a>> {
    let selected = match (content.selected_node, content.selected_edge) {
        (Some(id), _) => Selected::Node(id),
        (None, Some(relation)) => Selected::Edge(relation),
        (None, None) => return None,
    };
    Some(focus_of(
        content.view,
        content.edges.iter().map(|edge| &edge.relation),
        selected,
    ))
}

fn paint_containers(paint: &Paint<'_>, content: &Content<'_>, hovered: Option<&ElementId>) {
    for frame in &content.layout.containers {
        let id = &frame.id;
        let rect = paint.camera.mul_rect(content.layout.rects[id]);
        let strength = paint.element(id);
        let selected = content.selected_node == Some(id) || content.draw_source == Some(id);
        let width = if selected {
            2.5
        } else if hovered == Some(id) {
            1.8
        } else {
            1.0
        };
        paint.painter.rect(
            rect,
            CornerRadius::same(6),
            shade(paint.wash(frame.depth), strength),
            Stroke::new(
                paint.stroke_width(width),
                shade(paint.base.gamma_multiply(0.6), strength),
            ),
            StrokeKind::Middle,
        );
        if let Some(font) = paint.font() {
            paint.painter.text(
                rect.min + vec2(10.0, 6.0) * paint.camera.scaling,
                Align2::LEFT_TOP,
                paint.labels.label(id).text(),
                font,
                shade(paint.base, strength),
            );
        }
    }
}

fn paint_edges(paint: &Paint<'_>, content: &Content<'_>, drawn: &Drawn) {
    for (index, edge) in content.edges.iter().enumerate() {
        let (Some(Some(route)), Some(Some(points))) =
            (drawn.routes.get(index), drawn.curves.get(index))
        else {
            continue;
        };
        let ink = match edge.status {
            EdgeStatus::Existing => paint.base,
            EdgeStatus::Severed => SEVERED,
            EdgeStatus::Drawn => DRAWN,
        };
        // The scope dims before the selection does: an internal edge stays
        // behind the crossings whether anything is selected or not.
        let color = shade(dim(ink, route.scope), paint.edge(&edge.relation));
        let selected = content.selected_edge == Some(&edge.relation);
        let width = edge_width(edge.weight, route.scope)
            + if selected {
                SELECTED_WIDTH
            } else if drawn.hovered == Some(index) {
                HOVER_WIDTH
            } else {
                0.0
            };
        let stroke = Stroke::new(paint.stroke_width(width), color);
        if edge.status == EdgeStatus::Existing {
            paint.painter.add(Shape::line(points.clone(), stroke));
        } else {
            let dash = paint.camera.scaling.max(0.5);
            for shape in Shape::dashed_line(points, stroke, 8.0 * dash, 5.0 * dash) {
                paint.painter.add(shape);
            }
        }
        arrow_head(&paint.painter, points, color, paint.camera.scaling);
        if edge.annotated {
            paint.painter.circle_filled(
                points[points.len() / 2],
                (4.0 * paint.camera.scaling).max(2.0),
                color,
            );
        }
    }
}

/// How thick an edge draws before selection and hover add to it: the weight
/// enters as its square root, so a boundary that answers ten dependencies
/// reads heavier than one that answers two without swallowing the picture.
fn edge_width(weight: usize, scope: Scope) -> f32 {
    let further = f32::from(u16::try_from(weight.saturating_sub(1)).unwrap_or(u16::MAX));
    let width = (EDGE_WIDTH + WEIGHT_WIDTH * further.sqrt()).min(MAX_EDGE_WIDTH);
    match scope {
        Scope::Intra => width * INTRA_WIDTH,
        Scope::Cross => width,
    }
}

fn dim(color: Color32, scope: Scope) -> Color32 {
    match scope {
        Scope::Intra => color.gamma_multiply(INTRA),
        Scope::Cross => color,
    }
}

fn paint_leaves(paint: &Paint<'_>, content: &Content<'_>, hovered: Option<&ElementId>) {
    for id in &content.layout.leaves {
        let rect = paint.camera.mul_rect(content.layout.rects[id]);
        let strength = paint.element(id);
        let selected = content.selected_node == Some(id) || content.draw_source == Some(id);
        let hover = hovered == Some(id);
        let width = if selected {
            2.5
        } else if hover {
            1.8
        } else {
            1.0
        };
        // A frame's own content is not a part beside the parts: it keeps a
        // dim border and no fill until the pointer or the selection reaches
        // it.
        let secondary = is_self_leaf(id) && !hover && !selected;
        let ink = if secondary {
            OWN_CONTENT
        } else if hover {
            1.0
        } else {
            0.8
        };
        paint.painter.rect(
            rect,
            CornerRadius::same(5),
            if secondary {
                Color32::TRANSPARENT
            } else {
                shade(paint.fill, strength)
            },
            Stroke::new(
                paint.stroke_width(width),
                shade(paint.base.gamma_multiply(ink), strength),
            ),
            StrokeKind::Middle,
        );
        if let Some(font) = paint.font() {
            let color = shade(
                paint
                    .base
                    .gamma_multiply(if secondary { OWN_CONTENT } else { 1.0 }),
                strength,
            );
            paint_leaf_label(
                &paint.painter,
                rect.center(),
                &paint.labels.label(id),
                &font,
                color,
            );
        }
    }
}

/// Writes a leaf's label centered in its box. The kind glyph paints dimmer
/// than the name beside it: the name is the subject, the glyph only says
/// what kind of thing carries it. Layout measures glyph and name together,
/// so the pair always fits.
fn paint_leaf_label(
    painter: &egui::Painter,
    center: Pos2,
    label: &Label,
    font: &FontId,
    color: Color32,
) {
    let name = painter.layout_no_wrap(label.name.clone(), font.clone(), color);
    let Some(glyph) = label.glyph else {
        let size = name.size();
        painter.galley(center - size / 2.0, name, color);
        return;
    };
    let dim = color.gamma_multiply(GLYPH);
    let glyph = painter.layout_no_wrap((*glyph).to_owned(), font.clone(), dim);
    let (glyph_size, name_size) = (glyph.size(), name.size());
    let gap = font.size * GLYPH_GAP;
    let left = center.x - (glyph_size.x + gap + name_size.x) / 2.0;
    painter.galley(pos2(left, center.y - glyph_size.y / 2.0), glyph, dim);
    painter.galley(
        pos2(left + glyph_size.x + gap, center.y - name_size.y / 2.0),
        name,
        color,
    );
}

fn flattened(controls: [Pos2; 4]) -> Vec<Pos2> {
    (0..=CURVE_SEGMENTS)
        .map(|segment| cubic_point(controls, f32::from(segment) / f32::from(CURVE_SEGMENTS)))
        .collect()
}

fn cubic_point(c: [Pos2; 4], t: f32) -> Pos2 {
    let u = 1.0 - t;
    (c[0].to_vec2() * (u * u * u)
        + c[1].to_vec2() * (3.0 * u * u * t)
        + c[2].to_vec2() * (3.0 * u * t * t)
        + c[3].to_vec2() * (t * t * t))
        .to_pos2()
}

fn arrow_head(painter: &egui::Painter, points: &[Pos2], color: Color32, zoom: f32) {
    let &[.., before, tip] = points else {
        return;
    };
    let direction = (tip - before).normalized();
    if !direction.is_finite() {
        return;
    }
    let normal = vec2(-direction.y, direction.x);
    let length = (10.0 * zoom).clamp(6.0, 20.0);
    let left = tip - direction * length + normal * (length / 2.0);
    let right = tip - direction * length - normal * (length / 2.0);
    painter.add(Shape::convex_polygon(
        vec![tip, left, right],
        color,
        Stroke::NONE,
    ));
}

fn nearest_curve(curves: &[Option<Vec<Pos2>>], pointer: Pos2) -> Option<usize> {
    let mut best: Option<(f32, usize)> = None;
    for (index, curve) in curves.iter().enumerate() {
        let Some(points) = curve else {
            continue;
        };
        for pair in points.windows(2) {
            let distance = distance_to_segment(pointer, pair[0], pair[1]);
            if distance < EDGE_REACH && best.is_none_or(|(closest, _)| distance < closest) {
                best = Some((distance, index));
            }
        }
    }
    best.map(|(_, index)| index)
}

fn distance_to_segment(point: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let length_squared = ab.length_sq();
    if length_squared == 0.0 {
        return (point - a).length();
    }
    let t = ((point - a).dot(ab) / length_squared).clamp(0.0, 1.0);
    (point - (a + ab * t)).length()
}

/// The rectangle the whole picture occupies in world coordinates.
pub(crate) fn world_bounds(layout: &Layout) -> Rect {
    layout
        .rects
        .values()
        .fold(Rect::NOTHING, |acc, rect| acc.union(*rect))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0))
    }

    #[test]
    fn a_box_already_on_screen_leaves_the_camera_where_it_is() {
        let on_screen = Rect::from_center_size(pos2(200.0, 150.0), vec2(60.0, 30.0));
        assert_eq!(reveal_shift(viewport(), on_screen), None);
    }

    #[test]
    fn a_box_larger_than_the_viewport_counts_as_on_screen() {
        let on_screen = Rect::from_center_size(pos2(200.0, 150.0), vec2(4000.0, 3000.0));
        assert_eq!(reveal_shift(viewport(), on_screen), None);
    }

    #[test]
    fn a_box_off_screen_moves_to_the_middle_of_the_viewport() {
        let on_screen = Rect::from_center_size(pos2(900.0, 700.0), vec2(60.0, 30.0));
        assert_eq!(
            reveal_shift(viewport(), on_screen),
            Some(vec2(-700.0, -550.0))
        );
    }

    #[test]
    fn nothing_moves_before_the_canvas_has_a_viewport() {
        let on_screen = Rect::from_center_size(pos2(900.0, 700.0), vec2(60.0, 30.0));
        assert_eq!(reveal_shift(Rect::NOTHING, on_screen), None);
    }
}
