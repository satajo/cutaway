use std::collections::BTreeMap;

use cutaway_architecture::{ArchitectureGraph, ElementId, ElementKind, RelationKind};
use cutaway_inspection::inspect;
use cutaway_inspection::ports::source_analyzer::{
    AnalysisGap, Extent, GapReason, SourceAnalyzer, SourceStructure,
};
use cutaway_inspection::ports::source_tree::{
    DirectoryPath, ProjectName, SourceFile, SourcePath, SourceTree, SourceTreeError,
};

use crate::RustSourceAnalyzer;

fn sources(files: &[(&str, &str)]) -> Vec<SourceFile> {
    files
        .iter()
        .map(|(path, contents)| SourceFile {
            path: SourcePath::new(*path).unwrap(),
            contents: contents.as_bytes().to_vec(),
        })
        .collect()
}

fn analyze(files: &[(&str, &str)]) -> SourceStructure {
    RustSourceAnalyzer.analyze(&sources(files))
}

/// Everywhere the analyzer could not read what the sources hold.
fn gaps(files: &[(&str, &str)]) -> Vec<AnalysisGap> {
    analyze(files).gaps
}

struct Fixture(Vec<SourceFile>);

impl SourceTree for Fixture {
    fn name(&self) -> ProjectName {
        ProjectName::new("fixture").unwrap()
    }

    fn files(&self) -> Result<Vec<SourceFile>, SourceTreeError> {
        Ok(self.0.clone())
    }
}

/// The architecture the wired application builds out of these sources.
fn inspected(files: &[(&str, &str)]) -> ArchitectureGraph {
    inspect(&Fixture(sources(files)), &[&RustSourceAnalyzer])
        .expect("the sources inspect")
        .graph
}

/// The boundary a picture speaking no directories and no files draws around
/// an element: the nearest holder a language read anything into. This is the
/// shape the model drew before the file tree became its skeleton, so it is
/// where a change of the substrate would show as a regression.
fn semantic_holder(graph: &ArchitectureGraph, id: &str) -> Option<String> {
    let parents: BTreeMap<&ElementId, &ElementId> = graph
        .relations()
        .filter(|relation| relation.kind == RelationKind::Contains)
        .map(|relation| (&relation.to, &relation.from))
        .collect();
    let mut current = parents.get(&ElementId::new(id).unwrap()).copied();
    while let Some(id) = current {
        if graph
            .element(id)
            .is_some_and(|element| element.semantic_aspect().is_some())
        {
            return Some(id.as_str().to_owned());
        }
        current = parents.get(id).copied();
    }
    None
}

/// What one element reads out of the sources.
fn extent_of(structure: &SourceStructure, id: &str) -> Extent {
    structure
        .interpretations
        .iter()
        .find(|interpretation| interpretation.element.id.as_str() == id)
        .unwrap_or_else(|| panic!("no interpretation {id}"))
        .extent
        .clone()
}

fn reads_nothing(structure: &SourceStructure, id: &str) -> bool {
    !structure
        .interpretations
        .iter()
        .any(|interpretation| interpretation.element.id.as_str() == id)
}

fn file(path: &str) -> SourcePath {
    SourcePath::new(path).unwrap()
}

fn directory(path: &str) -> DirectoryPath {
    DirectoryPath::new(path).unwrap()
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
        .interpretations
        .iter()
        .filter(|i| i.element.primary_kind() == ElementKind::Package)
        .map(|i| i.element.id.as_str())
        .collect();
    assert_eq!(packages, ["package:a", "package:b-lib"]);
}

#[test]
fn a_package_reads_the_directory_its_manifest_sits_in() {
    let structure = analyze(&[MANIFEST_A, ("Cargo.toml", "[package]\nname = \"root\"\n")]);

    assert_eq!(
        extent_of(&structure, "package:a"),
        Extent::Directory(directory("crates/a"))
    );
    assert_eq!(
        extent_of(&structure, "package:root"),
        Extent::Root,
        "a manifest at the top of the tree makes the whole repository the package"
    );
}

