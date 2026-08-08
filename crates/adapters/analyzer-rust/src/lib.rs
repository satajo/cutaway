//! Rust ecosystem adapter: implements
//! [`cutaway_inspection::ports::source_analyzer::SourceAnalyzer`] for Rust
//! projects.
//!
//! Cargo manifests locate the packages; the source text alone witnesses
//! what depends on what, through `use` declarations, every qualified path
//! the code mentions, and the bare names its calls and type positions
//! speak. A dependency the manifest declares but nothing names produces no
//! relation: the sources are the one truth about coupling.
//!
//! A dependency speaks from the declaration that writes it: a call in a
//! function's body is the function's edge, a field's type the struct's, and
//! what an `impl` block references belongs to the type it implements. A
//! `use` declaration, top-level code, and everything a private declaration
//! writes speak from the module - a private declaration is no element, so
//! its coupling is the module's own.
//!
//! Modules are files: the module tree of a package derives from the `src/`
//! file layout (`foo.rs` or `foo/mod.rs`), and inline `mod` blocks stay
//! items of their enclosing file. Only declarations with a visibility
//! modifier become items: a bare declaration is the module's internals, and
//! a path that names one lands on the module. The crate root file is the package's own
//! code rather than a module of its own: its declarations are the package's
//! items, its child modules are the package's modules, and an import that
//! resolves to it lands on the package. Imports resolve down to the deepest
//! file module that exists, one segment further onto a top-level
//! declaration of that module when the path continues, and one more onto a
//! public method the declaration holds (`Config::new`). A public method of
//! an inherent impl is an element of its own, contained by its type; what
//! a trait impl gives a type stays the type's own. A module that re-exports the
//! named item instead of declaring it forwards the import onwards, so
//! facades (`pub use element::Element;`) resolve to the item behind them.
//! Targets outside the project (std, third-party crates) are not part of
//! the architecture and produce no relation.
//!
//! Nothing outside this crate knows any of this is Rust-specific.

mod declarations;
mod imports;
mod manifest;
mod modules;
mod reexports;

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{Element, ElementId, ElementName, Relation, RelationKind, SemanticKind};
use cutaway_inspection::ports::source_analyzer::{
    AnalyzedElement, SourceAnalysisError, SourceAnalyzer, SourceStructure,
};
use cutaway_inspection::ports::source_tree::{DirectoryPath, SourceFile, SourcePath};

use crate::declarations::{Attributions, Declaration, DeclarationIndex, NestedDeclaration};
use crate::imports::{Import, Reference};
use crate::manifest::DiscoveredPackage;
use crate::modules::{ModuleCatalog, ModuleSurface, ResolvedTarget};
use crate::reexports::ReexportTable;

pub struct RustSourceAnalyzer;

/// What one source file contributes before import resolution.
struct ParsedFile {
    path: SourcePath,
    declarations: Vec<Declaration>,
    /// The public methods the file's inherent impl blocks declare for its
    /// public types.
    nested: Vec<NestedDeclaration>,
    imports: Vec<Import>,
    /// Paths the file mentions outside its `use` declarations.
    references: Vec<Reference>,
    /// Which declaration speaks for each part of the file.
    attributions: Attributions,
}

