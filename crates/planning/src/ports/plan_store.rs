use crate::plan::Plan;

/// Driven port: where the plan of a project lives between sessions.
///
/// One store is bound to one project; the composition root creates it when a
/// project opens. The stored plan is also the hand-off artifact for agents,
/// so an implementation must keep it in a form an agent can read where the
/// agent will look for it.
pub trait PlanStore {
    /// The plan saved earlier, or `None` when the project has none yet.
    fn load(&self) -> Result<Option<Plan>, PlanStoreError>;

    fn save(&self, plan: &Plan) -> Result<(), PlanStoreError>;
}

#[derive(Debug, thiserror::Error)]
pub enum PlanStoreError {
    #[error("cannot read the stored plan: {reason}")]
    Unreadable { reason: String },
    #[error("cannot write the plan: {reason}")]
    Unwritable { reason: String },
    #[error("the stored plan is corrupt: {reason}")]
    Corrupt { reason: String },
}
