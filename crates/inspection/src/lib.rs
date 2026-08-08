//! Builds the architecture model from a project's sources.
//!
//! This is the application core for inspecting a project. It reads sources
//! through the [`ports::source_tree::SourceTree`] port, collects what every
//! [`ports::source_analyzer::SourceAnalyzer`] read out of them, and assembles
//! an [`cutaway_architecture::ArchitectureGraph`] whose skeleton is the file
//! tree itself. Adapters implement the ports; this crate performs no I/O
//! itself and names no concrete technology.

mod inspector;
pub mod ports;
mod substrate;

pub use inspector::{InspectionError, inspect};
pub use substrate::ExtentError;
