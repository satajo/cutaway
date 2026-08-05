//! Dogfood: the wired-together adapters inspect Cutaway's own repository.
//!
//! Ignored by default: the test needs the `.git` directory, which the nix
//! build sandbox strips from the source. Run it from a checkout with
//! `cargo test -p cutaway -- --ignored`.

use std::path::Path;

use cutaway_analyzer_go::GoSourceAnalyzer;
use cutaway_analyzer_rust::RustSourceAnalyzer;
use cutaway_analyzer_typescript::TypeScriptSourceAnalyzer;
use cutaway_inspection::inspect;
use cutaway_lenses::{Cut, Detail, boundary_view};
use cutaway_source_git::GitSourceTree;

#[test]
#[ignore = "needs the .git directory, absent in the nix build sandbox"]
fn the_boundary_lens_shows_cutaways_own_packages() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two levels below the repository root");

    let tree = GitSourceTree::open(repository).unwrap();
    let graph = inspect(
        &tree,
        &[
            &RustSourceAnalyzer,
            &GoSourceAnalyzer,
            &TypeScriptSourceAnalyzer,
        ],
    )
    .unwrap();

    let view = boundary_view(&graph, &Cut::uniform(Detail::Packages)).unwrap();
    let packages: Vec<&str> = view
        .graph
        .elements()
        .map(|element| element.name.as_str())
        .collect();
    assert!(
        packages.contains(&"cutaway-architecture"),
        "the view shows {packages:?}"
    );
    assert!(
        !view.provenance.is_empty(),
        "the workspace crates depend on each other"
    );

    for detail in Detail::ALL {
        let view = boundary_view(&graph, &Cut::uniform(detail)).unwrap();
        assert!(
            !view.provenance.is_empty(),
            "the {detail:?} detail shows connections"
        );
    }
}
