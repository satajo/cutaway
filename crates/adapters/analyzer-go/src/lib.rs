//! Go ecosystem adapter: implements
//! [`cutaway_inspection::ports::source_analyzer::SourceAnalyzer`] for Go
//! projects.
//!
//! go.mod manifests locate the modules; the source text alone witnesses what
//! depends on what, through the imports of every file and every qualified
//! name the code mentions. A requirement the manifest declares but nothing
//! names produces no relation: the sources are the one truth about coupling.
//!
//! Directories are the boundaries. Go compiles and imports a whole
//! directory at once and lets its files share one namespace, so the
//! directory is a module element. Inside it the files carry the internal
//! organization of the package: a directory of several files gives each file
//! a module of its own, and a directory of one file lets that file dissolve
//! into it, because one file groups nothing. The module root directory is
//! the module's own code rather than a directory of its own: its
//! declarations are the package's items, its subdirectories and files are
//! the package's modules, and an import that resolves to it lands on the
//! package. Imports resolve down to the deepest directory that exists, and a
//! qualified name one step further onto a declaration of that directory.
//!
//! Capitalization decides what joins the architecture: an upper-case name
//! leaves its directory and becomes an item of the file that declares it, a
//! lower-case one is the directory's internals, and a reference naming it
//! lands on the directory. Every dependency starts at the file that states
//! the import or the reference. Test files declare nothing importable, so
//! they contribute no items - their imports and references still witness
//! what they need.
//! Targets outside the project (the standard library, third-party modules)
//! are not part of the architecture and produce no relation.
//!
//! Nothing outside this crate knows any of this is Go-specific.

mod declarations;
mod imports;
mod manifest;
mod packages;

use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{Element, ElementId, ElementKind, ElementName, Relation, RelationKind};
use cutaway_inspection::ports::source_analyzer::{
    AnalyzedElement, SourceAnalysisError, SourceAnalyzer, SourceStructure,
};
use cutaway_inspection::ports::source_tree::{SourceFile, SourcePath};

use crate::declarations::{Declaration, DeclarationIndex};
use crate::imports::{Binding, Import, PackageNames};
use crate::manifest::DiscoveredModule;
use crate::packages::{DirectoryCatalog, ResolvedImport};

pub struct GoSourceAnalyzer;

/// What one source file contributes before import resolution.
struct ParsedFile {
    path: SourcePath,
    declarations: Vec<Declaration>,
    imports: Vec<Import>,
    /// Qualified names the file mentions, as (qualifier, name) pairs.
    references: Vec<(String, String)>,
}

