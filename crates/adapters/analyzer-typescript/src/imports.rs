//! Specifier extraction: which modules a file names, which names it takes
//! from each, and the qualified references a namespace import leaves behind.
//!
//! Every specifier witnesses a dependency, whatever form it takes: a static
//! import, a type-only import (type coupling is coupling), a re-export, a
//! dynamic `import()`, and a `require()` all state that one module needs
//! another. The names taken from a specifier state which part of it the code
//! actually touches, which is a fact of its own.

use crate::text_of;

/// One statement that names another module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    /// The specifier as written, without its quotes.
    pub specifier: String,
    /// The names the statement takes from the target, as the target offers
    /// them. A default import takes `default`.
    pub names: Vec<String>,
    /// The name an `import * as ns` binds for the rest of the file.
    pub namespace: Option<String>,
    /// What the statement puts on its own file's surface.
    pub reexports: Vec<Reexport>,
    /// True for `export * from "..."`: the target answers for any name this
    /// file does not declare itself.
    pub wildcard_reexport: bool,
}

impl Import {
    fn of(specifier: String) -> Self {
        Self {
            specifier,
            names: Vec::new(),
            namespace: None,
            reexports: Vec::new(),
            wildcard_reexport: false,
        }
    }
}

/// One name a file offers while the declaration behind it lives elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reexport {
    /// The name the re-exporting file offers.
    pub name: String,
    /// The name the target offers, which an alias may rename.
    pub from: String,
}

/// Collects every specifier the file names. Import and export statements
/// carry the names they move across the boundary; a dynamic `import()` or a
/// `require()` carries only the specifier, wherever in the file it stands.
pub fn declared(root: tree_sitter::Node<'_>, text: &str) -> Vec<Import> {
    let mut imports = Vec::new();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        let statement = match node.kind() {
            "import_statement" => imported(node, text),
            "export_statement" => reexported(node, text),
            _ => None,
        };
        imports.extend(statement);
    }
    collect_calls(root, text, &mut imports);
    imports
}

fn imported(node: tree_sitter::Node<'_>, text: &str) -> Option<Import> {
    // `import x = require("...")` carries the specifier on its clause rather
    // than on the statement.
    let source = node
        .child_by_field_name("source")
        .or_else(|| child_of_kind(node, "import_require_clause")?.child_by_field_name("source"))?;
    let mut import = Import::of(literal(source, text)?);
    let Some(clause) = child_of_kind(node, "import_clause") else {
        return Some(import);
    };
    let mut cursor = clause.walk();
    for child in clause.named_children(&mut cursor) {
        match child.kind() {
            // A default import binds whatever the target exports as its
            // default.
            "identifier" => import.names.push("default".to_owned()),
            "namespace_import" => {
                let mut names = child.walk();
                import.namespace = child
                    .named_children(&mut names)
                    .next()
                    .map(|name| text_of(name, text));
            }
            "named_imports" => {
                let mut specifiers = child.walk();
                import.names.extend(
                    child
                        .named_children(&mut specifiers)
                        .filter(|specifier| specifier.kind() == "import_specifier")
                        .filter_map(|specifier| specifier.child_by_field_name("name"))
                        .map(|name| text_of(name, text)),
                );
            }
            _ => {}
        }
    }
    Some(import)
}

fn reexported(node: tree_sitter::Node<'_>, text: &str) -> Option<Import> {
    let source = node.child_by_field_name("source")?;
    let mut import = Import::of(literal(source, text)?);
    let Some(clause) = child_of_kind(node, "export_clause") else {
        // `export * as ns from "..."` offers the target under one name and no
        // name of the target's own; `export * from "..."` offers every name
        // the target holds.
        import.wildcard_reexport = child_of_kind(node, "namespace_export").is_none();
        return Some(import);
    };
    let mut cursor = clause.walk();
    for specifier in clause
        .named_children(&mut cursor)
        .filter(|child| child.kind() == "export_specifier")
    {
        let Some(from) = specifier
            .child_by_field_name("name")
            .map(|name| text_of(name, text))
        else {
            continue;
        };
        let name = specifier
            .child_by_field_name("alias")
            .map_or_else(|| from.clone(), |alias| text_of(alias, text));
        import.names.push(from.clone());
        import.reexports.push(Reexport { name, from });
    }
    Some(import)
}

/// Collects the specifiers that reach the module system through a call:
/// a dynamic `import("...")` and a `require("...")`. Only a string literal
/// names a module the sources can witness; a computed specifier is decided at
/// runtime and states nothing.
fn collect_calls(node: tree_sitter::Node<'_>, text: &str, out: &mut Vec<Import>) {
    if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
        && (function.kind() == "import"
            || (function.kind() == "identifier" && text_of(function, text) == "require"))
        && let Some(arguments) = node.child_by_field_name("arguments")
    {
        let mut cursor = arguments.walk();
        if let Some(argument) = arguments.named_children(&mut cursor).next()
            && let Some(specifier) = literal(argument, text)
        {
            out.push(Import::of(specifier));
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_calls(child, text, out);
    }
}

/// Collects every qualified name the file mentions: the qualifier and the
/// name behind the dot. A namespace import binds the qualifier, so
/// `ns.Widget` states which part of the imported module the code touches.
///
/// The ecosystem writes the qualification two ways. In an expression it is a
/// member access whose object may be any value, so only a plain identifier
/// can be a namespace qualifier and `x.field.method()` witnesses `x.field`
/// and nothing deeper. In a type position it is a nested type name, whose
/// qualifier is always a namespace.
pub fn referenced(root: tree_sitter::Node<'_>, text: &str) -> Vec<(String, String)> {
    let mut references = Vec::new();
    collect_referenced(root, text, &mut references);
    references
}

fn collect_referenced(node: tree_sitter::Node<'_>, text: &str, out: &mut Vec<(String, String)>) {
    match node.kind() {
        "member_expression" => {
            if let (Some(object), Some(property)) = (
                node.child_by_field_name("object"),
                node.child_by_field_name("property"),
            ) && object.kind() == "identifier"
            {
                out.push((text_of(object, text), text_of(property, text)));
            }
        }
        "nested_type_identifier" => {
            if let (Some(module), Some(name)) = (
                node.child_by_field_name("module"),
                node.child_by_field_name("name"),
            ) && module.kind() == "identifier"
            {
                out.push((text_of(module, text), text_of(name, text)));
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_referenced(child, text, out);
    }
}

/// The text of a string literal, without its quotes. An empty literal names
/// no module.
fn literal(node: tree_sitter::Node<'_>, text: &str) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == "string_fragment")
        .map(|fragment| text_of(fragment, text))
}

fn child_of_kind<'tree>(
    node: tree_sitter::Node<'tree>,
    kind: &str,
) -> Option<tree_sitter::Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}
