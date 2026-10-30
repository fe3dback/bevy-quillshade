//! The editor state (design §6.3): plain Rust wrapped in a Bevy resource. UI systems only push actions,
//! [`Editor::apply`] mutates the model, the canvas and panels redraw from it. Core keeps only domain-neutral fields;
//! each domain's own state lives in [`Editor::domains`].

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use crate::action::{Dialog, TreeId, ViewRequest};
use crate::clipboard::Clipboards;
use crate::config::{ProjectTabs, UserConfig};
use crate::doc_kind::{DocCtx, TabDoc};
use crate::message::{Level, Message};
use crate::tab::{Tab, TabKey};
use project_core::watch::ProjectWatcher;
use project_core::{AssetRef, DomainId, Project};

use crate::domain::{DomainList, DomainStates};
use crate::nav::NavHistory;

/// Counters the UI watches to know what to redraw.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiRev {
    pub browser: u64,
    /// The secondbar's tabs; bumped whenever a tab is opened, closed, activated or renamed (the editor statusbar keys
    /// on it).
    pub tabs: u64,
    pub props: u64,
    pub log: u64,
}

#[derive(bevy::prelude::Resource)]
pub struct Editor {
    pub project: Option<Project>,
    /// Bumped whenever a project opens or closes: domains keep per-project state keyed on it.
    pub project_generation: u64,
    /// The registered domains (design §6.1).
    pub domain_list: Arc<DomainList>,
    /// Per-domain state (design §6.3), one per registered domain with a state.
    pub domains: DomainStates,
    /// Folders of each domain's Project tree: directories under `quillshade/<domain>/` (hidden ones skipped),
    /// relative `/` paths, sorted; re-read by [`Editor::rescan`].
    pub folders: BTreeMap<DomainId, Vec<String>>,
    /// The files of each domain's registered asset kinds (the default Project tree rows, design §14.4); re-read by
    /// [`Editor::rescan`].
    pub kind_files: BTreeMap<DomainId, Vec<AssetRef>>,
    /// The game's files under `assets/` some domain shows (the Game Assets tree, design §8.1): paths relative to
    /// `assets/`, sorted; refreshed on open and by `Editor::poll_watcher`.
    pub game_files: Vec<String>,
    /// Open tabs: documents of every domain and read-only views.
    pub tabs: Vec<Tab>,
    pub active: Option<usize>,
    /// The domain whose left sidebar, Assets menu and console filter show (design §14.4): `None` without a project.
    /// A tab of a real domain makes its domain active; a project tool's tab overlays it (design §14.2).
    pub active_domain: Option<DomainId>,
    /// Each domain's last active tab ([`crate::tab::Tab::key`]), activated again when the domain is switched to.
    pub last_tabs: BTreeMap<DomainId, TabKey>,
    pub messages: Vec<Message>,
    /// The console/logs shows every domain, not only the active one (design §6.4); session only.
    pub console_all: bool,
    /// The domain whose hook is running ([`Editor::with_domain`]): the console/logs lines it writes are tagged with it.
    pub(crate) msg_domain: Option<DomainId>,
    /// Collapsed folders per sidebar tree.
    pub collapsed: BTreeMap<TreeId, BTreeSet<String>>,
    /// The selected folder of each domain's Project tree (where its "New ..." items create assets, design §6.3); a
    /// domain without one uses its root.
    pub current_folders: BTreeMap<DomainId, String>,
    pub dialog: Option<Dialog>,
    pub ui: UiRev,
    pub view_requests: Vec<ViewRequest>,
    /// Visited tabs and graph views for Back / Forward (in memory only, decision 0041).
    pub nav: NavHistory<TabKey>,
    pub(crate) watcher: Option<std::sync::Mutex<ProjectWatcher>>,
    pub config: UserConfig,
    pub config_path: Option<PathBuf>,
    pub quit: bool,
    /// Files to open in an external program.
    pub external: Vec<PathBuf>,
    /// Text for the system clipboard (Ctrl+C in a text tab).
    pub copy_text: Option<String>,
    /// What the tabs copied inside the editor (curve points, …), shared by every tab ([`DocCtx::clipboards`]).
    pub clipboards: Clipboards,
    /// The native folder picker is requested (Open / New project dialogs); the app runs it and sends
    /// [`crate::action::Action::FolderPicked`].
    pub pick_folder: bool,
    /// Set when [`DocCtx::saved_settings`] updated [`Project::settings`], so [`Editor::apply`] or
    /// [`Editor::files_changed`] synchronizes enabled domains.
    pub(crate) settings_changed: bool,
}

