//! Top-level declarations of one source file.

use std::collections::BTreeMap;

use cutaway_architecture::{Element, ElementId, ElementKind, ElementName};
use cutaway_inspection::ports::source_tree::SourcePath;

/// One top-level declaration and how far it reaches.
#[derive(Debug, Clone)]
pub struct Declaration {
    pub element: Element,
    /// True when the declaration carries a visibility modifier (`pub`,
    /// `pub(crate)`, ...): the item belongs to the module's surface. A bare
    /// declaration is the module's internals - other modules of the same
    /// crate may still name it, but the architecture shows the module, not
    /// the item.
    pub public: bool,
}

/// What the index answers about one declared name.
#[derive(Debug)]
pub struct IndexedDeclaration {
    pub id: ElementId,
    pub public: bool,
}

/// Looks up a top-level declaration of a file by name, so that the tail of
/// an import path resolves onto the declared item. Private declarations are
/// indexed too: a path may legally name them from within the crate, and
/// resolution must know the name lands here. When one name declares several
/// items in a file, the first declaration in source order answers.
#[derive(Debug, Default)]
pub struct DeclarationIndex(BTreeMap<(SourcePath, String), IndexedDeclaration>);

impl DeclarationIndex {
    pub fn add(&mut self, path: &SourcePath, declarations: &[Declaration]) {
        for declaration in declarations {
            self.0
                .entry((path.clone(), declaration.element.name.as_str().to_owned()))
                .or_insert_with(|| IndexedDeclaration {
                    id: declaration.element.id.clone(),
                    public: declaration.public,
                });
        }
    }

    pub fn declaration(&self, path: &SourcePath, name: &str) -> Option<&IndexedDeclaration> {
        self.0.get(&(path.clone(), name.to_owned()))
    }
}

/// The top-level functions, types, and inline modules a file declares.
/// Nested items stay undeclared until the architecture model needs them.
pub fn top_level(root: tree_sitter::Node<'_>, text: &str, path: &SourcePath) -> Vec<Declaration> {
    let mut cursor = root.walk();
    let mut declarations = Vec::new();
    for node in root.named_children(&mut cursor) {
        let kind = match node.kind() {
            "function_item" => ElementKind::Function,
            "struct_item" | "enum_item" | "trait_item" | "union_item" | "type_item" => {
                ElementKind::Type
            }
            // A bodyless `mod foo;` only points at a file; that file is
            // already a module element of its own, so declaring it again
            // here would duplicate it.
            "mod_item" if node.child_by_field_name("body").is_some() => ElementKind::Module,
            _ => continue,
        };
        let Some(name_node) = node.child_by_field_name("name") else {
            continue;
        };
        let name = name_node
            .utf8_text(text.as_bytes())
            .expect("node ranges lie within the parsed text");
        let mut children = node.walk();
        let public = node
            .named_children(&mut children)
            .any(|child| child.kind() == "visibility_modifier");
        declarations.push(Declaration {
            element: Element {
                id: declaration_id(path, kind, name),
                name: ElementName::new(name).expect("a parsed identifier is never empty"),
                kind,
            },
            public,
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
