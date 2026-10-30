//! Watches `quillshade/` for external changes (git checkout, other editors) and the game's `assets/` for files
//! coming and going (the Game Assets tree, design §4). Events under `assets/quillshade/` (our own generation
//! output) are dropped, so writing it never triggers a refresh.

use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::layout::{OUTPUT_DIR, Project};

/// Whether `path` lies under `assets/quillshade/` (design §4). Accepts paths absolute under the project root and
/// paths relative to it; compares components, so `assets/quillshadey/x` is not own output.
pub fn is_own_output(project: &Project, path: &Path) -> bool {
    let rel = path.strip_prefix(&project.root).unwrap_or(path);
    let mut comps = rel.components().filter(|c| !matches!(c, Component::CurDir));
    Path::new(OUTPUT_DIR).components().all(|want| comps.next() == Some(want))
}

/// Keeps the watcher alive; changed paths (under `quillshade/` or `assets/`) arrive on `rx`.
pub struct ProjectWatcher {
    _watcher: RecommendedWatcher,
    pub rx: Receiver<PathBuf>,
}

impl ProjectWatcher {
    pub fn new(project: &Project) -> notify::Result<Self> {
        let (tx, rx) = channel();
        let owner = project.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res
                && (ev.kind.is_modify() || ev.kind.is_create() || ev.kind.is_remove())
            {
                for p in ev.paths.into_iter().filter(|p| !is_own_output(&owner, p)) {
                    let _ = tx.send(p);
                }
            }
        })?;
        // A missing source directory (fresh project before the first save) is simply not watched.
        let src = project.quillshade_dir();
        if src.is_dir() {
            watcher.watch(&src, RecursiveMode::Recursive)?;
        }
        // Best effort: a huge `assets/` may exceed the OS watch limit; `quillshade/` is still watched then.
        let assets = project.assets_dir();
        if assets.is_dir() {
            let _ = watcher.watch(&assets, RecursiveMode::Recursive);
        }
        Ok(ProjectWatcher { _watcher: watcher, rx })
    }

    /// Drains pending events.
    pub fn poll(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.rx.try_iter().collect();
        v.sort();
        v.dedup();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_output_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
        std::fs::create_dir_all(dir.path().join("assets")).unwrap();
        let p = Project::open(dir.path()).unwrap();
        let abs = |r: &str| p.root.join(r);
        assert!(is_own_output(&p, &abs("assets/quillshade/x")));
        assert!(is_own_output(&p, &abs("assets/quillshade")));
        assert!(is_own_output(&p, Path::new("assets/quillshade/x")));
        assert!(!is_own_output(&p, &abs("assets/quillshadey/x")));
        assert!(!is_own_output(&p, &abs("assets/a.png")));
        assert!(!is_own_output(&p, &abs("quillshade/material/a.curve.ron")));
        assert!(!is_own_output(&p, Path::new("quillshade/material/a.curve.ron")));
    }
}
