//! Go ecosystem adapter: implements
//! [`cutaway_inspection::ports::source_analyzer::SourceAnalyzer`] for Go
//! projects.
//!
//! go.mod manifests locate the modules; the source text alone witnesses what
//! depends on what, through the imports of every file and every name the code
//! writes where a name can only mean a declaration: a call, a type position,
//! a composite literal. A qualified name resolves against the directory its
//! qualifier was imported from, a bare one against the file's own directory,
//! whose files share a namespace. A requirement the manifest declares but
//! nothing names produces no relation: the sources are the one truth about
//! coupling.
//!
//! A dependency speaks from the declaration that writes it: a call in a
//! function's body is the function's edge, a field's type the struct's, and
//! what a method writes belongs to the type its receiver extends. An
//! exported method of an exported same-file type is an element of its own,
//! contained by the type and speaking for itself; an unexported method
//! keeps speaking as the type. An import, top-level code, and everything an
//! unexported declaration writes speak from the file's own element - an
//! unexported declaration is no element, so its coupling is the module's
//! own.
//!
//! Directories are the boundaries. Go compiles and imports a whole
//! directory at once and lets its files share one namespace, so the
//! directory is a module element. Go reads nothing into a file, so a file is
//! no module: it stands where the tree puts it, inside its directory,
//! holding the declarations written in it. The module root directory is the
//! module's own code rather than a directory of its own: the go.mod module
//! is that boundary, and an import that resolves to it lands on the package.
//! Imports resolve down to the deepest directory that exists, and a
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

use cutaway_architecture::{Element, ElementId, ElementName, Relation, RelationKind, SemanticKind};
use cutaway_inspection::ports::source_analyzer::{
    AnalysisGap, Extent, GapReason, Interpretation, SourceAnalyzer, SourceStructure,
};
use cutaway_inspection::ports::source_tree::{DirectoryPath, SourceFile, SourcePath};

use crate::declarations::{Attributions, Declaration, DeclarationIndex, NestedDeclaration};
use crate::imports::{Binding, Import, PackageNames, Reference};
use crate::manifest::DiscoveredModule;
use crate::packages::{Directory, DirectoryCatalog, GoFile, ResolvedImport};

pub struct GoSourceAnalyzer;

/// What one source file contributes before import resolution.
struct ParsedFile {
    path: SourcePath,
    declarations: Vec<Declaration>,
    /// The exported methods the file declares for its exported types.
    nested: Vec<NestedDeclaration>,
    imports: Vec<Import>,
    /// The names the file writes outside its imports.
    references: Vec<Reference>,
    /// Which declaration speaks for each part of the file.
    attributions: Attributions,
}

impl SourceAnalyzer for GoSourceAnalyzer {
    fn analyze(&self, files: &[SourceFile]) -> SourceStructure {
        let (modules, mut gaps) = manifest::discover_modules(files);
        let catalog = DirectoryCatalog::build(&modules, files);

        let mut interpretations = structure(&modules, &catalog);
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
            let Ok(text) = std::str::from_utf8(&file.contents) else {
                gaps.push(AnalysisGap {
                    path: file.path.clone(),
                    reason: GapReason::NonUtf8Text,
                });
                continue;
            };
            let (tree, gap) = parse(text, &file.path);
            gaps.extend(gap);
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
            let nested = declarations::nested(root, text, &file.path, &declarations);
            parsed.push(ParsedFile {
                path: file.path.clone(),
                attributions: declarations::attributions(root, text, &declarations, &nested),
                declarations,
                nested,
                imports: imports::declared(root, text),
                references: imports::referenced(root, text),
            });
        }

        for file in &parsed {
            let cataloged = catalog
                .file(&file.path)
                .expect("the first pass kept only cataloged files");
            for declaration in &file.declarations {
                // Only the directory's surface joins the architecture;
                // unexported declarations are its internals.
                if !declaration.exported {
                    continue;
                }
                interpretations.push(Interpretation {
                    element: declaration.element.clone(),
                    extent: Extent::Within {
                        file: file.path.clone(),
                        parent: None,
                    },
                });
            }
            for declaration in &file.nested {
                interpretations.push(Interpretation {
                    element: declaration.element.clone(),
                    extent: Extent::Within {
                        file: file.path.clone(),
                        parent: Some(declaration.holder.clone()),
                    },
                });
            }

            witness_dependencies(
                file,
                cataloged,
                &catalog,
                &declaration_index,
                &package_names,
                &modules,
                &mut relations,
            );
        }

        SourceStructure {
            interpretations,
            relations: relations.into_iter().collect(),
            gaps,
        }
    }
}

