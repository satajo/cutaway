//! Dogfood: the wired-together adapters inspect Cutaway's own repository.
//!
//! Ignored by default: the tests need the `.git` directory, which the nix
//! build sandbox strips from the source. Run them from a checkout with
//! `cargo test -p cutaway -- --ignored`.

use std::collections::BTreeSet;
use std::path::Path;

use cutaway_analyzer_go::GoSourceAnalyzer;
use cutaway_analyzer_rust::RustSourceAnalyzer;
use cutaway_analyzer_typescript::TypeScriptSourceAnalyzer;
use cutaway_architecture::{ArchitectureGraph, ElementId, ElementKind, RelationKind};
use cutaway_inspection::inspect;
use cutaway_lenses::{Cut, boundary_view};
use cutaway_source_git::GitSourceTree;

fn cutaways_own_architecture() -> ArchitectureGraph {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two levels below the repository root");

    let tree = GitSourceTree::open(repository).unwrap();
    inspect(
        &tree,
        &[
            &RustSourceAnalyzer,
            &GoSourceAnalyzer,
            &TypeScriptSourceAnalyzer,
        ],
    )
    .unwrap()
}

/// The vocabulary that speaks what the languages read and nothing the
/// filesystem adds.
fn languages_alone() -> Cut {
    let mut cut = Cut::whole();
    cut.kinds.remove(&ElementKind::Directory);
    cut.kinds.remove(&ElementKind::File);
    cut
}

fn names(graph: &ArchitectureGraph, cut: &Cut) -> Vec<String> {
    boundary_view(graph, cut)
        .unwrap()
        .graph
        .elements()
        .map(|element| element.primary_name().to_string())
        .collect()
}

#[test]
#[ignore = "needs the .git directory, absent in the nix build sandbox"]
fn the_picture_starts_at_the_tree_a_listing_shows() {
    let graph = cutaways_own_architecture();
    let shown = names(&graph, &Cut::whole());

    for expected in ["crates", "Makefile", "Cargo.toml", "flake.nix"] {
        assert!(
            shown.contains(&expected.to_owned()),
            "the closed picture shows {shown:?}"
        );
    }
}

#[test]
#[ignore = "needs the .git directory, absent in the nix build sandbox"]
fn the_languages_reading_alone_shows_the_packages() {
    let graph = cutaways_own_architecture();
    let shown = names(&graph, &languages_alone());

    assert!(
        shown.contains(&"cutaway-architecture".to_owned()),
        "the packages hoist out of crates/, which draws nothing: {shown:?}"
    );
    assert!(
        !shown.contains(&"crates".to_owned()),
        "a hidden kind draws nothing"
    );
}

#[test]
#[ignore = "needs the .git directory, absent in the nix build sandbox"]
fn a_module_spanning_a_file_and_a_directory_holds_what_the_directory_holds() {
    let graph = cutaways_own_architecture();
    let ports = ElementId::new("crates/inspection/src/ports/mod.rs").unwrap();

    assert!(
        graph.element(&ports).is_some(),
        "the module ports is read out of mod.rs together with the directory beside it"
    );
    assert!(
        graph.relations().any(|relation| {
            relation.kind == RelationKind::Contains
                && relation.from == ports
                && relation.to
                    == ElementId::new("crates/inspection/src/ports/source_analyzer.rs").unwrap()
        }),
        "the ports of the directory stand inside the module that spans it"
    );
}

#[test]
#[ignore = "needs the .git directory, absent in the nix build sandbox"]
fn every_reading_keeps_the_workspaces_own_dependencies_in_the_picture() {
    let graph = cutaways_own_architecture();

    // The closed boxes, the opened structure, and the opened structure with
    // the items in the vocabulary.
    let mut structure = languages_alone();
    structure.open = graph.elements().map(|element| element.id.clone()).collect();
    structure.kinds = BTreeSet::from([ElementKind::Package, ElementKind::Module]);
    let mut everything = Cut::whole();
    everything.open.clone_from(&structure.open);
    for (reading, cut) in [
        ("closed", languages_alone()),
        ("structural", structure),
        ("complete", everything),
    ] {
        let view = boundary_view(&graph, &cut).unwrap();
        assert!(
            !view.provenance.is_empty(),
            "the {reading} reading shows connections"
        );
    }
}
