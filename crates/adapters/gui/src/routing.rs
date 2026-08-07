//! Where a dependency edge runs, and how much of the picture it crosses.
//!
//! An edge leaves the side of its dependent that faces the dependency and
//! enters the far side facing back. Every edge that meets one side of one
//! box shares that side with the others: the attachment points spread
//! evenly along it, so a boundary that eight edges reach receives eight
//! arrowheads instead of one pile. Within a side the anchors order by the
//! far end of each edge, so edges of one group never cross each other.
//!
//! A long edge does not take the straight way. A curve drawn from one end
//! of the picture to the other cuts through every boundary in between, and
//! a reader who follows it must decide, box by box, that none of them is a
//! party to it. An edge between boundaries therefore rounds whatever stands
//! between its ends, so a line that enters a box is a line that means that
//! box.
//!
//! The routing also answers how far an edge reaches: an edge between two
//! leaves of one top-level boundary is that boundary's internal wiring,
//! while an edge between two top-level boundaries is the architecture the
//! picture is about. The canvas draws the two differently; it does not
//! decide which is which.
//!
//! Last, it answers how much company an edge keeps: the busiest side it
//! attaches to, counted in edges. Twenty arrivals spread along one border
//! are twenty strokes over one box however well they are spread, and their
//! ink together says more than any of them means. The routing measures the
//! crowd; the canvas decides what to do about it.
//!
//! The whole computation is pure geometry and containment over the laid-out
//! view, so it is unit-testable without a screen.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{ArchitectureGraph, ElementId, Relation, RelationKind};
use eframe::egui::{Pos2, Rect, Vec2, pos2};

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

/// The line one edge draws: cubics joined end to end, the tangent shared at
/// every joint, so a run that rounds a boundary still reads as one stroke.
/// A run that meets nothing is a single cubic, exactly as a picture without
/// obstacles has always drawn.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Path {
    /// The legs of the run, in travel order. Never empty.
    legs: Vec<[Pos2; 4]>,
}

impl Path {
    fn single(curve: [Pos2; 4]) -> Self {
        Self { legs: vec![curve] }
    }

    /// A run of several legs, or None when there are no legs to run: a path
    /// is a line, and a line has at least one piece.
    fn joined(legs: Vec<[Pos2; 4]>) -> Option<Self> {
        (!legs.is_empty()).then_some(Self { legs })
    }

    /// The run as a polyline, `steps` straight pieces per leg. One walk
    /// answers both the paint and the pointer, so the line the reader sees
    /// is exactly the line the reader can catch.
    pub(crate) fn points(&self, steps: u16) -> Vec<Pos2> {
        let mut points: Vec<Pos2> = Vec::with_capacity(self.legs.len() * (usize::from(steps) + 1));
        for leg in &self.legs {
            // Neighboring legs meet at a point they share; it belongs to the
            // polyline once.
            let already = usize::from(!points.is_empty());
            points.extend(sample(*leg, steps).into_iter().skip(already));
        }
        points
    }
}

/// One edge as the canvas draws it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Route {
    /// The line the edge follows, in world coordinates.
    pub(crate) path: Path,
    pub(crate) scope: Scope,
    /// How many edges share the busiest side this one attaches to, itself
    /// counted. One means the edge meets its boxes alone.
    pub(crate) crowd: usize,
}

/// How close to a corner an anchor may come.
const CORNER_MARGIN: f32 = 8.0;
/// How far a curve leaves its box along the travel direction, as a share of
/// the distance it covers.
const REACH: f32 = 0.4;
/// The shortest such departure, so even neighboring boxes meet squarely.
const MIN_REACH: f32 = 24.0;
/// How wide a berth a run gives the boundary it rounds: far enough that the
/// gap reads as deliberate rather than as a near miss, near enough that the
/// way around still reads as the shortest way past. The rounding of a
/// corner may enter this band; the box itself stays clear of the ink.
const CLEARANCE: f32 = 16.0;
/// Straight probes a curve is walked as when asking whether it runs through
/// a box. The answer only chooses between a detour and none, so a coarse
/// walk decides it.
const PROBES: u16 = 16;

