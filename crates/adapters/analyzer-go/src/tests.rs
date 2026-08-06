use cutaway_architecture::{ElementId, ElementKind, RelationKind};
use cutaway_inspection::ports::source_analyzer::{
    AnalyzedElement, SourceAnalysisError, SourceAnalyzer, SourceStructure,
};
use cutaway_inspection::ports::source_tree::{SourceFile, SourcePath};

use crate::GoSourceAnalyzer;

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
    GoSourceAnalyzer.analyze(&files)
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

fn element(structure: &SourceStructure, id: &str) -> AnalyzedElement {
    structure
        .elements
        .iter()
        .find(|e| e.element.id.as_str() == id)
        .expect("the element exists")
        .clone()
}

fn children_of(structure: &SourceStructure, parent: &str) -> Vec<(String, ElementKind)> {
    structure
        .elements
        .iter()
        .filter(|e| e.parent.as_ref().map(ElementId::as_str) == Some(parent))
        .map(|e| (e.element.name.as_str().to_owned(), e.element.kind))
        .collect()
}

fn has_element(structure: &SourceStructure, id: &str) -> bool {
    structure
        .elements
        .iter()
        .any(|e| e.element.id.as_str() == id)
}

const ALPHA: (&str, &str) = ("alpha/go.mod", "module example.com/alpha\n\ngo 1.22\n");
const BETA: (&str, &str) = ("beta/go.mod", "module example.com/beta\n\ngo 1.22\n");

#[test]
fn modules_are_discovered_from_their_manifests() {
    let structure = analyze(&[ALPHA, BETA]);
    let packages: Vec<_> = structure
        .elements
        .iter()
        .filter(|e| e.element.kind == ElementKind::Package)
        .map(|e| e.element.id.as_str())
        .collect();
    assert_eq!(
        packages,
        ["package:example.com/alpha", "package:example.com/beta"]
    );
}

#[test]
fn a_manifest_without_a_module_path_is_rejected() {
    let result = try_analyze(&[("go.mod", "go 1.22\n")]);
    assert!(matches!(
        result,
        Err(SourceAnalysisError::Unparseable { .. })
    ));
}

#[test]
fn a_directory_of_go_files_is_a_module_within_its_package() {
    let structure = analyze(&[
        ALPHA,
        ("alpha/internal/server/serve.go", "package server\n"),
    ]);
    let directory = structure
        .elements
        .iter()
        .find(|e| e.element.id.as_str() == "alpha/internal/server")
        .expect("the directory is an element");
    assert_eq!(directory.element.kind, ElementKind::Module);
    assert_eq!(directory.element.name.as_str(), "internal/server");
    assert_eq!(
        directory.parent.as_ref().map(ElementId::as_str),
        Some("package:example.com/alpha")
    );
}

#[test]
fn the_module_root_directory_dissolves_into_its_package() {
    let structure = analyze(&[
        ALPHA,
        ("alpha/main.go", "package main\n\ntype Config struct{}\n"),
    ]);
    assert!(
        !has_element(&structure, "alpha"),
        "the module root directory is no element of its own"
    );
    assert_eq!(
        parent_of(&structure, "alpha/main.go#type:Config"),
        Some("package:example.com/alpha".to_owned()),
        "the root directory's declarations are the package's items"
    );
}

#[test]
fn a_directory_belongs_to_the_nearest_ancestor_directory_of_go_files() {
    let structure = analyze(&[
        ALPHA,
        ("alpha/a/a.go", "package a\n"),
        ("alpha/a/b/c/c.go", "package c\n"),
    ]);
    assert!(
        !has_element(&structure, "alpha/a/b"),
        "a directory without go files is no package"
    );
    assert_eq!(
        parent_of(&structure, "alpha/a/b/c"),
        Some("alpha/a".to_owned())
    );
}

#[test]
fn an_import_of_another_module_witnesses_the_dependency_between_the_packages() {
    let structure = analyze(&[
        ALPHA,
        BETA,
        (
            "alpha/main.go",
            "package main\n\nimport \"example.com/beta\"\n\nfunc main() { beta.Run() }\n",
        ),
        ("beta/run.go", "package beta\n\nfunc Run() {}\n"),
    ]);
    assert!(dependencies(&structure).contains(&(
        "package:example.com/alpha".to_owned(),
        "package:example.com/beta".to_owned()
    )));
}

