//! Planned changes to an architecture: redlines, annotations, and plans.
//!
//! This crate is the planning core of Cutaway. A [`Redline`] is an ordered
//! list of proposed changes drawn on top of an existing architecture graph,
//! the way a reviewer marks up a paper drawing. A [`Plan`] wraps the redline
//! with rationale: a note per change, plus [`Annotation`]s on parts of the
//! architecture that stay as they are. Plans persist through the
//! [`ports::plan_store::PlanStore`] port and become the work order handed to
//! an agent.

mod annotation;
mod plan;
pub mod ports;
mod redline;

pub use annotation::{Annotation, InvalidNote, Note, Subject};
pub use plan::{Plan, PlanError, PlannedChange};
pub use redline::{ProposedChange, Redline, RedlineError};
