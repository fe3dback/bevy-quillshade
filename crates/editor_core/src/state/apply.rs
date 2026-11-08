//! Core command dispatch (design §6.2): [`Editor::apply`] runs the core [`Action`]s, hands
//! [`Action::Tab`] edits to the active tab's document and [`Action::Domain`] commands to their domain's state, and
//! calls the [`DomainState`] hooks where today's save / close / rescan paths need a domain (spec §3.2.4, design
//! §6.3).
//!
//! Opening and creating assets go through the registered asset kinds (design §6.1); renaming, moving and deleting
//! them, folders, focusing a diagnostic and Generate are in `assets.rs`.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::action::{Action, Dialog, Handled, Submit, TabAction, Unsaved};
use crate::doc_kind::{StatusCtx, StatusInfo};
use crate::dynval::{PropKey, PropValue};
use crate::message::{Level, Message};
use project_core::{AssetRef, Diagnostic, DomainId, OpenError, Project};

use super::assets::UuidFix;
use super::editor::Editor;
use crate::domain::DomainHooks;
#[cfg(doc)]
use crate::domain::DomainState;
use crate::nav::NavDir;

impl Editor {
    /// Applies an action, then lets the domain it reached follow up ([`DomainState::after_apply`]): the active
    /// tab's domain for an edit of its document, the addressed domain for a domain command or a tool's argument.
    ///
    /// While the Migration Tool is open ([`Editor::migration`], ADR 0046) the rest of the editor is unavailable: only
    /// [`Action::Migration`] and quitting run, everything else (hotkeys, menus, other dialogs) is dropped. While its
    /// migration runs, quitting is deferred until the run ends ([`Editor::defer_quit`]).
    pub fn apply(&mut self, action: impl Into<Action>) {
        let action = action.into();
        if self.migration.is_some()
            && !matches!(action, Action::Migration(_) | Action::None | Action::Quit | Action::ForceQuit)
        {
            return bevy::log::debug!("{action:?} ignored: the Migration Tool is open");
        }
        if matches!(action, Action::Quit | Action::ForceQuit) && self.defer_quit() {
            return;
        }
        let reached = match &action {
            Action::Tab(_) => self.tab().map(|t| t.domain),
            Action::Domain(id, _) => Some(*id),
            // The argument is an edit of the tool's document.
            Action::OpenTool { domain, arg: Some(_), .. } => Some(*domain),
            _ => None,
        };
        self.apply_action(action);
        if std::mem::take(&mut self.settings_changed) {
            self.sync_enabled_domains();
        }
        if let Some(id) = reached {
            self.after_apply(id);
        }
        self.follow_active();
    }

    /// Records a change of the active tab, by whatever path it came, in the navigation history (decision 0041).
    pub fn follow_active(&mut self) {
        if let Some(k) = self.active_key() {
            self.nav.visit(k);
        }
    }

    // ---- domain hooks ----

    /// Runs `f` with the [`DomainHooks`] of domain `id` while its state stays inside `self.domains`, so hooks and
    /// nested `with_domain` calls can access it directly via `self.domains.get::<T>()` / `self.domains.get_mut::<T>()`.
    /// `None` (and `f` does not run) when the domain has no state. Console/logs lines written meanwhile are tagged with
    /// the domain.
    pub fn with_domain<R>(&mut self, id: DomainId, f: impl FnOnce(&mut Editor, DomainHooks) -> R) -> Option<R> {
        let hooks = self.domains.get_dyn(id)?.hooks();
        Some(self.as_domain(id, |e| f(e, hooks)))
    }

    /// Runs `f`, a callback domain `id` registered, with the console/logs lines it writes tagged with the domain
    /// (design §6.4).
    pub fn as_domain<R>(&mut self, id: DomainId, f: impl FnOnce(&mut Editor) -> R) -> R {
        let outer = self.msg_domain.replace(id);
        let out = f(self);
        self.msg_domain = outer;
        out
    }

    /// [`DomainState::after_apply`] of domain `id`.
    pub fn after_apply(&mut self, id: DomainId) {
        self.with_domain(id, |e, s| s.after_apply(e));
    }