/// The route of every edge, in the order the edges arrive. An edge whose
/// endpoints the layout does not place has no route, and neither has one
/// whose ends attach to the same box.
///
/// `stands_for` names, for every element the picture does not paint, the box
/// that stands for it: an edge into such an element lands on that box
/// instead. The routing does not ask why an element is unpainted, and an
/// empty map draws every edge to the box it names.
pub(crate) fn routes<'a>(
    view: &ArchitectureGraph,
    layout: &Layout,
    stands_for: &'a BTreeMap<ElementId, ElementId>,
    edges: impl IntoIterator<Item = &'a Relation>,
) -> Vec<Option<Route>> {
    let edges: Vec<&'a Relation> = edges.into_iter().collect();
    let roots = Roots::of(view);
    let scopes = scopes(&roots, &edges);
    let obstacles = Obstacles::of(layout, &roots);
    paths(layout, stands_for, &edges, &scopes, &roots, &obstacles)
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
enum Plan<'a> {
    /// Boxes clear of each other: the anchors come from the shared sides,
    /// and the run may have to round what stands between the two boxes.
    Sided {
        sides: Sides,
        /// The boxes the ends attach to: what the run leaves and enters, and
        /// therefore what can never be in its way.
        tail: &'a ElementId,
        head: &'a ElementId,
    },
    /// Overlapping boxes, e.g. an edge into a surrounding container: a
    /// straight border-to-border line, which no side can spread and nothing
    /// can be in the way of, the two boxes already sharing their ground.
    Straight([Pos2; 4]),
}

/// One edge's claim on one side of one box.
struct Slot {
    key: f32,
    edge: usize,
    end: End,
}

/// Where one end of one edge meets its box, and how many ends meet that
/// same side.
#[derive(Debug, Clone, Copy)]
struct Anchor {
    at: Pos2,
    crowd: usize,
}

fn paths<'a>(
    layout: &Layout,
    stands_for: &'a BTreeMap<ElementId, ElementId>,
    edges: &[&'a Relation],
    scopes: &[Scope],
    roots: &Roots<'_>,
    obstacles: &Obstacles<'_>,
) -> Vec<Option<Route>> {
    let mut plans: Vec<Option<Plan<'a>>> = Vec::with_capacity(edges.len());
    let mut claims: BTreeMap<(&'a ElementId, Side), Vec<Slot>> = BTreeMap::new();
    for (index, edge) in edges.iter().enumerate() {
        let (Some((tail, from)), Some((head, to))) = (
            attachment(layout, stands_for, &edge.from),
            attachment(layout, stands_for, &edge.to),
        ) else {
            plans.push(None);
            continue;
        };
        // One box carrying both ends already stands for the whole
        // dependency, and a line from a box to itself says nothing.
        if tail == head {
            plans.push(None);
            continue;
        }
        let Some(sides) = facing_sides(from, to) else {
            plans.push(Some(Plan::Straight(straight(from, to))));
            continue;
        };
        claims.entry((tail, sides.from)).or_default().push(Slot {
            key: sides.from_key,
            edge: index,
            end: End::From,
        });
        claims.entry((head, sides.to)).or_default().push(Slot {
            key: sides.to_key,
            edge: index,
            end: End::To,
        });
        plans.push(Some(Plan::Sided { sides, tail, head }));
    }

    let anchors = spread(layout, claims);
    plans
        .into_iter()
        .enumerate()
        .map(|(index, plan)| {
            let scope = scopes[index];
            match plan? {
                Plan::Straight(curve) => Some(Route {
                    path: Path::single(curve),
                    scope,
                    crowd: 1,
                }),
                Plan::Sided { sides, tail, head } => {
                    let start = *anchors.get(&(index, End::From))?;
                    let end = *anchors.get(&(index, End::To))?;
                    let path = match scope {
                        // An edge inside one boundary runs between siblings
                        // of one frame: it is short, it stays on its own
                        // frame's ground, and the boxes it passes are the
                        // very ones it belongs among. It keeps the plain
                        // curve.
                        Scope::Intra => Path::single(bend(start.at, end.at, sides.from)),
                        Scope::Cross => around(
                            start.at,
                            end.at,
                            sides.from,
                            &obstacles.between(roots.root_of(tail), roots.root_of(head)),
                        ),
                    };
                    Some(Route {
                        path,
                        scope,
                        crowd: start.crowd.max(end.crowd),
                    })
                }
            }
        })
        .collect()
}

/// The box one end of an edge attaches to, and where that box sits: the
/// element's own box, or the one that stands for it while the picture leaves
/// it unpainted. None while no box of either name is placed.
fn attachment<'a>(
    layout: &Layout,
    stands_for: &'a BTreeMap<ElementId, ElementId>,
    end: &'a ElementId,
) -> Option<(&'a ElementId, Rect)> {
    let attached = stands_for.get(end).unwrap_or(end);
    Some((attached, *layout.rects.get(attached)?))
}

