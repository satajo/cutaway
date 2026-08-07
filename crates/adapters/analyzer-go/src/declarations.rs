//! Top-level declarations of one source file, and what the directory they
//! sit in offers to the rest of the project.

use std::collections::BTreeMap;
use std::ops::Range;

use cutaway_architecture::{Element, ElementId, ElementKind, ElementName};
use cutaway_inspection::ports::source_tree::SourcePath;

/// One top-level declaration and how far it reaches.
#[derive(Debug, Clone)]
pub struct Declaration {
    pub element: Element,
    /// True when the name starts with an upper-case letter: Go's own rule
    /// for what leaves the package. An unexported declaration is the
    /// directory's internals - code of the same directory may name it, but
    /// the architecture shows the directory, not the item.
    pub exported: bool,
    /// The byte range the declaration covers, so a reference found inside it
    /// attributes to the declaration rather than to the whole module.
    pub span: Range<usize>,
}

/// One declaration living inside another: an exported method of an exported
/// same-file type. It is an element of its own, contained by the type its
/// receiver extends.
#[derive(Debug, Clone)]
pub struct NestedDeclaration {
    pub element: Element,
    /// The type element that holds this declaration.
    pub holder: ElementId,
    pub span: Range<usize>,
}

/// What the index answers about one declared name.
#[derive(Debug)]
pub struct IndexedDeclaration {
    pub id: ElementId,
    pub exported: bool,
}

/// Looks up a top-level declaration by the directory that declares it and by
/// name, so that a qualified reference resolves onto the declared item. The
/// key is the directory rather than the file because the files of one Go
/// directory share a single namespace: an importer names the directory and
/// never learns which file answered.
///
/// Unexported declarations are indexed too: resolution must know that the
/// name lands in this directory, even though the item itself stays out of
/// the architecture. When one name is declared twice in a directory, the
/// first declaration in source order answers.
#[derive(Debug, Default)]
pub struct DeclarationIndex(BTreeMap<(String, String), IndexedDeclaration>);

impl DeclarationIndex {
    pub fn add(&mut self, directory: &str, declarations: &[Declaration]) {
        for declaration in declarations {
            self.0
                .entry((
                    directory.to_owned(),
                    declaration.element.name.as_str().to_owned(),
                ))
                .or_insert_with(|| IndexedDeclaration {
                    id: declaration.element.id.clone(),
                    exported: declaration.exported,
                });
        }
    }

    pub fn declaration(&self, directory: &str, name: &str) -> Option<&IndexedDeclaration> {
        self.0.get(&(directory.to_owned(), name.to_owned()))
    }
}

/// The top-level functions and types a file declares.
///
/// Methods carry a receiver and belong to the type they extend, not to the
/// directory's namespace; [`nested`] declares them inside their type.
/// Constants and variables stay out for the same reason the Rust adapter
/// keeps them out: the architecture speaks about behaviour and shape, not
/// about values.
pub fn top_level(root: tree_sitter::Node<'_>, text: &str, path: &SourcePath) -> Vec<Declaration> {
    let mut cursor = root.walk();
    let mut declarations = Vec::new();
    for node in root.named_children(&mut cursor) {
        match node.kind() {
            "function_declaration" => {
                if let Some(declared) = declared(node, text, path, ElementKind::Function) {
                    declarations.push(declared);
                }
            }
            // One `type ( ... )` block declares several types, each in its
            // own spec; a lone `type X = Y` is an alias spec.
            "type_declaration" => {
                let mut specs = node.walk();
                for spec in node.named_children(&mut specs) {
                    if !matches!(spec.kind(), "type_spec" | "type_alias") {
                        continue;
                    }
                    if let Some(declared) = declared(spec, text, path, ElementKind::Type) {
                        declarations.push(declared);
                    }
                }
            }
            _ => {}
        }
    }
    declarations
}

fn declared(
    node: tree_sitter::Node<'_>,
    text: &str,
    path: &SourcePath,
    kind: ElementKind,
) -> Option<Declaration> {
    let name = node
        .child_by_field_name("name")?
        .utf8_text(text.as_bytes())
        .expect("node ranges lie within the parsed text");
    Some(Declaration {
        element: Element {
            id: declaration_id(path, kind, name),
            name: ElementName::new(name).expect("a parsed identifier is never empty"),
            kind,
            fingerprint: None,
        },
        exported: name.chars().next().is_some_and(char::is_uppercase),
        span: node.byte_range(),
    })
}