    /// One save operation wrote the tabs `saved`: each domain with a saved tab follows once
    /// ([`DomainState::after_save`] with its own tabs; Material regenerates). `every_domain`: the domains none of whose
    /// tabs were saved follow too, with no tabs (Save All regenerates even when nothing was dirty, as before). Then the
    /// domains whose assets refer to the saved ones across a domain boundary regenerate them
    /// ([`Editor::regenerate_affected_by`]).
    pub fn after_saves(&mut self, saved: &[usize], every_domain: bool) {
        let assets: BTreeSet<AssetRef> =
            saved.iter().filter_map(|&i| self.tabs.get(i)?.doc().asset().cloned()).collect();
        let ids: Vec<DomainId> = self.domains.ids().filter(|&id| self.domain_enabled(id)).collect();
        for id in ids {
            let tabs: Vec<usize> =
                saved.iter().copied().filter(|i| self.tabs.get(*i).is_some_and(|t| t.domain == id)).collect();
            if every_domain || !tabs.is_empty() {
                self.with_domain(id, |e, s| s.after_save(e, &tabs));
            }
        }
        if !assets.is_empty() {
            self.regenerate_affected_by(&assets);
        }
    }

    /// Saves every dirty tab, then each domain follows ([`Editor::after_saves`], every domain); `false` (and no
    /// follow-up) when a save failed.
    pub fn save_all_and_follow(&mut self) -> bool {
        let (ok, saved) = self.save_dirty();
        if ok {
            self.after_saves(&saved, true);
        }
        ok
    }

    /// Tabs `pick` selects are about to lose their unsaved edits (closing or reloading): each one's domain cleans up
    /// first ([`DomainState::before_close`]). Returns the domains reached, for their `after_apply` once done.
    pub fn discarding(&mut self, pick: impl Fn(usize) -> bool) -> Vec<DomainId> {
        let mut reached = Vec::new();
        for i in (0..self.tabs.len()).filter(|i| pick(*i)) {
            let id = self.tabs[i].domain;
            self.with_domain(id, |e, s| s.before_close(e, i));
            if !reached.contains(&id) {
                reached.push(id);
            }
        }
        reached
    }

    /// [`Editor::discarding`] for every tab, then each domain's `after_apply`: the open tabs' unsaved edits are
    /// dropped (quit or switch / close the project without saving).
    pub fn discard_all(&mut self) {
        for id in self.discarding(|_| true) {
            self.after_apply(id);
        }
    }

    /// Errors of every enabled domain, the asset index's included (the mainbar's count).
    pub fn error_count(&self) -> usize {
        let states = self.domains.ids().filter(|&id| self.domain_enabled(id)).filter_map(|id| self.domains.get_dyn(id));
        let domains: usize = states.map(|s| s.diagnostics().iter().filter(|d| d.is_error()).count()).sum();
        domains + self.index_diagnostics.iter().filter(|d| d.is_error()).count()
    }

    /// The one domain the console/logs is limited to (design §6.4, §14.2): the active real domain, unless "All
    /// domains" is on or the project domain itself is active. A project tool's tab does not change it.
    pub fn console_domain(&self) -> Option<DomainId> {
        self.active_domain.filter(|d| !self.console_all && *d != DomainId::PROJECT)
    }

    /// The diagnostics as the console/logs lists them (design §6.4): domain by domain, each in its
    /// [`DomainState::console_diagnostics`] order for the active tab's asset followed by the domain's asset index
    /// diagnostics (ADR 0047 §10, the active asset's first), limited by [`Editor::console_domain`].
    pub fn console_diagnostics(&self) -> Vec<Diagnostic> {
        let active = self.tab().and_then(|t| t.doc().asset());
        let only = self.console_domain();
        let mut ids: Vec<DomainId> = self.domains.ids().collect();
        for id in self.index_diagnostics.iter().filter_map(|d| d.domain) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        let shown = ids.into_iter().filter(|&id| self.domain_enabled(id) && only.is_none_or(|o| o == id));
        shown
            .flat_map(|id| {
                let own = self.domains.get_dyn(id).map(|s| s.console_diagnostics(active)).unwrap_or_default();
                own.into_iter().chain(self.index_diagnostics_of(id, active))
            })
            .collect()
    }

    /// The console/logs lines: global ones, the project domain's (they concern every domain, design §14.2) and those
    /// of [`Editor::console_domain`] (all without a limit).
    pub fn console_messages(&self) -> impl Iterator<Item = &Message> {
        let only = self.console_domain();
        let shown = move |d: DomainId| self.domain_enabled(d) && only.is_none_or(|o| d == o || d == DomainId::PROJECT);
        self.messages.iter().filter(move |m| m.domain.is_none_or(shown))
    }

