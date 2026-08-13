use std::fmt;

/// Driven port: where one version of a project's sources comes from.
///
/// An implementation presents the version as a flat list of files. What
/// "version" means belongs to the adapter: a git tree at a commit, a plain
/// directory, an in-memory fixture.
pub trait SourceTree {
    /// The identity of the project these sources belong to. It roots the
    /// containment hierarchy and must be stable across versions of the same
    /// project.
    fn name(&self) -> ProjectName;

    fn files(&self) -> Result<Vec<SourceFile>, SourceTreeError>;
}

/// The name of an inspected project: never empty.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProjectName(String);

impl ProjectName {
    pub fn new(name: impl Into<String>) -> Result<Self, InvalidProjectName> {
        let name = name.into();
        if name.is_empty() {
            return Err(InvalidProjectName::Empty);
        }
        Ok(Self(name))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProjectName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum InvalidProjectName {
    #[error("a project name must not be empty")]
    Empty,
}

/// One file in a source tree.
///
/// Contents stay raw bytes: whether and how to decode them is a language
/// concern and belongs to the syntax analyzers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    pub path: SourcePath,
    pub contents: Vec<u8>,
}

/// A path inside a source tree: relative, slash-separated, never empty.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourcePath(String);

impl SourcePath {
    pub fn new(path: impl Into<String>) -> Result<Self, InvalidSourcePath> {
        let path = path.into();
        if path.is_empty() {
            return Err(InvalidSourcePath::Empty);
        }
        if path.starts_with('/') {
            return Err(InvalidSourcePath::Absolute { path });
        }
        if path.contains('\\') {
            return Err(InvalidSourcePath::Backslash { path });
        }
        Ok(Self(path))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The extension of the file name, if it has one: `src/lib.rs` -> `rs`.
    #[must_use]
    pub fn extension(&self) -> Option<&str> {
        let file_name = self.0.rsplit('/').next()?;
        let (stem, extension) = file_name.rsplit_once('.')?;
        if stem.is_empty() {
            None
        } else {
            Some(extension)
        }
    }
}

impl fmt::Display for SourcePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum InvalidSourcePath {
    #[error("a source path must not be empty")]
    Empty,
    #[error("a source path must be relative: {path}")]
    Absolute { path: String },
    #[error("a source path must use forward slashes: {path}")]
    Backslash { path: String },
}

/// A directory inside a source tree: relative, slash-separated, without a
/// trailing slash. The empty path names the tree's root - a directory no
/// [`SourcePath`] can name, because a source path always names a file.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DirectoryPath(String);

impl DirectoryPath {
    /// The root of the source tree.
    #[must_use]
    pub fn root() -> Self {
        Self(String::new())
    }

    pub fn new(path: impl Into<String>) -> Result<Self, InvalidDirectoryPath> {
        let path = path.into();
        if path.starts_with('/') {
            return Err(InvalidDirectoryPath::Absolute { path });
        }
        if path.contains('\\') {
            return Err(InvalidDirectoryPath::Backslash { path });
        }
        if path.ends_with('/') {
            return Err(InvalidDirectoryPath::TrailingSlash { path });
        }
        Ok(Self(path))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DirectoryPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum InvalidDirectoryPath {
    #[error("a directory path must be relative: {path}")]
    Absolute { path: String },
    #[error("a directory path must use forward slashes: {path}")]
    Backslash { path: String },
    #[error("a directory path carries no trailing slash: {path}")]
    TrailingSlash { path: String },
}

#[derive(Debug, thiserror::Error)]
pub enum SourceTreeError {
    /// The tree could not be read. What stopped it is whatever the adapter
    /// holding the sources says, carried along as the cause rather than
    /// flattened into these words: the core names the operation, the cause
    /// names the file, the path, or the permission behind it.
    #[error("cannot read the source tree")]
    Unreadable {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("the source tree contains an invalid path")]
    InvalidPath {
        #[from]
        source: InvalidSourcePath,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_path_is_always_relative() {
        assert!(matches!(
            SourcePath::new("/etc/passwd"),
            Err(InvalidSourcePath::Absolute { .. })
        ));
    }

    #[test]
    fn a_source_path_always_uses_forward_slashes() {
        assert!(matches!(
            SourcePath::new("src\\lib.rs"),
            Err(InvalidSourcePath::Backslash { .. })
        ));
    }

    #[test]
    fn a_directory_path_admits_the_root_and_refuses_a_trailing_slash() {
        assert_eq!(DirectoryPath::new(""), Ok(DirectoryPath::root()));
        assert!(matches!(
            DirectoryPath::new("crates/app/"),
            Err(InvalidDirectoryPath::TrailingSlash { .. })
        ));
    }

    #[test]
    fn the_extension_is_the_suffix_of_the_file_name() {
        assert_eq!(
            SourcePath::new("src/lib.rs").unwrap().extension(),
            Some("rs")
        );
        assert_eq!(SourcePath::new("Makefile").unwrap().extension(), None);
        assert_eq!(SourcePath::new("src/.gitignore").unwrap().extension(), None);
    }
}
