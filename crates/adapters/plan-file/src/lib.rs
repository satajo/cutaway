//! JSON plan store adapter: persists a [`cutaway_planning::Plan`] as
//! `cutaway.json` in the root of the planned repository.
//!
//! The file lives in the repository on purpose: it is the hand-off artifact
//! for an agent working in that repository, and it versions together with
//! the code it talks about. The format is versioned and stable; treat any
//! change to it as a breaking change of the agent contract.
//!
//! Every relation the file stores - in changes and in relation-subject
//! annotations alike - is a concrete source-level relation: real element
//! ids on both ends, never a rolled-up boundary pair and never a synthetic
//! `#self` own-content id. Files written before this contract held may
//! carry both; loading a plan against a known base graph expands and strips
//! them through `cutaway_planning`'s normalization (`Plan::normalized`).
//! This store stays a dumb serializer and applies none of that itself.
//!
//! A `remove-element` entry takes the containment subtree of the element
//! with it: the entry for the root of a subtree is the whole intent, and
//! the parts inside it carry no entry of their own. The `remove-relation`
//! entries beside it spell out the external couplings that must go first -
//! every dependency crossing the border of that subtree, in both
//! directions. The couplings interior to the subtree need no entry: they
//! leave with the code that holds them.
//!
//! An `add-element` entry names an element that exists in no source tree
//! yet, so its id is provisional: derived from the parent, the kind and the
//! name, in the shape a real id would take (`package:<name>`,
//! `<parent>/<name>`, `<parent>#<kind>:<name>`). Realize the element at
//! whatever source path the implementation calls for; the next inspection
//! replaces the provisional id with the real one. The `add-relation`
//! entry of kind `contains` beside it says which boundary the new element
//! belongs to.
//!
//! Its `kind` is one a language reads: `project`, `package`, `module`,
//! `function` or `type`. The directories and files a repository lies in are
//! read out of the source tree by inspecting it, never stated ahead of it,
//! so `directory` and `file` are refused - and refused for the whole file,
//! because a plan is applied as one intent and half of one is no intent at
//! all. The file names the reason, so whoever wrote it reads the law rather
//! than a parse error. Nothing is versioned around this: `cutaway.json` is
//! written afresh by the session that owns it.
//!
//! A `modifications` entry states how one element that stays is to change:
//! `rename` with the name it takes, `split` with the names it becomes,
//! `merge` with the id of the element it folds into, or `rework`, whose
//! `note` is the description of the work. At most one entry per subject, and
//! every subject is an element the inspected architecture holds. These are
//! structured intents for an agent to interpret, not changes to the graph:
//! no entry beside them redraws a dependency, because which couplings
//! survive a rename, a split or a merge is the work being ordered. A file
//! written before modifications existed carries no such array, and loads as
//! a plan with none.

mod format;

use std::path::{Path, PathBuf};

use cutaway_planning::Plan;
use cutaway_planning::ports::plan_store::{PlanStore, PlanStoreError};

pub struct JsonPlanStore {
    file: PathBuf,
}

impl JsonPlanStore {
    /// The store for the repository rooted at `repository`.
    #[must_use]
    pub fn for_repository(repository: &Path) -> Self {
        Self {
            file: repository.join("cutaway.json"),
        }
    }
}

