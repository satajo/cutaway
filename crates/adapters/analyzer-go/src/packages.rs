//! The directory structure of the discovered modules, and import resolution
//! against it.
//!
//! In Go the unit of encapsulation is the directory, not the file: the files
//! of one directory share a namespace, compile together, and are imported
//! together under one path. A directory that directly holds `.go` files is
//! therefore one Go package and one module element; the individual files are
//! no boundary and no element.
//!
//! The module root directory is the module's own code, not a directory of
//! its own: to every importer the module and its root namespace are one
//! boundary, named by the module path. The root directory therefore
//! dissolves into the package - it is no element, its exported declarations
//! are the package's items, its subdirectories are the package's modules,
//! and an import that resolves to it lands on the package.
//!
//! Which files exist at all is the go tool's decision, not this adapter's.
//! The go tool excludes `vendor` and `testdata` directories and every name
//! starting with `.` or `_` from the build, so such files are outside the
//! architecture by the language's own definition.

use std::collections::BTreeMap;

use cutaway_architecture::{Element, ElementId, ElementKind, ElementName};
use cutaway_inspection::ports::source_tree::{SourceFile, SourcePath};

use crate::manifest::DiscoveredModule;
use crate::module_id;

/// One directory of `.go` files, placed in the module structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directory {
    /// The directory, relative to the repository root; `""` for the root.
    path: String,
    /// Index into the discovered modules that owns this directory.
    module: usize,
    /// The element this directory speaks as: its own path, or the package
    /// for a module root that dissolves into it.
    id: ElementId,
    /// Human-facing name: the directory relative to its module's directory,
    /// empty for the module root.
    name: String,
}

impl Directory {
    pub fn id(&self) -> ElementId {
        self.id.clone()
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// The module element this directory contributes, and None for a module
    /// root: that directory dissolves into its package, which is an element
    /// already.
    pub fn element(&self) -> Option<Element> {
        if self.name.is_empty() {
            return None;
        }
        Some(Element {
            id: self.id(),
            name: ElementName::new(&self.name).expect("a non-root directory has a name"),
            kind: ElementKind::Module,
        })
    }
}

/// Where an import path led: the directory, so that a qualified reference
/// can ask what that directory declares, and the element that speaks for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedImport {
    pub directory: String,
    pub id: ElementId,
}

pub struct DirectoryCatalog {
    directories: Vec<Directory>,
    by_path: BTreeMap<String, usize>,
    /// Source path -> directory index, for the files the go tool builds.
    by_file: BTreeMap<SourcePath, usize>,
}

impl DirectoryCatalog {
    pub fn build(modules: &[DiscoveredModule], files: &[SourceFile]) -> Self {
        let mut catalog = Self {
            directories: Vec::new(),
            by_path: BTreeMap::new(),
            by_file: BTreeMap::new(),
        };
        for file in files {
            let path = file.path.as_str();
            if file.path.extension() != Some("go") {
                continue;
            }
            // Files outside every module are outside the build: modules mode
            // knows no GOPATH, so nothing can import them.
            let Some(module) = owning_module(modules, path) else {
                continue;
            };
            if is_outside_the_build(strip_dir(path, &modules[module].dir)) {
                continue;
            }
            let directory = parent_dir(path).to_owned();
            let index = if let Some(index) = catalog.by_path.get(&directory) {
                *index
            } else {
                let name = strip_dir(&directory, &modules[module].dir).to_owned();
                let id = if name.is_empty() {
                    module_id(&modules[module])
                } else {
                    ElementId::new(&directory).expect("a non-root directory path is not empty")
                };
                let index = catalog.directories.len();
                catalog.directories.push(Directory {
                    path: directory.clone(),
                    module,
                    id,
                    name,
                });
                catalog.by_path.insert(directory, index);
                index
            };
            catalog.by_file.insert(file.path.clone(), index);
        }
        catalog
    }

    pub fn directories(&self) -> impl Iterator<Item = &Directory> {
        self.directories.iter()
    }

