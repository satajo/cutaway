//! The module catalog: which files and directories are modules, which of them
//! dissolve, and where a specifier written in a file leads.
//!
//! A file is a module, and it is the unit of dependency: a specifier names a
//! file, so every import lands on one. Nothing about where a file sits
//! restricts what may reach it - any file may import any other.
//!
//! A directory is therefore pure grouping - the author's organization, with
//! nothing the language reads into it - and it is a boundary of its own only
//! when it groups at least two things: the files directly in it, and the
//! directories directly beneath it that earned a boundary of their own. A
//! group of fewer than two things groups nothing, so such a directory
//! dissolves the way the entry file dissolves into its package - it is no
//! element, and what it held belongs to the nearest directory above it that
//! survived, else to the package. A chain of directories holding one child
//! each compresses away completely, leaving only the group at its end. The
//! package's own directory is never an element: it is the package.
//!
//! A name reads against the element that holds it: a directory beneath a
//! surviving directory carries the segment below it, and a module whose own
//! directories dissolved carries every segment they gave up. The whole path
//! stays in the id, where it identifies; the name only has to tell the
//! boundary apart from its siblings.
//!
//! The entry module is the package's own code, not a module of its own: to
//! every consumer, importing the package by name and the surface of its entry
//! file are one boundary, named by the package. The entry file therefore
//! dissolves into the package - it is no element, its declarations are the
//! package's items, and a specifier that resolves to it lands on the package.
//! A package whose declared entry names a file the repository does not hold
//! (a built artifact, say) simply has no entry: its files stay plain modules
//! and a bare import of it lands on the package.
//!
//! Nothing under a `node_modules` directory belongs to the architecture. The
//! module system itself puts installed third-party code there, so it is
//! foreign material by the language's own rule, and its files are never even
//! read.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{Element, ElementId, ElementKind, ElementName};
use cutaway_inspection::ports::source_tree::{SourceFile, SourcePath};

use crate::declarations::DeclarationIndex;
use crate::manifest::DiscoveredPackage;
use crate::package_id;
use crate::reexports::ReexportTable;

/// The extensions the ecosystem compiles as source, in the order a resolver
/// tries them: TypeScript before JavaScript, because a repository that holds
/// both holds the JavaScript as build output.
pub const SOURCE_EXTENSIONS: [&str; 8] = ["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];

/// One source file, placed in the package structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    path: SourcePath,
    /// The element this file speaks as: its own path, or the package for an
    /// entry file that dissolves into it.
    id: ElementId,
    /// Index into the discovered packages; None for files outside every
    /// package.
    package: Option<usize>,
    /// Human-facing name: the path relative to the element that holds this
    /// one, without its extension. A label reads inside the frame that draws
    /// it, so the segments the frame already spells say nothing twice.
    name: String,
    /// The nearest directory above the file that survived as a boundary; None
    /// when the package (or the project root) holds the file directly.
    enclosing: Option<ElementId>,
    entry: bool,
}

impl Module {
    pub fn id(&self) -> ElementId {
        self.id.clone()
    }

    /// The containing element of this module: the nearest surviving directory
    /// above it, else its package, else nothing (the project root).
    pub fn parent(&self, packages: &[DiscoveredPackage]) -> Option<ElementId> {
        self.enclosing
            .clone()
            .or_else(|| self.package.map(|index| package_id(&packages[index])))
    }

    /// The module element this file contributes, and None for an entry file:
    /// that file dissolves into its package, which is an element already.
    pub fn element(&self) -> Option<Element> {
        if self.entry {
            return None;
        }
        Some(Element {
            id: self.id(),
            name: ElementName::new(&self.name).expect("a module name is never empty"),
            kind: ElementKind::Module,
        })
    }
}

/// One directory that groups enough to be a boundary of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directory {
    /// The directory path verbatim, so a file's id begins with its
    /// directory's id.
    id: ElementId,
    /// Human-facing name: the path relative to the element that holds this
    /// one, as a module's name is.
    name: String,
    package: Option<usize>,
    /// The nearest surviving directory above this one.
    enclosing: Option<ElementId>,
}