    /// The diagnostics of the active tab's asset, from its domain and then the asset index, in console/logs order
    /// (the editor statusbar).
    pub fn active_diagnostics(&self) -> Vec<Diagnostic> {
        let Some(tab) = self.tab() else { return Vec::new() };
        let Some(asset) = tab.doc().asset() else { return Vec::new() };
        let rel = asset.source_rel();
        let own = self.domains.get_dyn(tab.domain).map(|s| s.diagnostics()).unwrap_or_default();
        let index = self.index_diagnostics.iter().filter(|d| d.domain == Some(asset.domain)).cloned();
        own.into_iter().chain(index).filter(|d| d.asset.as_deref() == Some(rel.as_str())).collect()
    }

    /// The editor statusbar's content: the active tab's [`TabDoc::status`](crate::doc_kind::TabDoc::status)
    /// with its asset's diagnostics and its domain's state.
    pub fn status_info(&self) -> Option<StatusInfo> {
        let tab = self.tab()?;
        let diagnostics = self.active_diagnostics();
        let domain = self.domains.get_dyn(tab.domain).map_or(&() as &dyn std::any::Any, |s| s as &dyn std::any::Any);
        tab.doc().status(&StatusCtx {
            diagnostics: &diagnostics,
            domain,
            index: &self.asset_index,
            project: self.project.as_ref(),
        })
    }

    /// The first diagnostic of the active tab's asset, in console/logs order: what a click on the editor statusbar's
    /// problem count focuses.
    pub fn active_first_diagnostic(&self) -> Option<Diagnostic> {
        self.active_diagnostics().into_iter().next()
    }

    // ---- commands ----

