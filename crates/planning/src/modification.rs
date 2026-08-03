//! Modifying an element that stays: renaming it, splitting it into several,
//! merging it into another, or reworking what is inside it.
//!
//! # A modification is an intent, not an edit of the graph
//!
//! A removal and an addition change what the architecture holds, so they
//! enter the change set and the graph a lens views. A modification does not.
//! A rename states the name the element should carry once the work lands, a
//! split states what it should become, a merge states where it should go,
//! and a rework states that its insides change while its place does not.
//! None of that can be drawn as a graph without inventing elements and
//! dependencies nobody has written yet: a merge in particular redraws no
//! dependency, because which couplings survive the fold is the work itself.
//! The plan therefore carries modifications beside its changes, as sentences
//! for whoever implements it, and the picture marks the element instead of
//! redrawing around it.
//!
//! # One modification per element
//!
//! An element renamed and split at once states two futures for one thing.
//! The later answer stands: [`Plan::plan_modification`] replaces whatever
//! the subject carried before, exactly as [`Plan::annotate`] does.
//!
//! # Only an element the architecture holds is modifiable
//!
//! A modification talks about code that exists. An element that lives only
//! in the plan already carries whatever name, kind and parent the planner
//! gave it, so a reader who wants it different edits the addition instead of
//! stating a change to it. [`Plan::plan_modification`] reads no graph and
//! cannot check this itself: the surfaces that offer the act - the shell and
//! the e2e driver - refuse a subject the architecture does not hold, and
//! [`Plan::normalized`] drops such an entry from a loaded plan.

use cutaway_architecture::{ElementId, ElementName};

use crate::annotation::Note;
use crate::plan::Plan;

/// How one element of the architecture changes while staying where it is,
/// and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Modification {
    pub subject: ElementId,
    pub kind: ModificationKind,
    pub note: Option<Note>,
}

/// What kind of change an element is in for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModificationKind {
    /// The element keeps everything but its name.
    Rename { to: ElementName },
    /// The element becomes several, named here.
    Split { into: SplitParts },
    /// The element folds into another that already exists.
    Merge { with: ElementId },
    /// The insides change and the shape of the picture does not. A rework
    /// carries no payload of its own: the note on the modification is the
    /// description of the work.
    Rework,
}

/// What an element becomes when it splits: two names at least. One name is
/// a rename, and none at all is a removal; a split is neither.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitParts(Vec<ElementName>);

impl SplitParts {
    pub fn new(names: Vec<ElementName>) -> Result<Self, InvalidSplit> {
        if names.len() < 2 {
            return Err(InvalidSplit::TooFew { named: names.len() });
        }
        Ok(Self(names))
    }

    #[must_use]
    pub fn names(&self) -> &[ElementName] {
        &self.0
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum InvalidSplit {
    #[error("a split names at least two elements to become, not {named}")]
    TooFew { named: usize },
}

impl Plan {
    /// States how one element changes. A subject that already carries a
    /// modification takes this one instead: one element, one future.
    pub fn plan_modification(&mut self, modification: Modification) {
        self.discard_modification(&modification.subject);
        self.modifications.push(modification);
    }

    pub fn discard_modification(&mut self, subject: &ElementId) {
        self.modifications
            .retain(|modification| &modification.subject != subject);
    }

    #[must_use]
    pub fn modification_of(&self, subject: &ElementId) -> Option<&Modification> {
        self.modifications
            .iter()
            .find(|modification| &modification.subject == subject)
    }

    pub fn modifications(&self) -> impl Iterator<Item = &Modification> {
        self.modifications.iter()
    }
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{ArchitectureGraph, Element, ElementKind, Relation, RelationKind};

    use super::*;

    fn id(text: &str) -> ElementId {
        ElementId::new(text).unwrap()
    }

    fn name(text: &str) -> ElementName {
        ElementName::new(text).unwrap()
    }

    /// package:a depends on package:b, and nothing else.
    fn base() -> ArchitectureGraph {
        let mut graph = ArchitectureGraph::new();
        for element in ["package:a", "package:b"] {
            graph
                .add_element(Element {
                    id: id(element),
                    name: name(element),
                    kind: ElementKind::Package,
                })
                .unwrap();
        }
        graph
            .add_relation(Relation {
                from: id("package:a"),
                to: id("package:b"),
                kind: RelationKind::DependsOn,
            })
            .unwrap();
        graph
    }

    fn modification(subject: &str, kind: ModificationKind) -> Modification {
        Modification {
            subject: id(subject),
            kind,
            note: None,
        }
    }

    #[test]
    fn a_split_names_at_least_two_elements_to_become() {
        assert_eq!(
            SplitParts::new(vec![name("engine")]),
            Err(InvalidSplit::TooFew { named: 1 })
        );
        assert_eq!(
            SplitParts::new(Vec::new()),
            Err(InvalidSplit::TooFew { named: 0 })
        );
        assert!(SplitParts::new(vec![name("engine"), name("transport")]).is_ok());
    }

    #[test]
    fn planning_a_second_modification_of_one_element_replaces_the_first() {
        let mut plan = Plan::new();
        plan.plan_modification(modification(
            "package:a",
            ModificationKind::Rename { to: name("engine") },
        ));
        plan.plan_modification(modification("package:a", ModificationKind::Rework));

        assert_eq!(
            plan.modification_of(&id("package:a")).map(|it| &it.kind),
            Some(&ModificationKind::Rework)
        );
        assert_eq!(plan.modifications().count(), 1);
    }

    #[test]
    fn discarding_a_modification_leaves_the_element_unmarked() {
        let mut plan = Plan::new();
        plan.plan_modification(modification("package:a", ModificationKind::Rework));
        plan.discard_modification(&id("package:a"));

        assert_eq!(plan.modification_of(&id("package:a")), None);
        assert!(plan.is_empty());
    }

    #[test]
    fn a_modification_states_intent_without_touching_the_architecture() {
        let base = base();
        let mut plan = Plan::new();
        plan.plan_modification(modification(
            "package:a",
            ModificationKind::Split {
                into: SplitParts::new(vec![name("engine"), name("transport")]).unwrap(),
            },
        ));

        assert_eq!(plan.viewed_architecture(&base), base);
        assert!(plan.change_set().changes().is_empty());
    }

    #[test]
    fn a_merge_names_where_the_element_goes_and_redraws_no_dependency() {
        let base = base();
        let mut plan = Plan::new();
        plan.plan_modification(modification(
            "package:a",
            ModificationKind::Merge {
                with: id("package:b"),
            },
        ));

        assert_eq!(
            plan.modification_of(&id("package:a")).map(|it| &it.kind),
            Some(&ModificationKind::Merge {
                with: id("package:b")
            })
        );
        assert_eq!(
            plan.viewed_architecture(&base),
            base,
            "which couplings survive the fold is the work, not the plan's to draw"
        );
    }
}
