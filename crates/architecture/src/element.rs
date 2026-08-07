use std::fmt;

/// Identifies an [`Element`] within one [`crate::ArchitectureGraph`].
///
/// Ids are opaque: the producer of a graph chooses the scheme, consumers only
/// compare them. Comparing graphs of the same project across versions relies
/// on the producer deriving ids deterministically from the sources.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ElementId(String);

impl ElementId {
    pub fn new(id: impl Into<String>) -> Result<Self, InvalidElementId> {
        let id = id.into();
        if id.is_empty() {
            return Err(InvalidElementId::Empty);
        }
        Ok(Self(id))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ElementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum InvalidElementId {
    #[error("an element id must not be empty")]
    Empty,
}

/// The human-facing name of an element, as it appears in the sources.
/// Unlike [`ElementId`], names carry no uniqueness guarantee.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ElementName(String);

impl ElementName {
    pub fn new(name: impl Into<String>) -> Result<Self, InvalidElementName> {
        let name = name.into();
        if name.is_empty() {
            return Err(InvalidElementName::Empty);
        }
        Ok(Self(name))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ElementName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum InvalidElementName {
    #[error("an element name must not be empty")]
    Empty,
}

/// The coarse classification of an element. The set grows as new lenses need
/// to distinguish more of the architecture.
///
/// The kinds form the levels of the containment hierarchy every producer
/// follows: project ⊃ package ⊃ directory* ⊃ module ⊃ item. Each level is a
/// boundary in the sense of the boundary lens: relations crossing it mean
/// more than relations inside it. Directories are the one level that repeats:
/// they nest inside each other, and a package that needs none holds its
/// modules directly.
///
/// The declaration order is the order of the hierarchy, so every ordering of
/// kinds reads from the coarsest level down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ElementKind {
    /// One inspected source tree: a repository, a monorepo root.
    Project,
    /// A unit of distribution and dependency declaration: a Rust crate, a Go
    /// module, a Java artifact, an npm package.
    Package,
    /// A source directory that is organization and nothing else: the author
    /// grouped files, and the language attaches no meaning to the grouping.
    /// A language that does read meaning into its directories states them at
    /// the level that carries that meaning instead - a Go directory is the
    /// compilation and import unit, and a Rust `mod` tree carries the
    /// nesting - so neither of those uses this kind.
    Directory,
    /// A grouping of code within a package: a source file, a namespace.
    Module,
    /// An executable unit: a function, a method, a procedure.
    Function,
    /// A data or interface definition: a struct, an enum, a trait.
    Type,
}

/// One node of the architecture graph.
///
/// All fields enforce their own invariants, so the struct exposes them
/// directly; there is no cross-field invariant to protect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub id: ElementId,
    pub name: ElementName,
    pub kind: ElementKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_element_id_must_not_be_empty() {
        assert_eq!(ElementId::new(""), Err(InvalidElementId::Empty));
    }

    #[test]
    fn an_element_name_must_not_be_empty() {
        assert_eq!(ElementName::new(""), Err(InvalidElementName::Empty));
    }
}