/// Places the edges that claim one side of one box evenly along it. The
/// order follows the far end of each edge, so the fan neither crosses
/// itself nor depends on the order the edges arrived in; a single claim
/// lands in the middle of the side. Every anchor carries the size of the
/// group it was placed with, which is what the canvas reads as the crowd.
fn spread(
    layout: &Layout,
    claims: BTreeMap<(&ElementId, Side), Vec<Slot>>,
) -> BTreeMap<(usize, End), Anchor> {
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
                Anchor {
                    at: attach(*rect, side, low + (high - low) * share),
                    crowd: slots.len(),
                },
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

/// The direction an edge leaving this side travels in, which is also the
/// direction it enters its dependency from: the run meets both boxes square
/// to the sides that face each other.
fn travel(leaving: Side) -> Vec2 {
    match leaving {
        Side::Left => Vec2::new(-1.0, 0.0),
        Side::Right => Vec2::new(1.0, 0.0),
        Side::Top => Vec2::new(0.0, -1.0),
        Side::Bottom => Vec2::new(0.0, 1.0),
    }
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

/// The run an edge takes past whatever stands between its ends: the plain
/// curve while nothing does, and otherwise a way around everything that
/// curve would cross.
///
/// The blockers answer as one region: the run rounds the union of every box
/// the plain curve meets. Two blockers with a clear gap between them
/// therefore cost one longer way around instead of a thread between them.
/// That is the trade the picture takes: one decision per edge, a shape the
/// reader can predict, and no route that squeezes through a gap it only
/// found by searching.
fn around(start: Pos2, end: Pos2, leaving: Side, blockers: &[Rect]) -> Path {
    let direct = bend(start, end, leaving);
    let probes = sample(direct, PROBES);
    let blocked = blockers
        .iter()
        .copied()
        .filter(|blocker| meets(&probes, *blocker))
        .reduce(Rect::union);
    let rounded = blocked
        .and_then(|blocked| lane(start, end, leaving, blocked))
        .and_then(|(first, second)| smooth(&[start, first, second, end], travel(leaving)));
    rounded.unwrap_or_else(|| Path::single(direct))
}

/// The two waypoints that carry an edge past a blocked region: out to one
/// free side of it, along that side, and back in. A mostly horizontal edge
/// passes above or below, a mostly vertical one left or right - the
/// departure side says which, since it already faces the dependency. The
/// side that adds the least length wins, and a tie goes to the first named
/// one, so one picture always draws the same way.
///
/// None while the region cannot be rounded in the direction of travel, e.g.
/// when an end already lies past the region it would round. The plain curve
/// then stands.
fn lane(start: Pos2, end: Pos2, leaving: Side, blocked: Rect) -> Option<(Pos2, Pos2)> {
    match leaving {
        Side::Left | Side::Right => {
            let (enter, exit) = along(
                start.x,
                end.x,
                (blocked.min.x, blocked.max.x),
                leaving == Side::Right,
            )?;
            let over = (pos2(enter, blocked.min.y), pos2(exit, blocked.min.y));
            let under = (pos2(enter, blocked.max.y), pos2(exit, blocked.max.y));
            Some(shorter(start, end, over, under))
        }
        Side::Top | Side::Bottom => {
            let (enter, exit) = along(
                start.y,
                end.y,
                (blocked.min.y, blocked.max.y),
                leaving == Side::Bottom,
            )?;
            let before = (pos2(blocked.min.x, enter), pos2(blocked.min.x, exit));
            let after = (pos2(blocked.max.x, enter), pos2(blocked.max.x, exit));
            Some(shorter(start, end, before, after))
        }
    }
}

/// Where a lane past a region begins and ends along the direction of
/// travel: at the near edge of the region, or at the end itself once that
/// end already lies within the region's span - an edge that starts beside
/// what it rounds leaves sideways instead of doubling back to the corner.
/// None once the two would cross, which is an end already past the region.
fn along(from: f32, to: f32, region: (f32, f32), forward: bool) -> Option<(f32, f32)> {
    let (low, high) = region;
    let (enter, exit) = if forward {
        (from.max(low), to.min(high))
    } else {
        (from.min(high), to.max(low))
    };
    let ordered = if forward {
        enter <= exit
    } else {
        exit <= enter
    };
    ordered.then_some((enter, exit))
}

fn shorter(start: Pos2, end: Pos2, a: (Pos2, Pos2), b: (Pos2, Pos2)) -> (Pos2, Pos2) {
    if detour_length(start, end, a) <= detour_length(start, end, b) {
        a
    } else {
        b
    }
}

fn detour_length(start: Pos2, end: Pos2, way: (Pos2, Pos2)) -> f32 {
    (way.0 - start).length() + (way.1 - way.0).length() + (end - way.1).length()
}

/// Whether a walked curve runs through a box. One walk answers for every
/// box the edge is asked about.
fn meets(probes: &[Pos2], box_of: Rect) -> bool {
    probes
        .windows(2)
        .any(|probe| crosses(probe[0], probe[1], box_of))
}

/// Whether the segment from `a` to `b` touches the box, by clipping the
/// segment against each of the four borders in turn: the piece that
/// survives all four is the piece inside.
fn crosses(a: Pos2, b: Pos2, box_of: Rect) -> bool {
    let step = b - a;
    let (mut entry, mut exit) = (0.0_f32, 1.0_f32);
    for (rate, room) in [
        (-step.x, a.x - box_of.min.x),
        (step.x, box_of.max.x - a.x),
        (-step.y, a.y - box_of.min.y),
        (step.y, box_of.max.y - a.y),
    ] {
        if rate == 0.0 {
            // Parallel to this border: either always inside it or never.
            if room < 0.0 {
                return false;
            }
            continue;
        }
        let crossing = room / rate;
        if rate < 0.0 {
            if crossing > exit {
                return false;
            }
            entry = entry.max(crossing);
        } else {
            if crossing < entry {
                return false;
            }
            exit = exit.min(crossing);
        }
    }
    entry <= exit
}

/// The polyline drawn as one smooth run: a cubic per leg, and at every
/// joint the two legs that meet there share a tangent, so the stroke bends
/// without kinking. The ends leave and enter along `travel`, the direction
/// the two boxes face, so the run still meets both of them square to their
/// sides. A joint's tangent is no longer than the shorter leg beside it,
/// which keeps the rounding of a corner within the corner.
///
/// None while the line has no length to draw.
fn smooth(through: &[Pos2], travel: Vec2) -> Option<Path> {
    let points = distinct(through);
    let last = points.len().checked_sub(1).filter(|last| *last > 0)?;
    let tangents: Vec<Vec2> = points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            if index == 0 {
                travel * (points[1] - *point).length()
            } else if index == last {
                travel * (*point - points[last - 1]).length()
            } else {
                let behind = *point - points[index - 1];
                let ahead = points[index + 1] - *point;
                let reach = behind.length().min(ahead.length());
                let across = behind + ahead;
                if across.length() > 0.0 {
                    across.normalized() * reach
                } else {
                    travel * reach
                }
            }
        })
        .collect();
    let legs = (0..last)
        .map(|leg| {
            [
                points[leg],
                points[leg] + tangents[leg] / 3.0,
                points[leg + 1] - tangents[leg + 1] / 3.0,
                points[leg + 1],
            ]
        })
        .collect();
    Path::joined(legs)
}