impl Directory {
    /// The containing element: the nearest surviving directory above it, else
    /// its package, else nothing (the project root).
    pub fn parent(&self, packages: &[DiscoveredPackage]) -> Option<ElementId> {
        self.enclosing
            .clone()
            .or_else(|| self.package.map(|index| package_id(&packages[index])))
    }

    pub fn element(&self) -> Element {
        Element {
            id: self.id.clone(),
            name: ElementName::new(&self.name).expect("a directory name is never empty"),
            kind: ElementKind::Directory,
        }
    }
}

/// Where a specifier led: the module, when the project holds one, and the
/// element that speaks for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSpecifier {
    pub module: Option<SourcePath>,
    pub id: ElementId,
}

/// What every module offers a specifier that names it: the items it declares
/// and the names it re-exports. Both are indexed across the whole project
/// before any specifier resolves.
#[derive(Clone, Copy)]
pub struct ModuleSurface<'a> {
    pub declarations: &'a DeclarationIndex,
    pub reexports: &'a ReexportTable,
}

/// Where a name led, and whether the answering module claims it. The
/// distinction decides between wildcard re-exports: an `export * from "./m"`
/// is the right forward only when something behind it claims the name, not
/// when resolution merely stopped at `./m`.
struct Resolution {
    id: ElementId,
    claims_name: bool,
}

impl Resolution {
    fn claimed(id: ElementId) -> Self {
        Self {
            id,
            claims_name: true,
        }
    }

    fn stopped_at(module: ElementId) -> Self {
        Self {
            id: module,
            claims_name: false,
        }
    }
}

/// The re-export hops taken so far, as (module, name) pairs.
type FollowedHops = BTreeSet<(SourcePath, String)>;

pub struct ModuleCatalog {
    modules: Vec<Module>,
    by_path: BTreeMap<SourcePath, usize>,
    /// Package index -> the module that dissolves into it.
    entries: BTreeMap<usize, usize>,
    directories: Vec<Directory>,
}

impl ModuleCatalog {
    pub fn build(packages: &[DiscoveredPackage], files: &[SourceFile]) -> Self {
        let mut catalog = Self {
            modules: Vec::new(),
            by_path: BTreeMap::new(),
            entries: BTreeMap::new(),
            directories: Vec::new(),
        };
        for file in files {
            let path = file.path.as_str();
            if !file
                .path
                .extension()
                .is_some_and(|extension| SOURCE_EXTENSIONS.contains(&extension))
            {
                continue;
            }
            if is_vendored(path) {
                continue;
            }
            let package = owning_package(packages, path);
            let index = catalog.modules.len();
            catalog.by_path.insert(file.path.clone(), index);
            catalog.modules.push(Module {
                path: file.path.clone(),
                id: ElementId::new(path).expect("a source path is never empty"),
                package,
                // The name settles below, together with the directory it
                // reads against.
                name: String::new(),
                enclosing: None,
                entry: false,
            });
        }

        // The entry resolves only once every file is cataloged: a manifest
        // field names a file, and only the tree tells whether it exists. A
        // candidate owned by another package is that package's file, not this
        // one's entry.
        for (index, package) in packages.iter().enumerate() {
            let Some(entry) = package.entry_candidates.iter().find_map(|candidate| {
                catalog
                    .file_named(candidate)
                    .filter(|found| catalog.modules[*found].package == Some(index))
            }) else {
                continue;
            };
            catalog.modules[entry].entry = true;
            catalog.modules[entry].id = package_id(package);
            catalog.entries.insert(index, entry);
        }

        // The directories settle only once the entries are known: a file that
        // dissolved into its package groups nothing. The names settle with
        // them: a name reads against the element that holds it, and which
        // element that is depends on which directories survived.
        let surviving = surviving_directories(&catalog.modules, packages);
        for module in &mut catalog.modules {
            let package_base = package_dir(packages, module.package);
            module.enclosing = enclosing(module.path.as_str(), package_base, &surviving);
            let base = read_against(module.enclosing.as_ref(), package_base);
            module.name = without_extension(strip_dir(module.path.as_str(), base)).to_owned();
        }
        catalog.directories = surviving
            .iter()
            .map(|(path, package)| {
                let package_base = package_dir(packages, *package);
                let enclosing = enclosing(path, package_base, &surviving);
                Directory {
                    id: directory_id(path),
                    name: strip_dir(path, read_against(enclosing.as_ref(), package_base))
                        .to_owned(),
                    package: *package,
                    enclosing,
                }
            })
            .collect();
        catalog
    }

