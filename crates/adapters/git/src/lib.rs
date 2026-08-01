//! Git adapter: presents a git repository as a
//! [`cutaway_inspection::ports::source_tree::SourceTree`].
//!
//! Only committed state is visible: the tree of the `HEAD` commit. The
//! working directory and the index are not part of any version, so they stay
//! out of the architecture. Access goes through gix, keeping the whole
//! application pure Rust.

use std::fmt::Display;
use std::path::{Path, PathBuf};

use cutaway_inspection::ports::source_tree::{SourceFile, SourcePath, SourceTree, SourceTreeError};
use gix::bstr::ByteSlice;

pub struct GitSourceTree {
    repo: gix::Repository,
}

impl GitSourceTree {
    /// Opens the repository at `path` (a worktree or a `.git` directory).
    pub fn open(path: &Path) -> Result<Self, OpenRepositoryError> {
        let repo = gix::open(path).map_err(|source| OpenRepositoryError::NotARepository {
            path: path.to_owned(),
            reason: source.to_string(),
        })?;
        Ok(Self { repo })
    }
}

impl SourceTree for GitSourceTree {
    fn files(&self) -> Result<Vec<SourceFile>, SourceTreeError> {
        let commit = self.repo.head_commit().map_err(unreadable)?;
        let tree = commit.tree().map_err(unreadable)?;

        let mut recorder = gix::traverse::tree::Recorder::default();
        tree.traverse()
            .breadthfirst(&mut recorder)
            .map_err(unreadable)?;

        let mut files = Vec::new();
        for record in recorder.records {
            if !record.mode.is_blob() {
                continue;
            }
            let path = record
                .filepath
                .to_str()
                .map_err(|_| SourceTreeError::Unreadable {
                    reason: format!("the tree contains a non-UTF-8 path: {}", record.filepath),
                })?;
            let blob = self.repo.find_object(record.oid).map_err(unreadable)?;
            files.push(SourceFile {
                path: SourcePath::new(path)?,
                contents: blob.data.clone(),
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }
}

fn unreadable(source: impl Display) -> SourceTreeError {
    SourceTreeError::Unreadable {
        reason: source.to_string(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OpenRepositoryError {
    #[error("{path} is not a git repository: {reason}")]
    NotARepository { path: PathBuf, reason: String },
}
