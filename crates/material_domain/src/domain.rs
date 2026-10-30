//! The Material domain's registration (design §6.1, §9.1): its asset kinds and their "New …" templates, its seven
//! editor kinds and their views (the graph kind `node_graph_base`'s, curves `curve_kit`'s, Texture and image tabs with
//! the Game Assets images and the virtual Internal folder `texture_kit`'s), its tab keys, its Quick Start steps and its
//! Bevy systems.

use std::io;
use std::sync::Arc;

use bevy::prelude::*;
use editor_core::action::Action;
use editor_core::menu::MenuEntry;
use editor_core::ui::icons::{IconId, IconRef};
use project_core::regen::DomainBackend;
use project_core::{AssetRef, DomainId, Project};
use shader_graph_core::model::{MaterialKind, PbrSettings, SpriteSettings, UnlitSettings};
use shader_project::backend::MaterialBackend;
use shader_project::layout::MATERIAL;
use shader_project::{AssetId, AssetType, ops};

use crate::curve::MaterialCurve;
use crate::editors::{ConstsEditor, GradientsEditor};
use crate::graph_domain::MaterialGraph;
use crate::icons;
use crate::material_state::MaterialState;
use crate::ops::MaterialAction;
use crate::ops::MaterialEditor;
use crate::tab::{RUST, WESL};
use crate::texture::MaterialTexture;
use crate::ui;
use editor_core::domain::{
    AssetKindDef, AssetTemplate, DomainInfo, DomainList, DomainRegistrar, EditorDomain, TreeAsset,
};
use editor_core::state::editor::Editor;

/// The Material domain.
pub struct MaterialDomain;

impl EditorDomain for MaterialDomain {
    fn id(&self) -> DomainId {
        MATERIAL
    }

    fn info(&self) -> DomainInfo {
        DomainInfo {
            title: "Material",
            icon: icons::PBR,
            summary: "Material — node-based materials in quillshade/material/, generated into \
                      assets/quillshade/material/ and crates/quillshade.",
        }
    }

    /// The CLI's generator: it finds the Bevy sources for `--compile` itself.
    fn backend(&self) -> Option<Box<dyn DomainBackend>> {
        Some(Box::new(MaterialBackend::discovering()))
    }

    fn register(&self, r: &mut DomainRegistrar) {
        r.state(|| Box::new(MaterialState::default()));
        // The graph editor kind, its canvas view and minimap (`node_graph_base`).
        node_graph_base::register_graph::<MaterialGraph>(r);
        // The asset kinds in the Assets menu's order: curves (`curve_kit`: the kind, its editor kind and view) third,
        // Texture assets (`texture_kit`: the kind, the image and Texture editor kinds and views, the Game Assets
        // images, the Internal folder of Material's built-in textures) after the constant tables.
        let [graphs, consts, gradients] = asset_kinds();
        for kind in graphs {
            r.asset_kind(kind);
        }
        curve_kit::editor::register::<MaterialCurve>(r);
        for kind in consts {
            r.asset_kind(kind);
        }
        texture_kit::editor::register::<MaterialTexture>(r);
        for kind in gradients {
            r.asset_kind(kind);
        }
        r.editor_kind::<ConstsEditor>()
            .editor_kind::<GradientsEditor>()
            .parse_tab(crate::ops::parse_tab)
            .tab_exists(crate::ops::tab_exists)
            .legacy_tab_keys()
            .quick_start(crate::quick_start::QUICK_START)
            .highlighter(crate::tab::WESL, crate::highlight_wesl::highlight_wesl)
            .logic(crate::material_logic)
            .bevy(|app| {
                app.add_systems(Startup, discover_bevy_sources);
            })
            .bevy(crate::material_window)
            .doc_view(|| Box::new(ui::consts_view::consts_view()))
            .doc_view(|| Box::new(ui::gradient_view::gradient_view()));
    }
}

/// A list holding only the Material domain (tests, tools).
pub fn material_only() -> Arc<DomainList> {
    Arc::new(DomainList::new(vec![Box::new(MaterialDomain)]))
}

/// An editor with the Material domain registered (the Material tests, tools).
pub fn test_editor() -> Editor {
    Editor::new(material_only())
}

/// Finds the Bevy 0.20 WESL sources for the compile check of generated WESL, once at startup.
fn discover_bevy_sources(mut editor: ResMut<Editor>) {
    let found = shader_codegen_wesl::check::BevySources::discover();
    let missing = found.is_none();
    if let Some(state) = editor.domains.get_mut::<MaterialState>() {
        state.bevy = found;
    }
    if missing {
        let text = "Bevy 0.20 WESL sources not found in the cargo registry: generated WESL is not compile-checked";
        editor.domain_msg(Some(MATERIAL), editor_core::message::Level::Info, text);
    }
}

