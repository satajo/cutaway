//! Compares two versions of an architecture.
//!
//! Given the architecture graphs of two versions of the same project, this
//! crate answers three questions, each built on the one before it:
//!
//! - What appeared and what disappeared: [`ArchitectureDelta`], a plain list
//!   of added and removed elements and relations.
//! - What the picture lays out: the union graph, every element and relation
//!   either version holds, so one drawing carries both versions at once.
//! - What each drawn box says about the change: [`Comparison::readings_at`],
//!   which attributes every change to the nearest boundary the picture
//!   actually draws.
//!
//! Elements match by id across versions, because ids derive deterministically
//! from the sources. An element whose id survives is the same element even if
//! its name or kind differs.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{ArchitectureGraph, Element, ElementId, Relation, RelationKind};

/// The difference between two versions of an architecture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchitectureDelta {
    pub added_elements: Vec<Element>,
    pub removed_elements: Vec<Element>,
    /// Elements present in both versions whose id survived but which
    /// differ: in either reading - name or kind - or in fingerprint where
    /// both versions carry one; a `None` on either side is absence of
    /// evidence, not a change. The list holds the after shape.
    pub changed_elements: Vec<Element>,
    pub added_relations: Vec<Relation>,
    pub removed_relations: Vec<Relation>,
}

impl ArchitectureDelta {
    #[must_use]
    pub fn between(before: &ArchitectureGraph, after: &ArchitectureGraph) -> Self {
        Self {
            added_elements: after
                .elements()
                .filter(|element| before.element(&element.id).is_none())
                .cloned()
                .collect(),
            removed_elements: before
                .elements()
                .filter(|element| after.element(&element.id).is_none())
                .cloned()
                .collect(),
            changed_elements: after
                .elements()
                .filter(|element| {
                    before
                        .element(&element.id)
                        .is_some_and(|earlier| differs(earlier, element))
                })
                .cloned()
                .collect(),
            added_relations: after
                .relations()
                .filter(|relation| !before.relations().any(|r| r == *relation))
                .cloned()
                .collect(),
            removed_relations: before
                .relations()
                .filter(|relation| !after.relations().any(|r| r == *relation))
                .cloned()
                .collect(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added_elements.is_empty()
            && self.removed_elements.is_empty()
            && self.changed_elements.is_empty()
            && self.added_relations.is_empty()
            && self.removed_relations.is_empty()
    }
}

/// Whether an element whose id survived differs between the versions: in
/// either of its readings - a name or a kind a language or the tree changed
/// its mind about, an aspect gained or lost - or in fingerprint where both
/// versions carry one. A fingerprint on one side alone is absence of
/// evidence, not a change.
fn differs(before: &Element, after: &Element) -> bool {
    before.semantic_aspect() != after.semantic_aspect()
        || before.substrate_aspect() != after.substrate_aspect()
        || matches!(
            (before.fingerprint, after.fingerprint),
            (Some(earlier), Some(later)) if earlier != later
        )
}

/// Two versions of one architecture, held together: the union graph the
/// picture lays out, and the delta that says what changed between them.
///
/// The picture draws the union, so a box exists wherever either version has
/// something to say, and asks this type how each box reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    union: ArchitectureGraph,
    delta: ArchitectureDelta,
    elements: BTreeMap<ElementId, Presence>,
    relations: BTreeMap<Relation, Presence>,
    /// The containment parents of every contained element of the union.
    ///
    /// Containment forks where an element moved: the union then holds a
    /// `Contains` edge from the parent of each version. The climb to the
    /// nearest rendered boundary walks every parent, so a moved element still
    /// speaks from the nearest boundary that held it in either version.
    parents: BTreeMap<ElementId, BTreeSet<ElementId>>,
}

impl Comparison {
    #[must_use]
    pub fn between(before: &ArchitectureGraph, after: &ArchitectureGraph) -> Self {
        let union = union_of(before, after);
        let elements = union
            .elements()
            .map(|element| {
                let presence = Presence::of(
                    before.element(&element.id).is_some(),
                    after.element(&element.id).is_some(),
                )
                .expect("every union element comes from one of the two versions");
                (element.id.clone(), presence)
            })
            .collect();
        let relations = union
            .relations()
            .map(|relation| {
                let presence = Presence::of(
                    before.relations().any(|r| r == relation),
                    after.relations().any(|r| r == relation),
                )
                .expect("every union relation comes from one of the two versions");
                (relation.clone(), presence)
            })
            .collect();
        Self {
            // Both versions speak here, while the union draws the newer
            // place alone: a change out of sight must still reach whichever
            // boundary held the element, in either version.
            parents: containment_parents(before.relations().chain(after.relations())),
            delta: ArchitectureDelta::between(before, after),
            elements,
            relations,
            union,
        }
    }

