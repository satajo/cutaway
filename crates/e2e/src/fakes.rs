//! Test doubles for the inspection ports.

use cutaway_inspection::ports::source_tree::{SourceFile, SourcePath, SourceTree, SourceTreeError};

/// A source tree held fully in memory; scenarios describe project contents
/// through it without touching a real repository.
#[derive(Debug, Default)]
pub struct InMemorySourceTree {
    files: Vec<SourceFile>,
}

impl InMemorySourceTree {
    pub fn add_file(&mut self, path: SourcePath, contents: impl Into<Vec<u8>>) {
        self.files.push(SourceFile {
            path,
            contents: contents.into(),
        });
    }
}

impl SourceTree for InMemorySourceTree {
    fn files(&self) -> Result<Vec<SourceFile>, SourceTreeError> {
        Ok(self.files.clone())
    }
}
