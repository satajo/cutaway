use cutaway_architecture::{Element, ElementId, Relation};

use crate::ports::source_tree::{SourceFile, SourcePath};

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
///   file, `<path>#<kind>:<name>` for a declaration.
/// - `parent` expresses containment. A parentless element attaches to the
///   project root. Containment must form a tree.
/// - Relations may reference only elements declared in the same
///   [`SourceStructure`].
/// - Two analyzers must not declare the same element; inspection fails if
///   they do.
pub trait SourceAnalyzer {
    fn analyze(&self, files: &[SourceFile]) -> Result<SourceStructure, SourceAnalysisError>;
}

/// Everything one analyzer found in a source tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceStructure {
    pub elements: Vec<AnalyzedElement>,
    pub relations: Vec<Relation>,
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