#[test]
fn a_manifest_is_a_file_of_the_package_it_names() {
    let structure = analyze(&[MANIFEST_A]);
    assert!(
        reads_nothing(&structure, "crates/a/Cargo.toml"),
        "the manifest names the package, it is not the package"
    );

    let graph = inspected(&[MANIFEST_A, ("crates/a/src/lib.rs", "")]);
    let manifest = graph
        .element(&ElementId::new("crates/a/Cargo.toml").unwrap())
        .expect("the manifest stands in the picture");
    assert_eq!(manifest.primary_kind(), ElementKind::File);
    assert!(
        manifest.fingerprint.is_some(),
        "an edit to the manifest must read as a change"
    );
    assert_eq!(
        semantic_holder(&graph, "crates/a/Cargo.toml"),
        Some("package:a".to_owned())
    );
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
    let files = [
        MANIFEST_A,
        ("crates/a/src/lib.rs", "mod foo;\n"),
        ("crates/a/src/foo.rs", "pub mod bar;\n"),
        ("crates/a/src/foo/bar.rs", "pub struct Baz;\n"),
    ];
    let structure = analyze(&files);
    assert_eq!(
        extent_of(&structure, "crates/a/src/foo.rs"),
        Extent::FileAndDirectory {
            file: file("crates/a/src/foo.rs"),
            directory: directory("crates/a/src/foo")
        },
        "a module whose name also names a directory reads both as one boundary"
    );
    assert_eq!(
        extent_of(&structure, "crates/a/src/foo/bar.rs"),
        Extent::File(file("crates/a/src/foo/bar.rs"))
    );

    let graph = inspected(&files);
    assert_eq!(
        semantic_holder(&graph, "crates/a/src/foo.rs"),
        Some("package:a".to_owned())
    );
    assert_eq!(
        semantic_holder(&graph, "crates/a/src/foo/bar.rs"),
        Some("crates/a/src/foo.rs".to_owned()),
        "the spanning extent makes the children of foo/ the children of module foo"
    );
}

