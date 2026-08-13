//! Path extraction: every name a file imports through its `use`
//! declarations, and every path the rest of the file mentions. Both witness
//! dependencies - code that writes `engine::physics::step()` depends on it
//! whether or not a `use` brought the name in.

use std::ops::Range;

/// One name a `use` declaration brings into its module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    /// The path as written, split into segments: `use a::b::C as D` gives
    /// `[a, b, C]`, `use a::e::*` gives `[a, e]`.
    pub path: Vec<String>,
    /// The name the import binds in its own module. A wildcard binds every
    /// public name of its target rather than one, so it binds no name here.
    pub binding: Option<String>,
    /// True for a top-level `pub use`: the binding joins the surface of the
    /// module, so other modules reach the target by importing it from here.
    pub reexport: bool,
}

/// Collects every `use` declaration of the file, at any nesting depth.
/// `use a::{b, c as d, e::*};` yields the paths `[a, b]`, `[a, c]`, and
/// `[a, e]`, binding `b`, `d`, and nothing.
pub fn declared(root: tree_sitter::Node<'_>, text: &str) -> Vec<Import> {
    let mut declarations = Vec::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        collect_use_declarations(child, true, &mut declarations);
    }
    declarations
        .into_iter()
        .flat_map(|(node, at_module_level)| {
            // Only a `pub use` written by the file itself re-exports; one
            // nested in an inline module or a function belongs to that scope.
            let reexport = at_module_level && is_public(node);
            let leaves = node
                .child_by_field_name("argument")
                .map_or_else(Vec::new, |argument| leaves_of(argument, text));
            leaves
                .into_iter()
                .map(move |leaf| leaf.into_import(reexport))
        })
        .collect()
}

/// One path the file mentions outside its `use` declarations, and where in
/// the file it stands, so the mention attributes to the declaration that
/// writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub path: Vec<String>,
    pub span: Range<usize>,
}

/// Collects every path the file mentions outside its `use` declarations:
/// qualified paths in expressions, type positions, attributes, and macro
/// names; the bare names calls and type positions speak. Paths inside macro
/// arguments are token soup to the parser and stay unseen.
///
/// A bare name is collected only where the grammar guarantees it names a
/// declaration - a call's callee, a type position - never as a plain
/// mention, so most local variables stay out. A local binding that shadows
/// a top-level name in one of those positions is still read as the
/// top-level name: telling them apart needs scope tracking, and the
/// imprecision is accepted the way qualifier shadowing is elsewhere.
///
/// A collected path that is a strict prefix of another at the same site is
/// dropped: the syntax tree nests a path inside every longer path over it,
/// and the longest one witnesses the same dependency most precisely.
pub fn referenced(root: tree_sitter::Node<'_>, text: &str) -> Vec<Reference> {
    let mut references = Vec::new();
    collect_referenced(root, text, &mut references);
    let all = references.clone();
    references.retain(|reference| {
        !all.iter().any(|other| {
            other.path.len() > reference.path.len()
                && other.path[..reference.path.len()] == reference.path[..]
                && other.span.start <= reference.span.start
                && reference.span.end <= other.span.end
        })
    });
    references
}

fn collect_referenced(node: tree_sitter::Node<'_>, text: &str, out: &mut Vec<Reference>) {
    // Error recovery nests well-formed-looking nodes inside a region the
    // grammar could not read, and what those nodes mean is anybody's guess.
    // A broken region witnesses no dependency.
    if node.is_error() {
        return;
    }
    let segment = |n: tree_sitter::Node<'_>| {
        n.utf8_text(text.as_bytes())
            .expect("node ranges lie within the parsed text")
            .to_owned()
    };
    match node.kind() {
        // `use` paths carry binding and re-export semantics; `declared`
        // reads them.
        "use_declaration" => return,
        "scoped_identifier" | "scoped_type_identifier" => {
            out.extend(leaves_of(node, text).into_iter().map(|leaf| Reference {
                path: leaf.path,
                span: node.byte_range(),
            }));
        }
        // A callee is a reference by position: `helper()` names a function
        // wherever `helper` leads. Only the bare form needs collecting here;
        // a scoped callee is a scoped path like any other.
        "call_expression" => {
            if let Some(callee) = node.child_by_field_name("function") {
                let callee = if callee.kind() == "generic_function" {
                    callee.child_by_field_name("function").unwrap_or(callee)
                } else {
                    callee
                };
                if callee.kind() == "identifier" {
                    out.push(Reference {
                        path: vec![segment(callee)],
                        span: callee.byte_range(),
                    });
                }
            }
        }
        // The grammar spells a type usage as its own node kind, so every
        // bare `type_identifier` is a reference - except the name a
        // declaration introduces, a generic parameter, and the tail of a
        // scoped path the scoped arm already collected whole.
        "type_identifier" => {
            let introduces = node.parent().is_some_and(|parent| {
                matches!(parent.kind(), "type_parameter" | "scoped_type_identifier")
                    || (matches!(
                        parent.kind(),
                        "struct_item" | "enum_item" | "trait_item" | "union_item" | "type_item"
                    ) && parent.child_by_field_name("name") == Some(node))
            });
            if !introduces {
                out.push(Reference {
                    path: vec![segment(node)],
                    span: node.byte_range(),
                });
            }
        }
        _ => {}
    }
    // The walk descends into scoped paths too: their own path child only
    // repeats a prefix (dropped above), while turbofish arguments hold
    // genuinely new paths.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_referenced(child, text, out);
    }
}

