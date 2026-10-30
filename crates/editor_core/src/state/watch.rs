//! External changes (spec §9 "State"): the project watcher's events. A change under `assets/` re-lists the game
//! files; one under a domain's `quillshade/<domain>/` rescans, lets the domains react
//! ([`DomainState::files_changed`](crate::domain::DomainState::files_changed)) and reloads or asks about open
//! documents; one of `quillshade/project.ron` re-reads the project settings and reloads or asks about the Project
//! Settings tab (design §14.2).

use std::path::PathBuf;

use crate::action::{Action, Dialog};
use crate::message::Level;
use project_core::DomainId;

use super::editor::Editor;

impl Editor {
    /// Handles the watcher's `notify` events.
    pub fn poll_watcher(&mut self) {
        let Some(w) = &self.watcher else { return };
        let changed = w.lock().map(|w| w.poll()).unwrap_or_default();
        self.files_changed(changed);
    }

    /// Reacts to external changes of `changed` (absolute paths, as the watcher reports them).
    pub fn files_changed(&mut self, changed: Vec<PathBuf>) {
        let Some(project) = &mut self.project else { return };
        let assets_dir = project.assets_dir();
        let domain_dirs: Vec<PathBuf> = self.domain_list.real_ids().map(|id| project.domain_dir(id)).collect();
        let settings_path = project.settings_path();
        // Saves rewrite the umbrella crate from these settings (design §4.1), so they must follow the file; no regen.
        let settings_on_disk = changed.contains(&settings_path);
        let mut settings_reloaded = false;
        if settings_on_disk {
            match project.reload_settings() {
                Ok(()) => settings_reloaded = true,
                Err(e) => self.push_msg(Level::Warning, format!("{e}; keeping the previous settings")),
            }
        }
        let (game, changed): (Vec<_>, Vec<_>) = changed.into_iter().partition(|p| p.starts_with(&assets_dir));
        let changed: Vec<_> = changed.into_iter().filter(|p| domain_dirs.iter().any(|d| p.starts_with(d))).collect();
        if !game.is_empty() {
            self.refresh_game_files();
            // Image tabs show the new file.
            self.reload_read_only_tabs();
        }
        if changed.is_empty() && !settings_on_disk {
            return;
        }
        if !changed.is_empty() {
            self.rescan();
            let ids: Vec<DomainId> = self.domains.ids().filter(|&id| self.domain_enabled(id)).collect();
            for id in ids {
                self.with_domain(id, |e, s| s.files_changed(e, &changed));
            }
        }
        self.follow_disk_changes();
        if std::mem::take(&mut self.settings_changed) || settings_reloaded {
            self.sync_enabled_domains();
        }
        self.touch_all();
    }

    /// Every tab whose file differs from what it last saved or loaded ([`TabDoc::changed_on_disk`]: an asset's, the
    /// Project Settings' `project.ron`, design §14.2) reloads when clean; a dirty one asks Keep mine / Reload (one
    /// dialog at a time).
    ///
    /// [`TabDoc::changed_on_disk`]: crate::doc_kind::TabDoc::changed_on_disk
    fn follow_disk_changes(&mut self) {
        for i in 0..self.tabs.len() {
            let Some(project) = &self.project else { return };
            let tab = &self.tabs[i];
            let Some(key) = tab.key().filter(|_| tab.doc().changed_on_disk(project)) else { continue };
            let name = self.tab_name(i);
            if self.tabs[i].doc().is_dirty() {
                if self.dialog.is_none() {
                    self.dialog = Some(Dialog::Confirm {
                        title: "File changed on disk".into(),
                        message: format!("{name} was changed outside the editor. Reload it and lose your changes?"),
                        buttons: vec![
                            ("Keep mine".into(), Action::KeepMine(key.clone())),
                            ("Reload".into(), Action::Reload(key)),
                        ],
                    });
                }
            } else if self.reload_tab(i) {
                self.info(format!("Reloaded {name} (changed on disk)"));
            }
        }
    }
}