/// Turns one file's imports and references into dependency relations. An
/// import is the file's own plumbing whatever wrote it, so its edge speaks
/// from the element the file speaks as; a reference speaks from the
/// declaration enclosing it.
fn witness_dependencies(
    file: &ParsedFile,
    cataloged: &GoFile,
    catalog: &DirectoryCatalog,
    declarations: &DeclarationIndex,
    package_names: &PackageNames,
    modules: &[DiscoveredModule],
    relations: &mut BTreeSet<Relation>,
) {
    let speaks_as = cataloged.id();
    let directory = catalog.directory_of(cataloged);
    let own_directory = directory.id();
    // The containment chain within this file: a method sits in its type, a
    // declaration in the file that writes it, and the file in its directory.
    // An edge along that chain, in either direction, restates containment
    // rather than witnessing a dependency, so the guard drops it - at the
    // file and at the directory alike, because an edge onto the directory a
    // file lies in says nothing either.
    let holder_of = |id: &ElementId| -> Option<ElementId> {
        if let Some(nested) = file.nested.iter().find(|n| &n.element.id == id) {
            return Some(nested.holder.clone());
        }
        file.declarations
            .iter()
            .any(|d| &d.element.id == id)
            .then(|| speaks_as.clone())
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
            || to == &speaks_as
            || to == &own_directory
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

    let mut qualifiers: BTreeMap<&str, ResolvedImport> = BTreeMap::new();
    for import in &file.imports {
        let Some(target) = catalog.resolve(&import.path, modules) else {
            continue;
        };
        depend(&speaks_as, &target.id);
        match &import.binding {
            Binding::Alias(alias) => {
                qualifiers.insert(alias.as_str(), target);
            }
            Binding::PackageName => {
                if let Some(name) = package_names.of(&target.directory) {
                    qualifiers.insert(name, target);
                }
            }
            Binding::Unbound => {}
        }
    }

    for reference in &file.references {
        let Some(to) = resolve_reference(reference, directory, declarations, &qualifiers) else {
            continue;
        };
        let from = file
            .attributions
            .speaker_at(reference.span.start)
            .cloned()
            .unwrap_or_else(|| speaks_as.clone());
        depend(&from, &to);
    }
}

/// Where one written name leads. A qualified name reads against the file's
/// imports alone, because only an import binds a qualifier that stands for a
/// directory. A bare name is the file's own directory, whose files share one
/// namespace: a declaration beside it needs no import to reach.
fn resolve_reference(
    reference: &Reference,
    directory: &Directory,
    declarations: &DeclarationIndex,
    qualifiers: &BTreeMap<&str, ResolvedImport>,
) -> Option<ElementId> {
    let (target_directory, target_id) = match &reference.qualifier {
        Some(qualifier) => {
            let target = qualifiers.get(qualifier.as_str())?;
            (target.directory.as_str(), target.id.clone())
        }
        None => (directory.path(), directory.id()),
    };
    Some(
        match declarations.declaration(target_directory, &reference.name) {
            Some(declaration) if declaration.exported => declaration.id.clone(),
            // An unexported or unknown name is the target directory's own
            // business; the dependency reaches no further than the directory.
            // For a bare name that directory is the file's own, so the
            // containment guard drops the edge and the name says nothing.
            _ => target_id,
        },
    )
}

/// Everything the layout of the sources alone says, before any file is read:
/// the modules and the directories that hold code. The files stand in the
/// picture because the tree holds them, not because Go reads anything into
/// them.
fn structure(modules: &[DiscoveredModule], catalog: &DirectoryCatalog) -> Vec<Interpretation> {
    let mut interpretations: Vec<Interpretation> = modules
        .iter()
        .map(|module| Interpretation {
            element: module_element(module),
            extent: module_extent(module),
        })
        .collect();
    interpretations.extend(catalog.directories().filter_map(Directory::interpretation));
    interpretations
}

/// What a go.mod module reads: the directory its manifest sits in - the
/// whole repository for a manifest at the root, which is the common case,
/// where the whole tree nests inside the module.
fn module_extent(module: &DiscoveredModule) -> Extent {
    Extent::directory(
        DirectoryPath::new(&module.dir).expect("a manifest directory carries no trailing slash"),
    )
}

/// Whether the go tool treats the file as test-only code.
fn is_test_file(path: &SourcePath) -> bool {
    path.as_str().ends_with("_test.go")
}

/// The syntax tree of one file, and the gap where the grammar lost the
/// thread.
///
/// A syntax error is no reason to drop a file: tree-sitter answers with a
/// whole tree whatever it meets, marking what it could not read and parsing
/// everything around it, so the file still contributes what it holds and the
/// gap declares the rest.
fn parse(text: &str, path: &SourcePath) -> (tree_sitter::Tree, Option<AnalysisGap>) {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_go::LANGUAGE.into())
        .expect("the bundled Go grammar matches the linked tree-sitter version");
    // tree-sitter answers with nothing only when no language is set - ruled
    // out by the line above - or when a cancellation flag or a timeout it was
    // never given fires. Any of those is this adapter miswiring the parser,
    // never something a source file can do.
    let tree = parser
        .parse(text, None)
        .expect("a parser with a language set and no deadline always answers");
    let gap = unreadable_regions(tree.root_node()).map(|(first, regions)| AnalysisGap {
        path: path.clone(),
        reason: GapReason::SyntaxErrors {
            // tree-sitter counts rows and columns from zero; a reader hunting
            // the construct counts from one.
            line: first.row + 1,
            column: first.column + 1,
            regions,
        },
    });
    (tree, gap)
}

/// Where the grammar first lost the thread, and how many such regions the
/// file holds.
///
/// tree-sitter marks what it could not read as an error node and what it had
/// to invent to keep going as a missing one. A region is the outermost of
/// either: what stands inside a broken region was recovered by guesswork
/// rather than read, so it is part of the same wound and not another one.
fn unreadable_regions(root: tree_sitter::Node<'_>) -> Option<(tree_sitter::Point, usize)> {
    let mut found = Vec::new();
    collect_unreadable(root, &mut found);
    Some((*found.first()?, found.len()))
}

fn collect_unreadable(node: tree_sitter::Node<'_>, out: &mut Vec<tree_sitter::Point>) {
    if node.is_error() || node.is_missing() {
        out.push(node.start_position());
        return;
    }
    // A subtree holding nothing broken cannot hide a broken region.
    if !node.has_error() {
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_unreadable(child, out);
    }
}

fn module_element(module: &DiscoveredModule) -> Element {
    Element::semantic(
        module_id(module),
        SemanticKind::Package,
        ElementName::new(&module.path).expect("a module path is never empty"),
    )
}

pub(crate) fn module_id(module: &DiscoveredModule) -> ElementId {
    ElementId::new(format!("package:{}", module.path)).expect("a module path is never empty")
}

#[cfg(test)]
mod tests;
