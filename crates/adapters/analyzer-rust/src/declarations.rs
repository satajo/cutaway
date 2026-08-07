//! Top-level declarations of one source file.

use std::collections::BTreeMap;
use std::ops::Range;

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
    /// The byte range the declaration covers, so a reference found inside
    /// it attributes to the declaration rather than to the whole module.
    pub span: Range<usize>,
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
            span: node.byte_range(),
        });
    }
    declarations
}

/// Which element speaks for each part of the file, so a reference
/// attributes to the declaration that writes it rather than to the whole
/// module. Only public declarations speak: a private declaration is no
/// element, so what it references honestly belongs to the module. Anything
/// outside every speaking span - `use` declarations, top-level code, the
/// bodies of private declarations - is the module's own.
#[derive(Debug, Default)]
pub struct Attributions(Vec<(Range<usize>, ElementId)>);

impl Attributions {
    pub fn speaker_at(&self, offset: usize) -> Option<&ElementId> {
        self.0
            .iter()
            .find(|(span, _)| span.contains(&offset))
            .map(|(_, id)| id)
    }
}

/// Reads the file's speaking spans: every public top-level declaration, and
/// every `impl` block whose self type is one of them. Rust attaches
/// behaviour to types in blocks apart from the type's own declaration, so
/// the references an `impl Config` makes are Config's coupling - including
/// a `impl Show for Config`, which couples Config to Show. An impl of a
/// private, foreign, or qualified type speaks for no declaration, and its
/// references stay the module's.
pub fn attributions(
    root: tree_sitter::Node<'_>,
    text: &str,
    declarations: &[Declaration],
) -> Attributions {
    let mut spans: Vec<(Range<usize>, ElementId)> = declarations
        .iter()
        .filter(|declaration| declaration.public)
        .map(|declaration| (declaration.span.clone(), declaration.element.id.clone()))
        .collect();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        if node.kind() != "impl_item" {
            continue;
        }
        let Some(self_type) = node.child_by_field_name("type") else {
            continue;
        };
        // `impl<T> Wrapper<T>` implements Wrapper; the generic arguments
        // narrow it, they do not change whose behaviour the block holds.
        let self_type = if self_type.kind() == "generic_type" {
            self_type.child_by_field_name("type").unwrap_or(self_type)
        } else {
            self_type
        };
        if self_type.kind() != "type_identifier" {
            continue;
        }
        let name = self_type
            .utf8_text(text.as_bytes())
            .expect("node ranges lie within the parsed text");
        let Some(implemented) = declarations
            .iter()
            .find(|declaration| declaration.public && declaration.element.name.as_str() == name)
        else {
            continue;
        };
        spans.push((node.byte_range(), implemented.element.id.clone()));
    }
    Attributions(spans)
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
