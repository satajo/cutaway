use std::collections::BTreeSet;

use cutaway_architecture::{ArchitectureGraph, ElementId, Relation, RelationKind};

use crate::annotation::{Annotation, Note, Subject};
use crate::change_set::{ChangeSet, ProposedChange};
use crate::modification::Modification;

/// A complete markup of an architecture: the proposed changes, each with an
/// optional rationale, the modifications of elements that stay, and
/// annotations on parts that change not at all.
///
/// The plan is the artifact Cutaway exports for an agent to work from and
/// checks against after the work lands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    changes: Vec<PlannedChange>,
    annotations: Vec<Annotation>,
    /// At most one per subject. The modification API lives beside the type
    /// it stores, in [`crate::modification`].
    pub(crate) modifications: Vec<Modification>,
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
        self.changes.is_empty() && self.annotations.is_empty() && self.modifications.is_empty()
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

    /// The base architecture with this plan's additions drawn into it, for
    /// viewing: a lens rolls a planned element and a planned dependency up
    /// exactly as it rolls the real ones, so what the plan adds stands in
    /// the picture at every cut, the way what the sources declare does.
    ///
    /// The planned elements enter first, so the containment that puts them
    /// where they belong finds them. An addition the graph rejects - an
    /// endpoint the base does not hold, an id it already carries - stays out
    /// of the picture without failing it: the plan may run ahead of the
    /// architecture, and a picture must still stand. A containment naming an
    /// element the base already holds a parent for stays out for the same
    /// reason: a second parent makes "the boundary that holds this" have two
    /// answers, and no lens can draw that.
    ///
    /// Removals stay out: a severed dependency and an element planned for
    /// removal both still exist, and the picture marks them instead of
    /// hiding them.
    #[must_use]
    pub fn viewed_architecture(&self, base: &ArchitectureGraph) -> ArchitectureGraph {
        let mut graph = base.clone();
        for planned in &self.changes {
            if let ProposedChange::AddElement(element) = &planned.change {
                let _ = graph.add_element(element.clone());
            }
        }
        let mut held: BTreeSet<ElementId> = graph
            .relations()
            .filter(|relation| relation.kind == RelationKind::Contains)
            .map(|relation| relation.to.clone())
            .collect();
        for planned in &self.changes {
            let ProposedChange::AddRelation(relation) = &planned.change else {
                continue;
            };
            if relation.kind == RelationKind::Contains && !held.insert(relation.to.clone()) {
                continue;
            }
            let _ = graph.add_relation(relation.clone());
        }
        graph
    }

    /// How this plan stands toward a group of concrete relations that render
    /// as one connection.
    ///
    /// Planned additions and the rest never mix in one answer: a group that
    /// is entirely planned additions reads as [`GroupStanding::Added`], and
    /// otherwise the additions stand aside - they are not part of the
    /// architecture yet - while the real relations decide. The empty group
    /// is [`GroupStanding::Untouched`]: with nothing concrete behind a
    /// connection, the plan holds no stance on it.
    #[must_use]
    pub fn standing_of<'a>(
        &self,
        concrete: impl IntoIterator<Item = &'a Relation>,
    ) -> GroupStanding {
        let (mut added, mut real, mut removed) = (0, 0, 0);
        for relation in concrete {
            if self.plans_addition_of(relation) {
                added += 1;
            } else {
                real += 1;
                if self.plans_removal_of(relation) {
                    removed += 1;
                }
            }
        }
        if real == 0 {
            return if added > 0 {
                GroupStanding::Added
            } else {
                GroupStanding::Untouched
            };
        }
        match removed {
            0 => GroupStanding::Untouched,
            all if all == real => GroupStanding::Removed,
            some => GroupStanding::PartlyRemoved {
                removed: some,
                of: real,
            },
        }
    }
}

