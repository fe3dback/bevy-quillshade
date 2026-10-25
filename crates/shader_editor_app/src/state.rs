//! Editor state and commands. Plain Rust (wrapped in a Bevy resource): UI systems only push
//! [`Action`]s, [`Editor::apply`] mutates the model, the canvas and panels redraw from it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use shader_codegen_wesl::check::BevySources;
use shader_graph_core::ir::declared_function_sig;
use shader_graph_core::model::{
    Asset, AssetKind, BlendMode, MaterialKind, NodeId, PbrSettings, Shading, SpriteSettings, UnlitSettings,
};
use shader_graph_core::registry::{FUNCTION_PREFIX, NodeRole};
use shader_graph_core::{Code, Diagnostic, FunctionSig, PinType, Registry, Value};
use shader_project::regen::{project_registry, rust_file, wesl_file};
use shader_project::watch::ShadersWatcher;
use shader_project::{AssetId, AssetSet, AssetType, Project, RegenOptions, Scope, assets, ops};

use crate::autolayout;
use crate::config::{ProjectTabs, UserConfig};
use crate::doc::{ClipboardData, Doc, bounds, connection_error, node_signature};
use crate::search::{self, WireEnd};

/// Kinds of assets the browser can create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewKind {
    Pbr3d,
    Unlit3d,
    Sprite2d,
    Function,
}

impl NewKind {
    pub const ALL: [NewKind; 4] = [NewKind::Pbr3d, NewKind::Unlit3d, NewKind::Sprite2d, NewKind::Function];

    pub fn label(self) -> &'static str {
        match self {
            NewKind::Pbr3d => "Material (Pbr3d)",
            NewKind::Unlit3d => "Material (Unlit3d)",
            NewKind::Sprite2d => "Material (Sprite2d)",
            NewKind::Function => "Function",
        }
    }

    fn material(self) -> Option<MaterialKind> {
        match self {
            NewKind::Pbr3d => Some(MaterialKind::Pbr3d(PbrSettings::default())),
            NewKind::Unlit3d => Some(MaterialKind::Unlit3d(UnlitSettings::default())),
            NewKind::Sprite2d => Some(MaterialKind::Sprite2d(SpriteSettings::default())),
            NewKind::Function => None,
        }
    }
}

/// Value a number field shows: Feathers prints every digit it is given (a picked color gives `0.73803073`),
/// so fields show 3 decimals like their `NumberInputPrecision`. The model keeps full precision.
pub fn display_value(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

/// What a property widget edits.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum PropTarget {
    #[default]
    None,
    Param {
        node: NodeId,
        key: String,
    },
    Input {
        node: NodeId,
        pin: String,
    },
    Blend,
    TwoSided,
    Shading,
    Description,
    FrameTitle(String),
}

