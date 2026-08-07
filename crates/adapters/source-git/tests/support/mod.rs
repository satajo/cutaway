//! Fixtures built with the real git CLI, isolated from the developer's own
//! git configuration so the repositories look the same on every machine.

use std::path::Path;
use std::process::Command;

pub fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .status()
        .expect("the git CLI is available");
    assert!(status.success(), "git {args:?} failed");
}
