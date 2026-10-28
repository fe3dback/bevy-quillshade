//! Editor state and commands. Plain Rust (wrapped in a Bevy resource): UI systems only push
//! [`Action`]s, [`Editor::apply`] mutates the model, the canvas and panels redraw from it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use shader_codegen_wesl::check::BevySources;
use shader_graph_core::consts::{ConstType, GlobalConst};
use shader_graph_core::curve::Interp;
use shader_graph_core::gradient::{GradientCollection, MAX_STOPS, Slot, Stop};
use shader_graph_core::ident::check_ident;
use shader_graph_core::ir::declared_function_sig;
use shader_graph_core::model::{
    Asset, AssetKind, BlendMode, MaterialKind, NodeId, PbrOutputMode, PbrSettings, PreviewMode, Shading,
    SpriteSettings, UnlitSettings,
};
use shader_graph_core::registry::{
    Conversion, FUNCTION_PREFIX, Hotkey, HotkeyKey, NodeAction, NodeOp, NodeRole, ParamKind, const_refs,
};
use shader_graph_core::texture::{TextureAsset, TextureSettings};
use shader_graph_core::{Code, Diagnostic, FunctionSig, PinType, Registry, Value};
use shader_project::regen::{project_registry, rust_file, wesl_file};
use shader_project::watch::ShadersWatcher;
use shader_project::{AssetId, AssetSet, AssetType, Project, RegenOptions, Scope, assets, game_assets, ops};

use crate::autolayout;
use crate::config::{ProjectTabs, UserConfig};
use crate::consts_doc::ConstsDoc;
use crate::curve_doc::CurveDoc;
use crate::doc::{ClipboardData, Doc, bounds, connection_error, node_signature};
use crate::doc_kind::PreviewDoc;
use crate::game_image::Channel;
use crate::gradients_doc::{GradientSel, GradientsDoc};
use crate::search::{self, WireEnd};
use crate::tab::{ImageDoc, Lang, Tab, TabKey, TextDoc, parse_config_key};
use crate::texture_doc::{FilterPreset, TextureDoc, TextureField, fits_easy};
use crate::ui::plural;
use crate::ui::tree::TreeId;

mod apply;
mod asset_ops;
mod consts_ops;
mod curve_ops;
mod gradient_ops;
mod graph_ops;
mod regen;
mod tabs;
#[cfg(test)]
mod tests;
mod texture_ops;
mod types;
mod watch;

use consts_ops::usage_message;
pub use types::*;

/// Label of a preview mode in the Properties select (`Unlit`, `Lit`, `Full`).
pub fn preview_mode_label(mode: PreviewMode) -> &'static str {
    match mode {
        PreviewMode::Unlit => "Unlit",
        PreviewMode::Lit => "Lit",
        PreviewMode::Full => "Full",
    }
}

