//! Top-level declarations of one source file, and which of them the file
//! exports.
//!
//! Export is the ecosystem's own rule for what leaves a file. An exported
//! declaration is an item of the architecture; an unexported one is the
//! module's internals, and a name that resolves onto it lands on the module.

use std::collections::BTreeMap;
use std::ops::Range;

use cutaway_architecture::{Element, ElementId, ElementKind, ElementName, SemanticKind};
use cutaway_inspection::ports::source_tree::SourcePath;

use crate::text_of;

/// One top-level declaration and how far it reaches.
#[derive(Debug, Clone)]
pub struct Declaration {
    pub element: Element,
    pub exported: bool,
    /// The byte range the declaration covers, so a reference found inside it
    /// attributes to the declaration rather than to the whole module.
    pub span: Range<usize>,
}

/// What one file declares and hands out.
#[derive(Debug, Default)]
pub struct FileSurface {
    pub declarations: Vec<Declaration>,
    /// The local name the file's `export default` names, when it names one.
    /// An anonymous default names nothing a consumer could land on.
    pub default_export: Option<String>,
}

/// One declaration living inside another: a public method of an exported
/// class. It is an element of its own, contained by the class that declares
/// it.
#[derive(Debug, Clone)]
pub struct NestedDeclaration {
    pub element: Element,
    /// The class element that holds this declaration.
    pub holder: ElementId,
    pub span: Range<usize>,
}

/// The public methods the file's exported classes declare. Each becomes an
/// element of its class, named `Class.name` in its id so that same-named
/// methods of different classes stay distinct; a static and an instance
/// method sharing a name collapse to the first, as duplicate names do
/// elsewhere. A `#name` or `private`-modifier member is the class's
/// internals and declares nothing.
pub fn nested(
    root: tree_sitter::Node<'_>,
    text: &str,
    path: &SourcePath,
    declarations: &[Declaration],
) -> Vec<NestedDeclaration> {
    let mut found = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for class in class_nodes(root) {
        let Some(name_node) = class.child_by_field_name("name") else {
            continue;
        };
        let class_name = text_of(name_node, text);
        let Some(holder) = declarations.iter().find(|declaration| {
            declaration.exported
                && declaration.element.primary_kind() == ElementKind::Type
                && declaration.element.primary_name().as_str() == class_name
        }) else {
            continue;
        };
        let Some(body) = class.child_by_field_name("body") else {
            continue;
        };
        let mut members = body.walk();
        for member in body.named_children(&mut members) {
            if !matches!(
                member.kind(),
                "method_definition" | "method_signature" | "abstract_method_signature"
            ) || is_private_member(member, text)
            {
                continue;
            }
            let Some(name) = member
                .child_by_field_name("name")
                .filter(|name| name.kind() == "property_identifier")
            else {
                continue;
            };
            let name = text_of(name, text);
            let id = declaration_id(
                path,
                SemanticKind::Function,
                &format!("{class_name}.{name}"),
            );
            if !seen.insert(id.clone()) {
                continue;
            }
            found.push(NestedDeclaration {
                element: Element::semantic(
                    id,
                    SemanticKind::Function,
                    ElementName::new(&name).expect("a parsed identifier is never empty"),
                ),
                holder: holder.element.id.clone(),
                span: member.byte_range(),
            });
        }
    }
    found
}

/// The class declarations of the file's top level, unwrapped from the export
/// and `declare` statements around them.
fn class_nodes(root: tree_sitter::Node<'_>) -> Vec<tree_sitter::Node<'_>> {
    let mut classes = Vec::new();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        let candidate = match node.kind() {
            "export_statement" | "ambient_declaration" => {
                node.child_by_field_name("declaration").or_else(|| {
                    let mut children = node.walk();
                    node.named_children(&mut children).find(|child| {
                        matches!(
                            child.kind(),
                            "class_declaration" | "abstract_class_declaration"
                        )
                    })
                })
            }
            _ => Some(node),
        };
        let Some(candidate) = candidate else {
            continue;
        };
        if matches!(
            candidate.kind(),
            "class_declaration" | "abstract_class_declaration"
        ) {
            classes.push(candidate);
        }
    }
    classes
}

