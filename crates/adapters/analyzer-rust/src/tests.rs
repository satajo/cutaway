use cutaway_architecture::{ElementKind, RelationKind};
use cutaway_inspection::ports::source_analyzer::{
    SourceAnalysisError, SourceAnalyzer, SourceStructure,
};
use cutaway_inspection::ports::source_tree::{SourceFile, SourcePath};

use crate::RustSourceAnalyzer;

fn analyze(files: &[(&str, &str)]) -> SourceStructure {
    try_analyze(files).unwrap()
}

fn try_analyze(files: &[(&str, &str)]) -> Result<SourceStructure, SourceAnalysisError> {
    let files: Vec<SourceFile> = files
        .iter()
        .map(|(path, contents)| SourceFile {
            path: SourcePath::new(*path).unwrap(),
            contents: contents.as_bytes().to_vec(),
        })
        .collect();
    RustSourceAnalyzer.analyze(&files)
}

fn parent_of(structure: &SourceStructure, id: &str) -> Option<String> {
    structure
        .elements
        .iter()
        .find(|e| e.element.id.as_str() == id)
        .expect("the element exists")
        .parent
        .as_ref()
        .map(|p| p.as_str().to_owned())
}

fn dependencies(structure: &SourceStructure) -> Vec<(String, String)> {
    structure
        .relations
        .iter()
        .filter(|r| r.kind == RelationKind::DependsOn)
        .map(|r| (r.from.as_str().to_owned(), r.to.as_str().to_owned()))
        .collect()
}

const MANIFEST_A: (&str, &str) = (
    "crates/a/Cargo.toml",
    "[package]\nname = \"a\"\n\n[dependencies]\nb-lib = { path = \"../b\" }\nserde = \"1\"\n",
);
const MANIFEST_B: (&str, &str) = ("crates/b/Cargo.toml", "[package]\nname = \"b-lib\"\n");

#[test]
fn packages_are_discovered_from_their_manifests() {
    let structure = analyze(&[
        ("Cargo.toml", "[workspace]\nmembers = [\"crates/a\"]\n"),
        MANIFEST_A,
        MANIFEST_B,
    ]);
    let packages: Vec<_> = structure
        .elements
        .iter()
        .filter(|e| e.element.kind == ElementKind::Package)
        .map(|e| e.element.id.as_str())
        .collect();
    assert_eq!(packages, ["package:a", "package:b-lib"]);
}

#[test]
fn a_manifest_dependency_no_code_exercises_yields_no_relation() {
    let structure = analyze(&[MANIFEST_A, MANIFEST_B]);
    assert!(dependencies(&structure).is_empty());
}

#[test]
fn a_qualified_path_in_a_function_body_witnesses_the_dependency() {
    let structure = analyze(&[
        MANIFEST_A,
        MANIFEST_B,
        (
            "crates/a/src/lib.rs",
            "pub fn go() {\n    b_lib::util::init();\n}\n",
        ),
        ("crates/b/src/lib.rs", "pub mod util;\n"),
        ("crates/b/src/util.rs", "pub fn init() {}\n"),
    ]);
    assert!(dependencies(&structure).contains(&(
        "crates/a/src/lib.rs#function:go".to_owned(),
        "crates/b/src/util.rs#function:init".to_owned()
    )));
}

#[test]
fn a_qualified_path_in_a_type_position_witnesses_the_dependency() {
    let structure = analyze(&[
        MANIFEST_A,
        MANIFEST_B,
        (
            "crates/a/src/lib.rs",
            "pub fn go(thing: b_lib::util::Thing) {\n    let _ = thing;\n}\n",
        ),
        ("crates/b/src/lib.rs", "pub mod util;\n"),
        ("crates/b/src/util.rs", "pub struct Thing;\n"),
    ]);
    assert!(dependencies(&structure).contains(&(
        "crates/a/src/lib.rs#function:go".to_owned(),
        "crates/b/src/util.rs#type:Thing".to_owned()
    )));
}

