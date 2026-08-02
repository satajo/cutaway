//! Where a dependency edge runs, and how much of the picture it crosses.
//!
//! An edge leaves the side of its dependent that faces the dependency and
//! enters the far side facing back. Every edge that meets one side of one
//! box shares that side with the others: the attachment points spread
//! evenly along it, so a boundary that eight edges reach receives eight
//! arrowheads instead of one pile. Within a side the anchors order by the
//! far end of each edge, so edges of one group never cross each other.
//!
//! The routing also answers how far an edge reaches: an edge between two
//! leaves of one top-level boundary is that boundary's internal wiring,
//! while an edge between two top-level boundaries is the architecture the
//! picture is about. The canvas draws the two differently; it does not
//! decide which is which.
//!
//! The whole computation is pure geometry and containment over the laid-out
//! view, so it is unit-testable without a screen.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{ArchitectureGraph, ElementId, Relation, RelationKind};
use eframe::egui::{Pos2, Rect, pos2};

use crate::layout::Layout;

/// How much of the picture one edge crosses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    /// Both ends sit inside one top-level boundary: internal wiring.
    Intra,
    /// The ends sit in different top-level boundaries: a crossing the
    /// architecture is made of.
    Cross,
}

/// One edge as the canvas draws it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Route {
    /// The cubic control points, in world coordinates.
    pub(crate) curve: [Pos2; 4],
    pub(crate) scope: Scope,
}

/// How close to a corner an anchor may come.
const CORNER_MARGIN: f32 = 8.0;
/// How far a curve leaves its box along the travel direction, as a share of
/// the distance it covers.
const REACH: f32 = 0.4;
/// The shortest such departure, so even neighboring boxes meet squarely.
const MIN_REACH: f32 = 24.0;

/// The route of every edge, in the order the edges arrive. An edge whose
/// endpoints the layout does not place has no route.
pub(crate) fn routes<'a>(
    view: &ArchitectureGraph,
    layout: &Layout,
    edges: impl IntoIterator<Item = &'a Relation>,
) -> Vec<Option<Route>> {
    let edges: Vec<&Relation> = edges.into_iter().collect();
    let scopes = scopes(view, &edges);
    curves(layout, &edges)
        .into_iter()
        .zip(scopes)
        .map(|(curve, scope)| curve.map(|curve| Route { curve, scope }))
        .collect()
}

/// The side of a box an edge attaches to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// Which end of an edge an attachment belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum End {
    From,
    To,
}

/// The sides one edge attaches to, and the sort keys that order its anchors
/// among the other edges on those sides.
#[derive(Debug, Clone, Copy)]
struct Sides {
    from: Side,
    to: Side,
    /// Where the far end of the edge sits along the departure side.
    from_key: f32,
    /// Where the near end sits along the arrival side.
    to_key: f32,
}

/// How one edge reaches its dependency once the boxes are known.
enum Plan {
    /// Boxes clear of each other: the anchors come from the shared sides.
    Sided(Sides),
    /// Overlapping boxes, e.g. an edge into a surrounding container: a
    /// straight border-to-border line, which no side can spread.
    Straight([Pos2; 4]),
}

/// One edge's claim on one side of one box.
struct Slot {
    key: f32,
    edge: usize,
    end: End,
}

fn curves(layout: &Layout, edges: &[&Relation]) -> Vec<Option<[Pos2; 4]>> {
    let mut plans: Vec<Option<Plan>> = Vec::with_capacity(edges.len());
    let mut claims: BTreeMap<(&ElementId, Side), Vec<Slot>> = BTreeMap::new();
    for (index, edge) in edges.iter().enumerate() {
        let (Some(from), Some(to)) = (layout.rects.get(&edge.from), layout.rects.get(&edge.to))
        else {
            plans.push(None);
            continue;
        };
        let Some(sides) = facing_sides(*from, *to) else {
            plans.push(Some(Plan::Straight(straight(*from, *to))));
            continue;
        };
        claims
            .entry((&edge.from, sides.from))
            .or_default()
            .push(Slot {
                key: sides.from_key,
                edge: index,
                end: End::From,
            });
        claims.entry((&edge.to, sides.to)).or_default().push(Slot {
            key: sides.to_key,
            edge: index,
            end: End::To,
        });
        plans.push(Some(Plan::Sided(sides)));
    }

    let anchors = spread(layout, claims);
    plans
        .into_iter()
        .enumerate()
        .map(|(index, plan)| match plan? {
            Plan::Straight(curve) => Some(curve),
            Plan::Sided(sides) => {
                let start = *anchors.get(&(index, End::From))?;
                let end = *anchors.get(&(index, End::To))?;
                Some(bend(start, end, sides.from))
            }
        })
        .collect()
}

