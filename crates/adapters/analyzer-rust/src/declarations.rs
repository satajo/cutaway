//! Top-level declarations of one source file.

use cutaway_architecture::{Element, ElementId, ElementKind, ElementName};
use cutaway_inspection::ports::source_tree::SourcePath;

/// The top-level functions, types, and inline modules a file declares.
/// Nested items stay undeclared until the architecture model needs them.
pub fn top_level(root: tree_sitter::Node<'_>, text: &str, path: &SourcePath) -> Vec<Element> {
    let mut cursor = root.walk();
    let mut declarations = Vec::new();
    for node in root.named_children(&mut cursor) {
        let kind = match node.kind() {
            "function_item" => ElementKind::Function,
            "struct_item" | "enum_item" | "trait_item" | "union_item" | "type_item" => {
                ElementKind::Type
            }
            "mod_item" => ElementKind::Module,
            _ => continue,
        };
        let Some(name_node) = node.child_by_field_name("name") else {
            continue;
        };
        let name = name_node
            .utf8_text(text.as_bytes())
            .expect("node ranges lie within the parsed text");
        declarations.push(Element {
            id: declaration_id(path, kind, name),
            name: ElementName::new(name).expect("a parsed identifier is never empty"),
            kind,
        });
    }
    declarations
}

/// Declaration ids embed the path and the kind so that same-named
/// declarations in different files, or in different namespaces of the same
/// file, stay distinct.
fn declaration_id(path: &SourcePath, kind: ElementKind, name: &str) -> ElementId {
    let tag = match kind {
        ElementKind::Project => "project",
        ElementKind::Package => "package",
        ElementKind::Module => "module",
        ElementKind::Function => "function",
        ElementKind::Type => "type",
    };
    ElementId::new(format!("{path}#{tag}:{name}")).expect("the id embeds a non-empty path")
}
