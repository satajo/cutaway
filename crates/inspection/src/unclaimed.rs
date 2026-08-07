//! The file tree of the unclaimed sources: every file no analyzer read
//! meaning from still stands in the graph, as a plain file shown where it
//! lies in the directory tree.
//!
//! Each file answers to an anchor: the deepest enclosing directory that is a
//! package territory or already names an element of the graph (a Go
//! directory is a module whose id is the directory path), and the project
//! root when neither exists. Between the anchor and the file the directories
//! follow the same law the languages apply to their own: a directory earns
//! an element only when it groups at least two things - the unclaimed files
//! directly in it, the surviving directories beneath it, and what the
//! dissolved ones beneath it hand up. A single-child chain dissolves and its
//! contents hoist upward. Names - directory and file alike - read relative
//! to the element that holds them while ids stay full paths, so what hoists
//! out of a dissolved directory carries the dissolved segments in its name.
//! The anchor directory itself never becomes an element: it is the anchor.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{
    ArchitectureGraph, Element, ElementId, ElementKind, ElementName, Fingerprint,
};

use crate::ports::source_analyzer::AnalyzedElement;
use crate::ports::source_tree::{DirectoryPath, SourceFile, SourcePath};

/// The elements of every file the analyzers left unclaimed: plain file
/// elements carrying a fingerprint of their contents, grouped into the
/// directories that earn a boundary, each anchored where the territories
/// and the graph say it belongs. A parentless element attaches to the
/// project root, as analyzer elements do.
pub(crate) fn unclaimed_files(
    files: &[SourceFile],
    claimed: &BTreeSet<SourcePath>,
    territories: &BTreeMap<DirectoryPath, ElementId>,
    graph: &ArchitectureGraph,
) -> Vec<AnalyzedElement> {
    // The files of one anchor settle their directories together: only they
    // decide which directories between them and the anchor earn a boundary.
    let mut anchored: BTreeMap<String, (Option<ElementId>, Vec<&SourceFile>)> = BTreeMap::new();
    for file in files {
        if claimed.contains(&file.path) {
            continue;
        }
        let (base, anchor) = anchor_of(&file.path, territories, graph);
        anchored
            .entry(base)
            .or_insert_with(|| (anchor, Vec::new()))
            .1
            .push(file);
    }
    let mut elements = Vec::new();
    for (base, (anchor, files)) in anchored {
        group(&base, anchor.as_ref(), &files, &mut elements);
    }
    elements
}

/// The element a file answers to, with the directory its group reads names
/// against. The deepest territory whose directory contains the file speaks
/// for it, unless an element already standing at an enclosing directory's
/// own path (a Go directory is a module with the directory path as its id)
/// lies deeper still; with neither, the file answers to the project root
/// (`None`).
fn anchor_of(
    path: &SourcePath,
    territories: &BTreeMap<DirectoryPath, ElementId>,
    graph: &ArchitectureGraph,
) -> (String, Option<ElementId>) {
    // Two distinct directories of one length cannot both contain the file,
    // so the deepest containing territory is unique.
    let territory = territories
        .iter()
        .filter(|(dir, _)| dir.contains(path))
        .max_by_key(|(dir, _)| dir.as_str().len());
    let (base, anchor) = territory.map_or((String::new(), None), |(dir, package)| {
        (dir.as_str().to_owned(), Some(package.clone()))
    });
    let mut dir = parent_dir(path.as_str());
    while is_inside(dir, &base) {
        let as_element = ElementId::new(dir).expect("a directory below the anchor is never empty");
        if graph.element(&as_element).is_some() {
            return (dir.to_owned(), Some(as_element));
        }
        dir = parent_dir(dir);
    }
    (base, anchor)
}

/// Turns the unclaimed files of one anchor into elements: the surviving
/// directories first, then the files, each parented into the nearest
/// surviving directory above it, else the anchor.
fn group(
    base: &str,
    anchor: Option<&ElementId>,
    files: &[&SourceFile],
    elements: &mut Vec<AnalyzedElement>,
) {
    let surviving = surviving_directories(base, files);
    for dir in &surviving {
        let holder = holder(dir, base, &surviving);
        let read_against = holder.map_or(base, String::as_str);
        elements.push(AnalyzedElement {
            element: Element {
                id: ElementId::new(dir.as_str())
                    .expect("a directory below the anchor is never empty"),
                name: ElementName::new(strip_dir(dir, read_against))
                    .expect("a directory name below its holder is never empty"),
                kind: ElementKind::Directory,
                fingerprint: None,
            },
            parent: parent_element(holder, anchor),
        });
    }
    for file in files {
        let holder = holder(file.path.as_str(), base, &surviving);
        let read_against = holder.map_or(base, String::as_str);
        elements.push(AnalyzedElement {
            element: Element {
                id: ElementId::new(file.path.as_str()).expect("a source path is never empty"),
                name: ElementName::new(strip_dir(file.path.as_str(), read_against))
                    .expect("a file path below its holder is never empty"),
                kind: ElementKind::File,
                fingerprint: Some(Fingerprint::of(&file.contents)),
            },
            parent: parent_element(holder, anchor),
        });
    }
}

