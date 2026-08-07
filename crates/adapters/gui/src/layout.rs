//! Spatial arrangement of a boundary view.
//!
//! Dependencies read along a flow that turns with each nesting level: the
//! canvas reads left to right, the contents of a top-level boundary top to
//! bottom, the contents of a frame inside one left to right again. One
//! direction alone would grow the picture into a long horizontal sprawl
//! whose edges all travel the same way and overlap; turning the flow lets
//! the picture grow in both directions. Top-level boundaries form columns:
//! a boundary sits one column past the boundaries that depend on it.
//! Inside a container the children form bands the same way along the
//! frame's own flow: every dependency, however deep its endpoints nest,
//! lifts to the enclosing siblings, and a child's dependency sits in the
//! band past it. A band with more members than the wrap limit splits into
//! adjacent runs laid across the flow, so a wide layer stays a readable
//! block instead of one long line. Within a column, and within every
//! band, boundaries order by the average position of their dependency
//! partners, which keeps edges short and crossings few. A leaf's box grows
//! with the number of concepts the full graph places inside it, so a busy
//! boundary reads as a big one.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{ArchitectureGraph, ElementId, RelationKind};
use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};

use crate::label::{Labels, Renames};

/// The height of the smallest box a leaf ever gets: room for one label and
/// the space around it.
pub(crate) const NODE_HEIGHT: f32 = 30.0;
const PADDING: f32 = 14.0;
pub(crate) const HEADER: f32 = 26.0;
const GAP: f32 = 16.0;
const COLUMN_GAP: f32 = 110.0;
const ROW_GAP: f32 = 40.0;
/// The gap between dependency bands inside a frame: tighter than the root
/// [`COLUMN_GAP`], but wide enough for the arrows crossing it to read.
const BAND_GAP: f32 = 48.0;
/// The gap between band members standing shoulder to shoulder when the
/// flow runs downward. Boxes are wider than tall, so across a downward
/// flow they read apart with less air than stacked rows need, while their
/// top and bottom edges still carry the arrows undisturbed.
const SIDE_GAP: f32 = 24.0;
/// Box growth in points per square root of the contained concept count:
/// the box area, not its edge length, tracks the content.
const WEIGHT_GROWTH: f32 = 7.0;
/// Barycenter passes over the columns; a handful settles the order.
const ORDERING_SWEEPS: usize = 4;
/// The fewest members a run inside a frame holds before its band may
/// wrap: a short list reads as a list, and wrapping it gains nothing.
const STACK_LIMIT: usize = 3;

/// The direction dependencies read along at one nesting level. The flow
/// turns as frames nest, so a deep picture spreads over both screen
/// directions instead of sprawling along one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Rightward,
    Downward,
}

impl Flow {
    /// The flow inside a frame this deep. The canvas reads rightward, so
    /// the contents of a top-level frame read downward, and every further
    /// level turns again.
    fn at(depth: usize) -> Self {
        if depth.is_multiple_of(2) {
            Self::Downward
        } else {
            Self::Rightward
        }
    }

    /// The extent of a box along the flow.
    fn along(self, size: Vec2) -> f32 {
        match self {
            Self::Rightward => size.x,
            Self::Downward => size.y,
        }
    }

    /// The extent of a box across the flow.
    fn across(self, size: Vec2) -> f32 {
        match self {
            Self::Rightward => size.y,
            Self::Downward => size.x,
        }
    }

    /// A vector from its along-flow and across-flow parts.
    fn compose(self, along: f32, across: f32) -> Vec2 {
        match self {
            Self::Rightward => vec2(along, across),
            Self::Downward => vec2(across, along),
        }
    }

    /// The gap between the members of one run. A rightward flow stacks its
    /// runs downward and rows of boxes need [`ROW_GAP`] of air; a downward
    /// flow lays boxes on their wide side, where [`SIDE_GAP`] reads apart.
    fn shoulder_gap(self) -> f32 {
        match self {
            Self::Rightward => ROW_GAP,
            Self::Downward => SIDE_GAP,
        }
    }
}

pub struct Layout {
    pub rects: BTreeMap<ElementId, Rect>,
    /// Boundaries with children, outermost first: paint these as boxes.
    pub containers: Vec<Frame>,
    /// Boundaries without children: paint these as nodes.
    pub leaves: Vec<ElementId>,
}

/// A boundary that holds others, and how deep it nests. A boundary on the
/// canvas itself has depth zero; every frame inside one counts one more.
/// The canvas shades by depth, so nested frames never share a shade.
pub struct Frame {
    pub id: ElementId,
    pub depth: usize,
}

