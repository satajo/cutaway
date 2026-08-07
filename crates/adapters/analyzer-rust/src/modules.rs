//! The module structure of the discovered packages, and import resolution
//! against it.
//!
//! Modules are files. Within a package the `src/` layout defines the module
//! tree: `src/lib.rs` (or `src/main.rs` without a lib) is the crate root,
//! `src/foo.rs` and `src/foo/mod.rs` are the module `foo`, and so on. Files
//! outside `src/` (tests, benches, examples, `build.rs`) are crate roots of
//! their own, contained directly by the package. Files outside every package
//! attach to the project root.
//!
//! The crate root is the package's own code, not a module of its own: to
//! every consumer the package and its root namespace are one boundary, named
//! by the package. The root file therefore dissolves into the package - it
//! is no element, its declarations are the package's items, its child
//! modules are the package's modules, and a path that resolves to it lands
//! on the package.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{Element, ElementId, ElementKind, ElementName};
use cutaway_inspection::ports::source_analyzer::SourceAnalysisError;
use cutaway_inspection::ports::source_tree::{SourceFile, SourcePath};

use crate::declarations::DeclarationIndex;
use crate::manifest::DiscoveredPackage;
use crate::package_id;
use crate::reexports::ReexportTable;

/// One `.rs` file, placed in the module structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    path: SourcePath,
    /// The element this file speaks as: its own path, or the package for a
    /// crate root that dissolves into it.
    id: ElementId,
    /// Index into the discovered packages; None for files outside every
    /// package.
    package: Option<usize>,
    /// The logical module path within the package's `src/` tree
    /// (`[]` = crate root); None for files that are crate roots of their own.
    segments: Option<Vec<String>>,
    /// Human-facing name: the `::`-joined module path, the package name for
    /// a crate root, or the file path relative to its package for standalone
    /// crate roots.
    name: String,
}

impl Module {
    pub fn id(&self) -> ElementId {
        self.id.clone()
    }

    /// The module element this file contributes, and None for a crate root:
    /// that file dissolves into its package, which is an element already.
    pub fn element(&self) -> Option<Element> {
        if self.is_crate_root() {
            return None;
        }
        Some(Element {
            id: self.id(),
            name: ElementName::new(&self.name).expect("a module name is never empty"),
            kind: ElementKind::Module,
            fingerprint: None,
        })
    }

    /// Whether this file is the root of its package's `src/` module tree.
    fn is_crate_root(&self) -> bool {
        self.segments.as_ref().is_some_and(Vec::is_empty)
    }
}

/// What an import path leads to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedTarget<'a> {
    /// A file module, or a top-level declaration within one.
    Element(ElementId),
    Package(&'a DiscoveredPackage),
}

/// What every module offers an import that names it: the items it declares
/// and the names it re-exports. Both are indexed across the whole project
/// before any import resolves.
#[derive(Clone, Copy)]
pub struct ModuleSurface<'a> {
    pub declarations: &'a DeclarationIndex,
    pub reexports: &'a ReexportTable,
}

/// Where a path led, and whether the answering module claims the name the
/// path ends with. The distinction decides between wildcard re-exports: a
/// `pub use m::*` is the right forward only when something behind it claims
/// the name, not when resolution merely stopped at `m`.
struct Resolution<'a> {
    target: ResolvedTarget<'a>,
    claims_name: bool,
}

impl<'a> Resolution<'a> {
    fn claimed(target: ResolvedTarget<'a>) -> Self {
        Self {
            target,
            claims_name: true,
        }
    }

    fn stopped_at(module: ElementId) -> Self {
        Self {
            target: ResolvedTarget::Element(module),
            claims_name: false,
        }
    }
}

