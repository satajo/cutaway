use cutaway_architecture::{ElementKind, RelationKind};
use cutaway_inspection::ports::source_analyzer::{
    SourceAnalysisError, SourceAnalyzer, SourceStructure,
};
use cutaway_inspection::ports::source_tree::{SourceFile, SourcePath};

use crate::TypeScriptSourceAnalyzer;

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
    TypeScriptSourceAnalyzer.analyze(&files)
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

fn name_of(structure: &SourceStructure, id: &str) -> String {
    structure
        .elements
        .iter()
        .find(|e| e.element.id.as_str() == id)
        .expect("the element exists")
        .element
        .name
        .as_str()
        .to_owned()
}

fn has_element(structure: &SourceStructure, id: &str) -> bool {
    structure
        .elements
        .iter()
        .any(|e| e.element.id.as_str() == id)
}

fn dependencies(structure: &SourceStructure) -> Vec<(String, String)> {
    structure
        .relations
        .iter()
        .filter(|r| r.kind == RelationKind::DependsOn)
        .map(|r| (r.from.as_str().to_owned(), r.to.as_str().to_owned()))
        .collect()
}

fn depends(structure: &SourceStructure, from: &str, to: &str) -> bool {
    dependencies(structure).contains(&(from.to_owned(), to.to_owned()))
}

const MANIFEST_APP: (&str, &str) = (
    "packages/app/package.json",
    r#"{"name":"app","main":"src/index.ts","dependencies":{"core":"*","left-pad":"1"}}"#,
);
const MANIFEST_CORE: (&str, &str) = (
    "packages/core/package.json",
    r#"{"name":"core","main":"src/index.ts"}"#,
);

#[test]
fn packages_are_discovered_from_their_manifests() {
    let structure = analyze(&[
        ("package.json", r#"{"workspaces":["packages/*"]}"#),
        MANIFEST_APP,
        MANIFEST_CORE,
    ]);
    let packages: Vec<_> = structure
        .elements
        .iter()
        .filter(|e| e.element.kind == ElementKind::Package)
        .map(|e| e.element.id.as_str())
        .collect();
    assert_eq!(packages, ["package:app", "package:core"]);
}

#[test]
fn a_manifest_without_a_name_contributes_nothing() {
    let structure = analyze(&[("package.json", r#"{"private":true,"version":"1.0.0"}"#)]);
    assert!(structure.elements.is_empty());
}

#[test]
fn source_files_are_modules_within_their_package_named_without_their_extension() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/utils/date.ts",
            "export const now = () => 1;\n",
        ),
    ]);
    assert_eq!(
        parent_of(&structure, "packages/app/src/utils/date.ts"),
        Some("package:app".to_owned())
    );
    let names: Vec<_> = structure
        .elements
        .iter()
        .filter(|e| e.element.kind == ElementKind::Module)
        .map(|e| e.element.name.as_str().to_owned())
        .collect();
    assert_eq!(names, ["src/utils/date"]);
}

#[test]
fn a_directory_holding_two_files_is_a_boundary_within_its_package() {
    let structure = analyze(&[
        MANIFEST_APP,
        ("packages/app/src/a.ts", "export const a = () => 1;\n"),
        ("packages/app/src/b.ts", "export const b = () => 1;\n"),
    ]);
    let directory = structure
        .elements
        .iter()
        .find(|e| e.element.id.as_str() == "packages/app/src")
        .expect("the directory is an element");
    assert_eq!(
        directory.element.kind,
        ElementKind::Directory,
        "a TypeScript directory is organization the author chose, and the \
         language reads nothing into it"
    );
    assert_eq!(directory.element.name.as_str(), "src");
    assert_eq!(
        parent_of(&structure, "packages/app/src"),
        Some("package:app".to_owned())
    );
    assert_eq!(
        parent_of(&structure, "packages/app/src/a.ts"),
        Some("packages/app/src".to_owned())
    );
    assert_eq!(
        parent_of(&structure, "packages/app/src/b.ts"),
        Some("packages/app/src".to_owned())
    );
}