#[test]
fn an_import_within_a_module_witnesses_the_dependency_between_its_directories() {
    let structure = analyze(&[
        ALPHA,
        (
            "alpha/main.go",
            "package main\n\nimport \"example.com/alpha/internal/store\"\n\nvar _ = store.Open\n",
        ),
        ("alpha/internal/store/store.go", "package store\n"),
    ]);
    assert!(dependencies(&structure).contains(&(
        "package:example.com/alpha".to_owned(),
        "alpha/internal/store".to_owned()
    )));
}

#[test]
fn a_qualified_reference_resolves_onto_the_exported_declaration() {
    let structure = analyze(&[
        ALPHA,
        (
            "alpha/main.go",
            "package main\n\nimport \"example.com/alpha/server\"\n\nfunc main() {\n\tvar h server.Handler\n\t_ = h\n}\n",
        ),
        (
            "alpha/server/server.go",
            "package server\n\ntype Handler struct{}\n",
        ),
    ]);
    assert!(dependencies(&structure).contains(&(
        "package:example.com/alpha".to_owned(),
        "alpha/server/server.go#type:Handler".to_owned()
    )));
}

#[test]
fn an_aliased_import_resolves_references_through_the_alias() {
    let structure = analyze(&[
        ALPHA,
        (
            "alpha/main.go",
            "package main\n\nimport srv \"example.com/alpha/server\"\n\nfunc main() { srv.Start() }\n",
        ),
        (
            "alpha/server/server.go",
            "package server\n\nfunc Start() {}\n",
        ),
    ]);
    assert!(dependencies(&structure).contains(&(
        "package:example.com/alpha".to_owned(),
        "alpha/server/server.go#function:Start".to_owned()
    )));
}

#[test]
fn an_import_binds_the_package_clause_name_over_the_last_path_segment() {
    let structure = analyze(&[
        ALPHA,
        (
            "alpha/main.go",
            "package main\n\nimport \"example.com/alpha/internal/store-v2\"\n\nfunc main() { store.Open() }\n",
        ),
        (
            "alpha/internal/store-v2/store.go",
            "package store\n\nfunc Open() {}\n",
        ),
    ]);
    assert!(dependencies(&structure).contains(&(
        "package:example.com/alpha".to_owned(),
        "alpha/internal/store-v2/store.go#function:Open".to_owned()
    )));
}

#[test]
fn a_reference_to_an_unexported_name_lands_on_its_directory() {
    let structure = analyze(&[
        ALPHA,
        (
            "alpha/main.go",
            "package main\n\nimport \"example.com/alpha/server\"\n\nfunc main() { server.start() }\n",
        ),
        (
            "alpha/server/server.go",
            "package server\n\nfunc start() {}\n",
        ),
    ]);
    let dependencies = dependencies(&structure);
    assert!(dependencies.contains(&(
        "package:example.com/alpha".to_owned(),
        "alpha/server".to_owned()
    )));
    assert!(!dependencies.iter().any(|(_, to)| to.contains("#function:")));
}

#[test]
fn exported_declarations_become_items_and_unexported_ones_stay_internals() {
    let structure = analyze(&[
        ALPHA,
        (
            "alpha/server/server.go",
            "package server\n\nfunc Start() {}\n\nfunc stop() {}\n\ntype Handler struct{}\n\ntype (\n\tConfig struct{}\n\tOption = int\n\tsecret struct{}\n)\n\nconst Version = 1\n\nvar Registry map[string]int\n",
        ),
    ]);
    assert_eq!(
        children_of(&structure, "alpha/server"),
        [
            ("Start".to_owned(), ElementKind::Function),
            ("Handler".to_owned(), ElementKind::Type),
            ("Config".to_owned(), ElementKind::Type),
            ("Option".to_owned(), ElementKind::Type),
        ],
        "only exported functions and types are the directory's surface"
    );
}

