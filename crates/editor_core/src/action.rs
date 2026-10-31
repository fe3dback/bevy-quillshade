//! What the UI asks the editor to do (design §6.2). [`Action`] holds the core commands (project, tabs, dialogs, View
//! toggles, navigation, save, Generate, folders, rename / move / delete, messages, quit) plus two open doors:
//! [`Action::Tab`] for an edit of the active tab's document and [`Action::Domain`] for a domain's own command, which
//! the domain downcasts from its [`Payload`].
//!
//! [`TabAction`]s are the shared edit verbs, property writes and kind-specific edits. The editor dispatches each to
//! [`crate::doc_kind::TabDoc::apply`] of the active tab, so core needs no per-kind code for Undo, Copy, Paste… A
//! document that does not support a verb, or does not know the payload, answers [`Handled::No`] and nothing happens.

use std::path::PathBuf;

use bevy::math::Vec2;
use project_core::{AssetRef, Diagnostic, DomainId};

use crate::dynval::{Payload, PropKey, PropValue};

#[derive(Debug, Clone, PartialEq)]
pub enum TabAction {
    Undo,
    Redo,
    Copy,
    Cut,
    /// Pastes the clipboard, at this position of the view (graph space on a canvas) when given.
    Paste(Option<Vec2>),
    Duplicate,
    DeleteSelection,
    /// Everything the view can select (all nodes of a graph, all points of a curve).
    SelectAll,
    /// An edit made in place finished (number drag released, field committed, picker closed): ends undo merging
    /// and redraws the right sidebar, which is not redrawn during merged edits.
    CommitInline,
    /// Sets the property `key`; `merge`d writes of one key (scrubbing, typing) are one undo step until
    /// [`TabAction::CommitInline`].
    SetValue {
        key: PropKey,
        value: PropValue,
        merge: bool,
    },
    /// Sets component `comp` of the property `key` (a number field of a vector); merged like
    /// [`TabAction::SetValue`] until `is_final`.
    SetComponent {
        key: PropKey,
        comp: usize,
        value: f32,
        is_final: bool,
    },
    /// A kind-specific edit, which the document downcasts (a curve's point drag, a table's Add…).
    Doc(Payload),
}

impl TabAction {
    /// A kind-specific edit.
    pub fn doc<T: crate::dynval::DynPayload>(edit: T) -> TabAction {
        TabAction::Doc(Payload::new(edit))
    }
}

/// Whether a document took a [`TabAction`].
#[derive(Debug, PartialEq)]
pub enum Handled {
    Yes,
    No,
}

/// Which sidebar tree a row belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum TreeId {
    /// A domain's `quillshade/<domain>/` tree of editor assets (design §8.1).
    Project(DomainId),
    /// The game's files under `assets/` the domains show (read-only).
    GameAssets,
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

/// Camera requests the canvas carries out once layout is known.
#[derive(Debug, Clone, PartialEq)]
pub enum ViewRequest {
    FitAll,
    FitSelection,
    /// Back / Forward restores a graph camera: `centre` (graph space) at the middle of the canvas, at `zoom`
    /// (decision 0041).
    Set {
        centre: Vec2,
        zoom: f32,
    },
}

