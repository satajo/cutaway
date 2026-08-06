//! TypeScript ecosystem adapter: implements
//! [`cutaway_inspection::ports::source_analyzer::SourceAnalyzer`] for
//! TypeScript and JavaScript projects.
//!
//! One adapter serves both languages because they are one ecosystem: the same
//! module system moves the same names between files, the same package.json
//! declares the packages, and the same grammar family reads the sources. A
//! project that mixes `.ts` and `.js` files has one architecture, not two.
//!
//! package.json manifests locate the packages; the source text alone
//! witnesses what depends on what, through every specifier a file names and
//! every name it takes across that boundary. A dependency the manifest
//! declares but nothing names produces no relation: the sources are the one
//! truth about coupling.
//!
//! Files are modules, and so are the directories that group at least two of
//! them: a specifier names a file, so the file is the unit of dependency,
//! while a directory only groups. A directory grouping fewer than two things
//! groups nothing and dissolves, handing its contents to the nearest
//! directory above it that survived, else to the package. The entry file of a
//! package dissolves the same way: to every consumer, importing the package by
//! name and the surface of its entry file are one boundary, so that file is no
//! element, its declarations are the package's items, and a specifier that
//! resolves to it lands on the package.
//!
//! Export decides what joins the architecture: an exported declaration
//! becomes an item, an unexported one is the module's internals, and a name
//! that resolves onto it lands on the module. A file that re-exports a name
//! instead of declaring it forwards resolution onwards, so an import through
//! a barrel file reaches the item behind it. Targets outside the project (npm
//! dependencies, runtime builtins, anything under `node_modules`) are not
//! part of the architecture and produce no relation.
//!
//! tsconfig.json path aliases stay out of scope on purpose: they are a
//! compiler configuration layer written over the module system, not the
//! module system itself.
//!
//! Nothing outside this crate knows any of this is TypeScript-specific.

mod declarations;
mod imports;
mod manifest;
mod modules;
mod reexports;

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{Element, ElementId, ElementKind, ElementName, Relation, RelationKind};
use cutaway_inspection::ports::source_analyzer::{
    AnalyzedElement, SourceAnalysisError, SourceAnalyzer, SourceStructure,
};
use cutaway_inspection::ports::source_tree::{SourceFile, SourcePath};

use crate::declarations::{Declaration, DeclarationIndex};
use crate::imports::Import;
use crate::manifest::DiscoveredPackage;
use crate::modules::{ModuleCatalog, ModuleSurface, ResolvedSpecifier};
use crate::reexports::ReexportTable;

pub struct TypeScriptSourceAnalyzer;

/// What one source file contributes before specifiers resolve.
struct ParsedFile {
    path: SourcePath,
    declarations: Vec<Declaration>,
    imports: Vec<Import>,
    /// Qualified names the file mentions, as (qualifier, name) pairs.
    references: Vec<(String, String)>,
}

impl SourceAnalyzer for TypeScriptSourceAnalyzer {
    fn analyze(&self, files: &[SourceFile]) -> Result<SourceStructure, SourceAnalysisError> {
        let packages = manifest::discover_packages(files)?;
        let catalog = ModuleCatalog::build(&packages, files);

        let mut elements = Vec::new();
        let mut relations = BTreeSet::new();

        for (index, package) in packages.iter().enumerate() {
            elements.push(AnalyzedElement {
                element: package_element(package),
                parent: enclosing_package(&packages, index).map(|p| package_id(&packages[p])),
            });
        }

        for directory in catalog.directories() {
            elements.push(AnalyzedElement {
                element: directory.element(),
                parent: directory.parent(&packages),
            });
        }

        for module in catalog.modules() {
            let Some(element) = module.element() else {
                continue;
            };
            elements.push(AnalyzedElement {
                element,
                parent: module.parent(&packages),
            });
        }

        // Two passes over the sources: a name resolves against the
        // declarations and re-exports of every file, so all files parse
        // before any specifier resolves.
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
            let surface = declarations::top_level(root, text, &file.path);
            declaration_index.add(&file.path, &surface);
            let imports = imports::declared(root, text);
            reexports.add(&file.path, &imports);
            parsed.push(ParsedFile {
                path: file.path.clone(),
                declarations: surface.declarations,
                imports,
                references: imports::referenced(root, text),
            });
        }

        let surface = ModuleSurface {
            declarations: &declaration_index,
            reexports: &reexports,
        };
        let mut declared_ids = BTreeSet::new();
        for file in parsed {
            let module = catalog
                .module_of(&file.path)
                .expect("the first pass kept only cataloged files");
            for declaration in file.declarations {
                // Only the module's surface joins the architecture;
                // unexported declarations are its internals. One file may
                // bind the same name twice; the first binding in source order
                // answers, as the declaration index does.
                if !declaration.exported || !declared_ids.insert(declaration.element.id.clone()) {
                    continue;
                }
                elements.push(AnalyzedElement {
                    element: declaration.element,
                    parent: Some(module.id()),
                });
            }

            let mut qualifiers: BTreeMap<String, ResolvedSpecifier> = BTreeMap::new();
            for import in &file.imports {
                let Some(target) =
                    catalog.resolve_specifier(&file.path, &import.specifier, &packages)
                else {
                    continue;
                };
                depend(&mut relations, &module.id(), &target.id);
                for name in &import.names {
                    let to = catalog.resolve_name(&target, name, &packages, surface);
                    depend(&mut relations, &module.id(), &to);
                }
                if let Some(namespace) = &import.namespace {
                    qualifiers.insert(namespace.clone(), target);
                }
            }

            // Shadowed qualifiers are accepted as imprecision: a local
            // variable named like a namespace import makes this crate see a
            // dependency the code does not have, which is the same class of
            // imprecision the other language adapters accept.
            for (qualifier, name) in &file.references {
                let Some(target) = qualifiers.get(qualifier) else {
                    continue;
                };
                let to = catalog.resolve_name(target, name, &packages, surface);
                depend(&mut relations, &module.id(), &to);
            }
        }

        Ok(SourceStructure {
            elements,
            relations: relations.into_iter().collect(),
        })
    }
}

/// Records a dependency unless it points at the element it starts from: a
/// module needing itself says nothing.
fn depend(relations: &mut BTreeSet<Relation>, from: &ElementId, to: &ElementId) {
    if from == to {
        return;
    }
    relations.insert(Relation {
        from: from.clone(),
        to: to.clone(),
        kind: RelationKind::DependsOn,
    });
}

fn parse(text: &str, path: &SourcePath) -> Result<tree_sitter::Tree, SourceAnalysisError> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&grammar(path.extension()))
        .expect("the bundled grammars match the linked tree-sitter version");
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

/// The grammar that reads a file, chosen by extension.
///
/// TSX is a superset of JavaScript with JSX, and plain JavaScript parses
/// identically under it, so one grammar reads every JavaScript dialect and
/// the `.tsx` files. The TypeScript grammar reads the extensions that forbid
/// JSX, where `<T>x` is a type assertion rather than an element.
fn grammar(extension: Option<&str>) -> tree_sitter::Language {
    match extension {
        Some("ts" | "mts" | "cts") => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        _ => tree_sitter_typescript::LANGUAGE_TSX.into(),
    }
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
/// for the workspace layout that nests one package inside another's
/// directory.
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

fn text_of(node: tree_sitter::Node<'_>, text: &str) -> String {
    node.utf8_text(text.as_bytes())
        .expect("node ranges lie within the parsed text")
        .to_owned()
}

#[cfg(test)]
mod tests;
