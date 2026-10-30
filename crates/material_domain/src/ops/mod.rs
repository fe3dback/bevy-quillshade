//! The Material domain's editor code (design §6.3): its commands ([`MaterialAction`]), documents, regeneration and
//! its side of the core flows. The `Editor` itself and the core dispatch are in `editor_core`; Material's methods on
//! it are extension traits, one per file ([`MaterialEditor`], [`MaterialApply`], [`MaterialAssets`],
//! [`MaterialConsts`], [`MaterialGraphOps`], [`MaterialRegen`], [`MaterialTabs`], [`MaterialWatch`], and
//! `crate::texture`'s [`MaterialTextures`]); `use crate::ops::prelude::*` brings them all in scope.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use editor_core::action::{Handled, TabAction};
use editor_core::dynval::{Payload, PropKey, PropTarget, PropValue};
use editor_core::history::MergeKey;
use project_core::AssetRef;
use shader_graph_core::consts::{ConstType, GlobalConst};
use shader_graph_core::gradient::{GradientCollection, MAX_STOPS, Stop};
use shader_graph_core::ident::check_ident;
use shader_graph_core::ir::declared_function_sig;
use shader_graph_core::model::{
    Asset, AssetKind, BlendMode, MaterialKind, NodeId, PbrOutputMode, PreviewMode, Shading,
};
use shader_graph_core::registry::{
    Conversion, FUNCTION_PREFIX, Hotkey, HotkeyKey, NodeAction, NodeOp, NodeRole, ParamKind, const_refs,
};
use shader_graph_core::{Code, Diagnostic, PinType, Registry, Value};
use shader_project::layout::{MATERIAL, MaterialLayout, USER_NODES_DIR};
use shader_project::regen::{project_registry, rust_file, wesl_file};
use shader_project::{AssetId, AssetSet, AssetType, RegenOptions, Scope, assets, ops};

use crate::autolayout;
use crate::consts_doc::ConstsDoc;
use crate::curve::CurveDoc;
use crate::doc::{Doc, bounds, connection_error, node_signature};
use crate::doc_kind::DocCtx;
use crate::gradients_doc::{GradientSel, GradientsDoc};
use crate::material_state::{MaterialState, MaterialStates};
use crate::search::{self, WireEnd};
use crate::tab::{MaterialKey, Tab, TextDoc, material_tab, parse_config_key};
use crate::texture::{MaterialTexture, TextureDoc, TextureDocId};
use crate::util::text::plural;
use texture_kit::editor::TextureDomain;

mod apply;
mod asset_ops;
mod consts_ops;
pub mod droppable;
mod gradient_ops;
mod graph_ops;
mod nav;
mod regen;
mod tabs;
mod types;
mod watch;

pub use crate::texture::MaterialTextures;
pub use apply::MaterialApply;
pub use asset_ops::MaterialAssets;
pub use consts_ops::MaterialConsts;
use consts_ops::usage_message;
pub use graph_ops::MaterialGraphOps;
pub use nav::*;
pub use regen::MaterialRegen;
pub use tabs::MaterialTabs;
pub(crate) use tabs::{asset_tab, parse_tab, tab_exists};
pub use types::*;
pub use watch::MaterialWatch;

/// Every Material extension trait of `Editor`.
pub mod prelude {
    pub use super::{
        MaterialApply, MaterialAssets, MaterialConsts, MaterialEditor, MaterialGraphOps, MaterialRegen, MaterialTabs,
        MaterialTextures, MaterialWatch,
    };
    pub use crate::texture::TextureDocId;
}

/// Label of a preview mode in the Properties select (`Unlit`, `Lit`, `Full`).
pub fn preview_mode_label(mode: PreviewMode) -> &'static str {
    match mode {
        PreviewMode::Unlit => "Unlit",
        PreviewMode::Lit => "Lit",
        PreviewMode::Full => "Full",
    }
}

pub(crate) fn kind_of(set: &AssetSet, id: &AssetId) -> Option<AssetKind> {
    set.assets.get(id).and_then(|a| a.as_ref().ok()).map(|a| a.kind)
}

/// Material's `Editor` methods (design §6.3): its documents among the tabs and what the project's assets offer.
pub trait MaterialEditor {
    /// The active tab's graph document (`None` when another kind of tab is active).
    fn doc(&self) -> Option<&Doc>;

    fn doc_mut(&mut self) -> Option<&mut Doc>;

    /// The active tab's curve document.
    fn curve_doc(&self) -> Option<&CurveDoc>;

    fn curve_doc_mut(&mut self) -> Option<&mut CurveDoc>;

    /// Tab index of a curve document.
    fn curve_index(&self, id: &AssetId) -> Option<usize>;

    /// The active tab's constant table.
    fn consts_doc(&self) -> Option<&ConstsDoc>;

    fn consts_doc_mut(&mut self) -> Option<&mut ConstsDoc>;

    /// Tab index of a constant table.
    fn consts_index(&self, id: &AssetId) -> Option<usize>;

    /// The active tab's Texture asset (or built-in texture).
    fn texture_doc(&self) -> Option<&TextureDoc>;

    fn texture_doc_mut(&mut self) -> Option<&mut TextureDoc>;

    /// The active tab's gradient collection.
    fn gradients_doc(&self) -> Option<&GradientsDoc>;

    fn gradients_doc_mut(&mut self) -> Option<&mut GradientsDoc>;

    /// The project's loadable gradient collections as `(id, label)` choices (a `GradientCollection` parameter).
    fn gradient_collections(&self) -> Vec<(String, String)>;

