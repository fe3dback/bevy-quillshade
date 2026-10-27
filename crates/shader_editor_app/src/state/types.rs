use super::*;

/// Kinds of assets the browser can create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewKind {
    Pbr3d,
    Unlit3d,
    Sprite2d,
    Function,
    Curve,
    Consts,
}

impl NewKind {
    pub const ALL: [NewKind; 6] =
        [NewKind::Pbr3d, NewKind::Unlit3d, NewKind::Sprite2d, NewKind::Function, NewKind::Curve, NewKind::Consts];

    pub fn label(self) -> &'static str {
        match self {
            NewKind::Pbr3d => "Material (Pbr3d)",
            NewKind::Unlit3d => "Material (Unlit3d)",
            NewKind::Sprite2d => "Material (Sprite2d)",
            NewKind::Function => "Function",
            NewKind::Curve => "Curve",
            NewKind::Consts => "Global Consts",
        }
    }

    pub(super) fn material(self) -> Option<MaterialKind> {
        match self {
            NewKind::Pbr3d => Some(MaterialKind::Pbr3d(PbrSettings::default())),
            NewKind::Unlit3d => Some(MaterialKind::Unlit3d(UnlitSettings::default())),
            NewKind::Sprite2d => Some(MaterialKind::Sprite2d(SpriteSettings::default())),
            NewKind::Function | NewKind::Curve | NewKind::Consts => None,
        }
    }

    /// Default name in the "New ..." dialog.
    pub(super) fn base_name(self) -> &'static str {
        match self {
            NewKind::Function => "new_function",
            NewKind::Curve => "new_curve",
            NewKind::Consts => "new_consts",
            _ => "new_material",
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
    /// Pin layout of the Pbr3d output node (decision 0023).
    OutputMode,
    Description,
    /// Preview mode of the active material (decision 0022).
    PreviewMode,
    /// Editor-only label of a constant node (decision 0021).
    NodeLabel(NodeId),
    FrameTitle(String),
    /// Hue of a comment frame (`Frame::hue`, design §4.6): a `Float` in degrees, a `Str` when unset (any non-float
    /// value clears it: "Default colour").
    FrameHue(String),
    /// `(t, value)` of a point of the active curve, as a `Float2`.
    CurvePoint(usize),
    /// Interpolation the active curve view draws (editor-only).
    CurvePreview,
    /// Value of a constant of the active table (decision 0017).
    Const(String),
    /// Type of a constant of the active table (a change may ask first).
    ConstType(String),
    /// Name in the active table's add form.
    ConstDraftName,
    /// Type in the active table's add form.
    ConstDraftType,
    /// A settings field of the active Texture tab (design §4.4).
    Texture(TextureField),
    /// The active Texture tab's preview repeats the image 3x3 (editor-only).
    TextureTiled,
}