fn template(label: &'static str, base_name: &'static str, icon: IconRef, create: CreateFn) -> AssetTemplate {
    AssetTemplate { label, base_name, icon, create }
}

type CreateFn = fn(&Project, &AssetRef) -> io::Result<()>;

/// An `ops` creation result as the template's.
fn created<T, E: std::error::Error + Send + Sync + 'static>(r: Result<T, E>) -> io::Result<()> {
    r.map(drop).map_err(io::Error::other)
}

fn create_material(p: &Project, r: &AssetRef, kind: MaterialKind) -> io::Result<()> {
    created(ops::create_material(p, &r.path, kind))
}

/// Material's own asset kinds, in the order of their templates in the Assets menu: the graphs, then the constant
/// tables after the curves (`curve_kit`'s), then the gradient collections after the Texture assets (`texture_kit`'s,
/// without templates: they are made from game images).
fn asset_kinds() -> [Vec<AssetKindDef>; 3] {
    let kind = |ty: AssetType, title, icon, templates| AssetKindDef {
        suffix: ty.suffix(),
        title,
        icon,
        templates,
        open: crate::ops::asset_tab,
        row_menu: asset_row_menu,
    };
    let graphs = vec![
        kind(
            AssetType::Material,
            "Material",
            icons::PBR,
            vec![
                template("Material (Pbr3d)", "new_material", icons::PBR, |p, r| {
                    create_material(p, r, MaterialKind::Pbr3d(PbrSettings::default()))
                }),
                template("Material (Unlit3d)", "new_material", icons::UNLIT, |p, r| {
                    create_material(p, r, MaterialKind::Unlit3d(UnlitSettings::default()))
                }),
                template("Material (Sprite2d)", "new_material", icons::sprite(), |p, r| {
                    create_material(p, r, MaterialKind::Sprite2d(SpriteSettings::default()))
                }),
            ],
        ),
        kind(
            AssetType::Function,
            "Function",
            icons::FUNCTION,
            vec![template("Function", "new_function", icons::FUNCTION, |p, r| {
                created(ops::create_function(p, &r.path))
            })],
        ),
    ];
    let consts = vec![kind(
        AssetType::Consts,
        "Global Consts",
        icons::CONSTS,
        vec![template("Global Consts", "new_consts", icons::CONSTS, |p, r| created(ops::create_consts(p, &r.path)))],
    )];
    let gradients = vec![kind(
        AssetType::Gradients,
        "Gradient Collection",
        icons::GRADIENTS,
        vec![template("Gradient Collection", "new_gradients", icons::GRADIENTS, |p, r| {
            created(ops::create_gradients(p, &r.path))
        })],
    )];
    [graphs, consts, gradients]
}

/// What a Material asset's row menu adds after Open / Rename / Move / Delete: its generated files and its source.
pub(crate) fn asset_row_menu(_editor: &Editor, r: &AssetRef) -> Vec<MenuEntry> {
    let Some(id) = AssetId::from_ref(r) else { return Vec::new() };
    let mut items: Vec<(Option<IconRef>, String, Action)> = Vec::new();
    if id.ty.has_wesl() {
        items.push((
            Some(IconId::Code.into()),
            "Show generated WESL".into(),
            MaterialAction::ShowGenerated(id.clone(), WESL).into(),
        ));
    }
    if id.ty == AssetType::Material {
        items.push((
            Some(IconId::Code.into()),
            "Show generated Rust".into(),
            MaterialAction::ShowGenerated(id.clone(), RUST).into(),
        ));
    }
    items.push((Some(IconId::File.into()), "Show source (.ron)".into(), MaterialAction::ShowFile(id, None).into()));
    items.into_iter().map(MenuEntry::from).collect()
}

impl MaterialState {
    /// The Project tree's rows of the domain: every asset on disk, with the icon of its kind (an open graph's kind as
    /// edited) and the error colour while its live diagnostics have an error.
    pub(crate) fn project_tree_items(&self, editor: &Editor) -> Vec<TreeAsset> {
        let item = |id: &AssetId| TreeAsset {
            asset: id.to_ref(),
            icon: icons::for_id(id, editor.asset_kind(id)),
            error: self.diags.get(id).is_some_and(|v| v.iter().any(|d| d.is_error())),
        };
        let mut ids: Vec<&AssetId> = self.disk.ids().collect();
        ids.sort();
        ids.into_iter().map(item).collect()
    }
}