/// Whether a class member keeps to itself: a `#name` never leaves the class,
/// and a `private` modifier says the same in TypeScript's words.
fn is_private_member(member: tree_sitter::Node<'_>, text: &str) -> bool {
    if member
        .child_by_field_name("name")
        .is_some_and(|name| name.kind() == "private_property_identifier")
    {
        return true;
    }
    let mut cursor = member.walk();
    member
        .children(&mut cursor)
        .any(|child| child.kind() == "accessibility_modifier" && text_of(child, text) == "private")
}

/// Which element speaks for each part of the file, so a reference attributes
/// to the declaration that writes it rather than to the whole module. Only
/// exported declarations speak: an unexported one is no element, so what it
/// references honestly belongs to the module. Anything outside every speaking
/// span - import statements, top-level code, the bodies of unexported
/// declarations - is the module's own.
///
/// A TypeScript declaration carries everything it owns inside its own node: a
/// class holds its methods, an interface its members, a `const` its arrow
/// function. Nothing attaches behaviour to a declaration from elsewhere in the
/// file, so the declaration spans alone answer, and a method's span nests
/// inside its class's.
#[derive(Debug, Default)]
pub struct Attributions(Vec<(Range<usize>, ElementId)>);

impl Attributions {
    pub fn of(declarations: &[Declaration], nested: &[NestedDeclaration]) -> Self {
        Self(
            declarations
                .iter()
                .filter(|declaration| declaration.exported)
                .map(|declaration| (declaration.span.clone(), declaration.element.id.clone()))
                .chain(
                    nested.iter().map(|declaration| {
                        (declaration.span.clone(), declaration.element.id.clone())
                    }),
                )
                .collect(),
        )
    }

    /// The innermost declaration whose span covers the offset: a public
    /// method's span lies inside its class's, and the nearest enclosing
    /// declaration is the one that writes the reference. What a private
    /// member writes stays the class's own, since only public methods carry
    /// spans of their own.
    pub fn speaker_at(&self, offset: usize) -> Option<&ElementId> {
        self.0
            .iter()
            .filter(|(span, _)| span.contains(&offset))
            .min_by_key(|(span, _)| span.end - span.start)
            .map(|(_, id)| id)
    }
}

/// What the index answers about one declared name.
#[derive(Debug)]
pub struct IndexedDeclaration {
    pub id: ElementId,
    pub exported: bool,
}

/// Looks up a top-level declaration of a file by name, so that an imported
/// name resolves onto the declared item. Unexported declarations are indexed
/// too: resolution must know that the name lands in this file, even though
/// the item itself stays out of the architecture. When one name is declared
/// twice in a file, the first declaration in source order answers.
#[derive(Debug, Default)]
pub struct DeclarationIndex {
    by_name: BTreeMap<(SourcePath, String), IndexedDeclaration>,
    defaults: BTreeMap<SourcePath, String>,
}

impl DeclarationIndex {
    pub fn add(&mut self, path: &SourcePath, surface: &FileSurface) {
        for declaration in &surface.declarations {
            self.by_name
                .entry((
                    path.clone(),
                    declaration.element.primary_name().as_str().to_owned(),
                ))
                .or_insert_with(|| IndexedDeclaration {
                    id: declaration.element.id.clone(),
                    exported: declaration.exported,
                });
        }
        if let Some(name) = &surface.default_export {
            self.defaults.insert(path.clone(), name.clone());
        }
    }

    pub fn declaration(&self, path: &SourcePath, name: &str) -> Option<&IndexedDeclaration> {
        self.by_name.get(&(path.clone(), name.to_owned()))
    }

    /// The local declaration the file exports as its default.
    pub fn default_export(&self, path: &SourcePath) -> Option<&str> {
        self.defaults.get(path).map(String::as_str)
    }
}

