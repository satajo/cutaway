//! The assembler of the whole tree: every file and directory of the sources
//! becomes a node, and what the languages read fuses onto those nodes.
//!
//! The file tree is the containment skeleton of every project, whatever
//! wrote it, so inspection is total by construction rather than by an
//! analyzer claiming honestly: there are no claims to get wrong, only
//! extents, and an extent naming something the tree does not hold fails the
//! inspection loudly.
//!
//! The laws, in order:
//!
//! - An element fuses with a node exactly when it is the sole interpretation
//!   of that whole piece of the tree. One node then carries two readings: the
//!   module `element` and the file `element.rs` are one boundary a reader
//!   addresses. A spanning extent (`foo.rs` beside `foo/`) fuses onto the
//!   directory and dissolves the defining file into it.
//! - Two interpretations of one piece are contested: neither fuses, the piece
//!   stands plain, and both elements stand inside it.
//! - A plain directory earns a node only when it groups two things or more -
//!   the files in it, the fused nodes in it, the surviving directories
//!   beneath it, and whatever the dissolved ones hand up. A single-child
//!   chain dissolves and what it held stands in the directory above it. A
//!   directory an element interprets always survives: an extent is never
//!   dissolved away.
//! - Substrate names read against the surviving directory that holds them, so
//!   what hoists out of a dissolved directory carries the dissolved segments
//!   in its name (`build/one.css`) and two namesakes stay apart. Ids stay
//!   full paths. The language's own name is never prefixed: it names a thing,
//!   not a place.
//! - Every file-backed node carries a fingerprint of the file's contents, so
//!   a comparison reads a content edit in any file at all. A directory
//!   carries none, unless a file dissolved into it: then it holds what that
//!   file holds.
//!
//! The answer depends on the set of files and interpretations alone, never on
//! the order the tree yielded them.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{
    Element, ElementId, ElementName, Fingerprint, Substrate, SubstrateKind,
};

use crate::ports::source_analyzer::{Extent, Interpretation};
use crate::ports::source_tree::{DirectoryPath, SourceFile, SourcePath};

/// One assembled node, with the element holding it. A parentless node
/// attaches to the project root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Placed {
    pub element: Element,
    pub parent: Option<ElementId>,
}

/// An analyzer stated an extent the source tree does not hold. Such a
/// statement is a bug in the analyzer, and inspection fails on it rather than
/// quietly dropping the element.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExtentError {
    #[error("the sources hold no file {path}")]
    NoSuchFile { path: SourcePath },
    #[error("the sources hold no directory {directory}")]
    NoSuchDirectory { directory: DirectoryPath },
}

/// Builds the tree of the sources with the interpretations fused onto it.
pub(crate) fn assemble(
    files: &[SourceFile],
    interpretations: &[Interpretation],
) -> Result<Vec<Placed>, ExtentError> {
    let tree = Tree::of(files);
    tree.validate(interpretations)?;
    let fusion = Fusion::of(interpretations, &Claims::of(interpretations));
    let surviving = surviving_directories(&tree, &fusion);
    let nodes = Nodes::of(&tree, &fusion, &surviving, interpretations);

    let mut placed = Vec::new();
    if let Some(index) = fusion.root {
        placed.push(Placed {
            element: interpretations[index].element.clone(),
            parent: None,
        });
    }
    placed.extend(nodes.directory_nodes(&tree, &fusion, &surviving, interpretations));
    placed.extend(nodes.file_nodes(&tree, &fusion, &surviving, interpretations));
    placed.extend(nodes.uncoincident(&fusion, interpretations));
    Ok(placed)
}

/// The files of the sources and every directory they lie in.
struct Tree<'a> {
    files: BTreeMap<&'a str, &'a SourceFile>,
    /// Every directory holding something, the root (`""`) included.
    directories: BTreeSet<&'a str>,
}

impl<'a> Tree<'a> {
    fn of(files: &'a [SourceFile]) -> Self {
        let mut directories = BTreeSet::from([""]);
        for file in files {
            let mut dir = parent_dir(file.path.as_str());
            while !dir.is_empty() {
                directories.insert(dir);
                dir = parent_dir(dir);
            }
        }
        Self {
            files: files
                .iter()
                .map(|file| (file.path.as_str(), file))
                .collect(),
            directories,
        }
    }