    pub fn modules(&self) -> impl Iterator<Item = &Module> {
        self.modules.iter()
    }

    pub fn directories(&self) -> impl Iterator<Item = &Directory> {
        self.directories.iter()
    }

    pub fn module_of(&self, path: &SourcePath) -> Option<&Module> {
        self.by_path.get(path).map(|index| &self.modules[*index])
    }

    /// Resolves one specifier written in the file at `from`. A relative
    /// specifier that names no file of this project, and a bare specifier
    /// naming no package of it, resolve to nothing: an npm dependency and a
    /// runtime builtin are outside the architecture.
    pub fn resolve_specifier(
        &self,
        from: &SourcePath,
        specifier: &str,
        packages: &[DiscoveredPackage],
    ) -> Option<ResolvedSpecifier> {
        if specifier.starts_with('.') {
            let path = normalize(parent_dir(from.as_str()), specifier)?;
            return Some(self.landed(self.file_named(&path)?));
        }
        let (index, remainder) = packages
            .iter()
            .enumerate()
            .filter_map(|(index, package)| {
                if specifier == package.name {
                    Some((index, ""))
                } else {
                    specifier
                        .strip_prefix(&package.name)
                        .and_then(|rest| rest.strip_prefix('/'))
                        .map(|rest| (index, rest))
                }
            })
            .max_by_key(|(index, _)| packages[*index].name.len())?;
        if remainder.is_empty() {
            return Some(self.entry_of(index, packages));
        }
        // A subpath naming no file this version holds still witnesses the
        // coupling to the package itself.
        Some(
            self.file_named(&join(&packages[index].dir, remainder))
                .map_or_else(
                    || ResolvedSpecifier {
                        module: None,
                        id: package_id(&packages[index]),
                    },
                    |found| self.landed(found),
                ),
        )
    }

    /// Resolves one name taken from a resolved specifier onto the element
    /// that answers for it: the item when the target's surface exports it,
    /// the target itself when the name is internal or unknown.
    pub fn resolve_name(
        &self,
        target: &ResolvedSpecifier,
        name: &str,
        packages: &[DiscoveredPackage],
        surface: ModuleSurface<'_>,
    ) -> ElementId {
        let Some(module) = &target.module else {
            return target.id.clone();
        };
        let mut followed = FollowedHops::new();
        self.name_in_module(module, name, packages, surface, &mut followed)
            .id
    }

    fn name_in_module(
        &self,
        path: &SourcePath,
        name: &str,
        packages: &[DiscoveredPackage],
        surface: ModuleSurface<'_>,
        followed: &mut FollowedHops,
    ) -> Resolution {
        let module = self
            .module_of(path)
            .expect("only cataloged modules are resolved against")
            .id();
        // A default import binds whatever the target exports as its default,
        // which is a local declaration only when the export names one.
        let local = if name == "default" {
            surface.declarations.default_export(path)
        } else {
            Some(name)
        };
        if let Some(local) = local
            && let Some(item) = surface.declarations.declaration(path, local)
        {
            // An unexported declaration is the module's internals: what names
            // it depends on the module.
            return Resolution::claimed(if item.exported {
                item.id.clone()
            } else {
                module
            });
        }
        // Re-exports may point at each other, directly or in a ring. Taking
        // each (module, name) hop at most once along a chain ends such a ring
        // at the module where it closes instead of recursing forever; the hop
        // leaves the set again so that a sibling branch may still take it.
        let hop = (path.clone(), name.to_owned());
        let forwarded = if followed.insert(hop.clone()) {
            let found = self.follow_reexport(path, name, packages, surface, followed);
            followed.remove(&hop);
            found
        } else {
            None
        };
        forwarded.unwrap_or_else(|| Resolution::stopped_at(module))
    }

