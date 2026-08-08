//! Top-level declarations of one source file.

use std::collections::BTreeMap;
use std::ops::Range;

use cutaway_architecture::{Element, ElementId, ElementName, SemanticKind};
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

/// One declaration living inside another: a public method or associated
/// function of a public type. It is an element of its own, contained by the
/// type whose `impl` block declares it.
#[derive(Debug, Clone)]
pub struct NestedDeclaration {
    pub element: Element,
    /// The type element that holds this declaration.
    pub holder: ElementId,
    /// The holder's declared name, so a path that walked onto the holder
    /// can take one more segment.
    pub holder_name: String,
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
pub struct DeclarationIndex {
    by_name: BTreeMap<(SourcePath, String), IndexedDeclaration>,
    /// (file, holder name, method name) -> the method element, so a path
    /// continuing one segment past a type lands on the method. Only public
    /// methods enter: a path naming a private one lands on the type, the
    /// way a path naming a private item lands on the module.
    nested: BTreeMap<(SourcePath, String, String), ElementId>,
}

impl DeclarationIndex {
    pub fn add(&mut self, path: &SourcePath, declarations: &[Declaration]) {
        for declaration in declarations {
            self.by_name
                .entry((
                    path.clone(),
                    declaration.element.primary_name().as_str().to_owned(),
                ))
                .or_insert_with(|| IndexedDeclaration {
                    id: declaration.element.id.clone(),
                    public: declaration.public,
                });
        }
    }

    pub fn add_nested(&mut self, path: &SourcePath, nested: &[NestedDeclaration]) {
        for declaration in nested {
            self.nested
                .entry((
                    path.clone(),
                    declaration.holder_name.clone(),
                    declaration.element.primary_name().as_str().to_owned(),
                ))
                .or_insert_with(|| declaration.element.id.clone());
        }
    }

    pub fn declaration(&self, path: &SourcePath, name: &str) -> Option<&IndexedDeclaration> {
        self.by_name.get(&(path.clone(), name.to_owned()))
    }

    pub fn nested_declaration(
        &self,
        path: &SourcePath,
        holder: &str,
        name: &str,
    ) -> Option<&ElementId> {
        self.nested
            .get(&(path.clone(), holder.to_owned(), name.to_owned()))
    }
}

/// The top-level functions, types, and inline modules a file declares.
/// What lives inside a declaration is [`nested`]'s business.
pub fn top_level(root: tree_sitter::Node<'_>, text: &str, path: &SourcePath) -> Vec<Declaration> {
    let mut cursor = root.walk();
    let mut declarations = Vec::new();
    for node in root.named_children(&mut cursor) {
        let kind = match node.kind() {
            "function_item" => SemanticKind::Function,
            "struct_item" | "enum_item" | "trait_item" | "union_item" | "type_item" => {
                SemanticKind::Type
            }
            // A bodyless `mod foo;` only points at a file; that file is
            // already a module element of its own, so declaring it again
            // here would duplicate it.
            "mod_item" if node.child_by_field_name("body").is_some() => SemanticKind::Module,
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
            element: Element::semantic(
                declaration_id(path, kind, name),
                kind,
                ElementName::new(name).expect("a parsed identifier is never empty"),
            ),
            public,
            span: node.byte_range(),
        });
    }
    declarations
}