#[test]
fn methods_do_not_become_items() {
    let structure = analyze(&[
        ALPHA,
        (
            "alpha/server/server.go",
            "package server\n\ntype Handler struct{}\n\nfunc (h *Handler) Serve() {}\n\nfunc (h Handler) Close() {}\n",
        ),
    ]);
    assert_eq!(
        children_of(&structure, "alpha/server"),
        [("Handler".to_owned(), ElementKind::Type)],
        "a method belongs to its type, not to the directory's namespace"
    );
}

#[test]
fn test_files_declare_nothing_but_their_imports_witness_dependencies() {
    let structure = analyze(&[
        ALPHA,
        ("alpha/server/server.go", "package server\n"),
        (
            "alpha/server/server_test.go",
            "package server\n\nimport \"example.com/alpha/internal/store\"\n\ntype Fixture struct{}\n\nfunc Helper() { store.Open() }\n",
        ),
        (
            "alpha/internal/store/store.go",
            "package store\n\nfunc Open() {}\n",
        ),
    ]);
    assert!(
        children_of(&structure, "alpha/server/server_test.go").is_empty(),
        "nothing a test file declares can be imported, so nothing of it is surface"
    );
    let dependencies = dependencies(&structure);
    assert!(dependencies.contains(&(
        "alpha/server/server_test.go".to_owned(),
        "alpha/internal/store".to_owned()
    )));
    assert!(dependencies.contains(&(
        "alpha/server/server_test.go".to_owned(),
        "alpha/internal/store/store.go#function:Open".to_owned()
    )));
}

#[test]
fn a_test_file_is_a_module_among_the_files_of_its_directory() {
    let structure = analyze(&[
        ALPHA,
        ("alpha/server/server.go", "package server\n"),
        ("alpha/server/server_test.go", "package server\n"),
    ]);
    let test_file = element(&structure, "alpha/server/server_test.go");
    assert_eq!(test_file.element.kind, ElementKind::Module);
    assert_eq!(test_file.element.name.as_str(), "server_test");
    assert_eq!(
        test_file.parent.as_ref().map(ElementId::as_str),
        Some("alpha/server"),
        "a test file is built with its directory, so it counts among its files"
    );
}

#[test]
fn every_file_of_a_directory_of_several_is_a_module_within_it() {
    let structure = analyze(&[
        ALPHA,
        ("alpha/internal/server/serve.go", "package server\n"),
        ("alpha/internal/server/routes.go", "package server\n"),
    ]);
    for (id, name) in [
        ("alpha/internal/server/serve.go", "serve"),
        ("alpha/internal/server/routes.go", "routes"),
    ] {
        let file = element(&structure, id);
        assert_eq!(file.element.kind, ElementKind::Module);
        assert_eq!(file.element.name.as_str(), name);
        assert_eq!(
            file.parent.as_ref().map(ElementId::as_str),
            Some("alpha/internal/server")
        );
    }
}

#[test]
fn the_only_file_of_a_directory_dissolves_into_it() {
    let structure = analyze(&[
        ALPHA,
        (
            "alpha/main.go",
            "package main\n\nimport \"example.com/alpha/server\"\n",
        ),
        (
            "alpha/server/server.go",
            "package server\n\ntype Handler struct{}\n",
        ),
    ]);
    assert!(
        !has_element(&structure, "alpha/server/server.go"),
        "one file groups nothing the directory does not already show"
    );
    assert_eq!(
        children_of(&structure, "alpha/server"),
        [("Handler".to_owned(), ElementKind::Type)],
        "the lone file's declarations are the directory's items"
    );
    assert!(dependencies(&structure).contains(&(
        "package:example.com/alpha".to_owned(),
        "alpha/server".to_owned()
    )));
}

#[test]
fn an_exported_declaration_belongs_to_the_file_that_declares_it() {
    let structure = analyze(&[
        ALPHA,
        (
            "alpha/server/serve.go",
            "package server\n\nfunc Serve() {}\n",
        ),
        (
            "alpha/server/routes.go",
            "package server\n\ntype Router struct{}\n\nfunc route() {}\n",
        ),
    ]);
    assert_eq!(
        children_of(&structure, "alpha/server/serve.go"),
        [("Serve".to_owned(), ElementKind::Function)]
    );
    assert_eq!(
        children_of(&structure, "alpha/server/routes.go"),
        [("Router".to_owned(), ElementKind::Type)]
    );
}

