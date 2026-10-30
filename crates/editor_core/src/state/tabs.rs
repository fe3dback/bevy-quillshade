//! Core halves of the project and tab paths (design §6.3): opening / closing the project, the Game Assets scan,
//! opening an asset through its kind (design §6.1) and a tool (design §14.1), restoring tabs from their `TabKey`s
//! (design §6.6, with core's default for a kind's source path, design §14.4), Back / Forward (decision 0041),
//! activating and closing tabs.

use std::path::Path;

use crate::action::{Action, CloseTabs, Dialog, Submit, TabAction, Unsaved, ViewRequest};
use crate::domain::{TOOL_KEY_PREFIX, ToolId};
use crate::dynval::Payload;
use crate::tab::{Tab, TabKey};
use project_core::{AssetRef, DomainId, Project};

use super::editor::Editor;
use super::kinds;
use crate::nav::{NavDir, NavEntry, Reach};

impl Editor {
    // ---- project ----

    pub fn open_project(&mut self, path: &Path) -> Result<(), String> {
        let project = Project::open(path).map_err(|e| e.to_string())?;
        project.ensure_layout().map_err(|e| format!("cannot create quillshade/: {e}"))?;
        for id in self.domain_list.real_ids() {
            if let Err(e) = std::fs::create_dir_all(project.domain_dir(id)) {
                return Err(format!("cannot create quillshade/{id}/: {e}"));
            }
        }
        self.save_tabs_to_config();
        self.watcher = project_core::watch::ProjectWatcher::new(&project).ok().map(std::sync::Mutex::new);
        self.config.touch_recent(&project.root);
        let tabs = self.config.tabs.get(&project.root).cloned().unwrap_or_default();
        self.project = Some(project);
        self.project_generation += 1;
        self.tabs.clear();
        self.active = None;
        self.last_tabs.clear();
        self.current_folders.clear();
        self.scan_game_files();
        // Each domain drops the previous project's state and loads this one.
        self.rescan();
        self.generate_missing_modules();
        self.nav.clear();
        // The saved active index is a position in `tabs.open`: it names the tab its own entry restores. When that
        // entry is skipped, the last restored tab stays active. Tool tabs are never restored (design §14.1).
        let list = self.domain_list.clone();
        let mut active = None;
        let restorable = |k: &TabKey| k.domain != DomainId::PROJECT && !k.key.starts_with(TOOL_KEY_PREFIX);
        for (i, entry) in tabs.open.iter().enumerate() {
            let Some(key) = list.parse_tab_key(entry).filter(|k| restorable(k) && self.key_exists(k)) else { continue };
            let before = self.tabs.len();
            self.open_key(&key);
            if tabs.active == Some(i) && self.tabs.len() > before {
                active = self.active;
            }
        }
        self.active = active.or(self.active);
        self.restore_domain(&tabs);
        self.follow_active();
        let root = self.project.as_ref().map(|p| p.root.display().to_string()).unwrap_or_default();
        self.info(format!("Opened project {root}"));
        self.save_config();
        self.touch_all();
        Ok(())
    }

    /// Every real domain with a module in the umbrella crate whose `crates/quillshade/src/<id>/mod.rs` is missing
    /// generates ([`DomainState::generate`](crate::domain::DomainState::generate)): saving another domain rewrites
    /// the umbrella's `src/lib.rs`, which names every domain's module (design §6.5), so a module never generated would
    /// leave the crate uncompilable.
    fn generate_missing_modules(&mut self) {
        let Some(project) = self.project.clone() else { return };
        let list = self.domain_list.clone();
        let missing = list.real_ids().filter(|id| {
            list.crate_contributions().iter().any(|c| c.domain == *id)
                && !project.module_dir(*id).join("mod.rs").exists()
        });
        for id in missing.collect::<Vec<_>>() {
            self.with_domain(id, |e, s| s.generate(e));
        }
    }

