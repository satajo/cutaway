//! Spatial arrangement of a boundary view.
//!
//! Top-level boundaries form columns: a boundary sits one column right of
//! the boundaries that depend on it, so dependencies read left to right.
//! Inside a container the children wrap into a grid that reads like text,
//! left to right and then down, so a container of twenty children stays a
//! readable box instead of a tower. Within a column, and within every
//! container, boundaries order by the average position of their dependency
//! partners, which keeps edges short and crossings few. A leaf's box grows
//! with the number of concepts the full graph places inside it, so a busy
//! boundary reads as a big one.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{ArchitectureGraph, ElementId, RelationKind};
use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};

const NODE_HEIGHT: f32 = 30.0;
const PADDING: f32 = 14.0;
pub(crate) const HEADER: f32 = 26.0;
const GAP: f32 = 16.0;
const COLUMN_GAP: f32 = 110.0;
const ROW_GAP: f32 = 40.0;
/// Box growth in points per square root of the contained concept count:
/// the box area, not its edge length, tracks the content.
const WEIGHT_GROWTH: f32 = 7.0;
/// Barycenter passes over the columns; a handful settles the order.
const ORDERING_SWEEPS: usize = 4;
/// The width-to-height ratio a packed container aims for: a box slightly
/// wider than tall fits both the canvas and the eye.
const TARGET_ASPECT: f32 = 1.6;
/// How much the space a grid wastes counts against the shape it reaches.
/// Below one, shape leads and area only breaks near ties.
const AREA_COST: f32 = 0.35;
/// Up to this many children stay in one column: a short list reads as a
/// list, and wrapping it gains nothing.
const STACK_LIMIT: usize = 3;

pub struct Layout {
    pub rects: BTreeMap<ElementId, Rect>,
    /// Boundaries with children, outermost first: paint these as boxes.
    pub containers: Vec<ElementId>,
    /// Boundaries without children: paint these as nodes.
    pub leaves: Vec<ElementId>,
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

pub fn compute(view: &ArchitectureGraph, weights: &BTreeMap<ElementId, usize>) -> Layout {
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

    let mut orders: BTreeMap<ElementId, Vec<ElementId>> = children
        .iter()
        .map(|(parent, kids)| (parent.clone(), ordered_by_layer(kids, &depends)))
        .collect();

    let name_of = |id: &ElementId| -> String {
        view.element(id)
            .map_or_else(String::new, |e| e.name.to_string())
    };

    // Arrange twice: the first pass reveals where everything lands, the
    // second reorders siblings toward their dependency partners.
    let first = arrange(&columns, &orders, weights, &name_of);
    refine_sibling_orders(&mut orders, &depends, &first);
    arrange(&columns, &orders, weights, &name_of)
}

fn arrange(
    columns: &[Vec<ElementId>],
    orders: &BTreeMap<ElementId, Vec<ElementId>>,
    weights: &BTreeMap<ElementId, usize>,
    name_of: &impl Fn(&ElementId) -> String,
) -> Layout {
    let mut measures = Measures::default();
    for root in columns.iter().flatten() {
        measure(root, orders, weights, name_of, &mut measures);
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
            place(root, pos2(x, y), orders, &measures, &mut layout);
            y += sizes[root].y + ROW_GAP;
        }
        x += width + COLUMN_GAP;
    }
    layout
}

/// What one arrangement pass measured: the extent of every box, and how
/// each container packs its children.
#[derive(Default)]
struct Measures {
    sizes: BTreeMap<ElementId, Vec2>,
    grids: BTreeMap<ElementId, Grid>,
}

fn measure(
    id: &ElementId,
    orders: &BTreeMap<ElementId, Vec<ElementId>>,
    weights: &BTreeMap<ElementId, usize>,
    name_of: &impl Fn(&ElementId) -> String,
    measures: &mut Measures,
) -> Vec2 {
    let label = label_width(&name_of(id));
    let size = match orders.get(id) {
        None => {
            let growth = growth_of(weights.get(id).copied().unwrap_or(0));
            vec2(label + growth, NODE_HEIGHT + growth * 0.6)
        }
        Some(inner) => {
            let children: Vec<Vec2> = inner
                .iter()
                .map(|child| measure(child, orders, weights, name_of, measures))
                .collect();
            let grid = pack(&children, label);
            let size = framed(grid.size(), label);
            measures.grids.insert(id.clone(), grid);
            size
        }
    };
    measures.sizes.insert(id.clone(), size);
    size
}

