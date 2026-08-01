//! The architecture model of a software project.
//!
//! This crate is the innermost domain of Cutaway: a graph of architecture
//! elements and the relations between them. Every other crate either builds
//! this model (inspection and its adapters) or consumes it (comparison,
//! redlining, the GUI). It performs no I/O and depends on no framework.

mod element;
mod graph;
mod relation;

pub use element::{
    Element, ElementId, ElementKind, ElementName, InvalidElementId, InvalidElementName,
};
pub use graph::{ArchitectureGraph, GraphError};
pub use relation::{Relation, RelationKind};