#[test]
fn a_qualified_path_to_a_child_module_witnesses_the_use_within_the_package() {
    let structure = analyze(&[
        MANIFEST_A,
        (
            "crates/a/src/lib.rs",
            "mod foo;\npub fn go() {\n    foo::helper();\n}\n",
        ),
        ("crates/a/src/foo.rs", "pub fn helper() {}\n"),
    ]);
    assert!(dependencies(&structure).contains(&(
        "crates/a/src/lib.rs#function:go".to_owned(),
        "crates/a/src/foo.rs#function:helper".to_owned()
    )));
}

#[test]
fn a_qualified_macro_invocation_witnesses_the_package_it_names() {
    let structure = analyze(&[
        MANIFEST_A,
        MANIFEST_B,
        (
            "crates/a/src/lib.rs",
            "pub fn go() {\n    b_lib::mac!();\n}\n",
        ),
        (
            "crates/b/src/lib.rs",
            "#[macro_export]\nmacro_rules! mac {\n    () => {};\n}\n",
        ),
    ]);
    assert!(dependencies(&structure).contains(&(
        "crates/a/src/lib.rs#function:go".to_owned(),
        "package:b-lib".to_owned()
    )));
}

#[test]
fn the_module_tree_follows_the_src_file_layout() {
    let structure = analyze(&[
        MANIFEST_A,
        ("crates/a/src/lib.rs", "mod foo;\n"),
        ("crates/a/src/foo.rs", "pub mod bar;\n"),
        ("crates/a/src/foo/bar.rs", "pub struct Baz;\n"),
    ]);
    assert_eq!(
        parent_of(&structure, "crates/a/src/foo.rs"),
        Some("package:a".to_owned())
    );
    assert_eq!(
        parent_of(&structure, "crates/a/src/foo/bar.rs"),
        Some("crates/a/src/foo.rs".to_owned())
    );
}

#[test]
fn the_crate_root_dissolves_into_its_package() {
    let structure = analyze(&[MANIFEST_B, ("crates/b/src/lib.rs", "pub struct Session;\n")]);
    assert!(
        structure
            .elements
            .iter()
            .all(|e| e.element.id.as_str() != "crates/b/src/lib.rs"),
        "the crate root is no element of its own"
    );
    assert_eq!(
        parent_of(&structure, "crates/b/src/lib.rs#type:Session"),
        Some("package:b-lib".to_owned()),
        "the crate root's declarations are the package's items"
    );
}

#[test]
fn an_import_resolving_to_the_crate_root_lands_on_the_package() {
    let structure = analyze(&[
        ("crates/a/Cargo.toml", "[package]\nname = \"a\"\n"),
        MANIFEST_B,
        ("crates/a/src/lib.rs", "use b_lib::Undeclared;\n"),
        ("crates/b/src/lib.rs", ""),
    ]);
    assert!(
        dependencies(&structure).contains(&("package:a".to_owned(), "package:b-lib".to_owned()))
    );
}

#[test]
fn use_crate_paths_resolve_onto_the_declared_item() {
    let structure = analyze(&[
        MANIFEST_A,
        (
            "crates/a/src/lib.rs",
            "mod foo;\nuse crate::foo::bar::Baz;\n",
        ),
        ("crates/a/src/foo.rs", "pub mod bar;\n"),
        ("crates/a/src/foo/bar.rs", "pub struct Baz;\n"),
    ]);
    assert!(dependencies(&structure).contains(&(
        "package:a".to_owned(),
        "crates/a/src/foo/bar.rs#type:Baz".to_owned()
    )));
}

#[test]
fn an_import_of_an_undeclared_name_stops_at_the_deepest_file_module() {
    let structure = analyze(&[
        MANIFEST_A,
        (
            "crates/a/src/lib.rs",
            "mod foo;\nuse crate::foo::bar::Reexported;\n",
        ),
        ("crates/a/src/foo.rs", "pub mod bar;\n"),
        ("crates/a/src/foo/bar.rs", "pub use other::Reexported;\n"),
    ]);
    assert!(
        dependencies(&structure)
            .contains(&("package:a".to_owned(), "crates/a/src/foo/bar.rs".to_owned()))
    );
}