pub(super) fn kind_of(set: &AssetSet, id: &AssetId) -> Option<AssetKind> {
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

    pub(super) fn push_msg(&mut self, level: Level, text: String) {
        self.messages.push(Message { level, text });
        if self.messages.len() > 200 {
            self.messages.remove(0);
        }
        self.ui.log += 1;
    }

    pub(super) fn io_error(&mut self, what: &str, e: impl std::fmt::Display) {
        let message = format!("{what}: {e}");
        self.error(message.clone());
        self.dialog = Some(Dialog::Message { title: "Error".into(), message });
    }

    pub(super) fn touch_all(&mut self) {
        self.ui.browser += 1;
        self.ui.tabs += 1;
        self.ui.props += 1;
        self.ui.log += 1;
    }

    // ---- documents ----

    pub fn tab(&self) -> Option<&Tab> {
        self.active.and_then(|i| self.tabs.get(i))
    }

    /// The active tab's graph document (`None` when another kind of tab is active).
    pub fn doc(&self) -> Option<&Doc> {
        self.tab().and_then(Tab::graph)
    }

    pub fn doc_mut(&mut self) -> Option<&mut Doc> {
        self.active.and_then(|i| self.tabs.get_mut(i)).and_then(Tab::graph_mut)
    }

    /// The active tab's curve document.
    pub fn curve_doc(&self) -> Option<&CurveDoc> {
        self.tab().and_then(Tab::curve)
    }

    pub fn curve_doc_mut(&mut self) -> Option<&mut CurveDoc> {
        self.active.and_then(|i| self.tabs.get_mut(i)).and_then(Tab::curve_mut)
    }

    /// Tab index of a curve document.
    pub fn curve_index(&self, id: &AssetId) -> Option<usize> {
        self.tabs.iter().position(|t| t.curve().is_some_and(|c| c.id == *id))
    }

    /// The active tab's constant table.
    pub fn consts_doc(&self) -> Option<&ConstsDoc> {
        self.tab().and_then(Tab::consts)
    }

    pub fn consts_doc_mut(&mut self) -> Option<&mut ConstsDoc> {
        self.active.and_then(|i| self.tabs.get_mut(i)).and_then(Tab::consts_mut)
    }

    /// Tab index of a constant table.
    pub fn consts_index(&self, id: &AssetId) -> Option<usize> {
        self.tabs.iter().position(|t| t.consts().is_some_and(|c| c.id == *id))
    }

    /// The active tab's Texture asset (or built-in texture).
    pub fn texture_doc(&self) -> Option<&TextureDoc> {
        self.tab().and_then(Tab::texture)
    }

    pub fn texture_doc_mut(&mut self) -> Option<&mut TextureDoc> {
        self.active.and_then(|i| self.tabs.get_mut(i)).and_then(Tab::texture_mut)
    }

    /// The active tab's gradient collection.
    pub fn gradients_doc(&self) -> Option<&GradientsDoc> {
        self.tab().and_then(Tab::gradients)
    }

    pub fn gradients_doc_mut(&mut self) -> Option<&mut GradientsDoc> {
        self.active.and_then(|i| self.tabs.get_mut(i)).and_then(Tab::gradients_mut)
    }

    /// The project's loadable gradient collections as `(id, label)` choices (a `GradientCollection` parameter).
    pub fn gradient_collections(&self) -> Vec<(String, String)> {
        let ok = self.disk.gradients.iter().filter(|(_, c)| c.is_ok()).map(|(id, _)| id.path.clone());
        ok.map(|p| (p.clone(), p)).collect()
    }

    /// Collection `id` (its path): an open tab's unsaved state first, then the file as last scanned.
    pub fn gradient_collection(&self, id: &str) -> Option<&GradientCollection> {
        let open = self.tabs.iter().filter_map(Tab::gradients).find(|g| g.id.path == id);
        open.map(|g| &g.collection).or_else(|| {
            let id = AssetId::new(AssetType::Gradients, id);
            self.disk.gradients.get(&id).and_then(|c| c.as_ref().ok())
        })
    }

    /// The active tab's preview view state, on a tab with the shared image preview (image, Texture; design §6.2).
    pub fn preview_doc_mut(&mut self) -> Option<&mut dyn PreviewDoc> {
        self.active.and_then(|i| self.tabs.get_mut(i)).and_then(Tab::preview_mut)
    }

    /// Tab index of a Texture asset or built-in texture (`Texture` + `@internal/…`).
    pub fn texture_index(&self, id: &AssetId) -> Option<usize> {
        self.tabs.iter().position(|t| t.texture().is_some_and(|t| t.id == *id))
    }

    /// Tab index of an asset's editable document (graph, curve, constant table or Texture asset).
    pub fn asset_index(&self, id: &AssetId) -> Option<usize> {
        self.tabs.iter().position(|t| t.doc().asset() == Some(id))
    }

    /// Every global constant by name with its table: open tables as edited, the others as saved. A name defined in
    /// several tables keeps the first one (path order), as in code generation.
    pub fn global_consts(&self) -> BTreeMap<String, (AssetId, GlobalConst)> {
        let mut set = AssetSet { consts: self.disk.consts.clone(), ..Default::default() };
        for c in self.tabs.iter().filter_map(Tab::consts) {
            set.consts.insert(c.id.clone(), Ok(c.table.clone()));
        }
        shader_project::regen::collect_consts(&set).0
    }

    /// Paths of the project's loadable curve assets (choices of a `Curve` parameter).
    pub fn curve_paths(&self) -> Vec<String> {
        self.disk.curves.iter().filter(|(_, c)| c.is_ok()).map(|(id, _)| id.path.clone()).collect()
    }

    /// Open graph documents.
    pub fn docs(&self) -> impl Iterator<Item = &Doc> {
        self.tabs.iter().filter_map(Tab::graph)
    }

    /// Tab index of an asset's graph document.
    pub fn doc_index(&self, id: &AssetId) -> Option<usize> {
        self.tabs.iter().position(|t| t.graph().is_some_and(|d| d.id == *id))
    }

    /// Tab index of an asset's generated file.
    pub fn text_index(&self, id: &AssetId, lang: Lang) -> Option<usize> {
        self.tabs.iter().position(|t| t.text().is_some_and(|t| t.source == *id && t.lang == lang))
    }

    /// Tab index of a game image.
    pub fn image_index(&self, path: &str) -> Option<usize> {
        self.tabs.iter().position(|t| t.image().is_some_and(|i| i.path == path))
    }

    pub fn is_dirty(&self) -> bool {
        self.tabs.iter().any(|t| t.doc().is_dirty())
    }

    /// Kind of an asset (open document first, then disk).
    pub fn asset_kind(&self, id: &AssetId) -> Option<AssetKind> {
        self.docs().find(|d| d.id == *id).map(|d| d.asset.kind).or_else(|| kind_of(&self.disk, id))
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
}
