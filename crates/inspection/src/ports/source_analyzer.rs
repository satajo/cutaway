use std::fmt;

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
/// An analyzer never states where an element sits. The file tree is the
/// containment skeleton of every project, and an analyzer says only what it
/// read and over which part of the tree it read it - its [`Extent`]. The
/// core builds the tree, fuses each element with the piece of tree it
/// interprets whole, and derives containment from that. What the analyzer
/// leaves uninterpreted stands as a plain directory or file, so inspection
/// is total whatever the analyzers do.
///
/// Contract, enforced by the inspector where possible:
/// - Element ids derive only from source paths, kinds, and names — never from
///   ambient state — so the graphs of two versions of a project align.
///   Established schemes: `project:<name>`, `package:<name>`, `<path>` for a
///   file, `<path>#<kind>:<name>` for a declaration. A declaration nested in
///   another extends the holder's name with the language's own separator
///   (`<path>#function:Config::new`, `<path>#function:Config.load`), so the
///   id stays deterministic however deep declarations nest.
/// - Every element carries the language's reading alone. The core adds the
///   substrate aspect and the fingerprint where an element fuses with the
///   tree, because the tree is the authority on what a place holds.
/// - Extents must lie within the analyzed tree: inspection fails on an
///   extent naming a file or a directory the sources do not hold.
/// - Two interpretations of one extent - from one analyzer or several - are
///   contested: neither fuses, the piece of tree stands plain, and both
///   elements stand inside it. Two elements sharing one id still fail
///   inspection.
/// - Declarations hang where they are written: a top-level declaration's
///   extent is [`Extent::Within`] with no parent, in every language.
/// - Relations may name interpreted elements and the ids of plain substrate
///   nodes, which are paths. The inspector validates every endpoint after
///   assembly.
/// - Analysis never fails. Input an analyzer cannot read is a gap in what it
///   read, declared as such ([`AnalysisGap`]) and never silently dropped:
///   whatever stands around the unreadable part is still read, and the
///   picture says where it is thin.
pub trait SourceAnalyzer {
    fn analyze(&self, files: &[SourceFile]) -> SourceStructure;
}

/// Everything one analyzer read out of a source tree, and everywhere it
/// could not read what stands there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceStructure {
    pub interpretations: Vec<Interpretation>,
    pub relations: Vec<Relation>,
    pub gaps: Vec<AnalysisGap>,
}

/// One place an analyzer could not read the sources, and what it therefore
/// left out of the picture.
///
/// A gap is no failure: the analysis went on around it. It is the analyzer
/// declaring the limit of its own reading, so a reader is never shown a
/// partial picture as a whole one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisGap {
    pub path: SourcePath,
    pub reason: GapReason,
}

/// Why one file yielded less than it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GapReason {
    /// The file is not valid UTF-8; the analyzer read none of it.
    NonUtf8Text,
    /// The file holds syntax the grammar cannot read; everything outside the
    /// unreadable regions was read. The position is that of the first such
    /// region, 1-based, for the reader hunting the construct. The count says
    /// how many regions the grammar marked, which is a floor rather than a
    /// tally: error recovery swallows what follows a break into the same
    /// region, so several broken constructs often come out as one - it
    /// tells a lone break from more of them, never how many there are.
    SyntaxErrors {
        line: usize,
        column: usize,
        regions: usize,
    },
    /// A manifest that could not be understood; the ecosystem structure it
    /// would have declared - the package, and everything standing under it -
    /// is absent from the picture.
    ManifestUnreadable { detail: String },
    /// Two files define the same boundary. Neither reading joins the
    /// picture: emitting both would collide, and picking one would lie about
    /// which of them the language reads.
    ConflictingDefinitions { other: SourcePath },
}

impl fmt::Display for GapReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonUtf8Text => write!(f, "not valid UTF-8, so nothing in it was read"),
            Self::SyntaxErrors {
                line,
                column,
                regions: 1,
            } => write!(
                f,
                "syntax the language cannot read at line {line}, column {column}; \
                 everything around it was read"
            ),
            Self::SyntaxErrors { line, column, .. } => write!(
                f,
                "syntax the language cannot read at line {line}, column {column}, \
                 and further syntax it could not read after that; everything \
                 around them was read"
            ),
            Self::ManifestUnreadable { detail } => {
                write!(f, "a manifest that cannot be read: {detail}")
            }
            Self::ConflictingDefinitions { other } => write!(
                f,
                "the same boundary is defined here and in {other}, so neither reading stands"
            ),
        }
    }
}

/// One element a language read, and the part of the source tree it read it
/// out of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interpretation {
    /// The language's reading alone. The core fuses the substrate aspect and
    /// the fingerprint onto it where the extent covers a whole piece of the
    /// tree.
    pub element: Element,
    pub extent: Extent,
}

/// The part of the source tree one element interprets.
///
/// Fusion follows from coincidence of extent, never from kind: an element
/// that is the sole interpretation of a whole file or directory becomes that
/// node of the tree under two readings at once, while an element covering
/// less than a file stands inside the node holding it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Extent {
    /// The whole file. The element fuses with the file's node: it gains a
    /// File substrate aspect named by the file name, and the file's
    /// fingerprint.
    File(SourcePath),
    /// The whole directory. The element fuses with the directory's node: it
    /// gains a Directory substrate aspect named by the directory. The
    /// repository root is [`Extent::Root`] instead, because the root carries
    /// no name of its own.
    Directory(DirectoryPath),
    /// A file together with the directory that expands it: Rust's `foo.rs`
    /// beside `foo/`, or `foo/` holding `foo/mod.rs`. One node covers both:
    /// the defining file dissolves into it, the substrate aspect is the
    /// directory, and the fingerprint comes from the defining file.
    FileAndDirectory {
        file: SourcePath,
        directory: DirectoryPath,
    },
    /// The repository root itself: a package whose manifest sits at the top
    /// of the tree. The element stands under the project and everything at
    /// the root stands inside it, because a repository whose root manifest
    /// names a package is that package's territory.
    Root,
    /// A span inside a file: an inline module, a type, a function. `parent`
    /// names the enclosing declaration; None means the file's own node,
    /// whatever the core made of it.
    Within {
        file: SourcePath,
        parent: Option<ElementId>,
    },
}

impl Extent {
    /// What a whole directory is an extent of, whichever directory it is:
    /// the repository root reads as [`Extent::Root`], because the root
    /// carries no name of its own to stand under.
    ///
    /// Every analyzer that discovers a package from a manifest asks this,
    /// so no analyzer has to know the root is the exception.
    #[must_use]
    pub fn directory(directory: DirectoryPath) -> Self {
        if directory == DirectoryPath::root() {
            Self::Root
        } else {
            Self::Directory(directory)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lone_break_in_the_syntax_reads_differently_from_several() {
        let at = |regions| {
            GapReason::SyntaxErrors {
                line: 12,
                column: 5,
                regions,
            }
            .to_string()
        };

        let one = at(1);
        assert!(one.contains("line 12, column 5"), "{one}");
        assert!(
            !one.contains("further"),
            "a lone break points at itself and promises nothing beyond it: {one}"
        );
        assert!(
            at(3).contains("further"),
            "several breaks send the reader past the first one: {}",
            at(3)
        );
    }
}