#[test]
fn an_import_through_a_facade_reexport_resolves_onto_the_declared_item() {
    let structure = analyze(&[
        MANIFEST_A,
        MANIFEST_B,
        ("crates/a/src/lib.rs", "use b_lib::Element;\n"),
        (
            "crates/b/src/lib.rs",
            "mod element;\npub use element::{Element, ElementId};\n",
        ),
        (
            "crates/b/src/element.rs",
            "pub struct Element;\npub struct ElementId;\n",
        ),
    ]);
    assert!(dependencies(&structure).contains(&(
        "package:a".to_owned(),
        "crates/b/src/element.rs#type:Element".to_owned()
    )));
}

#[test]
fn a_renamed_reexport_resolves_onto_the_item_it_forwards_to() {
    let structure = analyze(&[
        MANIFEST_A,
        MANIFEST_B,
        ("crates/a/src/lib.rs", "use b_lib::Widget;\n"),
        (
            "crates/b/src/lib.rs",
            "mod inner;\npub use inner::Thing as Widget;\n",
        ),
        ("crates/b/src/inner.rs", "pub struct Thing;\n"),
    ]);
    assert!(dependencies(&structure).contains(&(
        "package:a".to_owned(),
        "crates/b/src/inner.rs#type:Thing".to_owned()
    )));
}

#[test]
fn a_chain_of_reexports_resolves_onto_the_item_at_its_end() {
    let structure = analyze(&[
        MANIFEST_A,
        MANIFEST_B,
        ("crates/a/src/lib.rs", "use b_lib::Thing;\n"),
        (
            "crates/b/src/lib.rs",
            "mod middle;\npub use middle::Thing;\n",
        ),
        (
            "crates/b/src/middle.rs",
            "mod deep;\npub use deep::Thing;\n",
        ),
        ("crates/b/src/middle/deep.rs", "pub struct Thing;\n"),
    ]);
    assert!(dependencies(&structure).contains(&(
        "package:a".to_owned(),
        "crates/b/src/middle/deep.rs#type:Thing".to_owned()
    )));
}

#[test]
fn reexports_pointing_at_each_other_stop_at_the_module_closing_the_cycle() {
    let structure = analyze(&[
        MANIFEST_A,
        MANIFEST_B,
        ("crates/a/src/lib.rs", "use b_lib::one::Thing;\n"),
        ("crates/b/src/lib.rs", "mod one;\nmod two;\n"),
        ("crates/b/src/one.rs", "pub use crate::two::Thing;\n"),
        ("crates/b/src/two.rs", "pub use crate::one::Thing;\n"),
    ]);
    assert!(
        dependencies(&structure)
            .contains(&("package:a".to_owned(), "crates/b/src/one.rs".to_owned()))
    );
}

#[test]
fn a_wildcard_reexport_forwards_a_name_its_module_does_not_declare() {
    let structure = analyze(&[
        MANIFEST_A,
        MANIFEST_B,
        ("crates/a/src/lib.rs", "use b_lib::Thing;\n"),
        ("crates/b/src/lib.rs", "mod inner;\npub use inner::*;\n"),
        ("crates/b/src/inner.rs", "pub struct Thing;\n"),
    ]);
    assert!(dependencies(&structure).contains(&(
        "package:a".to_owned(),
        "crates/b/src/inner.rs#type:Thing".to_owned()
    )));
}

#[test]
fn a_private_use_keeps_the_name_out_of_the_modules_surface() {
    let structure = analyze(&[
        MANIFEST_A,
        MANIFEST_B,
        ("crates/a/src/lib.rs", "use b_lib::Thing;\n"),
        ("crates/b/src/lib.rs", "mod inner;\nuse inner::Thing;\n"),
        ("crates/b/src/inner.rs", "pub struct Thing;\n"),
    ]);
    let dependencies = dependencies(&structure);
    assert!(dependencies.contains(&("package:a".to_owned(), "package:b-lib".to_owned())));
    assert!(!dependencies.contains(&(
        "package:a".to_owned(),
        "crates/b/src/inner.rs#type:Thing".to_owned()
    )));
}

#[test]
fn a_path_starting_at_an_item_of_the_importing_module_resolves_onto_it() {
    let structure = analyze(&[
        MANIFEST_A,
        ("crates/a/src/lib.rs", "mod foo;\nuse foo::Bar;\n"),
        ("crates/a/src/foo.rs", "pub struct Bar;\n"),
    ]);
    assert!(dependencies(&structure).contains(&(
        "package:a".to_owned(),
        "crates/a/src/foo.rs#type:Bar".to_owned()
    )));
}