/// The number of concepts each element transitively contains in the full
/// graph. Sizing uses this, not the boundary view: a rolled-up boundary
/// still answers for everything hidden inside it.
pub fn concept_weights(graph: &ArchitectureGraph) -> BTreeMap<ElementId, usize> {
    fn count(
        id: &ElementId,
        children: &BTreeMap<&ElementId, Vec<&ElementId>>,
        memo: &mut BTreeMap<ElementId, usize>,
        visiting: &mut BTreeSet<ElementId>,
    ) -> usize {
        if let Some(existing) = memo.get(id) {
            return *existing;
        }
        if !visiting.insert(id.clone()) {
            return 0;
        }
        let total = children
            .get(id)
            .into_iter()
            .flatten()
            .map(|child| 1 + count(child, children, memo, visiting))
            .sum();
        visiting.remove(id);
        memo.insert(id.clone(), total);
        total
    }

    let mut children: BTreeMap<&ElementId, Vec<&ElementId>> = BTreeMap::new();
    for relation in graph.relations() {
        if relation.kind == RelationKind::Contains {
            children
                .entry(&relation.from)
                .or_default()
                .push(&relation.to);
        }
    }
    let mut memo = BTreeMap::new();
    let mut visiting = BTreeSet::new();
    graph
        .elements()
        .map(|element| {
            let weight = count(&element.id, &children, &mut memo, &mut visiting);
            (element.id.clone(), weight)
        })
        .collect()
}

pub fn compute(
    view: &ArchitectureGraph,
    weights: &BTreeMap<ElementId, usize>,
    renames: &Renames,
) -> Layout {
    let mut parents: BTreeMap<ElementId, ElementId> = BTreeMap::new();
    let mut children: BTreeMap<ElementId, Vec<ElementId>> = BTreeMap::new();
    for relation in view.relations() {
        if relation.kind == RelationKind::Contains {
            parents.insert(relation.to.clone(), relation.from.clone());
            children
                .entry(relation.from.clone())
                .or_default()
                .push(relation.to.clone());
        }
    }

    let depends: Vec<(ElementId, ElementId)> = view
        .relations()
        .filter(|r| r.kind == RelationKind::DependsOn)
        .map(|r| (r.from.clone(), r.to.clone()))
        .collect();

    let root_of = |id: &ElementId| -> ElementId {
        let mut current = id.clone();
        while let Some(parent) = parents.get(&current) {
            current = parent.clone();
        }
        current
    };

    let roots: Vec<ElementId> = view
        .elements()
        .map(|e| e.id.clone())
        .filter(|id| !parents.contains_key(id))
        .collect();

    let root_edges: Vec<(ElementId, ElementId)> = depends
        .iter()
        .map(|(from, to)| (root_of(from), root_of(to)))
        .filter(|(from, to)| from != to)
        .collect();
    let root_layers = layers(&roots, &root_edges);

    let mut grouped: BTreeMap<usize, Vec<ElementId>> = BTreeMap::new();
    for root in &roots {
        grouped
            .entry(root_layers[root])
            .or_default()
            .push(root.clone());
    }
    let mut columns: Vec<Vec<ElementId>> = grouped.into_values().collect();
    reduce_crossings(&mut columns, &root_edges);

    let mut orders: BTreeMap<ElementId, Vec<Vec<ElementId>>> = children
        .iter()
        .map(|(parent, kids)| (parent.clone(), banded(kids, &depends, &parents)))
        .collect();

    // A box must fit the label the canvas writes on it, glyph, shortened
    // name and the plan's rename included, so both read the label from the
    // same place.
    let labels = Labels::renaming(view, renames);
    let label_of = |id: &ElementId| -> String { labels.label(id).text() };

    // Refinement must read each frame the way its own flow does, and the
    // flow follows from how deep the frame nests.
    let flows: BTreeMap<ElementId, Flow> = children
        .keys()
        .map(|parent| {
            let mut depth = 0;
            let mut current = parent;
            while let Some(above) = parents.get(current) {
                depth += 1;
                current = above;
            }
            (parent.clone(), Flow::at(depth))
        })
        .collect();

    // Arrange twice: the first pass reveals where everything lands, the
    // second reorders siblings toward their dependency partners.
    let first = arrange(&columns, &orders, weights, &label_of);
    refine_sibling_orders(&mut orders, &depends, &flows, &first);
    arrange(&columns, &orders, weights, &label_of)
}

fn arrange(
    columns: &[Vec<ElementId>],
    orders: &BTreeMap<ElementId, Vec<Vec<ElementId>>>,
    weights: &BTreeMap<ElementId, usize>,
    label_of: &impl Fn(&ElementId) -> String,
) -> Layout {
    let mut measures = Measures::default();
    for root in columns.iter().flatten() {
        measure(root, 0, orders, weights, label_of, &mut measures);
    }

    let mut layout = Layout {
        rects: BTreeMap::new(),
        containers: Vec::new(),
        leaves: Vec::new(),
    };
    let sizes = &measures.sizes;
    let mut x = 0.0;
    for column in columns {
        let width = column.iter().map(|id| sizes[id].x).fold(0.0_f32, f32::max);
        let height: f32 = column.iter().map(|id| sizes[id].y + ROW_GAP).sum::<f32>() - ROW_GAP;
        // Columns center on a shared axis so edges cross the gap squarely.
        let mut y = -height / 2.0;
        for root in column {
            place(root, pos2(x, y), 0, orders, &measures, &mut layout);
            y += sizes[root].y + ROW_GAP;
        }
        x += width + COLUMN_GAP;
    }
    layout
}

