//! Spatial arrangement of a boundary view.
//!
//! Top-level boundaries form columns: a boundary sits one column right of
//! the boundaries that depend on it, so dependencies read left to right.
//! A boundary with children is a container box; its children stack
//! vertically inside it, ordered by their dependencies on each other.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{ArchitectureGraph, ElementId, RelationKind};
use eframe::egui::{Rect, Vec2, pos2, vec2};

const NODE_HEIGHT: f32 = 30.0;
const PADDING: f32 = 14.0;
const HEADER: f32 = 26.0;
const GAP: f32 = 16.0;
const COLUMN_GAP: f32 = 90.0;
const ROW_GAP: f32 = 40.0;

pub struct Layout {
    pub rects: BTreeMap<ElementId, Rect>,
    /// Boundaries with children, outermost first: paint these as boxes.
    pub containers: Vec<ElementId>,
    /// Boundaries without children: paint these as nodes.
    pub leaves: Vec<ElementId>,
}

pub fn compute(view: &ArchitectureGraph) -> Layout {
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

    // Order every sibling group and the roots by dependency layers.
    let root_edges: Vec<(ElementId, ElementId)> = depends
        .iter()
        .map(|(from, to)| (root_of(from), root_of(to)))
        .filter(|(from, to)| from != to)
        .collect();
    let root_layers = layers(&roots, &root_edges);

    let mut layout = Layout {
        rects: BTreeMap::new(),
        containers: Vec::new(),
        leaves: Vec::new(),
    };

    // Measure bottom-up, then place top-down, column by column.
    let name_of = |id: &ElementId| -> String {
        view.element(id)
            .map_or_else(String::new, |e| e.name.to_string())
    };
    let mut sizes: BTreeMap<ElementId, Vec2> = BTreeMap::new();
    for root in &roots {
        measure(root, &children, &depends, &name_of, &mut sizes);
    }

    let mut columns: BTreeMap<usize, Vec<ElementId>> = BTreeMap::new();
    for root in &roots {
        columns
            .entry(root_layers[root])
            .or_default()
            .push(root.clone());
    }

    let mut x = 0.0;
    for column in columns.values() {
        let width = column.iter().map(|id| sizes[id].x).fold(0.0_f32, f32::max);
        let mut y = 0.0;
        for root in column {
            place(root, pos2(x, y), &children, &depends, &sizes, &mut layout);
            y += sizes[root].y + ROW_GAP;
        }
        x += width + COLUMN_GAP;
    }
    layout
}

fn measure(
    id: &ElementId,
    children: &BTreeMap<ElementId, Vec<ElementId>>,
    depends: &[(ElementId, ElementId)],
    name_of: &impl Fn(&ElementId) -> String,
    sizes: &mut BTreeMap<ElementId, Vec2>,
) -> Vec2 {
    let label = label_width(&name_of(id));
    let size = match children.get(id) {
        None => vec2(label, NODE_HEIGHT),
        Some(inner) => {
            let mut width = label;
            let mut height = HEADER + PADDING;
            for child in ordered(inner, depends) {
                let child_size = measure(&child, children, depends, name_of, sizes);
                width = width.max(child_size.x + 2.0 * PADDING);
                height += child_size.y + GAP;
            }
            vec2(width, height - GAP + PADDING)
        }
    };
    sizes.insert(id.clone(), size);
    size
}

fn place(
    id: &ElementId,
    origin: eframe::egui::Pos2,
    children: &BTreeMap<ElementId, Vec<ElementId>>,
    depends: &[(ElementId, ElementId)],
    sizes: &BTreeMap<ElementId, Vec2>,
    layout: &mut Layout,
) {
    layout
        .rects
        .insert(id.clone(), Rect::from_min_size(origin, sizes[id]));
    match children.get(id) {
        None => layout.leaves.push(id.clone()),
        Some(inner) => {
            layout.containers.push(id.clone());
            let mut y = origin.y + HEADER + PADDING;
            for child in ordered(inner, depends) {
                place(
                    &child,
                    pos2(origin.x + PADDING, y),
                    children,
                    depends,
                    sizes,
                    layout,
                );
                y += sizes[&child].y + GAP;
            }
        }
    }
}

/// Siblings ordered by their dependency layering, ties by id.
fn ordered(siblings: &[ElementId], depends: &[(ElementId, ElementId)]) -> Vec<ElementId> {
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

fn label_width(name: &str) -> f32 {
    let chars: f32 = f32::from(u16::try_from(name.chars().count()).unwrap_or(u16::MAX));
    (chars * 7.4 + 26.0).max(60.0)
}