    /// Continues a name that the module at `path` offers without declaring
    /// it, through the `export ... from` that puts it there. Barrel files are
    /// built this way: an `index.ts` re-exports a directory, and consumers
    /// import the names from the barrel while the declarations live behind
    /// it.
    fn follow_reexport(
        &self,
        path: &SourcePath,
        name: &str,
        packages: &[DiscoveredPackage],
        surface: ModuleSurface<'_>,
        followed: &mut FollowedHops,
    ) -> Option<Resolution> {
        if let Some(forwarded) = surface.reexports.forwarded(path, name)
            && let Some(target) = self.resolve_specifier(path, &forwarded.specifier, packages)
        {
            let id = match &target.module {
                Some(module) => {
                    self.name_in_module(module, &forwarded.name, packages, surface, followed)
                        .id
                }
                None => target.id.clone(),
            };
            // The re-export names `name`, so this module claims it even when
            // the target resolves no further than a module of its own.
            return Some(Resolution::claimed(id));
        }
        // An `export * from "./m"` re-exports whatever `./m` holds, so `name`
        // may hide behind any of them. The first wildcard in source order
        // that leads to something claiming `name` answers.
        for specifier in surface.reexports.wildcards(path) {
            let Some(target) = self.resolve_specifier(path, specifier, packages) else {
                continue;
            };
            let Some(module) = &target.module else {
                continue;
            };
            let resolution = self.name_in_module(module, name, packages, surface, followed);
            if resolution.claims_name {
                return Some(resolution);
            }
        }
        None
    }

    /// Enters a package through the file that dissolved into it, or lands on
    /// the package itself when no entry resolved.
    fn entry_of(&self, package: usize, packages: &[DiscoveredPackage]) -> ResolvedSpecifier {
        self.entries.get(&package).map_or_else(
            || ResolvedSpecifier {
                module: None,
                id: package_id(&packages[package]),
            },
            |index| self.landed(*index),
        )
    }

    fn landed(&self, index: usize) -> ResolvedSpecifier {
        ResolvedSpecifier {
            module: Some(self.modules[index].path.clone()),
            id: self.modules[index].id(),
        }
    }

    /// The module a repository-relative path names, trying the forms a
    /// resolver accepts for it.
    fn file_named(&self, path: &str) -> Option<usize> {
        candidate_paths(path).into_iter().find_map(|candidate| {
            SourcePath::new(candidate)
                .ok()
                .and_then(|path| self.by_path.get(&path).copied())
        })
    }
}

/// The files a path may name, in the order a resolver tries them.
///
/// The path as written comes first. A `.js`, `.mjs`, or `.cjs` suffix is then
/// tried against the TypeScript sources: `NodeNext` TypeScript writes the name
/// of the compiled file (`./x.js`) while the repository holds the source
/// (`./x.ts`), and a `main` field names the built artifact for the same
/// reason. Then the extension-less forms, and finally the directory's index
/// file.
fn candidate_paths(path: &str) -> Vec<String> {
    let mut candidates = vec![path.to_owned()];
    for (compiled, sources) in [
        (".js", ["ts", "tsx"].as_slice()),
        (".mjs", ["mts"].as_slice()),
        (".cjs", ["cts"].as_slice()),
    ] {
        if let Some(stem) = path.strip_suffix(compiled) {
            candidates.extend(sources.iter().map(|source| format!("{stem}.{source}")));
        }
    }
    candidates.extend(
        SOURCE_EXTENSIONS
            .iter()
            .map(|extension| format!("{path}.{extension}")),
    );
    candidates.extend(
        SOURCE_EXTENSIONS
            .iter()
            .map(|extension| format!("{path}/index.{extension}")),
    );
    candidates
}

/// Applies a relative specifier to the directory of the file that writes it.
/// A specifier climbing past the repository root names nothing this project
/// holds, so it resolves to nothing.
fn normalize(base: &str, specifier: &str) -> Option<String> {
    let mut components: Vec<&str> = base.split('/').filter(|part| !part.is_empty()).collect();
    for part in specifier.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            named => components.push(named),
        }
    }
    Some(components.join("/")).filter(|path| !path.is_empty())
}