#[test]
fn a_chain_of_directories_holding_one_child_each_dissolves_into_the_group_at_its_end() {
    let structure = analyze(&[
        MANIFEST_APP,
        ("packages/app/src/a/b/c/one.ts", "export const one = 1;\n"),
        ("packages/app/src/a/b/c/two.ts", "export const two = 2;\n"),
    ]);
    for dissolved in [
        "packages/app/src",
        "packages/app/src/a",
        "packages/app/src/a/b",
    ] {
        assert!(
            !has_element(&structure, dissolved),
            "{dissolved} groups one thing and is no element"
        );
    }
    assert_eq!(
        parent_of(&structure, "packages/app/src/a/b/c"),
        Some("package:app".to_owned())
    );
    assert_eq!(
        parent_of(&structure, "packages/app/src/a/b/c/one.ts"),
        Some("packages/app/src/a/b/c".to_owned())
    );
}

#[test]
fn a_directory_groups_what_its_dissolved_subdirectories_hand_up() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/plugins/index.ts",
            "export const plugins = [];\n",
        ),
        (
            "packages/app/src/plugins/cleanup/cleanup.ts",
            "export const cleanup = () => 1;\n",
        ),
    ]);
    assert!(
        !has_element(&structure, "packages/app/src/plugins/cleanup"),
        "a directory of one file dissolves"
    );
    assert_eq!(
        parent_of(&structure, "packages/app/src/plugins"),
        Some("package:app".to_owned()),
        "the dissolved subdirectory leaves its file standing in the directory above"
    );
    assert_eq!(
        parent_of(&structure, "packages/app/src/plugins/index.ts"),
        Some("packages/app/src/plugins".to_owned())
    );
    assert_eq!(
        parent_of(&structure, "packages/app/src/plugins/cleanup/cleanup.ts"),
        Some("packages/app/src/plugins".to_owned())
    );
}

#[test]
fn a_directory_counts_a_surviving_subdirectory_among_its_children() {
    let structure = analyze(&[
        MANIFEST_APP,
        ("packages/app/src/util.ts", "export const now = () => 1;\n"),
        (
            "packages/app/src/widgets/panel.ts",
            "export class Panel {}\n",
        ),
        (
            "packages/app/src/widgets/button.ts",
            "export class Button {}\n",
        ),
    ]);
    assert_eq!(
        parent_of(&structure, "packages/app/src"),
        Some("package:app".to_owned()),
        "one file and one surviving subdirectory are two children"
    );
    assert_eq!(
        parent_of(&structure, "packages/app/src/widgets"),
        Some("packages/app/src".to_owned())
    );
    assert_eq!(
        parent_of(&structure, "packages/app/src/util.ts"),
        Some("packages/app/src".to_owned())
    );
    assert_eq!(
        parent_of(&structure, "packages/app/src/widgets/panel.ts"),
        Some("packages/app/src/widgets".to_owned())
    );
}

#[test]
fn a_module_parents_onto_the_nearest_surviving_directory_above_it() {
    let structure = analyze(&[
        MANIFEST_APP,
        ("packages/app/src/a.ts", "export const a = 1;\n"),
        ("packages/app/src/b.ts", "export const b = 2;\n"),
        (
            "packages/app/src/lonely/deep.ts",
            "export const deep = 3;\n",
        ),
    ]);
    assert!(!has_element(&structure, "packages/app/src/lonely"));
    assert_eq!(
        parent_of(&structure, "packages/app/src/lonely/deep.ts"),
        Some("packages/app/src".to_owned())
    );
}

#[test]
fn a_name_says_only_what_the_element_above_it_does_not_already_spell() {
    let structure = analyze(&[
        MANIFEST_APP,
        ("packages/app/src/util.ts", "export const now = () => 1;\n"),
        (
            "packages/app/src/widgets/panel.ts",
            "export class Panel {}\n",
        ),
        (
            "packages/app/src/widgets/button.ts",
            "export class Button {}\n",
        ),
    ]);
    assert_eq!(name_of(&structure, "packages/app/src"), "src");
    assert_eq!(name_of(&structure, "packages/app/src/widgets"), "widgets");
    assert_eq!(name_of(&structure, "packages/app/src/util.ts"), "util");
    assert_eq!(
        name_of(&structure, "packages/app/src/widgets/panel.ts"),
        "panel"
    );
    assert_eq!(
        structure
            .elements
            .iter()
            .find(|e| e.element.id.as_str() == "packages/app/src/widgets")
            .map(|e| e.element.id.as_str().to_owned()),
        Some("packages/app/src/widgets".to_owned()),
        "the id keeps the whole path, because it identifies rather than reads"
    );
}