    /// Re-reads the project's files: each domain's Project tree folders and the files of its asset kinds
    /// ([`super::kinds::scan`]), then every domain's [`DomainState::rescan`](crate::domain::DomainState::rescan)
    /// (also without a project: the domains go back to their empty state).
    pub fn rescan(&mut self) {
        self.folders.clear();
        self.kind_files.clear();
        if let Some(p) = &self.project {
            for id in self.domain_list.real_ids() {
                self.folders.insert(id, scan_dirs(&p.domain_dir(id)));
                self.kind_files.insert(id, super::kinds::scan(p, &self.domain_list, id));
            }
        }
        let ids: Vec<DomainId> = self.domains.ids().collect();
        for id in ids {
            self.with_domain(id, |e, s| s.rescan(e));
        }
        self.ui.browser += 1;
    }

    /// Re-lists the game files the domains show (Game Assets tree); when they changed, the domains rescan (Material:
    /// W109 depends on them).
    pub fn refresh_game_files(&mut self) {
        if self.scan_game_files() {
            self.rescan();
        }
    }

    /// [`Editor::refresh_game_files`] without the rescan: whether the list changed.
    fn scan_game_files(&mut self) -> bool {
        let Some(project) = &self.project else { return false };
        let list = &self.domain_list;
        let files = project_core::game_files::scan(project, |f| list.is_game_file(f));
        if files == self.game_files {
            return false;
        }
        self.game_files = files;
        self.ui.browser += 1;
        true
    }

    // ---- tab keys (design §6.6) ----

    /// Whether what `key` names still exists: a registered tool (design §14.1), else what one of the domain's
    /// `tab_exists` says or, by core's default, an existing file of one of its asset kinds (design §14.4; an unknown
    /// domain: no).
    pub(crate) fn key_exists(&self, key: &TabKey) -> bool {
        if let Some(id) = key.key.strip_prefix(TOOL_KEY_PREFIX) {
            return self.domain_list.tool(key.domain, id).is_some();
        }
        match self.domain_list.get(key.domain) {
            Some(d) => d.tab_exists.iter().any(|exists| exists(self, &key.key)) || kinds::kind_file_exists(self, key),
            None => false,
        }
    }

    /// Opens (or activates) the tab `key` names: a tool's through [`Editor::open_tool`], any other through its
    /// domain's `parse_tab`s, else (core's default, design §14.4) a kind's source path through [`Editor::open_ref`];
    /// nothing may open (a load error is logged by the domain).
    fn open_key(&mut self, key: &TabKey) {
        if let Some(i) = self.key_index(key) {
            return self.activate(i);
        }
        if let Some(name) = key.key.strip_prefix(TOOL_KEY_PREFIX) {
            if let Some(id) = self.domain_list.tool(key.domain, name).map(|t| t.id) {
                self.open_tool(key.domain, id, None);
            }
            return;
        }
        let parsers = self.domain_list.get(key.domain).map(|d| d.parse_tabs.clone()).unwrap_or_default();
        for parse in parsers {
            if let Some(tab) = self.as_domain(key.domain, |e| parse(e, &key.key)) {
                return self.push_tab(tab);
            }
        }
        if let Some(r) = kinds::kind_ref(&self.domain_list, key.domain, &key.key) {
            self.open_ref(&r);
        }
    }

    /// Index of the open tab `key` names ([`Tab::key`]).
    pub(crate) fn key_index(&self, key: &TabKey) -> Option<usize> {
        self.tabs.iter().position(|t| t.key().as_ref() == Some(key))
    }

    /// The active tab's identity ([`Tab::key`]).
    pub fn active_key(&self) -> Option<TabKey> {
        self.tab()?.key()
    }

    /// Back / Forward (decision 0041): walks the history to the next entry that opens, dropping entries whose
    /// target is gone and skipping (keeping) ones that fail to open, then restores the entry's graph view.
    /// An open tab is never gone, even if its file vanished; a load error is only logged (no dialog), and at an end
    /// of the list nothing happens. Ignored while a dialog is open.
    pub fn navigate(&mut self, dir: NavDir) {
        if self.dialog.is_some() || self.project.is_none() {
            return;
        }
        let mut nav = std::mem::take(&mut self.nav);
        let active = self.active_key();
        let reached = nav.step(dir, active.as_ref(), |entry| {
            if let Some(i) = self.key_index(&entry.tab) {
                self.activate(i);
                return Reach::Opened;
            }
            if !self.key_exists(&entry.tab) {
                return Reach::Gone;
            }
            // A load error is also shown in a dialog (`io_error`); the walk goes on, so keep only the console line.
            let had_dialog = self.dialog.is_some();
            self.open_key(&entry.tab);
            if !had_dialog {
                self.dialog = None;
            }
            if self.active_key().as_ref() == Some(&entry.tab) { Reach::Opened } else { Reach::Failed }
        });
        self.nav = nav;
        if let Some(NavEntry { view: Some(v), .. }) = reached {
            // Replaces the FitAll a reopened graph tab queued.
            self.view_requests.clear();
            self.view_requests.push(ViewRequest::Set { centre: v.centre, zoom: v.zoom });
        }
    }