/// The points with every repeat of the one before it dropped: two points in
/// the same place name no direction, and a leg between them would cusp.
fn distinct(points: &[Pos2]) -> Vec<Pos2> {
    let mut kept: Vec<Pos2> = Vec::with_capacity(points.len());
    for point in points {
        if kept
            .last()
            .is_none_or(|last| (*point - *last).length() > 0.5)
        {
            kept.push(*point);
        }
    }
    kept
}

fn sample(curve: [Pos2; 4], steps: u16) -> Vec<Pos2> {
    let steps = steps.max(1);
    (0..=steps)
        .map(|step| at(curve, f32::from(step) / f32::from(steps)))
        .collect()
}

fn at(c: [Pos2; 4], t: f32) -> Pos2 {
    let u = 1.0 - t;
    (c[0].to_vec2() * (u * u * u)
        + c[1].to_vec2() * (3.0 * u * u * t)
        + c[2].to_vec2() * (3.0 * u * t * t)
        + c[3].to_vec2() * (t * t * t))
        .to_pos2()
}

/// The top-level boundary every element belongs to.
struct Roots<'a> {
    frame_of: BTreeMap<&'a ElementId, &'a ElementId>,
}

impl<'a> Roots<'a> {
    fn of(view: &'a ArchitectureGraph) -> Self {
        let mut frame_of = BTreeMap::new();
        for relation in view.relations() {
            if relation.kind == RelationKind::Contains {
                frame_of.insert(&relation.to, &relation.from);
            }
        }
        Self { frame_of }
    }