/// Which text a dialog's input submits to.
#[derive(Debug, Clone, PartialEq)]
pub enum Submit {
    Create(NewKind),
    Folder,
    Rename(AssetId),
    MoveTo(AssetId),
    OpenProject,
    NewProject,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Dialog {
    Input {
        title: String,
        label: String,
        initial: String,
        submit: Submit,
    },
    Confirm {
        title: String,
        message: String,
        buttons: Vec<(String, Action)>,
    },
    Message {
        title: String,
        message: String,
    },
    /// Help → About: logo, version, links.
    About,
    /// Help → Quick Start: how to hook a game up to the editor, showing this step (one tab per step).
    QuickStart(usize),
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

/// How far in `t` pasted / duplicated curve points land from the copied ones.
pub(super) const CURVE_PASTE_STEP: f32 = 0.05;

/// Edits of the active curve tab (decision 0015).
#[derive(Debug, Clone, PartialEq)]
pub enum CurveAction {
    Select(BTreeSet<usize>),
    /// Adds a point and selects it.
    Add((f32, f32)),
    /// Replaces all points and the selection; `merge`d edits (a drag in progress) are one undo step until
    /// [`Action::CommitInline`].
    SetPoints {
        points: Vec<(f32, f32)>,
        selection: BTreeSet<usize>,
        merge: bool,
    },
    /// Moves the selected points by `(dt, dvalue)` (arrow keys).
    Nudge((f32, f32)),
}

/// Edits of the active global constant table (decision 0017).
#[derive(Debug, Clone, PartialEq)]
pub enum ConstsAction {
    /// Adds the constant of the add form.
    Add,
    /// Changes a constant's type; `confirmed` once the user agreed to clear the references that break.
    SetType { name: String, ty: ConstType, confirmed: bool },
    /// Deletes a constant; `confirmed` once the user agreed to clear its references.
    Delete { name: String, confirmed: bool },
    /// Undo (`undo`) or redo in the table; asks first when the step retypes or removes a constant nodes read.
    History { undo: bool, confirmed: bool },
}

/// Edits of the active Texture tab (design §4.4); ignored on a built-in (read-only) texture.
#[derive(Debug, Clone, PartialEq)]
pub enum TextureAction {
    /// Points the asset at another image under `assets/`. Refused (log + dialog, model unchanged) when the path
    /// breaks the E012 rules or another Texture asset already claims the file (on disk or in an open tab).
    SetPath(String),
    /// Replaces the settings (one undo step).
    SetSettings(TextureSettings),
    /// Applies an Easy mode preset's sampler (one undo step; nothing when it already matches).
    Preset(FilterPreset),
    /// Easy (presets + Wrap) or Advanced (every sampler field) form; editor-only, not undoable.
    SetEasy(bool),
}

/// Which tabs the tab context menu closes, relative to the tab it was opened on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseTabs {
    All,
    Others,
    Left,
    Right,
    /// Every tab without unsaved changes.
    Unmodified,
}

/// What happens to unsaved changes of tabs being closed (or of the project).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unsaved {
    /// Ask first (a dialog that answers with `Discard` or `Save`).
    Ask,
    Discard,
    Save,
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
    /// Sets up a game crate as a project (asks for its directory).
    NewProjectDialog,
    NewProject(PathBuf),
    /// Opens the native folder picker for the Open / New project dialog.
    BrowseFolder,
    /// The picker's choice: submits the Open / New project dialog, if it is still open.
    FolderPicked(PathBuf),
    About,
    /// Opens Help → Quick Start on this step, or switches the open one to it.
    QuickStart(usize),
    /// The active Texture tab's preview shows only channel 0 / 1 / 2 (R / G / B) in grey, or (`None`) the image;
    /// editor-only, also on a built-in texture.
    TextureChannel(Option<usize>),
    /// Opens a web page in the default browser.
    OpenUrl(String),
    CloseProject(Unsaved),
    OpenAsset(AssetId),
    /// Opens (or activates) the read-only tab of a game image, path relative to `assets/` (design §4.4).
    OpenGameImage(String),
    ActivateTab(usize),
    CloseTab(usize),
    /// Several tabs at once (tab context menu).
    CloseTabs {
        which: CloseTabs,
        at: usize,
        unsaved: Unsaved,
    },
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
    /// All nodes of the graph / all points of the curve.
    SelectAll,
    Select(BTreeSet<NodeId>),
    SelectFrame(Option<String>),
    /// Frames selected on the canvas together with the node selection (pushed after [`Action::Select`]); only
    /// those whose nodes are all selected are kept in [`Doc::enclosed_frames`](crate::doc::Doc::enclosed_frames).
    SelectFrames(BTreeSet<String>),
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
    /// Removes every wire at a pin (Alt+LMB on it).
    DisconnectPin {
        node: NodeId,
        pin: String,
        output: bool,
    },
    /// Moves the wires of a pin to another pin of the same direction (Ctrl+LMB drag, decision 0032): an input's
    /// single wire to the input `to`, or every wire of an output to the output `to`. All or nothing, one undo step.
    MoveWires {
        node: NodeId,
        pin: String,
        output: bool,
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
    /// Changes the kind of nodes in place (node menu / hotkey "Convert to", decision 0021).
    ConvertNodes {
        nodes: Vec<NodeId>,
        conv: Conversion,
    },
    /// Something dragged from a sidebar tree dropped onto the active graph at `pos` (graph space), over `node` if
    /// any (design §4.3): a node with a matching param gets the reference, otherwise a new node is added at `pos`.
    Drop {
        item: crate::droppable::Dropped,
        pos: (f32, f32),
        node: Option<NodeId>,
    },
    /// Previews a node of the active material, or stops previewing it (node menu / `V`, decision 0022).
    TogglePreview(NodeId),
    Curve(CurveAction),
    Consts(ConstsAction),
    Texture(TextureAction),
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
    /// Opens a file in the user's default application.
    ShowFile(AssetId, GenFile),
    /// Opens a generated file in a read-only text tab.
    ShowGenerated(AssetId, Lang),
    /// Expands / collapses a folder of a sidebar tree.
    ToggleFolder(TreeId, String),
    /// Selects a Project-tree folder: the target of "New ..." items.
    SelectFolder(String),
    /// Creates a Texture asset for a game image (path relative to `assets/`, design §4.3).
    CreateTextureForFile(String),
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
    /// Images under the game's `assets/` (the Game Assets tree, design §4.2): paths relative to `assets/`, sorted;
    /// refreshed on open and by [`Editor::poll_watcher`].
    pub game_files: Vec<String>,
    /// Open tabs: graph documents and read-only views of generated files.
    pub tabs: Vec<Tab>,
    pub active: Option<usize>,
    pub clipboard: Option<ClipboardData>,
    /// Copied curve points.
    pub curve_clipboard: Option<Vec<(f32, f32)>>,
    pub(super) paste_count: u32,
    /// Declared function signatures (disk, overridden by open documents).
    pub functions: BTreeMap<String, FunctionSig>,
    /// Live diagnostics per asset (validation of the in-memory state).
    pub diags: BTreeMap<AssetId, Vec<Diagnostic>>,
    pub project_diags: Vec<Diagnostic>,
    /// E008/E010 from the last real regeneration (not reproduced by dry runs).
    pub(super) regen_only: BTreeMap<AssetId, Vec<Diagnostic>>,
    /// Line count of each asset's generated `.wesl` (the editor statusbar, design §2.3): taken from what
    /// regeneration writes, read once when a graph tab opens and refreshed whenever a generated WESL tab is read.
    pub wesl_lines: HashMap<AssetId, usize>,
    pub messages: Vec<Message>,
    pub bevy: Option<BevySources>,
    pub(super) validated: Option<Vec<(AssetId, u64)>>,
    /// Collapsed folders per sidebar tree.
    pub collapsed: BTreeMap<TreeId, BTreeSet<String>>,
    /// The selected Project-tree folder (where "New ..." creates assets).
    pub current_folder: String,
    pub dialog: Option<Dialog>,
    pub ui: UiRev,
    pub view_requests: Vec<ViewRequest>,
    pub(super) watcher: Option<std::sync::Mutex<ShadersWatcher>>,
    pub config: UserConfig,
    pub config_path: Option<PathBuf>,
    pub quit: bool,
    /// Files to open in an external program.
    pub external: Vec<PathBuf>,
    /// Text for the system clipboard (Ctrl+C in a text tab).
    pub copy_text: Option<String>,
    /// The native folder picker is requested (Open / New project dialogs); the app runs it and sends
    /// [`Action::FolderPicked`].
    pub pick_folder: bool,
}

impl Default for Editor {
    fn default() -> Self {
        Editor {
            project: None,
            registry: shader_stdlib::registry(),
            disk: AssetSet::default(),
            folders: Vec::new(),
            game_files: Vec::new(),
            tabs: Vec::new(),
            active: None,
            clipboard: None,
            curve_clipboard: None,
            paste_count: 0,
            functions: BTreeMap::new(),
            diags: BTreeMap::new(),
            project_diags: Vec::new(),
            regen_only: BTreeMap::new(),
            wesl_lines: HashMap::new(),
            messages: Vec::new(),
            bevy: None,
            validated: None,
            collapsed: BTreeMap::new(),
            current_folder: String::new(),
            dialog: None,
            ui: UiRev::default(),
            view_requests: Vec::new(),
            watcher: None,
            config: UserConfig::default(),
            config_path: None,
            quit: false,
            external: Vec::new(),
            copy_text: None,
            pick_folder: false,
        }
    }
}