/// What one arrangement pass measured: the extent of every box, and how
/// each container bands its children.
#[derive(Default)]
struct Measures {
    sizes: BTreeMap<ElementId, Vec2>,
    bands: BTreeMap<ElementId, Bands>,
}

fn measure(
    id: &ElementId,
    depth: usize,
    orders: &BTreeMap<ElementId, Vec<Vec<ElementId>>>,
    weights: &BTreeMap<ElementId, usize>,
    label_of: &impl Fn(&ElementId) -> String,
    measures: &mut Measures,
) -> Vec2 {
    let label = label_width(&label_of(id));
    let size = match orders.get(id) {
        None => {
            let growth = growth_of(weights.get(id).copied().unwrap_or(0));
            vec2(label + growth, NODE_HEIGHT + growth * 0.6)
        }
        Some(inner) => {
            let sized: Vec<Vec<Vec2>> = inner
                .iter()
                .map(|band| {
                    band.iter()
                        .map(|child| measure(child, depth + 1, orders, weights, label_of, measures))
                        .collect()
                })
                .collect();
            let bands = Bands::new(&sized, Flow::at(depth));
            let size = framed(bands.extent, label);
            measures.bands.insert(id.clone(), bands);
            size
        }
    };
    measures.sizes.insert(id.clone(), size);
    size
}

fn place(
    id: &ElementId,
    origin: Pos2,
    depth: usize,
    orders: &BTreeMap<ElementId, Vec<Vec<ElementId>>>,
    measures: &Measures,
    layout: &mut Layout,
) {
    layout
        .rects
        .insert(id.clone(), Rect::from_min_size(origin, measures.sizes[id]));
    match orders.get(id) {
        None => layout.leaves.push(id.clone()),
        Some(inner) => {
            layout.containers.push(Frame {
                id: id.clone(),
                depth,
            });
            let bands = &measures.bands[id];
            let content = pos2(origin.x + PADDING, origin.y + HEADER + PADDING);
            for (index, child) in inner.iter().flatten().enumerate() {
                let offset = bands.offsets[index];
                place(child, content + offset, depth + 1, orders, measures, layout);
            }
        }
    }
}

/// A container's children arranged into dependency bands: one band per
/// layer, bands advance along the frame's flow, and a band with more
/// members than the wrap limit splits into adjacent runs laid across the
/// flow. Runs centre on the content's middle, so edges cross the band
/// gaps squarely.
struct Bands {
    /// Where each child sits relative to the content's top left, indexed
    /// in flattened band order.
    offsets: Vec<Vec2>,
    extent: Vec2,
}

impl Bands {
    fn new(bands: &[Vec<Vec2>], flow: Flow) -> Self {
        let total: usize = bands.iter().map(Vec::len).sum();
        let limit = wrap_limit(total, flow);
        // A run is a stretch of at most `limit` members of one band; the
        // run remembers its band, so the gap to the previous run tells
        // wrapping apart from layering.
        let mut runs: Vec<(usize, Vec<(usize, Vec2)>)> = Vec::new();
        let mut flat = 0;
        for (band, members) in bands.iter().enumerate() {
            for chunk in members.chunks(limit) {
                let mut entries = Vec::with_capacity(chunk.len());
                for size in chunk {
                    entries.push((flat, *size));
                    flat += 1;
                }
                runs.push((band, entries));
            }
        }

        let breadth_of = |entries: &[(usize, Vec2)]| -> f32 {
            entries
                .iter()
                .map(|(_, size)| flow.across(*size) + flow.shoulder_gap())
                .sum::<f32>()
                - flow.shoulder_gap()
        };
        let breadth = runs
            .iter()
            .map(|(_, entries)| breadth_of(entries))
            .fold(0.0_f32, f32::max);

        let mut offsets = vec![Vec2::ZERO; total];
        let mut along = 0.0_f32;
        let mut previous_band = None;
        for (band, entries) in &runs {
            if let Some(previous) = previous_band {
                along += if previous == *band { GAP } else { BAND_GAP };
            }
            previous_band = Some(*band);
            let mut across = (breadth - breadth_of(entries)) / 2.0;
            for (index, size) in entries {
                offsets[*index] = flow.compose(along, across);
                across += flow.across(*size) + flow.shoulder_gap();
            }
            along += entries
                .iter()
                .map(|(_, size)| flow.along(*size))
                .fold(0.0_f32, f32::max);
        }
        Self {
            offsets,
            extent: flow.compose(along, breadth),
        }
    }
}

