//! The boundary canvas: draws the laid-out view and reports what the user
//! clicked. All state changes happen in the app shell; the canvas only
//! renders and hit-tests.

use cutaway_architecture::{ArchitectureGraph, ElementId, Relation};
use eframe::egui::{
    self, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, vec2,
};

use crate::layout::Layout;

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

pub const SEVERED: Color32 = Color32::from_rgb(205, 70, 60);
pub const DRAWN: Color32 = Color32::from_rgb(70, 165, 80);

pub fn show(
    ui: &mut Ui,
    view: &ArchitectureGraph,
    layout: &Layout,
    edges: &[EdgeVisual],
    selected_edge: Option<&Relation>,
    selected_node: Option<&ElementId>,
    draw_source: Option<&ElementId>,
) -> Option<CanvasAction> {
    let mut action = None;
    let painter = ui.painter().clone();
    let visuals = ui.visuals().clone();
    let base = visuals.text_color();

    // Containers first so nodes and edges paint over them.
    for id in &layout.containers {
        let rect = layout.rects[id];
        let selected = selected_node == Some(id);
        painter.rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(if selected { 2.5 } else { 1.0 }, base.gamma_multiply(0.6)),
            StrokeKind::Middle,
        );
        painter.text(
            rect.min + vec2(10.0, 6.0),
            egui::Align2::LEFT_TOP,
            name_of(view, id),
            FontId::proportional(13.0),
            base,
        );
        if response_for(ui, rect_header(rect), id).clicked() {
            action = Some(CanvasAction::Node(id.clone()));
        }
    }

    for edge in edges {
        let Some((from, to)) = endpoints(layout, &edge.relation) else {
            continue;
        };
        let selected = selected_edge == Some(&edge.relation);
        let color = match edge.status {
            EdgeStatus::Existing => base,
            EdgeStatus::Severed => SEVERED,
            EdgeStatus::Drawn => DRAWN,
        };
        let stroke = Stroke::new(if selected { 3.0 } else { 1.5 }, color);
        match edge.status {
            EdgeStatus::Existing => {
                painter.line_segment([from, to], stroke);
            }
            _ => {
                for shape in egui::Shape::dashed_line(&[from, to], stroke, 8.0, 5.0) {
                    painter.add(shape);
                }
            }
        }
        arrow_head(&painter, from, to, color);
        if edge.annotated {
            painter.circle_filled(from.lerp(to, 0.5), 4.0, color);
        }
    }

    for id in &layout.leaves {
        let rect = layout.rects[id];
        let selected = selected_node == Some(id) || draw_source == Some(id);
        painter.rect(
            rect,
            CornerRadius::same(5),
            visuals.extreme_bg_color,
            Stroke::new(if selected { 2.5 } else { 1.0 }, base.gamma_multiply(0.8)),
            StrokeKind::Middle,
        );
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            name_of(view, id),
            FontId::proportional(13.0),
            base,
        );
        if response_for(ui, rect, id).clicked() {
            action = Some(CanvasAction::Node(id.clone()));
        }
    }

    // Anything not caught by a node falls through to edge hit-testing.
    if action.is_none() {
        let background = ui.interact(
            ui.max_rect().union(bounds(layout)),
            ui.id().with("canvas-background"),
            Sense::click(),
        );
        if background.clicked() {
            action = background
                .interact_pointer_pos()
                .and_then(|pointer| nearest_edge(layout, edges, pointer))
                .map(CanvasAction::Edge)
                .or(Some(CanvasAction::Background));
        }
    }
    action
}

fn response_for(ui: &mut Ui, rect: Rect, id: &ElementId) -> egui::Response {
    ui.interact(rect, ui.id().with(id.as_str()), Sense::click())
}

fn rect_header(rect: Rect) -> Rect {
    Rect::from_min_size(rect.min, vec2(rect.width(), 24.0))
}

fn name_of(view: &ArchitectureGraph, id: &ElementId) -> String {
    view.element(id)
        .map_or_else(|| id.to_string(), |element| element.name.to_string())
}

fn endpoints(layout: &Layout, relation: &Relation) -> Option<(Pos2, Pos2)> {
    let from = layout.rects.get(&relation.from)?;
    let to = layout.rects.get(&relation.to)?;
    Some((
        border_point(*from, to.center()),
        border_point(*to, from.center()),
    ))
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

fn arrow_head(painter: &egui::Painter, from: Pos2, to: Pos2, color: Color32) {
    let direction = (to - from).normalized();
    let normal = vec2(-direction.y, direction.x);
    let tip = to;
    let left = tip - direction * 10.0 + normal * 5.0;
    let right = tip - direction * 10.0 - normal * 5.0;
    painter.add(egui::Shape::convex_polygon(
        vec![tip, left, right],
        color,
        Stroke::NONE,
    ));
}

fn nearest_edge(layout: &Layout, edges: &[EdgeVisual], pointer: Pos2) -> Option<Relation> {
    let mut best: Option<(f32, &Relation)> = None;
    for edge in edges {
        let Some((from, to)) = endpoints(layout, &edge.relation) else {
            continue;
        };
        let distance = distance_to_segment(pointer, from, to);
        if distance < 8.0 && best.is_none_or(|(closest, _)| distance < closest) {
            best = Some((distance, &edge.relation));
        }
    }
    best.map(|(_, relation)| relation.clone())
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

fn bounds(layout: &Layout) -> Rect {
    layout
        .rects
        .values()
        .fold(Rect::NOTHING, |acc, rect| acc.union(*rect))
}
