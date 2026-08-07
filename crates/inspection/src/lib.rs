//! Builds the architecture model from a project's sources.
//!
//! This is the application core for inspecting a project. It reads sources
//! through the [`ports::source_tree::SourceTree`] port, extracts structure
//! through [`ports::syntax_analyzer::SyntaxAnalyzer`] implementations, and
//! assembles an [`cutaway_architecture::ArchitectureGraph`]. Adapters
//! implement the ports; this crate performs no I/O itself and names no
//! concrete technology.

mod inspector;
pub mod ports;
mod unclaimed;

pub use inspector::{InspectionError, inspect};