impl SourceAnalyzer for RustSourceAnalyzer {
    fn analyze(&self, files: &[SourceFile]) -> Result<SourceStructure, SourceAnalysisError> {
        let packages = manifest::discover_packages(files)?;
        let catalog = ModuleCatalog::build(&packages, files)?;

        let mut elements = Vec::new();
        let mut relations = BTreeSet::new();

        for (index, package) in packages.iter().enumerate() {
            elements.push(AnalyzedElement {
                element: package_element(package),
                parent: enclosing_package(&packages, index).map(|p| package_id(&packages[p])),
            });
        }

        for module in catalog.modules() {
            let Some(element) = module.element() else {
                continue;
            };
            elements.push(AnalyzedElement {
                element,
                parent: catalog.parent_of(module, &packages),
            });
        }

        // Two passes over the sources: imports resolve against the
        // declarations and re-exports of every file, so all files parse
        // before any import resolves.
        let mut parsed = Vec::new();
        let mut declaration_index = DeclarationIndex::default();
        let mut reexports = ReexportTable::default();
        for file in files {
            if catalog.module_of(&file.path).is_none() {
                continue;
            }
            let text = std::str::from_utf8(&file.contents).map_err(|_| {
                SourceAnalysisError::NonUtf8Text {
                    path: file.path.clone(),
                }
            })?;
            let tree = parse(text, &file.path)?;
            let root = tree.root_node();
            let declarations = declarations::top_level(root, text, &file.path);
            declaration_index.add(&file.path, &declarations);
            let nested = declarations::nested(root, text, &file.path, &declarations);
            declaration_index.add_nested(&file.path, &nested);
            let imports = imports::declared(root, text);
            reexports.add(&file.path, &imports);
            let references = imports::referenced(root, text);
            let attributions = declarations::attributions(root, text, &declarations, &nested);
            parsed.push(ParsedFile {
                path: file.path.clone(),
                declarations,
                nested,
                imports,
                references,
                attributions,
            });
        }

        let surface = ModuleSurface {
            declarations: &declaration_index,
            reexports: &reexports,
        };
        for file in parsed {
            let module = catalog
                .module_of(&file.path)
                .expect("the first pass kept only cataloged files");
            for declaration in &file.declarations {
                // Only the module's surface joins the architecture; bare
                // declarations are its internals.
                if !declaration.public {
                    continue;
                }
                elements.push(AnalyzedElement {
                    element: declaration.element.clone(),
                    parent: Some(module.id()),
                });
            }
            for declaration in &file.nested {
                elements.push(AnalyzedElement {
                    element: declaration.element.clone(),
                    parent: Some(declaration.holder.clone()),
                });
            }

            witness_dependencies(&file, module, &catalog, &packages, surface, &mut relations);
        }

        Ok(SourceStructure {
            elements,
            relations: relations.into_iter().collect(),
            claimed: claimed(&packages, &catalog, files),
            territories: territories(&packages),
        })
    }
}

/// The files this analyzer read meaning from: every `.rs` file the catalog
/// placed - it became a module element or dissolved into its package - and
/// every manifest that named a package. A workspace-only manifest named
/// nothing, so it stays unclaimed and keeps a place of its own in the
/// picture.
fn claimed(
    packages: &[DiscoveredPackage],
    catalog: &ModuleCatalog,
    files: &[SourceFile],
) -> BTreeSet<SourcePath> {
    files
        .iter()
        .map(|file| file.path.clone())
        .filter(|path| catalog.module_of(path).is_some())
        .chain(packages.iter().map(DiscoveredPackage::manifest))
        .collect()
}

/// The directory each package occupies: what the language leaves unclaimed
/// inside it still belongs inside the package's boundary.
fn territories(packages: &[DiscoveredPackage]) -> BTreeMap<DirectoryPath, ElementId> {
    packages
        .iter()
        .map(|package| {
            (
                DirectoryPath::new(&package.dir)
                    .expect("a manifest directory carries no trailing slash"),
                package_id(package),
            )
        })
        .collect()
}

