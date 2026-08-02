//! The module structure of the discovered packages, and import resolution
//! against it.
//!
//! Modules are files. Within a package the `src/` layout defines the module
//! tree: `src/lib.rs` (or `src/main.rs` without a lib) is the crate root,
//! `src/foo.rs` and `src/foo/mod.rs` are the module `foo`, and so on. Files
//! outside `src/` (tests, benches, examples, `build.rs`) are crate roots of
//! their own, contained directly by the package. Files outside every package
//! attach to the project root.

use std::collections::BTreeMap;

use cutaway_architecture::{Element, ElementId, ElementKind, ElementName};
use cutaway_inspection::ports::source_analyzer::SourceAnalysisError;
use cutaway_inspection::ports::source_tree::{SourceFile, SourcePath};

use crate::declarations::DeclarationIndex;
use crate::manifest::DiscoveredPackage;
use crate::package_id;

/// One `.rs` file, placed in the module structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    path: SourcePath,
    /// Index into the discovered packages; None for files outside every
    /// package.
    package: Option<usize>,
    /// The logical module path within the package's `src/` tree
    /// (`[]` = crate root); None for files that are crate roots of their own.
    segments: Option<Vec<String>>,
    /// Human-facing name: the `::`-joined module path, or the file path
    /// relative to its package for standalone crate roots.
    name: String,
}

impl Module {
    pub fn id(&self) -> ElementId {
        ElementId::new(self.path.as_str()).expect("a source path is never empty")
    }

    pub fn element(&self) -> Element {
        Element {
            id: self.id(),
            name: ElementName::new(&self.name).expect("a module name is never empty"),
            kind: ElementKind::Module,
        }
    }
}

/// What an import path leads to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedTarget<'a> {
    /// A file module, or a top-level declaration within one.
    Element(ElementId),
    Package(&'a DiscoveredPackage),
}

pub struct ModuleCatalog {
    modules: Vec<Module>,
    by_path: BTreeMap<SourcePath, usize>,
    /// (package index, module path) -> module index, for `src/` trees.
    by_segments: BTreeMap<(usize, Vec<String>), usize>,
    /// Package name with `-` normalized to `_`, as it appears in `use`
    /// declarations -> package index.
    package_by_import_name: BTreeMap<String, usize>,
}

impl ModuleCatalog {
    pub fn build(
        packages: &[DiscoveredPackage],
        files: &[SourceFile],
    ) -> Result<Self, SourceAnalysisError> {
        let mut catalog = Self {
            modules: Vec::new(),
            by_path: BTreeMap::new(),
            by_segments: BTreeMap::new(),
            package_by_import_name: packages
                .iter()
                .enumerate()
                .map(|(index, package)| (package.name.replace('-', "_"), index))
                .collect(),
        };

        let lib_dirs: Vec<&str> = files
            .iter()
            .filter(|f| f.path.as_str().ends_with("/lib.rs") || f.path.as_str() == "src/lib.rs")
            .map(|f| f.path.as_str())
            .collect();

        for file in files {
            let path = file.path.as_str();
            if file.path.extension() != Some("rs") {
                continue;
            }
            let package = owning_package(packages, path);
            let package_dir = package.map_or("", |p| packages[p].dir.as_str());
            let relative = strip_dir(path, package_dir);

            let segments = package.and_then(|_| {
                let src_relative = relative.strip_prefix("src/")?;
                if src_relative.starts_with("bin/") {
                    return None;
                }
                if src_relative == "lib.rs" {
                    return Some(Vec::new());
                }
                if src_relative == "main.rs" {
                    let has_lib = lib_dirs
                        .iter()
                        .any(|lib| strip_dir(lib, package_dir) == "src/lib.rs");
                    return if has_lib { None } else { Some(Vec::new()) };
                }
                let logical = src_relative
                    .strip_suffix("/mod.rs")
                    .or_else(|| src_relative.strip_suffix(".rs"))?;
                Some(logical.split('/').map(str::to_owned).collect())
            });

            let name = match &segments {
                Some(s) if s.is_empty() => "crate".to_owned(),
                Some(s) => s.join("::"),
                None => relative.to_owned(),
            };

            let index = catalog.modules.len();
            if let (Some(package), Some(segments)) = (package, segments.as_ref())
                && let Some(existing) = catalog
                    .by_segments
                    .insert((package, segments.clone()), index)
            {
                return Err(SourceAnalysisError::Unparseable {
                    path: file.path.clone(),
                    reason: format!(
                        "module {name} is defined by both {} and {path}",
                        catalog.modules[existing].path
                    ),
                });
            }
            catalog.by_path.insert(file.path.clone(), index);
            catalog.modules.push(Module {
                path: file.path.clone(),
                package,
                segments,
                name,
            });
        }
        Ok(catalog)
    }