fn place(
    id: &ElementId,
    origin: Pos2,
    orders: &BTreeMap<ElementId, Vec<ElementId>>,
    measures: &Measures,
    layout: &mut Layout,
) {
    layout
        .rects
        .insert(id.clone(), Rect::from_min_size(origin, measures.sizes[id]));
    match orders.get(id) {
        None => layout.leaves.push(id.clone()),
        Some(inner) => {
            layout.containers.push(id.clone());
            let grid = &measures.grids[id];
            let content = pos2(origin.x + PADDING, origin.y + HEADER + PADDING);
            for (index, child) in inner.iter().enumerate() {
                let offset = grid.cell(index, measures.sizes[child]);
                place(child, content + offset, orders, measures, layout);
            }
        }
    }
}

/// A container's children wrapped into rows: a row fills left to right,
/// then the next row starts, so the order the caller established still
/// reads like text. A column is as wide as its widest member and a row as
/// tall as its tallest, which keeps boxes of mixed size apart.
struct Grid {
    column_widths: Vec<f32>,
    row_heights: Vec<f32>,
}

impl Grid {
    fn new(children: &[Vec2], columns: usize) -> Self {
        let columns = columns.clamp(1, children.len().max(1));
        let mut grid = Self {
            column_widths: vec![0.0; columns],
            row_heights: vec![0.0; children.len().div_ceil(columns).max(1)],
        };
        for (index, child) in children.iter().enumerate() {
            let (row, column) = grid.cell_of(index);
            grid.column_widths[column] = grid.column_widths[column].max(child.x);
            grid.row_heights[row] = grid.row_heights[row].max(child.y);
        }
        grid
    }

    fn cell_of(&self, index: usize) -> (usize, usize) {
        let columns = self.column_widths.len();
        (index / columns, index % columns)
    }

    fn size(&self) -> Vec2 {
        vec2(span(&self.column_widths), span(&self.row_heights))
    }

    /// Where the child at `index` sits relative to the content's top left.
    /// Every child of a row shares the row's middle line, so the sibling
    /// refinement reads one row as one position.
    fn cell(&self, index: usize, size: Vec2) -> Vec2 {
        let (row, column) = self.cell_of(index);
        vec2(
            after(&self.column_widths[..column]),
            after(&self.row_heights[..row]) + (self.row_heights[row] - size.y) / 2.0,
        )
    }
}

/// Wraps the children into the grid that reads best: the shape closest to
/// [`TARGET_ASPECT`] for the least wasted space. Ties go to the fewest
/// columns, so equal candidates always resolve the same way.
fn pack(children: &[Vec2], label: f32) -> Grid {
    if children.len() <= STACK_LIMIT {
        return Grid::new(children, 1);
    }
    let content: f32 = children.iter().map(|child| child.x * child.y).sum();
    (1..=children.len())
        .map(|columns| Grid::new(children, columns))
        .min_by(|a, b| {
            cost(a, label, content)
                .partial_cmp(&cost(b, label, content))
                .unwrap_or(Ordering::Equal)
        })
        .unwrap_or_else(|| Grid::new(children, 1))
}

/// How badly a grid reads: how far its box departs from the target ratio,
/// plus the space it wastes around the children. Both terms are ratios in
/// logarithmic form, so they compare across containers of any size.
fn cost(grid: &Grid, label: f32, content: f32) -> f32 {
    let size = framed(grid.size(), label);
    let shape = (size.x / (size.y * TARGET_ASPECT)).ln().abs();
    let waste = (size.x * size.y / content.max(1.0)).max(1.0).ln();
    shape + AREA_COST * waste
}

/// The container box around packed content: room for the header, the
/// padding, and at least the container's own label.
fn framed(content: Vec2, label: f32) -> Vec2 {
    vec2(
        (content.x + 2.0 * PADDING).max(label),
        content.y + HEADER + 2.0 * PADDING,
    )
}

/// The extent of a run of grid tracks, gaps included.
fn span(track: &[f32]) -> f32 {
    (after(track) - GAP).max(0.0)
}