#[test]
fn a_name_keeps_the_segments_the_directories_that_dissolved_gave_up() {
    let structure = analyze(&[
        MANIFEST_APP,
        ("packages/app/src/a.ts", "export const a = 1;\n"),
        ("packages/app/src/b.ts", "export const b = 2;\n"),
        (
            "packages/app/src/lonely/deep.ts",
            "export const deep = 3;\n",
        ),
    ]);
    assert_eq!(
        name_of(&structure, "packages/app/src/lonely/deep.ts"),
        "lonely/deep",
        "no box spells `lonely`, so the module has to"
    );
}

#[test]
fn the_entry_file_counts_for_no_directory() {
    let structure = analyze(&[
        MANIFEST_APP,
        ("packages/app/src/index.ts", "export class Session {}\n"),
        ("packages/app/src/a.ts", "export const a = 1;\n"),
    ]);
    assert!(
        !has_element(&structure, "packages/app/src"),
        "the dissolved entry leaves one child, which groups nothing"
    );
    assert_eq!(
        parent_of(&structure, "packages/app/src/a.ts"),
        Some("package:app".to_owned())
    );
}

#[test]
fn items_and_imports_keep_naming_their_file_when_directories_are_modules() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/widgets/a.ts",
            "import { Widget } from \"./b\";\nexport function use() {}\n",
        ),
        ("packages/app/src/widgets/b.ts", "export class Widget {}\n"),
    ]);
    assert_eq!(
        parent_of(&structure, "packages/app/src/widgets/a.ts"),
        Some("packages/app/src/widgets".to_owned())
    );
    assert_eq!(
        parent_of(&structure, "packages/app/src/widgets/a.ts#function:use"),
        Some("packages/app/src/widgets/a.ts".to_owned())
    );
    assert!(depends(
        &structure,
        "packages/app/src/widgets/a.ts",
        "packages/app/src/widgets/b.ts#type:Widget"
    ));
}

#[test]
fn the_entry_file_dissolves_into_its_package() {
    let structure = analyze(&[
        MANIFEST_CORE,
        ("packages/core/src/index.ts", "export class Session {}\n"),
    ]);
    assert!(
        !has_element(&structure, "packages/core/src/index.ts"),
        "the entry file is no element of its own"
    );
    assert_eq!(
        parent_of(&structure, "packages/core/src/index.ts#type:Session"),
        Some("package:core".to_owned()),
        "the entry file's declarations are the package's items"
    );
}

#[test]
fn a_main_field_naming_the_built_output_finds_the_typescript_source() {
    let structure = analyze(&[
        (
            "packages/core/package.json",
            r#"{"name":"core","main":"./index.js"}"#,
        ),
        ("packages/core/index.ts", "export class Session {}\n"),
    ]);
    assert!(!has_element(&structure, "packages/core/index.ts"));
    assert_eq!(
        parent_of(&structure, "packages/core/index.ts#type:Session"),
        Some("package:core".to_owned())
    );
}

#[test]
fn a_package_without_a_resolvable_entry_keeps_its_files_as_modules() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/core/package.json",
            r#"{"name":"core","main":"dist/index.js"}"#,
        ),
        ("packages/core/lib/session.ts", "export class Session {}\n"),
        (
            "packages/app/src/index.ts",
            "import { Session } from \"core\";\n",
        ),
    ]);
    assert!(has_element(&structure, "packages/core/lib/session.ts"));
    assert!(depends(&structure, "package:app", "package:core"));
}

#[test]
fn a_relative_import_witnesses_the_dependency_between_modules() {
    let structure = analyze(&[
        MANIFEST_APP,
        ("packages/app/src/a.ts", "import \"./b\";\n"),
        ("packages/app/src/b.ts", "export const x = () => 1;\n"),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts",
        "packages/app/src/b.ts"
    ));
}

#[test]
fn a_specifier_finds_the_typescript_source_with_and_without_its_extension() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "import \"./b\";\nimport \"./c.js\";\n",
        ),
        ("packages/app/src/b.ts", ""),
        ("packages/app/src/c.ts", ""),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts",
        "packages/app/src/b.ts"
    ));
    assert!(depends(
        &structure,
        "packages/app/src/a.ts",
        "packages/app/src/c.ts"
    ));
}

