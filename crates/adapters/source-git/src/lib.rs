//! Git adapter: presents a git repository as a
//! [`cutaway_inspection::ports::source_tree::SourceTree`].
//!
//! Only committed state is visible: the tree of the `HEAD` commit. The
//! working directory and the index are not part of any version, so they stay
//! out of the architecture. Access goes through gix, keeping the whole
//! application pure Rust.

use std::fmt::Display;
use std::path::{Path, PathBuf};

use cutaway_inspection::ports::source_tree::{
    ProjectName, SourceFile, SourcePath, SourceTree, SourceTreeError,
};
use gix::bstr::ByteSlice;

pub struct GitSourceTree {
    repo: gix::Repository,
    name: ProjectName,
}

impl GitSourceTree {
    /// Opens the repository at `path` (a worktree or a `.git` directory).
    /// The project takes its name from the worktree directory.
    pub fn open(path: &Path) -> Result<Self, OpenRepositoryError> {
        let repo = gix::open(path).map_err(|source| OpenRepositoryError::NotARepository {
            path: path.to_owned(),
            reason: source.to_string(),
        })?;
        let directory = repo.workdir().unwrap_or(path);
        let name = directory
            .canonicalize()
            .ok()
            .as_deref()
            .unwrap_or(directory)
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| ProjectName::new(n).ok())
            .ok_or_else(|| OpenRepositoryError::Unnameable {
                path: path.to_owned(),
            })?;
        Ok(Self { repo, name })
    }
}

impl SourceTree for GitSourceTree {
    fn name(&self) -> ProjectName {
        self.name.clone()
    }

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
    #[error("{path} yields no usable project name")]
    Unnameable { path: PathBuf },
}
