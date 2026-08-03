//! Re-anchoring a loaded plan to the concrete relations of its architecture.
//!
//! A plan anchors to concrete source-level relations: they hold whatever
//! detail a picture is cut at, so a markup made on one view stays visible on
//! every other. Earlier versions of the shell instead recorded a connection
//! exactly as the picture drew it - a rolled-up boundary pair, or an
//! endpoint naming the synthetic own-content leaf a boundary lens grows
//! (an id ending in `#self`). Those names exist in no source graph, so a
//! plan carrying them silently loses its marks the moment the cut changes,
//! and hands an agent ids it can never resolve.
//!
//! [`Plan::normalized`] is where that legacy dies. It is deterministic, and
//! its rules are:
//!
//! - Every id loses a trailing `#self`: the own-content leaf stands for its
//!   frame, so the frame is what the entry meant.
//! - A relation removal the base graph carries verbatim stays as it is.
//! - Any other dependency removal is a boundary-level entry: it expands to
//!   every concrete `DependsOn` relation of the base graph whose endpoints
//!   lie within (containment-wise, the boundary itself included - which
//!   also covers a manifest dependency naming the boundary verbatim) the
//!   stored endpoints. An entry that expands to nothing is stale and drops.
//! - Where two entries expand to the same concrete removal, the first
//!   occurrence stands, note included: the same removal stated twice is one
//!   removal.
//! - A note on an expanded entry lands on every concrete relation it
//!   expands to: the note explains the severing of the whole connection,
//!   and each concrete removal is a part of that severing.
//! - Relation additions only lose `#self` marks: a drawn dependency already
//!   names the elements the planner picked, and it may run ahead of the
//!   base graph on purpose.
//! - An element removal naming neither an element of the base graph nor an
//!   element this plan adds is stale and drops: the sources no longer hold
//!   what it asks to remove, so the entry states no work.
//! - Element additions always survive: an addition names an element that
//!   exists nowhere but the plan, which is the point of it.
//! - Relation-subject annotations expand exactly as removals do, the note
//!   landing on every concrete relation; element subjects only lose `#self`
//!   marks. Where expansions overlap, the later annotation replaces the
//!   earlier on the shared subjects, as [`Plan::annotate`] always has.
//! - A modification loses the `#self` marks of its subject and of the
//!   element a merge folds into. One whose subject the base graph does not
//!   hold drops: a modification talks about code that exists, and an element
//!   the plan merely proposes is edited as an addition instead of modified.

use std::collections::BTreeMap;

use cutaway_architecture::{ArchitectureGraph, ElementId, Relation, RelationKind};

use crate::annotation::Subject;
use crate::change_set::ProposedChange;
use crate::containment::{containment_parents, lies_within};
use crate::modification::{Modification, ModificationKind};
use crate::plan::Plan;

/// How older plan files spelled the boundary lens's own-content leaf. No
/// source element ever ends this way - an item id ends in `#<kind>:<name>` -
/// so stripping the suffix is unambiguous.
const LEGACY_SELF_LEAF_SUFFIX: &str = "#self";

impl Plan {
    /// This plan re-anchored to the concrete relations of `base`, by the
    /// rules of [`crate::normalize`]. Apply it to every plan loaded against
    /// a known architecture, before anything reads or extends it.
    #[must_use]
    pub fn normalized(&self, base: &ArchitectureGraph) -> Plan {
        let parents = containment_parents(base);
        let mut normalized = Plan::new();
        for planned in self.changes() {
            for change in concrete_changes(&planned.change, self, base, &parents) {
                if normalized.propose(change.clone()).is_err() {
                    // The first occurrence of a change stands, note included.
                    continue;
                }
                if planned.note.is_some() {
                    normalized
                        .explain(&change, planned.note.clone())
                        .expect("the change was proposed above");
                }
            }
        }
        for annotation in self.annotations() {
            match &annotation.subject {
                Subject::Element(id) => {
                    normalized.annotate(Subject::Element(stripped(id)), annotation.note.clone());
                }
                Subject::Relation(relation) => {
                    for concrete in concrete_forms(relation, base, &parents) {
                        normalized.annotate(Subject::Relation(concrete), annotation.note.clone());
                    }
                }
            }
        }
        for modification in self.modifications() {
            let subject = stripped(&modification.subject);
            if base.element(&subject).is_none() {
                continue;
            }
            normalized.plan_modification(Modification {
                subject,
                kind: stripped_kind(&modification.kind),
                note: modification.note.clone(),
            });
        }
        normalized
    }
}