#[test]
fn a_directory_specifier_finds_its_index_file() {
    let structure = analyze(&[
        MANIFEST_APP,
        ("packages/app/src/a.ts", "import \"./widgets\";\n"),
        ("packages/app/src/widgets/index.ts", ""),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts",
        "packages/app/src/widgets/index.ts"
    ));
}

#[test]
fn a_named_import_resolves_onto_the_exported_declaration() {
    let structure = analyze(&[
        MANIFEST_APP,
        ("packages/app/src/a.ts", "import { Widget } from \"./b\";\n"),
        ("packages/app/src/b.ts", "export class Widget {}\n"),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts",
        "packages/app/src/b.ts#type:Widget"
    ));
}

#[test]
fn an_import_of_an_unexported_name_lands_on_the_module() {
    let structure = analyze(&[
        MANIFEST_APP,
        ("packages/app/src/a.ts", "import { secret } from \"./b\";\n"),
        (
            "packages/app/src/b.ts",
            "function secret() {}\nexport const other = () => 1;\n",
        ),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts",
        "packages/app/src/b.ts"
    ));
    assert!(!depends(
        &structure,
        "packages/app/src/a.ts",
        "packages/app/src/b.ts#function:secret"
    ));
}

#[test]
fn a_barrel_reexport_forwards_the_import_onto_the_declaring_modules_item() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "import { Widget } from \"./widgets\";\n",
        ),
        (
            "packages/app/src/widgets/index.ts",
            "export { Widget } from \"./widget\";\n",
        ),
        (
            "packages/app/src/widgets/widget.ts",
            "export class Widget {}\n",
        ),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts",
        "packages/app/src/widgets/widget.ts#type:Widget"
    ));
}

#[test]
fn a_renamed_reexport_resolves_onto_the_item_it_forwards_to() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "import { Widget } from \"./barrel\";\n",
        ),
        (
            "packages/app/src/barrel.ts",
            "export { Thing as Widget } from \"./inner\";\n",
        ),
        ("packages/app/src/inner.ts", "export class Thing {}\n"),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts",
        "packages/app/src/inner.ts#type:Thing"
    ));
}

#[test]
fn a_reexported_default_resolves_onto_the_item_behind_it() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "import { Widget } from \"./barrel\";\n",
        ),
        (
            "packages/app/src/barrel.ts",
            "export { default as Widget } from \"./inner\";\n",
        ),
        (
            "packages/app/src/inner.ts",
            "export default class Thing {}\n",
        ),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts",
        "packages/app/src/inner.ts#type:Thing"
    ));
}

#[test]
fn a_type_only_import_witnesses_the_dependency() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "import type { Config } from \"./b\";\n",
        ),
        ("packages/app/src/b.ts", "export interface Config {}\n"),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts",
        "packages/app/src/b.ts#type:Config"
    ));
}

#[test]
fn an_import_equals_require_witnesses_the_dependency() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "import helper = require(\"./b\");\n",
        ),
        ("packages/app/src/b.ts", "export const go = () => 1;\n"),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts",
        "packages/app/src/b.ts"
    ));
}

#[test]
fn a_wildcard_reexport_forwards_a_name_its_module_does_not_declare() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "import { Thing } from \"./barrel\";\n",
        ),
        ("packages/app/src/barrel.ts", "export * from \"./inner\";\n"),
        ("packages/app/src/inner.ts", "export class Thing {}\n"),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts",
        "packages/app/src/inner.ts#type:Thing"
    ));
}

#[test]
fn reexports_pointing_at_each_other_stop_at_the_module_closing_the_cycle() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "import { Thing } from \"./one\";\n",
        ),
        (
            "packages/app/src/one.ts",
            "export { Thing } from \"./two\";\n",
        ),
        (
            "packages/app/src/two.ts",
            "export { Thing } from \"./one\";\n",
        ),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts",
        "packages/app/src/one.ts"
    ));
}