    pub fn apply_action(&mut self, action: Action) {
        match action {
            Action::None => {}
            Action::OpenProjectDialog => {
                let initial = self.project.as_ref().map(|p| p.root.display().to_string()).unwrap_or_default();
                self.dialog = Some(Dialog::Input {
                    title: "Open project".into(),
                    label: "Project directory (with Cargo.toml and assets/)".into(),
                    initial,
                    submit: Submit::OpenProject,
                });
            }
            Action::ForgetRecent(p) => {
                self.config.forget_recent(&p);
                self.save_config();
            }
            Action::OpenProject(p) => self.switch_project(&p, Unsaved::Ask),
            Action::SwitchProject(p, unsaved) => {
                self.dialog = None;
                self.switch_project(&p, unsaved);
            }
            Action::NewProjectDialog => {
                self.dialog = Some(Dialog::Input {
                    title: "New project".into(),
                    label: "Game directory (a Cargo crate; assets/ and quillshade/ are created in it)".into(),
                    initial: String::new(),
                    submit: Submit::NewProject,
                });
                self.pick_folder = true;
            }
            Action::BrowseFolder => self.pick_folder = true,
            Action::FolderPicked(p) => {
                let next = match &self.dialog {
                    Some(Dialog::Input { submit: Submit::OpenProject, .. }) => Action::OpenProject(p),
                    Some(Dialog::Input { submit: Submit::NewProject, .. }) => Action::NewProject(p),
                    _ => return,
                };
                self.apply(Action::CloseDialog);
                self.apply(next);
            }
            Action::NewProject(p) => match Project::create(&p) {
                // An existing older project: opening it starts the Migration Tool (ADR 0046).
                Ok(_) | Err(OpenError::NeedsMigration { .. }) => self.apply(Action::OpenProject(p)),
                Err(e) => self.io_error("Cannot create project", e),
            },
            Action::About => self.dialog = Some(Dialog::About),
            Action::QuickStart(step) => self.dialog = Some(Dialog::QuickStart(step)),
            Action::OpenUrl(url) => self.external.push(PathBuf::from(url)),
            // Also a dialog's button (the UUID quick fixes' Open, ADR 0047 §11).
            Action::OpenAsset(r) => {
                self.dialog = None;
                self.open_ref(&r);
            }
            Action::OpenFile(path) => match self.project.as_ref() {
                Some(_) if path.exists() => self.external.push(path),
                Some(p) => {
                    let msg = format!("{} does not exist yet (save the asset to generate it)", p.rel(&path));
                    self.push_msg(Level::Warning, msg);
                }
                None => {}
            },
            Action::OpenGameImage(path) => self.open_game_file(&path),
            Action::OpenTool { domain, id, arg } => self.open_tool(domain, id, arg),
            Action::ActivateTab(i) => self.activate(i),
            Action::SwitchDomain(id) => self.switch_domain(id),
            Action::CloseTab(i) => self.close_tab(i, false),
            Action::CloseTabs { which, at, unsaved } => {
                self.dialog = None;
                self.close_tabs(which, at, unsaved);
            }
            Action::CloseProject(unsaved) => {
                self.dialog = None;
                self.close_project(unsaved);
            }
            Action::DiscardTab(i) => {
                self.dialog = None;
                self.close_tab(i, true);
            }
            Action::SaveAndCloseTab(i) => {
                self.dialog = None;
                if self.save(i) {
                    self.after_saves(&[i], false);
                    self.close_tab(i, true);
                }
            }
            Action::NextTab => {
                if !self.tabs.is_empty() {
                    let n = self.active.map_or(0, |a| (a + 1) % self.tabs.len());
                    self.activate(n);
                }
            }
            // The layout follows the flags themselves (`ui::log`, `ui::areas`, `ui::show_left_sidebar`), no revision to
            // bump. In Zen mode a panel's flag still flips; the panel shows once Zen is off (design §3).
            Action::ToggleLeftSidebar => {
                self.config.show_left_sidebar = !self.config.show_left_sidebar;
                self.save_config();
            }
            Action::ToggleLog => {
                self.config.show_log = !self.config.show_log;
                self.save_config();
            }
            Action::ToggleProperties => {
                self.config.show_properties = !self.config.show_properties;
                self.save_config();
            }
            Action::ToggleConsoleAll => {
                self.console_all = !self.console_all;
                self.ui.log += 1;
            }
            Action::ToggleConsoleInfo => {
                self.console_info = !self.console_info;
                self.ui.log += 1;
            }
            Action::ToggleZen => {
                self.config.zen_mode = !self.config.zen_mode;
                self.save_config();
            }
            Action::NavBack => self.navigate(NavDir::Back),
            Action::NavForward => self.navigate(NavDir::Forward),
            Action::Save => {
                if let Some(i) = self.active
                    && self.save(i)
                {
                    self.after_saves(&[i], false);
                }
            }
            Action::SaveAll => {
                self.save_all_and_follow();
            }
            Action::RegenAll => self.generate(),
            Action::Rescan => self.rescan(),
            Action::Tab(a) => self.apply_tab(a),
            Action::Domain(id, payload) => self.domain_step(id, &payload),
            Action::NewAssetDialog { kind, template } => self.new_asset_dialog(kind, template),
            Action::NewFolderDialog(domain) => self.new_folder_dialog(domain),
            Action::DuplicateAsset(r) => self.duplicate_asset(&r),
            Action::CopyAsset(r) => self.copy_asset(&r),
            Action::PasteAsset { domain, folder } => self.paste_asset(domain, &folder),
            Action::RenameDialog(r) => {
                self.dialog = Some(Dialog::Input {
                    title: format!("Rename {}", r.source_rel()),
                    label: format!("New path inside quillshade/{}/", r.domain),
                    initial: r.path.clone(),
                    submit: Submit::Rename(r),
                });
            }
            Action::MoveDialog(r) => {
                let dir = r.path.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
                self.dialog = Some(Dialog::Input {
                    title: format!("Move {}", r.source_rel()),
                    label: format!("Target folder inside quillshade/{}/ (empty = root)", r.domain),
                    initial: dir,
                    submit: Submit::MoveTo(r),
                });
            }
            Action::DeleteDialog(r) => self.delete_dialog(&r),
            Action::DeleteFolderDialog(domain, path) => {
                self.dialog = Some(Dialog::Confirm {
                    title: "Delete folder".into(),
                    message: format!("Delete empty folder {path}?"),
                    buttons: vec![
                        ("Cancel".into(), Action::CloseDialog),
                        ("Delete".into(), Action::DeleteFolder(domain, path)),
                    ],
                });
            }
            Action::Submit(text) => self.submit(text.trim()),
            Action::MoveTo(r, dir) => self.move_to(&r, &dir),
            Action::Delete(r) => self.delete(&r),
            Action::DeleteFolder(domain, path) => self.delete_folder(domain, &path),
            Action::ToggleFolder(tree, f) => {
                let collapsed = self.collapsed.entry(tree).or_default();
                if !collapsed.remove(&f) {
                    collapsed.insert(f);
                }
                self.ui.browser += 1;
            }
            Action::SelectFolder(domain, f) => {
                self.current_folders.insert(domain, f);
                self.ui.browser += 1;
            }
            Action::FocusDiagnostic(diag) => self.focus_diagnostic(&diag),
            Action::RegenerateUuid(r) => self.set_uuid(&r, UuidFix::Regenerate),
            Action::AssignUuid(r) => self.set_uuid(&r, UuidFix::Assign),
            Action::Reload(key) => {
                self.dialog = None;
                if let Some(i) = self.key_index(&key) {
                    let id = self.tabs[i].domain;
                    // The domain follows the new document (Material: generated files written from the dropped
                    // edits, decision 0022, regenerate from disk); a reload that fails changes nothing.
                    if self.reload_tab(i) {
                        let name = self.tab_name(i);
                        self.info(format!("Reloaded {name}"));
                        self.after_apply(id);
                    }
                }
                self.touch_all();
            }
            Action::KeepMine(key) => {
                self.dialog = None;
                // The disk version becomes the "saved" baseline, so the document stays dirty.
                if let (Some(i), Some(project)) = (self.key_index(&key), &self.project) {
                    self.tabs[i].doc_mut().keep_mine(project);
                }
                self.ui.tabs += 1;
            }
            Action::CloseDialog => {
                self.dialog = None;
                self.pick_folder = false;
            }
            Action::CopyText(s) => self.copy_text = Some(s),
            Action::Notify(text) => self.push_msg(Level::Warning, text),
            Action::ClearMessages => {
                self.messages.clear();
                self.ui.log += 1;
            }
            Action::Quit => {
                if self.is_dirty() {
                    self.dialog = Some(Dialog::Confirm {
                        title: "Unsaved changes".into(),
                        message: "Some assets have unsaved changes.".into(),
                        buttons: vec![
                            ("Cancel".into(), Action::CloseDialog),
                            ("Quit without saving".into(), Action::ForceQuit),
                            ("Save all and quit".into(), Action::SaveAllAndQuit),
                        ],
                    });
                } else {
                    self.save_config();
                    self.quit = true;
                }
            }
            Action::ForceQuit => {
                self.discard_all();
                self.save_config();
                self.quit = true;
            }
            Action::SaveAllAndQuit => {
                self.dialog = None;
                if self.save_all_and_follow() {
                    self.save_config();
                    self.quit = true;
                }
            }
            Action::Migration(a) => self.apply_migration(a),
        }
    }

