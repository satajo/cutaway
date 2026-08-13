//! Exercises the adapter against real repositories created with the git CLI.

mod support;

use std::fs;
use std::path::Path;

use cutaway_inspection::ports::source_tree::SourceTree;
use cutaway_source_git::GitSourceTree;
use support::git;

fn committed_repository(dir: &Path) {
    git(dir, &["-c", "init.defaultBranch=main", "init"]);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/lib.rs"), "pub fn hello() {}\n").unwrap();
    fs::write(dir.join("README.md"), "# Fixture\n").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "initial"]);
}

#[test]
fn the_committed_head_state_is_visible_as_source_files() {
    let dir = tempfile::tempdir().unwrap();
    committed_repository(dir.path());

    let tree = GitSourceTree::open(dir.path()).unwrap();
    let files = tree.files().unwrap();

    let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["README.md", "src/lib.rs"]);
    let lib = files
        .iter()
        .find(|f| f.path.as_str() == "src/lib.rs")
        .unwrap();
    assert_eq!(lib.contents, b"pub fn hello() {}\n");
}

#[test]
fn uncommitted_changes_stay_invisible() {
    let dir = tempfile::tempdir().unwrap();
    committed_repository(dir.path());
    fs::write(dir.path().join("src/uncommitted.rs"), "pub fn later() {}\n").unwrap();

    let tree = GitSourceTree::open(dir.path()).unwrap();
    let files = tree.files().unwrap();

    assert!(
        !files
            .iter()
            .any(|f| f.path.as_str() == "src/uncommitted.rs")
    );
}

#[test]
fn a_directory_without_a_repository_is_rejected_on_open() {
    let dir = tempfile::tempdir().unwrap();
    assert!(GitSourceTree::open(dir.path()).is_err());
}

#[test]
fn a_refused_open_keeps_what_git_said_about_the_path() {
    let dir = tempfile::tempdir().unwrap();

    let Err(error) = GitSourceTree::open(dir.path()) else {
        panic!("a directory that is no repository does not open");
    };

    assert!(
        std::error::Error::source(&error).is_some(),
        "an open fails in many ways - no repository, an unreadable .git, a \
         broken config - and only git's own account tells them apart: {error}"
    );
}