/// The directories between the anchor and the files that group at least two
/// things. A directory's children are the unclaimed files directly in it,
/// one per directory beneath it that survived this same rule, and whatever
/// the dissolved ones beneath it hand up: a dissolved directory is not there
/// any more, so what it held stands in the directory above it and counts for
/// it. The deepest directories therefore settle first.
///
/// The answer depends on the set of file paths alone, never on the order
/// the tree yielded them.
fn surviving_directories(base: &str, files: &[&SourceFile]) -> BTreeSet<String> {
    let mut files_in: BTreeMap<String, usize> = BTreeMap::new();
    let mut candidates: BTreeSet<String> = BTreeSet::new();
    for file in files {
        let dir = parent_dir(file.path.as_str());
        if !is_inside(dir, base) {
            continue;
        }
        *files_in.entry(dir.to_owned()).or_default() += 1;
        let mut current = dir;
        while is_inside(current, base) {
            candidates.insert(current.to_owned());
            current = parent_dir(current);
        }
    }

    let mut ordered: Vec<&String> = candidates.iter().collect();
    ordered.sort_by_key(|dir| std::cmp::Reverse(depth(dir)));
    let mut surviving = BTreeSet::new();
    let mut from_below: BTreeMap<String, usize> = BTreeMap::new();
    for dir in ordered {
        let children = files_in.get(dir).copied().unwrap_or_default()
            + from_below.get(dir).copied().unwrap_or_default();
        let contribution = if children < 2 {
            children
        } else {
            surviving.insert(dir.clone());
            1
        };
        *from_below.entry(parent_dir(dir).to_owned()).or_default() += contribution;
    }
    surviving
}

/// The surviving directory that most closely encloses `path`, searching only
/// strictly inside the anchor directory `base`: the anchor itself is no
/// directory element.
fn holder<'a>(path: &str, base: &str, surviving: &'a BTreeSet<String>) -> Option<&'a String> {
    let mut current = parent_dir(path);
    while is_inside(current, base) {
        if let Some(found) = surviving.get(current) {
            return Some(found);
        }
        current = parent_dir(current);
    }
    None
}

fn parent_element(holder: Option<&String>, anchor: Option<&ElementId>) -> Option<ElementId> {
    holder.map_or_else(
        || anchor.cloned(),
        |dir| Some(ElementId::new(dir.as_str()).expect("a surviving directory is never empty")),
    )
}

