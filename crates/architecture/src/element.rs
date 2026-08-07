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
/// modules directly. Files sit beside modules under a directory or a
/// package and contain nothing.
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
    /// One source file standing as itself: a leaf that no language read
    /// declarations out of, shown where it lies in the directory tree. It
    /// holds nothing - declarations belong to modules, and a file with
    /// declarations is a module.
    File,
    /// An executable unit: a function, a method, a procedure.
    Function,
    /// A data or interface definition: a struct, an enum, a trait.
    Type,
}

/// What an element holds, condensed to one number, so two versions can ask
/// "did it change inside" without carrying the contents.
///
/// Equal fingerprints read as unchanged. The digest is deterministic across
/// platforms and runs, because graphs of different versions must align. The
/// honest limit: a hash collision reads as unchanged, which is the accepted
/// cost of not carrying contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fingerprint(u64);

impl Fingerprint {
    /// Digests the contents with FNV-1a 64, written out here because the
    /// domain depends on no hash crate.
    #[must_use]
    pub fn of(contents: &[u8]) -> Self {
        const OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;
        const PRIME: u64 = 1_099_511_628_211;
        let mut digest = OFFSET_BASIS;
        for byte in contents {
            digest ^= u64::from(*byte);
            digest = digest.wrapping_mul(PRIME);
        }
        Self(digest)
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
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
    /// What the element holds, condensed for change detection between
    /// versions; None where the producer made no statement about the
    /// contents - which is different from stating they are empty.
    pub fingerprint: Option<Fingerprint>,
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

    #[test]
    fn the_same_contents_always_give_the_same_fingerprint() {
        assert_eq!(
            Fingerprint::of(b"fn main() {}"),
            Fingerprint::of(b"fn main() {}")
        );
    }

    #[test]
    fn different_contents_give_different_fingerprints() {
        assert_ne!(
            Fingerprint::of(b"fn main() {}"),
            Fingerprint::of(b"fn main() { run(); }")
        );
    }

    #[test]
    fn empty_contents_have_a_valid_and_stable_fingerprint() {
        assert_eq!(Fingerprint::of(b""), Fingerprint::of(b""));
        // The FNV-1a offset basis, so the digest is pinned across releases
        // and platforms alike.
        assert_eq!(Fingerprint::of(b"").to_string(), "cbf29ce484222325");
    }
}