#[test]
fn a_namespace_imports_qualified_reference_resolves_onto_the_item() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "import * as widgets from \"./b\";\nexport const go = () => widgets.build();\n",
        ),
        ("packages/app/src/b.ts", "export function build() {}\n"),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts#function:go",
        "packages/app/src/b.ts#function:build"
    ));
}

#[test]
fn a_call_speaks_from_the_declaration_whose_body_writes_it() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "import { build } from \"./b\";\nexport function go() {\n  build();\n}\n",
        ),
        ("packages/app/src/b.ts", "export function build() {}\n"),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts#function:go",
        "packages/app/src/b.ts#function:build"
    ));
}

#[test]
fn the_functions_of_one_module_wire_to_each_other() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/api.ts",
            "export function outer() {\n  inner();\n}\nexport function inner() {}\n",
        ),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/api.ts#function:outer",
        "packages/app/src/api.ts#function:inner"
    ));
}

#[test]
fn a_declaration_naming_its_own_modules_internals_says_nothing() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/api.ts",
            "export function run() {\n  helper();\n}\nfunction helper() {}\n",
        ),
    ]);
    assert!(
        dependencies(&structure).is_empty(),
        "an unexported sibling is the module's internals, not a dependency"
    );
}

#[test]
fn a_constructed_class_is_the_constructing_declarations_dependency() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "import { Widget } from \"./b\";\nexport const make = () => new Widget();\n",
        ),
        ("packages/app/src/b.ts", "export class Widget {}\n"),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts#function:make",
        "packages/app/src/b.ts#type:Widget"
    ));
}

#[test]
fn a_class_depends_on_what_it_extends_and_implements() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "import { Base, Contract } from \"./b\";\nexport class Panel extends Base implements Contract {}\n",
        ),
        (
            "packages/app/src/b.ts",
            "export class Base {}\nexport interface Contract {}\n",
        ),
    ]);
    for target in ["type:Base", "type:Contract"] {
        assert!(
            depends(
                &structure,
                "packages/app/src/a.ts#type:Panel",
                &format!("packages/app/src/b.ts#{target}")
            ),
            "a heritage clause couples the class to {target}"
        );
    }
}

#[test]
fn a_type_annotation_witnesses_the_type_it_names() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "import { Cfg, Out } from \"./b\";\nexport function convert(input: Cfg): Out {\n  return load(input);\n}\nexport interface Holder {\n  part: Cfg;\n}\n",
        ),
        (
            "packages/app/src/b.ts",
            "export interface Cfg {}\nexport interface Out {}\n",
        ),
    ]);
    for target in ["type:Cfg", "type:Out"] {
        assert!(
            depends(
                &structure,
                "packages/app/src/a.ts#function:convert",
                &format!("packages/app/src/b.ts#{target}")
            ),
            "the signature of convert speaks {target}"
        );
    }
    assert!(depends(
        &structure,
        "packages/app/src/a.ts#type:Holder",
        "packages/app/src/b.ts#type:Cfg"
    ));
}

#[test]
fn a_class_member_speaks_as_the_class_that_holds_it() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "import { store } from \"./b\";\nexport class Config {\n  load = () => {\n    store();\n  };\n}\n",
        ),
        ("packages/app/src/b.ts", "export function store() {}\n"),
    ]);
    // An arrow-function property is no method definition, so it declares no
    // element of its own and its writing stays the class's.
    assert!(depends(
        &structure,
        "packages/app/src/a.ts#type:Config",
        "packages/app/src/b.ts#function:store"
    ));
}

#[test]
fn a_rendered_component_is_the_rendering_components_dependency() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/panel.tsx",
            "import { BookCover } from \"./cover\";\nexport const Panel = () => <div><BookCover /></div>;\n",
        ),
        (
            "packages/app/src/cover.tsx",
            "export const BookCover = () => <img />;\n",
        ),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/panel.tsx#function:Panel",
        "packages/app/src/cover.tsx#function:BookCover"
    ));
}

#[test]
fn a_lowercase_jsx_name_is_a_markup_tag_and_names_no_declaration() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/panel.tsx",
            "export const div = () => 1;\nexport const Wrapper = () => <div />;\n",
        ),
    ]);
    assert!(
        dependencies(&structure).is_empty(),
        "`<div />` is the host's markup, not the exported `div` of this module"
    );
}