/// Places the edges that claim one side of one box evenly along it. The
/// order follows the far end of each edge, so the fan neither crosses
/// itself nor depends on the order the edges arrived in; a single claim
/// lands in the middle of the side.
fn spread(
    layout: &Layout,
    claims: BTreeMap<(&ElementId, Side), Vec<Slot>>,
) -> BTreeMap<(usize, End), Pos2> {
    let mut anchors = BTreeMap::new();
    for ((id, side), mut slots) in claims {
        let Some(rect) = layout.rects.get(id) else {
            continue;
        };
        slots.sort_by(|a, b| {
            a.key
                .total_cmp(&b.key)
                .then_with(|| a.edge.cmp(&b.edge))
                .then_with(|| a.end.cmp(&b.end))
        });
        let (low, high) = usable(*rect, side);
        let steps = count(slots.len() + 1);
        for (position, slot) in slots.iter().enumerate() {
            let share = count(position + 1) / steps;
            anchors.insert(
                (slot.edge, slot.end),
                attach(*rect, side, low + (high - low) * share),
            );
        }
    }
    anchors
}

/// The sides two clear boxes turn toward each other, or None when the boxes
/// overlap and no side faces the other.
fn facing_sides(from: Rect, to: Rect) -> Option<Sides> {
    if to.min.x > from.max.x || from.min.x > to.max.x {
        let rightward = to.center().x >= from.center().x;
        Some(Sides {
            from: if rightward { Side::Right } else { Side::Left },
            to: if rightward { Side::Left } else { Side::Right },
            from_key: to.center().y,
            to_key: from.center().y,
        })
    } else if to.min.y > from.max.y || from.min.y > to.max.y {
        let downward = to.center().y >= from.center().y;
        Some(Sides {
            from: if downward { Side::Bottom } else { Side::Top },
            to: if downward { Side::Top } else { Side::Bottom },
            from_key: to.center().x,
            to_key: from.center().x,
        })
    } else {
        None
    }
}

/// The stretch of a side anchors may attach to: the side less its corners,
/// or the middle alone when the side is too short to keep the corners
/// clear.
fn usable(rect: Rect, side: Side) -> (f32, f32) {
    let (low, high) = match side {
        Side::Left | Side::Right => (rect.min.y, rect.max.y),
        Side::Top | Side::Bottom => (rect.min.x, rect.max.x),
    };
    if high - low <= 2.0 * CORNER_MARGIN {
        let middle = f32::midpoint(low, high);
        return (middle, middle);
    }
    (low + CORNER_MARGIN, high - CORNER_MARGIN)
}

fn attach(rect: Rect, side: Side, along: f32) -> Pos2 {
    match side {
        Side::Left => pos2(rect.min.x, along),
        Side::Right => pos2(rect.max.x, along),
        Side::Top => pos2(along, rect.min.y),
        Side::Bottom => pos2(along, rect.max.y),
    }
}

/// The cubic between two anchors: it leaves and enters along the direction
/// of travel, so the curve meets both boxes square to their sides.
fn bend(start: Pos2, end: Pos2, leaving: Side) -> [Pos2; 4] {
    match leaving {
        Side::Left | Side::Right => {
            let reach = departure(end.x - start.x, leaving == Side::Right);
            [
                start,
                pos2(start.x + reach, start.y),
                pos2(end.x - reach, end.y),
                end,
            ]
        }
        Side::Top | Side::Bottom => {
            let reach = departure(end.y - start.y, leaving == Side::Bottom);
            [
                start,
                pos2(start.x, start.y + reach),
                pos2(end.x, end.y - reach),
                end,
            ]
        }
    }
}

fn departure(distance: f32, forward: bool) -> f32 {
    (distance.abs() * REACH).max(MIN_REACH) * if forward { 1.0 } else { -1.0 }
}

