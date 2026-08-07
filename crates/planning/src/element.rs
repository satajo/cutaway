//! Planning whole elements: what a plan states to remove one, what it
//! states to add one, and the id an element carries while it exists only in
//! the plan.
//!
//! # Removal takes the subtree
//!
//! A removal takes the containment subtree of its element with it, so one
//! entry on the root of a subtree is the whole intent and nothing inside it
//! needs an entry of its own. What does need stating is every dependency
//! that crosses the border of that subtree: the element cannot leave while
//! something outside it still reaches in, or while it still reaches out.
//! The couplings interior to the subtree go with it unstated, and so does
//! the containment - the one inside the subtree and the one that puts the
//! element where it sits alike. Where a thing sits is the architecture's
//! answer; the plan states the couplings that must be undone by hand.
//!
//! # An addition runs ahead of the sources
//!
//! A planned element exists in no source tree yet, so it carries a
//! provisional id derived from its parent, its kind and its name, in the
//! shape a real graph would give it. Whoever implements the plan realizes
//! the element at a real source path, and the next inspection replaces the
//! provisional id with the real one.

use std::collections::BTreeSet;

use cutaway_architecture::{
    ArchitectureGraph, Element, ElementId, ElementKind, ElementName, Relation, RelationKind,
};

use crate::change_set::ProposedChange;
use crate::containment::{containment_parents, lies_within};
use crate::plan::Plan;

