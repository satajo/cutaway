//! Rust ecosystem adapter: implements
//! [`cutaway_inspection::ports::source_analyzer::SourceAnalyzer`] for Rust
//! projects.
//!
//! The analyzer reads two kinds of truth and emits both:
//! - Cargo manifests *declare* packages and their dependencies.
//! - Source text *shows* modules, their declarations, and the dependencies
//!   their `use` declarations actually exercise.
//!
//! Modules are files: the module tree of a package derives from the `src/`
//! file layout (`foo.rs` or `foo/mod.rs`), and inline `mod` blocks stay
//! items of their enclosing file. Imports resolve down to the deepest file
//! module that exists; targets outside the project (std, third-party
//! crates) are not part of the architecture and produce no relation.
//!
//! Nothing outside this crate knows any of this is Rust-specific.

mod declarations;
mod imports;
mod manifest;
mod modules;

use std::collections::BTreeSet;

use cutaway_architecture::{Element, ElementId, ElementKind, ElementName, Relation, RelationKind};
use cutaway_inspection::ports::source_analyzer::{
    AnalyzedElement, SourceAnalysisError, SourceAnalyzer, SourceStructure,
};
use cutaway_inspection::ports::source_tree::{SourceFile, SourcePath};

use crate::manifest::DiscoveredPackage;
use crate::modules::{ModuleCatalog, ResolvedTarget};

pub struct RustSourceAnalyzer;

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
            for dependency in &package.dependencies {
                if let Some(target) = packages.iter().find(|p| &p.name == dependency) {
                    relations.insert(Relation {
                        from: package_id(package),
                        to: package_id(target),
                        kind: RelationKind::DependsOn,
                    });
                }
            }
        }

        for module in catalog.modules() {
            elements.push(AnalyzedElement {
                element: module.element(),
                parent: catalog.parent_of(module, &packages),
            });
        }

        for file in files {
            let Some(module) = catalog.module_of(&file.path) else {
                continue;
            };
            let text = std::str::from_utf8(&file.contents).map_err(|_| {
                SourceAnalysisError::NonUtf8Text {
                    path: file.path.clone(),
                }
            })?;
            let tree = parse(text, &file.path)?;
            let root = tree.root_node();

            for declaration in declarations::top_level(root, text, &file.path) {
                elements.push(AnalyzedElement {
                    element: declaration,
                    parent: Some(module.id()),
                });
            }

            for import in imports::use_paths(root, text) {
                let Some(target) = catalog.resolve(module, &import, &packages) else {
                    continue;
                };
                let to = match target {
                    ResolvedTarget::Module(id) => {
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