/// Which text a dialog's input submits to.
#[derive(Debug, Clone, PartialEq)]
pub enum Submit {
    /// Creates an asset of a registered asset kind (`(domain, suffix)`) from its template `template` (design §6.2).
    Create {
        kind: (DomainId, &'static str),
        template: usize,
    },
    /// Creates a folder in a domain's Project tree (design §6.3).
    Folder(DomainId),
    Rename(AssetRef),
    MoveTo(AssetRef),
    OpenProject,
    NewProject,
    /// Sets a text value of the active tab (a node's name / label, a comment's title; design §5.5).
    SetText(PropKey),
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

impl From<TabAction> for Action {
    fn from(a: TabAction) -> Action {
        Action::Tab(a)
    }
}

/// UI → editor command queue, drained once per frame.
#[derive(bevy::prelude::Resource, Default)]
pub struct Actions(pub Vec<Action>);

impl Actions {
    pub fn push(&mut self, a: impl Into<Action>) {
        self.0.push(a.into());
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Action {
    #[default]
    None,
    OpenProjectDialog,
    /// Opens a project in place of the open one; asks first when tabs have unsaved changes.
    OpenProject(PathBuf),
    /// [`Action::OpenProject`] with the answer about the unsaved changes (the buttons of its question).
    SwitchProject(PathBuf, Unsaved),
    /// Removes a project from the recent projects (the "x" of its row on the welcome screen / Open project dialog).
    ForgetRecent(PathBuf),
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
    /// Opens a web page in the default browser.
    OpenUrl(String),
    CloseProject(Unsaved),
    OpenAsset(AssetRef),
    /// Opens a file in the default application (a generated file's "Open externally"); a warning when it is missing.
    OpenFile(PathBuf),
    /// Opens (or activates) the read-only tab of a game image, path relative to `assets/` (design §4.4).
    OpenGameImage(String),
    /// Opens tool `id` of `domain` (design §14.1), or activates its open tab; `arg` then goes to that tab's document
    /// as [`TabAction::Doc`]. An unknown tool does nothing.
    OpenTool {
        domain: DomainId,
        id: crate::domain::ToolId,
        arg: Option<Payload>,
    },
    ActivateTab(usize),
    /// Makes a domain the active one (a domain tab, design §14.3) and activates its last active tab, if open; else
    /// no tab is active. The active domain: nothing happens.
    SwitchDomain(DomainId),
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
    /// View → Show project (Ctrl+1): flips `UserConfig::show_left_sidebar` and saves the user config.
    ToggleLeftSidebar,
    /// View → Show console (Ctrl+2): flips `UserConfig::show_log` and saves the user config.
    ToggleLog,
    /// View → Show properties (Ctrl+3): flips `UserConfig::show_properties` and saves the user config.
    ToggleProperties,
    /// View → Zen mode (`` ` ``): flips `UserConfig::zen_mode` and saves the user config.
    ToggleZen,
    /// The console/logs "All domains" checkbox: flips `Editor::console_all` (session only, design §6.4).
    ToggleConsoleAll,
    /// Back / Forward through the navigation history (decision 0041).
    NavBack,
    NavForward,
    Save,
    SaveAll,
    /// Generate (Assets menu): every domain regenerates from what is saved.
    RegenAll,
    /// Re-scans the open project's files and refreshes every domain's state.
    Rescan,
    /// An edit for the active tab's document (design §6.2): the shared edit verbs, property writes and
    /// kind-specific edits.
    Tab(TabAction),
    /// A command of one domain, which downcasts the payload (Material: `MaterialAction`); an unknown domain or
    /// payload is ignored.
    Domain(DomainId, Payload),
    /// Asks for the path of a new asset of kind `(domain, suffix)` made from its template `template` (the "New …"
    /// items of the Assets menu and the folder menus, design §6.2).
    NewAssetDialog {
        kind: (DomainId, &'static str),
        template: usize,
    },
    /// Asks for the path of a new folder of a domain's Project tree (`None`: the active tab's domain, else the first
    /// domain), proposed in that domain's selected folder.
    NewFolderDialog(Option<DomainId>),
    RenameDialog(AssetRef),
    MoveDialog(AssetRef),
    DeleteDialog(AssetRef),
    DeleteFolderDialog(DomainId, String),
    Submit(String),
    MoveTo(AssetRef, String),
    Delete(AssetRef),
    DeleteFolder(DomainId, String),
    /// Expands / collapses a folder of a sidebar tree.
    ToggleFolder(TreeId, String),
    /// Selects a folder of a domain's Project tree: the target of that domain's "New ..." items (design §6.3).
    SelectFolder(DomainId, String),
    FocusDiagnostic(Diagnostic),
    /// The tab `TabKey` names (an asset's or a tool's, [`crate::tab::Tab::key`]) reloads its file, dropping its edits
    /// (the "File changed on disk" dialog).
    Reload(crate::tab::TabKey),
    /// The tab keeps its edits: the file's version becomes its saved baseline ([`crate::doc_kind::TabDoc::keep_mine`]).
    KeepMine(crate::tab::TabKey),
    CloseDialog,
    ClearMessages,
    /// Warning for the log (e.g. a refused wire).
    Notify(String),
    Quit,
    ForceQuit,
    SaveAllAndQuit,
}

#[cfg(test)]
mod tests {
    use project_core::DomainId;

    use super::*;
    use crate::doc_kind::DocCtx;
    use crate::dynval::PropTarget;
    use crate::tab::Tab;
    use crate::tab::tests::TestDoc;

    #[derive(Debug, PartialEq)]
    struct Nudge(i32);
    #[derive(Debug, PartialEq)]
    struct Field(&'static str);
    impl PropTarget for Field {}

    #[test]
    fn actions_compare_by_value() {
        assert_eq!(TabAction::doc(Nudge(1)), TabAction::doc(Nudge(1)));
        assert_ne!(TabAction::doc(Nudge(1)), TabAction::doc(Nudge(2)));
        let set = |merge| TabAction::SetValue { key: PropKey::new(Field("a")), value: PropValue::Float(1.0), merge };
        assert_eq!(set(true), set(true));
        assert_ne!(set(true), set(false));
        let TabAction::Doc(p) = TabAction::doc(Nudge(3)) else { panic!("a doc payload") };
        assert_eq!(p.downcast::<Nudge>(), Some(&Nudge(3)));
        assert!(p.downcast::<Field>().is_none());
    }

    /// A document that implements no verb ignores every action.
    #[test]
    fn default_apply_handles_nothing() {
        let mut tab = Tab::new(DomainId::new("test"), TestDoc("a.x"));
        let (mut state, mut messages) = ((), Vec::new());
        let mut ctx = DocCtx::new(None, &mut state, &mut messages);
        for a in [TabAction::Undo, TabAction::Paste(Some(Vec2::ONE)), TabAction::doc(Nudge(1))] {
            assert_eq!(tab.doc_mut().apply(&mut ctx, &a), Handled::No);
        }
        assert!(tab.doc().current_value(&PropKey::new(Field("a")), &Default::default()).is_none());
        assert!(!ctx.redraw_props && ctx.deferred.is_empty() && messages.is_empty());
    }
}
