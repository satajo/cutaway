//! The boundary canvas: draws the laid-out view and reports what the user
//! clicked. All state changes happen in the app shell; the canvas only
//! renders, steers its camera, and hit-tests.
//!
//! The canvas maps world coordinates to the screen with its own pan/zoom
//! camera instead of egui's Scene. Scene scales already-rasterized glyphs,
//! which pixelates text; the camera instead picks the font size each frame,
//! so labels stay sharp at every magnification.

use cutaway_architecture::{ArchitectureGraph, ElementId, Relation};
use eframe::egui::emath::TSTransform;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Response, Sense, Shape,
    Stroke, StrokeKind, Ui, pos2, vec2,
};

use crate::focus::{Focus, Selected, Strength, focus_of};
use crate::layout::{HEADER, Layout};

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
}

/// What the user clicked on the canvas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanvasAction {
    Node(ElementId),
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

    let (hovered_node, clicked_node) = interact_nodes(ui, content.layout, camera, viewport);

    let curves: Vec<Option<Vec<Pos2>>> = content
        .edges
        .iter()
        .map(|edge| {
            curve_between(content.layout, &edge.relation)
                .map(|controls| flattened(controls.map(|point| camera.mul_pos(point))))
        })
        .collect();
    let pointer = ui
        .input(|input| input.pointer.hover_pos())
        .filter(|position| viewport.contains(*position));
    let hovered_edge = match &hovered_node {
        Some(_) => None,
        None => pointer.and_then(|position| nearest_curve(&curves, position)),
    };
    if hovered_edge.is_some() {
        ui.ctx()
            .output_mut(|output| output.cursor_icon = CursorIcon::PointingHand);
    }

    paint(
        ui,
        content,
        camera,
        viewport,
        &curves,
        hovered_node.as_ref(),
        hovered_edge,
    );

    if let Some(id) = clicked_node {
        return Some(CanvasAction::Node(id));
    }
    if background.clicked() {
        return background
            .interact_pointer_pos()
            .and_then(|position| nearest_curve(&curves, position))
            .map(|index| CanvasAction::Edge(content.edges[index].relation.clone()))
            .or(Some(CanvasAction::Background));
    }
    None
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

/// Registers a click-and-hover area for every node. Leaves register after
/// containers, so they win overlapping hits; a container answers only on
/// its header strip.
fn interact_nodes(
    ui: &mut Ui,
    layout: &Layout,
    camera: TSTransform,
    viewport: Rect,
) -> (Option<ElementId>, Option<ElementId>) {
    let mut targets: Vec<(&ElementId, Rect)> = layout
        .containers
        .iter()
        .map(|id| (id, header_of(layout.rects[id])))
        .collect();
    targets.extend(layout.leaves.iter().map(|id| (id, layout.rects[id])));

    let mut hovered = None;
    let mut clicked = None;
    for (id, rect) in targets {
        let on_screen = camera.mul_rect(rect).intersect(viewport);
        if !on_screen.is_positive() {
            continue;
        }
        let response = ui
            .interact(on_screen, ui.id().with(id.as_str()), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        if response.hovered() {
            hovered = Some(id.clone());
        }
        if response.clicked() {
            clicked = Some(id.clone());
        }
    }
    (hovered, clicked)
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
    curves: &[Option<Vec<Pos2>>],
    hovered_node: Option<&ElementId>,
    hovered_edge: Option<usize>,
) {
    let visuals = ui.visuals();
    let paint = Paint {
        painter: ui.painter().with_clip_rect(viewport),
        camera,
        base: visuals.text_color(),
        fill: visuals.extreme_bg_color,
        focus: focus(content),
    };
    paint_containers(&paint, content, hovered_node);
    paint_edges(&paint, content, curves, hovered_edge);
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
    for id in &content.layout.containers {
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
        paint.painter.rect_stroke(
            rect,
            CornerRadius::same(6),
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
                name_of(content.view, id),
                font,
                shade(paint.base, strength),
            );
        }
    }
}

