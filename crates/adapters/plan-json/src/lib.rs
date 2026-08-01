//! JSON plan store adapter: persists a
//! [`cutaway_redlining::Plan`] as `.cutaway/redline.json` inside the planned
//! repository.
//!
//! The file lives in the repository on purpose: it is the hand-off artifact
//! for an agent working in that repository, and it versions together with
//! the code it talks about. The format is versioned and stable; treat any
//! change to it as a breaking change of the agent contract.

mod format;

use std::path::{Path, PathBuf};

use cutaway_redlining::Plan;
use cutaway_redlining::ports::plan_store::{PlanStore, PlanStoreError};

pub struct JsonPlanStore {
    file: PathBuf,
}

impl JsonPlanStore {
    /// The store for the repository rooted at `repository`.
    #[must_use]
    pub fn for_repository(repository: &Path) -> Self {
        Self {
            file: repository.join(".cutaway").join("redline.json"),
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
        let unwritable = |error: std::io::Error| PlanStoreError::Unwritable {
            reason: error.to_string(),
        };
        let directory = self.file.parent().expect("the store path has a parent");
        std::fs::create_dir_all(directory).map_err(unwritable)?;
        let stored = format::StoredPlan::from_plan(plan);
        let text = serde_json::to_string_pretty(&stored).expect("the format is serializable");
        std::fs::write(&self.file, text + "\n").map_err(unwritable)
    }
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{ElementId, Relation, RelationKind};
    use cutaway_redlining::{Note, ProposedChange, Subject};

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
        std::fs::create_dir_all(dir.path().join(".cutaway")).unwrap();
        std::fs::write(dir.path().join(".cutaway/redline.json"), "not json").unwrap();
        let store = JsonPlanStore::for_repository(dir.path());
        assert!(matches!(store.load(), Err(PlanStoreError::Corrupt { .. })));
    }

    #[test]
    fn the_stored_form_is_readable_json_for_agents() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonPlanStore::for_repository(dir.path());
        store.save(&marked_up_plan()).unwrap();

        let text = std::fs::read_to_string(dir.path().join(".cutaway/redline.json")).unwrap();
        assert!(text.contains("\"version\": 1"));
        assert!(text.contains("remove-relation"));
        assert!(text.contains("cut the cycle"));
    }
}