#[test]
fn an_import_witnesses_the_dependency_from_the_file_that_states_it() {
    let structure = analyze(&[
        ALPHA,
        (
            "alpha/server/serve.go",
            "package server\n\nimport \"example.com/alpha/internal/store\"\n\nvar _ = store.Open\n",
        ),
        ("alpha/server/routes.go", "package server\n"),
        (
            "alpha/internal/store/store.go",
            "package store\n\nfunc Open() {}\n",
        ),
    ]);
    let dependencies = dependencies(&structure);
    assert!(dependencies.contains(&(
        "alpha/server/serve.go".to_owned(),
        "alpha/internal/store".to_owned()
    )));
    assert!(dependencies.contains(&(
        "alpha/server/serve.go".to_owned(),
        "alpha/internal/store/store.go#function:Open".to_owned()
    )));
    assert!(
        !dependencies.iter().any(|(from, _)| from == "alpha/server"),
        "the directory states no import of its own; its files do"
    );
}

#[test]
fn the_files_of_the_module_root_sit_directly_in_the_package() {
    let structure = analyze(&[
        ALPHA,
        ("alpha/main.go", "package main\n"),
        ("alpha/config.go", "package main\n\ntype Config struct{}\n"),
    ]);
    assert!(
        !has_element(&structure, "alpha"),
        "the module root directory is still no element of its own"
    );
    assert_eq!(
        parent_of(&structure, "alpha/main.go"),
        Some("package:example.com/alpha".to_owned())
    );
    assert_eq!(
        parent_of(&structure, "alpha/config.go"),
        Some("package:example.com/alpha".to_owned())
    );
    assert_eq!(
        parent_of(&structure, "alpha/config.go#type:Config"),
        Some("alpha/config.go".to_owned()),
        "the root's declarations still belong to the file that declares them"
    );
}

#[test]
fn standard_library_and_third_party_imports_stay_outside_the_architecture() {
    let structure = analyze(&[
        ALPHA,
        (
            "alpha/main.go",
            "package main\n\nimport (\n\t\"fmt\"\n\t\"net/http\"\n\t\"github.com/spf13/cobra\"\n)\n\nfunc main() { fmt.Println(http.StatusOK, cobra.Command{}) }\n",
        ),
    ]);
    assert!(dependencies(&structure).is_empty());
}

#[test]
fn directories_the_go_tool_excludes_stay_outside_the_architecture() {
    let structure = analyze(&[
        ALPHA,
        ("alpha/main.go", "package main\n"),
        ("alpha/vendor/example.com/dep/dep.go", "package dep\n"),
        ("alpha/testdata/golden/golden.go", "package golden\n"),
        ("alpha/_tools/gen.go", "package tools\n"),
        ("alpha/.hidden/hidden.go", "package hidden\n"),
        ("alpha/server/_ignored.go", "package server\n"),
    ]);
    let directories: Vec<_> = structure
        .elements
        .iter()
        .filter(|e| e.element.kind == ElementKind::Module)
        .map(|e| e.element.id.as_str())
        .collect();
    assert!(directories.is_empty(), "found {directories:?}");
}

#[test]
fn a_file_the_go_tool_excludes_is_never_parsed() {
    let result = try_analyze(&[
        ALPHA,
        ("alpha/main.go", "package main\n"),
        ("alpha/vendor/example.com/dep/dep.go", "package dep func(\n"),
    ]);
    assert!(
        result.is_ok(),
        "a broken vendored file is outside the build, so it cannot fail analysis"
    );
}

#[test]
fn an_import_of_a_missing_directory_stops_at_the_deepest_existing_one() {
    let structure = analyze(&[
        ALPHA,
        (
            "alpha/main.go",
            "package main\n\nimport \"example.com/alpha/internal/store/sql\"\n",
        ),
        ("alpha/internal/store/store.go", "package store\n"),
    ]);
    assert!(dependencies(&structure).contains(&(
        "package:example.com/alpha".to_owned(),
        "alpha/internal/store".to_owned()
    )));
}

