//! `use` declaration extraction: every import path a file mentions, as raw
//! segment lists ready for resolution against the module catalog.

/// Collects the segment paths of every `use` declaration in the file, at any
/// nesting depth. `use a::{b, c as d, e::*};` yields `[a, b]`, `[a, c]`, and
/// `[a, e]`.
pub fn use_paths(root: tree_sitter::Node<'_>, text: &str) -> Vec<Vec<String>> {
    let mut declarations = Vec::new();
    collect_use_declarations(root, &mut declarations);
    declarations
        .into_iter()
        .filter_map(|node| node.child_by_field_name("argument"))
        .flat_map(|argument| paths_of(argument, text))
        .collect()
}

fn collect_use_declarations<'tree>(
    node: tree_sitter::Node<'tree>,
    found: &mut Vec<tree_sitter::Node<'tree>>,
) {
    if node.kind() == "use_declaration" {
        found.push(node);
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_use_declarations(child, found);
    }
}

fn paths_of(node: tree_sitter::Node<'_>, text: &str) -> Vec<Vec<String>> {
    let segment = |n: tree_sitter::Node<'_>| {
        n.utf8_text(text.as_bytes())
            .expect("node ranges lie within the parsed text")
            .to_owned()
    };
    match node.kind() {
        "identifier" | "crate" | "super" | "self" | "metavariable" => vec![vec![segment(node)]],
        "scoped_identifier" | "scoped_type_identifier" => {
            let name = node.child_by_field_name("name").map(segment);
            let mut bases = node
                .child_by_field_name("path")
                .map_or_else(|| vec![Vec::new()], |path| paths_of(path, text));
            if let Some(name) = name {
                for base in &mut bases {
                    base.push(name.clone());
                }
            }
            bases
        }
        "scoped_use_list" => {
            let bases = node
                .child_by_field_name("path")
                .map_or_else(|| vec![Vec::new()], |path| paths_of(path, text));
            let mut result = Vec::new();
            if let Some(list) = node.child_by_field_name("list") {
                let mut cursor = list.walk();
                for entry in list.named_children(&mut cursor) {
                    for suffix in paths_of(entry, text) {
                        for base in &bases {
                            let mut combined = base.clone();
                            combined.extend(suffix.iter().cloned());
                            result.push(combined);
                        }
                    }
                }
            }
            result
        }
        "use_list" => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .flat_map(|entry| paths_of(entry, text))
                .collect()
        }
        "use_as_clause" => node
            .child_by_field_name("path")
            .map_or_else(Vec::new, |path| paths_of(path, text)),
        "use_wildcard" => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .next()
                .map_or_else(Vec::new, |path| paths_of(path, text))
        }
        _ => Vec::new(),
    }
}