impl PlanStore for JsonPlanStore {
    fn load(&self) -> Result<Option<Plan>, PlanStoreError> {
        let text = match std::fs::read_to_string(&self.file) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(PlanStoreError::Unreadable {
                    reason: error.to_string(),
                });
            }
        };
        let stored: format::StoredPlan =
            serde_json::from_str(&text).map_err(|error| PlanStoreError::Corrupt {
                reason: error.to_string(),
            })?;
        stored.into_plan().map(Some)
    }

    fn save(&self, plan: &Plan) -> Result<(), PlanStoreError> {
        let stored = format::StoredPlan::from_plan(plan);
        let text = serde_json::to_string_pretty(&stored).expect("the format is serializable");
        std::fs::write(&self.file, text + "\n").map_err(|error| PlanStoreError::Unwritable {
            reason: error.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{ElementId, ElementName, Relation, RelationKind};
    use cutaway_planning::{
        Modification, ModificationKind, Note, ProposedChange, SplitParts, Subject,
    };

    use super::*;

    fn relation(from: &str, to: &str) -> Relation {
        Relation {
            from: ElementId::new(from).unwrap(),
            to: ElementId::new(to).unwrap(),
            kind: RelationKind::DependsOn,
        }
    }

    fn marked_up_plan() -> Plan {
        let mut plan = Plan::new();
        plan.propose(ProposedChange::RemoveRelation(relation("a", "b")))
            .unwrap();
        plan.explain(
            &ProposedChange::RemoveRelation(relation("a", "b")),
            Some(Note::new("cut the cycle").unwrap()),
        )
        .unwrap();
        plan.propose(ProposedChange::AddRelation(relation("a", "c")))
            .unwrap();
        plan.annotate(
            Subject::Element(ElementId::new("package:a").unwrap()),
            Note::new("deprecated, shrink it").unwrap(),
        );
        plan.plan_modification(Modification {
            subject: ElementId::new("package:b").unwrap(),
            kind: ModificationKind::Split {
                into: SplitParts::new(vec![
                    ElementName::new("engine").unwrap(),
                    ElementName::new("transport").unwrap(),
                ])
                .unwrap(),
            },
            note: Some(Note::new("the transport belongs on its own").unwrap()),
        });
        plan
    }

    #[test]
    fn a_saved_plan_loads_back_identically() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonPlanStore::for_repository(dir.path());
        let plan = marked_up_plan();

        store.save(&plan).unwrap();
        assert_eq!(store.load().unwrap(), Some(plan));
    }

    #[test]
    fn a_project_without_a_stored_plan_has_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonPlanStore::for_repository(dir.path());
        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn a_corrupt_file_is_reported_not_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("cutaway.json"), "not json").unwrap();
        let store = JsonPlanStore::for_repository(dir.path());
        assert!(matches!(store.load(), Err(PlanStoreError::Corrupt { .. })));
    }

    #[test]
    fn the_stored_form_is_readable_json_for_agents() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonPlanStore::for_repository(dir.path());
        store.save(&marked_up_plan()).unwrap();

        let text = std::fs::read_to_string(dir.path().join("cutaway.json")).unwrap();
        assert!(text.contains("\"version\": 1"));
        assert!(text.contains("remove-relation"));
        assert!(text.contains("cut the cycle"));
        assert!(text.contains("\"modify\": \"split\""));
    }

    #[test]
    fn a_plan_file_written_before_modifications_existed_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("cutaway.json"),
            r#"{"version": 1, "changes": [], "annotations": []}"#,
        )
        .unwrap();
        let store = JsonPlanStore::for_repository(dir.path());
        assert_eq!(store.load().unwrap(), Some(Plan::new()));
    }

    #[test]
    fn a_split_naming_a_single_element_is_reported_as_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("cutaway.json"),
            r#"{"version": 1, "changes": [], "annotations": [], "modifications":
               [{"subject": "package:a", "modify": "split", "into": ["engine"]}]}"#,
        )
        .unwrap();
        let store = JsonPlanStore::for_repository(dir.path());
        assert!(matches!(store.load(), Err(PlanStoreError::Corrupt { .. })));
    }

    #[test]
    fn a_planned_directory_or_file_is_reported_as_corrupt() {
        for kind in ["directory", "file"] {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(
                dir.path().join("cutaway.json"),
                format!(
                    r#"{{"version": 1, "changes": [{{"action": "add-element", "element":
                       {{"id": "app/src", "name": "src", "kind": "{kind}"}}}}],
                       "annotations": []}}"#
                ),
            )
            .unwrap();
            let store = JsonPlanStore::for_repository(dir.path());
            assert!(
                matches!(store.load(), Err(PlanStoreError::Corrupt { .. })),
                "the tree a project lies in is inspected, never planned: {kind}"
            );
        }
    }

    #[test]
    fn two_modifications_of_one_element_are_reported_as_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("cutaway.json"),
            r#"{"version": 1, "changes": [], "annotations": [], "modifications":
               [{"subject": "package:a", "modify": "rework"},
                {"subject": "package:a", "modify": "rename", "to": "engine"}]}"#,
        )
        .unwrap();
        let store = JsonPlanStore::for_repository(dir.path());
        assert!(matches!(store.load(), Err(PlanStoreError::Corrupt { .. })));
    }
}
