use std::fmt;

use crate::element::ElementId;

/// A directed relation between two elements of the same graph.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Relation {
    pub from: ElementId,
    pub to: ElementId,
    pub kind: RelationKind,
}

impl fmt::Display for Relation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let arrow = match self.kind {
            RelationKind::Contains => "contains",
            RelationKind::DependsOn => "depends on",
        };
        write!(f, "{} {arrow} {}", self.from, self.to)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RelationKind {
    /// `from` structurally contains `to`: a file contains a function.
    Contains,
    /// `from` needs `to` to work: a module imports another.
    DependsOn,
}