    /// The outermost boundary around an element, which is the element itself
    /// while nothing contains it.
    ///
    /// Containment of a view is a tree, but a walk that trusts that and
    /// meets a cycle never ends; the seen set bounds every walk.
    fn root_of<'b>(&'b self, id: &'b ElementId) -> &'b ElementId {
        let mut current = id;
        let mut seen = BTreeSet::new();
        while let Some(frame) = self.frame_of.get(current) {
            if !seen.insert(*frame) {
                break;
            }
            current = frame;
        }
        current
    }
}

/// The boxes a crossing edge must not run through: every top-level
/// boundary, grown by the clearance a run keeps from it.
///
/// Only top-level boundaries obstruct. What sits inside one is that
/// boundary's own business, and a run that also rounded every leaf would
/// turn a picture of a system into a maze.
struct Obstacles<'a> {
    boxes: Vec<(&'a ElementId, Rect)>,
}

impl<'a> Obstacles<'a> {
    fn of(layout: &'a Layout, roots: &Roots<'_>) -> Self {
        Self {
            boxes: layout
                .rects
                .iter()
                .filter(|(id, _)| roots.root_of(id) == *id)
                .map(|(id, rect)| (id, rect.expand(CLEARANCE)))
                .collect(),
        }
    }

    /// What stands between two boundaries: every top-level box but the two
    /// the edge itself belongs to. An edge leaves its own boundary and
    /// enters the other, so neither of them is ever in its way.
    fn between(&self, from: &ElementId, to: &ElementId) -> Vec<Rect> {
        self.boxes
            .iter()
            .filter(|(id, _)| *id != from && *id != to)
            .map(|(_, rect)| *rect)
            .collect()
    }
}

