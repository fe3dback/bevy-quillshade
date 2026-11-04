//! Game Features editor kind: a static feature enum table ([`FeaturesDoc`]).

use bevy::prelude::*;
use editor_core::action::{Handled, TabAction};
use editor_core::dynval::{PropKey, PropValue};
use editor_core::tab::ViewKindId;
use project_core::AssetRef;
use shader_graph_core::features::FeatureTable;
use shader_project::{AssetId, Project, assets, ops};

use super::{EditorAreas, SidebarKey, StatusCtx, StatusInfo, fill_sidebar_with};
use crate::doc_kind::{DocCtx, TabDoc, asset_title, io_error, project_of};
use crate::features_doc::FeaturesDoc;
use crate::ops::Editor;
use crate::ops::prelude::*;
use crate::ui::features_view::FeaturesViewBar;
use crate::ui::icons::IconRef;
use crate::ui::props::hotkey_row;
use crate::ui::widgets::subpane_box;
use crate::ui::{TEXT, TEXT_DIM, text};
use crate::util::text::plural;

/// The Game Features editor's window areas.
pub struct FeaturesEditor;

/// Game Features table: static enum definitions and variants.
pub const FEATURES: ViewKindId = ViewKindId("material.features");

impl EditorAreas for FeaturesEditor {
    const VIEW: ViewKindId = FEATURES;
    const SIDEBAR_TITLE: &'static str = "Game Features";

    fn topbar() -> Option<impl Scene> {
        Some(bsn! { @super::bar_row() FeaturesViewBar })
    }

    fn sidebar() -> Option<impl Scene> {
        Some(bsn! { @super::sidebar_body() FeaturesSidebarBody })
    }

    fn build(app: &mut App) {
        app.add_systems(Update, redraw_sidebar);
    }
}

/// The Game Features editor's right sidebar body.
#[derive(Component, Default, Clone)]
struct FeaturesSidebarBody;

fn redraw_sidebar(
    editor: Res<Editor>,
    body: Query<Entity, With<FeaturesSidebarBody>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some(f) = editor.features_doc() else { return key.forget() };
    fill_sidebar_with(&mut commands, &body, &mut key, SidebarKey::now(&editor), || features_rows(f));
}

fn features_rows(f: &FeaturesDoc) -> Vec<Box<dyn Scene>> {
    let table: Vec<Box<dyn Scene>> =
        vec![Box::new(text(f.id.source_rel(), TEXT, 15.0)), Box::new(text("Type: Game Features", TEXT_DIM, 12.0))];
    let usage = [
        "Branch on a feature enum with the Game Feature Switch node (Utility category); each variant becomes an input pin on the node.",
        "A material using M feature enums compiles N1 * ... * NM specialized shader permutations (up to 64 per material), pruning dead branches and unused textures.",
        "In the game, each enum is generated into crates/quillshade/src/material/features.rs and stored in the QuillshadeMaterialFeatures resource; mutating the resource switches material permutations automatically.",
        "The first variant of each enum is its default in the game and in the editor preview.",
    ]
    .map(|line| -> Box<dyn Scene> { Box::new(text(line, TEXT_DIM, 12.0)) });
    let hotkeys = [("Ctrl+Z / Y", "Undo / redo"), ("Ctrl+S", "Save and regenerate")]
        .map(|(keys, what)| -> Box<dyn Scene> { Box::new(hotkey_row(keys, what)) });
    vec![subpane_box("Table", table), subpane_box("Usage", usage.into()), subpane_box("Hotkeys", hotkeys.into())]
}

impl TabDoc for FeaturesDoc {
    fn apply(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        self.apply_edit(ctx, action)
    }

    fn current_value(&self, key: &PropKey, _ctx: &StatusCtx) -> Option<PropValue> {
        self.prop_of(key.downcast()?)
    }

    fn asset(&self) -> Option<&AssetRef> {
        Some(&self.aref)
    }

    fn view(&self) -> ViewKindId {
        FEATURES
    }

    fn base_title(&self) -> String {
        asset_title(&self.id)
    }

    fn icon(&self) -> IconRef {
        crate::icons::FEATURES
    }

    fn watermark(&self) -> Option<&'static str> {
        Some("features")
    }

    fn is_dirty(&self) -> bool {
        FeaturesDoc::is_dirty(self)
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn save(&mut self, ctx: &mut DocCtx) -> std::io::Result<()> {
        let project = project_of(ctx)?;
        ops::save_features(project, &self.id, &mut self.table).map_err(io_error)?;
        self.mark_saved();
        Ok(())
    }

    fn changed_on_disk(&self, project: &Project) -> bool {
        assets::load_features(project, &self.id).is_ok_and(|t| FeatureTable::from_ron(&self.saved).ok() != Some(t))
    }

    fn reload_from_disk(&mut self, ctx: &mut DocCtx) -> Result<(), String> {
        let project = ctx.project.ok_or("no project is open")?;
        self.reload(assets::load_features(project, &self.id)?);
        Ok(())
    }

    fn keep_mine(&mut self, project: &Project) {
        if let Ok(on_disk) = assets::load_features(project, &self.id) {
            self.saved = on_disk.to_ron();
        }
    }

    fn rename(&mut self, _project: &Project, from: &AssetRef, to: &AssetRef) {
        if self.aref != *from {
            return;
        }
        let Some(to_id) = AssetId::from_ref(to) else { return };
        self.aref = to.clone();
        self.id = to_id;
        self.table.name = self.id.name().to_string();
        self.mark_saved();
        self.revision += 1;
        self.relayout();
    }

    fn config_key(&self) -> Option<String> {
        Some(self.id.source_rel())
    }

    fn status(&self, ctx: &StatusCtx) -> Option<StatusInfo> {
        let counts = vec![plural(self.table.enums.len(), "enum", "enums")];
        Some(StatusInfo::new(FeaturesDoc::is_dirty(self), ctx, counts))
    }
}