#[test]
fn a_nested_module_carves_its_subtree_out_of_the_enclosing_module() {
    let structure = analyze(&[
        ("go.mod", "module example.com/outer\n"),
        ("tools/go.mod", "module example.com/outer/tools\n"),
        (
            "main.go",
            "package main\n\nimport \"example.com/outer/tools/gen\"\n\nfunc main() { gen.Run() }\n",
        ),
        ("tools/gen/gen.go", "package gen\n\nfunc Run() {}\n"),
    ]);
    assert_eq!(
        parent_of(&structure, "package:example.com/outer/tools"),
        Some("package:example.com/outer".to_owned()),
        "the nested module sits inside the enclosing module's directory"
    );
    assert_eq!(
        parent_of(&structure, "tools/gen"),
        Some("package:example.com/outer/tools".to_owned()),
        "the deeper manifest owns the subtree"
    );
    assert!(dependencies(&structure).contains(&(
        "package:example.com/outer".to_owned(),
        "tools/gen/gen.go#function:Run".to_owned()
    )));
}

#[test]
fn a_file_with_syntax_errors_is_rejected() {
    let result = try_analyze(&[ALPHA, ("alpha/main.go", "package main\n\nfunc broken( {\n")]);
    assert!(matches!(
        result,
        Err(SourceAnalysisError::Unparseable { .. })
    ));
}

#[test]
fn a_blank_or_dot_import_still_witnesses_the_directory_dependency() {
    let structure = analyze(&[
        ALPHA,
        (
            "alpha/main.go",
            "package main\n\nimport (\n\t_ \"example.com/alpha/internal/driver\"\n\t. \"example.com/alpha/internal/dsl\"\n)\n",
        ),
        ("alpha/internal/driver/driver.go", "package driver\n"),
        ("alpha/internal/dsl/dsl.go", "package dsl\n"),
    ]);
    let dependencies = dependencies(&structure);
    assert!(dependencies.contains(&(
        "package:example.com/alpha".to_owned(),
        "alpha/internal/driver".to_owned()
    )));
    assert!(dependencies.contains(&(
        "package:example.com/alpha".to_owned(),
        "alpha/internal/dsl".to_owned()
    )));
}

#[test]
fn go_files_outside_every_module_stay_outside_the_architecture() {
    let structure = analyze(&[
        ALPHA,
        ("alpha/main.go", "package main\n"),
        (
            "scripts/loose.go",
            "package scripts\n\ntype Loose struct{}\n",
        ),
    ]);
    assert!(!has_element(&structure, "scripts"));
    assert!(!has_element(&structure, "scripts/loose.go#type:Loose"));
}

#[test]
fn the_same_dependency_witnessed_by_import_and_reference_is_one_relation() {
    let structure = analyze(&[
        ALPHA,
        (
            "alpha/main.go",
            "package main\n\nimport \"example.com/alpha/server\"\n\nfunc main() { server.unexported() }\n",
        ),
        (
            "alpha/server/server.go",
            "package server\n\nfunc unexported() {}\n",
        ),
    ]);
    let landings: Vec<_> = dependencies(&structure)
        .into_iter()
        .filter(|(_, to)| to == "alpha/server")
        .collect();
    assert_eq!(landings.len(), 1);
}

#[test]
fn current_go_syntax_reaches_the_architecture() {
    let src = r#"//go:build linux && amd64

package server

import "fmt"

type Number interface {
	~int | ~float64
}

type Set[T comparable] struct {
	items map[T]struct{}
}

func Map[T, U any](in []T, f func(T) U) []U {
	out := make([]U, 0, len(in))
	for _, v := range in {
		out = append(out, f(v))
	}
	return out
}

func (s *Set[T]) Add(item T) { s.items[item] = struct{}{} }

func run(ctx interface{ Done() <-chan struct{} }) {
	select {
	case <-ctx.Done():
		fmt.Println("done")
	default:
	}
	go func() { defer func() { _ = recover() }() }()
	ch := make(chan int, 1)
	ch <- 1
	for i := range 10 {
		_ = i
	}
}
"#;
    let structure = analyze(&[ALPHA, ("alpha/server/server.go", src)]);
    assert_eq!(
        children_of(&structure, "alpha/server"),
        [
            ("Number".to_owned(), ElementKind::Type),
            ("Set".to_owned(), ElementKind::Type),
            ("Map".to_owned(), ElementKind::Function),
        ],
        "build tags, type sets, and generics are ordinary Go, not a parse failure"
    );
}
