//! Exercises the history the adapter reads out of real repositories created
//! with the git CLI.

mod support;

use std::fs;
use std::num::NonZeroUsize;
use std::path::Path;

use cutaway_inspection::ports::project_history::{ProjectHistory, ProjectHistoryError, VersionId};
use cutaway_inspection::ports::source_tree::SourceTree;
use cutaway_source_git::GitSourceTree;
use support::git;

fn empty_repository(dir: &Path) {
    git(dir, &["-c", "init.defaultBranch=main", "init"]);
}

fn commit(dir: &Path, path: &str, contents: &str, summary: &str) {
    let file = dir.join(path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, contents).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", summary]);
}

fn summaries(tree: &GitSourceTree, limit: usize) -> Vec<String> {
    tree.recent(NonZeroUsize::new(limit).unwrap())
        .unwrap()
        .into_iter()
        .map(|version| version.summary)
        .collect()
}

fn contents_of(tree: &dyn SourceTree, path: &str) -> String {
    let files = tree.files().unwrap();
    let file = files
        .iter()
        .find(|file| file.path.as_str() == path)
        .unwrap_or_else(|| panic!("the version holds {path}"));
    String::from_utf8(file.contents.clone()).unwrap()
}

#[test]
fn the_recent_versions_arrive_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    empty_repository(dir.path());
    commit(dir.path(), "notes.txt", "one\n", "first");
    commit(dir.path(), "notes.txt", "two\n", "second");
    commit(dir.path(), "notes.txt", "three\n", "third");

    let tree = GitSourceTree::open(dir.path()).unwrap();

    assert_eq!(summaries(&tree, 3), ["third", "second", "first"]);
}

#[test]
fn the_history_follows_first_parents_only() {
    let dir = tempfile::tempdir().unwrap();
    empty_repository(dir.path());
    commit(dir.path(), "notes.txt", "one\n", "on the mainline");
    git(dir.path(), &["checkout", "-b", "side"]);
    commit(dir.path(), "side.txt", "aside\n", "on the side branch");
    git(dir.path(), &["checkout", "main"]);
    git(
        dir.path(),
        &["merge", "--no-ff", "side", "-m", "merging the side branch"],
    );

    let tree = GitSourceTree::open(dir.path()).unwrap();
    let summaries = summaries(&tree, 10);

    assert_eq!(summaries, ["merging the side branch", "on the mainline"]);
}

#[test]
fn the_sources_at_a_version_are_the_sources_that_version_committed() {
    let dir = tempfile::tempdir().unwrap();
    empty_repository(dir.path());
    commit(dir.path(), "src/lib.rs", "pub fn v1() {}\n", "first shape");
    commit(dir.path(), "src/lib.rs", "pub fn v2() {}\n", "second shape");

    let tree = GitSourceTree::open(dir.path()).unwrap();
    let versions = tree.recent(NonZeroUsize::new(2).unwrap()).unwrap();
    let newer = tree.tree_at(&versions[0].id).unwrap();
    let older = tree.tree_at(&versions[1].id).unwrap();

    let path = "src/lib.rs";
    assert_eq!(contents_of(older.as_ref(), path), "pub fn v1() {}\n");
    assert_eq!(contents_of(newer.as_ref(), path), "pub fn v2() {}\n");
}

#[test]
fn an_unknown_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    empty_repository(dir.path());
    commit(dir.path(), "notes.txt", "one\n", "only");

    let tree = GitSourceTree::open(dir.path()).unwrap();
    let absent = VersionId::new("0".repeat(40)).unwrap();

    assert!(matches!(
        tree.tree_at(&absent),
        Err(ProjectHistoryError::UnknownVersion { .. })
    ));
}

#[test]
fn asking_for_more_versions_than_exist_answers_with_all_of_them() {
    let dir = tempfile::tempdir().unwrap();
    empty_repository(dir.path());
    commit(dir.path(), "notes.txt", "one\n", "first");
    commit(dir.path(), "notes.txt", "two\n", "second");

    let tree = GitSourceTree::open(dir.path()).unwrap();

    assert_eq!(summaries(&tree, 50), ["second", "first"]);
}

#[test]
fn a_repository_without_commits_has_no_readable_history() {
    let dir = tempfile::tempdir().unwrap();
    empty_repository(dir.path());

    let tree = GitSourceTree::open(dir.path()).unwrap();

    assert!(matches!(
        tree.recent(NonZeroUsize::new(1).unwrap()),
        Err(ProjectHistoryError::Unreadable { .. })
    ));
}