    // ---- opening ----

    /// Opens (or activates) an asset's tab through its asset kind (`Action::OpenAsset`, design §6.1).
    pub fn open_ref(&mut self, r: &AssetRef) {
        if let Some(i) = self.asset_tab(r) {
            return self.activate(i);
        }
        let Some(open) = self.domain_list.asset_kind(r.domain, r.suffix).map(|k| k.open) else {
            return bevy::log::debug!("cannot open {r}: no such asset kind");
        };
        if let Some(tab) = self.as_domain(r.domain, |e| open(e, r)) {
            self.push_tab(tab);
        }
    }

    /// Opens tool `id` of `domain` (design §14.1), or activates its open tab: a tool is unique per `(domain, id)`.
    /// Then `arg`, if any, goes to that tab's document as [`TabAction::Doc`]. An unknown tool does nothing.
    pub fn open_tool(&mut self, domain: DomainId, id: ToolId, arg: Option<Payload>) {
        let Some(open) = self.domain_list.tool(domain, id).map(|t| t.open) else {
            return bevy::log::debug!("cannot open tool `{id}` of domain `{domain}`: not registered");
        };
        match self.key_index(&TabKey::tool(domain, id)) {
            Some(i) => self.activate(i),
            None => {
                let doc = self.as_domain(domain, open);
                self.push_tab(Tab { domain, doc });
            }
        }
        if let Some(arg) = arg {
            self.apply_tab(TabAction::Doc(arg));
        }
    }

    /// A double click on a game file (`Action::OpenGameImage`): the active domain opens it if it shows the file
    /// (design §14.4).
    pub fn open_game_file(&mut self, path: &str) {
        let Some(id) = self.active_domain else { return };
        let Some(open) = self.domain_list.game_assets_of(id).filter(|g| g.accepts(path)).map(|g| g.open) else {
            return;
        };
        self.as_domain(id, |e| open(e, path));
    }

    /// Appends a tab and activates it.
    pub fn push_tab(&mut self, tab: Tab) {
        self.tabs.push(tab);
        self.activate(self.tabs.len() - 1);
    }

    /// Re-reads the files of read-only tabs (after regeneration; each one also on activation).
    pub fn reload_read_only_tabs(&mut self) {
        if self.project.is_none() {
            return;
        }
        let (mut changed, mut active_changed) = (false, false);
        let read_only: Vec<usize> = (0..self.tabs.len()).filter(|i| self.tabs[*i].doc().is_read_only()).collect();
        for i in read_only {
            let revision = self.tabs[i].doc().revision();
            // A missing file is shown as such; nothing to report here.
            self.with_doc_ctx(i, |t, ctx| drop(t.reload_from_disk(ctx)));
            let moved = self.tabs[i].doc().revision() != revision;
            changed |= moved;
            active_changed |= moved && self.active == Some(i);
        }
        if changed {
            self.ui.tabs += 1;
        }
        // The right sidebar of the active tab shows the file (a text tab's line count).
        if active_changed {
            self.ui.props += 1;
        }
    }

    pub fn activate(&mut self, i: usize) {
        if i < self.tabs.len() {
            if self.project.is_some() && self.tabs[i].doc().is_read_only() {
                self.with_doc_ctx(i, |t, ctx| drop(t.reload_from_disk(ctx)));
            }
            self.active = Some(i);
            self.follow_tab_domain(i);
            self.ui.tabs += 1;
            self.ui.props += 1;
            self.ui.log += 1;
        }
    }

    // ---- new assets (design §6.1) ----

