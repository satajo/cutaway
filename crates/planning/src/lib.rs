//! Planned changes to an architecture: change sets, annotations, and plans.
//!
//! This crate is the planning core of Cutaway. A [`ChangeSet`] is an ordered
//! list of proposed changes drawn on top of an existing architecture graph,
//! the way a reviewer marks up a paper drawing. A [`Plan`] wraps the change
//! set with rationale: a note per change, plus [`Annotation`]s on parts of
//! the architecture that stay as they are. Plans persist through the
//! [`ports::plan_store::PlanStore`] port and become the work order handed to
//! an agent.

mod annotation;
mod change_set;
mod containment;
mod element;
mod normalize;
mod plan;
pub mod ports;

pub use annotation::{Annotation, InvalidNote, Note, Subject};
pub use change_set::{ChangeSet, ChangeSetError, ProposedChange};
pub use element::{ProvisionalIdError, addition_of_element, provisional_id};
pub use plan::{GroupStanding, Plan, PlanError, PlannedChange};