#[test]
fn a_default_import_binds_its_local_name_to_the_targets_default_export() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "import Widget from \"./b\";\nexport const make = () => new Widget();\n",
        ),
        ("packages/app/src/b.ts", "export default class Thing {}\n"),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts#function:make",
        "packages/app/src/b.ts#type:Thing"
    ));
}

#[test]
fn a_renamed_import_binds_the_name_the_file_writes() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "import { Widget as W } from \"./b\";\nexport const make = () => new W();\n",
        ),
        ("packages/app/src/b.ts", "export class Widget {}\n"),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts#function:make",
        "packages/app/src/b.ts#type:Widget"
    ));
}

#[test]
fn a_reference_outside_every_declaration_stays_with_the_module() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "import * as widgets from \"./b\";\nconst shared: widgets.Widget = init();\nexport function unrelated() {}\n",
        ),
        ("packages/app/src/b.ts", "export class Widget {}\n"),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts",
        "packages/app/src/b.ts#type:Widget"
    ));
    assert!(
        !depends(
            &structure,
            "packages/app/src/a.ts#function:unrelated",
            "packages/app/src/b.ts#type:Widget"
        ),
        "a top-level reference belongs to no declaration"
    );
}

#[test]
fn an_unexported_declarations_references_stay_with_the_module() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "import { serve } from \"./b\";\nfunction quietly() {\n  serve();\n}\nexport function shown() {}\n",
        ),
        ("packages/app/src/b.ts", "export function serve() {}\n"),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts",
        "packages/app/src/b.ts#function:serve"
    ));
    assert!(
        !depends(
            &structure,
            "packages/app/src/a.ts#function:shown",
            "packages/app/src/b.ts#function:serve"
        ),
        "an unexported declaration is no element, so what it writes is the module's"
    );
}

#[test]
fn exported_functions_classes_interfaces_aliases_and_enums_become_items() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/api.ts",
            "export function connect() {}\nexport class Session {}\nexport interface Config {}\nexport type Id = string;\nexport enum Mode { On }\n",
        ),
    ]);
    let items: Vec<_> = structure
        .elements
        .iter()
        .filter(|e| {
            e.parent
                .as_ref()
                .map(cutaway_architecture::ElementId::as_str)
                == Some("packages/app/src/api.ts")
        })
        .map(|e| (e.element.name.as_str().to_owned(), e.element.kind))
        .collect();
    assert_eq!(
        items,
        [
            ("connect".to_owned(), ElementKind::Function),
            ("Session".to_owned(), ElementKind::Type),
            ("Config".to_owned(), ElementKind::Type),
            ("Id".to_owned(), ElementKind::Type),
            ("Mode".to_owned(), ElementKind::Type),
        ]
    );
}

#[test]
fn an_exported_const_is_an_item_only_when_it_holds_a_function() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/api.ts",
            "export const render = () => 1;\nexport const build = function () {};\nexport const LIMIT = 10;\n",
        ),
    ]);
    let items: Vec<_> = structure
        .elements
        .iter()
        .filter(|e| e.element.kind == ElementKind::Function)
        .map(|e| e.element.name.as_str().to_owned())
        .collect();
    assert_eq!(items, ["render", "build"]);
}

#[test]
fn an_export_list_marks_local_declarations_as_exported() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/api.ts",
            "function connect() {}\nclass Session {}\nfunction hidden() {}\nexport { connect, Session as Handle };\n",
        ),
    ]);
    let items: Vec<_> = structure
        .elements
        .iter()
        .filter(|e| matches!(e.element.kind, ElementKind::Function | ElementKind::Type))
        .map(|e| e.element.name.as_str().to_owned())
        .collect();
    assert_eq!(items, ["connect", "Session"]);
}

#[test]
fn a_default_export_of_a_named_function_is_an_item_and_an_anonymous_one_is_none() {
    let named = analyze(&[
        MANIFEST_APP,
        ("packages/app/src/a.ts", "import run from \"./b\";\n"),
        (
            "packages/app/src/b.ts",
            "export default function run() {}\n",
        ),
    ]);
    assert!(depends(
        &named,
        "packages/app/src/a.ts",
        "packages/app/src/b.ts#function:run"
    ));

    let anonymous = analyze(&[
        MANIFEST_APP,
        ("packages/app/src/a.ts", "import run from \"./b\";\n"),
        ("packages/app/src/b.ts", "export default () => 1;\n"),
    ]);
    assert!(
        anonymous
            .elements
            .iter()
            .all(|e| e.element.kind != ElementKind::Function),
        "an anonymous default declares no item"
    );
    assert!(depends(
        &anonymous,
        "packages/app/src/a.ts",
        "packages/app/src/b.ts"
    ));
}