    /// `Action::NewAssetDialog`: the path of a new asset from template `template` of kind `kind`, proposed in the
    /// selected folder.
    pub fn new_asset_dialog(&mut self, kind: (DomainId, &'static str), template: usize) {
        let Some(t) = self.domain_list.asset_kind(kind.0, kind.1).and_then(|k| k.templates.get(template)) else {
            return;
        };
        let (dir, base) = (self.current_folder(kind.0), t.base_name);
        self.dialog = Some(Dialog::Input {
            title: format!("New {}", t.label),
            label: format!("Path inside quillshade/{}/ (snake_case, `/` for folders)", kind.0),
            initial: if dir.is_empty() { base.into() } else { format!("{dir}/{base}") },
            submit: Submit::Create { kind, template },
        });
    }

    /// The path dialog of `Action::NewAssetDialog` answered: the template writes the file, the domains rescan, the
    /// asset opens and its domain follows as after a save (Material regenerates).
    pub fn create_asset(&mut self, kind: (DomainId, &'static str), template: usize, path: &str) {
        let Some(project) = self.project.clone() else { return };
        let Some(t) = self.domain_list.asset_kind(kind.0, kind.1).and_then(|k| k.templates.get(template)) else {
            return;
        };
        let r = AssetRef { domain: kind.0, path: path.to_string(), suffix: kind.1 };
        let create = t.create;
        // The template's result is the domain's line of the console/logs.
        let created = self.as_domain(kind.0, |e| match create(&project, &r) {
            Ok(()) => {
                e.info(format!("Created {}", r.source_rel()));
                true
            }
            Err(err) => {
                e.io_error("Cannot create asset", err);
                false
            }
        });
        if created {
            self.rescan();
            self.open_ref(&r);
            self.with_domain(kind.0, |e, s| s.after_save(e, &[]));
        }
    }

    // ---- closing ----

    /// Closes a tab; asks first when it has unsaved changes.
    pub fn close_tab(&mut self, i: usize, force: bool) {
        // `usize::MAX` = the active tab (Ctrl+W).
        let i = if i == usize::MAX {
            match self.active {
                Some(a) => a,
                None => return,
            }
        } else {
            i
        };
        let Some(t) = self.tabs.get(i) else { return };
        if t.doc().is_dirty() && !force {
            let name = t.doc().asset().map_or_else(|| t.doc().base_title(), AssetRef::source_rel);
            self.dialog = Some(Dialog::Confirm {
                title: "Unsaved changes".into(),
                message: format!("Save changes to {name} before closing?"),
                buttons: vec![
                    ("Cancel".into(), Action::CloseDialog),
                    ("Discard".into(), Action::DiscardTab(i)),
                    ("Save".into(), Action::SaveAndCloseTab(i)),
                ],
            });
            return;
        }
        let close: Vec<bool> = (0..self.tabs.len()).map(|j| j == i).collect();
        self.remove_tabs(&close);
    }

    /// Closes the tabs `which` names relative to tab `at` (tab context menu).
    pub fn close_tabs(&mut self, which: CloseTabs, at: usize, unsaved: Unsaved) {
        if at >= self.tabs.len() {
            return;
        }
        let close: Vec<bool> = (0..self.tabs.len())
            .map(|i| match which {
                CloseTabs::All => true,
                CloseTabs::Others => i != at,
                CloseTabs::Left => i < at,
                CloseTabs::Right => i > at,
                CloseTabs::Unmodified => !self.tabs[i].doc().is_dirty(),
            })
            .collect();
        let dirty: Vec<usize> = (0..self.tabs.len()).filter(|i| close[*i] && self.tabs[*i].doc().is_dirty()).collect();
        if !dirty.is_empty() {
            match unsaved {
                Unsaved::Ask => {
                    let action = |unsaved| Action::CloseTabs { which, at, unsaved };
                    self.dialog = Some(Dialog::Confirm {
                        title: "Unsaved changes".into(),
                        message: format!("{} of the tabs being closed have unsaved changes.", dirty.len()),
                        buttons: vec![
                            ("Cancel".into(), Action::CloseDialog),
                            ("Discard".into(), action(Unsaved::Discard)),
                            ("Save".into(), action(Unsaved::Save)),
                        ],
                    });
                    return;
                }
                Unsaved::Discard => {}
                Unsaved::Save => {
                    let mut saved = true;
                    for i in &dirty {
                        saved &= self.save(*i);
                    }
                    if !saved {
                        return;
                    }
                    self.after_saves(&dirty, false);
                }
            }
        }
        self.remove_tabs(&close);
    }

    /// Removes the tabs marked in `close`. The active tab stays active; when it goes, the nearest staying tab of the
    /// active domain or a project tool takes over ([`Editor::close_successor`]), else none.
    pub fn remove_tabs(&mut self, close: &[bool]) {
        if !close.contains(&true) {
            return;
        }
        // Each closing tab's domain cleans up first (Material: previews written from unsaved edits, decision 0022).
        let reached = self.discarding(|i| close[i]);
        let kept_before = |i: usize| (0..i).filter(|j| !close[*j]).count();
        let (active, successor) = match self.active {
            Some(a) if !close[a] => (Some(kept_before(a)), None),
            Some(a) => (None, self.close_successor(a, close)),
            None => (None, None),
        };
        self.active = active;
        let mut i = 0;
        self.tabs.retain(|_| {
            i += 1;
            !close[i - 1]
        });
        if let Some(s) = successor {
            self.activate(s);
        }
        for id in reached {
            self.after_apply(id);
        }
        self.touch_all();
        self.save_config();
    }

    /// Opens the project at `path` in place of the open one. With unsaved tabs `Ask` asks Cancel / Discard / Save
    /// first, as closing tabs does; a path that is no project shows its error without asking. The Open project
    /// dialog it came from (a recent project, the field) has done its job and closes.
    pub fn switch_project(&mut self, path: &Path, unsaved: Unsaved) {
        if let Some(Dialog::Input { submit: Submit::OpenProject, .. }) = self.dialog {
            self.dialog = None;
        }
        if let Err(e) = Project::open(path) {
            self.io_error("Cannot open project", e.to_string());
            return;
        }
        if self.is_dirty() {
            match unsaved {
                Unsaved::Ask => {
                    let action = |unsaved| Action::SwitchProject(path.to_path_buf(), unsaved);
                    self.dialog = Some(Dialog::Confirm {
                        title: "Unsaved changes".into(),
                        message: "Some tabs have unsaved changes. Save them before opening the project?".into(),
                        buttons: vec![
                            ("Cancel".into(), Action::CloseDialog),
                            ("Discard".into(), action(Unsaved::Discard)),
                            ("Save".into(), action(Unsaved::Save)),
                        ],
                    });
                    return;
                }
                Unsaved::Discard => self.discard_all(),
                Unsaved::Save => {
                    if !self.save_all_and_follow() {
                        return;
                    }
                }
            }
        }
        if let Err(e) = self.open_project(path) {
            self.io_error("Cannot open project", e);
        }
    }

    /// Closes the project (its open tabs are remembered for the next time it opens). Asks first when tabs have
    /// unsaved changes.
    pub fn close_project(&mut self, unsaved: Unsaved) {
        if self.project.is_none() {
            return;
        }
        if self.is_dirty() {
            match unsaved {
                Unsaved::Ask => {
                    self.dialog = Some(Dialog::Confirm {
                        title: "Unsaved changes".into(),
                        message: "Some assets have unsaved changes.".into(),
                        buttons: vec![
                            ("Cancel".into(), Action::CloseDialog),
                            ("Close without saving".into(), Action::CloseProject(Unsaved::Discard)),
                            ("Save all and close".into(), Action::CloseProject(Unsaved::Save)),
                        ],
                    });
                    return;
                }
                Unsaved::Discard => self.discard_all(),
                Unsaved::Save => {
                    if !self.save_all_and_follow() {
                        return;
                    }
                }
            }
        }
        self.save_config();
        let root = self.project.take().map(|p| p.root.display().to_string()).unwrap_or_default();
        self.project_generation += 1;
        self.watcher = None;
        self.game_files.clear();
        self.tabs.clear();
        self.active = None;
        self.active_domain = None;
        self.last_tabs.clear();
        self.nav.clear();
        self.current_folders.clear();
        // Folders cleared, every domain back to its empty state.
        self.rescan();
        self.info(format!("Closed project {root}"));
        self.touch_all();
    }
}

/// Directories under `root` (hidden ones skipped and not entered), as sorted relative `/` paths.
fn scan_dirs(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && !e.file_name().to_string_lossy().starts_with('.') {
                if let Ok(rel) = p.strip_prefix(root) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
                stack.push(p);
            }
        }
    }
    out.sort();
    out
}
