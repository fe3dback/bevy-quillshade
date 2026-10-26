use super::*;

impl Editor {
    // ---- external changes ----

    /// Handles `notify` events: a change under `assets/` re-lists the game's images; one under `shaders/` rescans
    /// and reloads or asks about open documents.
    pub fn poll_watcher(&mut self) {
        let Some(w) = &self.watcher else { return };
        let changed = w.lock().map(|w| w.poll()).unwrap_or_default();
        let Some(project) = &self.project else { return };
        let assets_dir = project.assets_dir();
        let (game, changed): (Vec<_>, Vec<_>) = changed.into_iter().partition(|p| p.starts_with(&assets_dir));
        if !game.is_empty() {
            self.refresh_game_files();
            // Image tabs show the new file.
            self.reload_read_only_tabs();
        }
        if changed.is_empty() {
            return;
        }
        self.rescan();
        let user_nodes = changed.iter().any(|p| p.components().any(|c| c.as_os_str() == ".nodes"));
        if user_nodes && let Some(project) = &self.project {
            let (r, d) = project_registry(project);
            self.registry = r;
            self.project_diags = d;
            for doc in self.tabs.iter_mut().filter_map(Tab::graph_mut) {
                doc.revision += 1;
            }
        }
        for i in 0..self.tabs.len() {
            let Some(project) = &self.project else { return };
            let doc = self.tabs[i].doc();
            // Whether the file differs from what the tab last saved/loaded.
            let Some(id) = doc.asset().filter(|_| doc.changed_on_disk(project)).cloned() else { continue };
            let dirty = doc.is_dirty();
            if dirty {
                if self.dialog.is_none() {
                    self.dialog = Some(Dialog::Confirm {
                        title: "File changed on disk".into(),
                        message: format!(
                            "{} was changed outside the editor. Reload it and lose your changes?",
                            id.source_rel()
                        ),
                        buttons: vec![
                            ("Keep mine".into(), Action::KeepMine(id.clone())),
                            ("Reload".into(), Action::Reload(id.clone())),
                        ],
                    });
                }
            } else if self.reload_tab(i) {
                self.info(format!("Reloaded {} (changed on disk)", id.source_rel()));
            }
        }
        self.touch_all();
    }

    /// Replaces tab `i`'s document with the disk version; `false` when that does not load.
    pub(super) fn reload_tab(&mut self, i: usize) -> bool {
        let Some(project) = &self.project else { return false };
        let doc = self.tabs[i].doc_mut();
        !doc.is_read_only() && doc.reload_from_disk(project).is_ok()
    }
}