#[test]
fn the_crate_root_reads_nothing_of_its_own() {
    let files = [MANIFEST_B, ("crates/b/src/lib.rs", "pub struct Session;\n")];
    let structure = analyze(&files);
    assert!(
        reads_nothing(&structure, "crates/b/src/lib.rs"),
        "the package and its root namespace are one boundary"
    );

    let graph = inspected(&files);
    assert_eq!(
        semantic_holder(&graph, "crates/b/src/lib.rs#type:Session"),
        Some("package:b-lib".to_owned()),
        "the crate root's declarations read as the package's items"
    );
    assert_eq!(
        graph
            .element(&ElementId::new("crates/b/src/lib.rs").unwrap())
            .map(cutaway_architecture::Element::primary_kind),
        Some(ElementKind::File),
        "the file itself still stands where the tree puts it"
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
    let files = [
        MANIFEST_B,
        ("crates/b/src/lib.rs", "pub mod util;\n"),
        ("crates/b/src/util.rs", "pub struct Thing;\n"),
        ("crates/b/tests/smoke.rs", "use b_lib::util::Thing;\n"),
    ];
    let structure = analyze(&files);
    assert!(
        reads_nothing(&structure, "crates/b/tests/smoke.rs"),
        "a standalone crate root is a crate root, so it reads no module"
    );
    assert!(dependencies(&structure).contains(&(
        "crates/b/tests/smoke.rs".to_owned(),
        "crates/b/src/util.rs#type:Thing".to_owned()
    )));

    let graph = inspected(&files);
    assert_eq!(
        semantic_holder(&graph, "crates/b/tests/smoke.rs"),
        Some("package:b-lib".to_owned()),
        "the test file lies in the package's directory, so it stands inside it"
    );
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
        .interpretations
        .iter()
        .filter(|i| {
            i.extent
                == Extent::Within {
                    file: file("crates/b/src/api.rs"),
                    parent: None,
                }
        })
        .map(|i| {
            (
                i.element.primary_name().as_str().to_owned(),
                i.element.primary_kind(),
            )
        })
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
        .interpretations
        .iter()
        .filter(|i| i.element.primary_kind() != ElementKind::Package)
        .map(|i| i.element.primary_name().as_str().to_owned())
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
        .interpretations
        .iter()
        .filter(|i| {
            i.element.primary_kind() == ElementKind::Module
                && i.element.primary_name().as_str() == "util"
        })
        .map(|i| i.element.id.as_str())
        .collect();
    assert_eq!(utils, ["crates/b/src/util.rs"]);
}

#[test]
fn a_module_two_files_define_costs_the_module_name_and_nothing_else() {
    let files = [
        MANIFEST_A,
        MANIFEST_B,
        ("crates/a/src/lib.rs", "pub struct Thing;\n"),
        ("crates/b/src/lib.rs", "pub mod util;\npub mod other;\n"),
        ("crates/b/src/util.rs", "use a::Thing;\n\npub struct One;\n"),
        ("crates/b/src/util/mod.rs", "pub struct Two;\n"),
        ("crates/b/src/other.rs", "use crate::util::One;\n"),
    ];
    let structure = analyze(&files);

    assert!(
        reads_nothing(&structure, "crates/b/src/util.rs")
            && reads_nothing(&structure, "crates/b/src/util/mod.rs"),
        "neither file may claim the module both of them define"
    );
    assert_eq!(
        extent_of(&structure, "crates/b/src/util.rs#type:One"),
        Extent::Within {
            file: file("crates/b/src/util.rs"),
            parent: None,
        },
        "the conflict is about the module's name, not about what the files write"
    );
    assert_eq!(
        extent_of(&structure, "crates/b/src/util/mod.rs#type:Two"),
        Extent::Within {
            file: file("crates/b/src/util/mod.rs"),
            parent: None,
        }
    );
    assert!(
        dependencies(&structure).contains(&(
            "crates/b/src/util.rs".to_owned(),
            "crates/a/src/lib.rs#type:Thing".to_owned()
        )),
        "a contested file still speaks from the node the tree gives it: {:?}",
        dependencies(&structure)
    );
    assert!(
        !dependencies(&structure)
            .iter()
            .any(|(_, to)| to.contains("util")),
        "a path through the contested name reaches neither claimant: {:?}",
        dependencies(&structure)
    );
    assert!(
        dependencies(&structure).contains(&(
            "crates/b/src/other.rs".to_owned(),
            "package:b-lib".to_owned()
        )),
        "a path into a name nobody claims stops at the deepest boundary that \
         does exist, as any unresolvable path does: {:?}",
        dependencies(&structure)
    );

    let conflicts = structure.gaps;
    assert_eq!(
        conflicts.len(),
        2,
        "both files are told what they are missing"
    );
    assert!(
        conflicts
            .iter()
            .all(|gap| matches!(gap.reason, GapReason::ConflictingDefinitions { .. })),
        "{conflicts:?}"
    );

    let graph = inspected(&files);
    assert_eq!(
        semantic_holder(&graph, "crates/b/src/util.rs"),
        Some("package:b-lib".to_owned()),
        "a file whose module nobody claims still stands where the tree puts it"
    );
}

#[test]
fn a_file_with_a_syntax_error_still_yields_the_declarations_that_parsed() {
    let structure = analyze(&[
        MANIFEST_B,
        ("crates/b/src/lib.rs", "pub mod good;\n"),
        (
            "crates/b/src/good.rs",
            "pub struct Kept;\n\npub fn broken( {\n",
        ),
    ]);
    assert_eq!(
        extent_of(&structure, "crates/b/src/good.rs#type:Kept"),
        Extent::Within {
            file: file("crates/b/src/good.rs"),
            parent: None,
        }
    );
}

#[test]
fn a_file_with_a_syntax_error_is_a_gap_at_the_failing_line() {
    let declared = gaps(&[
        MANIFEST_B,
        ("crates/b/src/lib.rs", "pub mod good;\n"),
        (
            "crates/b/src/good.rs",
            "pub struct Kept;\n\npub fn broken( {\n",
        ),
    ]);
    assert_eq!(declared.len(), 1);
    assert_eq!(declared[0].path.as_str(), "crates/b/src/good.rs");
    assert!(
        matches!(declared[0].reason, GapReason::SyntaxErrors { line: 3, .. }),
        "the gap points at the broken construct, not at the file: {:?}",
        declared[0].reason
    );
}

#[test]
fn a_file_of_pure_garbage_stands_as_a_plain_file_and_a_gap() {
    let files = [
        MANIFEST_B,
        ("crates/b/src/lib.rs", "pub mod junk;\n"),
        ("crates/b/src/junk.rs", "\u{1}\u{2} not rust at all ][\n"),
    ];
    let structure = analyze(&files);
    assert_eq!(structure.gaps.len(), 1);
    assert_eq!(structure.gaps[0].path.as_str(), "crates/b/src/junk.rs");

    let graph = inspected(&files);
    assert_eq!(
        semantic_holder(&graph, "crates/b/src/junk.rs"),
        Some("package:b-lib".to_owned()),
        "the file the language could not read still stands where the tree puts it"
    );
}

#[test]
fn a_manifest_naming_its_package_with_nothing_is_a_gap() {
    let files = [
        MANIFEST_A,
        ("crates/b/Cargo.toml", "[package]\nname = \"\"\n"),
        ("crates/b/src/lib.rs", "pub struct Kept;\n"),
    ];
    let structure = analyze(&files);
    let packages: Vec<_> = structure
        .interpretations
        .iter()
        .filter(|i| i.element.primary_kind() == ElementKind::Package)
        .map(|i| i.element.id.as_str())
        .collect();
    assert_eq!(
        packages,
        ["package:a"],
        "a package nothing can name is no package"
    );
    assert_eq!(structure.gaps.len(), 1);
    assert_eq!(structure.gaps[0].path.as_str(), "crates/b/Cargo.toml");
    assert!(matches!(
        structure.gaps[0].reason,
        GapReason::ManifestUnreadable { .. }
    ));

    assert!(
        inspected(&files)
            .element(&ElementId::new("crates/b/src/lib.rs").unwrap())
            .is_some(),
        "the rest of the tree stands whatever one manifest fails to say"
    );
}

#[test]
fn a_broken_manifest_leaves_the_other_packages_standing() {
    let structure = analyze(&[
        MANIFEST_A,
        ("crates/b/Cargo.toml", "[package\nname = \"b-lib\"\n"),
    ]);
    let packages: Vec<_> = structure
        .interpretations
        .iter()
        .filter(|i| i.element.primary_kind() == ElementKind::Package)
        .map(|i| i.element.id.as_str())
        .collect();
    assert_eq!(packages, ["package:a"]);
    assert_eq!(structure.gaps.len(), 1);
    assert_eq!(structure.gaps[0].path.as_str(), "crates/b/Cargo.toml");
    assert!(matches!(
        structure.gaps[0].reason,
        GapReason::ManifestUnreadable { .. }
    ));
}

#[test]
fn a_reference_inside_a_broken_region_witnesses_nothing() {
    let structure = analyze(&[
        MANIFEST_B,
        ("crates/b/src/lib.rs", "pub mod store;\npub mod app;\n"),
        ("crates/b/src/store.rs", "pub struct Item;\n"),
        // The same name twice: once where the grammar can read it, and once
        // in a declaration cut off mid-way, whose unclosed parenthesis
        // swallows the rest of the file into one broken region.
        (
            "crates/b/src/app.rs",
            "pub struct Sound(pub crate::store::Item);\n\npub struct Broken(crate::store::Item\n",
        ),
    ]);
    let witnessed = dependencies(&structure);
    assert!(
        witnessed.contains(&(
            "crates/b/src/app.rs#type:Sound".to_owned(),
            "crates/b/src/store.rs#type:Item".to_owned()
        )),
        "the reading itself must still work, or the test below proves nothing: {witnessed:?}"
    );
    assert!(
        !witnessed.contains(&(
            "crates/b/src/app.rs".to_owned(),
            "crates/b/src/store.rs#type:Item".to_owned()
        )),
        "what stands inside a region the grammar could not read means nothing: {witnessed:?}"
    );
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
        extent_of(&structure, "crates/a/src/config.rs#function:Config::new"),
        Extent::Within {
            file: file("crates/a/src/config.rs"),
            parent: Some(ElementId::new("crates/a/src/config.rs#type:Config").unwrap())
        }
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
        reads_nothing(&structure, "crates/a/src/lib.rs#function:Config::refresh"),
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
        reads_nothing(&structure, "crates/a/src/lib.rs#function:Config::show"),
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

#[test]
fn a_workspace_manifest_that_names_no_package_reads_nothing() {
    let structure = analyze(&[
        ("Cargo.toml", "[workspace]\nmembers = [\"crates/a\"]\n"),
        MANIFEST_A,
    ]);

    assert_eq!(
        structure
            .interpretations
            .iter()
            .filter(|i| i.element.primary_kind() == ElementKind::Package)
            .count(),
        1,
        "a workspace root declares no package"
    );
    assert_eq!(
        extent_of(&structure, "package:a"),
        Extent::Directory(directory("crates/a")),
        "the member package keeps its own directory rather than the whole tree"
    );
}

#[test]
fn every_file_of_a_rust_project_stands_in_the_picture() {
    let graph = inspected(&[
        MANIFEST_A,
        ("crates/a/src/lib.rs", "mod foo;\n"),
        ("crates/a/src/foo.rs", ""),
        ("crates/a/tests/behaviour.rs", ""),
        ("crates/a/notes.txt", "loose"),
        ("README.md", "about"),
    ]);

    for path in [
        "crates/a/Cargo.toml",
        "crates/a/src/lib.rs",
        "crates/a/src/foo.rs",
        "crates/a/tests/behaviour.rs",
        "crates/a/notes.txt",
        "README.md",
    ] {
        let element = graph
            .element(&ElementId::new(path).unwrap())
            .unwrap_or_else(|| panic!("{path} stands in the picture"));
        assert!(
            element.fingerprint.is_some(),
            "{path} must speak through its contents"
        );
    }
}

#[test]
fn a_module_written_beside_its_directory_is_one_entry_of_the_listing() {
    let graph = inspected(&[
        MANIFEST_A,
        ("crates/a/src/lib.rs", "mod foo;\n"),
        ("crates/a/src/foo.rs", "pub mod bar;\n"),
        ("crates/a/src/foo/bar.rs", "pub struct Baz;\n"),
    ]);

    let foo = graph
        .element(&ElementId::new("crates/a/src/foo.rs").unwrap())
        .expect("the module spanning the file and the directory stands");
    assert_eq!(
        foo.substrate_aspect()
            .map(|aspect| aspect.name.as_str().to_owned()),
        Some("foo".to_owned()),
        "one box for both pieces, named by the name they share"
    );
    assert_eq!(
        foo.fingerprint,
        Some(cutaway_architecture::Fingerprint::of(b"pub mod bar;\n")),
        "the file the module is written in states what the box holds"
    );
    assert!(
        graph
            .element(&ElementId::new("crates/a/src/foo").unwrap())
            .is_none(),
        "the directory leaves no second entry beside the module"
    );
}

#[test]
fn the_default_picture_shows_the_tree_a_listing_would() {
    let graph = inspected(&[
        MANIFEST_A,
        MANIFEST_B,
        ("crates/a/src/lib.rs", "mod foo;\n"),
        ("crates/a/src/foo.rs", ""),
    ]);
    let holds = |frame: &str, id: &str| {
        graph.relations().any(|relation| {
            relation.kind == RelationKind::Contains
                && relation.from.as_str() == frame
                && relation.to.as_str() == id
        })
    };

    assert!(holds("project:fixture", "crates"));
    assert!(holds("crates", "package:a"));
    assert!(holds("package:a", "crates/a/Cargo.toml"));
    assert!(holds("package:a", "crates/a/src"));
    assert!(holds("crates/a/src", "crates/a/src/lib.rs"));
    assert!(holds("crates/a/src", "crates/a/src/foo.rs"));
}