/// The directories that group at least two things, each with the package it
/// belongs to. A directory's children are the files directly in it, one per
/// directory beneath it that survived this same rule, and whatever the
/// dissolved ones beneath it hand up: a dissolved directory is not there any
/// more, so what it held stands in the directory above it and counts for it.
/// The deepest directories therefore settle first.
///
/// The answer depends on the set of source paths alone, never on the order
/// the tree yielded them.
fn surviving_directories(
    modules: &[Module],
    packages: &[DiscoveredPackage],
) -> BTreeMap<String, Option<usize>> {
    let mut files_in: BTreeMap<String, usize> = BTreeMap::new();
    let mut candidates: BTreeMap<String, Option<usize>> = BTreeMap::new();
    for module in modules {
        // An entry file speaks as its package, so it is not in any directory.
        if module.entry {
            continue;
        }
        let base = package_dir(packages, module.package);
        let mut current = parent_dir(module.path.as_str()).to_owned();
        if !is_inside(&current, base) {
            continue;
        }
        *files_in.entry(current.clone()).or_default() += 1;
        while is_inside(&current, base) {
            let above = parent_dir(&current).to_owned();
            candidates.insert(current, module.package);
            current = above;
        }
    }

    let mut ordered: Vec<&String> = candidates.keys().collect();
    ordered.sort_by_key(|path| std::cmp::Reverse(depth(path)));
    let mut surviving = BTreeMap::new();
    // What the directories beneath each directory contribute to it: one for a
    // surviving one, and a dissolved one's own children, which now stand
    // directly in it.
    let mut from_below: BTreeMap<String, usize> = BTreeMap::new();
    for path in ordered {
        let children = files_in.get(path).copied().unwrap_or_default()
            + from_below.get(path).copied().unwrap_or_default();
        let contribution = if children < 2 {
            children
        } else {
            surviving.insert(path.clone(), candidates[path]);
            1
        };
        *from_below.entry(parent_dir(path).to_owned()).or_default() += contribution;
    }
    surviving
}

/// The surviving directory that most closely encloses `path`, searching only
/// within the package directory `base`: the package itself is no directory
/// module.
fn enclosing(
    path: &str,
    base: &str,
    surviving: &BTreeMap<String, Option<usize>>,
) -> Option<ElementId> {
    let mut current = parent_dir(path).to_owned();
    while is_inside(&current, base) {
        if surviving.contains_key(&current) {
            return Some(directory_id(&current));
        }
        current = parent_dir(&current).to_owned();
    }
    None
}

/// The directory a name reads against: the surviving directory that holds
/// the element, and the package's own directory when no directory does. What
/// the holder already spells stays out of the name it draws inside.
fn read_against<'a>(enclosing: Option<&'a ElementId>, package_base: &'a str) -> &'a str {
    enclosing.map_or(package_base, ElementId::as_str)
}

/// A directory speaks as its own path, the same string every file under it
/// begins with.
fn directory_id(path: &str) -> ElementId {
    ElementId::new(path).expect("a directory path is never empty")
}

/// The directory of a package, and the repository root for what no package
/// owns.
fn package_dir(packages: &[DiscoveredPackage], package: Option<usize>) -> &str {
    package.map_or("", |index| packages[index].dir.as_str())
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

/// Whether the path lies in installed third-party material.
pub fn is_vendored(path: &str) -> bool {
    path.split('/').any(|component| component == "node_modules")
}

/// The package whose directory contains `path`, preferring the most deeply
/// nested one.
fn owning_package(packages: &[DiscoveredPackage], path: &str) -> Option<usize> {
    packages
        .iter()
        .enumerate()
        .filter(|(_, package)| {
            package.dir.is_empty() || path.starts_with(&format!("{}/", package.dir))
        })
        .max_by_key(|(_, package)| package.dir.len())
        .map(|(index, _)| index)
}

fn without_extension(path: &str) -> &str {
    path.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty() && !stem.ends_with('/'))
        .map_or(path, |(stem, _)| stem)
}

fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(head, _)| head)
}

pub fn join(dir: &str, rest: &str) -> String {
    match (dir.is_empty(), rest.is_empty()) {
        (true, _) => rest.to_owned(),
        (_, true) => dir.to_owned(),
        (false, false) => format!("{dir}/{rest}"),
    }
}

fn strip_dir<'a>(path: &'a str, dir: &str) -> &'a str {
    if dir.is_empty() {
        path
    } else {
        path.strip_prefix(dir)
            .map_or(path, |rest| rest.strip_prefix('/').unwrap_or(rest))
    }
}