    pub fn modules(&self) -> impl Iterator<Item = &Module> {
        self.modules.iter()
    }

    pub fn module_of(&self, path: &SourcePath) -> Option<&Module> {
        self.by_path.get(path).map(|index| &self.modules[*index])
    }

    /// The containing element of a module: the nearest ancestor module in the
    /// `src/` tree, else its package, else nothing (the project root).
    pub fn parent_of(&self, module: &Module, packages: &[DiscoveredPackage]) -> Option<ElementId> {
        if let (Some(package), Some(segments)) = (module.package, module.segments.as_ref()) {
            let mut prefix = segments.clone();
            while !prefix.is_empty() {
                prefix.pop();
                if let Some(parent) = self.by_segments.get(&(package, prefix.clone()))
                    && self.modules[*parent].path != module.path
                {
                    return Some(self.modules[*parent].id());
                }
            }
        }
        module.package.map(|p| package_id(&packages[p]))
    }

    /// Resolves one `use` path from `module` to the deepest project element
    /// it leads to. Paths leaving the project (std, third-party crates)
    /// resolve to nothing.
    pub fn resolve<'a>(
        &self,
        module: &Module,
        segments: &[String],
        packages: &'a [DiscoveredPackage],
        declarations: &DeclarationIndex,
    ) -> Option<ResolvedTarget<'a>> {
        let first = segments.first()?;
        match first.as_str() {
            "std" | "core" | "alloc" | "proc_macro" => None,
            "crate" => {
                let package = module.package?;
                let start = match &module.segments {
                    Some(_) => self.by_segments.get(&(package, Vec::new())).copied()?,
                    None => self.by_path[&module.path],
                };
                Some(ResolvedTarget::Element(self.descend(
                    start,
                    &segments[1..],
                    declarations,
                )))
            }
            "self" => Some(ResolvedTarget::Element(self.descend(
                self.by_path[&module.path],
                &segments[1..],
                declarations,
            ))),
            "super" => {
                let package = module.package?;
                let own = module.segments.as_ref()?;
                let supers = segments
                    .iter()
                    .take_while(|s| s.as_str() == "super")
                    .count();
                if supers > own.len() {
                    return None;
                }
                let mut prefix = own[..own.len() - supers].to_vec();
                let start = loop {
                    if let Some(index) = self.by_segments.get(&(package, prefix.clone())) {
                        break *index;
                    }
                    if prefix.is_empty() {
                        return None;
                    }
                    prefix.pop();
                };
                Some(ResolvedTarget::Element(self.descend(
                    start,
                    &segments[supers..],
                    declarations,
                )))
            }
            name => {
                let target = *self.package_by_import_name.get(name)?;
                match self.by_segments.get(&(target, Vec::new())) {
                    Some(root) => Some(ResolvedTarget::Element(self.descend(
                        *root,
                        &segments[1..],
                        declarations,
                    ))),
                    None => Some(ResolvedTarget::Package(&packages[target])),
                }
            }
        }
    }

    /// Follows `rest` down the module tree from `start` to the deepest file
    /// module that exists; when the path continues past it, one further
    /// segment may land on a top-level declaration of that module.
    fn descend(&self, start: usize, rest: &[String], declarations: &DeclarationIndex) -> ElementId {
        let mut current = start;
        let mut consumed = 0;
        if let (Some(package), Some(base)) = (
            self.modules[start].package,
            self.modules[start].segments.clone(),
        ) {
            let mut prefix = base;
            for segment in rest {
                if segment == "self" {
                    consumed += 1;
                    break;
                }
                prefix.push(segment.clone());
                match self.by_segments.get(&(package, prefix.clone())) {
                    Some(next) => {
                        current = *next;
                        consumed += 1;
                    }
                    None => break,
                }
            }
        }
        let module = &self.modules[current];
        if let Some(next) = rest.get(consumed)
            && let Some(item) = declarations.declaration(&module.path, next)
        {
            return item.clone();
        }
        module.id()
    }
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

fn strip_dir<'a>(path: &'a str, dir: &str) -> &'a str {
    if dir.is_empty() {
        path
    } else {
        path.strip_prefix(dir)
            .and_then(|p| p.strip_prefix('/'))
            .unwrap_or(path)
    }
}