/// Whether `path` names something strictly below the directory `base`.
fn is_inside(path: &str, base: &str) -> bool {
    if base.is_empty() {
        !path.is_empty()
    } else {
        path.starts_with(&format!("{base}/"))
    }
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
    use super::*;

    fn file(path: &str, contents: &str) -> SourceFile {
        SourceFile {
            path: SourcePath::new(path).unwrap(),
            contents: contents.as_bytes().to_vec(),
        }
    }

    fn territory(dir: &str, package: &str) -> (DirectoryPath, ElementId) {
        (
            DirectoryPath::new(dir).unwrap(),
            ElementId::new(package).unwrap(),
        )
    }

    fn tree(
        files: &[SourceFile],
        territories: &BTreeMap<DirectoryPath, ElementId>,
        graph: &ArchitectureGraph,
    ) -> Vec<AnalyzedElement> {
        unclaimed_files(files, &BTreeSet::new(), territories, graph)
    }

    fn find<'a>(elements: &'a [AnalyzedElement], id: &str) -> &'a AnalyzedElement {
        elements
            .iter()
            .find(|analyzed| analyzed.element.id.as_str() == id)
            .expect("the element exists")
    }

    #[test]
    fn a_lone_root_file_attaches_to_the_project_root() {
        let elements = tree(
            &[file("README.md", "hello")],
            &BTreeMap::new(),
            &ArchitectureGraph::new(),
        );
        let readme = find(&elements, "README.md");
        assert_eq!(readme.element.kind, ElementKind::File);
        assert_eq!(readme.element.name.as_str(), "README.md");
        assert_eq!(readme.parent, None);
        assert_eq!(elements.len(), 1, "no directory groups a single file");
    }

    #[test]
    fn two_files_in_one_directory_earn_the_directory() {
        let elements = tree(
            &[file("docs/guide.md", "a"), file("docs/setup.md", "b")],
            &BTreeMap::new(),
            &ArchitectureGraph::new(),
        );
        let docs = find(&elements, "docs");
        assert_eq!(docs.element.kind, ElementKind::Directory);
        assert_eq!(docs.element.fingerprint, None);
        assert_eq!(docs.parent, None);
        let docs_id = ElementId::new("docs").unwrap();
        assert_eq!(
            find(&elements, "docs/guide.md").parent,
            Some(docs_id.clone())
        );
        assert_eq!(find(&elements, "docs/setup.md").parent, Some(docs_id));
    }

    #[test]
    fn a_single_child_chain_dissolves_into_one_name() {
        let elements = tree(
            &[
                file("docs/guide/intro.md", "a"),
                file("docs/guide/deep.md", "b"),
            ],
            &BTreeMap::new(),
            &ArchitectureGraph::new(),
        );
        assert!(
            !elements
                .iter()
                .any(|analyzed| analyzed.element.id.as_str() == "docs"),
            "a directory holding one thing groups nothing"
        );
        let guide = find(&elements, "docs/guide");
        assert_eq!(guide.element.name.as_str(), "docs/guide");
        assert_eq!(guide.parent, None);
    }

    #[test]
    fn a_file_in_a_dissolved_directory_carries_the_dissolved_segments_in_its_name() {
        let elements = tree(
            &[
                file("docs/build/one.css", "a"),
                file("docs/other/one.css", "b"),
            ],
            &BTreeMap::new(),
            &ArchitectureGraph::new(),
        );
        let docs = ElementId::new("docs").unwrap();
        let hoisted = find(&elements, "docs/build/one.css");
        assert_eq!(hoisted.element.name.as_str(), "build/one.css");
        assert_eq!(hoisted.parent, Some(docs.clone()));
        assert_eq!(
            find(&elements, "docs/other/one.css").element.name.as_str(),
            "other/one.css",
            "the dissolved segments keep two namesake files apart"
        );
    }

    #[test]
    fn a_lone_deep_file_keeps_its_location_in_its_name() {
        let elements = tree(
            &[file("docs/guide/intro.md", "a")],
            &BTreeMap::new(),
            &ArchitectureGraph::new(),
        );
        let intro = find(&elements, "docs/guide/intro.md");
        assert_eq!(intro.element.name.as_str(), "docs/guide/intro.md");
        assert_eq!(intro.parent, None);
    }

    #[test]
    fn a_file_inside_a_package_territory_parents_into_the_package() {
        let territories = BTreeMap::from([territory("crates/app", "package:app")]);
        let elements = tree(
            &[file("crates/app/notes.txt", "remember")],
            &territories,
            &ArchitectureGraph::new(),
        );
        assert_eq!(
            find(&elements, "crates/app/notes.txt").parent,
            Some(ElementId::new("package:app").unwrap())
        );
    }

    #[test]
    fn the_deepest_of_nested_territories_wins() {
        let territories = BTreeMap::from([
            territory("", "package:outer"),
            territory("sub", "package:inner"),
        ]);
        let elements = tree(
            &[file("sub/notes.txt", "a"), file("top.txt", "b")],
            &territories,
            &ArchitectureGraph::new(),
        );
        assert_eq!(
            find(&elements, "sub/notes.txt").parent,
            Some(ElementId::new("package:inner").unwrap())
        );
        assert_eq!(
            find(&elements, "top.txt").parent,
            Some(ElementId::new("package:outer").unwrap())
        );
    }

    #[test]
    fn the_contents_drive_the_fingerprint() {
        let fingerprint_of = |contents: &str| {
            tree(
                &[file("notes.txt", contents)],
                &BTreeMap::new(),
                &ArchitectureGraph::new(),
            )[0]
            .element
            .fingerprint
        };
        assert_eq!(fingerprint_of("same"), fingerprint_of("same"));
        assert_ne!(fingerprint_of("old"), fingerprint_of("new"));
        assert_eq!(fingerprint_of("same"), Some(Fingerprint::of(b"same")));
    }

    #[test]
    fn a_directory_already_standing_in_the_graph_holds_its_unclaimed_files_itself() {
        let mut graph = ArchitectureGraph::new();
        graph
            .add_element(Element {
                id: ElementId::new("mymod/util").unwrap(),
                name: ElementName::new("util").unwrap(),
                kind: ElementKind::Module,
                fingerprint: None,
            })
            .unwrap();
        let territories = BTreeMap::from([territory("", "package:mymod")]);
        let elements = tree(
            &[
                file("mymod/util/notes.txt", "a"),
                file("mymod/util/todo.txt", "b"),
            ],
            &territories,
            &graph,
        );
        assert_eq!(
            elements.len(),
            2,
            "the existing element must not be created again"
        );
        let module = ElementId::new("mymod/util").unwrap();
        assert_eq!(
            find(&elements, "mymod/util/notes.txt").parent,
            Some(module.clone())
        );
        assert_eq!(find(&elements, "mymod/util/todo.txt").parent, Some(module));
    }

    #[test]
    fn a_claimed_file_contributes_nothing() {
        let claimed = BTreeSet::from([SourcePath::new("src/lib.rs").unwrap()]);
        let elements = unclaimed_files(
            &[file("src/lib.rs", "claimed"), file("notes.txt", "loose")],
            &claimed,
            &BTreeMap::new(),
            &ArchitectureGraph::new(),
        );
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].element.id.as_str(), "notes.txt");
    }
}