/// The public methods and associated functions the file's inherent `impl`
/// blocks declare for its public types. Each becomes an element of the type
/// that holds it, named `Type::method` in its id so that same-named methods
/// of different types stay distinct.
///
/// A trait impl (`impl Show for Config`) contributes no method elements:
/// two traits may hand the same type same-named methods, which one id per
/// name cannot tell apart, so trait-given behaviour stays the type's own -
/// its spans keep speaking as the type through [`attributions`].
pub fn nested(
    root: tree_sitter::Node<'_>,
    text: &str,
    path: &SourcePath,
    declarations: &[Declaration],
) -> Vec<NestedDeclaration> {
    let mut found = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        if node.kind() != "impl_item" || node.child_by_field_name("trait").is_some() {
            continue;
        }
        let Some(implemented) = implemented_type(node, text, declarations) else {
            continue;
        };
        let Some(body) = node.child_by_field_name("body") else {
            continue;
        };
        let mut items = body.walk();
        for item in body.named_children(&mut items) {
            if item.kind() != "function_item" {
                continue;
            }
            let mut children = item.walk();
            let public = item
                .named_children(&mut children)
                .any(|child| child.kind() == "visibility_modifier");
            if !public {
                continue;
            }
            let Some(name_node) = item.child_by_field_name("name") else {
                continue;
            };
            let name = name_node
                .utf8_text(text.as_bytes())
                .expect("node ranges lie within the parsed text");
            let holder_name = implemented.element.primary_name().as_str().to_owned();
            let id = declaration_id(
                path,
                SemanticKind::Function,
                &format!("{holder_name}::{name}"),
            );
            if !seen.insert(id.clone()) {
                continue;
            }
            found.push(NestedDeclaration {
                element: Element::semantic(
                    id,
                    SemanticKind::Function,
                    ElementName::new(name).expect("a parsed identifier is never empty"),
                ),
                holder: implemented.element.id.clone(),
                holder_name,
                span: item.byte_range(),
            });
        }
    }
    found
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
    /// The innermost declaration whose span covers the offset: spans nest -
    /// a method inside its impl block - and the nearest enclosing
    /// declaration is the one that writes the reference.
    pub fn speaker_at(&self, offset: usize) -> Option<&ElementId> {
        self.0
            .iter()
            .filter(|(span, _)| span.contains(&offset))
            .min_by_key(|(span, _)| span.end - span.start)
            .map(|(_, id)| id)
    }
}

/// Reads the file's speaking spans: every public top-level declaration,
/// every `impl` block whose self type is one of them, and every method
/// element the file declares. Rust attaches behaviour to types in blocks
/// apart from the type's own declaration, so the references an `impl
/// Config` makes are Config's coupling - including a `impl Show for
/// Config`, which couples Config to Show. Within such a block a public
/// method of an inherent impl speaks for itself; the rest - associated
/// consts, private methods, trait-given methods - keeps speaking as the
/// type. An impl of a private, foreign, or qualified type speaks for no
/// declaration, and its references stay the module's.
pub fn attributions(
    root: tree_sitter::Node<'_>,
    text: &str,
    declarations: &[Declaration],
    nested: &[NestedDeclaration],
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
        let Some(implemented) = implemented_type(node, text, declarations) else {
            continue;
        };
        spans.push((node.byte_range(), implemented.element.id.clone()));
    }
    for declaration in nested {
        spans.push((declaration.span.clone(), declaration.element.id.clone()));
    }
    Attributions(spans)
}

/// The public same-file declaration an `impl` block implements, when its
/// self type names one.
fn implemented_type<'a>(
    node: tree_sitter::Node<'_>,
    text: &str,
    declarations: &'a [Declaration],
) -> Option<&'a Declaration> {
    let self_type = node.child_by_field_name("type")?;
    // `impl<T> Wrapper<T>` implements Wrapper; the generic arguments
    // narrow it, they do not change whose behaviour the block holds.
    let self_type = if self_type.kind() == "generic_type" {
        self_type.child_by_field_name("type").unwrap_or(self_type)
    } else {
        self_type
    };
    if self_type.kind() != "type_identifier" {
        return None;
    }
    let name = self_type
        .utf8_text(text.as_bytes())
        .expect("node ranges lie within the parsed text");
    declarations.iter().find(|declaration| {
        declaration.public && declaration.element.primary_name().as_str() == name
    })
}

/// Declaration ids embed the path and the kind so that same-named
/// declarations in different files, or in different namespaces of the same
/// file, stay distinct.
fn declaration_id(path: &SourcePath, kind: SemanticKind, name: &str) -> ElementId {
    let tag = match kind {
        SemanticKind::Project => "project",
        SemanticKind::Package => "package",
        SemanticKind::Module => "module",
        SemanticKind::Function => "function",
        SemanticKind::Type => "type",
    };
    ElementId::new(format!("{path}#{tag}:{name}")).expect("the id embeds a non-empty path")
}