/// The most members a run inside a frame holds before its band wraps into
/// an adjacent run: enough that a frame without dependencies still packs
/// into a block, but never below [`STACK_LIMIT`], so a short list stays
/// one run. A leaf box is about four times as wide as tall, so a downward
/// flow, whose runs lay boxes on their wide side, balances at half the
/// members a rightward one stacks.
fn wrap_limit(count: usize, flow: Flow) -> usize {
    let balanced = match flow {
        Flow::Rightward => (1..=count).find(|root| root * root >= count),
        Flow::Downward => (1..=count).find(|root| root * root * 4 >= count),
    };
    balanced.unwrap_or(1).max(STACK_LIMIT)
}

/// The container box around banded content: room for the header, the
/// padding, and at least the container's own label.
fn framed(content: Vec2, label: f32) -> Vec2 {
    vec2(
        (content.x + 2.0 * PADDING).max(label),
        content.y + HEADER + 2.0 * PADDING,
    )
}

/// Orders every column by the average position of each member's dependency
/// partners; repeated sweeps let mutually dependent columns settle.
/// A member without partners keeps its place.
fn reduce_crossings(columns: &mut [Vec<ElementId>], edges: &[(ElementId, ElementId)]) {
    let mut partners: BTreeMap<ElementId, Vec<ElementId>> = BTreeMap::new();
    for (from, to) in edges {
        partners.entry(from.clone()).or_default().push(to.clone());
        partners.entry(to.clone()).or_default().push(from.clone());
    }

    let mut position: BTreeMap<ElementId, f32> = BTreeMap::new();
    for column in columns.iter() {
        for (index, id) in column.iter().enumerate() {
            position.insert(id.clone(), ordinal(index));
        }
    }

    for _ in 0..ORDERING_SWEEPS {
        for column in columns.iter_mut() {
            let keys: BTreeMap<ElementId, f32> = column
                .iter()
                .map(|id| {
                    let others = partners
                        .get(id)
                        .into_iter()
                        .flatten()
                        .filter_map(|partner| position.get(partner).copied());
                    (id.clone(), average(others).unwrap_or(position[id]))
                })
                .collect();
            column.sort_by(|a, b| {
                keys[a]
                    .partial_cmp(&keys[b])
                    .unwrap_or(Ordering::Equal)
                    .then_with(|| a.cmp(b))
            });
            for (index, id) in column.iter().enumerate() {
                position.insert(id.clone(), ordinal(index));
            }
        }
    }
}

/// Reorders every band by where the members' dependency partners landed in
/// a previous arrangement: a child moves toward the place its partners
/// occupy, so its edges travel the shortest way. The move stays inside the
/// child's band: the dependency layering decided the band, and refinement
/// must never move a child across layers. Members without partners keep
/// their place.
fn refine_sibling_orders(
    orders: &mut BTreeMap<ElementId, Vec<Vec<ElementId>>>,
    depends: &[(ElementId, ElementId)],
    flows: &BTreeMap<ElementId, Flow>,
    previous: &Layout,
) {
    let mut partners: BTreeMap<&ElementId, Vec<&ElementId>> = BTreeMap::new();
    for (from, to) in depends {
        partners.entry(from).or_default().push(to);
        partners.entry(to).or_default().push(from);
    }

    let mut keys: BTreeMap<ElementId, Pos2> = BTreeMap::new();
    for bands in orders.values() {
        for kid in bands.iter().flatten() {
            // A container follows its whole subtree's partners: the edges
            // that matter attach to its descendants.
            let members = subtree(kid, orders);
            let centers = members.iter().flat_map(|member| {
                partners
                    .get(member)
                    .into_iter()
                    .flatten()
                    .filter(|partner| !members.contains(**partner))
                    .filter_map(|partner| previous.rects.get(*partner))
                    .map(Rect::center)
            });
            let key = centroid(centers).unwrap_or_else(|| previous.rects[kid].center());
            keys.insert(kid.clone(), key);
        }
    }
    // The sort is stable, so children whose partners pull them to the same
    // place keep the crossing-reduced order they arrived in.
    for (parent, bands) in orders.iter_mut() {
        for band in bands.iter_mut() {
            band.sort_by(|a, b| in_reading_order(flows[parent], keys[a], keys[b]));
        }
    }
}

