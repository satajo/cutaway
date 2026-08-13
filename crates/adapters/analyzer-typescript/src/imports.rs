//! Specifier extraction: which modules a file names, which names it takes
//! from each, and the names the rest of the file writes.
//!
//! Every specifier witnesses a dependency, whatever form it takes: a static
//! import, a type-only import (type coupling is coupling), a re-export, a
//! dynamic `import()`, and a `require()` all state that one module needs
//! another. The names taken from a specifier state which part of it the code
//! actually touches, which is a fact of its own.

use std::ops::Range;

use crate::text_of;

/// One statement that names another module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    /// The specifier as written, without its quotes.
    pub specifier: String,
    /// The names the statement takes from the target.
    pub names: Vec<ImportedName>,
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

/// One name a statement moves across a module boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedName {
    /// The name as the target offers it. A default import takes `default`.
    pub name: String,
    /// The name the importing file writes for it, when the statement binds one
    /// in the file's own scope: an alias renames what it binds, and a
    /// re-export binds nothing locally at all.
    pub local: Option<String>,
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
            // default, under a name of the importing file's choosing.
            "identifier" => import.names.push(ImportedName {
                name: "default".to_owned(),
                local: Some(text_of(child, text)),
            }),
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
                        .filter_map(|specifier| {
                            let name = text_of(specifier.child_by_field_name("name")?, text);
                            // `import { a as b }` takes `a` and binds `b`;
                            // without an alias the two are the same name.
                            let local = specifier
                                .child_by_field_name("alias")
                                .map_or_else(|| name.clone(), |alias| text_of(alias, text));
                            Some(ImportedName {
                                name,
                                local: Some(local),
                            })
                        }),
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
        import.names.push(ImportedName {
            name: from.clone(),
            local: None,
        });
        import.reexports.push(Reexport { name, from });
    }
    Some(import)
}

/// Collects the specifiers that reach the module system through a call:
/// a dynamic `import("...")` and a `require("...")`. Only a string literal
/// names a module the sources can witness; a computed specifier is decided at
/// runtime and states nothing.
fn collect_calls(node: tree_sitter::Node<'_>, text: &str, out: &mut Vec<Import>) {
    // What a region the grammar could not read seems to require is a guess of
    // the error recovery, not a specifier the file names.
    if node.is_error() {
        return;
    }
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

/// One name the file mentions outside its import statements, and where in the
/// file it stands, so the mention attributes to the declaration that writes
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// The qualifier the name was written behind, when it was written behind
    /// one. A namespace import binds such a qualifier.
    pub qualifier: Option<String>,
    pub name: String,
    pub span: Range<usize>,
}

/// Collects every name the file mentions outside its import statements: the
/// qualified ones as a qualifier and the name behind the dot, and the bare
/// ones the code writes where a name can only mean a declaration.
///
/// The ecosystem writes the qualification two ways. In an expression it is a
/// member access whose object may be any value, so only a plain identifier
/// can be a namespace qualifier and `x.field.method()` witnesses `x.field`
/// and nothing deeper. In a type position it is a nested type name, whose
/// qualifier is always a namespace.
///
/// A bare name is collected only where the grammar guarantees it names a
/// declaration - a call's callee, a constructor, a heritage clause, a type
/// position, a JSX element - never as a plain mention, so ordinary values
/// stay out. A local binding or a type parameter that shadows an imported or
/// top-level name in one of those positions is still read as that name:
/// telling them apart needs scope tracking, and the imprecision is accepted
/// the way qualifier shadowing is.
pub fn referenced(root: tree_sitter::Node<'_>, text: &str) -> Vec<Reference> {
    let mut references = Vec::new();
    collect_referenced(root, text, &mut references);
    references
}

fn collect_referenced(node: tree_sitter::Node<'_>, text: &str, out: &mut Vec<Reference>) {
    // Error recovery nests well-formed-looking nodes inside a region the
    // grammar could not read, and what those nodes mean is anybody's guess.
    // A broken region witnesses no dependency.
    if node.is_error() {
        return;
    }
    let bare = |name: tree_sitter::Node<'_>| Reference {
        qualifier: None,
        name: text_of(name, text),
        span: name.byte_range(),
    };
    match node.kind() {
        // Import statements carry binding and re-export semantics; `declared`
        // reads them, and they speak for the module rather than for any
        // declaration.
        "import_statement" => return,
        "member_expression" => {
            if let (Some(object), Some(property)) = (
                node.child_by_field_name("object"),
                node.child_by_field_name("property"),
            ) && object.kind() == "identifier"
            {
                out.push(Reference {
                    qualifier: Some(text_of(object, text)),
                    name: text_of(property, text),
                    span: node.byte_range(),
                });
            }
        }
        "nested_type_identifier" => {
            if let (Some(module), Some(name)) = (
                node.child_by_field_name("module"),
                node.child_by_field_name("name"),
            ) && module.kind() == "identifier"
            {
                out.push(Reference {
                    qualifier: Some(text_of(module, text)),
                    name: text_of(name, text),
                    span: node.byte_range(),
                });
            }
        }
        // A callee and a constructor are references by position: `helper()`
        // and `new Widget()` name a declaration wherever the name leads. Only
        // the bare form needs collecting here; a qualified one is a member
        // expression the arm above already read.
        "call_expression" | "new_expression" => {
            let field = if node.kind() == "call_expression" {
                "function"
            } else {
                "constructor"
            };
            if let Some(callee) = node.child_by_field_name(field)
                && callee.kind() == "identifier"
            {
                out.push(bare(callee));
            }
        }
        // `extends Base` on a class names a value, so the grammar spells it as
        // an identifier rather than a type. An `extends mixin(Base)` is a call
        // like any other, and `implements` takes types the type arm reads.
        "extends_clause" => {
            if let Some(value) = node.child_by_field_name("value")
                && value.kind() == "identifier"
            {
                out.push(bare(value));
            }
        }
        // The grammar spells a type usage as its own node kind, so every bare
        // `type_identifier` is a reference - except the name a declaration
        // introduces, a type parameter, and the tail of a qualified type name
        // the arm above already read whole.
        "type_identifier" => {
            let introduces = node.parent().is_some_and(|parent| {
                matches!(parent.kind(), "type_parameter" | "nested_type_identifier")
                    || (matches!(
                        parent.kind(),
                        "class"
                            | "class_declaration"
                            | "abstract_class_declaration"
                            | "interface_declaration"
                            | "type_alias_declaration"
                    ) && parent.child_by_field_name("name") == Some(node))
            });
            if !introduces {
                out.push(bare(node));
            }
        }
        // A JSX element names the component it renders, which is the plainest
        // dependency a component has. Only a capitalized name can name one: the
        // ecosystem reserves the lowercase names for the markup tags of the
        // host, which are no declaration of anybody's.
        "jsx_opening_element" | "jsx_self_closing_element" => {
            if let Some(name) = node.child_by_field_name("name")
                && name.kind() == "identifier"
                && text_of(name, text)
                    .chars()
                    .next()
                    .is_some_and(char::is_uppercase)
            {
                out.push(bare(name));
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