fn collect_use_declarations<'tree>(
    node: tree_sitter::Node<'tree>,
    at_module_level: bool,
    found: &mut Vec<(tree_sitter::Node<'tree>, bool)>,
) {
    // What a region the grammar could not read seems to import is a guess of
    // the error recovery, not a `use` the file writes.
    if node.is_error() {
        return;
    }
    if node.kind() == "use_declaration" {
        found.push((node, at_module_level));
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_use_declarations(child, false, found);
    }
}

fn is_public(node: tree_sitter::Node<'_>) -> bool {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| child.kind() == "visibility_modifier")
}

/// One leaf of a use-tree, before the bound name is worked out.
struct Leaf {
    path: Vec<String>,
    alias: Option<String>,
    wildcard: bool,
}

impl Leaf {
    fn of(path: Vec<String>) -> Self {
        Self {
            path,
            alias: None,
            wildcard: false,
        }
    }

    fn into_import(self, reexport: bool) -> Import {
        let binding = if self.wildcard {
            None
        } else {
            self.alias.clone().or_else(|| {
                // `use a::{self, B}` binds `a`: the trailing `self` names the
                // module the list hangs from, not a name of its own.
                self.path
                    .iter()
                    .rev()
                    .find(|segment| segment.as_str() != "self")
                    .cloned()
            })
        };
        Import {
            path: self.path,
            binding,
            reexport,
        }
    }
}

fn leaves_of(node: tree_sitter::Node<'_>, text: &str) -> Vec<Leaf> {
    let segment = |n: tree_sitter::Node<'_>| {
        n.utf8_text(text.as_bytes())
            .expect("node ranges lie within the parsed text")
            .to_owned()
    };
    match node.kind() {
        "identifier" | "crate" | "super" | "self" | "metavariable" => {
            vec![Leaf::of(vec![segment(node)])]
        }
        "scoped_identifier" | "scoped_type_identifier" => {
            let name = node.child_by_field_name("name").map(segment);
            let mut bases = node
                .child_by_field_name("path")
                .map_or_else(|| vec![Leaf::of(Vec::new())], |path| leaves_of(path, text));
            if let Some(name) = name {
                for base in &mut bases {
                    base.path.push(name.clone());
                }
            }
            bases
        }
        "scoped_use_list" => {
            let bases: Vec<Vec<String>> = node.child_by_field_name("path").map_or_else(
                || vec![Vec::new()],
                |path| {
                    leaves_of(path, text)
                        .into_iter()
                        .map(|leaf| leaf.path)
                        .collect()
                },
            );
            let mut result = Vec::new();
            if let Some(list) = node.child_by_field_name("list") {
                let mut cursor = list.walk();
                for entry in list.named_children(&mut cursor) {
                    for leaf in leaves_of(entry, text) {
                        for base in &bases {
                            let mut combined = base.clone();
                            combined.extend(leaf.path.iter().cloned());
                            result.push(Leaf {
                                path: combined,
                                alias: leaf.alias.clone(),
                                wildcard: leaf.wildcard,
                            });
                        }
                    }
                }
            }
            result
        }
        "use_list" => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .flat_map(|entry| leaves_of(entry, text))
                .collect()
        }
        "use_as_clause" => {
            let alias = node.child_by_field_name("alias").map(segment);
            let mut leaves = node
                .child_by_field_name("path")
                .map_or_else(Vec::new, |path| leaves_of(path, text));
            for leaf in &mut leaves {
                leaf.alias.clone_from(&alias);
            }
            leaves
        }
        "use_wildcard" => {
            let mut cursor = node.walk();
            let mut leaves = node
                .named_children(&mut cursor)
                .next()
                .map_or_else(|| vec![Leaf::of(Vec::new())], |path| leaves_of(path, text));
            for leaf in &mut leaves {
                leaf.wildcard = true;
            }
            leaves
        }
        _ => Vec::new(),
    }
}