#[test]
fn an_import_into_an_inline_module_stops_at_that_module() {
    let structure = analyze(&[
        MANIFEST_A,
        (
            "crates/a/src/lib.rs",
            "mod foo;\nuse crate::foo::inner::Deep;\n",
        ),
        (
            "crates/a/src/foo.rs",
            "pub mod inner { pub struct Deep; }\n",
        ),
    ]);
    assert!(dependencies(&structure).contains(&(
        "package:a".to_owned(),
        "crates/a/src/foo.rs#module:inner".to_owned()
    )));
}

#[test]
fn use_of_another_workspace_package_resolves_into_its_declarations() {
    let structure = analyze(&[
        MANIFEST_A,
        MANIFEST_B,
        ("crates/a/src/lib.rs", "use b_lib::util::Thing;\n"),
        ("crates/b/src/lib.rs", "pub mod util;\n"),
        ("crates/b/src/util.rs", "pub struct Thing;\n"),
    ]);
    assert!(dependencies(&structure).contains(&(
        "package:a".to_owned(),
        "crates/b/src/util.rs#type:Thing".to_owned()
    )));
}

#[test]
fn super_paths_resolve_within_the_module_tree() {
    let structure = analyze(&[
        MANIFEST_A,
        ("crates/a/src/lib.rs", "mod foo;\n"),
        ("crates/a/src/foo.rs", "pub mod bar;\npub mod baz;\n"),
        ("crates/a/src/foo/bar.rs", "use super::baz::X;\n"),
        ("crates/a/src/foo/baz.rs", "pub struct X;\n"),
    ]);
    assert!(dependencies(&structure).contains(&(
        "crates/a/src/foo/bar.rs".to_owned(),
        "crates/a/src/foo/baz.rs#type:X".to_owned()
    )));
}

#[test]
fn integration_tests_are_their_own_crate_roots_using_the_package_by_name() {
    let structure = analyze(&[
        MANIFEST_B,
        ("crates/b/src/lib.rs", "pub mod util;\n"),
        ("crates/b/src/util.rs", "pub struct Thing;\n"),
        ("crates/b/tests/smoke.rs", "use b_lib::util::Thing;\n"),
    ]);
    assert_eq!(
        parent_of(&structure, "crates/b/tests/smoke.rs"),
        Some("package:b-lib".to_owned())
    );
    assert!(dependencies(&structure).contains(&(
        "crates/b/tests/smoke.rs".to_owned(),
        "crates/b/src/util.rs#type:Thing".to_owned()
    )));
}

#[test]
fn a_bare_name_a_use_brought_in_speaks_from_the_declaration_using_it() {
    let structure = analyze(&[
        MANIFEST_A,
        ("crates/a/src/lib.rs", "pub mod caller;\npub mod callee;\n"),
        (
            "crates/a/src/caller.rs",
            "use crate::callee::serve;\n\npub fn drive() {\n    serve();\n}\n",
        ),
        ("crates/a/src/callee.rs", "pub fn serve() {}\n"),
    ]);
    assert!(dependencies(&structure).contains(&(
        "crates/a/src/caller.rs#function:drive".to_owned(),
        "crates/a/src/callee.rs#function:serve".to_owned()
    )));
}

#[test]
fn a_qualified_call_through_a_use_bound_name_resolves_onto_the_item() {
    let structure = analyze(&[
        MANIFEST_A,
        MANIFEST_B,
        (
            "crates/a/src/lib.rs",
            "use b_lib::util;\n\npub fn go() {\n    util::init();\n}\n",
        ),
        ("crates/b/src/lib.rs", "pub mod util;\n"),
        ("crates/b/src/util.rs", "pub fn init() {}\n"),
    ]);
    assert!(dependencies(&structure).contains(&(
        "crates/a/src/lib.rs#function:go".to_owned(),
        "crates/b/src/util.rs#function:init".to_owned()
    )));
}