    fn fingerprint(&self, path: &str) -> Option<Fingerprint> {
        self.files
            .get(path)
            .map(|file| Fingerprint::of(&file.contents))
    }

    /// Fails on the first extent naming something the sources do not hold.
    fn validate(&self, interpretations: &[Interpretation]) -> Result<(), ExtentError> {
        let file = |path: &SourcePath| {
            if self.files.contains_key(path.as_str()) {
                Ok(())
            } else {
                Err(ExtentError::NoSuchFile { path: path.clone() })
            }
        };
        let directory = |dir: &DirectoryPath| {
            if self.directories.contains(dir.as_str()) {
                Ok(())
            } else {
                Err(ExtentError::NoSuchDirectory {
                    directory: dir.clone(),
                })
            }
        };
        for interpretation in interpretations {
            match &interpretation.extent {
                Extent::File(path) | Extent::Within { file: path, .. } => file(path)?,
                Extent::Directory(dir) => directory(dir)?,
                Extent::FileAndDirectory {
                    file: path,
                    directory: dir,
                } => {
                    file(path)?;
                    directory(dir)?;
                }
                Extent::Root => {}
            }
        }
        Ok(())
    }
}

/// How many interpretations claim each piece of the tree. A piece two of them
/// claim is contested.
#[derive(Default)]
struct Claims<'a> {
    files: BTreeMap<&'a str, usize>,
    directories: BTreeMap<&'a str, usize>,
    roots: usize,
}

impl<'a> Claims<'a> {
    fn of(interpretations: &'a [Interpretation]) -> Self {
        let mut claims = Self::default();
        for interpretation in interpretations {
            match &interpretation.extent {
                Extent::File(path) => *claims.files.entry(path.as_str()).or_default() += 1,
                Extent::Directory(dir) => {
                    *claims.directories.entry(dir.as_str()).or_default() += 1;
                }
                Extent::FileAndDirectory { file, directory } => {
                    *claims.files.entry(file.as_str()).or_default() += 1;
                    *claims.directories.entry(directory.as_str()).or_default() += 1;
                }
                Extent::Root => claims.roots += 1,
                Extent::Within { .. } => {}
            }
        }
        claims
    }

    fn file_is_sole(&self, path: &SourcePath) -> bool {
        self.files.get(path.as_str()) == Some(&1)
    }

    fn directory_is_sole(&self, dir: &DirectoryPath) -> bool {
        self.directories.get(dir.as_str()) == Some(&1)
    }
}

/// Which interpretation fuses with which piece of the tree, and which pieces
/// stand plain although something interpreted them.
#[derive(Default)]
struct Fusion<'a> {
    /// File path -> the interpretation that becomes that file's node.
    files: BTreeMap<&'a str, usize>,
    /// Directory -> the interpretation that becomes that directory's node.
    directories: BTreeMap<&'a str, usize>,
    /// Directory -> the defining file that dissolved into it.
    spanning: BTreeMap<&'a str, &'a str>,
    /// Defining file -> the directory it dissolved into.
    dissolved: BTreeMap<&'a str, &'a str>,
    /// The interpretation of the repository root, while one stands
    /// uncontested.
    root: Option<usize>,
    /// Directories a contested interpretation names: they survive whatever
    /// they group, so the rivals have a place to stand.
    contested: BTreeSet<&'a str>,
    /// The interpretations that fused with nothing.
    loose: BTreeSet<usize>,
}

impl<'a> Fusion<'a> {
    fn of(interpretations: &'a [Interpretation], claims: &Claims<'_>) -> Self {
        let mut fusion = Self::default();
        for (index, interpretation) in interpretations.iter().enumerate() {
            match &interpretation.extent {
                Extent::File(path) if claims.file_is_sole(path) => {
                    fusion.files.insert(path.as_str(), index);
                }
                Extent::Directory(dir) if claims.directory_is_sole(dir) => {
                    fusion.directories.insert(dir.as_str(), index);
                }
                Extent::FileAndDirectory { file, directory }
                    if claims.file_is_sole(file) && claims.directory_is_sole(directory) =>
                {
                    fusion.directories.insert(directory.as_str(), index);
                    fusion.spanning.insert(directory.as_str(), file.as_str());
                    fusion.dissolved.insert(file.as_str(), directory.as_str());
                }
                Extent::Root if claims.roots == 1 => fusion.root = Some(index),
                Extent::Within { .. } => {}
                rival => {
                    if let Extent::Directory(dir)
                    | Extent::FileAndDirectory { directory: dir, .. } = rival
                    {
                        fusion.contested.insert(dir.as_str());
                    }
                    fusion.loose.insert(index);
                }
            }
        }
        fusion
    }