fn paint_edges(
    paint: &Paint<'_>,
    content: &Content<'_>,
    curves: &[Option<Vec<Pos2>>],
    hovered: Option<usize>,
) {
    for (index, (edge, curve)) in content.edges.iter().zip(curves).enumerate() {
        let Some(points) = curve else {
            continue;
        };
        let color = shade(
            match edge.status {
                EdgeStatus::Existing => paint.base,
                EdgeStatus::Severed => SEVERED,
                EdgeStatus::Drawn => DRAWN,
            },
            paint.edge(&edge.relation),
        );
        let selected = content.selected_edge == Some(&edge.relation);
        let width = if selected {
            3.0
        } else if hovered == Some(index) {
            2.2
        } else {
            1.5
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
        paint.painter.rect(
            rect,
            CornerRadius::same(5),
            shade(paint.fill, strength),
            Stroke::new(
                paint.stroke_width(width),
                shade(
                    paint.base.gamma_multiply(if hover { 1.0 } else { 0.8 }),
                    strength,
                ),
            ),
            StrokeKind::Middle,
        );
        if let Some(font) = paint.font() {
            paint.painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                name_of(content.view, id),
                font,
                shade(paint.base, strength),
            );
        }
    }
}

fn name_of(view: &ArchitectureGraph, id: &ElementId) -> String {
    view.element(id)
        .map_or_else(|| id.to_string(), |element| element.name.to_string())
}

/// The cubic control points of one dependency curve, in world coordinates.
/// The curve leaves the side of `from` that faces `to` and enters `to`
/// facing back, with tangents along the travel direction.
fn curve_between(layout: &Layout, relation: &Relation) -> Option<[Pos2; 4]> {
    let from = *layout.rects.get(&relation.from)?;
    let to = *layout.rects.get(&relation.to)?;
    Some(if to.min.x > from.max.x || from.min.x > to.max.x {
        horizontal_curve(from, to)
    } else if to.min.y > from.max.y || from.min.y > to.max.y {
        vertical_curve(from, to)
    } else {
        // Overlapping boxes, e.g. an edge into a surrounding container:
        // fall back to a straight border-to-border line.
        let a = border_point(from, to.center());
        let b = border_point(to, from.center());
        [a, a.lerp(b, 1.0 / 3.0), a.lerp(b, 2.0 / 3.0), b]
    })
}

fn horizontal_curve(from: Rect, to: Rect) -> [Pos2; 4] {
    let rightward = to.center().x >= from.center().x;
    let (start_x, end_x) = if rightward {
        (from.max.x, to.min.x)
    } else {
        (from.min.x, to.max.x)
    };
    let start = pos2(
        start_x,
        anchor(from.center().y, to.center().y, from.height()),
    );
    let end = pos2(end_x, anchor(to.center().y, from.center().y, to.height()));
    let reach = ((end_x - start_x).abs() * 0.4).max(24.0) * if rightward { 1.0 } else { -1.0 };
    [
        start,
        pos2(start.x + reach, start.y),
        pos2(end.x - reach, end.y),
        end,
    ]
}

fn vertical_curve(from: Rect, to: Rect) -> [Pos2; 4] {
    let downward = to.center().y >= from.center().y;
    let (start_y, end_y) = if downward {
        (from.max.y, to.min.y)
    } else {
        (from.min.y, to.max.y)
    };
    let start = pos2(
        anchor(from.center().x, to.center().x, from.width()),
        start_y,
    );
    let end = pos2(anchor(to.center().x, from.center().x, to.width()), end_y);
    let reach = ((end_y - start_y).abs() * 0.4).max(24.0) * if downward { 1.0 } else { -1.0 };
    [
        start,
        pos2(start.x, start.y + reach),
        pos2(end.x, end.y - reach),
        end,
    ]
}

/// An attachment point along a box side: near the middle, pulled toward
/// the far endpoint so that parallel edges fan out instead of stacking.
fn anchor(own: f32, other: f32, extent: f32) -> f32 {
    let limit = (extent / 2.0 - 8.0).max(0.0);
    own + ((other - own) * 0.2).clamp(-limit, limit)
}

/// Where the line from the rect's center toward `target` leaves the rect.
fn border_point(rect: Rect, target: Pos2) -> Pos2 {
    let center = rect.center();
    let direction = target - center;
    let half = rect.size() / 2.0;
    let scale_x = if direction.x == 0.0 {
        f32::INFINITY
    } else {
        (half.x / direction.x).abs()
    };
    let scale_y = if direction.y == 0.0 {
        f32::INFINITY
    } else {
        (half.y / direction.y).abs()
    };
    let scale = scale_x.min(scale_y).min(1.0);
    center + direction * scale
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

fn world_bounds(layout: &Layout) -> Rect {
    layout
        .rects
        .values()
        .fold(Rect::NOTHING, |acc, rect| acc.union(*rect))
}