#[test]
fn the_functions_of_one_module_wire_to_each_other() {
    let structure = analyze(&[
        MANIFEST_B,
        (
            "crates/b/src/lib.rs",
            "pub fn outer() {\n    inner();\n}\npub fn inner() {}\n",
        ),
    ]);
    assert!(dependencies(&structure).contains(&(
        "crates/b/src/lib.rs#function:outer".to_owned(),
        "crates/b/src/lib.rs#function:inner".to_owned()
    )));
}

#[test]
fn a_signature_witnesses_the_types_it_speaks() {
    let structure = analyze(&[
        MANIFEST_B,
        (
            "crates/b/src/lib.rs",
            "pub struct Input;\npub struct Output;\npub fn convert(input: Input) -> Output {\n    let _ = input;\n    Output\n}\n",
        ),
    ]);
    let dependencies = dependencies(&structure);
    for target in ["Input", "Output"] {
        assert!(
            dependencies.contains(&(
                "crates/b/src/lib.rs#function:convert".to_owned(),
                format!("crates/b/src/lib.rs#type:{target}")
            )),
            "convert speaks {target} in its signature; the view has {dependencies:?}"
        );
    }
}

#[test]
fn a_struct_field_witnesses_the_type_it_holds() {
    let structure = analyze(&[
        MANIFEST_B,
        (
            "crates/b/src/lib.rs",
            "pub struct Part;\npub struct Holder {\n    part: Part,\n}\n",
        ),
    ]);
    assert!(dependencies(&structure).contains(&(
        "crates/b/src/lib.rs#type:Holder".to_owned(),
        "crates/b/src/lib.rs#type:Part".to_owned()
    )));
}

#[test]
fn an_enum_variant_witnesses_the_type_it_carries() {
    let structure = analyze(&[
        MANIFEST_B,
        (
            "crates/b/src/lib.rs",
            "pub struct Payload;\npub enum Message {\n    Carrying(Payload),\n}\n",
        ),
    ]);
    assert!(dependencies(&structure).contains(&(
        "crates/b/src/lib.rs#type:Message".to_owned(),
        "crates/b/src/lib.rs#type:Payload".to_owned()
    )));
}

#[test]
fn a_public_methods_reference_speaks_from_the_method() {
    let structure = analyze(&[
        MANIFEST_A,
        (
            "crates/a/src/lib.rs",
            "pub mod store;\npub struct Config;\nimpl Config {\n    pub fn load() {\n        crate::store::fetch();\n    }\n}\n",
        ),
        ("crates/a/src/store.rs", "pub fn fetch() {}\n"),
    ]);
    assert!(dependencies(&structure).contains(&(
        "crates/a/src/lib.rs#function:Config::load".to_owned(),
        "crates/a/src/store.rs#function:fetch".to_owned()
    )));
}

#[test]
fn a_trait_impl_couples_the_type_to_the_trait() {
    let structure = analyze(&[
        MANIFEST_B,
        (
            "crates/b/src/lib.rs",
            "pub trait Draw {}\npub struct Shape;\nimpl Draw for Shape {}\n",
        ),
    ]);
    assert!(dependencies(&structure).contains(&(
        "crates/b/src/lib.rs#type:Shape".to_owned(),
        "crates/b/src/lib.rs#type:Draw".to_owned()
    )));
}

#[test]
fn a_private_declarations_references_stay_with_the_module() {
    let structure = analyze(&[
        MANIFEST_A,
        ("crates/a/src/lib.rs", "pub mod caller;\npub mod callee;\n"),
        (
            "crates/a/src/caller.rs",
            "fn quietly() {\n    crate::callee::serve();\n}\n",
        ),
        ("crates/a/src/callee.rs", "pub fn serve() {}\n"),
    ]);
    assert!(dependencies(&structure).contains(&(
        "crates/a/src/caller.rs".to_owned(),
        "crates/a/src/callee.rs#function:serve".to_owned()
    )));
}

