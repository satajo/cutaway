//! Git adapter: presents a git repository as a
//! [`cutaway_inspection::ports::source_tree::SourceTree`] and as a
//! [`cutaway_inspection::ports::project_history::ProjectHistory`], where a
//! version is a commit.
//!
//! Only committed state is visible: the tree of a commit. The working
//! directory and the index are not part of any version, so they stay out of
//! the architecture. Access goes through gix, keeping the whole application
//! pure Rust.

use std::error::Error;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use cutaway_inspection::ports::project_history::{
    ProjectHistory, ProjectHistoryError, Version, VersionId,
};
use cutaway_inspection::ports::source_tree::{
    ProjectName, SourceFile, SourcePath, SourceTree, SourceTreeError,
};
use gix::bstr::ByteSlice;

pub struct GitSourceTree {
    repo: gix::Repository,
    name: ProjectName,
    at: Revision,
}

/// Which commit a tree presents.
///
/// `Head` resolves on every read, so a tree opened on a repository follows
/// whatever is checked out. `Pinned` names one commit for good, so a tree
/// taken out of the history keeps showing that version even after the
/// repository moves on.
enum Revision {
    Head,
    Pinned(gix::ObjectId),
}

impl GitSourceTree {
    /// Opens the repository at `path` (a worktree or a `.git` directory).
    /// The project takes its name from the worktree directory.
    pub fn open(path: &Path) -> Result<Self, OpenRepositoryError> {
        let repo = gix::open(path).map_err(|source| OpenRepositoryError::NotARepository {
            path: path.to_owned(),
            source: Box::new(source),
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
        Ok(Self {
            repo,
            name,
            at: Revision::Head,
        })
    }

    fn commit(&self) -> Result<gix::Commit<'_>, SourceTreeError> {
        match &self.at {
            Revision::Head => self.repo.head_commit().map_err(unreadable_tree),
            Revision::Pinned(id) => self.repo.find_commit(*id).map_err(unreadable_tree),
        }
    }
}

impl SourceTree for GitSourceTree {
    fn name(&self) -> ProjectName {
        self.name.clone()
    }

    fn files(&self) -> Result<Vec<SourceFile>, SourceTreeError> {
        let tree = self.commit()?.tree().map_err(unreadable_tree)?;

        let mut recorder = gix::traverse::tree::Recorder::default();
        tree.traverse()
            .breadthfirst(&mut recorder)
            .map_err(unreadable_tree)?;

        let mut files = Vec::new();
        for record in recorder.records {
            if !record.mode.is_blob() {
                continue;
            }
            let path = record.filepath.to_str().map_err(|_| {
                unreadable_tree(UnusableGitData::NonUtf8Path {
                    path: record.filepath.to_string(),
                })
            })?;
            let blob = self.repo.find_object(record.oid).map_err(unreadable_tree)?;
            files.push(SourceFile {
                path: SourcePath::new(path)?,
                contents: blob.data.clone(),
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }
}

impl ProjectHistory for GitSourceTree {
    fn recent(&self, limit: NonZeroUsize) -> Result<Vec<Version>, ProjectHistoryError> {
        let head = self.repo.head_id().map_err(|source| {
            unreadable_history(UnusableGitData::NoCommit {
                source: Box::new(source),
            })
        })?;
        let walk = self
            .repo
            .rev_walk(Some(head.detach()))
            .first_parent_only()
            .all()
            .map_err(unreadable_history)?;

        let mut versions = Vec::with_capacity(limit.get());
        for step in walk.take(limit.get()) {
            let step = step.map_err(unreadable_history)?;
            let commit = step.object().map_err(unreadable_history)?;
            let message = commit.message_raw().map_err(unreadable_history)?;
            // A message that is not UTF-8 is refused rather than decoded
            // lossily: a summary the reader cannot trust is worse than none.
            let message = message
                .to_str()
                .map_err(|_| unreadable_history(UnusableGitData::NonUtf8Message { id: step.id }))?;
            versions.push(Version {
                id: version_id(step.id)?,
                summary: message.lines().next().unwrap_or_default().trim().to_owned(),
            });
        }
        Ok(versions)
    }

    fn tree_at(&self, id: &VersionId) -> Result<Box<dyn SourceTree>, ProjectHistoryError> {
        let hash = gix::ObjectId::from_hex(id.as_str().as_bytes())
            .map_err(|_| ProjectHistoryError::UnknownVersion { id: id.clone() })?;
        // A well-formed hash that names nothing in this repository is still an
        // unknown version, so the commit is looked up before it is pinned.
        let commit = self
            .repo
            .find_commit(hash)
            .map_err(|_| ProjectHistoryError::UnknownVersion { id: id.clone() })?;
        // The repository handle is a cheap, shareable view of the same object
        // database, so a pinned tree costs no reopening.
        Ok(Box::new(Self {
            repo: self.repo.clone(),
            name: self.name.clone(),
            at: Revision::Pinned(commit.id),
        }))
    }
}

/// What stopped a read crosses the port whole, as the cause of the port's
/// own refusal: git says which object, which reference or which permission
/// was in the way, and only the last link of the chain says it.
fn unreadable_tree(source: impl Error + Send + Sync + 'static) -> SourceTreeError {
    SourceTreeError::Unreadable {
        source: Box::new(source),
    }
}

fn unreadable_history(source: impl Error + Send + Sync + 'static) -> ProjectHistoryError {
    ProjectHistoryError::Unreadable {
        source: Box::new(source),
    }
}

/// The refusals this adapter makes itself, where git handed it something it
/// cannot pass on. They travel as causes exactly as git's own errors do, so
/// the reader meets one chain whichever end of the adapter stopped.
#[derive(Debug, thiserror::Error)]
enum UnusableGitData {
    #[error("the repository has no commit to start from")]
    NoCommit {
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    #[error("the tree contains a non-UTF-8 path: {path}")]
    NonUtf8Path { path: String },
    #[error("the message of commit {id} is not UTF-8")]
    NonUtf8Message { id: gix::ObjectId },
}

/// A commit hash in full hex is the version's identity: it is stable, and it
/// is what the reader sees in their own log.
fn version_id(commit: gix::ObjectId) -> Result<VersionId, ProjectHistoryError> {
    VersionId::new(commit.to_hex().to_string()).map_err(unreadable_history)
}

#[derive(Debug, thiserror::Error)]
pub enum OpenRepositoryError {
    /// What git says about the path stays with the refusal instead of being
    /// folded into this line: a directory that is no repository and one whose
    /// `.git` cannot be read both fail here, and only the cause tells them
    /// apart.
    #[error("{path} is not a git repository")]
    NotARepository {
        path: PathBuf,
        /// Boxed: what git says about a failed open is far larger than
        /// everything else this enum carries, and every caller of `open`
        /// would pay for it on the way it succeeds.
        #[source]
        source: Box<gix::open::Error>,
    },
    #[error("{path} yields no usable project name")]
    Unnameable { path: PathBuf },
}