/// Where the track after this run begins.
fn after(track: &[f32]) -> f32 {
    track.iter().sum::<f32>() + GAP * ordinal(track.len())
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

/// Reorders every sibling group by where the members' dependency partners
/// landed in a previous arrangement: a child moves toward the place its
/// partners occupy, so its edges travel the shortest way. The sibling
/// group reads as a grid, so the move is two-dimensional: the partners'
/// centroid ranks in reading order, rows before positions along a row.
/// Members without partners keep their place.
fn refine_sibling_orders(
    orders: &mut BTreeMap<ElementId, Vec<ElementId>>,
    depends: &[(ElementId, ElementId)],
    previous: &Layout,
) {
    let mut partners: BTreeMap<&ElementId, Vec<&ElementId>> = BTreeMap::new();
    for (from, to) in depends {
        partners.entry(from).or_default().push(to);
        partners.entry(to).or_default().push(from);
    }

    let mut keys: BTreeMap<ElementId, Pos2> = BTreeMap::new();
    for kids in orders.values() {
        for kid in kids {
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
    // place keep the dependency layering they arrived in.
    for kids in orders.values_mut() {
        kids.sort_by(|a, b| in_reading_order(keys[a], keys[b]));
    }
}

/// Which of two points comes first when a grid is read like text: the
/// higher one, and among points of one row the one further left. Positions
/// count in whole steps of [`GAP`], the closest two boxes ever come, so a
/// centroid that misses a row's middle line by rounding still ranks with
/// that row.
fn in_reading_order(a: Pos2, b: Pos2) -> Ordering {
    let step = |value: f32| (value / GAP).round();
    step(a.y)
        .partial_cmp(&step(b.y))
        .unwrap_or(Ordering::Equal)
        .then_with(|| step(a.x).partial_cmp(&step(b.x)).unwrap_or(Ordering::Equal))
}

fn subtree(id: &ElementId, orders: &BTreeMap<ElementId, Vec<ElementId>>) -> BTreeSet<ElementId> {
    let mut members = BTreeSet::from([id.clone()]);
    let mut queue = vec![id.clone()];
    while let Some(current) = queue.pop() {
        for child in orders.get(&current).into_iter().flatten() {
            if members.insert(child.clone()) {
                queue.push(child.clone());
            }
        }
    }
    members
}

/// Siblings ordered by their dependency layering, ties by id.
fn ordered_by_layer(siblings: &[ElementId], depends: &[(ElementId, ElementId)]) -> Vec<ElementId> {
    let set: BTreeSet<&ElementId> = siblings.iter().collect();
    let local: Vec<(ElementId, ElementId)> = depends
        .iter()
        .filter(|(from, to)| set.contains(from) && set.contains(to))
        .cloned()
        .collect();
    let layer = layers(siblings, &local);
    let mut result: Vec<ElementId> = siblings.to_vec();
    result.sort_by_key(|id| (layer[id], id.clone()));
    result
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
    use cutaway_architecture::{Element, ElementKind, ElementName, Relation};

    fn add_package(graph: &mut ArchitectureGraph, name: &str) -> ElementId {
        let id = ElementId::new(format!("package:{name}")).unwrap();
        graph
            .add_element(Element {
                id: id.clone(),
                name: ElementName::new(name).unwrap(),
                kind: ElementKind::Package,
            })
            .unwrap();
        id
    }

    fn add_module(graph: &mut ArchitectureGraph, parent: &ElementId, path: &str) -> ElementId {
        let id = ElementId::new(path).unwrap();
        graph
            .add_element(Element {
                id: id.clone(),
                name: ElementName::new(path).unwrap(),
                kind: ElementKind::Module,
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

        let layout = compute(&graph, &no_weights());
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

        let layout = compute(&graph, &no_weights());
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
        let layout = compute(&graph, &weights);
        assert!(layout.rects[&heavy].area() > layout.rects[&light].area());
    }

    #[test]
    fn a_crowded_container_grows_wide_instead_of_tall() {
        let mut graph = ArchitectureGraph::new();
        let package = add_package(&mut graph, "crowded");
        for index in 0..16 {
            add_module(&mut graph, &package, &format!("crowded/m{index:02}.rs"));
        }

        let layout = compute(&graph, &no_weights());
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

        let layout = compute(&graph, &no_weights());
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
    fn packing_keeps_the_dependency_reading_order() {
        let mut graph = ArchitectureGraph::new();
        let package = add_package(&mut graph, "chain");
        let chain: Vec<ElementId> = (0..9)
            .map(|index| add_module(&mut graph, &package, &format!("chain/m{index}.rs")))
            .collect();
        for pair in chain.windows(2) {
            depend(&mut graph, &pair[0], &pair[1]);
        }

        let layout = compute(&graph, &no_weights());
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
    fn columns_reorder_so_that_edges_do_not_cross() {
        let mut graph = ArchitectureGraph::new();
        let upper = add_package(&mut graph, "a-upper");
        let lower = add_package(&mut graph, "b-lower");
        let crossed = add_package(&mut graph, "x-crossed");
        let straight = add_package(&mut graph, "y-straight");
        // With alphabetic order in both columns these two edges cross.
        depend(&mut graph, &upper, &straight);
        depend(&mut graph, &lower, &crossed);

        let layout = compute(&graph, &no_weights());
        let above =
            |a: &ElementId, b: &ElementId| layout.rects[a].center().y < layout.rects[b].center().y;
        assert_eq!(above(&upper, &lower), above(&straight, &crossed));
    }
}