/// A modification's payload with its `#self` marks gone. Only a merge names
/// an element at all; the rest carry names of things that do not exist yet.
fn stripped_kind(kind: &ModificationKind) -> ModificationKind {
    match kind {
        ModificationKind::Merge { with } => ModificationKind::Merge {
            with: stripped(with),
        },
        other => other.clone(),
    }
}

/// The concrete changes one stored change stands for. A dependency removal
/// expands, an element removal the architecture no longer answers for
/// drops, and everything else merely loses `#self` marks.
fn concrete_changes(
    change: &ProposedChange,
    plan: &Plan,
    base: &ArchitectureGraph,
    parents: &BTreeMap<ElementId, ElementId>,
) -> Vec<ProposedChange> {
    match change {
        ProposedChange::AddElement(element) => {
            let mut element = element.clone();
            element.id = stripped(&element.id);
            vec![ProposedChange::AddElement(element)]
        }
        ProposedChange::RemoveElement(id) => {
            let id = stripped(id);
            // A removal answers for an element of the architecture, or for
            // one this plan adds and then takes back out; anything else
            // names something no longer there.
            if base.element(&id).is_some() || plan.plans_addition_of_element(&id) {
                vec![ProposedChange::RemoveElement(id)]
            } else {
                Vec::new()
            }
        }
        ProposedChange::AddRelation(relation) => {
            vec![ProposedChange::AddRelation(stripped_relation(relation))]
        }
        ProposedChange::RemoveRelation(relation) => concrete_forms(relation, base, parents)
            .into_iter()
            .map(ProposedChange::RemoveRelation)
            .collect(),
    }
}

/// The concrete relations of the base graph one stored relation stands for:
/// itself when the base carries it verbatim, and otherwise every concrete
/// `DependsOn` relation between what its endpoints contain. Empty for a
/// stale entry that covers nothing.
fn concrete_forms(
    relation: &Relation,
    base: &ArchitectureGraph,
    parents: &BTreeMap<ElementId, ElementId>,
) -> Vec<Relation> {
    let relation = stripped_relation(relation);
    if base.relations().any(|concrete| *concrete == relation) {
        return vec![relation];
    }
    if relation.kind != RelationKind::DependsOn {
        // Only dependencies roll up in a view, so only they ever reached a
        // plan under a boundary-level name.
        return Vec::new();
    }
    base.relations()
        .filter(|concrete| {
            concrete.kind == RelationKind::DependsOn
                && lies_within(&concrete.from, &relation.from, parents)
                && lies_within(&concrete.to, &relation.to, parents)
        })
        .cloned()
        .collect()
}

fn stripped(id: &ElementId) -> ElementId {
    id.as_str()
        .strip_suffix(LEGACY_SELF_LEAF_SUFFIX)
        .and_then(|frame| ElementId::new(frame).ok())
        .unwrap_or_else(|| id.clone())
}

