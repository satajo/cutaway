//! Top-level declarations of one source file, and what the directory they
//! sit in offers to the rest of the project.

use std::collections::BTreeMap;

use cutaway_architecture::{Element, ElementId, ElementKind, ElementName};
use cutaway_inspection::ports::source_tree::SourcePath;

/// One top-level declaration and how far it reaches.
#[derive(Debug, Clone)]
pub struct Declaration {
    pub element: Element,
    /// True when the name starts with an upper-case letter: Go's own rule
    /// for what leaves the package. An unexported declaration is the
    /// directory's internals - code of the same directory may name it, but
    /// the architecture shows the directory, not the item.
    pub exported: bool,
}

/// What the index answers about one declared name.
#[derive(Debug)]
pub struct IndexedDeclaration {
    pub id: ElementId,
    pub exported: bool,
}

/// Looks up a top-level declaration by the directory that declares it and by
/// name, so that a qualified reference resolves onto the declared item. The
/// key is the directory rather than the file because the files of one Go
/// directory share a single namespace: an importer names the directory and
/// never learns which file answered.
///
/// Unexported declarations are indexed too: resolution must know that the
/// name lands in this directory, even though the item itself stays out of
/// the architecture. When one name is declared twice in a directory, the
/// first declaration in source order answers.
#[derive(Debug, Default)]
pub struct DeclarationIndex(BTreeMap<(String, String), IndexedDeclaration>);

impl DeclarationIndex {
    pub fn add(&mut self, directory: &str, declarations: &[Declaration]) {
        for declaration in declarations {
            self.0
                .entry((
                    directory.to_owned(),
                    declaration.element.name.as_str().to_owned(),
                ))
                .or_insert_with(|| IndexedDeclaration {
                    id: declaration.element.id.clone(),
                    exported: declaration.exported,
                });
        }
    }

    pub fn declaration(&self, directory: &str, name: &str) -> Option<&IndexedDeclaration> {
        self.0.get(&(directory.to_owned(), name.to_owned()))
    }
}

/// The top-level functions and types a file declares.
///
/// Methods carry a receiver and belong to the type they extend, not to the
/// directory's namespace, so they are no items of their own. Constants and
/// variables stay out for the same reason the Rust adapter keeps them out:
/// the architecture speaks about behaviour and shape, not about values.
pub fn top_level(root: tree_sitter::Node<'_>, text: &str, path: &SourcePath) -> Vec<Declaration> {
    let mut cursor = root.walk();
    let mut declarations = Vec::new();
    for node in root.named_children(&mut cursor) {
        match node.kind() {
            "function_declaration" => {
                if let Some(declared) = declared(node, text, path, ElementKind::Function) {
                    declarations.push(declared);
                }
            }
            // One `type ( ... )` block declares several types, each in its
            // own spec; a lone `type X = Y` is an alias spec.
            "type_declaration" => {
                let mut specs = node.walk();
                for spec in node.named_children(&mut specs) {
                    if !matches!(spec.kind(), "type_spec" | "type_alias") {
                        continue;
                    }
                    if let Some(declared) = declared(spec, text, path, ElementKind::Type) {
                        declarations.push(declared);
                    }
                }
            }
            _ => {}
        }
    }
    declarations
}

fn declared(
    node: tree_sitter::Node<'_>,
    text: &str,
    path: &SourcePath,
    kind: ElementKind,
) -> Option<Declaration> {
    let name = node
        .child_by_field_name("name")?
        .utf8_text(text.as_bytes())
        .expect("node ranges lie within the parsed text");
    Some(Declaration {
        element: Element {
            id: declaration_id(path, kind, name),
            name: ElementName::new(name).expect("a parsed identifier is never empty"),
            kind,
        },
        exported: name.chars().next().is_some_and(char::is_uppercase),
    })
}

/// Declaration ids embed the path and the kind so that same-named
/// declarations in different files, or in different namespaces of the same
/// file, stay distinct.
fn declaration_id(path: &SourcePath, kind: ElementKind, name: &str) -> ElementId {
    let tag = match kind {
        ElementKind::Project => "project",
        ElementKind::Package => "package",
        ElementKind::Directory => "directory",
        ElementKind::Module => "module",
        ElementKind::Function => "function",
        ElementKind::Type => "type",
    };
    ElementId::new(format!("{path}#{tag}:{name}")).expect("the id embeds a non-empty path")
}
