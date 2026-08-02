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
fn manifest_dependencies_link_workspace_packages_and_ignore_external_ones() {
    let structure = analyze(&[MANIFEST_A, MANIFEST_B]);
    assert_eq!(
        dependencies(&structure),
        [("package:a".to_owned(), "package:b-lib".to_owned())]
    );
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
        parent_of(&structure, "crates/a/src/lib.rs"),
        Some("package:a".to_owned())
    );
    assert_eq!(
        parent_of(&structure, "crates/a/src/foo.rs"),
        Some("crates/a/src/lib.rs".to_owned())
    );
    assert_eq!(
        parent_of(&structure, "crates/a/src/foo/bar.rs"),
        Some("crates/a/src/foo.rs".to_owned())
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
        "crates/a/src/lib.rs".to_owned(),
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
    assert!(dependencies(&structure).contains(&(
        "crates/a/src/lib.rs".to_owned(),
        "crates/a/src/foo/bar.rs".to_owned()
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
        "crates/a/src/lib.rs".to_owned(),
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
        "crates/a/src/lib.rs".to_owned(),
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
        (
            "crates/b/src/lib.rs",
            "pub fn connect() {}\npub struct Session;\nmod internal {}\n",
        ),
    ]);
    let declared: Vec<_> = structure
        .elements
        .iter()
        .filter(|e| {
            e.parent
                .as_ref()
                .map(cutaway_architecture::ElementId::as_str)
                == Some("crates/b/src/lib.rs")
        })
        .map(|e| (e.element.name.as_str().to_owned(), e.element.kind))
        .collect();
    assert_eq!(
        declared,
        [
            ("connect".to_owned(), ElementKind::Function),
            ("Session".to_owned(), ElementKind::Type),
            ("internal".to_owned(), ElementKind::Module),
        ]
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