#[test]
fn a_reference_outside_every_declaration_stays_with_the_module() {
    let structure = analyze(&[
        MANIFEST_A,
        ("crates/a/src/lib.rs", "pub mod holder;\npub mod parts;\n"),
        (
            "crates/a/src/holder.rs",
            "use crate::parts::Part;\n\npub static FIXED: Part = Part;\npub fn unrelated() {}\n",
        ),
        ("crates/a/src/parts.rs", "pub struct Part;\n"),
    ]);
    let dependencies = dependencies(&structure);
    assert!(dependencies.contains(&(
        "crates/a/src/holder.rs".to_owned(),
        "crates/a/src/parts.rs#type:Part".to_owned()
    )));
    assert!(
        !dependencies.contains(&(
            "crates/a/src/holder.rs#function:unrelated".to_owned(),
            "crates/a/src/parts.rs#type:Part".to_owned()
        )),
        "a top-level reference belongs to no declaration"
    );
}

#[test]
fn a_declaration_naming_its_own_modules_internals_says_nothing() {
    let structure = analyze(&[
        MANIFEST_B,
        (
            "crates/b/src/lib.rs",
            "pub fn run() {\n    helper();\n}\nfn helper() {}\n",
        ),
    ]);
    assert!(
        dependencies(&structure).is_empty(),
        "a private sibling is the module's internals, not a dependency"
    );
}

#[test]
fn std_and_third_party_imports_stay_outside_the_architecture() {
    let structure = analyze(&[
        MANIFEST_B,
        (
            "crates/b/src/lib.rs",
            "use std::fmt;\nuse serde::Serialize;\n",
        ),
    ]);
    assert!(dependencies(&structure).is_empty());
}

#[test]
fn top_level_declarations_belong_to_their_module() {
    let structure = analyze(&[
        MANIFEST_B,
        ("crates/b/src/lib.rs", "mod api;\n"),
        (
            "crates/b/src/api.rs",
            "pub fn connect() {}\npub struct Session;\npub mod inner {}\n",
        ),
    ]);
    let declared: Vec<_> = structure
        .elements
        .iter()
        .filter(|e| {
            e.parent
                .as_ref()
                .map(cutaway_architecture::ElementId::as_str)
                == Some("crates/b/src/api.rs")
        })
        .map(|e| (e.element.name.as_str().to_owned(), e.element.kind))
        .collect();
    assert_eq!(
        declared,
        [
            ("connect".to_owned(), ElementKind::Function),
            ("Session".to_owned(), ElementKind::Type),
            ("inner".to_owned(), ElementKind::Module),
        ]
    );
}

#[test]
fn declarations_without_a_visibility_modifier_stay_out_of_the_architecture() {
    let structure = analyze(&[
        MANIFEST_B,
        (
            "crates/b/src/lib.rs",
            "pub fn run() {}\nfn helper() {}\nstruct Secret;\nmod tests {}\n",
        ),
    ]);
    let items: Vec<_> = structure
        .elements
        .iter()
        .filter(|e| e.element.kind != ElementKind::Package)
        .map(|e| e.element.name.as_str().to_owned())
        .collect();
    assert_eq!(items, ["run"]);
}

#[test]
fn a_path_naming_a_private_item_lands_on_its_module() {
    let structure = analyze(&[
        MANIFEST_A,
        ("crates/a/src/lib.rs", "mod foo;\nfn secret() {}\n"),
        ("crates/a/src/foo.rs", "use super::secret;\n"),
    ]);
    assert!(
        dependencies(&structure)
            .contains(&("crates/a/src/foo.rs".to_owned(), "package:a".to_owned()))
    );
}

#[test]
fn a_file_module_declaration_adds_no_element_beside_the_file_module() {
    let structure = analyze(&[
        MANIFEST_B,
        ("crates/b/src/lib.rs", "mod util;\n"),
        ("crates/b/src/util.rs", ""),
    ]);
    let utils: Vec<_> = structure
        .elements
        .iter()
        .filter(|e| e.element.kind == ElementKind::Module && e.element.name.as_str() == "util")
        .map(|e| e.element.id.as_str())
        .collect();
    assert_eq!(utils, ["crates/b/src/util.rs"]);
}

#[test]
fn a_module_defined_by_two_files_is_rejected() {
    let result = try_analyze(&[
        MANIFEST_B,
        ("crates/b/src/lib.rs", "mod util;\n"),
        ("crates/b/src/util.rs", ""),
        ("crates/b/src/util/mod.rs", ""),
    ]);
    assert!(matches!(
        result,
        Err(SourceAnalysisError::Unparseable { .. })
    ));
}

