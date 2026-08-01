use cutaway_architecture::{ElementKind, ElementName};

use crate::ports::source_tree::{SourceFile, SourcePath};

/// Driven port: extracts the declared structure of a single source file.
///
/// One implementation understands one language. The core asks [`supports`]
/// before calling [`analyze`], so an implementation only ever sees files it
/// claimed.
///
/// [`supports`]: SyntaxAnalyzer::supports
/// [`analyze`]: SyntaxAnalyzer::analyze
pub trait SyntaxAnalyzer {
    fn supports(&self, path: &SourcePath) -> bool;
    fn analyze(&self, file: &SourceFile) -> Result<Vec<Declaration>, SyntaxAnalysisError>;
}

/// A top-level declaration found in one source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub name: ElementName,
    pub kind: ElementKind,
}

#[derive(Debug, thiserror::Error)]
pub enum SyntaxAnalysisError {
    #[error("{path} is not valid UTF-8")]
    NonUtf8Text { path: SourcePath },
    #[error("cannot parse {path}: {reason}")]
    Unparseable { path: SourcePath, reason: String },
}