    /// Whether the directory stands whatever it groups.
    fn pins(&self, dir: &str) -> bool {
        self.directories.contains_key(dir) || self.contested.contains(dir)
    }
}

/// The directories that earn a node: the ones an interpretation pins, and the
/// plain ones grouping two things or more. A directory's children are the
/// file nodes directly in it, one per surviving directory beneath it, and
/// whatever the dissolved ones beneath it hand up - a dissolved directory is
/// not there any more, so what it held stands in the directory above it and
/// counts for it. The deepest directories therefore settle first.
fn surviving_directories<'a>(tree: &Tree<'a>, fusion: &Fusion<'_>) -> BTreeSet<&'a str> {
    let mut files_in: BTreeMap<&str, usize> = BTreeMap::new();
    for path in tree.files.keys() {
        if fusion.dissolved.contains_key(path) {
            continue;
        }
        *files_in.entry(parent_dir(path)).or_default() += 1;
    }

    let mut ordered: Vec<&'a str> = tree.directories.iter().copied().collect();
    ordered.sort_by_key(|dir| std::cmp::Reverse(depth(dir)));
    let mut surviving = BTreeSet::new();
    let mut from_below: BTreeMap<&str, usize> = BTreeMap::new();
    for dir in ordered {
        if dir.is_empty() {
            continue;
        }
        let children = files_in.get(dir).copied().unwrap_or_default()
            + from_below.get(dir).copied().unwrap_or_default();
        let contribution = if fusion.pins(dir) || children >= 2 {
            surviving.insert(dir);
            1
        } else {
            children
        };
        *from_below.entry(parent_dir(dir)).or_default() += contribution;
    }
    surviving
}

/// The node every piece of the tree became, so that whatever stands inside a
/// piece can name it.
struct Nodes {
    directories: BTreeMap<String, ElementId>,
    /// Every file of the tree, a dissolved one included: it answers with the
    /// node it dissolved into, which is where its declarations belong.
    files: BTreeMap<String, ElementId>,
    /// What the repository root became, and None while the project root
    /// itself holds what lies there.
    root: Option<ElementId>,
}

impl Nodes {
    fn of(
        tree: &Tree<'_>,
        fusion: &Fusion<'_>,
        surviving: &BTreeSet<&str>,
        interpretations: &[Interpretation],
    ) -> Self {
        let id_of = |index: usize| interpretations[index].element.id.clone();
        let directories: BTreeMap<String, ElementId> = surviving
            .iter()
            .map(|dir| {
                let id = fusion.directories.get(dir).map_or_else(
                    || ElementId::new(*dir).expect("a directory below the root is never empty"),
                    |index| id_of(*index),
                );
                ((*dir).to_owned(), id)
            })
            .collect();
        let files = tree
            .files
            .keys()
            .map(|path| {
                let id = match (fusion.dissolved.get(path), fusion.files.get(path)) {
                    (Some(directory), _) => directories[*directory].clone(),
                    (None, Some(index)) => id_of(*index),
                    (None, None) => ElementId::new(*path).expect("a source path is never empty"),
                };
                ((*path).to_owned(), id)
            })
            .collect();
        Self {
            directories,
            files,
            root: fusion.root.map(id_of),
        }
    }

    /// The node holding whatever lies at `path`: the nearest surviving
    /// directory above it, else what stands for the repository root.
    fn holder(&self, path: &str, surviving: &BTreeSet<&str>) -> Option<ElementId> {
        let against = read_against(path, surviving);
        if against.is_empty() {
            self.root.clone()
        } else {
            Some(self.directories[against].clone())
        }
    }

