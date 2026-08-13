use std::collections::BTreeMap;

use cutaway_architecture::{ArchitectureGraph, ElementId, ElementKind, RelationKind};
use cutaway_inspection::inspect;
use cutaway_inspection::ports::source_analyzer::{
    AnalysisGap, Extent, GapReason, SourceAnalyzer, SourceStructure,
};
use cutaway_inspection::ports::source_tree::{
    DirectoryPath, ProjectName, SourceFile, SourcePath, SourceTree, SourceTreeError,
};

use crate::TypeScriptSourceAnalyzer;

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
    TypeScriptSourceAnalyzer.analyze(&sources(files))
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
    inspect(&Fixture(sources(files)), &[&TypeScriptSourceAnalyzer])
        .expect("the sources inspect")
        .graph
}

/// The containment parent one element stands under in the whole picture.
fn holder(graph: &ArchitectureGraph, id: &str) -> Option<String> {
    graph
        .relations()
        .find(|relation| relation.kind == RelationKind::Contains && relation.to.as_str() == id)
        .map(|relation| relation.from.as_str().to_owned())
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

/// The element a declaration hangs under, as the analyzer states it: the
/// declaration that holds it, else the file that writes it.
fn parent_of(structure: &SourceStructure, id: &str) -> Option<String> {
    match extent_of(structure, id) {
        Extent::Within { file, parent } => Some(parent.map_or_else(
            || file.as_str().to_owned(),
            |holder| holder.as_str().to_owned(),
        )),
        _ => None,
    }
}

fn name_of(structure: &SourceStructure, id: &str) -> String {
    structure
        .interpretations
        .iter()
        .find(|interpretation| interpretation.element.id.as_str() == id)
        .expect("the element exists")
        .element
        .primary_name()
        .as_str()
        .to_owned()
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
        .interpretations
        .iter()
        .filter(|i| i.element.primary_kind() == ElementKind::Package)
        .map(|i| i.element.id.as_str())
        .collect();
    assert_eq!(packages, ["package:app", "package:core"]);
}

#[test]
fn a_manifest_without_a_name_contributes_nothing() {
    let structure = analyze(&[("package.json", r#"{"private":true,"version":"1.0.0"}"#)]);
    assert!(structure.interpretations.is_empty());
}

#[test]
fn a_package_reads_the_directory_its_manifest_sits_in() {
    let structure = analyze(&[
        MANIFEST_APP,
        ("package.json", r#"{"name":"whole","main":"index.ts"}"#),
    ]);

    assert_eq!(
        extent_of(&structure, "package:app"),
        Extent::Directory(DirectoryPath::new("packages/app").unwrap())
    );
    assert_eq!(
        extent_of(&structure, "package:whole"),
        Extent::Root,
        "a manifest at the top of the tree makes the whole repository the package"
    );
}

#[test]
fn a_source_file_is_a_module_named_without_its_extension() {
    let files = [
        MANIFEST_APP,
        (
            "packages/app/src/utils/date.ts",
            "export const now = () => 1;\n",
        ),
    ];
    let structure = analyze(&files);
    assert_eq!(
        extent_of(&structure, "packages/app/src/utils/date.ts"),
        Extent::File(SourcePath::new("packages/app/src/utils/date.ts").unwrap())
    );
    assert_eq!(
        name_of(&structure, "packages/app/src/utils/date.ts"),
        "date",
        "the language names the file; where it lies is the tree's word"
    );

    assert_eq!(
        semantic_holder(&inspected(&files), "packages/app/src/utils/date.ts"),
        Some("package:app".to_owned()),
        "the directories between hold nothing else, so they dissolve"
    );
}

#[test]
fn the_directories_of_a_package_come_from_the_tree() {
    let graph = inspected(&[
        MANIFEST_APP,
        ("packages/app/src/a.ts", "export const a = () => 1;\n"),
        ("packages/app/src/b.ts", "export const b = () => 1;\n"),
    ]);

    let directory = graph
        .element(&ElementId::new("packages/app/src").unwrap())
        .expect("the directory groups two things, so it stands");
    assert_eq!(
        directory.primary_kind(),
        ElementKind::Directory,
        "a TypeScript directory is organization the author chose, and the \
         language reads nothing into it"
    );
    assert_eq!(directory.primary_name().as_str(), "src");
    assert_eq!(
        holder(&graph, "packages/app/src"),
        Some("package:app".to_owned())
    );
    assert_eq!(
        holder(&graph, "packages/app/src/a.ts"),
        Some("packages/app/src".to_owned())
    );
}

#[test]
fn items_and_imports_keep_naming_their_file_whatever_holds_it() {
    let files = [
        MANIFEST_APP,
        (
            "packages/app/src/widgets/a.ts",
            "import { Widget } from \"./b\";\nexport function use() {}\n",
        ),
        ("packages/app/src/widgets/b.ts", "export class Widget {}\n"),
    ];
    let structure = analyze(&files);
    assert_eq!(
        parent_of(&structure, "packages/app/src/widgets/a.ts#function:use"),
        Some("packages/app/src/widgets/a.ts".to_owned())
    );
    assert!(depends(
        &structure,
        "packages/app/src/widgets/a.ts",
        "packages/app/src/widgets/b.ts#type:Widget"
    ));

    assert_eq!(
        holder(&inspected(&files), "packages/app/src/widgets/a.ts"),
        Some("packages/app/src/widgets".to_owned())
    );
}

#[test]
fn the_entry_file_dissolves_into_its_package() {
    let files = [
        MANIFEST_CORE,
        ("packages/core/src/index.ts", "export class Session {}\n"),
    ];
    let structure = analyze(&files);
    assert!(
        reads_nothing(&structure, "packages/core/src/index.ts"),
        "the entry file is no module of its own"
    );

    assert_eq!(
        semantic_holder(
            &inspected(&files),
            "packages/core/src/index.ts#type:Session"
        ),
        Some("package:core".to_owned()),
        "the entry file's declarations read as the package's items"
    );
}

#[test]
fn a_main_field_naming_the_built_output_finds_the_typescript_source() {
    let files = [
        (
            "packages/core/package.json",
            r#"{"name":"core","main":"./index.js"}"#,
        ),
        ("packages/core/index.ts", "export class Session {}\n"),
    ];
    assert!(reads_nothing(&analyze(&files), "packages/core/index.ts"));
    assert_eq!(
        semantic_holder(&inspected(&files), "packages/core/index.ts#type:Session"),
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
    assert!(!reads_nothing(&structure, "packages/core/lib/session.ts"));
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
        .interpretations
        .iter()
        .filter(|i| {
            i.extent
                == Extent::Within {
                    file: SourcePath::new("packages/app/src/api.ts").unwrap(),
                    parent: None,
                }
        })
        .map(|e| {
            (
                e.element.primary_name().as_str().to_owned(),
                e.element.primary_kind(),
            )
        })
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
        .interpretations
        .iter()
        .filter(|i| i.element.primary_kind() == ElementKind::Function)
        .map(|i| i.element.primary_name().as_str().to_owned())
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
        .interpretations
        .iter()
        .filter(|i| {
            matches!(
                i.element.primary_kind(),
                ElementKind::Function | ElementKind::Type
            )
        })
        .map(|i| i.element.primary_name().as_str().to_owned())
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
            .interpretations
            .iter()
            .all(|i| i.element.primary_kind() != ElementKind::Function),
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
        .interpretations
        .iter()
        .filter(|i| {
            matches!(
                i.element.primary_kind(),
                ElementKind::Function | ElementKind::Type
            )
        })
        .map(|i| i.element.id.as_str().to_owned())
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
    assert!(reads_nothing(&structure, "package:left-pad"));
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
    let graph = inspected(&[("package.json", r#"{"name":"root"}"#), MANIFEST_APP]);
    assert_eq!(
        semantic_holder(&graph, "package:app"),
        Some("package:root".to_owned()),
        "a root manifest makes the whole repository its package's territory"
    );
    assert_eq!(
        semantic_holder(&graph, "package:root"),
        Some("project:fixture".to_owned())
    );
}

#[test]
fn a_file_with_a_syntax_error_still_yields_the_declarations_that_parsed() {
    let structure = analyze(&[
        MANIFEST_APP,
        (
            "packages/app/src/widget.ts",
            "export class Kept {}\n\nexport function broken( {\n",
        ),
    ]);
    assert_eq!(
        parent_of(&structure, "packages/app/src/widget.ts#type:Kept"),
        Some("packages/app/src/widget.ts".to_owned())
    );
}

#[test]
fn a_file_with_a_syntax_error_is_a_gap_at_the_failing_line() {
    let declared = gaps(&[
        MANIFEST_APP,
        (
            "packages/app/src/widget.ts",
            "export class Kept {}\n\nexport function broken( {\n",
        ),
    ]);
    assert_eq!(declared.len(), 1);
    assert_eq!(declared[0].path.as_str(), "packages/app/src/widget.ts");
    assert!(
        matches!(declared[0].reason, GapReason::SyntaxErrors { line: 3, .. }),
        "the gap points at the broken construct, not at the file: {:?}",
        declared[0].reason
    );
}

#[test]
fn a_file_of_pure_garbage_stands_as_a_plain_file_and_a_gap() {
    let files = [
        MANIFEST_APP,
        (
            "packages/app/src/junk.ts",
            "\u{1}\u{2} not typescript at all ][\n",
        ),
    ];
    let structure = analyze(&files);
    assert_eq!(structure.gaps.len(), 1);
    assert_eq!(structure.gaps[0].path.as_str(), "packages/app/src/junk.ts");

    let graph = inspected(&files);
    assert_eq!(
        semantic_holder(&graph, "packages/app/src/junk.ts"),
        Some("package:app".to_owned()),
        "the file the language could not read still stands where the tree puts it"
    );
}

#[test]
fn a_malformed_manifest_is_a_gap() {
    let declared = gaps(&[("package.json", "{\"name\": }")]);
    assert_eq!(declared.len(), 1);
    assert_eq!(declared[0].path.as_str(), "package.json");
    assert!(matches!(
        declared[0].reason,
        GapReason::ManifestUnreadable { .. }
    ));
}

#[test]
fn a_manifest_naming_its_package_with_nothing_is_a_gap() {
    let files = [
        MANIFEST_APP,
        ("packages/core/package.json", r#"{"name":""}"#),
        ("packages/core/src/index.ts", "export class Kept {}\n"),
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
        ["package:app"],
        "a package nothing can name is no package"
    );
    assert_eq!(structure.gaps.len(), 1);
    assert_eq!(
        structure.gaps[0].path.as_str(),
        "packages/core/package.json"
    );
    assert!(matches!(
        structure.gaps[0].reason,
        GapReason::ManifestUnreadable { .. }
    ));

    assert!(
        inspected(&files)
            .element(&ElementId::new("packages/core/src/index.ts").unwrap())
            .is_some(),
        "the rest of the tree stands whatever one manifest fails to say"
    );
}

#[test]
fn a_broken_manifest_leaves_the_other_packages_standing() {
    let structure = analyze(&[MANIFEST_APP, ("packages/core/package.json", "{\"name\": }")]);
    let packages: Vec<_> = structure
        .interpretations
        .iter()
        .filter(|i| i.element.primary_kind() == ElementKind::Package)
        .map(|i| i.element.id.as_str())
        .collect();
    assert_eq!(packages, ["package:app"]);
    assert_eq!(structure.gaps.len(), 1);
    assert_eq!(
        structure.gaps[0].path.as_str(),
        "packages/core/package.json"
    );
}

#[test]
fn a_reference_inside_a_broken_region_witnesses_nothing() {
    let structure = analyze(&[
        MANIFEST_APP,
        MANIFEST_CORE,
        ("packages/core/src/index.ts", "export class Widget {}\n"),
        // The namespace import binds the qualifier without naming anything
        // behind it, so only what the file writes can witness the class: once
        // where the grammar reads it, and once past an unclosed brace that
        // swallows the rest of the file into one broken region.
        (
            "packages/app/src/index.ts",
            "import * as core from \"core\";\n\n\
             export function sound() { return new core.Widget(); }\n\n\
             export function broken() { { new core.Widget(\n",
        ),
    ]);
    let witnessed = dependencies(&structure);
    assert!(
        witnessed.contains(&(
            "packages/app/src/index.ts#function:sound".to_owned(),
            "packages/core/src/index.ts#type:Widget".to_owned()
        )),
        "the reading itself must still work, or the test below proves nothing: {witnessed:?}"
    );
    assert!(
        !witnessed.contains(&(
            "package:app".to_owned(),
            "packages/core/src/index.ts#type:Widget".to_owned()
        )),
        "what stands inside a region the grammar could not read means nothing: {witnessed:?}"
    );
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
    let graph = inspected(&[
        ("tools/build.ts", "export const run = () => 1;\n"),
        ("tools/check.ts", "export const check = () => 2;\n"),
    ]);
    assert_eq!(holder(&graph, "tools"), Some("project:fixture".to_owned()));
    assert_eq!(holder(&graph, "tools/build.ts"), Some("tools".to_owned()));
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
        reads_nothing(&structure, "packages/app/src/store.ts#function:Store.warm"),
        "a private-modifier member is the class's internals"
    );
    assert!(
        reads_nothing(
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

#[test]
fn every_file_of_a_typescript_project_stands_in_the_picture() {
    let graph = inspected(&[
        ("package.json", r#"{"workspaces":["packages/*"]}"#),
        MANIFEST_APP,
        ("packages/app/src/index.ts", "export const go = () => 1;\n"),
        ("packages/app/src/util.ts", "export const u = 1;\n"),
        (
            "packages/app/node_modules/dep/index.js",
            "module.exports = 1;\n",
        ),
        ("packages/app/README.md", "docs"),
    ]);

    for path in [
        "package.json",
        "packages/app/package.json",
        "packages/app/src/index.ts",
        "packages/app/src/util.ts",
        "packages/app/node_modules/dep/index.js",
        "packages/app/README.md",
    ] {
        let element = graph
            .element(&ElementId::new(path).unwrap())
            .unwrap_or_else(|| panic!("{path} stands in the picture"));
        assert!(
            element.fingerprint.is_some(),
            "{path} must speak through its contents"
        );
    }
    assert_eq!(
        semantic_holder(&graph, "packages/app/README.md"),
        Some("package:app".to_owned()),
        "a file the language never read still lies in the package's directory"
    );
}
