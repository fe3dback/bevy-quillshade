//! Watches `shaders/` for external changes (git checkout, other editors).

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::layout::Project;

/// Keeps the watcher alive; changed paths arrive on `rx`.
pub struct ShadersWatcher {
    _watcher: RecommendedWatcher,
    pub rx: Receiver<PathBuf>,
}

impl ShadersWatcher {
    pub fn new(project: &Project) -> notify::Result<Self> {
        let (tx, rx) = channel();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res
                && (ev.kind.is_modify() || ev.kind.is_create() || ev.kind.is_remove())
            {
                for p in ev.paths {
                    let _ = tx.send(p);
                }
            }
        })?;
        let dir = project.shaders_dir();
        if dir.is_dir() {
            watcher.watch(&dir, RecursiveMode::Recursive)?;
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
