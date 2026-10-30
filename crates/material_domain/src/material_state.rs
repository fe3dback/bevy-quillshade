//! Everything the Material domain keeps inside [`crate::ops::Editor`] (design §6.3), and its hooks.

use std::any::Any;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use editor_core::dynval::Payload;
use editor_core::tab::TabKey;
use project_core::{AssetRef, DomainId};
use shader_codegen_wesl::check::BevySources;
use shader_graph_core::gradient::Slot;
use shader_graph_core::{Diagnostic, FunctionSig, Registry};
use shader_project::layout::MATERIAL;
use shader_project::{AssetId, AssetSet};

use crate::doc::ClipboardData;
use crate::ops::prelude::*;
use editor_core::domain::{DomainState, DomainStates, TreeAsset};
use editor_core::state::editor::Editor;

pub struct MaterialState {
    pub registry: Registry,
    /// Assets as stored on disk (last scan).
    pub disk: AssetSet,
    pub clipboard: Option<ClipboardData>,
    /// A copied gradient (stops + name; its id is never pasted).
    pub gradient_clipboard: Option<Slot>,
    pub(crate) paste_count: u32,
    /// Declared function signatures (disk, overridden by open documents).
    pub functions: BTreeMap<String, FunctionSig>,
    /// Live diagnostics per asset (validation of the in-memory state).
    pub diags: BTreeMap<AssetId, Vec<Diagnostic>>,
    pub project_diags: Vec<Diagnostic>,
    /// E008/E010 from the last real regeneration (not reproduced by dry runs).
    pub(crate) regen_only: BTreeMap<AssetId, Vec<Diagnostic>>,
    /// Line count of each asset's generated `.wesl` (the editor statusbar, design §2.3): taken from what
    /// regeneration writes, read once when a graph tab opens and refreshed whenever a generated WESL tab is read.
    pub wesl_lines: HashMap<AssetId, usize>,
    pub bevy: Option<BevySources>,
    pub(crate) validated: Option<Vec<(AssetId, u64)>>,
    /// A tab of the domain closed (or dropped its edits) since the last `after_apply`: function signatures and
    /// validation follow.
    pub(crate) tabs_closed: bool,
    /// Node / frame drags applied since the canvas last looked: the graph shown before and after each
    /// (`crate::follow_moves`).
    pub(crate) moves: Vec<(Option<(TabKey, u64)>, Option<(TabKey, u64)>)>,
    /// The constants the asset being deleted defines ([`DomainState::before_delete`] to [`DomainState::deleted`]).
    pub(crate) deleting: Vec<String>,
    /// [`Editor::project_generation`] the per-project fields belong to: a rescan after the project was opened or
    /// closed starts them over.
    pub(crate) loaded: u64,
}

impl Default for MaterialState {
    fn default() -> Self {
        MaterialState { registry: shader_stdlib::registry(), ..MaterialState::empty() }
    }
}

impl MaterialState {
    /// A state without node definitions (cheap): what stands in while the real one is lent ([`MaterialState::lent`]).
    fn empty() -> Self {
        MaterialState {
            registry: Registry::default(),
            disk: AssetSet::default(),
            clipboard: None,
            gradient_clipboard: None,
            paste_count: 0,
            functions: BTreeMap::new(),
            diags: BTreeMap::new(),
            project_diags: Vec::new(),
            regen_only: BTreeMap::new(),
            wesl_lines: HashMap::new(),
            bevy: None,
            validated: None,
            tabs_closed: false,
            moves: Vec::new(),
            deleting: Vec::new(),
            loaded: 0,
        }
    }

    /// Runs `f` with this state back in `editor.domains`. A hook runs with the state taken out
    /// ([`Editor::with_domain`]), while Material's `Editor` methods read it through `editor.domains`
    /// ([`MaterialStates`]): the hook lends it back for the call, so they never find it missing.
    fn lent<R>(&mut self, editor: &mut Editor, f: impl FnOnce(&mut Editor) -> R) -> R {
        editor.domains.put_back(Box::new(std::mem::replace(self, MaterialState::empty())));
        let out = f(editor);
        let back = editor.domains.take(MATERIAL).and_then(|s| (s as Box<dyn Any>).downcast::<MaterialState>().ok());
        // Nothing replaces or keeps a lent state; going on with the placeholder would silently drop the project.
        let Some(state) = back else { panic!("the lent Material state was not given back") };
        *self = *state;
        out
    }
}