    /// Dispatches an edit to the active tab's document (design §6.2), with disjoint borrows of the tab, its domain's
    /// state, the console/logs and the project. A verb the document does not support, or a payload it does not know
    /// (an edit queued for a tab that is no longer active), is dropped with a debug log line: never an error. Steps
    /// the document handed over ([`crate::doc_kind::DocCtx::defer`]) then go to its domain's
    /// [`DomainState::apply`].
    pub fn apply_tab(&mut self, action: TabAction) {
        let Some(i) = self.active else {
            return bevy::log::debug!("{action:?} ignored: no active tab");
        };
        let Some((handled, redraw, copy_text, deferred)) = self.with_doc_ctx(i, |doc, ctx| {
            let handled = doc.apply(ctx, &action);
            (handled, ctx.redraw_props, ctx.copy_text.take(), std::mem::take(&mut ctx.deferred))
        }) else {
            return;
        };
        if handled == Handled::No {
            return bevy::log::debug!("{action:?} ignored by the active tab ({})", self.tabs[i].title());
        }
        if redraw {
            self.ui.props += 1;
        }
        if copy_text.is_some() {
            self.copy_text = copy_text;
        }
        let id = self.tabs[i].domain;
        for step in &deferred {
            self.domain_step(id, step);
        }
    }

    /// A payload of domain `id` (an [`Action::Domain`] command, a deferred document step): the domain's step
    /// handlers first ([`crate::domain::DomainRegistrar::steps`], with its state in place), then its state's
    /// [`DomainState::apply`]. Nobody handling it is no error (debug log).
    pub fn domain_step(&mut self, id: DomainId, payload: &crate::dynval::Payload) {
        let handlers = self.domain_list.get(id).map(|d| d.steps.clone()).unwrap_or_default();
        for handle in handlers {
            if self.as_domain(id, |e| handle(e, payload)) == Handled::Yes {
                return;
            }
        }
        if self.with_domain(id, |e, s| s.apply(e, payload)).is_none() {
            bevy::log::debug!("{payload:?} ignored: no handler or state of domain `{id}`");
        }
    }

    /// The value a property widget editing `key` shows, from the active tab's document.
    pub fn current_value(&self, key: &PropKey) -> Option<PropValue> {
        let tab = self.tab()?;
        let domain = self.domains.get_dyn(tab.domain).map_or(&() as &dyn std::any::Any, |s| s as &dyn std::any::Any);
        tab.doc().current_value(
            key,
            &StatusCtx { diagnostics: &[], domain, index: &self.asset_index, project: self.project.as_ref() },
        )
    }
}