#[test]
fn a_file_with_syntax_errors_is_rejected() {
    let result = try_analyze(&[MANIFEST_B, ("crates/b/src/lib.rs", "pub fn broken( {\n")]);
    assert!(matches!(
        result,
        Err(SourceAnalysisError::Unparseable { .. })
    ));
}

#[test]
fn a_public_method_is_an_element_inside_its_type() {
    let structure = analyze(&[
        MANIFEST_A,
        (
            "crates/a/src/config.rs",
            "pub struct Config;\n\nimpl Config {\n    pub fn new() -> Self {\n        Config\n    }\n}\n",
        ),
        ("crates/a/src/lib.rs", "pub mod config;\n"),
    ]);
    assert_eq!(
        parent_of(&structure, "crates/a/src/config.rs#function:Config::new"),
        Some("crates/a/src/config.rs#type:Config".to_owned())
    );
}

#[test]
fn a_path_continuing_past_a_type_lands_on_its_method() {
    let structure = analyze(&[
        MANIFEST_A,
        MANIFEST_B,
        (
            "crates/a/src/lib.rs",
            "use b_lib::config::Config;\n\npub fn go() {\n    let _ = Config::fresh();\n}\n",
        ),
        ("crates/b/src/lib.rs", "pub mod config;\n"),
        (
            "crates/b/src/config.rs",
            "pub struct Config;\n\nimpl Config {\n    pub fn fresh() -> Self {\n        Config\n    }\n}\n",
        ),
    ]);
    assert!(dependencies(&structure).contains(&(
        "crates/a/src/lib.rs#function:go".to_owned(),
        "crates/b/src/config.rs#function:Config::fresh".to_owned()
    )));
}

#[test]
fn a_private_methods_writing_stays_the_types_own() {
    let structure = analyze(&[
        MANIFEST_A,
        (
            "crates/a/src/lib.rs",
            "pub mod store;\n\npub struct Config;\n\nimpl Config {\n    fn refresh() {\n        crate::store::read();\n    }\n}\n",
        ),
        ("crates/a/src/store.rs", "pub fn read() {}\n"),
    ]);
    assert!(
        !structure
            .elements
            .iter()
            .any(|e| e.element.id.as_str() == "crates/a/src/lib.rs#function:Config::refresh"),
        "a private method is the type's internals, not an element"
    );
    assert!(dependencies(&structure).contains(&(
        "crates/a/src/lib.rs#type:Config".to_owned(),
        "crates/a/src/store.rs#function:read".to_owned()
    )));
}

#[test]
fn a_trait_impls_methods_stay_the_types_own() {
    let structure = analyze(&[
        MANIFEST_A,
        (
            "crates/a/src/lib.rs",
            "pub mod store;\n\npub trait Show {\n    fn show(&self);\n}\n\npub struct Config;\n\nimpl Show for Config {\n    fn show(&self) {\n        crate::store::read();\n    }\n}\n",
        ),
        ("crates/a/src/store.rs", "pub fn read() {}\n"),
    ]);
    assert!(
        !structure
            .elements
            .iter()
            .any(|e| e.element.id.as_str() == "crates/a/src/lib.rs#function:Config::show"),
        "two traits may hand a type same-named methods, which one id per name cannot tell apart"
    );
    assert!(dependencies(&structure).contains(&(
        "crates/a/src/lib.rs#type:Config".to_owned(),
        "crates/a/src/store.rs#function:read".to_owned()
    )));
}

#[test]
fn a_method_naming_its_own_type_says_nothing() {
    let structure = analyze(&[
        MANIFEST_A,
        (
            "crates/a/src/config.rs",
            "pub struct Config;\n\nimpl Config {\n    pub fn new() -> Config {\n        Config\n    }\n}\n",
        ),
        ("crates/a/src/lib.rs", "pub mod config;\n"),
    ]);
    assert!(
        !dependencies(&structure).contains(&(
            "crates/a/src/config.rs#function:Config::new".to_owned(),
            "crates/a/src/config.rs#type:Config".to_owned()
        )),
        "the edge onto the holding type restates containment"
    );
}
