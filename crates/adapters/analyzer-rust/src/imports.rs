//! `use` declaration extraction: every name a file imports, the path that
//! name points at, and whether the file re-exports it.

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

fn collect_use_declarations<'tree>(
    node: tree_sitter::Node<'tree>,
    at_module_level: bool,
    found: &mut Vec<(tree_sitter::Node<'tree>, bool)>,
) {
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
