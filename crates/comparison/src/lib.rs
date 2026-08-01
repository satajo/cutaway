//! Compares two versions of an architecture.
//!
//! This crate backs the version-delta views: given the architecture graphs
//! of two versions of the same project, it computes what appeared and what
//! disappeared. An element whose id survives counts as unchanged even if its
//! name or kind differs; a richer change model arrives with the comparison
//! views that need it.

use cutaway_architecture::{ArchitectureGraph, Element, Relation};

/// The difference between two versions of an architecture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchitectureDelta {
    pub added_elements: Vec<Element>,
    pub removed_elements: Vec<Element>,
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
            && self.added_relations.is_empty()
            && self.removed_relations.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{ElementId, ElementKind, ElementName, RelationKind};

    use super::*;

    fn element(id: &str) -> Element {
        Element {
            id: ElementId::new(id).unwrap(),
            name: ElementName::new(id).unwrap(),
            kind: ElementKind::Module,
        }
    }

    fn graph_of(ids: &[&str]) -> ArchitectureGraph {
        let mut graph = ArchitectureGraph::new();
        for id in ids {
            graph.add_element(element(id)).unwrap();
        }
        graph
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
}