/// An editor without domains (core tests).
impl Default for Editor {
    fn default() -> Self {
        Editor::new(Arc::default())
    }
}

impl Editor {
    /// An editor of the registered `domains`, each with its empty state (design §6.1).
    pub fn new(domains: Arc<DomainList>) -> Editor {
        Editor {
            project: None,
            project_generation: 0,
            domains: domains.states(),
            domain_list: domains,
            folders: BTreeMap::new(),
            kind_files: BTreeMap::new(),
            game_files: Vec::new(),
            tabs: Vec::new(),
            active: None,
            active_domain: None,
            console_all: false,
            last_tabs: BTreeMap::new(),
            messages: Vec::new(),
            msg_domain: None,
            collapsed: BTreeMap::new(),
            current_folders: BTreeMap::new(),
            dialog: None,
            ui: UiRev::default(),
            view_requests: Vec::new(),
            nav: NavHistory::default(),
            watcher: None,
            config: UserConfig::default(),
            config_path: None,
            quit: false,
            external: Vec::new(),
            copy_text: None,
            clipboards: Clipboards::default(),
            pick_folder: false,
            settings_changed: false,
        }
    }

    /// Whether `domain` is enabled in the open project (always `true` for [`DomainId::PROJECT`] or without a project).
    pub fn domain_enabled(&self, domain: DomainId) -> bool {
        domain == DomainId::PROJECT || self.project.as_ref().is_none_or(|p| p.domain_enabled(domain))
    }

    /// The user config (and the file it is saved to).
    pub fn with_config(self, config: UserConfig, config_path: Option<PathBuf>) -> Self {
        Editor { config, config_path, ..self }
    }

    // ---- messages / ui ----

    pub fn info(&mut self, text: impl Into<String>) {
        self.push_msg(Level::Info, text.into());
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.push_msg(Level::Error, text.into());
    }

    /// A console/logs line: about the domain whose hook is running, global otherwise (design §6.4).
    pub fn push_msg(&mut self, level: Level, text: String) {
        self.push_message(Message { domain: self.msg_domain, level, text });
    }

    /// A console/logs line about one domain (`None`: global, design §6.4).
    pub fn domain_msg(&mut self, domain: Option<DomainId>, level: Level, text: impl Into<String>) {
        self.push_message(Message { domain, level, text: text.into() });
    }

    fn push_message(&mut self, message: Message) {
        self.messages.push(message);
        if self.messages.len() > 200 {
            self.messages.remove(0);
        }
        self.ui.log += 1;
    }

    pub fn io_error(&mut self, what: &str, e: impl std::fmt::Display) {
        let message = format!("{what}: {e}");
        self.error(message.clone());
        self.dialog = Some(Dialog::Message { title: "Error".into(), message });
    }

    pub fn touch_all(&mut self) {
        self.ui.browser += 1;
        self.ui.tabs += 1;
        self.ui.props += 1;
        self.ui.log += 1;
    }

    /// The selected folder of `domain`'s Project tree (`""`: its root).
    pub fn current_folder(&self, domain: DomainId) -> &str {
        self.current_folders.get(&domain).map_or("", String::as_str)
    }

    // ---- tabs ----

    pub fn tab(&self) -> Option<&Tab> {
        self.active.and_then(|i| self.tabs.get(i))
    }

    /// The active tab's document as `T` (`None` when another kind of tab, or none, is active).
    pub fn active_doc<T: TabDoc>(&self) -> Option<&T> {
        self.tab().and_then(Tab::get::<T>)
    }

    pub fn active_doc_mut<T: TabDoc>(&mut self) -> Option<&mut T> {
        self.active.and_then(|i| self.tabs.get_mut(i)).and_then(Tab::get_mut::<T>)
    }

    /// Tab `i`'s document as `T`.
    pub fn tab_of<T: TabDoc>(&self, i: usize) -> Option<&T> {
        self.tabs.get(i).and_then(Tab::get::<T>)
    }

    /// Tab index of an asset's editable document.
    pub fn asset_tab(&self, asset: &AssetRef) -> Option<usize> {
        self.tabs.iter().position(|t| t.doc().asset() == Some(asset))
    }

    /// How messages and dialogs name tab `i`: its asset's source path, else its title (a tool, a generated file).
    pub fn tab_name(&self, i: usize) -> String {
        let Some(doc) = self.tabs.get(i).map(Tab::doc) else { return String::new() };
        doc.asset().map_or_else(|| doc.base_title(), AssetRef::source_rel)
    }

