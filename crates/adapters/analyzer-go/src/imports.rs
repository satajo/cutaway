//! Import extraction: which directories a file imports, under which
//! qualifier each import binds, and every qualified name the rest of the file
//! mentions.
//!
//! Go forces the two apart. An import states the coupling between two
//! directories; the qualified name states which declaration of the imported
//! directory the code actually touches. Both are facts about the same
//! sources, and both witness a dependency.

use std::collections::BTreeMap;

/// One directory a file imports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    /// The import path as written, without its quotes.
    pub path: String,
    pub binding: Binding,
}

/// The name an import binds in the importing file. Every use of imported
/// code goes through it, so it is what turns `server.Handler` back into a
/// directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    /// No alias: the import binds the `package` clause name of the target
    /// directory, which only that directory's own sources tell.
    PackageName,
    /// `import alias "path"`.
    Alias(String),
    /// `import . "path"` merges the target into the file's own namespace and
    /// `import _ "path"` binds nothing at all. Neither leaves a qualifier
    /// behind, so the import itself is the only dependency they witness.
    Unbound,
}

/// The `package` clause name of every directory, which an import binds when
/// it gives no alias.
///
/// Only the directory's own sources tell this name, and it may differ from
/// the last segment of the import path, so every file is read before any
/// qualifier resolves. External test files (`package foo_test` beside
/// `package foo`) name a namespace nothing can import, so they do not speak
/// for the directory.
#[derive(Debug, Default)]
pub struct PackageNames(BTreeMap<String, String>);

impl PackageNames {
    pub fn add(&mut self, directory: &str, name: &str) {
        self.0
            .entry(directory.to_owned())
            .or_insert_with(|| name.to_owned());
    }

    pub fn of(&self, directory: &str) -> Option<&str> {
        self.0.get(directory).map(String::as_str)
    }
}

/// The name this file's directory answers to in an import that does not
/// rename it.
pub fn package_clause<'a>(root: tree_sitter::Node<'_>, text: &'a str) -> Option<&'a str> {
    let mut cursor = root.walk();
    let clause = root
        .named_children(&mut cursor)
        .find(|node| node.kind() == "package_clause")?;
    let mut names = clause.walk();
    let name = clause
        .named_children(&mut names)
        .find(|node| node.kind() == "package_identifier")?;
    Some(
        name.utf8_text(text.as_bytes())
            .expect("node ranges lie within the parsed text"),
    )
}

/// Collects every import of the file, from single imports and from factored
/// `import ( ... )` blocks alike.
pub fn declared(root: tree_sitter::Node<'_>, text: &str) -> Vec<Import> {
    let mut imports = Vec::new();
    let mut cursor = root.walk();
    for declaration in root.named_children(&mut cursor) {
        if declaration.kind() != "import_declaration" {
            continue;
        }
        let mut children = declaration.walk();
        for node in declaration.named_children(&mut children) {
            match node.kind() {
                "import_spec" => imports.extend(spec(node, text)),
                "import_spec_list" => {
                    let mut specs = node.walk();
                    imports.extend(
                        node.named_children(&mut specs)
                            .filter(|spec| spec.kind() == "import_spec")
                            .filter_map(|node| spec(node, text)),
                    );
                }
                _ => {}
            }
        }
    }
    imports
}

fn spec(node: tree_sitter::Node<'_>, text: &str) -> Option<Import> {
    let literal = node.child_by_field_name("path")?;
    // The literal's content child holds the path without the surrounding
    // quotes or backticks.
    let mut cursor = literal.walk();
    let path = literal
        .named_children(&mut cursor)
        .next()
        .map(|content| segment(content, text))?;
    let binding = match node.child_by_field_name("name") {
        None => Binding::PackageName,
        Some(name) if name.kind() == "package_identifier" => Binding::Alias(segment(name, text)),
        Some(_) => Binding::Unbound,
    };
    Some(Import { path, binding })
}

/// Collects every qualified name the file mentions: the qualifier and the
/// name behind the dot.
///
/// Go writes the same qualification two ways. In an expression it is a
/// selector whose operand may be any value, so only a plain identifier can
/// be a directory's qualifier and `x.field.Method()` witnesses `x.field` and
/// nothing deeper. In a type position it is a qualified type, where the
/// qualifier is always a package name.
pub fn referenced(root: tree_sitter::Node<'_>, text: &str) -> Vec<(String, String)> {
    let mut references = Vec::new();
    collect_referenced(root, text, &mut references);
    references
}

fn collect_referenced(node: tree_sitter::Node<'_>, text: &str, out: &mut Vec<(String, String)>) {
    let mut cursor = node.walk();
    match node.kind() {
        "selector_expression" => {
            if let (Some(operand), Some(field)) = (
                node.child_by_field_name("operand"),
                node.child_by_field_name("field"),
            ) && operand.kind() == "identifier"
            {
                out.push((segment(operand, text), segment(field, text)));
            }
        }
        "qualified_type" => {
            let mut parts = node.named_children(&mut cursor);
            if let (Some(qualifier), Some(name)) = (parts.next(), parts.next()) {
                out.push((segment(qualifier, text), segment(name, text)));
            }
        }
        _ => {}
    }
    let mut children = node.walk();
    for child in node.named_children(&mut children) {
        collect_referenced(child, text, out);
    }
}

fn segment(node: tree_sitter::Node<'_>, text: &str) -> String {
    node.utf8_text(text.as_bytes())
        .expect("node ranges lie within the parsed text")
        .to_owned()
}
