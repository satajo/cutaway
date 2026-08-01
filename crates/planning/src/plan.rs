use cutaway_architecture::Relation;

use crate::annotation::{Annotation, Note, Subject};
use crate::change_set::{ChangeSet, ProposedChange};

/// A complete markup of an architecture: the proposed changes, each with an
/// optional rationale, plus annotations on parts that stay as they are.
///
/// The plan is the artifact Cutaway exports for an agent to work from and
/// checks against after the work lands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    changes: Vec<PlannedChange>,
    annotations: Vec<Annotation>,
}

/// One proposed change and why it is proposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedChange {
    pub change: ProposedChange,
    pub note: Option<Note>,
}

impl Plan {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.annotations.is_empty()
    }

    #[must_use]
    pub fn changes(&self) -> &[PlannedChange] {
        &self.changes
    }

    #[must_use]
    pub fn annotations(&self) -> &[Annotation] {
        &self.annotations
    }

    pub fn propose(&mut self, change: ProposedChange) -> Result<(), PlanError> {
        if self.changes.iter().any(|planned| planned.change == change) {
            return Err(PlanError::AlreadyPlanned { change });
        }
        self.changes.push(PlannedChange { change, note: None });
        Ok(())
    }

    /// Withdraws a proposed change, note included.
    pub fn retract(&mut self, change: &ProposedChange) -> Result<(), PlanError> {
        let index = self
            .changes
            .iter()
            .position(|planned| &planned.change == change)
            .ok_or_else(|| PlanError::NotPlanned {
                change: change.clone(),
            })?;
        self.changes.remove(index);
        Ok(())
    }

    /// Sets or clears the rationale of an already proposed change.
    pub fn explain(
        &mut self,
        change: &ProposedChange,
        note: Option<Note>,
    ) -> Result<(), PlanError> {
        let planned = self
            .changes
            .iter_mut()
            .find(|planned| &planned.change == change)
            .ok_or_else(|| PlanError::NotPlanned {
                change: change.clone(),
            })?;
        planned.note = note;
        Ok(())
    }

    /// Sets or replaces the annotation of a subject.
    pub fn annotate(&mut self, subject: Subject, note: Note) {
        self.clear_annotation(&subject);
        self.annotations.push(Annotation { subject, note });
    }

    pub fn clear_annotation(&mut self, subject: &Subject) {
        self.annotations
            .retain(|annotation| &annotation.subject != subject);
    }

    #[must_use]
    pub fn annotation_of(&self, subject: &Subject) -> Option<&Note> {
        self.annotations
            .iter()
            .find(|annotation| &annotation.subject == subject)
            .map(|annotation| &annotation.note)
    }

    #[must_use]
    pub fn note_of(&self, change: &ProposedChange) -> Option<&Note> {
        self.changes
            .iter()
            .find(|planned| &planned.change == change)
            .and_then(|planned| planned.note.as_ref())
    }

    #[must_use]
    pub fn plans_removal_of(&self, relation: &Relation) -> bool {
        self.changes.iter().any(|planned| {
            matches!(&planned.change, ProposedChange::RemoveRelation(removed) if removed == relation)
        })
    }

    #[must_use]
    pub fn plans_addition_of(&self, relation: &Relation) -> bool {
        self.changes.iter().any(|planned| {
            matches!(&planned.change, ProposedChange::AddRelation(added) if added == relation)
        })
    }

    /// The plan's changes as a bare change set, ready to apply to a graph.
    #[must_use]
    pub fn change_set(&self) -> ChangeSet {
        let mut changes = ChangeSet::new();
        for planned in &self.changes {
            changes.propose(planned.change.clone());
        }
        changes
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("the change is already in the plan")]
    AlreadyPlanned { change: ProposedChange },
    #[error("the change is not in the plan")]
    NotPlanned { change: ProposedChange },
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{ElementId, RelationKind};

    use super::*;

    fn relation(from: &str, to: &str) -> Relation {
        Relation {
            from: ElementId::new(from).unwrap(),
            to: ElementId::new(to).unwrap(),
            kind: RelationKind::DependsOn,
        }
    }

    #[test]
    fn a_change_can_be_planned_only_once() {
        let mut plan = Plan::new();
        let change = ProposedChange::RemoveRelation(relation("a", "b"));
        plan.propose(change.clone()).unwrap();
        assert_eq!(
            plan.propose(change.clone()),
            Err(PlanError::AlreadyPlanned { change })
        );
    }

    #[test]
    fn retracting_a_change_also_drops_its_note() {
        let mut plan = Plan::new();
        let change = ProposedChange::RemoveRelation(relation("a", "b"));
        plan.propose(change.clone()).unwrap();
        plan.explain(&change, Some(Note::new("cut this").unwrap()))
            .unwrap();
        plan.retract(&change).unwrap();
        plan.propose(change.clone()).unwrap();
        assert_eq!(plan.note_of(&change), None);
    }

    #[test]
    fn annotating_a_subject_twice_replaces_the_note() {
        let mut plan = Plan::new();
        let subject = Subject::Relation(relation("a", "b"));
        plan.annotate(subject.clone(), Note::new("first").unwrap());
        plan.annotate(subject.clone(), Note::new("second").unwrap());
        assert_eq!(
            plan.annotation_of(&subject),
            Some(&Note::new("second").unwrap())
        );
        assert_eq!(plan.annotations().len(), 1);
    }

    #[test]
    fn the_change_set_carries_the_changes_in_planning_order() {
        let mut plan = Plan::new();
        plan.propose(ProposedChange::RemoveRelation(relation("a", "b")))
            .unwrap();
        plan.propose(ProposedChange::AddRelation(relation("a", "c")))
            .unwrap();
        assert_eq!(plan.change_set().changes().len(), 2);
    }
}