impl DomainState for MaterialState {
    fn id(&self) -> DomainId {
        MATERIAL
    }

    fn apply(&mut self, editor: &mut Editor, action: &Payload) {
        self.lent(editor, |e| e.material_payload(action));
    }

    fn after_apply(&mut self, editor: &mut Editor) {
        self.lent(editor, Editor::material_after_apply);
    }

    /// A save operation regenerates the project from disk once, whichever tabs it wrote (unsaved previews keep their
    /// output, decision 0022).
    fn after_save(&mut self, editor: &mut Editor, _tabs: &[usize]) {
        self.lent(editor, |e| {
            e.regenerate();
        });
    }

    fn before_close(&mut self, editor: &mut Editor, tab: usize) {
        self.lent(editor, |e| e.material_before_close(tab));
    }

    fn rescan(&mut self, editor: &mut Editor) {
        self.lent(editor, Editor::material_rescan);
    }

    /// Live diagnostics of the project, then each asset's (by id): live ones, then the regeneration-only codes.
    fn diagnostics(&self) -> Vec<project_core::Diagnostic> {
        self.console_diagnostics(None)
    }

    /// [`DomainState::diagnostics`] with the active graph's asset first, after the project's own.
    fn console_diagnostics(&self, active: Option<&AssetRef>) -> Vec<project_core::Diagnostic> {
        let active = active.and_then(AssetId::from_ref).filter(|id| id.ty.is_graph());
        let mut ids: Vec<&AssetId> = self.diags.keys().chain(self.regen_only.keys()).collect();
        ids.sort_by_key(|id| (Some(*id) != active.as_ref(), (*id).clone()));
        ids.dedup();
        let assets =
            ids.into_iter().flat_map(|id| self.diags.get(id).into_iter().chain(self.regen_only.get(id)).flatten());
        self.project_diags.iter().chain(assets).map(|d| d.to_generic(MATERIAL)).collect()
    }

    fn tree_items(&self, editor: &Editor) -> Option<Vec<TreeAsset>> {
        Some(self.project_tree_items(editor))
    }

    fn generate(&mut self, editor: &mut Editor) {
        self.lent(editor, |e| {
            e.regenerate();
        });
    }

    fn delete_message(&self, editor: &Editor, r: &AssetRef) -> Option<String> {
        editor.material_delete_message(r)
    }

    fn rename_file(&mut self, editor: &mut Editor, r: &AssetRef, new_path: &str) -> Result<AssetRef, String> {
        self.lent(editor, |e| e.material_rename_file(r, new_path))
    }

    fn renamed(&mut self, editor: &mut Editor, from: &AssetRef, to: &AssetRef) {
        self.lent(editor, |e| e.material_renamed(from, to));
    }

    fn before_delete(&mut self, editor: &mut Editor, r: &AssetRef) {
        self.lent(editor, |e| e.material_before_delete(r));
    }

    fn delete_file(&mut self, editor: &mut Editor, r: &AssetRef) -> Result<(), String> {
        self.lent(editor, |e| e.material_delete_file(r))
    }

    fn deleted(&mut self, editor: &mut Editor, r: &AssetRef) {
        self.lent(editor, |e| e.material_deleted(r));
    }

    fn focus_diagnostic(&mut self, editor: &mut Editor, diag: &project_core::Diagnostic) {
        self.lent(editor, |e| e.material_focus_diagnostic(diag));
    }

    fn files_changed(&mut self, editor: &mut Editor, changed: &[PathBuf]) {
        self.lent(editor, |e| e.material_files_changed(changed));
    }
}

/// Typed access to the Material state inside [`DomainStates`]; panics when the domain is not registered.
pub trait MaterialStates {
    fn material(&self) -> &MaterialState;
    fn material_mut(&mut self) -> &mut MaterialState;
}

impl MaterialStates for DomainStates {
    fn material(&self) -> &MaterialState {
        self.get::<MaterialState>().expect("Material domain not registered")
    }
    fn material_mut(&mut self) -> &mut MaterialState {
        self.get_mut::<MaterialState>().expect("Material domain not registered")
    }
}