/// Which text a dialog's input submits to.
#[derive(Debug, Clone, PartialEq)]
pub enum Submit {
    Create(NewKind),
    Folder,
    Rename(AssetId),
    MoveTo(AssetId),
    OpenProject,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Dialog {
    Input { title: String, label: String, initial: String, submit: Submit },
    Confirm { title: String, message: String, buttons: Vec<(String, Action)> },
    Message { title: String, message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenFile {
    Wesl,
    Rust,
    Source,
}

/// Camera requests the canvas carries out once layout is known.
#[derive(Debug, Clone, PartialEq)]
pub enum ViewRequest {
    FitAll,
    FitSelection,
}

/// A wire end the palette should connect the new node to.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub node: NodeId,
    pub pin: String,
    /// The existing pin is an output (the new node gets an input connected).
    pub from_output: bool,
}

/// UI → editor command queue, drained once per frame.
#[derive(bevy::prelude::Resource, Default)]
pub struct Actions(pub Vec<Action>);

impl Actions {
    pub fn push(&mut self, a: Action) {
        self.0.push(a);
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Action {
    #[default]
    None,
    OpenProjectDialog,
    OpenProject(PathBuf),
    OpenAsset(AssetId),
    ActivateTab(usize),
    CloseTab(usize),
    DiscardTab(usize),
    SaveAndCloseTab(usize),
    NextTab,
    Save,
    SaveAll,
    RegenAll,
    Undo,
    Redo,
    Copy,
    Cut,
    Paste(Option<(f32, f32)>),
    Duplicate,
    DeleteSelection,
    Select(BTreeSet<NodeId>),
    SelectFrame(Option<String>),
    AddNode {
        kind: String,
        pos: (f32, f32),
        link: Option<Link>,
        /// Preset parameters (a palette entry of one named route).
        params: Vec<(String, Value)>,
    },
    Connect {
        from: (NodeId, String),
        to: (NodeId, String),
    },
    Disconnect {
        to: (NodeId, String),
    },
    /// Final node/frame positions after a drag (one undo step).
    Moved {
        nodes: Vec<(NodeId, (f32, f32))>,
        frames: Vec<(String, (f32, f32))>,
    },
    SetValue {
        target: PropTarget,
        value: Value,
        merge: bool,
    },
    SetComponent {
        target: PropTarget,
        comp: usize,
        value: f32,
        is_final: bool,
    },
    /// An edit made on the canvas finished (slider released, field committed, picker closed): ends undo
    /// merging and redraws Properties, which is not redrawn during merged edits.
    CommitInline,
    /// Type of a node's polymorphic inputs, stored as the type of their inline values (RMB pin menu).
    SetPinType {
        node: NodeId,
        ty: PinType,
    },
    /// Drops an inline value override; the pin falls back to its default.
    ResetInput {
        node: NodeId,
        pin: String,
    },
    AutoLayout,
    AddComment(Option<(f32, f32)>),
    AddReroute {
        from: (NodeId, String),
        to: (NodeId, String),
        pos: (f32, f32),
    },
    View(ViewRequest),
    NewAssetDialog(NewKind),
    NewFolderDialog,
    RenameDialog(AssetId),
    MoveDialog(AssetId),
    DeleteDialog(AssetId),
    DeleteFolderDialog(String),
    Submit(String),
    MoveTo(AssetId, String),
    Delete(AssetId),
    DeleteFolder(String),
    ShowFile(AssetId, GenFile),
    ToggleFolder(String),
    SelectFolder(String),
    FocusDiagnostic(Diagnostic),
    Reload(AssetId),
    KeepMine(AssetId),
    CloseDialog,
    ClearMessages,
    /// Warning for the log (e.g. a refused wire).
    Notify(String),
    Quit,
    ForceQuit,
    SaveAllAndQuit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub level: Level,
    pub text: String,
}

/// Counters the UI watches to know what to redraw.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiRev {
    pub browser: u64,
    pub tabs: u64,
    pub props: u64,
    pub log: u64,
}

#[derive(bevy::prelude::Resource)]
pub struct Editor {
    pub project: Option<Project>,
    pub registry: Registry,
    /// Assets as stored on disk (last scan).
    pub disk: AssetSet,
    pub folders: Vec<String>,
    pub docs: Vec<Doc>,
    pub active: Option<usize>,
    pub clipboard: Option<ClipboardData>,
    paste_count: u32,
    /// Declared function signatures (disk, overridden by open documents).
    pub functions: BTreeMap<String, FunctionSig>,
    /// Live diagnostics per asset (validation of the in-memory state).
    pub diags: BTreeMap<AssetId, Vec<Diagnostic>>,
    pub project_diags: Vec<Diagnostic>,
    /// E008/E010 from the last real regeneration (not reproduced by dry runs).
    regen_only: BTreeMap<AssetId, Vec<Diagnostic>>,
    pub messages: Vec<Message>,
    pub bevy: Option<BevySources>,
    validated: Option<Vec<(AssetId, u64)>>,
    pub collapsed: BTreeSet<String>,
    pub current_folder: String,
    pub dialog: Option<Dialog>,
    pub ui: UiRev,
    pub view_requests: Vec<ViewRequest>,
    watcher: Option<std::sync::Mutex<ShadersWatcher>>,
    pub config: UserConfig,
    pub config_path: Option<PathBuf>,
    pub quit: bool,
    /// Files to open in an external program.
    pub external: Vec<PathBuf>,
    pub pick_folder: bool,
}

impl Default for Editor {
    fn default() -> Self {
        Editor {
            project: None,
            registry: shader_stdlib::registry(),
            disk: AssetSet::default(),
            folders: Vec::new(),
            docs: Vec::new(),
            active: None,
            clipboard: None,
            paste_count: 0,
            functions: BTreeMap::new(),
            diags: BTreeMap::new(),
            project_diags: Vec::new(),
            regen_only: BTreeMap::new(),
            messages: Vec::new(),
            bevy: None,
            validated: None,
            collapsed: BTreeSet::new(),
            current_folder: String::new(),
            dialog: None,
            ui: UiRev::default(),
            view_requests: Vec::new(),
            watcher: None,
            config: UserConfig::default(),
            config_path: None,
            quit: false,
            external: Vec::new(),
            pick_folder: false,
        }
    }
}

fn kind_of(set: &AssetSet, id: &AssetId) -> Option<AssetKind> {
    set.assets.get(id).and_then(|a| a.as_ref().ok()).map(|a| a.kind)
}

impl Editor {
    pub fn with_config(config: UserConfig, config_path: Option<PathBuf>) -> Self {
        Editor { config, config_path, ..Default::default() }
    }

    // ---- messages / ui ----

    pub fn info(&mut self, text: impl Into<String>) {
        self.push_msg(Level::Info, text.into());
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.push_msg(Level::Error, text.into());
    }

    fn push_msg(&mut self, level: Level, text: String) {
        self.messages.push(Message { level, text });
        if self.messages.len() > 200 {
            self.messages.remove(0);
        }
        self.ui.log += 1;
    }

    fn io_error(&mut self, what: &str, e: impl std::fmt::Display) {
        let message = format!("{what}: {e}");
        self.error(message.clone());
        self.dialog = Some(Dialog::Message { title: "Error".into(), message });
    }

    fn touch_all(&mut self) {
        self.ui.browser += 1;
        self.ui.tabs += 1;
        self.ui.props += 1;
        self.ui.log += 1;
    }

    // ---- documents ----

    pub fn doc(&self) -> Option<&Doc> {
        self.active.and_then(|i| self.docs.get(i))
    }

    pub fn doc_mut(&mut self) -> Option<&mut Doc> {
        self.active.and_then(|i| self.docs.get_mut(i))
    }

    pub fn doc_index(&self, id: &AssetId) -> Option<usize> {
        self.docs.iter().position(|d| d.id == *id)
    }

    pub fn is_dirty(&self) -> bool {
        self.docs.iter().any(Doc::is_dirty)
    }

    /// Kind of an asset (open document first, then disk).
    pub fn asset_kind(&self, id: &AssetId) -> Option<AssetKind> {
        self.docs.iter().find(|d| d.id == *id).map(|d| d.asset.kind).or_else(|| kind_of(&self.disk, id))
    }

    pub fn window_title(&self) -> String {
        let mut t = String::from("Shader Editor");
        if let Some(p) = &self.project {
            t.push_str(&format!(" - {}", p.root.display()));
        }
        if let Some(d) = self.doc() {
            t.push_str(&format!(" - {}{}", d.id.source_rel(), if d.is_dirty() { " *" } else { "" }));
        }
        t
    }

    // ---- project ----

    pub fn open_project(&mut self, path: &Path) -> Result<(), String> {
        let project = Project::open(path).map_err(|e| e.to_string())?;
        if let Err(e) = project.ensure_shaders_dir() {
            return Err(format!("cannot create shaders/: {e}"));
        }
        self.save_tabs_to_config();
        let (registry, diags) = project_registry(&project);
        self.registry = registry;
        self.project_diags = diags;
        self.watcher = ShadersWatcher::new(&project).ok().map(std::sync::Mutex::new);
        self.config.touch_recent(&project.root);
        let tabs = self.config.tabs.get(&project.root).cloned().unwrap_or_default();
        self.project = Some(project);
        self.docs.clear();
        self.active = None;
        self.diags.clear();
        self.regen_only.clear();
        self.current_folder.clear();
        self.rescan();
        for rel in &tabs.open {
            if let Some(id) = assets::id_from_source_rel(rel)
                && self.disk.assets.contains_key(&id)
            {
                self.open_asset(&id);
            }
        }
        self.active = tabs.active.filter(|i| *i < self.docs.len()).or(self.active);
        let root = self.project.as_ref().map(|p| p.root.display().to_string()).unwrap_or_default();
        self.info(format!("Opened project {root}"));
        self.save_config();
        self.touch_all();
        Ok(())
    }

    pub fn save_tabs_to_config(&mut self) {
        let Some(p) = &self.project else { return };
        let open = self.docs.iter().map(|d| d.id.source_rel()).collect();
        self.config.tabs.insert(p.root.clone(), ProjectTabs { open, active: self.active });
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

    /// Reloads the asset list and declared function signatures from disk.
    pub fn rescan(&mut self) {
        let Some(project) = &self.project else { return };
        self.disk = AssetSet::load_all(project);
        self.folders = assets::scan_dirs(project);
        self.refresh_functions();
        self.ui.browser += 1;
        self.validated = None;
    }

    fn refresh_functions(&mut self) {
        let mut sigs = BTreeMap::new();
        for (id, a) in &self.disk.assets {
            if id.ty != AssetType::Function {
                continue;
            }
            let asset = self.docs.iter().find(|d| d.id == *id).map(|d| &d.asset).or(a.as_ref().ok());
            if let Some(asset) = asset {
                sigs.insert(id.path.clone(), declared_function_sig(asset, &id.path, &self.registry));
            }
        }
        self.functions = sigs;
    }

    pub fn open_asset(&mut self, id: &AssetId) {
        if let Some(i) = self.doc_index(id) {
            self.activate(i);
            return;
        }
        let Some(project) = &self.project else { return };
        match assets::load(project, id) {
            Ok(a) => {
                self.docs.push(Doc::new(id.clone(), a));
                self.activate(self.docs.len() - 1);
                self.view_requests.push(ViewRequest::FitAll);
            }
            Err(e) => self.io_error(&format!("cannot open {}", id.source_rel()), e),
        }
    }

    fn activate(&mut self, i: usize) {
        if i < self.docs.len() {
            self.active = Some(i);
            self.ui.tabs += 1;
            self.ui.props += 1;
            self.ui.log += 1;
        }
    }

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
        let Some(d) = self.docs.get(i) else { return };
        if d.is_dirty() && !force {
            self.dialog = Some(Dialog::Confirm {
                title: "Unsaved changes".into(),
                message: format!("Save changes to {} before closing?", d.id.source_rel()),
                buttons: vec![
                    ("Cancel".into(), Action::CloseDialog),
                    ("Discard".into(), Action::DiscardTab(i)),
                    ("Save".into(), Action::SaveAndCloseTab(i)),
                ],
            });
            return;
        }
        self.docs.remove(i);
        self.active = match self.active {
            _ if self.docs.is_empty() => None,
            Some(a) if a > i => Some(a - 1),
            Some(a) => Some(a.min(self.docs.len() - 1)),
            None => None,
        };
        self.refresh_functions();
        self.validated = None;
        self.touch_all();
        self.save_config();
    }

    // ---- validation / generation ----

    fn overridden_set(&self) -> AssetSet {
        let mut set = self.disk.clone();
        for d in &self.docs {
            set.assets.insert(d.id.clone(), Ok(d.asset.clone()));
        }
        set
    }

    /// Whether the in-memory state changed since the last validation.
    pub fn needs_validation(&self) -> bool {
        self.project.is_some() && self.validated.as_ref() != Some(&self.revisions())
    }

    fn revisions(&self) -> Vec<(AssetId, u64)> {
        self.docs.iter().map(|d| (d.id.clone(), d.revision)).collect()
    }

    /// Live validation (dry run of the whole pipeline on the unsaved state).
    pub fn validate(&mut self) {
        let Some(project) = self.project.clone() else { return };
        self.refresh_functions();
        let set = self.overridden_set();
        let opts = RegenOptions { dry_run: true, ..Default::default() };
        if let Ok(report) = shader_project::regen::regenerate_set(&project, &set, &opts) {
            self.project_diags = report.project;
            self.diags = report.assets.into_iter().map(|(id, r)| (id, r.diagnostics)).collect();
        }
        self.validated = Some(self.revisions());
        self.ui.log += 1;
    }

    /// Full regeneration from what is saved on disk (with WESL compile check when available).
    pub fn regenerate(&mut self) -> bool {
        let Some(project) = self.project.clone() else { return false };
        self.rescan();
        let opts = RegenOptions { scope: Scope::All, dry_run: false, compile_check: self.bevy.as_ref() };
        match shader_project::regen::regenerate_set(&project, &self.disk, &opts) {
            Ok(report) => {
                let errors = report.diagnostics().filter(|d| d.is_error()).count();
                self.regen_only = report
                    .assets
                    .iter()
                    .map(|(id, r)| {
                        let v = r
                            .diagnostics
                            .iter()
                            .filter(|d| matches!(d.code, Code::E008 | Code::E010))
                            .cloned()
                            .collect();
                        (id.clone(), v)
                    })
                    .collect();
                let n = report.changes.len();
                let msg = if n == 0 {
                    "Regenerated: no changes".to_string()
                } else {
                    format!("Regenerated: {n} file(s) written")
                };
                if errors > 0 {
                    self.push_msg(
                        Level::Warning,
                        format!("{msg}; {errors} error(s) - affected assets kept their previous output"),
                    );
                } else {
                    self.info(msg);
                }
                self.validate();
                true
            }
            Err(e) => {
                self.io_error("Regeneration failed", e);
                false
            }
        }
    }

    /// All diagnostics for display: live ones plus regeneration-only codes.
    pub fn all_diagnostics(&self) -> Vec<Diagnostic> {
        let mut out: Vec<Diagnostic> = self.project_diags.clone();
        let active = self.doc().map(|d| d.id.clone());
        let mut ids: Vec<&AssetId> = self.diags.keys().chain(self.regen_only.keys()).collect();
        ids.sort_by_key(|id| (Some(*id) != active.as_ref(), (*id).clone()));
        ids.dedup();
        for id in ids {
            out.extend(self.diags.get(id).into_iter().flatten().cloned());
            out.extend(self.regen_only.get(id).into_iter().flatten().cloned());
        }
        out
    }

    /// Diagnostics of the active document by node.
    pub fn node_diags(&self) -> BTreeMap<NodeId, Vec<&Diagnostic>> {
        let mut m: BTreeMap<NodeId, Vec<&Diagnostic>> = BTreeMap::new();
        let Some(d) = self.doc() else { return m };
        for diag in self.diags.get(&d.id).into_iter().flatten().chain(self.regen_only.get(&d.id).into_iter().flatten())
        {
            if let Some(n) = &diag.node {
                m.entry(n.clone()).or_default().push(diag);
            }
        }
        m
    }

    pub fn save(&mut self, i: usize) -> bool {
        let Some(project) = self.project.clone() else { return false };
        let Some(d) = self.docs.get(i) else { return false };
        let (id, asset) = (d.id.clone(), d.asset.clone());
        if let Err(e) = ops::save(&project, &id, &asset) {
            self.io_error(&format!("Cannot save {}", id.source_rel()), e);
            return false;
        }
        self.docs[i].mark_saved();
        self.info(format!("Saved {}", id.source_rel()));
        self.ui.tabs += 1;
        true
    }

    pub fn save_all(&mut self) -> bool {
        let mut ok = true;
        for i in 0..self.docs.len() {
            if self.docs[i].is_dirty() {
                ok &= self.save(i);
            }
        }
        ok
    }

    // ---- external changes ----

    /// Handles `notify` events: rescans and reloads or asks about open documents.
    pub fn poll_watcher(&mut self) {
        let Some(w) = &self.watcher else { return };
        let changed = w.lock().map(|w| w.poll()).unwrap_or_default();
        if changed.is_empty() || self.project.is_none() {
            return;
        }
        self.rescan();
        let user_nodes = changed.iter().any(|p| p.components().any(|c| c.as_os_str() == ".nodes"));
        if user_nodes && let Some(project) = &self.project {
            let (r, d) = project_registry(project);
            self.registry = r;
            self.project_diags = d;
            for doc in &mut self.docs {
                doc.revision += 1;
            }
        }
        for i in 0..self.docs.len() {
            let (id, dirty) = (self.docs[i].id.clone(), self.docs[i].is_dirty());
            let on_disk = match self.disk.assets.get(&id) {
                Some(Ok(a)) => a.clone(),
                Some(Err(_)) | None => continue,
            };
            let saved = Asset::from_ron(&self.docs[i].saved).ok();
            if saved.as_ref() == Some(&on_disk) {
                continue;
            }
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
            } else {
                self.docs[i].reload(on_disk);
                self.info(format!("Reloaded {} (changed on disk)", id.source_rel()));
            }
        }
        self.touch_all();
    }

    // ---- commands ----

    pub fn apply(&mut self, action: Action) {
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
                self.pick_folder = true;
            }
            Action::OpenProject(p) => {
                if self.is_dirty() {
                    self.save_all();
                }
                if let Err(e) = self.open_project(&p) {
                    self.io_error("Cannot open project", e);
                }
            }
            Action::OpenAsset(id) => self.open_asset(&id),
            Action::ActivateTab(i) => self.activate(i),
            Action::CloseTab(i) => self.close_tab(i, false),
            Action::DiscardTab(i) => {
                self.dialog = None;
                self.close_tab(i, true);
            }
            Action::SaveAndCloseTab(i) => {
                self.dialog = None;
                if self.save(i) {
                    self.regenerate();
                    self.close_tab(i, true);
                }
            }
            Action::NextTab => {
                if !self.docs.is_empty() {
                    let n = self.active.map_or(0, |a| (a + 1) % self.docs.len());
                    self.activate(n);
                }
            }
            Action::Save => {
                if let Some(i) = self.active
                    && self.save(i)
                {
                    self.regenerate();
                }
            }
            Action::SaveAll => {
                if self.save_all() {
                    self.regenerate();
                }
            }
            Action::RegenAll => {
                self.regenerate();
            }
            Action::Undo => self.history(true),
            Action::Redo => self.history(false),
            Action::Copy => self.copy(),
            Action::Cut => {
                self.copy();
                self.delete_selection();
            }
            Action::Paste(at) => self.paste(at),
            Action::Duplicate => {
                let saved = self.clipboard.take();
                self.copy();
                self.paste(None);
                self.clipboard = saved;
            }
            Action::DeleteSelection => self.delete_selection(),
            Action::Select(sel) => {
                if let Some(d) = self.doc_mut()
                    && (d.selection != sel || d.selected_frame.is_some())
                {
                    d.selection = sel;
                    d.selected_frame = None;
                    self.ui.props += 1;
                }
            }
            Action::SelectFrame(f) => {
                if let Some(d) = self.doc_mut()
                    && d.selected_frame != f
                {
                    d.selected_frame = f;
                    self.ui.props += 1;
                }
            }
            Action::AddNode { kind, pos, link, params } => self.add_node(&kind, pos, link, &params),
            Action::Connect { from, to } => self.connect(from, to),
            Action::Disconnect { to } => {
                if let Some(d) = self.doc_mut() {
                    d.edit(|a| a.disconnect_input((&to.0, &to.1)));
                }
            }
            Action::Moved { nodes, frames } => {
                if let Some(d) = self.doc_mut() {
                    d.edit(|a| {
                        for (id, p) in &nodes {
                            if let Some(n) = a.nodes.get_mut(id) {
                                n.pos = *p;
                            }
                        }
                        for (id, p) in &frames {
                            if let Some(f) = a.comments.iter_mut().find(|f| f.id == *id) {
                                f.rect.0 = p.0;
                                f.rect.1 = p.1;
                            }
                        }
                    });
                }
            }
            Action::SetValue { target, value, merge } => self.set_value(target, value, merge),
            Action::SetComponent { target, comp, value, is_final } => self.set_component(target, comp, value, is_final),
            Action::CommitInline => {
                if let Some(d) = self.doc_mut() {
                    d.end_merge();
                }
                self.ui.props += 1;
            }
            Action::SetPinType { node, ty } => self.set_pin_type(&node, ty),
            Action::ResetInput { node, pin } => {
                if let Some(d) = self.doc_mut() {
                    d.edit(|a| a.nodes.get_mut(&node).map(|n| n.inputs.remove(&pin)));
                }
                self.ui.props += 1;
            }
            Action::AutoLayout => self.auto_layout(),
            Action::AddComment(at) => {
                if let Some(d) = self.doc_mut() {
                    let sel = d.selection.clone();
                    if sel.is_empty() {
                        let (x, y) = at.unwrap_or((0.0, 0.0));
                        let id = d.add_frame("Comment", &sel);
                        d.edit(|a| {
                            if let Some(f) = a.comments.iter_mut().find(|f| f.id == id) {
                                f.rect.0 = x;
                                f.rect.1 = y;
                            }
                        });
                    } else {
                        d.add_frame("Comment", &sel);
                    }
                }
            }
            Action::AddReroute { from, to, pos } => {
                if let Some(d) = self.doc_mut() {
                    let r = shader_graph_core::model::new_node_id();
                    d.edit(|a| {
                        a.nodes.insert(r.clone(), shader_graph_core::Node::new("stdlib::Reroute", pos));
                        a.connect((&from.0, &from.1), (&r, "in"));
                        a.connect((&r, "out"), (&to.0, &to.1));
                    });
                }
            }
            Action::View(v) => self.view_requests.push(v),
            Action::NewAssetDialog(kind) => {
                let dir = self.current_folder.clone();
                let base = if kind == NewKind::Function { "new_function" } else { "new_material" };
                self.dialog = Some(Dialog::Input {
                    title: format!("New {}", kind.label()),
                    label: "Path inside shaders/ (snake_case, `/` for folders)".into(),
                    initial: if dir.is_empty() { base.into() } else { format!("{dir}/{base}") },
                    submit: Submit::Create(kind),
                });
            }
            Action::NewFolderDialog => {
                let dir = self.current_folder.clone();
                self.dialog = Some(Dialog::Input {
                    title: "New folder".into(),
                    label: "Folder path inside shaders/".into(),
                    initial: if dir.is_empty() { "new_folder".into() } else { format!("{dir}/new_folder") },
                    submit: Submit::Folder,
                });
            }
            Action::RenameDialog(id) => {
                self.dialog = Some(Dialog::Input {
                    title: format!("Rename {}", id.source_rel()),
                    label: "New path inside shaders/".into(),
                    initial: id.path.clone(),
                    submit: Submit::Rename(id),
                });
            }
            Action::MoveDialog(id) => {
                let dir = id.path.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
                self.dialog = Some(Dialog::Input {
                    title: format!("Move {}", id.source_rel()),
                    label: "Target folder inside shaders/ (empty = root)".into(),
                    initial: dir,
                    submit: Submit::MoveTo(id),
                });
            }
            Action::DeleteDialog(id) => {
                self.dialog = Some(Dialog::Confirm {
                    title: "Delete asset".into(),
                    message: format!("Delete {} and its generated files?", id.source_rel()),
                    buttons: vec![("Cancel".into(), Action::CloseDialog), ("Delete".into(), Action::Delete(id))],
                });
            }
            Action::DeleteFolderDialog(path) => {
                self.dialog = Some(Dialog::Confirm {
                    title: "Delete folder".into(),
                    message: format!("Delete empty folder {path}?"),
                    buttons: vec![
                        ("Cancel".into(), Action::CloseDialog),
                        ("Delete".into(), Action::DeleteFolder(path)),
                    ],
                });
            }
            Action::Submit(text) => self.submit(text.trim()),
            Action::MoveTo(id, dir) => {
                let name = id.name().to_string();
                let new_path = if dir.is_empty() { name } else { format!("{dir}/{name}") };
                self.rename(&id, &new_path);
            }
            Action::Delete(id) => self.delete(&id),
            Action::DeleteFolder(path) => {
                self.dialog = None;
                if let Some(p) = &self.project {
                    match ops::delete_folder(p, &path) {
                        Ok(()) => self.info(format!("Deleted folder {path}")),
                        Err(e) => self.io_error("Cannot delete folder (is it empty?)", e),
                    }
                }
                self.rescan();
            }
            Action::ShowFile(id, which) => self.show_file(&id, which),
            Action::ToggleFolder(f) => {
                if !self.collapsed.remove(&f) {
                    self.collapsed.insert(f);
                }
                self.ui.browser += 1;
            }
            Action::SelectFolder(f) => {
                self.current_folder = f;
                self.ui.browser += 1;
            }
            Action::FocusDiagnostic(diag) => self.focus_diagnostic(&diag),
            Action::Reload(id) => {
                self.dialog = None;
                if let (Some(i), Some(Ok(a))) = (self.doc_index(&id), self.disk.assets.get(&id).cloned()) {
                    self.docs[i].reload(a);
                    self.info(format!("Reloaded {}", id.source_rel()));
                }
                self.touch_all();
            }
            Action::KeepMine(id) => {
                self.dialog = None;
                if let (Some(i), Some(Ok(a))) = (self.doc_index(&id), self.disk.assets.get(&id)) {
                    // The disk version becomes the "saved" baseline, so the document stays dirty.
                    self.docs[i].saved = a.to_ron();
                }
                self.ui.tabs += 1;
            }
            Action::CloseDialog => {
                self.dialog = None;
                self.pick_folder = false;
            }
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
                self.save_config();
                self.quit = true;
            }
            Action::SaveAllAndQuit => {
                self.dialog = None;
                if self.save_all() {
                    self.regenerate();
                    self.save_config();
                    self.quit = true;
                }
            }
        }
    }

    fn history(&mut self, undo: bool) {
        if let Some(d) = self.doc_mut() {
            let done = if undo { d.undo() } else { d.redo() };
            if done {
                self.ui.props += 1;
            }
        }
    }

    fn copy(&mut self) {
        let Some(d) = self.doc() else { return };
        let clip = d.copy(&d.selection, &self.registry);
        if !clip.nodes.is_empty() {
            self.clipboard = Some(clip);
            self.paste_count = 0;
        }
    }

    fn paste(&mut self, at: Option<(f32, f32)>) {
        let Some(clip) = self.clipboard.clone() else { return };
        self.paste_count += 1;
        let step = 40.0 * self.paste_count as f32;
        let Some(d) = self.doc_mut() else { return };
        let ids: BTreeSet<NodeId> = BTreeSet::new();
        let tmp = Asset { nodes: clip.nodes.clone(), ..Asset::new("clip", AssetKind::Function) };
        let offset = match (at, bounds(&tmp, &ids)) {
            (Some(p), Some(b)) => (p.0 - b.0, p.1 - b.1),
            _ => (step, step),
        };
        d.paste(&clip, offset);
        self.ui.props += 1;
    }

    fn delete_selection(&mut self) {
        let registry = &self.registry;
        let Some(i) = self.active else { return };
        let d = &mut self.docs[i];
        if let Some(f) = d.selected_frame.take() {
            d.edit(|a| a.comments.retain(|c| c.id != f));
        } else {
            let sel = d.selection.clone();
            d.delete_nodes(&sel, registry);
        }
        self.ui.props += 1;
    }

    fn add_node(&mut self, kind: &str, pos: (f32, f32), link: Option<Link>, params: &[(String, Value)]) {
        let Some(i) = self.active else { return };
        let asset_kind = self.docs[i].asset.kind;
        let id = self.docs[i].add_node_with(kind, pos, params);
        if let Some(link) = link {
            let path = self.docs[i].id.path.clone();
            let d = &self.docs[i];
            let ty = d.asset.nodes.get(&link.node).and_then(|_| {
                let sig = node_signature(&d.asset, &link.node, &self.registry, &self.functions, &path)?;
                if link.from_output { sig.output(&link.pin).map(|p| p.ty) } else { sig.input(&link.pin).map(|p| p.ty) }
            });
            let end = ty.map(|t| if link.from_output { WireEnd::FromOutput(t) } else { WireEnd::FromInput(t) });
            if let Some(pin) =
                end.and_then(|e| search::compatible_pin(&self.registry, &self.functions, asset_kind, kind, e))
            {
                let (from, to) = if link.from_output {
                    ((link.node, link.pin), (id.clone(), pin))
                } else {
                    ((id.clone(), pin), (link.node, link.pin))
                };
                // Merge into the creation step: undo removes node and wire together.
                let d = &mut self.docs[i];
                if d.undo() {
                    d.redo();
                }
                self.connect(from, to);
            }
        }
        let d = &mut self.docs[i];
        d.selection = [id].into();
        d.selected_frame = None;
        self.ui.props += 1;
    }

    fn connect(&mut self, from: (NodeId, String), to: (NodeId, String)) {
        let Some(i) = self.active else { return };
        let d = &self.docs[i];
        if let Some(e) =
            connection_error(&d.asset, (&from.0, &from.1), (&to.0, &to.1), &self.registry, &self.functions, &d.id.path)
        {
            self.push_msg(Level::Warning, format!("Connection refused: {e}"));
            return;
        }
        self.docs[i].edit(|a| a.connect((&from.0, &from.1), (&to.0, &to.1)));
    }

    /// Value a property widget shows: the stored one, or the pin's default.
    pub fn current_value(&self, target: &PropTarget) -> Option<Value> {
        let d = self.doc()?;
        match target {
            PropTarget::Param { node, key } => {
                let n = d.asset.nodes.get(node)?;
                Some(self.registry.resolved_params(&n.kind, &n.params).get(key).cloned().unwrap_or(Value::Float(0.0)))
            }
            PropTarget::Input { node, pin } => {
                let n = d.asset.nodes.get(node)?;
                let pin_def = node_signature(&d.asset, node, &self.registry, &self.functions, &d.id.path)
                    .and_then(|sig| sig.input(pin).cloned());
                // In the pin's resolved type: a wire on another poly pin may widen a stored scalar.
                match (n.inputs.get(pin), pin_def) {
                    (Some(v), Some(p)) => Some(v.coerce(p.ty)),
                    (Some(v), None) => Some(v.clone()),
                    (None, p) => p.map(|p| p.default.coerce(p.ty)),
                }
            }
            PropTarget::Description => Some(Value::Str(d.asset.meta.description.clone())),
            PropTarget::FrameTitle(id) => {
                d.asset.comments.iter().find(|f| f.id == *id).map(|f| Value::Str(f.title.clone()))
            }
            _ => None,
        }
    }

    fn set_component(&mut self, target: PropTarget, comp: usize, value: f32, is_final: bool) {
        let Some(cur) = self.current_value(&target) else { return };
        // A field sends its shown (rounded) text back on focus loss: that is not an edit.
        let old = cur.floats().get(comp).copied();
        if old.is_some_and(|old| old != value && display_value(old) == value) {
            return;
        }
        let new = match cur {
            Value::Float(_) => Value::Float(value),
            Value::Int(_) => Value::Int(value.round() as i32),
            Value::Float2(mut v) => {
                if comp < 2 {
                    v[comp] = value;
                }
                Value::Float2(v)
            }
            Value::Float3(mut v) => {
                if comp < 3 {
                    v[comp] = value;
                }
                Value::Float3(v)
            }
            Value::Float4(mut v) => {
                if comp < 4 {
                    v[comp] = value;
                }
                Value::Float4(v)
            }
            Value::Color(mut v) => {
                if comp < 4 {
                    v[comp] = value;
                }
                Value::Color(v)
            }
            other => other,
        };
        self.set_value(target, new, true);
        if is_final && let Some(d) = self.doc_mut() {
            d.end_merge();
        }
    }

    /// Context menu of an input pin (RMB): type of polymorphic inputs while no poly pin is wired, reset of the
    /// inline value, or disconnect.
    pub fn pin_menu(&self, node: &str, pin: &str) -> Vec<(String, Action)> {
        let Some(d) = self.doc() else { return Vec::new() };
        let Some(sig) = node_signature(&d.asset, node, &self.registry, &self.functions, &d.id.path) else {
            return Vec::new();
        };
        let Some(p) = sig.input(pin) else { return Vec::new() };
        let wired = |id: &str| d.asset.edge_into(node, id).is_some();
        if wired(pin) {
            return vec![("Disconnect".into(), Action::Disconnect { to: (node.into(), pin.into()) })];
        }
        let mut items = Vec::new();
        if p.poly && !sig.inputs.iter().any(|q| q.poly && wired(&q.id)) {
            for ty in [PinType::Float, PinType::Float2, PinType::Float3, PinType::Float4] {
                let label = if ty == p.ty { format!("{} (current)", ty.label()) } else { ty.label().to_string() };
                items.push((label, Action::SetPinType { node: node.into(), ty }));
            }
        }
        if p.implicit.is_none() && !p.required {
            items.push(("Reset to default".into(), Action::ResetInput { node: node.into(), pin: pin.into() }));
        }
        items
    }

    /// Rewrites every unconnected polymorphic input of `node` to `ty`: they resolve to one common type
    /// (the widest), so changing only one pin could not narrow it.
    fn set_pin_type(&mut self, node: &str, ty: PinType) {
        let Some(d) = self.doc() else { return };
        let Some(n) = d.asset.nodes.get(node) else { return };
        let Some(sig) = node_signature(&d.asset, node, &self.registry, &self.functions, &d.id.path) else { return };
        let values: Vec<(String, Value)> = sig
            .inputs
            .iter()
            .filter(|p| p.poly && d.asset.edge_into(node, &p.id).is_none())
            .map(|p| (p.id.clone(), n.inputs.get(&p.id).unwrap_or(&p.default).coerce(ty)))
            .collect();
        if values.is_empty() {
            return;
        }
        if let Some(d) = self.doc_mut() {
            d.edit(|a| {
                if let Some(n) = a.nodes.get_mut(node) {
                    n.inputs.extend(values);
                }
            });
        }
        self.ui.props += 1;
    }

    fn set_value(&mut self, target: PropTarget, value: Value, merge: bool) {
        if self.current_value(&target).as_ref() == Some(&value) {
            return;
        }
        let key = format!("{target:?}");
        let key = merge.then_some(key.as_str());
        let Some(d) = self.doc_mut() else { return };
        let mut refresh = !merge;
        match &target {
            PropTarget::None => {}
            PropTarget::Param { node, key: k } => {
                d.edit_merge(key, |a| {
                    if let Some(n) = a.nodes.get_mut(node) {
                        n.params.insert(k.clone(), value.clone());
                    }
                });
            }
            PropTarget::Input { node, pin } => {
                d.edit_merge(key, |a| {
                    if let Some(n) = a.nodes.get_mut(node) {
                        n.inputs.insert(pin.clone(), value.clone());
                    }
                });
            }
            PropTarget::Blend => {
                let blend = match value.as_str() {
                    "Mask" => BlendMode::Mask,
                    "Blend" => BlendMode::Blend,
                    "Add" => BlendMode::Add,
                    "Multiply" => BlendMode::Multiply,
                    _ => BlendMode::Opaque,
                };
                d.edit(|a| {
                    if let AssetKind::Material(m) = &mut a.kind {
                        m.set_blend(blend);
                    }
                });
            }
            PropTarget::TwoSided => {
                d.edit(|a| match &mut a.kind {
                    AssetKind::Material(MaterialKind::Pbr3d(s)) => s.two_sided = value.as_bool(),
                    AssetKind::Material(MaterialKind::Unlit3d(s)) => s.two_sided = value.as_bool(),
                    _ => {}
                });
            }
            PropTarget::Shading => {
                let shading = if value.as_str() == "Unlit" { Shading::Unlit } else { Shading::Lit };
                d.edit(|a| {
                    if let AssetKind::Material(MaterialKind::Pbr3d(s)) = &mut a.kind {
                        s.shading = shading;
                    }
                });
            }
            PropTarget::Description => {
                d.edit_merge(key, |a| a.meta.description = value.as_str().to_string());
                refresh = false;
            }
            PropTarget::FrameTitle(id) => {
                d.edit_merge(key, |a| {
                    if let Some(f) = a.comments.iter_mut().find(|f| f.id == *id) {
                        f.title = value.as_str().to_string();
                    }
                });
                refresh = false;
            }
        }
        if refresh {
            self.ui.props += 1;
        }
    }

    /// Estimated node height for layout (title + one row per pin).
    pub fn node_height(&self, id: &str) -> f32 {
        let Some(d) = self.doc() else { return 60.0 };
        let rows = node_signature(&d.asset, id, &self.registry, &self.functions, &d.id.path)
            .map_or(1, |s| s.inputs.len() + s.outputs.len());
        40.0 + rows as f32 * 24.0
    }

    fn auto_layout(&mut self) {
        let Some(d) = self.doc() else { return };
        let pos = autolayout::layout(&d.asset, &d.selection, |n| self.node_height(n));
        if let Some(d) = self.doc_mut() {
            d.edit(|a| {
                for (id, p) in pos {
                    if let Some(n) = a.nodes.get_mut(&id) {
                        n.pos = p;
                    }
                }
            });
        }
        self.view_requests.push(ViewRequest::FitAll);
    }

    fn submit(&mut self, text: &str) {
        let Some(Dialog::Input { submit, .. }) = self.dialog.take() else { return };
        self.pick_folder = false;
        if let Submit::OpenProject = submit {
            self.apply(Action::OpenProject(PathBuf::from(text)));
            return;
        }
        let Some(project) = self.project.clone() else { return };
        let text = text.trim_matches('/');
        match submit {
            Submit::Create(kind) => {
                let r = match kind.material() {
                    Some(m) => ops::create_material(&project, text, m),
                    None => ops::create_function(&project, text),
                };
                match r {
                    Ok(id) => {
                        self.info(format!("Created {}", id.source_rel()));
                        self.rescan();
                        self.open_asset(&id);
                        self.regenerate();
                    }
                    Err(e) => self.io_error("Cannot create asset", e),
                }
            }
            Submit::Folder => match ops::create_folder(&project, text) {
                Ok(()) => {
                    self.current_folder = text.to_string();
                    self.rescan();
                }
                Err(e) => self.io_error("Cannot create folder", e),
            },
            Submit::Rename(id) => self.rename(&id, text),
            Submit::MoveTo(id) => {
                self.apply(Action::MoveTo(id, text.to_string()));
            }
            Submit::OpenProject => {}
        }
    }

    /// Rename/move; open documents follow, function references are updated, autogen regenerates.
    fn rename(&mut self, id: &AssetId, new_path: &str) {
        self.dialog = None;
        let Some(project) = self.project.clone() else { return };
        if let Some(i) = self.doc_index(id)
            && self.docs[i].is_dirty()
            && !self.save(i)
        {
            return;
        }
        let new_id = match ops::rename(&project, id, new_path) {
            Ok(n) => n,
            Err(e) => return self.io_error("Cannot rename", e),
        };
        if new_id == *id {
            return;
        }
        self.rescan();
        if let Some(i) = self.doc_index(id) {
            let d = &mut self.docs[i];
            d.id = new_id.clone();
            d.asset.name = new_id.name().to_string();
            d.mark_saved();
            d.revision += 1;
        }
        if id.ty == AssetType::Function {
            let (old_kind, new_kind) =
                (format!("{FUNCTION_PREFIX}{}", id.path), format!("{FUNCTION_PREFIX}{}", new_id.path));
            for i in 0..self.docs.len() {
                if !self.docs[i].asset.nodes.values().any(|n| n.kind == old_kind) {
                    continue;
                }
                if self.docs[i].is_dirty() {
                    self.docs[i].edit(|a| {
                        for n in a.nodes.values_mut().filter(|n| n.kind == old_kind) {
                            n.kind = new_kind.clone();
                        }
                    });
                } else if let Some(Ok(a)) = self.disk.assets.get(&self.docs[i].id).cloned() {
                    self.docs[i].reload(a);
                }
            }
        }
        self.info(format!("Renamed {} -> {}", id.source_rel(), new_id.source_rel()));
        self.regenerate();
        self.touch_all();
        self.save_config();
    }

    fn delete(&mut self, id: &AssetId) {
        self.dialog = None;
        let Some(project) = self.project.clone() else { return };
        if let Some(i) = self.doc_index(id) {
            self.close_tab(i, true);
        }
        if let Err(e) = ops::delete(&project, id) {
            return self.io_error("Cannot delete", e);
        }
        self.info(format!("Deleted {}", id.source_rel()));
        self.regenerate();
        self.touch_all();
    }

    fn show_file(&mut self, id: &AssetId, which: GenFile) {
        let Some(project) = &self.project else { return };
        let path = match which {
            GenFile::Wesl => Some(wesl_file(project, id)),
            GenFile::Rust => rust_file(project, id),
            GenFile::Source => Some(id.file(project)),
        };
        match path {
            Some(p) if p.exists() => self.external.push(p),
            Some(p) => {
                let msg = format!("{} does not exist yet (save the asset to generate it)", project.rel(&p));
                self.push_msg(Level::Warning, msg);
            }
            None => self.push_msg(Level::Warning, "functions have no Rust module".into()),
        }
    }

    fn focus_diagnostic(&mut self, diag: &Diagnostic) {
        let Some(id) = diag.asset.as_deref().and_then(assets::id_from_source_rel) else { return };
        if self.disk.assets.contains_key(&id) {
            self.open_asset(&id);
        }
        if let (Some(node), Some(d)) = (&diag.node, self.doc_mut())
            && d.id == id
            && d.asset.nodes.contains_key(node)
        {
            d.selection = [node.clone()].into();
            d.selected_frame = None;
            d.revision += 1;
            self.view_requests.retain(|v| *v != ViewRequest::FitAll);
            self.view_requests.push(ViewRequest::FitSelection);
            self.ui.props += 1;
        }
    }

    /// Display data of a node pin, used by the canvas and the properties panel.
    pub fn pin_type(&self, node: &str, pin: &str, output: bool) -> Option<PinType> {
        let d = self.doc()?;
        let sig = node_signature(&d.asset, node, &self.registry, &self.functions, &d.id.path)?;
        if output { sig.output(pin).map(|p| p.ty) } else { sig.input(pin).map(|p| p.ty) }
    }

    pub fn is_output_node(&self, node: &str) -> bool {
        self.doc()
            .and_then(|d| d.asset.nodes.get(node))
            .and_then(|n| self.registry.get(&n.kind))
            .is_some_and(|def| def.role() == NodeRole::MaterialOutput)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shader_project::fixtures;

    fn project() -> (tempfile::TempDir, Editor) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
        std::fs::create_dir_all(dir.path().join("assets")).unwrap();
        let p = Project::open(dir.path()).unwrap();
        for (path, is_fn, a) in fixtures::all() {
            let id = AssetId::new(if is_fn { AssetType::Function } else { AssetType::Material }, path);
            ops::save(&p, &id, &a).unwrap();
        }
        let mut e = Editor::default();
        e.open_project(dir.path()).unwrap();
        (dir, e)
    }

    #[test]
    fn open_edit_save_regenerates() {
        let (dir, mut e) = project();
        let id = AssetId::new(AssetType::Material, "env/rock_wet");
        e.apply(Action::OpenAsset(id.clone()));
        assert_eq!(e.docs.len(), 1);
        e.validate();
        assert!(!e.all_diagnostics().iter().any(Diagnostic::is_error), "{:?}", e.all_diagnostics());
        e.apply(Action::AddNode { kind: "stdlib::Time".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
        assert!(e.is_dirty());
        e.validate();
        assert!(e.all_diagnostics().iter().any(|d| d.code == Code::W101));
        e.apply(Action::Undo);
        assert!(!e.is_dirty());
        e.apply(Action::Redo);
        e.apply(Action::Save);
        assert!(!e.is_dirty());
        assert!(dir.path().join("assets/shaders/autogen/env/rock_wet.wesl").is_file());
        assert!(dir.path().join("crates/shaders/src/materials/env/rock_wet.rs").is_file());
    }

    #[test]
    fn add_node_from_wire_connects() {
        let (_dir, mut e) = project();
        e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
        let out = e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "output::Pbr3d").unwrap().0.clone();
        e.apply(Action::Disconnect { to: (out.clone(), "roughness".into()) });
        let link = Link { node: out.clone(), pin: "roughness".into(), from_output: false };
        e.apply(Action::AddNode {
            kind: "stdlib::Constant".into(),
            pos: (0.0, 0.0),
            link: Some(link),
            params: Vec::new(),
        });
        let d = e.doc().unwrap();
        let edge = d.asset.edge_into(&out, "roughness").expect("auto-connected");
        assert_eq!(d.asset.nodes[&edge.from.0].kind, "stdlib::Constant");
        // One undo removes node and wire.
        e.apply(Action::Undo);
        assert!(e.doc().unwrap().asset.edge_into(&out, "roughness").is_none());
    }

    fn add_lerp(e: &mut Editor) -> NodeId {
        e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
        e.apply(Action::AddNode { kind: "stdlib::Lerp".into(), pos: (-700.0, 0.0), link: None, params: Vec::new() });
        let d = e.doc().unwrap();
        d.asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Lerp").unwrap().0.clone()
    }

    #[test]
    fn inline_drag_is_one_undo_step() {
        let (_dir, mut e) = project();
        let lerp = add_lerp(&mut e);
        let alpha = PropTarget::Input { node: lerp.clone(), pin: "alpha".into() };
        let set = |value: f32| Action::SetComponent { target: alpha.clone(), comp: 0, value, is_final: false };
        let input = |e: &Editor| e.doc().unwrap().asset.nodes[&lerp].inputs.get("alpha").cloned();
        for v in [0.2, 0.4, 0.6] {
            e.apply(set(v));
        }
        let props = e.ui.props;
        e.apply(Action::CommitInline);
        assert!(e.ui.props > props, "Properties redraws with the committed value");
        // The next drag is a separate undo step.
        e.apply(set(0.9));
        e.apply(Action::Undo);
        assert_eq!(input(&e), Some(Value::Float(0.6)));
        e.apply(Action::Undo);
        assert_eq!(input(&e), None);
    }

    fn lerp_types(e: &Editor, lerp: &str) -> Vec<PinType> {
        let d = e.doc().unwrap();
        let sig = node_signature(&d.asset, lerp, &e.registry, &e.functions, &d.id.path).unwrap();
        ["a", "b", "alpha"].iter().map(|p| sig.input(p).unwrap().ty).chain([sig.outputs[0].ty]).collect()
    }

    #[test]
    fn set_pin_type_resolves_poly() {
        let (_dir, mut e) = project();
        let lerp = add_lerp(&mut e);
        e.apply(Action::SetPinType { node: lerp.clone(), ty: PinType::Float3 });
        use PinType::*;
        assert_eq!(lerp_types(&e, &lerp), vec![Float3, Float3, Float, Float3]);
        e.apply(Action::SetPinType { node: lerp.clone(), ty: Float });
        assert_eq!(lerp_types(&e, &lerp), vec![Float, Float, Float, Float]);
        e.apply(Action::Undo);
        assert_eq!(lerp_types(&e, &lerp), vec![Float3, Float3, Float, Float3]);
    }

    #[test]
    fn reset_input_removes_override() {
        let (_dir, mut e) = project();
        let lerp = add_lerp(&mut e);
        let alpha = PropTarget::Input { node: lerp.clone(), pin: "alpha".into() };
        e.apply(Action::SetComponent { target: alpha, comp: 0, value: 0.8, is_final: true });
        e.apply(Action::ResetInput { node: lerp.clone(), pin: "alpha".into() });
        let input = |e: &Editor| e.doc().unwrap().asset.nodes[&lerp].inputs.get("alpha").cloned();
        assert_eq!(input(&e), None);
        e.apply(Action::Undo);
        assert_eq!(input(&e), Some(Value::Float(0.8)));
    }

    #[test]
    fn pin_menu_offers_type_reset_and_disconnect() {
        let (_dir, mut e) = project();
        let lerp = add_lerp(&mut e);
        let labels = |e: &Editor, pin: &str| e.pin_menu(&lerp, pin).into_iter().map(|i| i.0).collect::<Vec<_>>();
        assert_eq!(labels(&e, "a"), ["Float (current)", "Float2", "Float3", "Float4", "Reset to default"]);
        // Alpha is not polymorphic.
        assert_eq!(labels(&e, "alpha"), ["Reset to default"]);
        let pick = e.pin_menu(&lerp, "a").into_iter().find(|i| i.0 == "Float3").unwrap().1;
        assert_eq!(pick, Action::SetPinType { node: lerp.clone(), ty: PinType::Float3 });

        e.apply(Action::AddNode { kind: "stdlib::Time".into(), pos: (-900.0, 0.0), link: None, params: Vec::new() });
        let d = e.doc().unwrap();
        let time = d.asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Time").unwrap().0.clone();
        e.apply(Action::Connect { from: (time, "out".into()), to: (lerp.clone(), "b".into()) });
        // A wire fixes the poly type: no type choice on any poly pin.
        assert_eq!(labels(&e, "a"), ["Reset to default"]);
        assert_eq!(labels(&e, "b"), ["Disconnect"]);
    }

    #[test]
    fn component_edit_follows_resolved_pin_type() {
        let (_dir, mut e) = project();
        let lerp = add_lerp(&mut e);
        let a = PropTarget::Input { node: lerp.clone(), pin: "a".into() };
        e.apply(Action::SetValue { target: a.clone(), value: Value::Float(0.3), merge: false });
        e.apply(Action::AddNode {
            kind: "stdlib::Constant3Vector".into(),
            pos: (-900.0, 0.0),
            link: None,
            params: Vec::new(),
        });
        let d = e.doc().unwrap();
        let c3 = d.asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Constant3Vector").unwrap().0.clone();
        e.apply(Action::Connect { from: (c3, "out".into()), to: (lerp.clone(), "b".into()) });
        // A (stored as a scalar) now resolves to Float3: editing G must keep R and B.
        e.apply(Action::SetComponent { target: a.clone(), comp: 1, value: 0.9, is_final: true });
        assert_eq!(e.doc().unwrap().asset.nodes[&lerp].inputs.get("a"), Some(&Value::Float3([0.3, 0.9, 0.3])));
    }

    #[test]
    fn committing_the_shown_rounded_value_keeps_precision() {
        let (_dir, mut e) = project();
        let lerp = add_lerp(&mut e);
        let alpha = PropTarget::Input { node: lerp.clone(), pin: "alpha".into() };
        e.apply(Action::SetValue { target: alpha.clone(), value: Value::Float(0.73803), merge: false });
        // A field shows 0.738; focus loss sends that text back.
        e.apply(Action::SetComponent { target: alpha.clone(), comp: 0, value: 0.738, is_final: true });
        assert_eq!(e.doc().unwrap().asset.nodes[&lerp].inputs.get("alpha"), Some(&Value::Float(0.73803)));
        e.apply(Action::SetComponent { target: alpha, comp: 0, value: 0.5, is_final: true });
        assert_eq!(e.doc().unwrap().asset.nodes[&lerp].inputs.get("alpha"), Some(&Value::Float(0.5)));
    }

    #[test]
    fn create_rename_delete_assets() {
        let (dir, mut e) = project();
        e.apply(Action::NewAssetDialog(NewKind::Unlit3d));
        e.apply(Action::Submit("fx/glow".into()));
        let id = AssetId::new(AssetType::Material, "fx/glow");
        assert!(e.doc_index(&id).is_some());
        assert!(dir.path().join("assets/shaders/autogen/fx/glow.wesl").is_file());
        e.apply(Action::MoveTo(id.clone(), "fx2".into()));
        let moved = AssetId::new(AssetType::Material, "fx2/glow");
        assert!(e.doc_index(&moved).is_some());
        assert!(!dir.path().join("assets/shaders/autogen/fx/glow.wesl").exists());
        assert!(dir.path().join("assets/shaders/autogen/fx2/glow.wesl").is_file());
        e.apply(Action::Delete(moved.clone()));
        assert!(e.doc_index(&moved).is_none());
        assert!(!dir.path().join("assets/shaders/autogen/fx2/glow.wesl").exists());
        // Function rename updates open callers in memory.
        let rock = AssetId::new(AssetType::Material, "env/rock_wet");
        e.apply(Action::OpenAsset(rock.clone()));
        e.apply(Action::RenameDialog(AssetId::new(AssetType::Function, "common/wet_mask")));
        e.apply(Action::Submit("common/wetness".into()));
        let d = &e.docs[e.doc_index(&rock).unwrap()];
        assert!(d.asset.nodes.values().any(|n| n.kind == "function::common/wetness"));
        assert!(!d.is_dirty());
    }

    #[test]
    fn close_dirty_tab_asks() {
        let (_dir, mut e) = project();
        e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
        e.apply(Action::AddNode { kind: "stdlib::Time".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
        e.apply(Action::CloseTab(0));
        assert!(matches!(e.dialog, Some(Dialog::Confirm { .. })));
        assert_eq!(e.docs.len(), 1);
        e.apply(Action::DiscardTab(0));
        assert!(e.docs.is_empty());
        assert!(e.active.is_none());
    }

    #[test]
    fn tabs_are_restored_on_next_launch() {
        let (dir, mut e) = project();
        let config = dir.path().join("config/config.ron");
        e.config_path = Some(config.clone());
        e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
        e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "fx/hologram")));
        e.apply(Action::ActivateTab(0));
        e.apply(Action::Quit);
        assert!(e.quit);

        let mut next = Editor::with_config(UserConfig::load(&config), Some(config));
        next.open_project(dir.path()).unwrap();
        let open: Vec<_> = next.docs.iter().map(|d| d.id.path.as_str()).collect();
        assert_eq!(open, ["env/rock_wet", "fx/hologram"]);
        assert_eq!(next.active, Some(0));
    }

    #[test]
    fn clicking_a_diagnostic_opens_its_asset() {
        let (_dir, mut e) = project();
        let id = AssetId::new(AssetType::Material, "env/rock_wet");
        let node = e.disk.assets[&id].as_ref().unwrap().nodes.keys().next().unwrap().clone();
        let diag = Diagnostic::new(Code::W101, "x").in_asset(id.source_rel()).at_node(node.clone());
        e.apply(Action::FocusDiagnostic(diag));
        assert_eq!(e.doc().map(|d| &d.id), Some(&id));
        assert!(e.doc().unwrap().selection.contains(&node));
    }

    #[test]
    fn named_routes_in_the_editor() {
        let (_dir, mut e) = project();
        e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
        e.apply(Action::AddNode { kind: "stdlib::NamedRoute".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
        let decl = e.doc().unwrap().selection.first().unwrap().clone();

        // The palette offers one usage entry per declaration, preset to it.
        let entries = search::route_entries(&e.registry, &e.doc().unwrap().asset);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Named Route Usage: route");
        let params = entries[0].params.clone();
        e.apply(Action::AddNode { kind: entries[0].kind.clone(), pos: (0.0, 0.0), link: None, params });
        let usage = e.doc().unwrap().selection.first().unwrap().clone();
        let d = e.doc().unwrap();
        assert_eq!(d.asset.nodes[&usage].params["route"], Value::Str(decl.clone()));
        assert_eq!(crate::canvas::node_title(&e, &d.asset, &usage), "route (Named Route Usage)");

        // usage → declaration would close a loop through the route.
        let err = connection_error(&d.asset, (&usage, "out"), (&decl, "in"), &e.registry, &e.functions, &d.id.path);
        assert!(err.unwrap().contains("E002"));

        // Copied with its declaration, a usage follows the copy; copied alone, it keeps the original.
        e.apply(Action::Select([decl.clone(), usage.clone()].into()));
        e.apply(Action::Duplicate);
        let d = e.doc().unwrap();
        let (new_decl, new_usage) = {
            let new: Vec<_> = d.selection.iter().cloned().collect();
            let is_decl = |id: &String| d.asset.nodes[id].kind == "stdlib::NamedRoute";
            (new.iter().find(|i| is_decl(i)).unwrap().clone(), new.iter().find(|i| !is_decl(i)).unwrap().clone())
        };
        assert_eq!(d.asset.nodes[&new_usage].params["route"], Value::Str(new_decl));
        e.apply(Action::Select([usage.clone()].into()));
        e.apply(Action::Duplicate);
        let d = e.doc().unwrap();
        let copy = d.selection.first().unwrap();
        assert_eq!(d.asset.nodes[copy].params["route"], Value::Str(decl));
    }
}