impl SourceAnalyzer for GoSourceAnalyzer {
    fn analyze(&self, files: &[SourceFile]) -> Result<SourceStructure, SourceAnalysisError> {
        let modules = manifest::discover_modules(files)?;
        let catalog = DirectoryCatalog::build(&modules, files);

        let mut elements = containment(&modules, &catalog);
        let mut relations = BTreeSet::new();

        // Two passes over the sources: an import binds the package clause
        // name of its target and a reference resolves against that target's
        // declarations, so all files parse before any import resolves.
        let mut parsed = Vec::new();
        let mut declaration_index = DeclarationIndex::default();
        let mut package_names = PackageNames::default();
        for file in files {
            let Some(directory) = catalog.file(&file.path).map(|f| catalog.directory_of(f)) else {
                continue;
            };
            let text = std::str::from_utf8(&file.contents).map_err(|_| {
                SourceAnalysisError::NonUtf8Text {
                    path: file.path.clone(),
                }
            })?;
            let tree = parse(text, &file.path)?;
            let root = tree.root_node();
            // The go tool forbids importing a test file, so nothing a test
            // file declares is surface, and its package clause names a
            // namespace no import reaches.
            let declarations = if is_test_file(&file.path) {
                Vec::new()
            } else {
                let declarations = declarations::top_level(root, text, &file.path);
                declaration_index.add(directory.path(), &declarations);
                if let Some(name) = imports::package_clause(root, text) {
                    package_names.add(directory.path(), name);
                }
                declarations
            };
            parsed.push(ParsedFile {
                path: file.path.clone(),
                declarations,
                imports: imports::declared(root, text),
                references: imports::referenced(root, text),
            });
        }

        for file in parsed {
            // What the file's code speaks as: the file's own module, or the
            // directory the file dissolved into.
            let from = catalog
                .file(&file.path)
                .expect("the first pass kept only cataloged files")
                .id();
            for declaration in file.declarations {
                // Only the directory's surface joins the architecture;
                // unexported declarations are its internals.
                if !declaration.exported {
                    continue;
                }
                elements.push(AnalyzedElement {
                    element: declaration.element,
                    parent: Some(from.clone()),
                });
            }

            let mut qualifiers: BTreeMap<String, ResolvedImport> = BTreeMap::new();
            for import in &file.imports {
                let Some(target) = catalog.resolve(&import.path, &modules) else {
                    continue;
                };
                depend(&mut relations, &from, &target.id);
                match &import.binding {
                    Binding::Alias(alias) => {
                        qualifiers.insert(alias.clone(), target);
                    }
                    Binding::PackageName => {
                        if let Some(name) = package_names.of(&target.directory) {
                            qualifiers.insert(name.to_owned(), target);
                        }
                    }
                    Binding::Unbound => {}
                }
            }

            // Shadowed qualifiers are accepted as imprecision: a local
            // variable named like an imported directory makes this crate see
            // a dependency the code does not have, which is the same class of
            // imprecision the Rust adapter accepts for its qualified paths.
            for (qualifier, name) in &file.references {
                let Some(target) = qualifiers.get(qualifier) else {
                    continue;
                };
                let to = match declaration_index.declaration(&target.directory, name) {
                    Some(declaration) if declaration.exported => declaration.id.clone(),
                    // An unexported or unknown name is the target's own
                    // business; the dependency reaches no further than the
                    // directory.
                    _ => target.id.clone(),
                };
                depend(&mut relations, &from, &to);
            }
        }

        Ok(SourceStructure {
            elements,
            relations: relations.into_iter().collect(),
        })
    }
}

/// Everything the structure of the sources alone says, before any file is
/// read: the modules, the directories that hold code, and the files of every
/// directory that holds more than one.
fn containment(modules: &[DiscoveredModule], catalog: &DirectoryCatalog) -> Vec<AnalyzedElement> {
    let mut elements = Vec::new();
    for (index, module) in modules.iter().enumerate() {
        elements.push(AnalyzedElement {
            element: module_element(module),
            parent: enclosing_module(modules, index).map(|m| module_id(&modules[m])),
        });
    }
    for directory in catalog.directories() {
        if let Some(element) = directory.element() {
            elements.push(AnalyzedElement {
                element,
                parent: Some(catalog.parent_of(directory, modules)),
            });
        }
    }
    for file in catalog.files() {
        if let Some(element) = file.element() {
            elements.push(AnalyzedElement {
                element,
                parent: Some(catalog.directory_of(file).id()),
            });
        }
    }
    elements
}

/// Records a dependency unless it points at the element it starts from: an
/// element needing itself says nothing.
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

/// Whether the go tool treats the file as test-only code.
fn is_test_file(path: &SourcePath) -> bool {
    path.as_str().ends_with("_test.go")
}

fn parse(text: &str, path: &SourcePath) -> Result<tree_sitter::Tree, SourceAnalysisError> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_go::LANGUAGE.into())
        .expect("the bundled Go grammar matches the linked tree-sitter version");
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

fn module_element(module: &DiscoveredModule) -> Element {
    Element {
        id: module_id(module),
        name: ElementName::new(&module.path).expect("a module path is never empty"),
        kind: ElementKind::Package,
    }
}

pub(crate) fn module_id(module: &DiscoveredModule) -> ElementId {
    ElementId::new(format!("package:{}", module.path)).expect("a module path is never empty")
}

/// The module whose directory most closely encloses the module at `index`,
/// for a nested go.mod that carves its subtree out of an outer module.
fn enclosing_module(modules: &[DiscoveredModule], index: usize) -> Option<usize> {
    let dir = &modules[index].dir;
    modules
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