    fn directory_nodes(
        &self,
        tree: &Tree<'_>,
        fusion: &Fusion<'_>,
        surviving: &BTreeSet<&str>,
        interpretations: &[Interpretation],
    ) -> Vec<Placed> {
        surviving
            .iter()
            .map(|dir| {
                let name = ElementName::new(strip_dir(dir, read_against(dir, surviving)))
                    .expect("a directory name below its holder is never empty");
                // A directory holds contents of its own only where a file
                // dissolved into it: what a reader would edit to change that
                // boundary is that file.
                let fingerprint = fusion
                    .spanning
                    .get(dir)
                    .and_then(|file| tree.fingerprint(file));
                let id = self.directories[*dir].clone();
                let element = match fusion.directories.get(dir) {
                    Some(index) => interpretations[*index].element.clone().with_substrate(
                        Substrate {
                            kind: SubstrateKind::Directory,
                            name,
                        },
                        fingerprint,
                    ),
                    None => Element::substrate(id, SubstrateKind::Directory, name, None),
                };
                Placed {
                    element,
                    parent: self.holder(dir, surviving),
                }
            })
            .collect()
    }

    fn file_nodes(
        &self,
        tree: &Tree<'_>,
        fusion: &Fusion<'_>,
        surviving: &BTreeSet<&str>,
        interpretations: &[Interpretation],
    ) -> Vec<Placed> {
        tree.files
            .keys()
            .filter(|path| !fusion.dissolved.contains_key(*path))
            .map(|path| {
                let name = ElementName::new(strip_dir(path, read_against(path, surviving)))
                    .expect("a file name below its holder is never empty");
                let fingerprint = tree.fingerprint(path);
                let id = self.files[*path].clone();
                let element = match fusion.files.get(path) {
                    Some(index) => interpretations[*index].element.clone().with_substrate(
                        Substrate {
                            kind: SubstrateKind::File,
                            name,
                        },
                        fingerprint,
                    ),
                    None => Element::substrate(id, SubstrateKind::File, name, fingerprint),
                };
                Placed {
                    element,
                    parent: self.holder(path, surviving),
                }
            })
            .collect()
    }

    /// The elements that coincide with no whole piece of the tree: what a
    /// span inside a file declares, and the rivals of a contested piece.
    fn uncoincident(&self, fusion: &Fusion<'_>, interpretations: &[Interpretation]) -> Vec<Placed> {
        interpretations
            .iter()
            .enumerate()
            .filter(|(index, interpretation)| {
                fusion.loose.contains(index)
                    || matches!(interpretation.extent, Extent::Within { .. })
            })
            .map(|(_, interpretation)| Placed {
                element: interpretation.element.clone(),
                parent: self.inside(&interpretation.extent),
            })
            .collect()
    }

    /// The node an element stands in when it fused with none.
    fn inside(&self, extent: &Extent) -> Option<ElementId> {
        match extent {
            Extent::File(path) => Some(self.files[path.as_str()].clone()),
            // A contested spanning extent speaks about a directory and the
            // file beside it at once, and the directory holds everything the
            // extent covers, so the rivals stand there.
            Extent::Directory(dir) | Extent::FileAndDirectory { directory: dir, .. } => {
                Some(self.directories[dir.as_str()].clone())
            }
            // Two readings of the whole repository leave the project root
            // holding what lies there, and the rivals stand beside each other.
            Extent::Root => None,
            Extent::Within { file, parent } => parent
                .clone()
                .or_else(|| Some(self.files[file.as_str()].clone())),
        }
    }
}

/// The directory a name reads against: the surviving directory that holds the
/// path, and the root while none does. What the holder already spells stays
/// out of the name drawn inside it.
fn read_against<'a>(path: &str, surviving: &BTreeSet<&'a str>) -> &'a str {
    let mut current = parent_dir(path);
    while !current.is_empty() {
        if let Some(found) = surviving.get(current) {
            return found;
        }
        current = parent_dir(current);
    }
    ""
}

fn depth(path: &str) -> usize {
    path.split('/').count()
}

fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(head, _)| head)
}

