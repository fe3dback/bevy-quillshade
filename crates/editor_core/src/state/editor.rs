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
use project_core::{AssetIndex, AssetRef, Diagnostic, DomainId, Project};

use crate::domain::{DomainList, DomainStates};
use crate::nav::NavHistory;
use crate::state::migration::MigrationSession;

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
    /// Every editor-owned asset of the project by UUID (ADR 0047 §10, techdoc §4.2): rebuilt from every compiled-in
    /// backend at the start of [`Editor::rescan`] (so the domains' rescans resolve against it) and refreshed for the
    /// saved tab's domain after every save ([`Editor::refresh_index_domain`]); empty without a project.
    pub asset_index: AssetIndex,
    /// The index's diagnostics (`AE001`–`AE004`, ADR 0047 §8) of the enabled domains, kept with
    /// [`Editor::asset_index`]: merged into the console/logs, the editor statusbar and the mainbar's error count.
    pub index_diagnostics: Vec<Diagnostic>,
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
    /// The console/logs also lists info diagnostics (`I…` codes, spec §13); session only.
    pub console_info: bool,
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
    /// The Migration Tool (ADR 0046): set instead of [`Editor::project`] when the project to open is older than this
    /// editor. While it is `Some`, [`Editor::apply`] ignores every action but the Migration Tool's and quitting.
    pub migration: Option<MigrationSession>,
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
            asset_index: AssetIndex::default(),
            index_diagnostics: Vec::new(),
            game_files: Vec::new(),
            tabs: Vec::new(),
            active: None,
            active_domain: None,
            console_all: false,
            console_info: false,
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
            migration: None,
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
        ctx.index = &self.asset_index;
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

    /// Writes tab `i`'s document; `false` (with an error dialog) when that fails, or for a read-only tab. The index
    /// part of the tab's domain is re-read after a successful save.
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
        self.refresh_index_domain(self.tabs[i].domain);
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

    // ---- asset index (ADR 0047 §10) ----

    /// Rebuilds [`Editor::asset_index`] from every compiled-in backend's part (enabled or not, so a reference into a
    /// disabled domain is detectable, ADR 0047 §9), and its diagnostics; an empty index without a project.
    pub fn refresh_index(&mut self) {
        let list = self.domain_list.clone();
        self.asset_index = match &self.project {
            Some(p) => AssetIndex::build(list.backends().iter().map(|b| b.index(p)).collect()),
            None => AssetIndex::default(),
        };
        self.refresh_index_diagnostics();
    }

    /// Re-reads `domain`'s part of [`Editor::asset_index`] (after a save), keeping every other domain's; nothing for
    /// a domain without a backend.
    pub fn refresh_index_domain(&mut self, domain: DomainId) {
        let Some(p) = &self.project else { return };
        let Some(backend) = self.domain_list.backends().iter().find(|b| b.id() == domain) else { return };
        let part = backend.index(p);
        self.asset_index.replace_domain(part);
        self.refresh_index_diagnostics();
    }

    /// Cross-domain follow-up of one save operation that wrote the assets `saved` (ADR 0047 §10, techdoc §4.3): every
    /// enabled domain with assets in [`AssetIndex::affected`] of `saved` gets [`DomainState::regenerate_affected`]
    /// with them, in domain order. What a saving domain reaches from its own saved assets without leaving the domain
    /// is not handed over (its [`DomainState::after_save`] regenerates that), nor are the saved assets themselves, so
    /// a domain hears only of references that cross a domain boundary somewhere on the way.
    ///
    /// [`DomainState::regenerate_affected`]: crate::domain::DomainState::regenerate_affected
    /// [`DomainState::after_save`]: crate::domain::DomainState::after_save
    pub fn regenerate_affected_by(&mut self, saved: &BTreeSet<AssetRef>) {
        let mut own: BTreeSet<AssetRef> = BTreeSet::new();
        for domain in saved.iter().map(|r| r.domain).collect::<BTreeSet<_>>() {
            let mine = saved.iter().filter(|r| r.domain == domain).cloned().collect();
            own.extend(affected_within(&self.asset_index, mine, domain));
        }
        let mut by_domain: BTreeMap<DomainId, Vec<AssetRef>> = BTreeMap::new();
        for r in self.asset_index.affected(saved).into_iter().filter(|r| !own.contains(r)) {
            by_domain.entry(r.domain).or_default().push(r);
        }
        for (domain, affected) in by_domain {
            if self.domain_enabled(domain) {
                self.with_domain(domain, |e, s| s.regenerate_affected(e, &affected));
            }
        }
    }

    /// [`Editor::index_diagnostics`] from the current index; the console/logs redraws when they changed.
    fn refresh_index_diagnostics(&mut self) {
        let diagnostics = self.project.as_ref().map(|p| self.asset_index.diagnostics(p)).unwrap_or_default();
        if diagnostics != self.index_diagnostics {
            self.index_diagnostics = diagnostics;
            self.ui.log += 1;
        }
    }

    /// The index diagnostics of `domain`, those of the asset `active` first (console/logs order).
    pub fn index_diagnostics_of(&self, domain: DomainId, active: Option<&AssetRef>) -> Vec<Diagnostic> {
        let mut out: Vec<Diagnostic> =
            self.index_diagnostics.iter().filter(|d| d.domain == Some(domain)).cloned().collect();
        if let Some(active) = active {
            let rel = active.source_rel();
            out.sort_by_key(|d| d.asset.as_deref() != Some(rel.as_str()));
        }
        out
    }

    /// The file whose UUID `r` duplicates (the winner, ADR 0047 §8) when `r` is a duplicate loser.
    pub fn duplicate_winner(&self, r: &AssetRef) -> Option<&AssetRef> {
        self.asset_index.duplicates().find_map(|(_, files)| files[1..].contains(r).then(|| &files[0]))
    }

    /// Whether `r` is a current-format file without a valid UUIDv7 `id` (`AE004`).
    pub fn lacks_uuid(&self, r: &AssetRef) -> bool {
        self.asset_index.missing_ids().any(|(m, _)| m == r)
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

/// `changed` plus every asset of `domain` that (transitively) refers to it through `domain`'s assets only: what
/// [`AssetIndex::affected`] gives when the walk never leaves the domain.
fn affected_within(index: &AssetIndex, changed: BTreeSet<AssetRef>, domain: DomainId) -> BTreeSet<AssetRef> {
    let mut todo: Vec<AssetRef> = changed.iter().cloned().collect();
    let mut out = changed;
    while let Some(r) = todo.pop() {
        // Only the winner of a duplicate UUID is referred to (ADR 0047 §8).
        let Some(id) = index.ref_id_of(&r) else { continue };
        for referrer in index.refs_to(id).filter(|x| x.domain == domain) {
            if out.insert(referrer.clone()) {
                todo.push(referrer.clone());
            }
        }
    }
    out
}