    /// The graph the comparison picture lays out: every element and relation
    /// of either version.
    #[must_use]
    pub fn union(&self) -> &ArchitectureGraph {
        &self.union
    }

    #[must_use]
    pub fn delta(&self) -> &ArchitectureDelta {
        &self.delta
    }

    /// Where the element stands across the two versions, and `None` while
    /// neither version holds it. Callers ask about members of the union, so
    /// `None` means the question was about something else entirely.
    #[must_use]
    pub fn presence_of_element(&self, id: &ElementId) -> Option<Presence> {
        self.elements.get(id).copied()
    }

    /// Where the relation stands across the two versions, and `None` while
    /// neither version holds it.
    #[must_use]
    pub fn presence_of_relation(&self, relation: &Relation) -> Option<Presence> {
        self.relations.get(relation).copied()
    }

    /// How each drawn boundary reads, given the set of boundaries the picture
    /// renders. Boundaries the comparison does not touch stay out of the map,
    /// so the picture draws them as the architecture has them.
    ///
    /// Every change speaks from the nearest rendered boundary at or above it,
    /// the same way a rolled-up edge attaches to the nearest boundary that
    /// draws: a boundary that arrives reads [`ElementChange::Added`], one that
    /// goes reads [`ElementChange::Removed`], and one that survives while
    /// something changes out of sight inside it reads
    /// [`ElementChange::Modified`].
    #[must_use]
    pub fn readings_at(
        &self,
        rendered: &BTreeSet<ElementId>,
    ) -> BTreeMap<ElementId, ElementChange> {
        let mut readings = BTreeMap::new();
        for id in rendered {
            let reading = match self.presence_of_element(id) {
                Some(Presence::OnlyAfter) => ElementChange::Added,
                Some(Presence::OnlyBefore) => ElementChange::Removed,
                // A surviving boundary speaks for what changed inside it, and
                // an id outside the union draws nothing to speak with.
                Some(Presence::InBoth) | None => continue,
            };
            readings.insert(id.clone(), reading);
        }

        for boundary in self.boundaries_hiding_change(rendered) {
            // Added and Removed win over Modified: what is arriving or going
            // is not "changed inside". Only a boundary present in both
            // versions hides a change, and the loop above marked no such
            // boundary, so no reading is overwritten here.
            readings.insert(boundary, ElementChange::Modified);
        }
        readings
    }

    /// The rendered boundaries that survive both versions and carry a
    /// change: churn hidden inside them, or a change of the element itself -
    /// its name, kind, or contents.
    fn boundaries_hiding_change(&self, rendered: &BTreeSet<ElementId>) -> BTreeSet<ElementId> {
        let mut hiding = BTreeSet::new();

        // An element that appeared or disappeared without being drawn itself
        // is churn inside whatever boundary holds it.
        let churned = self
            .delta
            .added_elements
            .iter()
            .chain(&self.delta.removed_elements)
            .map(|element| &element.id)
            .filter(|id| !rendered.contains(*id));
        hiding.extend(churned.filter_map(|id| self.nearest_rendered(rendered, id)));

        // A changed element speaks from wherever it is drawn: from itself
        // when rendered - the climb starts at the element, so a rendered one
        // answers with itself - and from the nearest rendered boundary above
        // it otherwise.
        let changed = self
            .delta
            .changed_elements
            .iter()
            .map(|element| &element.id);
        hiding.extend(changed.filter_map(|id| self.nearest_rendered(rendered, id)));

        // A dependency that appeared or disappeared belongs to the rendered
        // edge between its two endpoints - unless both endpoints roll up to
        // the same boundary, which leaves no edge to carry it. Containment
        // changes contribute nothing of their own: an element arriving or
        // leaving is already counted above, and a reparenting shows through
        // the moved element's own entries in the delta.
        let dependencies = self
            .delta
            .added_relations
            .iter()
            .chain(&self.delta.removed_relations)
            .filter(|relation| relation.kind == RelationKind::DependsOn);
        for relation in dependencies {
            let from = self.nearest_rendered(rendered, &relation.from);
            let to = self.nearest_rendered(rendered, &relation.to);
            if let (Some(from), Some(to)) = (from, to)
                && from == to
            {
                hiding.insert(from);
            }
        }

        hiding.retain(|id| self.presence_of_element(id) == Some(Presence::InBoth));
        hiding
    }

