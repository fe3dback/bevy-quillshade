//! The Material domain's editor tests: `Editor` driven synchronously with Material registered
//! ([`crate::test_editor`]) on the sample fixtures.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use editor_core::action::TabAction;
use editor_core::dynval::{Payload, PropKey, PropTarget, PropValue};
use shader_graph_core::consts::ConstType;
use shader_graph_core::curve::Interp;
use shader_graph_core::gradient::{MAX_STOPS, Stop};
use shader_graph_core::model::{Asset, AssetKind, MaterialKind, NodeId, PbrOutputMode, PreviewMode};
use shader_graph_core::registry::{Conversion, Hotkey, HotkeyKey, const_refs};
use shader_graph_core::{Code, Diagnostic, PinType, Value};
use shader_project::layout::MATERIAL;
use shader_project::regen::{rust_file, wesl_file};
use shader_project::{AssetId, AssetType, assets, ops};

use crate::curve::{CurveClipboard, CurveDoc};
use crate::doc::{Doc, connection_error, node_signature};
use crate::gradients_doc::{GradientSel, GradientsDoc};
use crate::material_state::{MaterialState, MaterialStates};
use crate::search::{self, WireEnd};
use crate::tab::{ImageDoc, MaterialKey, RUST, Tab, TextDoc, WESL, parse_config_key};
use crate::texture::TextureField;

use crate::config::UserConfig;
use crate::ops::prelude::*;
use crate::ops::*;
use editor_core::tab::TabKey;
use project_core::DomainId;
use shader_graph_core::ir::WPO_PIN;
use shader_project::Project;
use shader_project::fixtures;

/// The model value a property widget of `target` shows on the active tab ([`Editor::current_value`]).
fn shown(e: &Editor, target: &(impl PropTarget + Clone)) -> Option<Value> {
    e.current_value(&PropKey::new(target.clone())).map(|v| model_value(&v))
}

/// The tab key of an asset's editable tab (its source path, as `TabDoc::config_key` gives it).
fn asset_key(id: &AssetId) -> TabKey {
    let r = id.to_ref();
    TabKey::new(r.domain, r.source_rel())
}

/// The "New …" item of Material's asset kind `suffix`, template `template`.
fn new_asset(suffix: &'static str, template: usize) -> Action {
    Action::NewAssetDialog { kind: (MATERIAL, suffix), template }
}

fn project() -> (tempfile::TempDir, Editor) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    let p = Project::open(dir.path()).unwrap();
    fixtures::save_all(&p).unwrap();
    let mut e = crate::test_editor();
    e.open_project(dir.path()).unwrap();
    (dir, e)
}

mod consts;
mod curves;
mod domain;
mod drop;
mod gradients;
mod graph;
mod nav;
mod palette;
mod project;
mod tab_docs;
mod textures;
mod wires;

use consts::*;
use curves::*;
use drop::*;
use graph::*;
use textures::*;