/// What a plan does to a group of concrete relations rendered as one
/// connection. See [`Plan::standing_of`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupStanding {
    /// Nothing in the group is marked; the connection stands as it is.
    Untouched,
    /// Every relation in the group is a planned addition: the connection
    /// exists only in the plan.
    Added,
    /// Every real relation in the group is planned for removal.
    Removed,
    /// Some but not all real relations are planned for removal.
    PartlyRemoved { removed: usize, of: usize },
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

    fn graph_of(elements: &[&str], relations: &[(&str, &str)]) -> ArchitectureGraph {
        use cutaway_architecture::{Element, ElementKind, ElementName};
        let mut graph = ArchitectureGraph::new();
        for id in elements {
            graph
                .add_element(Element {
                    id: ElementId::new(*id).unwrap(),
                    name: ElementName::new(*id).unwrap(),
                    kind: ElementKind::Module,
                })
                .unwrap();
        }
        for (from, to) in relations {
            graph.add_relation(relation(from, to)).unwrap();
        }
        graph
    }

    #[test]
    fn planned_dependency_additions_join_the_graph_a_lens_views() {
        let base = graph_of(&["a", "b"], &[]);
        let mut plan = Plan::new();
        plan.propose(ProposedChange::AddRelation(relation("a", "b")))
            .unwrap();

        let viewed = plan.viewed_architecture(&base);
        assert!(viewed.relations().any(|r| *r == relation("a", "b")));
        assert!(
            base.relations().next().is_none(),
            "the base architecture stays untouched"
        );
    }

    #[test]
    fn a_planned_element_joins_the_graph_a_lens_views_with_its_containment() {
        use cutaway_architecture::{Element, ElementKind, ElementName};
        let base = graph_of(&["a"], &[]);
        let mut plan = Plan::new();
        plan.propose(ProposedChange::AddElement(Element {
            id: ElementId::new("a/new").unwrap(),
            name: ElementName::new("new").unwrap(),
            kind: ElementKind::Module,
        }))
        .unwrap();
        plan.propose(ProposedChange::AddRelation(Relation {
            from: ElementId::new("a").unwrap(),
            to: ElementId::new("a/new").unwrap(),
            kind: RelationKind::Contains,
        }))
        .unwrap();

        let viewed = plan.viewed_architecture(&base);
        assert!(viewed.element(&ElementId::new("a/new").unwrap()).is_some());
        assert_eq!(
            viewed
                .relations()
                .filter(|r| r.kind == RelationKind::Contains)
                .count(),
            1,
            "the element enters before the containment that names it"
        );
    }

    #[test]
    fn an_addition_the_graph_cannot_hold_yet_stays_out_of_the_viewed_graph() {
        let base = graph_of(&["a"], &[]);
        let mut plan = Plan::new();
        plan.propose(ProposedChange::AddRelation(relation("a", "missing")))
            .unwrap();
        assert_eq!(plan.viewed_architecture(&base), base);
    }

    #[test]
    fn a_planned_containment_of_an_element_that_already_has_one_stays_out() {
        let mut base = graph_of(&["a", "b", "c"], &[]);
        base.add_relation(Relation {
            from: ElementId::new("a").unwrap(),
            to: ElementId::new("b").unwrap(),
            kind: RelationKind::Contains,
        })
        .unwrap();
        let mut plan = Plan::new();
        plan.propose(ProposedChange::AddRelation(Relation {
            from: ElementId::new("c").unwrap(),
            to: ElementId::new("b").unwrap(),
            kind: RelationKind::Contains,
        }))
        .unwrap();
        assert_eq!(
            plan.viewed_architecture(&base),
            base,
            "one boundary holds an element, and a second answer draws no picture"
        );
    }

    #[test]
    fn a_planned_removal_keeps_its_relation_in_the_viewed_graph() {
        let base = graph_of(&["a", "b"], &[("a", "b")]);
        let mut plan = Plan::new();
        plan.propose(ProposedChange::RemoveRelation(relation("a", "b")))
            .unwrap();
        assert_eq!(
            plan.viewed_architecture(&base),
            base,
            "a severed dependency still exists; the picture marks it instead of hiding it"
        );
    }

    #[test]
    fn an_element_planned_for_removal_stays_in_the_viewed_graph() {
        let base = graph_of(&["a", "b"], &[("a", "b")]);
        let mut plan = Plan::new();
        plan.propose(ProposedChange::RemoveElement(ElementId::new("b").unwrap()))
            .unwrap();
        assert_eq!(
            plan.viewed_architecture(&base),
            base,
            "the picture marks what is going instead of hiding it"
        );
    }

    #[test]
    fn a_group_the_plan_never_mentions_is_untouched() {
        assert_eq!(
            Plan::new().standing_of([relation("a", "b")].iter()),
            GroupStanding::Untouched
        );
        assert_eq!(
            Plan::new().standing_of(std::iter::empty::<&Relation>()),
            GroupStanding::Untouched
        );
    }

    #[test]
    fn a_group_with_every_relation_planned_for_removal_reads_as_removed() {
        let mut plan = Plan::new();
        plan.propose(ProposedChange::RemoveRelation(relation("a", "b")))
            .unwrap();
        plan.propose(ProposedChange::RemoveRelation(relation("a", "c")))
            .unwrap();
        assert_eq!(
            plan.standing_of([relation("a", "b"), relation("a", "c")].iter()),
            GroupStanding::Removed
        );
    }

    #[test]
    fn a_group_with_part_of_its_relations_planned_for_removal_reads_as_partly_removed() {
        let mut plan = Plan::new();
        plan.propose(ProposedChange::RemoveRelation(relation("a", "b")))
            .unwrap();
        assert_eq!(
            plan.standing_of([relation("a", "b"), relation("a", "c")].iter()),
            GroupStanding::PartlyRemoved { removed: 1, of: 2 }
        );
    }

    #[test]
    fn a_group_of_planned_additions_alone_reads_as_added() {
        let mut plan = Plan::new();
        plan.propose(ProposedChange::AddRelation(relation("a", "b")))
            .unwrap();
        assert_eq!(
            plan.standing_of([relation("a", "b")].iter()),
            GroupStanding::Added
        );
    }

    #[test]
    fn an_addition_folded_among_real_relations_leaves_the_real_ones_deciding() {
        let mut plan = Plan::new();
        plan.propose(ProposedChange::AddRelation(relation("a", "b")))
            .unwrap();
        plan.propose(ProposedChange::RemoveRelation(relation("a", "c")))
            .unwrap();
        assert_eq!(
            plan.standing_of([relation("a", "b"), relation("a", "c")].iter()),
            GroupStanding::Removed,
            "the planned addition stands aside; the one real relation is removed"
        );
    }
}