/// The id a planned element carries until the sources give it a real one.
///
/// The shape follows the ids a producer derives from the sources: a package
/// is named by itself, a directory and a module by the path of the boundary
/// that holds it, an item by the module it is declared in and its kind. A
/// package therefore ignores the parent it is planned under - the project
/// root holds every package, and a package id names the package alone.
pub fn provisional_id(
    parent: Option<&ElementId>,
    kind: ElementKind,
    name: &ElementName,
) -> Result<ElementId, ProvisionalIdError> {
    let inside = || parent.ok_or(ProvisionalIdError::MissingParent { kind });
    let id = match kind {
        ElementKind::Project => return Err(ProvisionalIdError::Whole),
        ElementKind::Package => format!("package:{name}"),
        ElementKind::Directory | ElementKind::Module | ElementKind::File => {
            format!("{}/{name}", inside()?)
        }
        ElementKind::Function => format!("{}#function:{name}", inside()?),
        ElementKind::Type => format!("{}#type:{name}", inside()?),
    };
    Ok(ElementId::new(id).expect("a name is never empty"))
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProvisionalIdError {
    #[error("an element of this kind needs a boundary to sit in")]
    MissingParent { kind: ElementKind },
    #[error("a project is the picture as a whole, not a part planned into it")]
    Whole,
}

/// What a plan must gain to add an element of `kind` named `name` inside
/// `parent`: the element itself under its provisional id, and the
/// containment that puts it there. A root-level element - one no boundary
/// holds - gains the element alone.
///
/// The entries come in the order a change set applies them: the element
/// before the relation that names it.
pub fn addition_of_element(
    parent: Option<&ElementId>,
    kind: ElementKind,
    name: &ElementName,
) -> Result<Vec<ProposedChange>, ProvisionalIdError> {
    let id = provisional_id(parent, kind, name)?;
    let mut changes = vec![ProposedChange::AddElement(Element {
        id: id.clone(),
        name: name.clone(),
        kind,
        fingerprint: None,
    })];
    if let Some(parent) = parent {
        changes.push(ProposedChange::AddRelation(Relation {
            from: parent.clone(),
            to: id,
            kind: RelationKind::Contains,
        }));
    }
    Ok(changes)
}

impl Plan {
    #[must_use]
    pub fn plans_removal_of_element(&self, id: &ElementId) -> bool {
        self.changes().iter().any(|planned| {
            matches!(&planned.change, ProposedChange::RemoveElement(removed) if removed == id)
        })
    }

    #[must_use]
    pub fn plans_addition_of_element(&self, id: &ElementId) -> bool {
        self.changes().iter().any(|planned| {
            matches!(&planned.change, ProposedChange::AddElement(added) if added.id == *id)
        })
    }

    /// The element whose planned removal takes `id` with it: `id` itself,
    /// or the nearest boundary above it the plan removes. None while no
    /// removal reaches it.
    #[must_use]
    pub fn removal_root_of(&self, id: &ElementId, base: &ArchitectureGraph) -> Option<ElementId> {
        let parents = containment_parents(base);
        let mut seen = BTreeSet::new();
        let mut current = Some(id.clone());
        while let Some(id) = current {
            if self.plans_removal_of_element(&id) {
                return Some(id);
            }
            if !seen.insert(id.clone()) {
                return None;
            }
            current = parents.get(&id).cloned();
        }
        None
    }

    /// What this plan must gain to remove `id`: the removal of every
    /// concrete dependency that crosses the border of what `id` contains,
    /// and the removal of `id` itself last, in the order a change set
    /// applies them.
    ///
    /// `base` is the architecture the sources declare: the crossings are
    /// real couplings that must go before the element can.
    ///
    /// Entries the plan already carries stay out, so every answer can be
    /// proposed without meeting one twice.
    #[must_use]
    pub fn removal_of_element(
        &self,
        id: &ElementId,
        base: &ArchitectureGraph,
    ) -> Vec<ProposedChange> {
        let parents = containment_parents(base);
        let mut changes: Vec<ProposedChange> = base
            .relations()
            .filter(|relation| relation.kind == RelationKind::DependsOn)
            .filter(|relation| {
                // A dependency with both ends inside is interior to the
                // subtree and goes with it; one with neither end inside is
                // no business of this removal.
                lies_within(&relation.from, id, &parents) != lies_within(&relation.to, id, &parents)
            })
            .filter(|relation| !self.plans_removal_of(relation))
            .map(|relation| ProposedChange::RemoveRelation(relation.clone()))
            .collect();
        if !self.plans_removal_of_element(id) {
            changes.push(ProposedChange::RemoveElement(id.clone()));
        }
        changes
    }

    /// What this plan already carries about the removal of `id`: the entry
    /// on the element, and every planned dependency removal that touches
    /// its containment subtree. Retracting all of them puts the element and
    /// its couplings back.
    #[must_use]
    pub fn planned_removal_of_element(
        &self,
        id: &ElementId,
        base: &ArchitectureGraph,
    ) -> Vec<ProposedChange> {
        let parents = containment_parents(base);
        self.changes()
            .iter()
            .map(|planned| &planned.change)
            .filter(|change| match change {
                ProposedChange::RemoveElement(removed) => removed == id,
                ProposedChange::RemoveRelation(relation) => {
                    lies_within(&relation.from, id, &parents)
                        || lies_within(&relation.to, id, &parents)
                }
                _ => false,
            })
            .cloned()
            .collect()
    }

    /// What this plan carries about the addition of `id`: the element
    /// itself, every planned element inside it, and every planned relation
    /// that touches any of them - the containment that puts them where they
    /// are included. Retracting all of them erases the addition.
    ///
    /// `base` is the architecture the picture shows, this plan's own
    /// additions drawn in ([`Plan::viewed_architecture`]): a planned
    /// element sits in the containment only there.
    #[must_use]
    pub fn planned_addition_of_element(
        &self,
        id: &ElementId,
        base: &ArchitectureGraph,
    ) -> Vec<ProposedChange> {
        let parents = containment_parents(base);
        let inside: BTreeSet<ElementId> = self
            .changes()
            .iter()
            .filter_map(|planned| match &planned.change {
                ProposedChange::AddElement(element) => Some(&element.id),
                _ => None,
            })
            .filter(|planned| lies_within(planned, id, &parents))
            .cloned()
            .collect();
        self.changes()
            .iter()
            .map(|planned| &planned.change)
            .filter(|change| match change {
                ProposedChange::AddElement(element) => inside.contains(&element.id),
                ProposedChange::AddRelation(relation) => {
                    inside.contains(&relation.from) || inside.contains(&relation.to)
                }
                _ => false,
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(text: &str) -> ElementId {
        ElementId::new(text).unwrap()
    }

    fn name(text: &str) -> ElementName {
        ElementName::new(text).unwrap()
    }

    fn depends(from: &str, to: &str) -> Relation {
        Relation {
            from: id(from),
            to: id(to),
            kind: RelationKind::DependsOn,
        }
    }

    fn contains(from: &str, to: &str) -> Relation {
        Relation {
            from: id(from),
            to: id(to),
            kind: RelationKind::Contains,
        }
    }

    /// package:a ⊃ a/lib ⊃ a/lib#type:X, package:b ⊃ b/lib. The type
    /// reaches b/lib, b/lib reaches a/lib, and a/lib reaches the type it
    /// holds itself.
    fn base() -> ArchitectureGraph {
        let mut graph = ArchitectureGraph::new();
        for (element, kind) in [
            ("package:a", ElementKind::Package),
            ("package:b", ElementKind::Package),
            ("a/lib", ElementKind::Module),
            ("a/lib#type:X", ElementKind::Type),
            ("b/lib", ElementKind::Module),
        ] {
            graph
                .add_element(Element {
                    id: id(element),
                    name: name(element),
                    kind,
                    fingerprint: None,
                })
                .unwrap();
        }
        for (from, to) in [
            ("package:a", "a/lib"),
            ("a/lib", "a/lib#type:X"),
            ("package:b", "b/lib"),
        ] {
            graph.add_relation(contains(from, to)).unwrap();
        }
        for (from, to) in [
            ("a/lib#type:X", "b/lib"),
            ("b/lib", "a/lib"),
            ("a/lib", "a/lib#type:X"),
        ] {
            graph.add_relation(depends(from, to)).unwrap();
        }
        graph
    }

    #[test]
    fn a_planned_module_is_named_by_the_boundary_that_holds_it() {
        assert_eq!(
            provisional_id(Some(&id("package:a")), ElementKind::Module, &name("wiring")),
            Ok(id("package:a/wiring"))
        );
    }

    #[test]
    fn a_planned_item_is_named_by_its_module_and_its_kind() {
        assert_eq!(
            provisional_id(Some(&id("a/lib")), ElementKind::Type, &name("Port")),
            Ok(id("a/lib#type:Port"))
        );
        assert_eq!(
            provisional_id(Some(&id("a/lib")), ElementKind::Function, &name("run")),
            Ok(id("a/lib#function:run"))
        );
    }

    #[test]
    fn a_planned_package_is_named_by_itself_wherever_it_is_planned() {
        assert_eq!(
            provisional_id(None, ElementKind::Package, &name("engine")),
            Ok(id("package:engine"))
        );
        assert_eq!(
            provisional_id(
                Some(&id("project:app")),
                ElementKind::Package,
                &name("engine")
            ),
            Ok(id("package:engine")),
            "the project root holds every package, so it adds nothing to the id"
        );
    }

    #[test]
    fn an_element_below_a_package_needs_the_boundary_it_sits_in() {
        assert_eq!(
            provisional_id(None, ElementKind::Module, &name("wiring")),
            Err(ProvisionalIdError::MissingParent {
                kind: ElementKind::Module
            })
        );
    }

    #[test]
    fn a_project_is_never_planned_into_a_picture() {
        assert_eq!(
            provisional_id(None, ElementKind::Project, &name("app")),
            Err(ProvisionalIdError::Whole)
        );
    }

    #[test]
    fn adding_an_element_states_the_element_before_the_containment() {
        let changes =
            addition_of_element(Some(&id("package:a")), ElementKind::Module, &name("wiring"))
                .unwrap();
        assert_eq!(
            changes,
            vec![
                ProposedChange::AddElement(Element {
                    id: id("package:a/wiring"),
                    name: name("wiring"),
                    kind: ElementKind::Module,
                    fingerprint: None,
                }),
                ProposedChange::AddRelation(contains("package:a", "package:a/wiring")),
            ]
        );
    }

    #[test]
    fn adding_a_root_level_element_states_the_element_alone() {
        let changes = addition_of_element(None, ElementKind::Package, &name("engine")).unwrap();
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn removing_an_element_severs_every_dependency_that_crosses_its_border() {
        let changes = Plan::new().removal_of_element(&id("package:a"), &base());
        assert_eq!(
            changes,
            vec![
                ProposedChange::RemoveRelation(depends("a/lib#type:X", "b/lib")),
                ProposedChange::RemoveRelation(depends("b/lib", "a/lib")),
                ProposedChange::RemoveElement(id("package:a")),
            ],
            "both directions cross; the couplings inside the package stay unstated"
        );
    }

    #[test]
    fn removing_an_element_leaves_the_containment_inside_it_unstated() {
        let changes = Plan::new().removal_of_element(&id("package:a"), &base());
        assert!(
            changes.iter().all(
                |change| !matches!(change, ProposedChange::RemoveRelation(relation)
                    if relation.kind == RelationKind::Contains)
            ),
            "an entry for the root of a subtree is the whole intent: {changes:?}"
        );
        assert_eq!(
            changes
                .iter()
                .filter(|change| matches!(change, ProposedChange::RemoveElement(_)))
                .count(),
            1,
            "nothing inside the subtree gains a removal of its own"
        );
    }

    #[test]
    fn a_severing_the_plan_already_carries_is_not_stated_twice() {
        let mut plan = Plan::new();
        plan.propose(ProposedChange::RemoveRelation(depends(
            "a/lib#type:X",
            "b/lib",
        )))
        .unwrap();
        assert_eq!(
            plan.removal_of_element(&id("package:a"), &base()),
            vec![
                ProposedChange::RemoveRelation(depends("b/lib", "a/lib")),
                ProposedChange::RemoveElement(id("package:a")),
            ]
        );
    }

    fn removing_package_a() -> Plan {
        let mut plan = Plan::new();
        for change in Plan::new().removal_of_element(&id("package:a"), &base()) {
            plan.propose(change).unwrap();
        }
        plan
    }

    #[test]
    fn a_planned_removal_reaches_everything_inside_the_element_it_names() {
        let plan = removing_package_a();
        for inside in ["package:a", "a/lib", "a/lib#type:X"] {
            assert_eq!(
                plan.removal_root_of(&id(inside), &base()),
                Some(id("package:a")),
                "{inside} goes with the package that holds it"
            );
        }
        assert_eq!(plan.removal_root_of(&id("b/lib"), &base()), None);
    }

    #[test]
    fn restoring_an_element_withdraws_its_removal_and_every_severing_with_it() {
        let plan = removing_package_a();
        let planned = plan.planned_removal_of_element(&id("package:a"), &base());
        assert_eq!(planned.len(), 3);

        let mut restored = plan.clone();
        for change in planned {
            restored.retract(&change).unwrap();
        }
        assert!(restored.is_empty());
    }

    #[test]
    fn erasing_a_planned_element_takes_the_planned_elements_inside_it() {
        let mut plan = Plan::new();
        for change in addition_of_element(None, ElementKind::Package, &name("engine")).unwrap() {
            plan.propose(change).unwrap();
        }
        for change in addition_of_element(
            Some(&id("package:engine")),
            ElementKind::Module,
            &name("wiring"),
        )
        .unwrap()
        {
            plan.propose(change).unwrap();
        }
        plan.propose(ProposedChange::AddRelation(depends(
            "b/lib",
            "package:engine/wiring",
        )))
        .unwrap();
        let viewed = plan.viewed_architecture(&base());

        let planned = plan.planned_addition_of_element(&id("package:engine"), &viewed);
        assert_eq!(
            planned.len(),
            4,
            "the package, the module inside it, its containment, and the dependency drawn to it"
        );
        let mut erased = plan.clone();
        for change in planned {
            erased.retract(&change).unwrap();
        }
        assert!(erased.is_empty());
    }
}