    /// The directory a file belongs to, and None for every file the go tool
    /// leaves out of the build.
    pub fn directory_of(&self, path: &SourcePath) -> Option<&Directory> {
        self.by_file
            .get(path)
            .map(|index| &self.directories[*index])
    }

    /// The containing element of a directory: the nearest ancestor directory
    /// within the same module that holds `.go` files of its own - the package
    /// itself when that ancestor is the module root - else the package.
    pub fn parent_of(&self, directory: &Directory, modules: &[DiscoveredModule]) -> ElementId {
        let module_dir = modules[directory.module].dir.as_str();
        let mut relative = directory.name.as_str();
        loop {
            relative = relative.rsplit_once('/').map_or("", |(head, _)| head);
            let ancestor = join(module_dir, relative);
            if let Some(found) = self.by_path.get(&ancestor)
                && self.directories[*found].module == directory.module
            {
                return self.directories[*found].id();
            }
            if relative.is_empty() {
                return module_id(&modules[directory.module]);
            }
        }
    }

    /// Resolves one import path to the deepest project directory it leads to.
    /// Paths leaving the project (the standard library, third-party modules)
    /// resolve to nothing.
    ///
    /// An import naming a directory this version of the sources does not hold
    /// stops at the deepest one that exists, so a partly resolved path still
    /// witnesses the coupling it can.
    pub fn resolve(
        &self,
        import_path: &str,
        modules: &[DiscoveredModule],
    ) -> Option<ResolvedImport> {
        let (module, remainder) = modules
            .iter()
            .enumerate()
            .filter_map(|(index, module)| {
                if import_path == module.path {
                    Some((index, ""))
                } else {
                    import_path
                        .strip_prefix(&module.path)
                        .and_then(|rest| rest.strip_prefix('/'))
                        .map(|rest| (index, rest))
                }
            })
            // Nested modules carve their own subtree out, so the longest
            // module path wins.
            .max_by_key(|(index, _)| modules[*index].path.len())?;

        let mut directory = modules[module].dir.clone();
        let mut landed = ResolvedImport {
            directory: directory.clone(),
            id: module_id(&modules[module]),
        };
        if let Some(found) = self.by_path.get(&directory)
            && self.directories[*found].module == module
        {
            landed.id = self.directories[*found].id();
        }
        // The walk crosses directories that hold no `.go` files of their own,
        // because Go groups packages under such directories freely
        // (`internal/`, `pkg/`). Only the deepest directory that is a package
        // answers.
        for segment in remainder.split('/').filter(|s| !s.is_empty()) {
            directory = join(&directory, segment);
            if let Some(found) = self.by_path.get(&directory)
                && self.directories[*found].module == module
            {
                landed = ResolvedImport {
                    directory: directory.clone(),
                    id: self.directories[*found].id(),
                };
            }
        }
        Some(landed)
    }
}

/// The module whose directory contains `path`, preferring the most deeply
/// nested one: a nested go.mod carves its subtree out of the enclosing
/// module.
fn owning_module(modules: &[DiscoveredModule], path: &str) -> Option<usize> {
    modules
        .iter()
        .enumerate()
        .filter(|(_, module)| {
            module.dir.is_empty() || path.starts_with(&format!("{}/", module.dir))
        })
        .max_by_key(|(_, module)| module.dir.len())
        .map(|(index, _)| index)
}

/// Whether the go tool leaves a file, given relative to its module's
/// directory, out of the build. Directories named `vendor` or `testdata` and
/// every path component starting with `.` or `_` are excluded by the tool
/// itself, so their contents never reach the parser.
fn is_outside_the_build(relative: &str) -> bool {
    let mut components = relative.split('/').peekable();
    while let Some(component) = components.next() {
        let is_file = components.peek().is_none();
        if component.starts_with('.') || component.starts_with('_') {
            return true;
        }
        if !is_file && (component == "vendor" || component == "testdata") {
            return true;
        }
    }
    false
}

fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(head, _)| head)
}

fn join(dir: &str, rest: &str) -> String {
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
            .map_or(path, |p| p.strip_prefix('/').unwrap_or(p))
    }
}
