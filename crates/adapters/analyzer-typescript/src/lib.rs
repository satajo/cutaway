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
//! witnesses what depends on what, through every specifier a file names, every
//! name it takes across that boundary, and every name the rest of the file
//! writes where a name can only mean a declaration: a call, a constructor, a
//! heritage clause, a type position, a JSX element. Such a name resolves
//! against the file's own declarations first, then through what its imports
//! bound. A dependency the manifest declares but nothing names produces no
//! relation: the sources are the one truth about coupling.
//!
//! A dependency speaks from the declaration that writes it: a call in a
//! function's body is the function's edge, a property's type the interface's,
//! and a component rendered in a component's body the renderer's. A public
//! method of an exported class is an element of its own, contained by the
//! class and speaking for itself; a private member keeps speaking as the
//! class. An import statement, top-level code, and everything an unexported
//! declaration writes speak from the module - an unexported declaration is no
//! element, so its coupling is the module's own.
//!
//! Files are modules, and the directories that group at least two of them are
//! directories: a specifier names a file, so the file is the unit of
//! dependency, while a directory only groups and the language reads nothing
//! into it. A directory grouping fewer than two things groups nothing and
//! dissolves, handing its contents to the nearest directory above it that
//! survived, else to the package. The entry file of a
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
use cutaway_inspection::ports::source_tree::{DirectoryPath, SourceFile, SourcePath};

use crate::declarations::{Attributions, Declaration, DeclarationIndex, NestedDeclaration};
use crate::imports::{Import, Reference};
use crate::manifest::DiscoveredPackage;
use crate::modules::{Module, ModuleCatalog, ModuleSurface, ResolvedSpecifier};
use crate::reexports::ReexportTable;

pub struct TypeScriptSourceAnalyzer;

/// What one source file contributes before specifiers resolve.
struct ParsedFile {
    path: SourcePath,
    declarations: Vec<Declaration>,
    /// The public methods the file's exported classes declare.
    nested: Vec<NestedDeclaration>,
    imports: Vec<Import>,
    /// The names the file writes outside its import statements.
    references: Vec<Reference>,
    /// Which declaration speaks for each part of the file.
    attributions: Attributions,
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
            let nested = declarations::nested(root, text, &file.path, &surface.declarations);
            let imports = imports::declared(root, text);
            reexports.add(&file.path, &imports);
            parsed.push(ParsedFile {
                path: file.path.clone(),
                attributions: Attributions::of(&surface.declarations, &nested),
                declarations: surface.declarations,
                nested,
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
            for declaration in &file.declarations {
                // Only the module's surface joins the architecture;
                // unexported declarations are its internals. One file may
                // bind the same name twice; the first binding in source order
                // answers, as the declaration index does.
                if !declaration.exported || !declared_ids.insert(declaration.element.id.clone()) {
                    continue;
                }
                elements.push(AnalyzedElement {
                    element: declaration.element.clone(),
                    parent: Some(module.id()),
                });
            }
            for declaration in &file.nested {
                if !declared_ids.insert(declaration.element.id.clone()) {
                    continue;
                }
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

/// The files this analyzer read meaning from: every source file the catalog
/// placed - it became a module element or dissolved into its package - and
/// every manifest that named a package. A nameless manifest (a workspace
/// root) named nothing, and vendored code under `node_modules` is never even
/// read, so both stay unclaimed and keep a place of their own in the
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

/// Turns one file's imports and references into dependency relations. An
/// import statement is the module's plumbing whatever wrote it, so its edge
/// speaks from the module; a reference speaks from the declaration enclosing
/// it.
fn witness_dependencies(
    file: &ParsedFile,
    module: &Module,
    catalog: &ModuleCatalog,
    packages: &[DiscoveredPackage],
    surface: ModuleSurface<'_>,
    relations: &mut BTreeSet<Relation>,
) {
    // The containment chain within this file: a method sits in its class, a
    // declaration in its module. An edge along that chain, in either
    // direction, restates containment rather than witnessing a dependency.
    let holder_of = |id: &ElementId| -> Option<ElementId> {
        if let Some(nested) = file.nested.iter().find(|n| &n.element.id == id) {
            return Some(nested.holder.clone());
        }
        file.declarations
            .iter()
            .any(|d| &d.element.id == id)
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
    let mut depend = |from: &ElementId, to: &ElementId| {
        if from == to
            || to == &module.id()
            || holders(from).contains(to)
            || holders(to).contains(from)
        {
            return;
        }
        relations.insert(Relation {
            from: from.clone(),
            to: to.clone(),
            kind: RelationKind::DependsOn,
        });
    };

    // What the file's imports leave behind for the rest of it: a namespace
    // under its qualifier, and every other imported name bound to the element
    // that answers for it. The first binding of a name in source order
    // answers, as everywhere else.
    let mut qualifiers: BTreeMap<&str, ResolvedSpecifier> = BTreeMap::new();
    let mut bindings: BTreeMap<&str, ElementId> = BTreeMap::new();
    for import in &file.imports {
        let Some(target) = catalog.resolve_specifier(&file.path, &import.specifier, packages)
        else {
            continue;
        };
        depend(&module.id(), &target.id);
        for imported in &import.names {
            let to = catalog.resolve_name(&target, &imported.name, packages, surface);
            depend(&module.id(), &to);
            if let Some(local) = &imported.local {
                bindings.entry(local).or_insert(to);
            }
        }
        if let Some(namespace) = &import.namespace {
            qualifiers.entry(namespace).or_insert(target);
        }
    }

    for reference in &file.references {
        let Some(to) = resolve_reference(
            file,
            reference,
            catalog,
            packages,
            surface,
            &qualifiers,
            &bindings,
        ) else {
            continue;
        };
        let from = file
            .attributions
            .speaker_at(reference.span.start)
            .cloned()
            .unwrap_or_else(|| module.id());
        depend(&from, &to);
    }
}

/// Where one written name leads. A qualified name reads against the namespace
/// imports alone, because only a namespace import binds a qualifier that
/// stands for a module. A bare name is the file's own declaration first - a
/// declaration shadows anything an import brought in - and what an import
/// bound for it otherwise. A method is never a target: `obj.method()` reads
/// against a value, and without type inference the value's class is unknown,
/// so member access resolves no deeper than the class.
fn resolve_reference(
    file: &ParsedFile,
    reference: &Reference,
    catalog: &ModuleCatalog,
    packages: &[DiscoveredPackage],
    surface: ModuleSurface<'_>,
    qualifiers: &BTreeMap<&str, ResolvedSpecifier>,
    bindings: &BTreeMap<&str, ElementId>,
) -> Option<ElementId> {
    if let Some(qualifier) = &reference.qualifier {
        let target = qualifiers.get(qualifier.as_str())?;
        return Some(catalog.resolve_name(target, &reference.name, packages, surface));
    }
    if let Some(own) = surface
        .declarations
        .declaration(&file.path, &reference.name)
    {
        // An unexported declaration is the module's internals: what names it
        // says nothing beyond the module it already sits in.
        return Some(if own.exported {
            own.id.clone()
        } else {
            catalog
                .module_of(&file.path)
                .expect("the first pass kept only cataloged files")
                .id()
        });
    }
    bindings.get(reference.name.as_str()).cloned()
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
        fingerprint: None,
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