/// The exported methods the file declares for its exported types. Each
/// becomes an element of the type its receiver extends, named `Type.name`
/// in its id so that same-named methods of different types stay distinct.
/// Go forbids a type two same-named methods, so the id is collision-free.
pub fn nested(
    root: tree_sitter::Node<'_>,
    text: &str,
    path: &SourcePath,
    declarations: &[Declaration],
) -> Vec<NestedDeclaration> {
    let mut found = Vec::new();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        let Some((method_name, extended)) = extended_type(node, text, declarations) else {
            continue;
        };
        if !method_name.chars().next().is_some_and(char::is_uppercase) {
            continue;
        }
        let holder_name = extended.element.name.as_str();
        found.push(NestedDeclaration {
            element: Element {
                id: declaration_id(
                    path,
                    ElementKind::Function,
                    &format!("{holder_name}.{method_name}"),
                ),
                name: ElementName::new(&method_name).expect("a parsed identifier is never empty"),
                kind: ElementKind::Function,
                fingerprint: None,
            },
            holder: extended.element.id.clone(),
            span: node.byte_range(),
        });
    }
    found
}

/// Which element speaks for each part of the file, so a reference attributes
/// to the declaration that writes it rather than to the whole module. Only
/// exported declarations speak: an unexported one is no element, so what it
/// references honestly belongs to the module. Anything outside every speaking
/// span - the package clause, the imports, top-level code, the bodies of
/// unexported declarations - is the module's own.
#[derive(Debug, Default)]
pub struct Attributions(Vec<(Range<usize>, ElementId)>);

impl Attributions {
    /// The innermost declaration whose span covers the offset, should spans
    /// ever nest; Go's top-level declarations stand apart, so the innermost
    /// is normally the only one.
    pub fn speaker_at(&self, offset: usize) -> Option<&ElementId> {
        self.0
            .iter()
            .filter(|(span, _)| span.contains(&offset))
            .min_by_key(|(span, _)| span.end - span.start)
            .map(|(_, id)| id)
    }
}

/// Reads the file's speaking spans: every exported top-level declaration,
/// every method element the file declares, and every remaining method whose
/// receiver is an exported same-file type. Go attaches behaviour to a type
/// in declarations that stand apart from the type's own, so what an
/// unexported method writes is still the receiver type's coupling; an
/// exported method is an element and speaks for itself. A method whose
/// receiver is unexported, or is declared in another file of the directory,
/// speaks for no declaration of this file, and its references stay the
/// module's.
pub fn attributions(
    root: tree_sitter::Node<'_>,
    text: &str,
    declarations: &[Declaration],
    nested: &[NestedDeclaration],
) -> Attributions {
    let mut spans: Vec<(Range<usize>, ElementId)> = declarations
        .iter()
        .filter(|declaration| declaration.exported)
        .map(|declaration| (declaration.span.clone(), declaration.element.id.clone()))
        .collect();
    for declaration in nested {
        spans.push((declaration.span.clone(), declaration.element.id.clone()));
    }
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        if nested
            .iter()
            .any(|declaration| declaration.span == node.byte_range())
        {
            continue;
        }
        let Some((_, extended)) = extended_type(node, text, declarations) else {
            continue;
        };
        spans.push((node.byte_range(), extended.element.id.clone()));
    }
    Attributions(spans)
}

/// The method name and the exported same-file type a method declaration
/// extends, when its receiver names one.
fn extended_type<'a>(
    node: tree_sitter::Node<'_>,
    text: &str,
    declarations: &'a [Declaration],
) -> Option<(String, &'a Declaration)> {
    if node.kind() != "method_declaration" {
        return None;
    }
    let receiver = receiver_type(node)?;
    let receiver_name = receiver
        .utf8_text(text.as_bytes())
        .expect("node ranges lie within the parsed text");
    let extended = declarations.iter().find(|declaration| {
        declaration.exported && declaration.element.name.as_str() == receiver_name
    })?;
    let method_name = node
        .child_by_field_name("name")?
        .utf8_text(text.as_bytes())
        .expect("node ranges lie within the parsed text")
        .to_owned();
    Some((method_name, extended))
}

/// The type a method extends, as the identifier that names it. `func (c
/// *Config)` and `func (s *Set[T])` both extend the plain type: the pointer
/// and the type parameters say how the method is called, not whose behaviour
/// it holds.
fn receiver_type(method: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    let receiver = method.child_by_field_name("receiver")?;
    let mut cursor = receiver.walk();
    let parameter = receiver
        .named_children(&mut cursor)
        .find(|node| node.kind() == "parameter_declaration")?;
    let mut base = parameter.child_by_field_name("type")?;
    if base.kind() == "pointer_type" {
        base = base.named_child(0)?;
    }
    if base.kind() == "generic_type" {
        base = base.child_by_field_name("type")?;
    }
    (base.kind() == "type_identifier").then_some(base)
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
        ElementKind::File => "file",
        ElementKind::Function => "function",
        ElementKind::Type => "type",
    };
    ElementId::new(format!("{path}#{tag}:{name}")).expect("the id embeds a non-empty path")
}