    /// Runs `f` on tab `i`'s document with a [`DocCtx`] of its domain; the messages it leaves go to the console/logs.
    /// `None` when there is no tab `i`.
    pub fn with_doc_ctx<R>(&mut self, i: usize, f: impl FnOnce(&mut dyn TabDoc, &mut DocCtx) -> R) -> Option<R> {
        let list = self.domain_list.clone();
        let tab = self.tabs.get_mut(i)?;
        let id = tab.domain;
        let mut none = ();
        let domain = match self.domains.get_dyn_mut(tab.domain) {
            Some(state) => state as &mut dyn Any,
            None => &mut none as &mut dyn Any,
        };
        let mut messages = Vec::new();
        let mut ctx = DocCtx::new(self.project.as_ref(), domain, &mut messages);
        // Lent to the document for the call.
        ctx.clipboards = std::mem::take(&mut self.clipboards);
        ctx.crates = list.crate_contributions();
        let out = f(tab.doc_mut(), &mut ctx);
        self.clipboards = std::mem::take(&mut ctx.clipboards);
        let saved_settings = ctx.saved_settings.take();
        if let (Some(settings), Some(project)) = (saved_settings, self.project.as_mut()) {
            project.settings = settings;
            self.settings_changed = true;
        }
        for m in messages {
            self.domain_msg(m.domain.or(Some(id)), m.level, m.text);
        }
        Some(out)
    }

    pub fn is_dirty(&self) -> bool {
        self.tabs.iter().any(|t| t.doc().is_dirty())
    }

    pub fn window_title(&self) -> String {
        let mut t = String::from("Shader Editor");
        if let Some(p) = &self.project {
            t.push_str(&format!(" - {}", p.root.display()));
        }
        if let Some(tab) = self.tab() {
            t.push_str(&format!(" - {}{}", tab.title(), if tab.doc().is_dirty() { " *" } else { "" }));
        }
        t
    }

    // ---- saving ----

    /// Writes tab `i`'s document; `false` (with an error dialog) when that fails, or for a read-only tab.
    pub fn save(&mut self, i: usize) -> bool {
        if self.project.is_none() || self.tabs.get(i).is_none_or(|t| t.doc().is_read_only()) {
            return false;
        }
        let saved = self.with_doc_ctx(i, |doc, ctx| {
            let name = doc.asset().map_or_else(|| doc.base_title(), AssetRef::source_rel);
            (doc.save(ctx), name)
        });
        let Some((result, name)) = saved else { return false };
        if let Err(e) = result {
            self.io_error(&format!("Cannot save {name}"), e);
            return false;
        }
        self.info(format!("Saved {name}"));
        self.ui.tabs += 1;
        true
    }

    /// Saves every dirty tab; `false` when one of them failed. Domains do not follow: see
    /// `Editor::save_all_and_follow`.
    pub fn save_all(&mut self) -> bool {
        self.save_dirty().0
    }

    /// Saves every dirty tab: whether all saves worked, and the tabs written.
    pub fn save_dirty(&mut self) -> (bool, Vec<usize>) {
        let (mut ok, mut saved) = (true, Vec::new());
        for i in 0..self.tabs.len() {
            if self.tabs[i].doc().is_dirty() {
                if self.save(i) {
                    saved.push(i);
                } else {
                    ok = false;
                }
            }
        }
        (ok, saved)
    }

    /// Replaces tab `i`'s document with the disk version; `false` when that does not load.
    pub fn reload_tab(&mut self, i: usize) -> bool {
        if self.project.is_none() || self.tabs[i].doc().is_read_only() {
            return false;
        }
        self.with_doc_ctx(i, |doc, ctx| doc.reload_from_disk(ctx).is_ok()).unwrap_or(false)
    }

    // ---- user config ----

    pub fn save_tabs_to_config(&mut self) {
        let Some(p) = &self.project else { return };
        // A tab without a key, or a tool tab (design §14.1), is not restored; the active index counts only the
        // remembered ones.
        let keys: Vec<Option<String>> =
            self.tabs.iter().map(|t| t.key().filter(|_| t.doc().tool().is_none()).map(|k| k.to_string())).collect();
        let active = self.active.filter(|a| keys.get(*a).is_some_and(Option::is_some));
        let active = active.map(|a| keys[..a].iter().flatten().count());
        let open = keys.into_iter().flatten().collect();
        let (active_domain, last_tabs) = self.saved_domain_state();
        self.config.tabs.insert(p.root.clone(), ProjectTabs { open, active, active_domain, last_tabs });
    }

    pub fn save_config(&mut self) {
        self.save_tabs_to_config();
        if let Some(path) = &self.config_path
            && let Err(e) = self.config.save(path)
        {
            let msg = format!("cannot save user config: {e}");
            self.push_msg(Level::Warning, msg);
        }
    }
}
