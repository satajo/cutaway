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
        "packages/app/src/a.ts",
        "packages/app/src/b.ts#function:build"
    ));
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