/// Which of two points comes first when a frame flowing this way is read:
/// band members stand across the flow, so the one earlier across it, and
/// among points of one rank the one earlier along it. Positions count in
/// whole steps of [`GAP`], the closest two boxes ever come on either
/// axis, so a centroid that misses a line by rounding still ranks with
/// it.
fn in_reading_order(flow: Flow, a: Pos2, b: Pos2) -> Ordering {
    let step = |value: f32| (value / GAP).round();
    let across = |point: Pos2| step(flow.across(point.to_vec2()));
    let along = |point: Pos2| step(flow.along(point.to_vec2()));
    across(a)
        .partial_cmp(&across(b))
        .unwrap_or(Ordering::Equal)
        .then_with(|| along(a).partial_cmp(&along(b)).unwrap_or(Ordering::Equal))
}

fn subtree(
    id: &ElementId,
    orders: &BTreeMap<ElementId, Vec<Vec<ElementId>>>,
) -> BTreeSet<ElementId> {
    let mut members = BTreeSet::from([id.clone()]);
    let mut queue = vec![id.clone()];
    while let Some(current) = queue.pop() {
        for child in orders.get(&current).into_iter().flatten().flatten() {
            if members.insert(child.clone()) {
                queue.push(child.clone());
            }
        }
    }
    members
}

/// Siblings split into dependency bands: every dependency of the view
/// lifts each endpoint to its ancestor-or-self among the siblings, exactly
/// as the top level lifts to the roots, so an edge between deeply nested
/// descendants still layers the frames that enclose them. Longest-path
/// layers become the bands, and each band orders to reduce crossings.
fn banded(
    siblings: &[ElementId],
    depends: &[(ElementId, ElementId)],
    parents: &BTreeMap<ElementId, ElementId>,
) -> Vec<Vec<ElementId>> {
    let set: BTreeSet<&ElementId> = siblings.iter().collect();
    let lift = |id: &ElementId| -> Option<ElementId> {
        let mut current = id.clone();
        loop {
            if set.contains(&current) {
                return Some(current);
            }
            current = parents.get(&current)?.clone();
        }
    };
    let lifted: Vec<(ElementId, ElementId)> = depends
        .iter()
        .filter_map(|(from, to)| Some((lift(from)?, lift(to)?)))
        .filter(|(from, to)| from != to)
        .collect();

    let layer = layers(siblings, &lifted);
    let mut grouped: BTreeMap<usize, Vec<ElementId>> = BTreeMap::new();
    for sibling in siblings {
        grouped
            .entry(layer[sibling])
            .or_default()
            .push(sibling.clone());
    }
    let mut bands: Vec<Vec<ElementId>> = grouped.into_values().collect();
    for band in &mut bands {
        band.sort();
    }
    reduce_crossings(&mut bands, &lifted);
    bands
}

/// Longest-path layering: an element sits one layer past everything that
/// depends on it. Cycles cut at the revisit.
fn layers(nodes: &[ElementId], edges: &[(ElementId, ElementId)]) -> BTreeMap<ElementId, usize> {
    fn visit(
        node: &ElementId,
        incoming: &BTreeMap<&ElementId, Vec<&ElementId>>,
        memo: &mut BTreeMap<ElementId, usize>,
        visiting: &mut BTreeSet<ElementId>,
    ) -> usize {
        if let Some(layer) = memo.get(node) {
            return *layer;
        }
        if !visiting.insert(node.clone()) {
            return 0;
        }
        let layer = incoming
            .get(node)
            .into_iter()
            .flatten()
            .map(|from| visit(from, incoming, memo, visiting) + 1)
            .max()
            .unwrap_or(0);
        visiting.remove(node);
        memo.insert(node.clone(), layer);
        layer
    }

    let mut incoming: BTreeMap<&ElementId, Vec<&ElementId>> = BTreeMap::new();
    for (from, to) in edges {
        incoming.entry(to).or_default().push(from);
    }
    let mut memo = BTreeMap::new();
    let mut visiting = BTreeSet::new();
    for node in nodes {
        visit(node, &incoming, &mut memo, &mut visiting);
    }
    memo
}

fn centroid(points: impl Iterator<Item = Pos2>) -> Option<Pos2> {
    let mut sum = Vec2::ZERO;
    let mut count = 0.0_f32;
    for point in points {
        sum += point.to_vec2();
        count += 1.0;
    }
    (count > 0.0).then(|| (sum / count).to_pos2())
}

fn average(values: impl Iterator<Item = f32>) -> Option<f32> {
    let mut sum = 0.0_f32;
    let mut count = 0.0_f32;
    for value in values {
        sum += value;
        count += 1.0;
    }
    (count > 0.0).then(|| sum / count)
}

fn ordinal(index: usize) -> f32 {
    f32::from(u16::try_from(index).unwrap_or(u16::MAX))
}

fn growth_of(weight: usize) -> f32 {
    f32::from(u16::try_from(weight).unwrap_or(u16::MAX)).sqrt() * WEIGHT_GROWTH
}

