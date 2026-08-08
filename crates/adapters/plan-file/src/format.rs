//! The on-disk shape of a plan. Version 1.
//!
//! Kept separate from the domain types so the file format can stay stable
//! while the domain evolves. Parsing validates back into domain invariants;
//! a file that fails validation is corrupt, not partially usable.

use cutaway_architecture::{Element, ElementId, ElementKind, ElementName, Relation, RelationKind};
use cutaway_planning::ports::plan_store::PlanStoreError;
use cutaway_planning::{
    Modification, ModificationKind, Note, Plan, ProposedChange, SplitParts, Subject,
};
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub struct StoredPlan {
    version: u32,
    changes: Vec<StoredChange>,
    annotations: Vec<StoredAnnotation>,
    /// Modifications joined the format after version 1 was in the field, and
    /// they state no change to what the architecture holds: a file written
    /// without them is a whole plan, so their absence reads as none.
    #[serde(default)]
    modifications: Vec<StoredModification>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredChange {
    #[serde(flatten)]
    action: StoredAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
enum StoredAction {
    AddElement { element: StoredElement },
    RemoveElement { element: String },
    AddRelation { relation: StoredRelation },
    RemoveRelation { relation: StoredRelation },
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredElement {
    id: String,
    name: String,
    kind: StoredKind,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum StoredKind {
    Project,
    Package,
    Directory,
    Module,
    File,
    Function,
    Type,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredRelation {
    from: String,
    to: String,
    kind: StoredRelationKind,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum StoredRelationKind {
    Contains,
    DependsOn,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredAnnotation {
    subject: StoredSubject,
    note: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum StoredSubject {
    Element(String),
    Relation(StoredRelation),
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredModification {
    subject: String,
    #[serde(flatten)]
    kind: StoredModificationKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "modify", rename_all = "kebab-case")]
enum StoredModificationKind {
    Rename { to: String },
    Split { into: Vec<String> },
    Merge { with: String },
    Rework,
}

impl StoredPlan {
    pub fn from_plan(plan: &Plan) -> Self {
        Self {
            version: VERSION,
            changes: plan
                .changes()
                .iter()
                .map(|planned| StoredChange {
                    action: StoredAction::from_change(&planned.change),
                    note: planned.note.as_ref().map(|note| note.as_str().to_owned()),
                })
                .collect(),
            annotations: plan
                .annotations()
                .iter()
                .map(|annotation| StoredAnnotation {
                    subject: match &annotation.subject {
                        Subject::Element(id) => StoredSubject::Element(id.as_str().to_owned()),
                        Subject::Relation(relation) => {
                            StoredSubject::Relation(StoredRelation::from_relation(relation))
                        }
                    },
                    note: annotation.note.as_str().to_owned(),
                })
                .collect(),
            modifications: plan
                .modifications()
                .map(|modification| StoredModification {
                    subject: modification.subject.as_str().to_owned(),
                    kind: StoredModificationKind::from_kind(&modification.kind),
                    note: modification
                        .note
                        .as_ref()
                        .map(|note| note.as_str().to_owned()),
                })
                .collect(),
        }
    }

    pub fn into_plan(self) -> Result<Plan, PlanStoreError> {
        if self.version != VERSION {
            return Err(PlanStoreError::Corrupt {
                reason: format!("unsupported plan version {}", self.version),
            });
        }
        let mut plan = Plan::new();
        for stored in self.changes {
            let change = stored.action.into_change()?;
            plan.propose(change.clone()).map_err(corrupt)?;
            if let Some(note) = stored.note {
                plan.explain(&change, Some(Note::new(note).map_err(corrupt)?))
                    .map_err(corrupt)?;
            }
        }
        for stored in self.annotations {
            let subject = match stored.subject {
                StoredSubject::Element(id) => Subject::Element(element_id(id)?),
                StoredSubject::Relation(relation) => Subject::Relation(relation.into_relation()?),
            };
            if plan.annotation_of(&subject).is_some() {
                return Err(PlanStoreError::Corrupt {
                    reason: "duplicate annotation subject".to_owned(),
                });
            }
            plan.annotate(subject, Note::new(stored.note).map_err(corrupt)?);
        }
        for stored in self.modifications {
            let subject = element_id(stored.subject)?;
            if plan.modification_of(&subject).is_some() {
                return Err(PlanStoreError::Corrupt {
                    reason: "duplicate modification subject".to_owned(),
                });
            }
            let note = stored.note.map(Note::new).transpose().map_err(corrupt)?;
            plan.plan_modification(Modification {
                subject,
                kind: stored.kind.into_kind()?,
                note,
            });
        }
        Ok(plan)
    }
}

impl StoredModificationKind {
    fn from_kind(kind: &ModificationKind) -> Self {
        match kind {
            ModificationKind::Rename { to } => Self::Rename {
                to: to.as_str().to_owned(),
            },
            ModificationKind::Split { into } => Self::Split {
                into: into
                    .names()
                    .iter()
                    .map(|name| name.as_str().to_owned())
                    .collect(),
            },
            ModificationKind::Merge { with } => Self::Merge {
                with: with.as_str().to_owned(),
            },
            ModificationKind::Rework => Self::Rework,
        }
    }

    fn into_kind(self) -> Result<ModificationKind, PlanStoreError> {
        Ok(match self {
            Self::Rename { to } => ModificationKind::Rename {
                to: ElementName::new(to).map_err(corrupt)?,
            },
            Self::Split { into } => ModificationKind::Split {
                into: SplitParts::new(
                    into.into_iter()
                        .map(|name| ElementName::new(name).map_err(corrupt))
                        .collect::<Result<Vec<ElementName>, PlanStoreError>>()?,
                )
                .map_err(corrupt)?,
            },
            Self::Merge { with } => ModificationKind::Merge {
                with: element_id(with)?,
            },
            Self::Rework => ModificationKind::Rework,
        })
    }
}

impl StoredAction {
    fn from_change(change: &ProposedChange) -> Self {
        match change {
            ProposedChange::AddElement(element) => Self::AddElement {
                element: StoredElement {
                    id: element.id.as_str().to_owned(),
                    name: element.primary_name().as_str().to_owned(),
                    kind: StoredKind::from_kind(element.primary_kind()),
                },
            },
            ProposedChange::RemoveElement(id) => Self::RemoveElement {
                element: id.as_str().to_owned(),
            },
            ProposedChange::AddRelation(relation) => Self::AddRelation {
                relation: StoredRelation::from_relation(relation),
            },
            ProposedChange::RemoveRelation(relation) => Self::RemoveRelation {
                relation: StoredRelation::from_relation(relation),
            },
        }
    }

    fn into_change(self) -> Result<ProposedChange, PlanStoreError> {
        Ok(match self {
            Self::AddElement { element } => ProposedChange::AddElement(Element::of_kind(
                element_id(element.id)?,
                element.kind.into_kind(),
                ElementName::new(element.name).map_err(corrupt)?,
            )),
            Self::RemoveElement { element } => ProposedChange::RemoveElement(element_id(element)?),
            Self::AddRelation { relation } => {
                ProposedChange::AddRelation(relation.into_relation()?)
            }
            Self::RemoveRelation { relation } => {
                ProposedChange::RemoveRelation(relation.into_relation()?)
            }
        })
    }
}

impl StoredKind {
    fn from_kind(kind: ElementKind) -> Self {
        match kind {
            ElementKind::Project => Self::Project,
            ElementKind::Package => Self::Package,
            ElementKind::Directory => Self::Directory,
            ElementKind::Module => Self::Module,
            ElementKind::File => Self::File,
            ElementKind::Function => Self::Function,
            ElementKind::Type => Self::Type,
        }
    }

    fn into_kind(self) -> ElementKind {
        match self {
            Self::Project => ElementKind::Project,
            Self::Package => ElementKind::Package,
            Self::Directory => ElementKind::Directory,
            Self::Module => ElementKind::Module,
            Self::File => ElementKind::File,
            Self::Function => ElementKind::Function,
            Self::Type => ElementKind::Type,
        }
    }
}

impl StoredRelation {
    fn from_relation(relation: &Relation) -> Self {
        Self {
            from: relation.from.as_str().to_owned(),
            to: relation.to.as_str().to_owned(),
            kind: match relation.kind {
                RelationKind::Contains => StoredRelationKind::Contains,
                RelationKind::DependsOn => StoredRelationKind::DependsOn,
            },
        }
    }

    fn into_relation(self) -> Result<Relation, PlanStoreError> {
        Ok(Relation {
            from: element_id(self.from)?,
            to: element_id(self.to)?,
            kind: match self.kind {
                StoredRelationKind::Contains => RelationKind::Contains,
                StoredRelationKind::DependsOn => RelationKind::DependsOn,
            },
        })
    }
}

fn element_id(id: String) -> Result<ElementId, PlanStoreError> {
    ElementId::new(id).map_err(corrupt)
}

fn corrupt(error: impl std::fmt::Display) -> PlanStoreError {
    PlanStoreError::Corrupt {
        reason: error.to_string(),
    }
}