/// How far each edge reaches, by the top-level boundary its ends sit in.
fn scopes(roots: &Roots<'_>, edges: &[&Relation]) -> Vec<Scope> {
    edges
        .iter()
        .map(|edge| {
            if roots.root_of(&edge.from) == roots.root_of(&edge.to) {
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
impl Path {
    /// How many cubics the run is made of: one while nothing bent it.
    fn legs(&self) -> usize {
        self.legs.len()
    }

    fn start(&self) -> Pos2 {
        self.legs[0][0]
    }

    fn end(&self) -> Pos2 {
        self.legs[self.legs.len() - 1][3]
    }
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

    /// A picture that paints every box it names.
    fn nothing_stands_for_anything() -> BTreeMap<ElementId, ElementId> {
        BTreeMap::new()
    }

    /// A picture of boundaries that contain nothing: every box stands on the
    /// canvas itself.
    fn flat() -> ArchitectureGraph {
        ArchitectureGraph::new()
    }

    fn routed(view: &ArchitectureGraph, layout: &Layout, edges: &[Relation]) -> Vec<Option<Route>> {
        routes(view, layout, &nothing_stands_for_anything(), edges.iter())
    }

    fn drawn(layout: &Layout, edges: &[Relation]) -> Vec<Path> {
        routed(&flat(), layout, edges)
            .into_iter()
            .map(|route| route.expect("every box in this layout is placed").path)
            .collect()
    }

    /// The one run of a one-edge picture, walked as the canvas walks it.
    fn walk(layout: &Layout, edges: &[Relation]) -> Vec<Pos2> {
        drawn(layout, edges)[0].points(24)
    }

    /// A wide box with two small boxes inside it, and one box to the left of
    /// all three.
    fn frame_and_neighbor() -> Layout {
        boxes(&[
            (
                "neighbor",
                Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 40.0)),
            ),
            (
                "frame",
                Rect::from_min_max(pos2(300.0, 0.0), pos2(400.0, 200.0)),
            ),
            (
                "inside",
                Rect::from_min_max(pos2(320.0, 20.0), pos2(380.0, 60.0)),
            ),
            (
                "beside",
                Rect::from_min_max(pos2(320.0, 120.0), pos2(380.0, 160.0)),
            ),
        ])
    }

    fn hidden_in_the_frame() -> BTreeMap<ElementId, ElementId> {
        BTreeMap::from([(id("inside"), id("frame")), (id("beside"), id("frame"))])
    }

    #[test]
    fn an_edge_into_a_summarized_frame_lands_on_its_border() {
        let layout = frame_and_neighbor();
        let edges = [depends("neighbor", "inside")];

        let route = routes(&flat(), &layout, &hidden_in_the_frame(), edges.iter())[0]
            .clone()
            .expect("the frame that stands for the endpoint is placed");
        let arrival = route.path.end();
        assert!(
            (arrival.x - 300.0).abs() < 0.1,
            "the edge lands on the left border of the frame, not on the box it hides: {arrival:?}"
        );
        assert!(
            (0.0..=200.0).contains(&arrival.y),
            "the arrival sits along that border: {arrival:?}"
        );
    }

    #[test]
    fn an_edge_ending_at_a_frame_with_painted_children_lands_on_its_border() {
        let mut view = ArchitectureGraph::new();
        for element in ["neighbor", "frame", "inside", "beside"] {
            view.add_element(Element {
                id: id(element),
                name: ElementName::new(element).unwrap(),
                kind: ElementKind::Module,
                fingerprint: None,
            })
            .unwrap();
        }
        for inner in ["inside", "beside"] {
            view.add_relation(Relation {
                from: id("frame"),
                to: id(inner),
                kind: RelationKind::Contains,
            })
            .unwrap();
        }
        let layout = frame_and_neighbor();
        let edges = [depends("neighbor", "frame")];

        let route = routed(&view, &layout, &edges)[0]
            .clone()
            .expect("the frame is placed like any box");
        assert_eq!(
            route.path.legs(),
            1,
            "the frame the edge enters never obstructs its own arrival"
        );
        let arrival = route.path.end();
        assert!(
            (arrival.x - 300.0).abs() < 0.1,
            "the edge lands on the frame's left border, children painted or not: {arrival:?}"
        );
        assert!(
            (0.0..=200.0).contains(&arrival.y),
            "the arrival sits along that border: {arrival:?}"
        );
    }

    #[test]
    fn an_edge_between_two_boxes_of_one_summarized_frame_is_not_drawn() {
        let layout = frame_and_neighbor();
        let edges = [depends("inside", "beside")];

        assert!(
            routes(&flat(), &layout, &hidden_in_the_frame(), edges.iter())[0].is_none(),
            "the frame already stands for both ends"
        );
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
        let runs = drawn(
            &layout,
            &[
                depends("top", "target"),
                depends("middle", "target"),
                depends("bottom", "target"),
            ],
        );

        let arrivals: Vec<Pos2> = runs.iter().map(Path::end).collect();
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
        let runs = drawn(
            &layout,
            &[depends("source", "upper"), depends("source", "lower")],
        );

        let departures: Vec<Pos2> = runs.iter().map(Path::start).collect();
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
        let runs = drawn(&layout, &[depends("middle", "target")]);

        let run = &runs[0];
        assert!(
            (run.start().y - 140.0).abs() < 1.0,
            "the edge leaves the middle of its dependent's side"
        );
        assert!(
            (run.end().y - 140.0).abs() < 1.0,
            "the edge enters the middle of its dependency's side"
        );
    }

    /// Two boxes with nothing between them, one depending on the other.
    fn two_boxes() -> Layout {
        boxes(&[
            (
                "left",
                Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0)),
            ),
            (
                "right",
                Rect::from_min_max(pos2(400.0, 0.0), pos2(500.0, 100.0)),
            ),
        ])
    }

    #[test]
    fn an_edge_with_a_clear_path_keeps_its_single_curve() {
        let layout = two_boxes();
        let runs = drawn(&layout, &[depends("left", "right")]);

        assert_eq!(
            runs[0].legs(),
            1,
            "nothing stands in the way to bend around"
        );
        assert_eq!(runs[0].start(), pos2(100.0, 50.0));
        assert_eq!(runs[0].end(), pos2(400.0, 50.0));
    }

    /// Two boxes with a boundary between them that neither of them names.
    fn across_a_stranger(stranger: Rect) -> Layout {
        boxes(&[
            (
                "left",
                Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0)),
            ),
            ("stranger", stranger),
            (
                "right",
                Rect::from_min_max(pos2(400.0, 0.0), pos2(500.0, 100.0)),
            ),
        ])
    }

    #[test]
    fn an_edge_through_a_foreign_boundary_detours_around_it() {
        let stranger = Rect::from_min_max(pos2(200.0, 20.0), pos2(300.0, 80.0));
        let layout = across_a_stranger(stranger);

        let runs = drawn(&layout, &[depends("left", "right")]);
        assert!(runs[0].legs() > 1, "the run bends around what it meets");
        for point in runs[0].points(24) {
            assert!(
                !stranger.contains(point),
                "the run stays out of a box it has nothing to do with: {point:?}"
            );
        }
    }

    #[test]
    fn a_detour_rounds_the_cheaper_side() {
        let over = across_a_stranger(Rect::from_min_max(pos2(200.0, 20.0), pos2(300.0, 600.0)));
        let points = walk(&over, &[depends("left", "right")]);
        assert!(
            points.iter().any(|point| point.y < 20.0),
            "a boundary that reaches far below is passed above it"
        );
        assert!(
            points.iter().all(|point| point.y < 200.0),
            "and the run never sets out the long way round"
        );

        let under = across_a_stranger(Rect::from_min_max(pos2(200.0, -500.0), pos2(300.0, 80.0)));
        let points = walk(&under, &[depends("left", "right")]);
        assert!(
            points.iter().any(|point| point.y > 80.0),
            "a boundary that reaches far above is passed below it"
        );
        assert!(
            points.iter().all(|point| point.y > -100.0),
            "and the run never sets out the long way round"
        );
    }

    #[test]
    fn a_detour_leaves_and_enters_its_boxes_where_the_plain_curve_would() {
        let stranger = Rect::from_min_max(pos2(200.0, 20.0), pos2(300.0, 80.0));
        let layout = across_a_stranger(stranger);

        let runs = drawn(&layout, &[depends("left", "right")]);
        assert_eq!(runs[0].start(), pos2(100.0, 50.0));
        assert_eq!(runs[0].end(), pos2(400.0, 50.0));
    }

    #[test]
    fn a_run_walks_as_one_polyline_from_anchor_to_anchor() {
        let stranger = Rect::from_min_max(pos2(200.0, 20.0), pos2(300.0, 80.0));
        let layout = across_a_stranger(stranger);
        let run = &drawn(&layout, &[depends("left", "right")])[0];

        let points = run.points(8);
        assert_eq!(
            points.len(),
            run.legs() * 8 + 1,
            "the legs meet at points they share, and a shared point walks once"
        );
        assert_eq!(points[0], run.start());
        assert_eq!(points[points.len() - 1], run.end());
    }

    /// Two frames, each around one leaf. What an edge between the leaves
    /// must cross to reach the far leaf is exactly the two frames it belongs
    /// to.
    fn two_frames() -> (ArchitectureGraph, Layout) {
        let mut view = ArchitectureGraph::new();
        for (element, kind) in [
            ("package:a", ElementKind::Package),
            ("package:b", ElementKind::Package),
            ("a/one", ElementKind::Module),
            ("b/one", ElementKind::Module),
        ] {
            view.add_element(Element {
                id: id(element),
                name: ElementName::new(element).unwrap(),
                kind,
                fingerprint: None,
            })
            .unwrap();
        }
        for (frame, inner) in [("package:a", "a/one"), ("package:b", "b/one")] {
            view.add_relation(Relation {
                from: id(frame),
                to: id(inner),
                kind: RelationKind::Contains,
            })
            .unwrap();
        }
        let layout = boxes(&[
            (
                "package:a",
                Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 200.0)),
            ),
            (
                "a/one",
                Rect::from_min_max(pos2(20.0, 40.0), pos2(120.0, 80.0)),
            ),
            (
                "package:b",
                Rect::from_min_max(pos2(400.0, 0.0), pos2(600.0, 200.0)),
            ),
            (
                "b/one",
                Rect::from_min_max(pos2(420.0, 40.0), pos2(520.0, 80.0)),
            ),
        ]);
        (view, layout)
    }

    #[test]
    fn the_endpoints_own_boundaries_never_count_as_obstacles() {
        let (view, layout) = two_frames();

        let route = routed(&view, &layout, &[depends("a/one", "b/one")])[0]
            .clone()
            .expect("both leaves are placed");
        assert_eq!(
            route.path.legs(),
            1,
            "an edge crosses its own frame and its partner's on the way out and in"
        );
    }

    #[test]
    fn an_edge_inside_one_boundary_never_detours() {
        // Boxes placed where no layout would put them: the stranger's box
        // lies between two parts of one frame. Only the rule that an
        // internal edge asks nothing about the boxes around it keeps this
        // run straight.
        let mut view = ArchitectureGraph::new();
        for element in ["package:a", "a/one", "a/two", "package:z"] {
            view.add_element(Element {
                id: id(element),
                name: ElementName::new(element).unwrap(),
                kind: ElementKind::Module,
                fingerprint: None,
            })
            .unwrap();
        }
        for inner in ["a/one", "a/two"] {
            view.add_relation(Relation {
                from: id("package:a"),
                to: id(inner),
                kind: RelationKind::Contains,
            })
            .unwrap();
        }
        let layout = boxes(&[
            (
                "package:a",
                Rect::from_min_max(pos2(0.0, 0.0), pos2(500.0, 100.0)),
            ),
            (
                "a/one",
                Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0)),
            ),
            (
                "a/two",
                Rect::from_min_max(pos2(400.0, 0.0), pos2(500.0, 100.0)),
            ),
            (
                "package:z",
                Rect::from_min_max(pos2(200.0, 20.0), pos2(300.0, 80.0)),
            ),
        ]);

        let route = routed(&view, &layout, &[depends("a/one", "a/two")])[0]
            .clone()
            .expect("both parts are placed");
        assert_eq!(route.scope, Scope::Intra);
        assert_eq!(route.path.legs(), 1);
    }

    #[test]
    fn a_crowded_side_reports_its_crowd() {
        let layout = fan();

        let crowded = routed(
            &flat(),
            &layout,
            &[
                depends("top", "target"),
                depends("middle", "target"),
                depends("bottom", "target"),
            ],
        );
        for route in crowded.iter().flatten() {
            assert_eq!(
                route.crowd, 3,
                "three edges meet the target's side, however few meet the other end"
            );
        }

        let alone = routed(&flat(), &layout, &[depends("middle", "target")]);
        assert_eq!(
            alone[0].as_ref().expect("both boxes are placed").crowd,
            1,
            "an edge that meets its boxes alone keeps no company"
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
                fingerprint: None,
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
        assert_eq!(
            scopes(&Roots::of(&view), &borrowed),
            vec![Scope::Intra, Scope::Cross]
        );
    }
}