#[test]
fn a_require_call_witnesses_the_dependency() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.js",
            "const helper = require(\"./b\");\n",
        ),
        ("packages/app/src/b.js", "exports.go = () => 1;\n"),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.js",
        "packages/app/src/b.js"
    ));
}

#[test]
fn a_dynamic_import_witnesses_the_dependency() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "export const load = async () => import(\"./b\");\n",
        ),
        ("packages/app/src/b.ts", "export const x = () => 1;\n"),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/a.ts",
        "packages/app/src/b.ts"
    ));
}

#[test]
fn commonjs_exports_assignments_declare_the_modules_items() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/b.js",
            "exports.connect = function () {};\nmodule.exports.Session = class {};\n",
        ),
        (
            "packages/app/src/c.js",
            "function run() {}\nfunction hidden() {}\nmodule.exports = { run };\n",
        ),
    ]);
    let items: Vec<_> = structure
        .elements
        .iter()
        .filter(|e| matches!(e.element.kind, ElementKind::Function | ElementKind::Type))
        .map(|e| e.element.id.as_str().to_owned())
        .collect();
    assert_eq!(
        items,
        [
            "packages/app/src/b.js#function:connect",
            "packages/app/src/b.js#type:Session",
            "packages/app/src/c.js#function:run",
        ]
    );
}

#[test]
fn a_bare_import_of_another_project_package_witnesses_the_dependency() {
    let structure = analyze(&[
        MANIFEST_APP,
        MANIFEST_CORE,
        (
            "packages/app/src/index.ts",
            "import { Session } from \"core\";\n",
        ),
        (
            "packages/core/src/index.ts",
            "export { Session } from \"./thing\";\n",
        ),
        ("packages/core/src/thing.ts", "export class Session {}\n"),
    ]);
    assert!(depends(&structure, "package:app", "package:core"));
    assert!(depends(
        &structure,
        "package:app",
        "packages/core/src/thing.ts#type:Session"
    ));
}

#[test]
fn a_subpath_import_resolves_to_the_file_and_an_unresolved_one_to_the_package() {
    let structure = analyze(&[
        MANIFEST_APP,
        MANIFEST_CORE,
        (
            "packages/app/src/index.ts",
            "import \"core/src/thing\";\nimport \"core/missing\";\n",
        ),
        ("packages/core/src/index.ts", ""),
        ("packages/core/src/thing.ts", "export class Session {}\n"),
    ]);
    assert!(depends(
        &structure,
        "package:app",
        "packages/core/src/thing.ts"
    ));
    assert!(depends(&structure, "package:app", "package:core"));
}

#[test]
fn third_party_and_builtin_imports_stay_outside_the_architecture() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/index.ts",
            "import fs from \"node:fs\";\nimport pad from \"left-pad\";\n",
        ),
    ]);
    assert!(dependencies(&structure).is_empty());
}

#[test]
fn node_modules_stays_outside_the_architecture_and_its_files_are_never_parsed() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/node_modules/left-pad/package.json",
            r#"{"name":"left-pad","main":"index.js"}"#,
        ),
        (
            "packages/app/node_modules/left-pad/index.js",
            "function ( {\n",
        ),
        (
            "packages/app/src/index.ts",
            "import pad from \"left-pad\";\n",
        ),
    ]);
    assert!(!has_element(&structure, "package:left-pad"));
    assert!(dependencies(&structure).is_empty());
}

#[test]
fn a_tsx_component_and_a_jsx_file_parse_and_their_imports_witness_dependencies() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/panel.tsx",
            "import { Button } from \"./button\";\nexport const Panel = () => <div><Button /></div>;\n",
        ),
        (
            "packages/app/src/button.jsx",
            "export const Button = () => <button>ok</button>;\n",
        ),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/panel.tsx",
        "packages/app/src/button.jsx#function:Button"
    ));
}