    /// The nearest boundary the picture renders at or above the element,
    /// climbing the union's containment one level at a time.
    fn nearest_rendered(
        &self,
        rendered: &BTreeSet<ElementId>,
        id: &ElementId,
    ) -> Option<ElementId> {
        let mut visited = BTreeSet::new();
        let mut level = BTreeSet::from([id.clone()]);
        while !level.is_empty() {
            if let Some(hit) = level.iter().find(|candidate| rendered.contains(*candidate)) {
                return Some(hit.clone());
            }
            let mut above = BTreeSet::new();
            for candidate in level {
                // Visiting each element once ends the climb even if the
                // union's containment closes into a cycle.
                if visited.insert(candidate.clone())
                    && let Some(parents) = self.parents.get(&candidate)
                {
                    above.extend(parents.iter().cloned());
                }
            }
            level = above;
        }
        None
    }
}

/// Where one element or relation stands across the two versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    InBoth,
    OnlyBefore,
    OnlyAfter,
}

impl Presence {
    /// None while neither version holds the thing asked about.
    fn of(in_before: bool, in_after: bool) -> Option<Self> {
        match (in_before, in_after) {
            (true, true) => Some(Self::InBoth),
            (true, false) => Some(Self::OnlyBefore),
            (false, true) => Some(Self::OnlyAfter),
            (false, false) => None,
        }
    }
}

/// How one rendered boundary reads in the comparison picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementChange {
    /// The boundary exists only in the newer version.
    Added,
    /// The boundary exists only in the older version.
    Removed,
    /// The boundary stands in both versions and hides a change inside itself:
    /// something beneath it arrived, left, rewired, or changed - its own
    /// name, kind, or contents included. Neither what is arriving nor what
    /// is going describes it.
    Modified,
}

/// Every element and relation of either version, in one graph.
///
/// Where an id exists in both versions, the union takes the newer element:
/// the comparison reads toward the change, so the picture shows the name,
/// kind, and fingerprint the project is arriving at - and the place too. An
/// element that survives is drawn where it is arriving, so the union holds
/// the newer version's `Contains` edge alone: a picture nests as a tree, and
/// an element held by the place it left and the place it reached at once is
/// no tree. What the older version made of the move is not lost - the
/// readings still climb from both places, through [`Comparison::parents`].
///
/// A departing element keeps the older version's edges: nothing newer says
/// where it stood.
fn union_of(before: &ArchitectureGraph, after: &ArchitectureGraph) -> ArchitectureGraph {
    let mut union = ArchitectureGraph::new();
    let surviving_or_arriving = after.elements();
    let departing = before
        .elements()
        .filter(|element| after.element(&element.id).is_none());
    for element in surviving_or_arriving.chain(departing) {
        union
            .add_element(element.clone())
            .expect("each id enters the union once: the after version wins the shared ones");
    }

    let superseded = |relation: &&Relation| {
        relation.kind == RelationKind::Contains && after.element(&relation.to).is_some()
    };
    let relations: BTreeSet<&Relation> = before
        .relations()
        .filter(|relation| !superseded(relation))
        .chain(after.relations())
        .collect();
    for relation in relations {
        union
            .add_relation(relation.clone())
            .expect("both endpoints came in with their own version, and the set holds no repeats");
    }
    union
}

