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
//! items of their enclosing file. A module whose name also names a directory
//! reads the file and that directory as one boundary, so what lies in `foo/`
//! belongs to the module `foo`. Only declarations with a visibility modifier
//! become items: a bare declaration is the module's internals, and a path
//! that names one lands on the module. The crate root file is the package's
//! own code rather than a module of its own, and so is every standalone
//! crate root (`tests/`, `benches/`, `examples/`, `build.rs`, `src/bin/`):
//! such a file reads nothing, it stands where the tree puts it holding the
//! declarations written in it, and an import that resolves to a crate root
//! lands on the package. Imports resolve down to the deepest
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
    AnalysisGap, Extent, GapReason, Interpretation, SourceAnalyzer, SourceStructure,
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
    fn analyze(&self, files: &[SourceFile]) -> SourceStructure {
        let (packages, mut gaps) = manifest::discover_packages(files);
        let (catalog, contested) = ModuleCatalog::build(&packages, files);
        gaps.extend(contested);

        let mut interpretations = Vec::new();
        let mut relations = BTreeSet::new();

        for package in &packages {
            interpretations.push(Interpretation {
                element: package_element(package),
                extent: package_extent(package),
            });
        }

        interpretations.extend(
            catalog
                .modules()
                .filter_map(modules::Module::interpretation),
        );

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

            witness_dependencies(&file, module, &catalog, &packages, surface, &mut relations);
        }

        SourceStructure {
            interpretations,
            relations: relations.into_iter().collect(),
            gaps,
        }
    }
}

/// What a package reads: the directory its manifest sits in - the whole
/// repository for a manifest at the root, because a repository whose root
/// manifest names a package is that package's territory.
fn package_extent(package: &DiscoveredPackage) -> Extent {
    Extent::directory(
        DirectoryPath::new(&package.dir).expect("a manifest directory carries no trailing slash"),
    )
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
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .expect("the bundled Rust grammar matches the linked tree-sitter version");
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

#[cfg(test)]
mod tests;