#[test]
fn a_package_nested_in_another_packages_directory_parents_onto_it() {
    let structure = analyze(&[("package.json", r#"{"name":"root"}"#), MANIFEST_APP]);
    assert_eq!(
        parent_of(&structure, "package:app"),
        Some("package:root".to_owned())
    );
    assert_eq!(parent_of(&structure, "package:root"), None);
}

#[test]
fn a_file_with_syntax_errors_is_rejected() {
    let result = try_analyze(&[
        MANIFEST_APP,
        ("packages/app/src/index.ts", "export function broken( {\n"),
    ]);
    assert!(matches!(
        result,
        Err(SourceAnalysisError::Unparseable { .. })
    ));
}

#[test]
fn a_malformed_manifest_is_rejected() {
    let result = try_analyze(&[("package.json", "{\"name\": }")]);
    assert!(matches!(
        result,
        Err(SourceAnalysisError::Unparseable { .. })
    ));
}

#[test]
fn files_outside_every_package_are_modules_under_the_project_root() {
    let structure = analyze(&[("tools/build.ts", "export const run = () => 1;\n")]);
    assert_eq!(parent_of(&structure, "tools/build.ts"), None);
    assert_eq!(
        parent_of(&structure, "tools/build.ts#function:run"),
        Some("tools/build.ts".to_owned())
    );
}

#[test]
fn a_directory_outside_every_package_groups_under_the_project_root() {
    let structure = analyze(&[
        ("tools/build.ts", "export const run = () => 1;\n"),
        ("tools/check.ts", "export const check = () => 2;\n"),
    ]);
    assert_eq!(parent_of(&structure, "tools"), None);
    assert_eq!(
        parent_of(&structure, "tools/build.ts"),
        Some("tools".to_owned())
    );
}

#[test]
fn the_same_dependency_witnessed_twice_is_one_relation() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/a.ts",
            "import \"./b\";\nexport const load = () => import(\"./b\");\n",
        ),
        ("packages/app/src/b.ts", ""),
    ]);
    let to_b: Vec<_> = dependencies(&structure)
        .into_iter()
        .filter(|(_, to)| to == "packages/app/src/b.ts")
        .collect();
    assert_eq!(to_b.len(), 1);
}

#[test]
fn a_public_method_is_an_element_inside_its_class() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/store.ts",
            "export class Store {\n  load(): void {}\n}\n",
        ),
        (
            "packages/app/src/other.ts",
            "export const other = () => 1;\n",
        ),
    ]);
    assert_eq!(
        parent_of(&structure, "packages/app/src/store.ts#function:Store.load"),
        Some("packages/app/src/store.ts#type:Store".to_owned())
    );
}

#[test]
fn a_public_methods_reference_speaks_from_the_method() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/api.ts",
            "export const fetchBooks = () => [];\n",
        ),
        (
            "packages/app/src/store.ts",
            "import { fetchBooks } from './api';\n\nexport class Store {\n  refresh(): void {\n    fetchBooks();\n  }\n}\n",
        ),
        (
            "packages/app/src/other.ts",
            "export const other = () => 1;\n",
        ),
    ]);
    assert!(depends(
        &structure,
        "packages/app/src/store.ts#function:Store.refresh",
        "packages/app/src/api.ts#function:fetchBooks"
    ));
}

#[test]
fn a_private_members_writing_stays_the_classs_own() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/api.ts",
            "export const fetchBooks = () => [];\n",
        ),
        (
            "packages/app/src/store.ts",
            "import { fetchBooks } from './api';\n\nexport class Store {\n  private warm(): void {\n    fetchBooks();\n  }\n  #prime(): void {\n    fetchBooks();\n  }\n}\n",
        ),
        (
            "packages/app/src/other.ts",
            "export const other = () => 1;\n",
        ),
    ]);
    assert!(
        !has_element(&structure, "packages/app/src/store.ts#function:Store.warm"),
        "a private-modifier member is the class's internals"
    );
    assert!(
        !has_element(
            &structure,
            "packages/app/src/store.ts#function:Store.#prime"
        ),
        "a #name never leaves the class"
    );
    assert!(depends(
        &structure,
        "packages/app/src/store.ts#type:Store",
        "packages/app/src/api.ts#function:fetchBooks"
    ));
}