/// Turns one file's imports and references into dependency relations. A
/// `use` declaration is the module's plumbing whatever wrote it, so its
/// edge speaks from the module; a reference speaks from the declaration
/// enclosing it.
fn witness_dependencies(
    file: &ParsedFile,
    module: &modules::Module,
    catalog: &ModuleCatalog,
    packages: &[DiscoveredPackage],
    surface: ModuleSurface<'_>,
    relations: &mut BTreeSet<Relation>,
) {
    // What a `use` binds in this file, so a bare name resolves to what the
    // import brought in. The first `use` of a name in source order answers,
    // as everywhere else.
    let mut bindings: BTreeMap<&str, &[String]> = BTreeMap::new();
    for import in &file.imports {
        if let Some(binding) = &import.binding {
            bindings.entry(binding).or_insert(&import.path);
        }
    }

    // The containment chain within this file: a method sits in its type, a
    // declaration in its module. An edge along that chain, in either
    // direction, restates containment rather than witnessing a dependency.
    let holder_of = |id: &ElementId| -> Option<ElementId> {
        if let Some(nested) = file.nested.iter().find(|n| n.element.id == *id) {
            return Some(nested.holder.clone());
        }
        file.declarations
            .iter()
            .any(|d| d.element.id == *id)
            .then(|| module.id())
    };
    let holders = |id: &ElementId| -> Vec<ElementId> {
        let mut chain = Vec::new();
        let mut current = id.clone();
        while let Some(above) = holder_of(&current) {
            chain.push(above.clone());
            current = above;
        }
        chain
    };
    let mut depend = |from: ElementId, target: ResolvedTarget<'_>| {
        let to = match target {
            ResolvedTarget::Element(id) => {
                if id == from
                    || id == module.id()
                    || holders(&from).contains(&id)
                    || holders(&id).contains(&from)
                {
                    return;
                }
                id
            }
            ResolvedTarget::Package(package) => package_id(package),
        };
        if to == from {
            return;
        }
        relations.insert(Relation {
            from,
            to,
            kind: RelationKind::DependsOn,
        });
    };

    for import in &file.imports {
        if let Some(target) = catalog.resolve(module, &import.path, packages, surface) {
            depend(module.id(), target);
        }
    }
    for reference in &file.references {
        let resolved = catalog
            .resolve(module, &reference.path, packages, surface)
            .or_else(|| {
                // A name no declaration of this module claims may be one a
                // `use` brought in: the bound path says where it leads.
                let bound = bindings.get(reference.path.first()?.as_str())?;
                let mut expanded = bound.to_vec();
                expanded.extend_from_slice(&reference.path[1..]);
                catalog.resolve(module, &expanded, packages, surface)
            });
        let Some(target) = resolved else {
            continue;
        };
        let from = file
            .attributions
            .speaker_at(reference.span.start)
            .cloned()
            .unwrap_or_else(|| module.id());
        depend(from, target);
    }
}

fn parse(text: &str, path: &SourcePath) -> Result<tree_sitter::Tree, SourceAnalysisError> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .expect("the bundled Rust grammar matches the linked tree-sitter version");
    let tree = parser
        .parse(text, None)
        .ok_or_else(|| SourceAnalysisError::Unparseable {
            path: path.clone(),
            reason: "the parser produced no syntax tree".to_owned(),
        })?;
    if tree.root_node().has_error() {
        return Err(SourceAnalysisError::Unparseable {
            path: path.clone(),
            reason: "the file contains syntax errors".to_owned(),
        });
    }
    Ok(tree)
}

fn package_element(package: &DiscoveredPackage) -> Element {
    Element::semantic(
        package_id(package),
        SemanticKind::Package,
        ElementName::new(&package.name).expect("a package name is never empty"),
    )
}

pub(crate) fn package_id(package: &DiscoveredPackage) -> ElementId {
    ElementId::new(format!("package:{}", package.name)).expect("a package name is never empty")
}

/// The package whose directory most closely encloses the package at `index`,
/// for the rare case of a package nested inside another package's directory.
fn enclosing_package(packages: &[DiscoveredPackage], index: usize) -> Option<usize> {
    let dir = &packages[index].dir;
    packages
        .iter()
        .enumerate()
        .filter(|(other, candidate)| {
            *other != index
                && !dir.is_empty()
                && (candidate.dir.is_empty() || dir.starts_with(&format!("{}/", candidate.dir)))
        })
        .max_by_key(|(_, candidate)| candidate.dir.len())
        .map(|(other, _)| other)
}

#[cfg(test)]
mod tests;
