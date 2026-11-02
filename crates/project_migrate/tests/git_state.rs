//! `git_state` (ADR 0046 "Git check"). The only test of this binary, so it may set the process environment: in the
//! `nono` sandbox git needs `GIT_CONFIG_GLOBAL=/dev/null` and `XDG_CONFIG_HOME=/tmp` (AGENTS.md §5); the library
//! itself never sets them.

use std::path::Path;
use std::process::Command;

use project_migrate::{GitState, git_state};

fn git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git").arg("-C").arg(dir).args(args).output().is_ok_and(|o| o.status.success())
}

#[test]
fn git_states() {
    // SAFETY: this binary runs a single test, so no other thread reads the environment concurrently.
    unsafe {
        std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
        std::env::set_var("XDG_CONFIG_HOME", "/tmp");
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    }
    let plain = tempfile::tempdir().unwrap();
    let state = git_state(plain.path());
    if !Command::new("git").arg("--version").output().is_ok_and(|o| o.status.success()) {
        assert_eq!(state, GitState::Unavailable);
        return;
    }
    assert!(matches!(state, GitState::NotARepo | GitState::Unavailable), "{state:?}");
    if state == GitState::Unavailable {
        // E.g. a temp directory inside an unreadable or unusual repository setup: nothing more to check.
        return;
    }
    assert_eq!(git_state(&plain.path().join("missing")), GitState::Unavailable, "git -C on a missing directory");

    // Only the project directory counts.
    let repo = tempfile::tempdir().unwrap();
    assert!(git(repo.path(), &["init", "-q"]), "git init");
    let game = repo.path().join("game");
    std::fs::create_dir(&game).unwrap();
    assert_eq!(git_state(&game), GitState::Clean);
    std::fs::write(repo.path().join("outside.txt"), "x").unwrap();
    assert_eq!(git_state(&game), GitState::Clean);
    std::fs::write(game.join("a.ron"), "a").unwrap();
    assert_eq!(git_state(&game), GitState::Dirty { changed: 1 });
    // `changed` counts porcelain entries: git reports an untracked directory as one entry.
    std::fs::create_dir(game.join("sub")).unwrap();
    std::fs::write(game.join("sub/b.ron"), "b").unwrap();
    assert!(matches!(git_state(&game), GitState::Dirty { changed } if changed >= 1));
}
