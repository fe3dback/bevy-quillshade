//! The "commit first" check of the Migration Tool (ADR 0046 "Orchestrator (D39)", "Git check"): a migration rewrites
//! source files in place, so the UI and CLI warn when the project has uncommitted changes. It never blocks.

use std::path::Path;
use std::process::Command;

/// The git state of a project directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitState {
    /// Inside a repository, nothing changed under the project directory.
    Clean,
    /// Inside a repository with `changed` modified / added / deleted / untracked paths under the project directory.
    Dirty { changed: usize },
    /// git runs but the directory is not inside a repository.
    NotARepo,
    /// git is missing or failed for another reason.
    Unavailable,
}

/// Runs `git -C <root> status --porcelain -- .` (only the project directory counts).
pub fn git_state(root: &Path) -> GitState {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain", "--", "."])
        // An English message to recognize "not a git repository"; no index lock taken for a read-only status.
        .env("LC_ALL", "C")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output();
    let Ok(output) = output else {
        return GitState::Unavailable;
    };
    if output.status.success() {
        let changed = String::from_utf8_lossy(&output.stdout).lines().filter(|l| !l.trim().is_empty()).count();
        return if changed == 0 { GitState::Clean } else { GitState::Dirty { changed } };
    }
    if String::from_utf8_lossy(&output.stderr).to_ascii_lowercase().contains("not a git repository") {
        GitState::NotARepo
    } else {
        GitState::Unavailable
    }
}
