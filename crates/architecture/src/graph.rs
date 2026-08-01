use std::collections::{BTreeMap, BTreeSet};

use crate::element::{Element, ElementId};
use crate::relation::Relation;

/// The architecture of one version of a software project.
///
/// Invariants, enforced on every mutation:
/// - Element ids are unique.
/// - Every relation is unique and both of its endpoints exist in the graph.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchitectureGraph {
    elements: BTreeMap<ElementId, Element>,
    relations: BTreeSet<Relation>,
}

impl ArchitectureGraph {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_element(&mut self, element: Element) -> Result<(), GraphError> {
        if self.elements.contains_key(&element.id) {
            return Err(GraphError::DuplicateElement { id: element.id });
        }
        self.elements.insert(element.id.clone(), element);
        Ok(())
    }

    /// Fails while any relation still refers to the element: the caller must
    /// retract those relations first, which keeps removals explicit instead
    /// of silently cascading.
    pub fn remove_element(&mut self, id: &ElementId) -> Result<Element, GraphError> {
        if !self.elements.contains_key(id) {
            return Err(GraphError::UnknownElement { id: id.clone() });
        }
        if self.relations.iter().any(|r| &r.from == id || &r.to == id) {
            return Err(GraphError::ElementInUse { id: id.clone() });
        }
        Ok(self
            .elements
            .remove(id)
            .expect("presence was checked above"))
    }

    pub fn add_relation(&mut self, relation: Relation) -> Result<(), GraphError> {
        for endpoint in [&relation.from, &relation.to] {
            if !self.elements.contains_key(endpoint) {
                return Err(GraphError::UnknownElement {
                    id: endpoint.clone(),
                });
            }
        }
        if !self.relations.insert(relation.clone()) {
            return Err(GraphError::DuplicateRelation { relation });
        }
        Ok(())
    }

    pub fn remove_relation(&mut self, relation: &Relation) -> Result<(), GraphError> {
        if self.relations.remove(relation) {
            Ok(())
        } else {
            Err(GraphError::UnknownRelation {
                relation: relation.clone(),
            })
        }
    }

    #[must_use]
    pub fn element(&self, id: &ElementId) -> Option<&Element> {
        self.elements.get(id)
    }

    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.elements.values()
    }

    pub fn relations(&self) -> impl Iterator<Item = &Relation> {
        self.relations.iter()
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum GraphError {
    #[error("element {id} is already in the graph")]
    DuplicateElement { id: ElementId },
    #[error("element {id} is not in the graph")]
    UnknownElement { id: ElementId },
    #[error("element {id} still has relations attached")]
    ElementInUse { id: ElementId },
    #[error("relation \"{relation}\" is already in the graph")]
    DuplicateRelation { relation: Relation },
    #[error("relation \"{relation}\" is not in the graph")]
    UnknownRelation { relation: Relation },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{ElementKind, ElementName};
    use crate::relation::RelationKind;

    fn element(id: &str) -> Element {
        Element {
            id: ElementId::new(id).unwrap(),
            name: ElementName::new(id).unwrap(),
            kind: ElementKind::Module,
        }
    }

    fn relation(from: &str, to: &str) -> Relation {
        Relation {
            from: ElementId::new(from).unwrap(),
            to: ElementId::new(to).unwrap(),
            kind: RelationKind::Contains,
        }
    }

    #[test]
    fn every_element_id_is_unique_within_a_graph() {
        let mut graph = ArchitectureGraph::new();
        graph.add_element(element("a")).unwrap();
        assert_eq!(
            graph.add_element(element("a")),
            Err(GraphError::DuplicateElement {
                id: ElementId::new("a").unwrap()
            })
        );
    }

    #[test]
    fn a_relation_requires_both_of_its_endpoints_to_exist() {
        let mut graph = ArchitectureGraph::new();
        graph.add_element(element("a")).unwrap();
        assert_eq!(
            graph.add_relation(relation("a", "missing")),
            Err(GraphError::UnknownElement {
                id: ElementId::new("missing").unwrap()
            })
        );
    }

    #[test]
    fn the_same_relation_cannot_be_added_twice() {
        let mut graph = ArchitectureGraph::new();
        graph.add_element(element("a")).unwrap();
        graph.add_element(element("b")).unwrap();
        graph.add_relation(relation("a", "b")).unwrap();
        assert_eq!(
            graph.add_relation(relation("a", "b")),
            Err(GraphError::DuplicateRelation {
                relation: relation("a", "b")
            })
        );
    }

    #[test]
    fn an_element_with_relations_attached_cannot_be_removed() {
        let mut graph = ArchitectureGraph::new();
        graph.add_element(element("a")).unwrap();
        graph.add_element(element("b")).unwrap();
        graph.add_relation(relation("a", "b")).unwrap();
        assert_eq!(
            graph.remove_element(&ElementId::new("b").unwrap()),
            Err(GraphError::ElementInUse {
                id: ElementId::new("b").unwrap()
            })
        );
    }

    #[test]
    fn an_element_can_be_removed_after_its_relations_are_retracted() {
        let mut graph = ArchitectureGraph::new();
        graph.add_element(element("a")).unwrap();
        graph.add_element(element("b")).unwrap();
        graph.add_relation(relation("a", "b")).unwrap();
        graph.remove_relation(&relation("a", "b")).unwrap();
        let removed = graph.remove_element(&ElementId::new("b").unwrap()).unwrap();
        assert_eq!(removed, element("b"));
    }
}