fn straight(from: Rect, to: Rect) -> [Pos2; 4] {
    let a = border_point(from, to.center());
    let b = border_point(to, from.center());
    [a, a.lerp(b, 1.0 / 3.0), a.lerp(b, 2.0 / 3.0), b]
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

/// How far each edge reaches, by the top-level boundary its ends sit in.
fn scopes(view: &ArchitectureGraph, edges: &[&Relation]) -> Vec<Scope> {
    let mut frame_of: BTreeMap<&ElementId, &ElementId> = BTreeMap::new();
    for relation in view.relations() {
        if relation.kind == RelationKind::Contains {
            frame_of.insert(&relation.to, &relation.from);
        }
    }
    // Containment of a view is a tree, but a walk that trusts that and meets
    // a cycle never ends; the seen set bounds every walk.
    let root_of = |id: &ElementId| -> ElementId {
        let mut current = id.clone();
        let mut seen = BTreeSet::new();
        while let Some(frame) = frame_of.get(&current) {
            if !seen.insert((*frame).clone()) {
                break;
            }
            current = (*frame).clone();
        }
        current
    };
    edges
        .iter()
        .map(|edge| {
            if root_of(&edge.from) == root_of(&edge.to) {
                Scope::Intra
            } else {
                Scope::Cross
            }
        })
        .collect()
}

fn count(value: usize) -> f32 {
    f32::from(u16::try_from(value).unwrap_or(u16::MAX))
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{Element, ElementKind, ElementName};

    use super::*;

    fn id(text: &str) -> ElementId {
        ElementId::new(text).unwrap()
    }

    fn depends(from: &str, to: &str) -> Relation {
        Relation {
            from: id(from),
            to: id(to),
            kind: RelationKind::DependsOn,
        }
    }

    fn boxes(placed: &[(&str, Rect)]) -> Layout {
        Layout {
            rects: placed
                .iter()
                .map(|(name, rect)| (id(name), *rect))
                .collect(),
            containers: Vec::new(),
            leaves: placed.iter().map(|(name, _)| id(name)).collect(),
        }
    }

    fn drawn(layout: &Layout, edges: &[Relation]) -> Vec<[Pos2; 4]> {
        let edges: Vec<&Relation> = edges.iter().collect();
        curves(layout, &edges)
            .into_iter()
            .map(|curve| curve.expect("every box in this layout is placed"))
            .collect()
    }

    /// Three boxes on the left, stacked top to bottom, and one tall box to
    /// the right of all of them.
    fn fan() -> Layout {
        boxes(&[
            ("top", Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 40.0))),
            (
                "middle",
                Rect::from_min_max(pos2(0.0, 120.0), pos2(100.0, 160.0)),
            ),
            (
                "bottom",
                Rect::from_min_max(pos2(0.0, 240.0), pos2(100.0, 280.0)),
            ),
            (
                "target",
                Rect::from_min_max(pos2(300.0, 0.0), pos2(400.0, 280.0)),
            ),
        ])
    }

    #[test]
    fn edges_arriving_at_one_side_spread_along_it() {
        let layout = fan();
        let curves = drawn(
            &layout,
            &[
                depends("top", "target"),
                depends("middle", "target"),
                depends("bottom", "target"),
            ],
        );

        let arrivals: Vec<Pos2> = curves.iter().map(|curve| curve[3]).collect();
        for arrival in &arrivals {
            assert!(
                (arrival.x - 300.0).abs() < 0.1,
                "every edge enters the target's left side"
            );
            assert!(
                (8.0..=272.0).contains(&arrival.y),
                "an anchor keeps clear of the corners"
            );
        }
        assert!(
            arrivals[0].y < arrivals[1].y && arrivals[1].y < arrivals[2].y,
            "the higher the dependent, the higher its arrowhead: {arrivals:?}"
        );
    }

    #[test]
    fn edges_leaving_one_side_spread_along_it() {
        let layout = boxes(&[
            (
                "source",
                Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 280.0)),
            ),
            (
                "upper",
                Rect::from_min_max(pos2(300.0, 0.0), pos2(400.0, 40.0)),
            ),
            (
                "lower",
                Rect::from_min_max(pos2(300.0, 240.0), pos2(400.0, 280.0)),
            ),
        ]);
        let curves = drawn(
            &layout,
            &[depends("source", "upper"), depends("source", "lower")],
        );

        let departures: Vec<Pos2> = curves.iter().map(|curve| curve[0]).collect();
        for departure in &departures {
            assert!(
                (departure.x - 100.0).abs() < 0.1,
                "every edge leaves the source's right side"
            );
        }
        assert!(
            departures[0].y < departures[1].y,
            "the edge to the upper partner leaves higher: {departures:?}"
        );
    }

    #[test]
    fn a_lone_edge_still_anchors_near_the_center() {
        let layout = fan();
        let curves = drawn(&layout, &[depends("middle", "target")]);

        let curve = curves[0];
        assert!(
            (curve[0].y - 140.0).abs() < 1.0,
            "the edge leaves the middle of its dependent's side"
        );
        assert!(
            (curve[3].y - 140.0).abs() < 1.0,
            "the edge enters the middle of its dependency's side"
        );
    }

    #[test]
    fn an_edge_between_boundaries_reaches_further_than_one_inside_a_boundary() {
        let mut view = ArchitectureGraph::new();
        for (element, kind) in [
            ("package:a", ElementKind::Package),
            ("package:b", ElementKind::Package),
            ("a/one", ElementKind::Module),
            ("a/two", ElementKind::Module),
            ("b/one", ElementKind::Module),
        ] {
            view.add_element(Element {
                id: id(element),
                name: ElementName::new(element).unwrap(),
                kind,
            })
            .unwrap();
        }
        for (frame, inner) in [
            ("package:a", "a/one"),
            ("package:a", "a/two"),
            ("package:b", "b/one"),
        ] {
            view.add_relation(Relation {
                from: id(frame),
                to: id(inner),
                kind: RelationKind::Contains,
            })
            .unwrap();
        }

        let edges = [depends("a/one", "a/two"), depends("a/two", "b/one")];
        let borrowed: Vec<&Relation> = edges.iter().collect();
        assert_eq!(scopes(&view, &borrowed), vec![Scope::Intra, Scope::Cross]);
    }
}