fn label_width(name: &str) -> f32 {
    let chars: f32 = f32::from(u16::try_from(name.chars().count()).unwrap_or(u16::MAX));
    (chars * 7.4 + 26.0).max(60.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyph;
    use cutaway_architecture::{Element, ElementKind, ElementName, Relation};

    fn add_package(graph: &mut ArchitectureGraph, name: &str) -> ElementId {
        let id = ElementId::new(format!("package:{name}")).unwrap();
        graph
            .add_element(Element {
                id: id.clone(),
                name: ElementName::new(name).unwrap(),
                kind: ElementKind::Package,
                fingerprint: None,
            })
            .unwrap();
        id
    }

    fn add_module(graph: &mut ArchitectureGraph, parent: &ElementId, path: &str) -> ElementId {
        add_named_module(graph, parent, path, path)
    }

    fn add_named_module(
        graph: &mut ArchitectureGraph,
        parent: &ElementId,
        path: &str,
        name: &str,
    ) -> ElementId {
        let id = ElementId::new(path).unwrap();
        graph
            .add_element(Element {
                id: id.clone(),
                name: ElementName::new(name).unwrap(),
                kind: ElementKind::Module,
                fingerprint: None,
            })
            .unwrap();
        graph
            .add_relation(Relation {
                from: parent.clone(),
                to: id.clone(),
                kind: RelationKind::Contains,
            })
            .unwrap();
        id
    }

    fn depend(graph: &mut ArchitectureGraph, from: &ElementId, to: &ElementId) {
        graph
            .add_relation(Relation {
                from: from.clone(),
                to: to.clone(),
                kind: RelationKind::DependsOn,
            })
            .unwrap();
    }

    fn no_weights() -> BTreeMap<ElementId, usize> {
        BTreeMap::new()
    }

    #[test]
    fn a_dependency_reads_from_left_to_right() {
        let mut graph = ArchitectureGraph::new();
        let dependent = add_package(&mut graph, "app");
        let dependency = add_package(&mut graph, "domain");
        depend(&mut graph, &dependent, &dependency);

        let layout = compute(&graph, &no_weights(), &Renames::default());
        assert!(layout.rects[&dependent].max.x < layout.rects[&dependency].min.x);
    }

    #[test]
    fn boxes_of_unrelated_boundaries_do_not_overlap() {
        let mut graph = ArchitectureGraph::new();
        let left = add_package(&mut graph, "left");
        let right = add_package(&mut graph, "right");
        let left_a = add_module(&mut graph, &left, "left/a.rs");
        let left_b = add_module(&mut graph, &left, "left/b.rs");
        let right_a = add_module(&mut graph, &right, "right/a.rs");
        depend(&mut graph, &left_a, &right_a);
        depend(&mut graph, &left_b, &right_a);

        let layout = compute(&graph, &no_weights(), &Renames::default());
        let separate = [
            (&left, &right),
            (&left_a, &left_b),
            (&left_a, &right_a),
            (&left_b, &right_a),
        ];
        for (a, b) in separate {
            assert!(
                !layout.rects[a]
                    .shrink(1.0)
                    .intersects(layout.rects[b].shrink(1.0)),
                "{a} and {b} overlap"
            );
        }
    }

    #[test]
    fn a_boundary_with_more_concepts_gets_a_bigger_box() {
        let mut graph = ArchitectureGraph::new();
        let heavy = add_package(&mut graph, "alpha");
        let light = add_package(&mut graph, "omega");

        let weights = BTreeMap::from([(heavy.clone(), 49), (light.clone(), 0)]);
        let layout = compute(&graph, &weights, &Renames::default());
        assert!(layout.rects[&heavy].area() > layout.rects[&light].area());
    }

    #[test]
    fn a_crowded_container_packs_into_a_readable_block() {
        let mut graph = ArchitectureGraph::new();
        let package = add_package(&mut graph, "crowded");
        for index in 0..16 {
            add_module(&mut graph, &package, &format!("crowded/m{index:02}.rs"));
        }

        let layout = compute(&graph, &no_weights(), &Renames::default());
        let box_of = layout.rects[&package];
        let aspect = box_of.width() / box_of.height();
        assert!(
            (0.34..=3.0).contains(&aspect),
            "a container of 16 children reads at {aspect}:1"
        );
    }

    #[test]
    fn packed_children_never_overlap() {
        let mut graph = ArchitectureGraph::new();
        let package = add_package(&mut graph, "mixed");
        let mut children = Vec::new();
        for index in 0..12 {
            let child = add_module(&mut graph, &package, &format!("mixed/m{index:02}.rs"));
            // Every third child holds two of its own, so rows meet boxes of
            // very different heights.
            if index % 3 == 0 {
                for inner in 0..2 {
                    add_module(
                        &mut graph,
                        &child,
                        &format!("mixed/m{index:02}/i{inner}.rs"),
                    );
                }
            }
            children.push(child);
        }

        let layout = compute(&graph, &no_weights(), &Renames::default());
        for (index, a) in children.iter().enumerate() {
            assert!(
                layout.rects[&package].contains_rect(layout.rects[a]),
                "{a} escapes its container"
            );
            for b in &children[index + 1..] {
                assert!(
                    !layout.rects[a]
                        .shrink(1.0)
                        .intersects(layout.rects[b].shrink(1.0)),
                    "{a} and {b} overlap"
                );
            }
        }
    }

    #[test]
    fn a_siblings_dependency_sits_in_a_band_beneath_it() {
        let mut graph = ArchitectureGraph::new();
        let package = add_package(&mut graph, "pkg");
        // The dependent sorts after its dependency by id, so only the
        // layering can put it first along the flow.
        let dependent = add_module(&mut graph, &package, "pkg/z-app.rs");
        let dependency = add_module(&mut graph, &package, "pkg/a-domain.rs");
        depend(&mut graph, &dependent, &dependency);

        let layout = compute(&graph, &no_weights(), &Renames::default());
        assert!(layout.rects[&dependent].max.y < layout.rects[&dependency].min.y);
    }

    #[test]
    fn a_dependency_between_nested_descendants_orders_their_enclosing_frames() {
        let mut graph = ArchitectureGraph::new();
        let package = add_package(&mut graph, "pkg");
        // The frames hold no direct edge of their own; only the edge
        // between their grandchildren says which one reads first.
        let dependent_frame = add_module(&mut graph, &package, "pkg/z-user");
        let dependency_frame = add_module(&mut graph, &package, "pkg/a-used");
        let dependent = add_module(&mut graph, &dependent_frame, "pkg/z-user/inner.rs");
        let dependency = add_module(&mut graph, &dependency_frame, "pkg/a-used/inner.rs");
        depend(&mut graph, &dependent, &dependency);

        let layout = compute(&graph, &no_weights(), &Renames::default());
        assert!(layout.rects[&dependent_frame].max.y < layout.rects[&dependency_frame].min.y);
    }

    #[test]
    fn the_flow_turns_at_each_nesting_level() {
        let mut graph = ArchitectureGraph::new();
        let package = add_package(&mut graph, "pkg");
        // Every dependent sorts after its dependency by id, so only the
        // layering can put it first along each level's flow.
        let outer_dependent = add_module(&mut graph, &package, "pkg/z-user");
        let outer_dependency = add_module(&mut graph, &package, "pkg/a-used");
        let inner_dependent = add_module(&mut graph, &outer_dependent, "pkg/z-user/z-caller");
        let inner_dependency = add_module(&mut graph, &outer_dependent, "pkg/z-user/a-callee");
        let deep_dependent =
            add_module(&mut graph, &inner_dependent, "pkg/z-user/z-caller/z-top.rs");
        let deep_dependency = add_module(
            &mut graph,
            &inner_dependent,
            "pkg/z-user/z-caller/a-bottom.rs",
        );
        depend(&mut graph, &outer_dependent, &outer_dependency);
        depend(&mut graph, &inner_dependent, &inner_dependency);
        depend(&mut graph, &deep_dependent, &deep_dependency);

        let layout = compute(&graph, &no_weights(), &Renames::default());
        assert!(
            layout.rects[&outer_dependent].max.y < layout.rects[&outer_dependency].min.y,
            "a top-level frame's contents flow downward"
        );
        assert!(
            layout.rects[&inner_dependent].max.x < layout.rects[&inner_dependency].min.x,
            "one level deeper the flow turns rightward"
        );
        assert!(
            layout.rects[&deep_dependent].max.y < layout.rects[&deep_dependency].min.y,
            "another level deeper it turns downward again"
        );
    }

    #[test]
    fn a_wide_layer_wraps_into_adjacent_runs_instead_of_one_long_line() {
        let mut graph = ArchitectureGraph::new();
        let package = add_package(&mut graph, "hub");
        let sink = add_module(&mut graph, &package, "hub/sink.rs");
        let callers: Vec<ElementId> = (0..10)
            .map(|index| add_module(&mut graph, &package, &format!("hub/c{index}.rs")))
            .collect();
        for caller in &callers {
            depend(&mut graph, caller, &sink);
        }

        let layout = compute(&graph, &no_weights(), &Renames::default());
        let mut runs: BTreeMap<u32, usize> = BTreeMap::new();
        for caller in &callers {
            assert!(
                layout.rects[caller].max.y < layout.rects[&sink].min.y,
                "{caller} does not read before its dependency"
            );
            *runs
                .entry(layout.rects[caller].min.y.to_bits())
                .or_default() += 1;
        }
        assert!(runs.len() > 1, "ten callers line up into one run");
        assert!(
            runs.values().all(|members| *members <= 3),
            "a wrapped run outgrows the wrap limit"
        );
    }

    #[test]
    fn independent_children_spread_into_several_runs() {
        let mut graph = ArchitectureGraph::new();
        let package = add_package(&mut graph, "flat");
        let children: Vec<ElementId> = (0..12)
            .map(|index| add_module(&mut graph, &package, &format!("flat/m{index:02}.rs")))
            .collect();

        let layout = compute(&graph, &no_weights(), &Renames::default());
        let runs: BTreeSet<u32> = children
            .iter()
            .map(|child| layout.rects[child].min.y.to_bits())
            .collect();
        assert!(
            runs.len() > 1,
            "twelve independent children form one long line"
        );
        let box_of = layout.rects[&package];
        let aspect = box_of.width() / box_of.height();
        assert!(
            (0.34..=3.0).contains(&aspect),
            "twelve independent children read at {aspect}:1"
        );
    }

    #[test]
    fn three_or_fewer_children_line_up_in_one_run() {
        let mut graph = ArchitectureGraph::new();
        let package = add_package(&mut graph, "small");
        let children: Vec<ElementId> = (0..3)
            .map(|index| add_module(&mut graph, &package, &format!("small/m{index}.rs")))
            .collect();

        let layout = compute(&graph, &no_weights(), &Renames::default());
        let runs: BTreeSet<u32> = children
            .iter()
            .map(|child| layout.rects[child].min.y.to_bits())
            .collect();
        assert_eq!(runs.len(), 1, "three children wrap into several runs");
    }

    #[test]
    fn the_same_view_yields_the_same_layout_every_time() {
        let mut graph = ArchitectureGraph::new();
        let package = add_package(&mut graph, "det");
        let sink = add_module(&mut graph, &package, "det/sink.rs");
        for index in 0..7 {
            let module = add_module(&mut graph, &package, &format!("det/m{index}.rs"));
            depend(&mut graph, &module, &sink);
        }

        let first = compute(&graph, &no_weights(), &Renames::default());
        let second = compute(&graph, &no_weights(), &Renames::default());
        assert_eq!(first.rects, second.rects);
    }

    #[test]
    fn packing_keeps_the_dependency_reading_order() {
        let mut graph = ArchitectureGraph::new();
        let package = add_package(&mut graph, "chain");
        let chain: Vec<ElementId> = (0..9)
            .map(|index| add_module(&mut graph, &package, &format!("chain/m{index}.rs")))
            .collect();
        for pair in chain.windows(2) {
            depend(&mut graph, &pair[0], &pair[1]);
        }

        let layout = compute(&graph, &no_weights(), &Renames::default());
        for pair in chain.windows(2) {
            let (dependent, dependency) = (layout.rects[&pair[0]], layout.rects[&pair[1]]);
            let reads_first = dependent.center().y < dependency.center().y
                || (dependent.center().y - dependency.center().y).abs() < 1.0
                    && dependent.center().x < dependency.center().x;
            assert!(
                reads_first,
                "{} does not read before {}",
                pair[0].as_str(),
                pair[1].as_str()
            );
        }
    }

    #[test]
    fn a_box_measures_the_name_it_shows_and_not_the_path_it_carries() {
        let mut graph = ArchitectureGraph::new();
        let nested_frame = add_package(&mut graph, "alpha");
        let nested = add_named_module(&mut graph, &nested_frame, "alpha/x.rs", "alpha::x");
        let plain_frame = add_package(&mut graph, "beta");
        let plain = add_named_module(&mut graph, &plain_frame, "beta/y.rs", "y");

        let layout = compute(&graph, &no_weights(), &Renames::default());
        let difference = (layout.rects[&nested].width() - layout.rects[&plain].width()).abs();
        assert!(
            difference < 1.0,
            "a name inside the frame it repeats measures as the segment it shows"
        );
    }

    #[test]
    fn a_box_leaves_room_for_the_kind_glyph_beside_the_name() {
        let mut graph = ArchitectureGraph::new();
        let package = add_package(&mut graph, "app");

        let layout = compute(&graph, &no_weights(), &Renames::default());
        assert!(layout.rects[&package].width() >= label_width(&format!("{} app", glyph::PACKAGE)));
    }

    #[test]
    fn columns_reorder_so_that_edges_do_not_cross() {
        let mut graph = ArchitectureGraph::new();
        let upper = add_package(&mut graph, "a-upper");
        let lower = add_package(&mut graph, "b-lower");
        let crossed = add_package(&mut graph, "x-crossed");
        let straight = add_package(&mut graph, "y-straight");
        // With alphabetic order in both columns these two edges cross.
        depend(&mut graph, &upper, &straight);
        depend(&mut graph, &lower, &crossed);

        let layout = compute(&graph, &no_weights(), &Renames::default());
        let above =
            |a: &ElementId, b: &ElementId| layout.rects[a].center().y < layout.rects[b].center().y;
        assert_eq!(above(&upper, &lower), above(&straight, &crossed));
    }
}
