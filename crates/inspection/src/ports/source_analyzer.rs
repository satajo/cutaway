use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{Element, ElementId, Relation};

use crate::ports::source_tree::{DirectoryPath, SourceFile, SourcePath};

/// Driven port: extracts architecture facts from one version of a project's
/// sources.
///
/// One implementation understands one language ecosystem and turns manifests
/// and source text into elements and relations. The analyzer sees the whole
/// tree at once because dependency resolution needs project-wide knowledge
/// (which packages exist, which file a module path leads to).
///
/// Contract, enforced by the inspector where possible:
/// - Element ids derive only from source paths, kinds, and names — never from
///   ambient state — so the graphs of two versions of a project align.
///   Established schemes: `project:<name>`, `package:<name>`, `<path>` for a
///   file, `<path>#<kind>:<name>` for a declaration. A declaration nested in
///   another extends the holder's name with the language's own separator
///   (`<path>#function:Config::new`, `<path>#function:Config.load`), so the
///   id stays deterministic however deep declarations nest.
/// - `parent` expresses containment. A parentless element attaches to the
///   project root. Containment must form a tree.
/// - Relations may reference only elements declared in the same
///   [`SourceStructure`].
/// - Two analyzers must not declare the same element; inspection fails if
///   they do.
/// - The analyzer claims every file it read meaning from and states the
///   territory of every package it discovered, so the inspector can show
///   what no language spoke for. See [`SourceStructure::claimed`] and
///   [`SourceStructure::territories`].
pub trait SourceAnalyzer {
    fn analyze(&self, files: &[SourceFile]) -> Result<SourceStructure, SourceAnalysisError>;
}

/// Everything one analyzer found in a source tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceStructure {
    pub elements: Vec<AnalyzedElement>,
    pub relations: Vec<Relation>,
    /// The files this analyzer read meaning from.
    ///
    /// The invariant: claim exactly the files the structure's elements and
    /// dissolutions represent - every source that became an element or
    /// dissolved into one, and every manifest that named a package. The
    /// inspector shows every unclaimed file as a plain file element, so the
    /// failure modes are asymmetric: under-claiming a file whose element
    /// carries its path as id fails inspection loudly on the collision,
    /// while over-claiming a file nothing represents makes it disappear
    /// from the picture silently - inspection cannot detect that, so only
    /// this contract prevents the regression. A manifest read but made
    /// nothing of (a workspace-only manifest naming no package) is not
    /// claimed: nothing represents it, so it keeps a place of its own in
    /// the picture.
    pub claimed: BTreeSet<SourcePath>,
    /// The directory each discovered package occupies, mapped to the
    /// package element's id. The territory of a package is the directory
    /// subtree it occupies: what the languages leave unclaimed inside a
    /// territory still belongs inside that package's boundary.
    pub territories: BTreeMap<DirectoryPath, ElementId>,
}

/// One element an analyzer found, plus where it sits in the containment tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalyzedElement {
    pub element: Element,
    pub parent: Option<ElementId>,
}

#[derive(Debug, thiserror::Error)]
pub enum SourceAnalysisError {
    #[error("{path} is not valid UTF-8")]
    NonUtf8Text { path: SourcePath },
    #[error("cannot parse {path}: {reason}")]
    Unparseable { path: SourcePath, reason: String },
}