/// The functions and types a file declares at its top level, and which of
/// them it exports.
///
/// A file exports in three ways, and all three end here: on the declaration
/// itself, through a list that names declarations made earlier, and through
/// the `CommonJS` exports object a file assigns to.
pub fn top_level(root: tree_sitter::Node<'_>, text: &str, path: &SourcePath) -> FileSurface {
    let mut surface = FileSurface::default();
    // An export list may name a declaration written further up or further
    // down the file, so the exported names are applied once the file is read.
    let mut exported_names = Vec::new();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        match node.kind() {
            "export_statement" => statement(node, text, path, &mut surface, &mut exported_names),
            "expression_statement" => {
                commonjs(
                    node,
                    text,
                    path,
                    &mut surface.declarations,
                    &mut exported_names,
                );
            }
            _ => surface
                .declarations
                .extend(declared(node, text, path, false)),
        }
    }
    for name in exported_names {
        if let Some(declaration) = surface
            .declarations
            .iter_mut()
            .find(|declaration| declaration.element.primary_name().as_str() == name)
        {
            declaration.exported = true;
        }
    }
    surface
}

fn statement(
    node: tree_sitter::Node<'_>,
    text: &str,
    path: &SourcePath,
    surface: &mut FileSurface,
    exported_names: &mut Vec<String>,
) {
    if let Some(declaration) = node.child_by_field_name("declaration") {
        let declared = declared(declaration, text, path, true);
        if exports_default(node)
            && let Some(first) = declared.first()
        {
            surface.default_export = Some(first.element.primary_name().as_str().to_owned());
        }
        surface.declarations.extend(declared);
        return;
    }
    // `export default f` hands out a declaration the file made elsewhere; an
    // anonymous default (`export default () => {}`) declares no item at all.
    if let Some(value) = node.child_by_field_name("value") {
        if value.kind() == "identifier" {
            let name = text_of(value, text);
            surface.default_export = Some(name.clone());
            exported_names.push(name);
        }
        return;
    }
    // `export { a, b as c }` without a source names local declarations; the
    // same statement with a source is a re-export and declares nothing here.
    if node.child_by_field_name("source").is_some() {
        return;
    }
    for specifier in export_specifiers(node) {
        let Some(name) = specifier
            .child_by_field_name("name")
            .map(|n| text_of(n, text))
        else {
            continue;
        };
        if specifier
            .child_by_field_name("alias")
            .is_some_and(|alias| text_of(alias, text) == "default")
        {
            surface.default_export = Some(name.clone());
        }
        exported_names.push(name);
    }
}

/// The declarations one syntactic declaration makes, empty for the forms that
/// declare no function and no type.
fn declared(
    node: tree_sitter::Node<'_>,
    text: &str,
    path: &SourcePath,
    exported: bool,
) -> Vec<Declaration> {
    let kind = match node.kind() {
        "function_declaration"
        | "generator_function_declaration"
        | "function_signature"
        | "generator_function_signature" => SemanticKind::Function,
        "class_declaration"
        | "abstract_class_declaration"
        | "interface_declaration"
        | "type_alias_declaration"
        | "enum_declaration" => SemanticKind::Type,
        // `declare` only states that the declaration inside it exists
        // elsewhere; the item it names is the same item.
        "ambient_declaration" => {
            let mut cursor = node.walk();
            return node
                .named_children(&mut cursor)
                .flat_map(|child| declared(child, text, path, exported))
                .collect();
        }
        "lexical_declaration" | "variable_declaration" => {
            return bound_functions(node, text, path, exported);
        }
        _ => return Vec::new(),
    };
    let Some(name) = node.child_by_field_name("name") else {
        return Vec::new();
    };
    vec![declaration(
        path,
        kind,
        &text_of(name, text),
        exported,
        node.byte_range(),
    )]
}

/// The functions a `const`, `let`, or `var` binds. Modern JavaScript writes
/// most functions this way, so a binding holding one is a function of the
/// module. A binding holding anything else is a value, and the architecture
/// speaks about behaviour and shape rather than values.
fn bound_functions(
    node: tree_sitter::Node<'_>,
    text: &str,
    path: &SourcePath,
    exported: bool,
) -> Vec<Declaration> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| child.kind() == "variable_declarator")
        .filter_map(|declarator| {
            let name = declarator
                .child_by_field_name("name")
                .filter(|name| name.kind() == "identifier")?;
            let value = declarator.child_by_field_name("value")?;
            if !is_function(value) {
                return None;
            }
            Some(declaration(
                path,
                SemanticKind::Function,
                &text_of(name, text),
                exported,
                declarator.byte_range(),
            ))
        })
        .collect()
}