fn strip_dir<'a>(path: &'a str, dir: &str) -> &'a str {
    if dir.is_empty() {
        path
    } else {
        path.strip_prefix(dir)
            .map_or(path, |rest| rest.strip_prefix('/').unwrap_or(rest))
    }
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{ElementKind, SemanticKind};

    use super::*;

    fn file(path: &str, contents: &str) -> SourceFile {
        SourceFile {
            path: SourcePath::new(path).unwrap(),
            contents: contents.as_bytes().to_vec(),
        }
    }

    fn path(path: &str) -> SourcePath {
        SourcePath::new(path).unwrap()
    }

    fn directory(path: &str) -> DirectoryPath {
        DirectoryPath::new(path).unwrap()
    }

    fn id(id: &str) -> ElementId {
        ElementId::new(id).unwrap()
    }

    fn read(id: &str, kind: SemanticKind, name: &str, extent: Extent) -> Interpretation {
        Interpretation {
            element: Element::semantic(
                ElementId::new(id).unwrap(),
                kind,
                ElementName::new(name).unwrap(),
            ),
            extent,
        }
    }

    fn assembled(files: &[SourceFile], interpretations: &[Interpretation]) -> Vec<Placed> {
        assemble(files, interpretations).expect("the extents lie within the tree")
    }

    fn find<'a>(placed: &'a [Placed], id: &str) -> &'a Placed {
        placed
            .iter()
            .find(|node| node.element.id.as_str() == id)
            .unwrap_or_else(|| panic!("no node {id} among {:?}", ids(placed)))
    }

    fn ids(placed: &[Placed]) -> Vec<&str> {
        placed.iter().map(|node| node.element.id.as_str()).collect()
    }

    #[test]
    fn every_file_of_the_tree_stands_in_the_assembly() {
        let placed = assembled(
            &[
                file("README.md", "hello"),
                file("src/main.rs", "fn main(){}"),
            ],
            &[],
        );

        assert_eq!(ids(&placed), vec!["README.md", "src/main.rs"]);
        assert_eq!(find(&placed, "README.md").parent, None);
    }

    #[test]
    fn a_file_no_language_read_carries_a_fingerprint_of_its_contents() {
        let fingerprint = |contents: &str| {
            assembled(&[file("notes.txt", contents)], &[])[0]
                .element
                .fingerprint
        };

        assert_eq!(fingerprint("same"), Some(Fingerprint::of(b"same")));
        assert_ne!(fingerprint("old"), fingerprint("new"));
    }

    #[test]
    fn an_element_reading_a_whole_file_becomes_that_file() {
        let placed = assembled(
            &[
                file("src/element.rs", "pub struct Element;"),
                file("src/graph.rs", ""),
            ],
            &[read(
                "src/element.rs",
                SemanticKind::Module,
                "element",
                Extent::File(path("src/element.rs")),
            )],
        );

        let fused = find(&placed, "src/element.rs");
        assert_eq!(
            fused.element.kinds().collect::<Vec<_>>(),
            vec![ElementKind::Module, ElementKind::File],
            "one node carries both readings"
        );
        assert_eq!(fused.element.primary_name().as_str(), "element");
        assert_eq!(
            fused
                .element
                .substrate_aspect()
                .map(|aspect| aspect.name.as_str()),
            Some("element.rs")
        );
        assert_eq!(
            fused.element.fingerprint,
            Some(Fingerprint::of(b"pub struct Element;")),
            "the tree states the contents of what fuses with it"
        );
        assert_eq!(
            ids(&placed),
            vec!["src", "src/element.rs", "src/graph.rs"],
            "the file leaves no second node behind"
        );
    }

    #[test]
    fn an_element_reading_a_whole_directory_becomes_that_directory() {
        let placed = assembled(
            &[
                file("crates/app/Cargo.toml", "[package]"),
                file("crates/app/src/lib.rs", ""),
            ],
            &[read(
                "package:app",
                SemanticKind::Package,
                "app",
                Extent::Directory(directory("crates/app")),
            )],
        );

        let package = find(&placed, "package:app");
        assert_eq!(
            package.element.kinds().collect::<Vec<_>>(),
            vec![ElementKind::Package, ElementKind::Directory]
        );
        assert_eq!(
            package
                .element
                .substrate_aspect()
                .map(|aspect| aspect.name.as_str()),
            Some("crates/app"),
            "the directory reads against the root, the chain above it holding nothing else"
        );
        assert_eq!(
            package.element.fingerprint, None,
            "a directory holds no text"
        );
        assert_eq!(
            find(&placed, "crates/app/Cargo.toml").parent,
            Some(id("package:app"))
        );
    }

    #[test]
    fn a_module_spanning_a_file_and_its_directory_is_one_node() {
        let placed = assembled(
            &[
                file("src/ports/mod.rs", "pub mod plan;"),
                file("src/ports/plan.rs", ""),
            ],
            &[read(
                "src/ports/mod.rs",
                SemanticKind::Module,
                "ports",
                Extent::FileAndDirectory {
                    file: path("src/ports/mod.rs"),
                    directory: directory("src/ports"),
                },
            )],
        );

        let ports = find(&placed, "src/ports/mod.rs");
        assert_eq!(
            ports
                .element
                .substrate_aspect()
                .map(|aspect| aspect.name.as_str()),
            Some("src/ports"),
            "the node presents as the directory it spans"
        );
        assert_eq!(
            ports.element.fingerprint,
            Some(Fingerprint::of(b"pub mod plan;")),
            "the defining file states what the spanning node holds"
        );
        assert!(
            !ids(&placed).contains(&"src/ports"),
            "the directory leaves no node of its own behind"
        );
        assert_eq!(
            find(&placed, "src/ports/plan.rs").parent,
            Some(id("src/ports/mod.rs")),
            "what the directory holds stands inside the spanning node"
        );
    }

    #[test]
    fn a_directory_grouping_two_things_earns_a_node() {
        let placed = assembled(
            &[file("docs/guide.md", "a"), file("docs/setup.md", "b")],
            &[],
        );

        let docs = find(&placed, "docs");
        assert_eq!(docs.element.primary_kind(), ElementKind::Directory);
        assert_eq!(docs.element.fingerprint, None);
        assert_eq!(docs.parent, None);
        assert_eq!(find(&placed, "docs/guide.md").parent, Some(id("docs")));
    }

    #[test]
    fn a_single_child_chain_of_directories_dissolves_into_the_name_of_what_it_held() {
        let placed = assembled(
            &[
                file("docs/build/one.css", "a"),
                file("docs/other/one.css", "b"),
            ],
            &[],
        );

        assert!(
            !ids(&placed).contains(&"docs/build"),
            "a directory holding one thing groups nothing"
        );
        let hoisted = find(&placed, "docs/build/one.css");
        assert_eq!(hoisted.element.primary_name().as_str(), "build/one.css");
        assert_eq!(hoisted.parent, Some(id("docs")));
        assert_eq!(
            find(&placed, "docs/other/one.css")
                .element
                .primary_name()
                .as_str(),
            "other/one.css",
            "the dissolved segments keep two namesakes apart"
        );
    }

    #[test]
    fn a_directory_counts_a_surviving_subdirectory_among_its_children() {
        let placed = assembled(
            &[
                file("src/util.ts", "a"),
                file("src/widgets/panel.ts", "b"),
                file("src/widgets/button.ts", "c"),
            ],
            &[],
        );

        assert_eq!(
            find(&placed, "src").parent,
            None,
            "one file and one surviving subdirectory are two children"
        );
        assert_eq!(find(&placed, "src/widgets").parent, Some(id("src")));
        assert_eq!(find(&placed, "src/util.ts").parent, Some(id("src")));
        assert_eq!(
            find(&placed, "src/widgets/panel.ts").parent,
            Some(id("src/widgets"))
        );
    }

    #[test]
    fn a_directory_counts_what_its_dissolved_subdirectories_hand_up() {
        let placed = assembled(
            &[
                file("src/plugins/index.ts", "a"),
                file("src/plugins/cleanup/cleanup.ts", "b"),
            ],
            &[],
        );

        assert!(
            !ids(&placed).contains(&"src/plugins/cleanup"),
            "a directory of one file dissolves"
        );
        assert_eq!(
            find(&placed, "src/plugins").parent,
            None,
            "the dissolved subdirectory leaves its file standing in the directory above"
        );
        assert_eq!(
            find(&placed, "src/plugins/cleanup/cleanup.ts").parent,
            Some(id("src/plugins"))
        );
    }

    #[test]
    fn a_lone_deep_file_keeps_its_location_in_its_name() {
        let placed = assembled(&[file("docs/guide/intro.md", "a")], &[]);

        let intro = find(&placed, "docs/guide/intro.md");
        assert_eq!(intro.element.primary_name().as_str(), "docs/guide/intro.md");
        assert_eq!(intro.parent, None);
    }

    #[test]
    fn a_directory_an_element_reads_stands_however_little_it_groups() {
        let placed = assembled(
            &[file("crates/app/Cargo.toml", "[package]")],
            &[read(
                "package:app",
                SemanticKind::Package,
                "app",
                Extent::Directory(directory("crates/app")),
            )],
        );

        assert_eq!(
            find(&placed, "crates/app/Cargo.toml").parent,
            Some(id("package:app")),
            "an extent is never dissolved by the grouping law"
        );
    }

    #[test]
    fn what_a_language_calls_a_thing_survives_the_dissolution_around_it() {
        let placed = assembled(
            &[file("crates/app/Cargo.toml", "[package]")],
            &[read(
                "package:app",
                SemanticKind::Package,
                "app",
                Extent::Directory(directory("crates/app")),
            )],
        );

        let package = find(&placed, "package:app");
        assert_eq!(package.element.primary_name().as_str(), "app");
        assert_eq!(
            package
                .element
                .substrate_aspect()
                .map(|aspect| aspect.name.as_str()),
            Some("crates/app"),
            "only the tree's name carries the dissolved segments"
        );
    }

    #[test]
    fn an_element_reading_the_repository_root_holds_everything_at_the_root() {
        let placed = assembled(
            &[file("Cargo.toml", "[package]"), file("README.md", "hi")],
            &[read(
                "package:app",
                SemanticKind::Package,
                "app",
                Extent::Root,
            )],
        );

        let package = find(&placed, "package:app");
        assert_eq!(package.parent, None);
        assert_eq!(
            package.element.substrate_aspect(),
            None,
            "the root carries no name of its own to fuse with"
        );
        assert_eq!(
            find(&placed, "README.md").parent,
            Some(id("package:app")),
            "a root manifest makes the whole repository the package's territory"
        );
    }

    #[test]
    fn a_declaration_stands_in_the_file_that_writes_it() {
        let placed = assembled(
            &[file("src/element.rs", "pub fn go() {}")],
            &[
                read(
                    "src/element.rs",
                    SemanticKind::Module,
                    "element",
                    Extent::File(path("src/element.rs")),
                ),
                read(
                    "src/element.rs#function:go",
                    SemanticKind::Function,
                    "go",
                    Extent::Within {
                        file: path("src/element.rs"),
                        parent: None,
                    },
                ),
            ],
        );

        assert_eq!(
            find(&placed, "src/element.rs#function:go").parent,
            Some(id("src/element.rs"))
        );
    }

    #[test]
    fn a_declaration_of_a_file_no_language_read_stands_in_that_plain_file() {
        let placed = assembled(
            &[file("main.go", "func main() {}")],
            &[read(
                "main.go#function:main",
                SemanticKind::Function,
                "main",
                Extent::Within {
                    file: path("main.go"),
                    parent: None,
                },
            )],
        );

        assert_eq!(
            find(&placed, "main.go#function:main").parent,
            Some(id("main.go"))
        );
    }

    #[test]
    fn a_declaration_of_a_dissolved_file_stands_in_the_node_that_absorbed_it() {
        let placed = assembled(
            &[
                file("src/ports/mod.rs", "pub fn go() {}"),
                file("src/ports/plan.rs", ""),
            ],
            &[
                read(
                    "src/ports/mod.rs",
                    SemanticKind::Module,
                    "ports",
                    Extent::FileAndDirectory {
                        file: path("src/ports/mod.rs"),
                        directory: directory("src/ports"),
                    },
                ),
                read(
                    "src/ports/mod.rs#function:go",
                    SemanticKind::Function,
                    "go",
                    Extent::Within {
                        file: path("src/ports/mod.rs"),
                        parent: None,
                    },
                ),
            ],
        );

        assert_eq!(
            find(&placed, "src/ports/mod.rs#function:go").parent,
            Some(id("src/ports/mod.rs"))
        );
    }

    #[test]
    fn a_declaration_named_by_another_stands_inside_it() {
        let placed = assembled(
            &[file("src/config.rs", "pub struct Config;")],
            &[
                read(
                    "src/config.rs#type:Config",
                    SemanticKind::Type,
                    "Config",
                    Extent::Within {
                        file: path("src/config.rs"),
                        parent: None,
                    },
                ),
                read(
                    "src/config.rs#function:Config::new",
                    SemanticKind::Function,
                    "new",
                    Extent::Within {
                        file: path("src/config.rs"),
                        parent: Some(id("src/config.rs#type:Config")),
                    },
                ),
            ],
        );

        assert_eq!(
            find(&placed, "src/config.rs#function:Config::new").parent,
            Some(id("src/config.rs#type:Config"))
        );
    }

    #[test]
    fn two_readings_of_one_directory_leave_it_plain_and_stand_inside_it() {
        let placed = assembled(
            &[file("app/main.ts", "")],
            &[
                read(
                    "package:one",
                    SemanticKind::Package,
                    "one",
                    Extent::Directory(directory("app")),
                ),
                read(
                    "package:two",
                    SemanticKind::Package,
                    "two",
                    Extent::Directory(directory("app")),
                ),
            ],
        );

        let app = find(&placed, "app");
        assert_eq!(app.element.primary_kind(), ElementKind::Directory);
        assert_eq!(find(&placed, "package:one").parent, Some(id("app")));
        assert_eq!(find(&placed, "package:two").parent, Some(id("app")));
        assert_eq!(
            find(&placed, "app/main.ts").parent,
            Some(id("app")),
            "the contested directory holds its contents itself"
        );
    }

    #[test]
    fn two_readings_of_one_file_leave_it_plain_and_stand_inside_it() {
        let placed = assembled(
            &[file("app/main.ts", "")],
            &[
                read(
                    "module:one",
                    SemanticKind::Module,
                    "one",
                    Extent::File(path("app/main.ts")),
                ),
                read(
                    "module:two",
                    SemanticKind::Module,
                    "two",
                    Extent::File(path("app/main.ts")),
                ),
            ],
        );

        assert_eq!(
            find(&placed, "app/main.ts").element.primary_kind(),
            ElementKind::File
        );
        assert_eq!(find(&placed, "module:one").parent, Some(id("app/main.ts")));
        assert_eq!(find(&placed, "module:two").parent, Some(id("app/main.ts")));
    }

    #[test]
    fn two_readings_of_the_repository_root_leave_the_project_holding_it() {
        let placed = assembled(
            &[file("main.ts", "")],
            &[
                read("package:one", SemanticKind::Package, "one", Extent::Root),
                read("package:two", SemanticKind::Package, "two", Extent::Root),
            ],
        );

        assert_eq!(find(&placed, "package:one").parent, None);
        assert_eq!(find(&placed, "package:two").parent, None);
        assert_eq!(find(&placed, "main.ts").parent, None);
    }

    #[test]
    fn an_extent_naming_a_file_the_sources_do_not_hold_fails_the_assembly() {
        assert_eq!(
            assemble(
                &[file("README.md", "")],
                &[read(
                    "src/lib.rs",
                    SemanticKind::Module,
                    "lib",
                    Extent::File(path("src/lib.rs"))
                )]
            ),
            Err(ExtentError::NoSuchFile {
                path: path("src/lib.rs")
            })
        );
    }

    #[test]
    fn an_extent_naming_a_directory_the_sources_do_not_hold_fails_the_assembly() {
        assert_eq!(
            assemble(
                &[file("README.md", "")],
                &[read(
                    "package:app",
                    SemanticKind::Package,
                    "app",
                    Extent::Directory(directory("crates/app"))
                )]
            ),
            Err(ExtentError::NoSuchDirectory {
                directory: directory("crates/app")
            })
        );
    }

    #[test]
    fn the_repository_root_reads_as_the_root_and_not_as_a_directory() {
        let placed = assembled(
            &[file("README.md", "")],
            &[read(
                "package:app",
                SemanticKind::Package,
                "app",
                Extent::directory(DirectoryPath::root()),
            )],
        );

        assert_eq!(find(&placed, "package:app").parent, None);
        assert_eq!(
            find(&placed, "README.md").parent,
            Some(id("package:app")),
            "the root is no directory to fuse with, so the whole tree stands inside the element"
        );
    }

    #[test]
    fn the_order_the_tree_yields_its_files_changes_nothing() {
        let files = [
            file("docs/guide.md", "a"),
            file("docs/setup.md", "b"),
            file("README.md", "c"),
        ];
        let mut reversed = files.to_vec();
        reversed.reverse();

        let mut forward = assembled(&files, &[]);
        let mut backward = assembled(&reversed, &[]);
        forward.sort_by(|a, b| a.element.id.cmp(&b.element.id));
        backward.sort_by(|a, b| a.element.id.cmp(&b.element.id));
        assert_eq!(forward, backward);
    }
}
