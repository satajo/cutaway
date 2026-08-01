//! Test doubles for the driven ports.

use std::cell::RefCell;

use cutaway_inspection::ports::source_tree::{
    ProjectName, SourceFile, SourcePath, SourceTree, SourceTreeError,
};
use cutaway_planning::Plan;
use cutaway_planning::ports::plan_store::{PlanStore, PlanStoreError};

/// A source tree held fully in memory; scenarios describe project contents
/// through it without touching a real repository.
#[derive(Debug, Default)]
pub struct InMemorySourceTree {
    files: Vec<SourceFile>,
}

impl InMemorySourceTree {
    pub fn add_file(&mut self, path: SourcePath, contents: impl Into<Vec<u8>>) {
        self.files.push(SourceFile {
            path,
            contents: contents.into(),
        });
    }
}

impl SourceTree for InMemorySourceTree {
    fn name(&self) -> ProjectName {
        ProjectName::new("fixture").expect("the fixture name is never empty")
    }

    fn files(&self) -> Result<Vec<SourceFile>, SourceTreeError> {
        Ok(self.files.clone())
    }
}

/// A plan store that remembers the last saved plan, so scenarios can check
/// what would have reached disk.
#[derive(Debug, Default)]
pub struct InMemoryPlanStore {
    saved: RefCell<Option<Plan>>,
}

impl InMemoryPlanStore {
    #[must_use]
    pub fn saved(&self) -> Option<Plan> {
        self.saved.borrow().clone()
    }
}

impl PlanStore for InMemoryPlanStore {
    fn load(&self) -> Result<Option<Plan>, PlanStoreError> {
        Ok(self.saved.borrow().clone())
    }

    fn save(&self, plan: &Plan) -> Result<(), PlanStoreError> {
        *self.saved.borrow_mut() = Some(plan.clone());
        Ok(())
    }
}
