use std::fmt;
use std::num::NonZeroUsize;

use super::source_tree::SourceTree;

/// Driven port: which versions of a project exist, and the sources it holds at
/// each of them.
///
/// [`SourceTree`] answers "what does one version look like". This port answers
/// "which versions are there, and give me that one" - the two questions a
/// comparison of two versions asks. What a version is belongs to the adapter:
/// a git commit, a release tag, a snapshot directory. The core only ever sees
/// an opaque [`VersionId`] and a human-readable summary.
pub trait ProjectHistory {
    /// The most recent versions, newest first, at most `limit` of them.
    ///
    /// The walk follows first parents only. The first-parent chain is the
    /// mainline story of the project: it is the sequence of versions the
    /// reader recognises from their own log, with the side branches that
    /// merges pulled in left out. A merged branch appears as the single merge
    /// version that landed it, not as the commits it was built from.
    ///
    /// A project with fewer than `limit` versions answers with all of them.
    fn recent(&self, limit: NonZeroUsize) -> Result<Vec<Version>, ProjectHistoryError>;

    /// The sources exactly as the version `id` holds them.
    ///
    /// An id that names no version of this project is refused with
    /// [`ProjectHistoryError::UnknownVersion`].
    fn tree_at(&self, id: &VersionId) -> Result<Box<dyn SourceTree>, ProjectHistoryError>;
}

/// One version of a project, as the history lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub id: VersionId,
    /// The one-line human description of the version, for a git commit the
    /// first line of its message. A version recorded without a description
    /// carries an empty summary: the id still identifies it, so an empty
    /// summary is a fact about the version, not a failure to read it.
    pub summary: String,
}

/// The identity of one version of a project: never empty.
///
/// The string is opaque to the core. Only the adapter that produced it knows
/// how to resolve it, so ids from one adapter never travel to another.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VersionId(String);

impl VersionId {
    pub fn new(id: impl Into<String>) -> Result<Self, InvalidVersionId> {
        let id = id.into();
        if id.is_empty() {
            return Err(InvalidVersionId::Empty);
        }
        Ok(Self(id))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VersionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum InvalidVersionId {
    #[error("a version id must not be empty")]
    Empty,
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectHistoryError {
    /// The history could not be read. What stopped it is whatever the
    /// adapter holding the versions says, carried along as the cause rather
    /// than flattened into these words: the core names the operation, the
    /// cause names the commit or the reference behind it.
    #[error("cannot read the project history")]
    Unreadable {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("the project has no version {id}")]
    UnknownVersion { id: VersionId },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_id_is_never_empty() {
        assert_eq!(VersionId::new(""), Err(InvalidVersionId::Empty));
    }

    #[test]
    fn a_version_id_reads_back_as_it_was_given() {
        let id = VersionId::new("v1.2.3").unwrap();
        assert_eq!(id.as_str(), "v1.2.3");
        assert_eq!(id.to_string(), "v1.2.3");
    }
}