/// The items a `CommonJS` file hands out. `CommonJS` has no export syntax: a
/// module builds its exports object by assignment. `exports.Name = f` and
/// `module.exports.Name = f` declare one item each, while
/// `module.exports = { a, b }` hands out declarations the file made already.
fn commonjs(
    node: tree_sitter::Node<'_>,
    text: &str,
    path: &SourcePath,
    declarations: &mut Vec<Declaration>,
    exported_names: &mut Vec<String>,
) {
    let mut cursor = node.walk();
    let Some(assignment) = node
        .named_children(&mut cursor)
        .find(|child| child.kind() == "assignment_expression")
    else {
        return;
    };
    let (Some(left), Some(right)) = (
        assignment.child_by_field_name("left"),
        assignment.child_by_field_name("right"),
    ) else {
        return;
    };
    if is_module_exports(left, text) {
        if right.kind() == "object" {
            exported_names.extend(handed_out(right, text));
        }
        return;
    }
    let (Some(object), Some(property)) = (
        left.child_by_field_name("object"),
        left.child_by_field_name("property"),
    ) else {
        return;
    };
    if !(is_named(object, text, "exports") || is_module_exports(object, text)) {
        return;
    }
    let kind = if is_function(right) {
        SemanticKind::Function
    } else if matches!(right.kind(), "class" | "class_declaration") {
        SemanticKind::Type
    } else {
        return;
    };
    // The assigned value is the declaration: what it writes is the item's, and
    // the name it is filed under sits outside it.
    declarations.push(declaration(
        path,
        kind,
        &text_of(property, text),
        true,
        right.byte_range(),
    ));
}

/// The local names an exports object hands out: `{ a, b }` names `a` and `b`,
/// `{ c: d }` names `d`.
fn handed_out(object: tree_sitter::Node<'_>, text: &str) -> Vec<String> {
    let mut cursor = object.walk();
    object
        .named_children(&mut cursor)
        .filter_map(|property| match property.kind() {
            "shorthand_property_identifier" => Some(text_of(property, text)),
            "pair" => property
                .child_by_field_name("value")
                .filter(|value| value.kind() == "identifier")
                .map(|value| text_of(value, text)),
            _ => None,
        })
        .collect()
}

fn is_function(node: tree_sitter::Node<'_>) -> bool {
    matches!(
        node.kind(),
        "arrow_function" | "function_expression" | "generator_function"
    )
}

fn is_named(node: tree_sitter::Node<'_>, text: &str, name: &str) -> bool {
    node.kind() == "identifier" && text_of(node, text) == name
}

fn is_module_exports(node: tree_sitter::Node<'_>, text: &str) -> bool {
    node.kind() == "member_expression"
        && node
            .child_by_field_name("object")
            .is_some_and(|object| is_named(object, text, "module"))
        && node
            .child_by_field_name("property")
            .is_some_and(|property| text_of(property, text) == "exports")
}

/// Whether an export statement hands its declaration out as the default.
fn exports_default(node: tree_sitter::Node<'_>) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| child.kind() == "default")
}

fn export_specifiers(node: tree_sitter::Node<'_>) -> Vec<tree_sitter::Node<'_>> {
    let mut cursor = node.walk();
    let Some(clause) = node
        .named_children(&mut cursor)
        .find(|child| child.kind() == "export_clause")
    else {
        return Vec::new();
    };
    let mut specifiers = clause.walk();
    clause
        .named_children(&mut specifiers)
        .filter(|child| child.kind() == "export_specifier")
        .collect()
}

fn declaration(
    path: &SourcePath,
    kind: SemanticKind,
    name: &str,
    exported: bool,
    span: Range<usize>,
) -> Declaration {
    Declaration {
        element: Element::semantic(
            declaration_id(path, kind, name),
            kind,
            ElementName::new(name).expect("a parsed identifier is never empty"),
        ),
        exported,
        span,
    }
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