fn stripped_relation(relation: &Relation) -> Relation {
    Relation {
        from: stripped(&relation.from),
        to: stripped(&relation.to),
        kind: relation.kind,
    }
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{Element, ElementKind, ElementName};

    use crate::Note;

    use super::*;

    fn id(text: &str) -> ElementId {
        ElementId::new(text).unwrap()
    }

    fn depends(from: &str, to: &str) -> Relation {
        Relation {
            from: id(from),
            to: id(to),
            kind: RelationKind::DependsOn,
        }
    }

    fn note(text: &str) -> Note {
        Note::new(text).unwrap()
    }

    /// package:a ⊃ {a/one, a/two}, package:b ⊃ b/lib ⊃ b/lib#function:go.
    /// Concrete dependencies: a/one -> b/lib, a/two -> b/lib#function:go,
    /// and the manifest-style package:a -> package:b.
    fn base() -> ArchitectureGraph {
        let mut graph = ArchitectureGraph::new();
        for (element, kind) in [
            ("package:a", ElementKind::Package),
            ("package:b", ElementKind::Package),
            ("a/one", ElementKind::Module),
            ("a/two", ElementKind::Module),
            ("b/lib", ElementKind::Module),
            ("b/lib#function:go", ElementKind::Function),
        ] {
            graph
                .add_element(Element {
                    id: id(element),
                    name: ElementName::new(element).unwrap(),
                    kind,
                })
                .unwrap();
        }
        for (from, to) in [
            ("package:a", "a/one"),
            ("package:a", "a/two"),
            ("package:b", "b/lib"),
            ("b/lib", "b/lib#function:go"),
        ] {
            graph
                .add_relation(Relation {
                    from: id(from),
                    to: id(to),
                    kind: RelationKind::Contains,
                })
                .unwrap();
        }
        for (from, to) in [
            ("a/one", "b/lib"),
            ("a/two", "b/lib#function:go"),
            ("package:a", "package:b"),
        ] {
            graph.add_relation(depends(from, to)).unwrap();
        }
        graph
    }

    fn removals(plan: &Plan) -> Vec<Relation> {
        plan.changes()
            .iter()
            .filter_map(|planned| match &planned.change {
                ProposedChange::RemoveRelation(relation) => Some(relation.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_removal_of_a_concrete_relation_survives_normalization_unchanged() {
        let mut plan = Plan::new();
        plan.propose(ProposedChange::RemoveRelation(depends("a/one", "b/lib")))
            .unwrap();
        assert_eq!(plan.normalized(&base()), plan);
    }

    #[test]
    fn a_boundary_level_removal_expands_to_the_concrete_dependencies_it_covers() {
        let mut base = base();
        // Without the manifest dependency the boundary pair names nothing
        // concrete, so it must expand instead of standing verbatim.
        base.remove_relation(&depends("package:a", "package:b"))
            .unwrap();
        let mut plan = Plan::new();
        plan.propose(ProposedChange::RemoveRelation(depends(
            "package:a",
            "package:b",
        )))
        .unwrap();

        assert_eq!(
            removals(&plan.normalized(&base)),
            vec![
                depends("a/one", "b/lib"),
                depends("a/two", "b/lib#function:go"),
            ]
        );
    }

    #[test]
    fn a_manifest_dependency_naming_the_boundary_verbatim_stays_itself() {
        let mut plan = Plan::new();
        plan.propose(ProposedChange::RemoveRelation(depends(
            "package:a",
            "package:b",
        )))
        .unwrap();
        assert_eq!(
            removals(&plan.normalized(&base())),
            vec![depends("package:a", "package:b")],
            "the base graph carries the pair verbatim, so it is already concrete"
        );
    }

    #[test]
    fn a_self_leaf_endpoint_is_stripped_to_the_frame_it_stands_for() {
        let mut plan = Plan::new();
        plan.propose(ProposedChange::RemoveRelation(depends(
            "a/one",
            "b/lib#self",
        )))
        .unwrap();
        plan.propose(ProposedChange::AddRelation(depends(
            "package:a#self",
            "b/lib",
        )))
        .unwrap();

        let normalized = plan.normalized(&base());
        assert_eq!(removals(&normalized), vec![depends("a/one", "b/lib")]);
        assert!(normalized.plans_addition_of(&depends("package:a", "b/lib")));
    }

    #[test]
    fn a_removal_that_covers_nothing_concrete_is_dropped_as_stale() {
        let mut plan = Plan::new();
        plan.propose(ProposedChange::RemoveRelation(depends(
            "package:b",
            "package:a",
        )))
        .unwrap();
        assert!(plan.normalized(&base()).is_empty());
    }

    #[test]
    fn a_note_on_an_expanded_removal_lands_on_every_concrete_relation() {
        let mut base = base();
        base.remove_relation(&depends("package:a", "package:b"))
            .unwrap();
        let boundary = ProposedChange::RemoveRelation(depends("package:a", "package:b"));
        let mut plan = Plan::new();
        plan.propose(boundary.clone()).unwrap();
        plan.explain(&boundary, Some(note("cut the coupling")))
            .unwrap();

        let normalized = plan.normalized(&base);
        for concrete in removals(&normalized) {
            assert_eq!(
                normalized.note_of(&ProposedChange::RemoveRelation(concrete)),
                Some(&note("cut the coupling"))
            );
        }
    }

    #[test]
    fn overlapping_removals_collapse_into_the_first_occurrence() {
        let mut base = base();
        base.remove_relation(&depends("package:a", "package:b"))
            .unwrap();
        let concrete = ProposedChange::RemoveRelation(depends("a/one", "b/lib"));
        let mut plan = Plan::new();
        plan.propose(concrete.clone()).unwrap();
        plan.explain(&concrete, Some(note("first"))).unwrap();
        let boundary = ProposedChange::RemoveRelation(depends("package:a", "package:b"));
        plan.propose(boundary.clone()).unwrap();
        plan.explain(&boundary, Some(note("second"))).unwrap();

        let normalized = plan.normalized(&base);
        assert_eq!(
            removals(&normalized),
            vec![
                depends("a/one", "b/lib"),
                depends("a/two", "b/lib#function:go"),
            ]
        );
        assert_eq!(
            normalized.note_of(&concrete),
            Some(&note("first")),
            "the first occurrence keeps its note"
        );
    }

    #[test]
    fn a_relation_annotation_on_a_boundary_pair_lands_on_the_concrete_dependencies() {
        let mut base = base();
        base.remove_relation(&depends("package:a", "package:b"))
            .unwrap();
        let mut plan = Plan::new();
        plan.annotate(
            Subject::Relation(depends("package:a", "package:b")),
            note("watch this seam"),
        );

        let normalized = plan.normalized(&base);
        for concrete in [
            depends("a/one", "b/lib"),
            depends("a/two", "b/lib#function:go"),
        ] {
            assert_eq!(
                normalized.annotation_of(&Subject::Relation(concrete)),
                Some(&note("watch this seam"))
            );
        }
    }

    #[test]
    fn a_stale_relation_annotation_is_dropped() {
        let mut plan = Plan::new();
        plan.annotate(
            Subject::Relation(depends("package:b", "package:a")),
            note("nothing is behind this"),
        );
        assert!(plan.normalized(&base()).is_empty());
    }

    fn renaming(subject: &str, to: &str) -> Modification {
        Modification {
            subject: id(subject),
            kind: ModificationKind::Rename {
                to: ElementName::new(to).unwrap(),
            },
            note: None,
        }
    }

    #[test]
    fn a_modification_of_an_element_of_the_architecture_survives_normalization() {
        let mut plan = Plan::new();
        plan.plan_modification(renaming("a/one", "first"));
        assert_eq!(plan.normalized(&base()), plan);
    }

    #[test]
    fn a_modification_of_an_element_the_architecture_no_longer_holds_is_dropped_as_stale() {
        let mut plan = Plan::new();
        plan.plan_modification(renaming("a/gone", "first"));
        assert!(plan.normalized(&base()).is_empty());
    }

    #[test]
    fn a_modification_written_on_a_self_leaf_speaks_about_the_frame() {
        let mut plan = Plan::new();
        plan.plan_modification(Modification {
            subject: id("b/lib#self"),
            kind: ModificationKind::Merge {
                with: id("a/one#self"),
            },
            note: None,
        });

        assert_eq!(
            plan.normalized(&base()).modification_of(&id("b/lib")),
            Some(&Modification {
                subject: id("b/lib"),
                kind: ModificationKind::Merge { with: id("a/one") },
                note: None,
            })
        );
    }

    #[test]
    fn a_normalized_plan_contains_no_self_leaf_id() {
        let mut plan = Plan::new();
        plan.propose(ProposedChange::RemoveRelation(depends(
            "a/one",
            "b/lib#self",
        )))
        .unwrap();
        plan.propose(ProposedChange::AddRelation(depends(
            "package:a#self",
            "b/lib",
        )))
        .unwrap();
        plan.annotate(Subject::Element(id("b/lib#self")), note("shrink this"));
        plan.annotate(
            Subject::Relation(depends("a/one", "b/lib#self")),
            note("a seam"),
        );
        plan.plan_modification(Modification {
            subject: id("a/two#self"),
            kind: ModificationKind::Merge {
                with: id("b/lib#self"),
            },
            note: None,
        });

        let normalized = plan.normalized(&base());
        let ids: Vec<ElementId> = normalized
            .changes()
            .iter()
            .flat_map(|planned| match &planned.change {
                ProposedChange::AddElement(element) => vec![element.id.clone()],
                ProposedChange::RemoveElement(id) => vec![id.clone()],
                ProposedChange::AddRelation(r) | ProposedChange::RemoveRelation(r) => {
                    vec![r.from.clone(), r.to.clone()]
                }
            })
            .chain(normalized.annotations().iter().flat_map(
                |annotation| match &annotation.subject {
                    Subject::Element(id) => vec![id.clone()],
                    Subject::Relation(r) => vec![r.from.clone(), r.to.clone()],
                },
            ))
            .chain(normalized.modifications().flat_map(|modification| {
                let mut ids = vec![modification.subject.clone()];
                if let ModificationKind::Merge { with } = &modification.kind {
                    ids.push(with.clone());
                }
                ids
            }))
            .collect();
        assert!(!ids.is_empty());
        for id in ids {
            assert!(
                !id.as_str().ends_with(LEGACY_SELF_LEAF_SUFFIX),
                "{id} slipped through"
            );
        }
    }

    #[test]
    fn an_element_removal_the_architecture_no_longer_holds_is_dropped_as_stale() {
        let mut plan = Plan::new();
        plan.propose(ProposedChange::RemoveElement(id("a/gone")))
            .unwrap();
        assert!(plan.normalized(&base()).is_empty());
    }

    #[test]
    fn an_element_removal_naming_an_element_of_the_architecture_survives() {
        let mut plan = Plan::new();
        plan.propose(ProposedChange::RemoveElement(id("a/one")))
            .unwrap();
        assert_eq!(plan.normalized(&base()), plan);
    }

    #[test]
    fn a_planned_element_can_be_planned_away_again() {
        let planned = Element {
            id: id("package:a/new"),
            name: ElementName::new("new").unwrap(),
            kind: ElementKind::Module,
        };
        let mut plan = Plan::new();
        plan.propose(ProposedChange::AddElement(planned)).unwrap();
        plan.propose(ProposedChange::RemoveElement(id("package:a/new")))
            .unwrap();
        assert_eq!(
            plan.normalized(&base()),
            plan,
            "an addition runs ahead of the architecture, and so does a removal that answers it"
        );
    }

    #[test]
    fn normalization_is_idempotent() {
        let mut base = base();
        base.remove_relation(&depends("package:a", "package:b"))
            .unwrap();
        let mut plan = Plan::new();
        plan.propose(ProposedChange::RemoveRelation(depends(
            "package:a",
            "package:b",
        )))
        .unwrap();
        plan.annotate(Subject::Relation(depends("a/one", "b/lib")), note("a seam"));
        plan.plan_modification(renaming("a/one#self", "first"));

        let once = plan.normalized(&base);
        assert_eq!(once.normalized(&base), once);
    }
}