    /// Collection `id` (its path): an open tab's unsaved state first, then the file as last scanned.
    fn gradient_collection(&self, id: &str) -> Option<&GradientCollection>;

    /// Tab index of a Texture asset or built-in texture (`Texture` + `@internal/…`).
    fn texture_index(&self, id: &AssetId) -> Option<usize>;

    /// Tab index of an asset's editable document (graph, curve, constant table or Texture asset).
    fn asset_index(&self, id: &AssetId) -> Option<usize>;

    /// Every global constant by name with its table: open tables as edited, the others as saved. A name defined in
    /// several tables keeps the first one (path order), as in code generation.
    fn global_consts(&self) -> BTreeMap<String, (AssetId, GlobalConst)>;

    /// Paths of the project's loadable curve assets (choices of a `Curve` parameter).
    fn curve_paths(&self) -> Vec<String>;

    /// Open graph documents.
    fn docs(&self) -> impl Iterator<Item = &Doc>;

    /// Tab index of an asset's graph document.
    fn doc_index(&self, id: &AssetId) -> Option<usize>;

    /// Tab index of an asset's generated file.
    fn text_index(&self, id: &AssetId, lang: &'static str) -> Option<usize>;

    /// Tab index of a game image.
    fn image_index(&self, path: &str) -> Option<usize>;

    /// Kind of an asset (open document first, then disk).
    fn asset_kind(&self, id: &AssetId) -> Option<AssetKind>;
}

impl MaterialEditor for Editor {
    // ---- documents ----

    fn doc(&self) -> Option<&Doc> {
        self.active_doc()
    }

    fn doc_mut(&mut self) -> Option<&mut Doc> {
        self.active_doc_mut()
    }

    fn curve_doc(&self) -> Option<&CurveDoc> {
        self.active_doc()
    }

    fn curve_doc_mut(&mut self) -> Option<&mut CurveDoc> {
        self.active_doc_mut()
    }

    fn curve_index(&self, id: &AssetId) -> Option<usize> {
        curve_kit::editor::index::<crate::curve::MaterialCurve>(self, &id.to_ref())
    }

    fn consts_doc(&self) -> Option<&ConstsDoc> {
        self.active_doc()
    }

    fn consts_doc_mut(&mut self) -> Option<&mut ConstsDoc> {
        self.active_doc_mut()
    }

    fn consts_index(&self, id: &AssetId) -> Option<usize> {
        self.tabs.iter().position(|t| t.get::<ConstsDoc>().is_some_and(|c| c.id == *id))
    }

    fn texture_doc(&self) -> Option<&TextureDoc> {
        self.active_doc()
    }

    fn texture_doc_mut(&mut self) -> Option<&mut TextureDoc> {
        self.active_doc_mut()
    }

    fn gradients_doc(&self) -> Option<&GradientsDoc> {
        self.active_doc()
    }

    fn gradients_doc_mut(&mut self) -> Option<&mut GradientsDoc> {
        self.active_doc_mut()
    }

    fn gradient_collections(&self) -> Vec<(String, String)> {
        let ok =
            self.domains.material().disk.gradients.iter().filter(|(_, c)| c.is_ok()).map(|(id, _)| id.path.clone());
        ok.map(|p| (p.clone(), p)).collect()
    }

    fn gradient_collection(&self, id: &str) -> Option<&GradientCollection> {
        let open = self.tabs.iter().filter_map(Tab::get::<GradientsDoc>).find(|g| g.id.path == id);
        open.map(|g| &g.collection).or_else(|| {
            let id = AssetId::new(AssetType::Gradients, id);
            self.domains.material().disk.gradients.get(&id).and_then(|c| c.as_ref().ok())
        })
    }

    fn texture_index(&self, id: &AssetId) -> Option<usize> {
        texture_kit::editor::index::<MaterialTexture>(self, &id.to_ref())
    }

    fn asset_index(&self, id: &AssetId) -> Option<usize> {
        self.asset_tab(&id.to_ref())
    }

    fn global_consts(&self) -> BTreeMap<String, (AssetId, GlobalConst)> {
        let mut set = AssetSet { consts: self.domains.material().disk.consts.clone(), ..Default::default() };
        for c in self.tabs.iter().filter_map(Tab::get::<ConstsDoc>) {
            set.consts.insert(c.id.clone(), Ok(c.table.clone()));
        }
        shader_project::regen::collect_consts(&set).0
    }

    fn curve_paths(&self) -> Vec<String> {
        self.domains.material().disk.curves.iter().filter(|(_, c)| c.is_ok()).map(|(id, _)| id.path.clone()).collect()
    }

    fn docs(&self) -> impl Iterator<Item = &Doc> {
        self.tabs.iter().filter_map(Tab::get::<Doc>)
    }

    fn doc_index(&self, id: &AssetId) -> Option<usize> {
        self.tabs.iter().position(|t| t.get::<Doc>().is_some_and(|d| d.id == *id))
    }

    fn text_index(&self, id: &AssetId, lang: &'static str) -> Option<usize> {
        self.tabs.iter().position(|t| t.get::<TextDoc>().is_some_and(|t| t.source == id.to_ref() && t.lang == lang))
    }

    fn image_index(&self, path: &str) -> Option<usize> {
        texture_kit::editor::image_index::<MaterialTexture>(self, path)
    }

    fn asset_kind(&self, id: &AssetId) -> Option<AssetKind> {
        self.docs().find(|d| d.id == *id).map(|d| d.asset.kind).or_else(|| kind_of(&self.domains.material().disk, id))
    }
}