/// The re-export hops taken so far, as (module, name) pairs.
type FollowedHops = BTreeSet<(SourcePath, String)>;

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
                Some(s) if s.is_empty() => packages
                    [package.expect("a src/ tree lies within a package")]
                .name
                .clone(),
                Some(s) => s.join("::"),
                None => relative.to_owned(),
            };
            let id = match &segments {
                Some(s) if s.is_empty() => {
                    package_id(&packages[package.expect("a src/ tree lies within a package")])
                }
                _ => ElementId::new(path).expect("a source path is never empty"),
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
                id,
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
    /// `src/` tree - the package itself when that ancestor is the crate root -
    /// else its package, else nothing (the project root).
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
        surface: ModuleSurface<'_>,
    ) -> Option<ResolvedTarget<'a>> {
        let mut followed = FollowedHops::new();
        Some(
            self.resolve_path(module, segments, packages, surface, &mut followed)?
                .target,
        )
    }

    fn resolve_path<'a>(
        &self,
        module: &Module,
        segments: &[String],
        packages: &'a [DiscoveredPackage],
        surface: ModuleSurface<'_>,
        followed: &mut FollowedHops,
    ) -> Option<Resolution<'a>> {
        let first = segments.first()?;
        match first.as_str() {
            "std" | "core" | "alloc" | "proc_macro" => None,
            "crate" => {
                let package = module.package?;
                let start = match &module.segments {
                    Some(_) => self.by_segments.get(&(package, Vec::new())).copied()?,
                    None => self.by_path[&module.path],
                };
                Some(self.descend(start, &segments[1..], packages, surface, followed))
            }
            "self" => Some(self.descend(
                self.by_path[&module.path],
                &segments[1..],
                packages,
                surface,
                followed,
            )),
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
                Some(self.descend(start, &segments[supers..], packages, surface, followed))
            }
            name => {
                if let Some(package) = self.package_by_import_name.get(name).copied() {
                    return Some(self.enter_package(
                        package,
                        &segments[1..],
                        packages,
                        surface,
                        followed,
                    ));
                }
                // A `use` path may also start at an item of the importing
                // module itself (`use element::Element;` beside `mod
                // element;`). Only a path that reaches what it names counts
                // as such, so imports of crates outside the project keep
                // resolving to nothing.
                let resolution = self.descend(
                    self.by_path[&module.path],
                    segments,
                    packages,
                    surface,
                    followed,
                );
                resolution.claims_name.then_some(resolution)
            }
        }
    }

    /// Enters another package of the project by its crate root, or lands on
    /// the package itself when it has no root in the sources.
    fn enter_package<'a>(
        &self,
        package: usize,
        rest: &[String],
        packages: &'a [DiscoveredPackage],
        surface: ModuleSurface<'_>,
        followed: &mut FollowedHops,
    ) -> Resolution<'a> {
        if let Some(root) = self.by_segments.get(&(package, Vec::new())).copied() {
            self.descend(root, rest, packages, surface, followed)
        } else {
            Resolution {
                target: ResolvedTarget::Package(&packages[package]),
                claims_name: rest.is_empty(),
            }
        }
    }

    /// Follows `rest` down the module tree from `start` to the deepest file
    /// module that exists; when the path continues past it, the next segment
    /// may name a top-level declaration of that module or a name the module
    /// re-exports.
    fn descend<'a>(
        &self,
        start: usize,
        rest: &[String],
        packages: &'a [DiscoveredPackage],
        surface: ModuleSurface<'_>,
        followed: &mut FollowedHops,
    ) -> Resolution<'a> {
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
        let Some(name) = rest.get(consumed) else {
            return Resolution::claimed(ResolvedTarget::Element(module.id()));
        };
        if let Some(item) = surface.declarations.declaration(&module.path, name) {
            // A path may continue one segment past a type onto a method the
            // type declares (`Config::new`). A name the type holds no method
            // element for - a private method, a trait-given one, an
            // associated const - lands on the type, the way a private item
            // lands on the module.
            if item.public
                && let Some(method) = rest.get(consumed + 1).and_then(|next| {
                    surface
                        .declarations
                        .nested_declaration(&module.path, name, next)
                })
            {
                return Resolution::claimed(ResolvedTarget::Element(method.clone()));
            }
            // A private item is the module's internals: what names it
            // depends on the module.
            let target = if item.public {
                item.id.clone()
            } else {
                module.id()
            };
            return Resolution::claimed(ResolvedTarget::Element(target));
        }
        // Re-exports may point at each other, directly or in a ring. Taking
        // each (module, name) hop at most once along a chain ends such a ring
        // at the module where it closes instead of recursing forever; the hop
        // leaves the set again so that a sibling branch may still take it.
        let hop = (module.path.clone(), name.clone());
        let forwarded = if followed.insert(hop.clone()) {
            let found = self.follow_reexport(
                module,
                name,
                &rest[consumed + 1..],
                packages,
                surface,
                followed,
            );
            followed.remove(&hop);
            found
        } else {
            None
        };
        forwarded.unwrap_or_else(|| Resolution::stopped_at(module.id()))
    }

    /// Continues an import that named `name` at `module` without finding a
    /// declaration, through the `pub use` that makes `name` available there.
    /// A re-export target is written from `module`'s own perspective, so it
    /// resolves like any other import path, carrying the still unused `tail`
    /// of the original path.
    fn follow_reexport<'a>(
        &self,
        module: &Module,
        name: &str,
        tail: &[String],
        packages: &'a [DiscoveredPackage],
        surface: ModuleSurface<'_>,
        followed: &mut FollowedHops,
    ) -> Option<Resolution<'a>> {
        if let Some(forwarded) = surface.reexports.forwarded(&module.path, name) {
            let mut path = forwarded.to_vec();
            path.extend_from_slice(tail);
            if let Some(resolution) = self.resolve_path(module, &path, packages, surface, followed)
            {
                // The `pub use` names `name`, so this module claims it even
                // when the target itself resolves no further than a module.
                return Some(Resolution::claimed(resolution.target));
            }
        }
        // A `pub use m::*` re-exports whatever `m` holds, so `name` may hide
        // behind any of them. The first wildcard in source order that leads
        // to something claiming `name` answers.
        for wildcard in surface.reexports.wildcards(&module.path) {
            let mut path = wildcard.clone();
            path.push(name.to_owned());
            path.extend_from_slice(tail);
            if let Some(resolution) = self.resolve_path(module, &path, packages, surface, followed)
                && resolution.claims_name
            {
                return Some(resolution);
            }
        }
        None
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
