//! Rust ecosystem adapter: implements
//! [`cutaway_inspection::ports::source_analyzer::SourceAnalyzer`] for Rust
//! projects.
//!
//! Cargo manifests locate the packages; the source text alone witnesses
//! what depends on what, through `use` declarations and every qualified
//! path the code mentions. A dependency the manifest declares but nothing
//! names produces no relation: the sources are the one truth about
//! coupling.
//!
//! Modules are files: the module tree of a package derives from the `src/`
//! file layout (`foo.rs` or `foo/mod.rs`), and inline `mod` blocks stay
//! items of their enclosing file. The crate root file is the package's own
//! code rather than a module of its own: its declarations are the package's
//! items, its child modules are the package's modules, and an import that
//! resolves to it lands on the package. Imports resolve down to the deepest
//! file module that exists, and one segment further onto a top-level
//! declaration of that module when the path continues. A module that re-exports the
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

use std::collections::BTreeSet;

use cutaway_architecture::{Element, ElementId, ElementKind, ElementName, Relation, RelationKind};
use cutaway_inspection::ports::source_analyzer::{
    AnalyzedElement, SourceAnalysisError, SourceAnalyzer, SourceStructure,
};
use cutaway_inspection::ports::source_tree::{SourceFile, SourcePath};

use crate::declarations::DeclarationIndex;
use crate::imports::Import;
use crate::manifest::DiscoveredPackage;
use crate::modules::{ModuleCatalog, ModuleSurface, ResolvedTarget};
use crate::reexports::ReexportTable;

pub struct RustSourceAnalyzer;

/// What one source file contributes before import resolution.
struct ParsedFile {
    path: SourcePath,
    declarations: Vec<Element>,
    imports: Vec<Import>,
    /// Qualified paths the file mentions outside its `use` declarations.
    references: Vec<Vec<String>>,
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
            let imports = imports::declared(root, text);
            reexports.add(&file.path, &imports);
            let references = imports::referenced(root, text);
            parsed.push(ParsedFile {
                path: file.path.clone(),
                declarations,
                imports,
                references,
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
            for declaration in file.declarations {
                elements.push(AnalyzedElement {
                    element: declaration,
                    parent: Some(module.id()),
                });
            }
            let imported = file.imports.iter().map(|import| import.path.as_slice());
            let referenced = file.references.iter().map(Vec::as_slice);
            for path in imported.chain(referenced) {
                let Some(target) = catalog.resolve(module, path, &packages, surface) else {
                    continue;
                };
                let to = match target {
                    ResolvedTarget::Element(id) => {
                        if id == module.id() {
                            continue;
                        }
                        id
                    }
                    ResolvedTarget::Package(package) => package_id(package),
                };
                relations.insert(Relation {
                    from: module.id(),
                    to,
                    kind: RelationKind::DependsOn,
                });
            }
        }

        Ok(SourceStructure {
            elements,
            relations: relations.into_iter().collect(),
        })
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
    Element {
        id: package_id(package),
        name: ElementName::new(&package.name).expect("a package name is never empty"),
        kind: ElementKind::Package,
    }
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
