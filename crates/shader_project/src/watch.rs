//! Watches `shaders/` for external changes (git checkout, other editors) and the game's `assets/` for images coming
//! and going (the Game Assets tree, design §4.2). Events under `assets/shaders/` (our own regeneration output) are
//! dropped, so writing it never triggers a refresh.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::layout::Project;

/// Keeps the watcher alive; changed paths (under `shaders/` or `assets/`) arrive on `rx`.
pub struct ShadersWatcher {
    _watcher: RecommendedWatcher,
    pub rx: Receiver<PathBuf>,
}

impl ShadersWatcher {
    pub fn new(project: &Project) -> notify::Result<Self> {
        let (tx, rx) = channel();
        let own = project.assets_dir().join(crate::game_assets::OWN_DIR);
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res
                && (ev.kind.is_modify() || ev.kind.is_create() || ev.kind.is_remove())
            {
                for p in ev.paths.into_iter().filter(|p| !p.starts_with(&own)) {
                    let _ = tx.send(p);
                }
            }
        })?;
        let shaders = project.shaders_dir();
        if shaders.is_dir() {
            watcher.watch(&shaders, RecursiveMode::Recursive)?;
        }
        // Best effort: a huge `assets/` may exceed the OS watch limit; `shaders/` is still watched then.
        let assets = project.assets_dir();
        if assets.is_dir() {
            let _ = watcher.watch(&assets, RecursiveMode::Recursive);
        }
        Ok(ShadersWatcher { _watcher: watcher, rx })
    }

    /// Drains pending events.
    pub fn poll(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.rx.try_iter().collect();
        v.sort();
        v.dedup();
        v
    }
}