/// The containment parents of every contained element, over relations from
/// either version: an element that moved answers with both places it stood.
fn containment_parents<'a>(
    relations: impl Iterator<Item = &'a Relation>,
) -> BTreeMap<ElementId, BTreeSet<ElementId>> {
    let mut parents: BTreeMap<ElementId, BTreeSet<ElementId>> = BTreeMap::new();
    for relation in relations.filter(|relation| relation.kind == RelationKind::Contains) {
        parents
            .entry(relation.to.clone())
            .or_default()
            .insert(relation.from.clone());
    }
    parents
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{
        ElementKind, ElementName, Fingerprint, SemanticKind, SubstrateKind,
    };

    use super::*;

    fn element(id: &str) -> Element {
        Element::semantic(
            ElementId::new(id).unwrap(),
            SemanticKind::Module,
            ElementName::new(id).unwrap(),
        )
    }

    fn fingerprinted(id: &str, contents: &[u8]) -> Element {
        let mut element = element(id);
        element.fingerprint = Some(Fingerprint::of(contents));
        element
    }

    fn graph_of(ids: &[&str]) -> ArchitectureGraph {
        let mut graph = ArchitectureGraph::new();
        for id in ids {
            graph.add_element(element(id)).unwrap();
        }
        graph
    }

    fn version_of(elements: Vec<Element>, relations: &[Relation]) -> ArchitectureGraph {
        let mut graph = ArchitectureGraph::new();
        for element in elements {
            graph.add_element(element).unwrap();
        }
        for relation in relations {
            graph.add_relation(relation.clone()).unwrap();
        }
        graph
    }

    fn id(id: &str) -> ElementId {
        ElementId::new(id).unwrap()
    }

    fn contains(parent: &str, child: &str) -> Relation {
        Relation {
            from: id(parent),
            to: id(child),
            kind: RelationKind::Contains,
        }
    }

    fn depends(from: &str, to: &str) -> Relation {
        Relation {
            from: id(from),
            to: id(to),
            kind: RelationKind::DependsOn,
        }
    }

    fn version(ids: &[&str], relations: &[Relation]) -> ArchitectureGraph {
        version_of(ids.iter().map(|id| element(id)).collect(), relations)
    }

    fn rendered(ids: &[&str]) -> BTreeSet<ElementId> {
        ids.iter().map(|id| ElementId::new(*id).unwrap()).collect()
    }

    fn readings(
        before: &ArchitectureGraph,
        after: &ArchitectureGraph,
        boundaries: &[&str],
    ) -> BTreeMap<ElementId, ElementChange> {
        Comparison::between(before, after).readings_at(&rendered(boundaries))
    }

    #[test]
    fn two_identical_versions_have_an_empty_delta() {
        let delta = ArchitectureDelta::between(&graph_of(&["a"]), &graph_of(&["a"]));
        assert!(delta.is_empty());
    }

    #[test]
    fn an_element_only_in_the_newer_version_counts_as_added() {
        let delta = ArchitectureDelta::between(&graph_of(&["a"]), &graph_of(&["a", "b"]));
        assert_eq!(delta.added_elements, [element("b")]);
        assert!(delta.removed_elements.is_empty());
    }

    #[test]
    fn an_element_only_in_the_older_version_counts_as_removed() {
        let delta = ArchitectureDelta::between(&graph_of(&["a", "b"]), &graph_of(&["a"]));
        assert_eq!(delta.removed_elements, [element("b")]);
        assert!(delta.added_elements.is_empty());
    }

    #[test]
    fn a_relation_only_in_the_newer_version_counts_as_added() {
        let before = graph_of(&["a", "b"]);
        let mut after = before.clone();
        let relation = Relation {
            from: ElementId::new("a").unwrap(),
            to: ElementId::new("b").unwrap(),
            kind: RelationKind::DependsOn,
        };
        after.add_relation(relation.clone()).unwrap();

        let delta = ArchitectureDelta::between(&before, &after);
        assert_eq!(delta.added_relations, [relation]);
    }

    #[test]
    fn the_union_of_two_versions_holds_everything_either_version_holds() {
        let before = version(&["p", "a"], &[contains("p", "a")]);
        let after = version(&["p", "b"], &[contains("p", "b")]);

        let comparison = Comparison::between(&before, &after);
        let union = comparison.union();

        let ids: Vec<&str> = union.elements().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "p"]);
        let relations: Vec<&Relation> = union.relations().collect();
        assert_eq!(relations, [&contains("p", "a"), &contains("p", "b")]);
    }

    #[test]
    fn an_element_surviving_both_versions_enters_the_union_in_its_newer_shape() {
        let before = graph_of(&["a"]);
        let mut after = ArchitectureGraph::new();
        after
            .add_element(Element::semantic(
                id("a"),
                SemanticKind::Type,
                ElementName::new("renamed").unwrap(),
            ))
            .unwrap();

        let comparison = Comparison::between(&before, &after);

        let survivor = comparison.union().element(&id("a")).unwrap();
        assert_eq!(survivor.primary_name().as_str(), "renamed");
        assert_eq!(survivor.primary_kind(), ElementKind::Type);
    }

    #[test]
    fn an_element_only_in_the_newer_version_is_present_only_after() {
        let comparison = Comparison::between(&graph_of(&["a"]), &graph_of(&["a", "b"]));

        assert_eq!(
            comparison.presence_of_element(&id("b")),
            Some(Presence::OnlyAfter)
        );
        assert_eq!(
            comparison.presence_of_element(&id("a")),
            Some(Presence::InBoth)
        );
        assert_eq!(
            Comparison::between(&graph_of(&["a", "b"]), &graph_of(&["a"]))
                .presence_of_element(&id("b")),
            Some(Presence::OnlyBefore)
        );
    }

    #[test]
    fn something_neither_version_holds_has_no_presence() {
        let comparison = Comparison::between(&graph_of(&["a"]), &graph_of(&["a"]));

        assert_eq!(comparison.presence_of_element(&id("elsewhere")), None);
        assert_eq!(comparison.presence_of_relation(&depends("a", "a")), None);
    }

    #[test]
    fn a_rendered_element_that_arrives_reads_as_added() {
        assert_eq!(
            readings(&graph_of(&["a"]), &graph_of(&["a", "b"]), &["a", "b"]),
            BTreeMap::from([(id("b"), ElementChange::Added)])
        );
    }

    #[test]
    fn a_rendered_element_that_goes_reads_as_removed() {
        assert_eq!(
            readings(&graph_of(&["a", "b"]), &graph_of(&["a"]), &["a", "b"]),
            BTreeMap::from([(id("b"), ElementChange::Removed)])
        );
    }

    #[test]
    fn a_closed_boundary_reads_as_modified_when_an_element_appears_inside_it() {
        let before = version(&["p"], &[]);
        let after = version(&["p", "a"], &[contains("p", "a")]);

        assert_eq!(
            readings(&before, &after, &["p"]),
            BTreeMap::from([(id("p"), ElementChange::Modified)])
        );
    }

    #[test]
    fn a_closed_boundary_reads_as_modified_when_an_element_disappears_inside_it() {
        let before = version(&["p", "a"], &[contains("p", "a")]);
        let after = version(&["p"], &[]);

        assert_eq!(
            readings(&before, &after, &["p"]),
            BTreeMap::from([(id("p"), ElementChange::Modified)])
        );
    }

    #[test]
    fn opening_a_boundary_moves_the_reading_onto_the_change_itself() {
        let before = version(&["p"], &[]);
        let after = version(&["p", "a"], &[contains("p", "a")]);

        assert_eq!(
            readings(&before, &after, &["p", "a"]),
            BTreeMap::from([(id("a"), ElementChange::Added)])
        );
    }

    #[test]
    fn a_dependency_changing_between_two_rendered_boundaries_leaves_both_unmarked() {
        let structure = [contains("p", "a"), contains("q", "b")];
        let before = version(&["p", "q", "a", "b"], &structure);
        let mut after = before.clone();
        after.add_relation(depends("a", "b")).unwrap();

        assert!(readings(&before, &after, &["p", "q"]).is_empty());
    }

    #[test]
    fn a_dependency_changing_wholly_inside_one_closed_boundary_reads_as_modified() {
        let structure = [contains("p", "a"), contains("p", "b")];
        let before = version(&["p", "a", "b"], &structure);
        let mut after = before.clone();
        after.add_relation(depends("a", "b")).unwrap();

        assert_eq!(
            readings(&before, &after, &["p"]),
            BTreeMap::from([(id("p"), ElementChange::Modified)])
        );
    }

    #[test]
    fn a_boundary_on_its_way_out_reads_as_removed_whatever_churns_inside_it() {
        let before = version(&["p", "a"], &[contains("p", "a"), depends("a", "a")]);
        let after = ArchitectureGraph::new();

        assert_eq!(
            readings(&before, &after, &["p"]),
            BTreeMap::from([(id("p"), ElementChange::Removed)])
        );
    }

    #[test]
    fn two_identical_versions_read_nothing_at_any_rendering() {
        let unchanged = version(&["p", "a"], &[contains("p", "a")]);

        assert!(readings(&unchanged, &unchanged, &["p"]).is_empty());
        assert!(readings(&unchanged, &unchanged, &["p", "a"]).is_empty());
    }

    #[test]
    fn a_change_climbs_past_unrendered_boundaries_to_the_nearest_rendered_one() {
        let structure = [contains("p", "d"), contains("d", "e")];
        let before = version(&["p", "d", "e"], &structure);
        let mut after = before.clone();
        after.add_element(element("m")).unwrap();
        after.add_relation(contains("e", "m")).unwrap();

        assert_eq!(
            readings(&before, &after, &["p"]),
            BTreeMap::from([(id("p"), ElementChange::Modified)])
        );
        assert_eq!(
            readings(&before, &after, &["p", "d", "e"]),
            BTreeMap::from([(id("e"), ElementChange::Modified)])
        );
    }

    #[test]
    fn an_element_whose_fingerprint_changed_reads_as_modified_where_it_is_rendered() {
        let before = version_of(vec![fingerprinted("a", b"old contents")], &[]);
        let after = version_of(vec![fingerprinted("a", b"new contents")], &[]);

        assert_eq!(
            readings(&before, &after, &["a"]),
            BTreeMap::from([(id("a"), ElementChange::Modified)])
        );
    }

    #[test]
    fn a_hidden_fingerprint_change_reads_as_modified_on_the_nearest_rendered_ancestor() {
        let structure = [contains("p", "a")];
        let before = version_of(
            vec![element("p"), fingerprinted("a", b"old contents")],
            &structure,
        );
        let after = version_of(
            vec![element("p"), fingerprinted("a", b"new contents")],
            &structure,
        );

        assert_eq!(
            readings(&before, &after, &["p"]),
            BTreeMap::from([(id("p"), ElementChange::Modified)])
        );
    }

    #[test]
    fn a_fingerprint_appearing_or_disappearing_is_not_a_change() {
        let bare = version_of(vec![element("a")], &[]);
        let carrying = version_of(vec![fingerprinted("a", b"contents")], &[]);

        assert!(ArchitectureDelta::between(&bare, &carrying).is_empty());
        assert!(ArchitectureDelta::between(&carrying, &bare).is_empty());
    }

    #[test]
    fn a_name_change_with_a_surviving_id_reads_as_modified() {
        let renamed = Element::semantic(
            id("a"),
            SemanticKind::Module,
            ElementName::new("renamed").unwrap(),
        );
        let before = graph_of(&["a"]);
        let after = version_of(vec![renamed], &[]);

        assert_eq!(
            readings(&before, &after, &["a"]),
            BTreeMap::from([(id("a"), ElementChange::Modified)])
        );
    }

    #[test]
    fn an_arriving_boundary_reads_as_added_even_when_a_change_hides_inside_it() {
        let before = version_of(vec![fingerprinted("a", b"old contents")], &[]);
        let after = version_of(
            vec![element("p"), fingerprinted("a", b"new contents")],
            &[contains("p", "a")],
        );

        assert_eq!(
            readings(&before, &after, &["p"]),
            BTreeMap::from([(id("p"), ElementChange::Added)])
        );
    }

    #[test]
    fn a_departing_boundary_reads_as_removed_even_when_a_change_hides_inside_it() {
        let before = version_of(
            vec![element("p"), fingerprinted("a", b"old contents")],
            &[contains("p", "a")],
        );
        let after = version_of(vec![fingerprinted("a", b"new contents")], &[]);

        assert_eq!(
            readings(&before, &after, &["p"]),
            BTreeMap::from([(id("p"), ElementChange::Removed)])
        );
    }

    #[test]
    fn a_module_becoming_a_file_reads_as_modified() {
        let file = Element::substrate(
            id("a"),
            SubstrateKind::File,
            ElementName::new("a").unwrap(),
            None,
        );
        let before = graph_of(&["a"]);
        let after = version_of(vec![file], &[]);

        assert_eq!(
            readings(&before, &after, &["a"]),
            BTreeMap::from([(id("a"), ElementChange::Modified)])
        );
    }

    #[test]
    fn a_changed_element_drawn_beside_its_parent_marks_itself_alone() {
        let structure = [contains("p", "a")];
        let before = version_of(
            vec![element("p"), fingerprinted("a", b"old contents")],
            &structure,
        );
        let after = version_of(
            vec![element("p"), fingerprinted("a", b"new contents")],
            &structure,
        );

        assert_eq!(
            readings(&before, &after, &["p", "a"]),
            BTreeMap::from([(id("a"), ElementChange::Modified)])
        );
    }

    #[test]
    fn a_delta_with_only_a_changed_element_is_not_empty() {
        let delta = ArchitectureDelta::between(
            &version_of(vec![fingerprinted("a", b"old contents")], &[]),
            &version_of(vec![fingerprinted("a", b"new contents")], &[]),
        );

        assert!(!delta.is_empty());
        assert_eq!(
            delta.changed_elements,
            [fingerprinted("a", b"new contents")]
        );
    }
}
