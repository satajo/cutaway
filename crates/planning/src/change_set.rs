//! The change set: an ordered list of proposed changes.
//!
//! Applying a change set yields the architecture as it would look after the
//! plan is enacted; the base graph stays untouched, so a change set can be
//! previewed, revised, and discarded freely.

use cutaway_architecture::{ArchitectureGraph, Element, ElementId, GraphError, Relation};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposedChange {
    AddElement(Element),
    /// Rejected while relations still point at the element; retract them
    /// with [`ProposedChange::RemoveRelation`] earlier in the change set.
    RemoveElement(ElementId),
    AddRelation(Relation),
    RemoveRelation(Relation),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSet {
    changes: Vec<ProposedChange>,
}

impl ChangeSet {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn propose(&mut self, change: ProposedChange) {
        self.changes.push(change);
    }

    #[must_use]
    pub fn changes(&self) -> &[ProposedChange] {
        &self.changes
    }

    /// Applies every change in order to a copy of `base`. The first change
    /// the graph rejects aborts the application and names its position, so
    /// the planner can point at the offending markup.
    pub fn apply_to(&self, base: &ArchitectureGraph) -> Result<ArchitectureGraph, ChangeSetError> {
        let mut graph = base.clone();
        for (index, change) in self.changes.iter().enumerate() {
            let applied = match change {
                ProposedChange::AddElement(element) => graph.add_element(element.clone()),
                ProposedChange::RemoveElement(id) => graph.remove_element(id).map(|_| ()),
                ProposedChange::AddRelation(relation) => graph.add_relation(relation.clone()),
                ProposedChange::RemoveRelation(relation) => graph.remove_relation(relation),
            };
            applied.map_err(|source| ChangeSetError::Rejected {
                change_index: index,
                source,
            })?;
        }
        Ok(graph)
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ChangeSetError {
    #[error("change {change_index} of the change set cannot be applied")]
    Rejected {
        change_index: usize,
        source: GraphError,
    },
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{ElementKind, ElementName, RelationKind};

    use super::*;

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
            kind: RelationKind::DependsOn,
        }
    }

    #[test]
    fn applying_a_change_set_leaves_the_base_architecture_untouched() {
        let mut base = ArchitectureGraph::new();
        base.add_element(element("a")).unwrap();

        let mut changes = ChangeSet::new();
        changes.propose(ProposedChange::AddElement(element("b")));
        let planned = changes.apply_to(&base).unwrap();

        assert_eq!(base.elements().count(), 1);
        assert_eq!(planned.elements().count(), 2);
    }

    #[test]
    fn removing_an_element_requires_retracting_its_relations_first() {
        let mut base = ArchitectureGraph::new();
        base.add_element(element("a")).unwrap();
        base.add_element(element("b")).unwrap();
        base.add_relation(relation("a", "b")).unwrap();

        let mut premature = ChangeSet::new();
        premature.propose(ProposedChange::RemoveElement(ElementId::new("b").unwrap()));
        assert!(premature.apply_to(&base).is_err());

        let mut complete = ChangeSet::new();
        complete.propose(ProposedChange::RemoveRelation(relation("a", "b")));
        complete.propose(ProposedChange::RemoveElement(ElementId::new("b").unwrap()));
        let planned = complete.apply_to(&base).unwrap();
        assert_eq!(planned.elements().count(), 1);
    }

    #[test]
    fn a_rejected_change_names_its_position_in_the_plan() {
        let base = ArchitectureGraph::new();

        let mut changes = ChangeSet::new();
        changes.propose(ProposedChange::AddElement(element("a")));
        changes.propose(ProposedChange::AddElement(element("a")));

        let error = changes.apply_to(&base).unwrap_err();
        assert!(matches!(
            error,
            ChangeSetError::Rejected {
                change_index: 1,
                ..
            }
        ));
    }
}
