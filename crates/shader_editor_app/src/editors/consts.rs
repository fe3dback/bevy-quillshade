//! Consts editor kind: a global constant table (decision 0017, [`ConstsDoc`]).

use bevy::prelude::*;
use shader_graph_core::consts::GlobalConsts;
use shader_project::{AssetId, AssetSet, Project, assets, ops};

use super::{EditorAreas, SidebarKey, StatusCtx, StatusInfo, fill_sidebar};
use crate::consts_doc::ConstsDoc;
use crate::doc_kind::{TabDoc, asset_title, io_error};
use crate::state::Editor;
use crate::tab::ViewKind;
use crate::ui::consts_view::ConstsViewBar;
use crate::ui::icons::IconId;
use crate::ui::props::{hotkey_row, section};
use crate::ui::{TEXT, TEXT_DIM, text};

/// The consts editor's window areas.
pub struct ConstsEditor;

impl EditorAreas for ConstsEditor {
    const VIEW: ViewKind = ViewKind::Consts;
    const SIDEBAR_TITLE: &'static str = "Constants";

    /// The bar row, filled by `ui::consts_view::redraw`.
    fn topbar() -> Option<impl Scene> {
        Some(bsn! { @super::bar_row() ConstsViewBar })
    }

    /// What the table is and how it is used, the hotkeys.
    fn sidebar() -> Option<impl Scene> {
        Some(bsn! { @super::sidebar_body() ConstsSidebarBody })
    }

    fn build(app: &mut App) {
        app.add_systems(Update, redraw_sidebar);
    }
}

/// The consts editor's right sidebar body.
#[derive(Component, Default, Clone)]
pub struct ConstsSidebarBody;

fn redraw_sidebar(
    editor: Res<Editor>,
    body: Query<Entity, With<ConstsSidebarBody>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some(c) = editor.consts_doc() else { return key.forget() };
    let Some(body) = key.changed(&editor, &body) else { return };
    let mut rows: Vec<Box<dyn Scene>> = Vec::new();
    consts_rows(c, &mut rows);
    fill_sidebar(&mut commands, body, rows);
}

/// A constant table tab: what it is and how it is used (the table itself is the view).
fn consts_rows(c: &ConstsDoc, out: &mut Vec<Box<dyn Scene>>) {
    out.push(Box::new(text(c.id.source_rel(), TEXT, 15.0)));
    out.push(Box::new(text(format!("Type: Global Consts, {} constant(s)", c.table.consts.len()), TEXT_DIM, 12.0)));
    out.push(Box::new(section("Usage")));
    for line in [
        "Read a constant with the Global* nodes (Constants category); names are unique project-wide.",
        "Values are copied into the generated WESL as literals: saving the table regenerates the shaders reading it.",
        "bool reads as 1.0 / 0.0; compileFlag is a real bool for logic nodes (If (Bool), And, Or, Not).",
        "Changing a type or deleting a constant clears the references of the nodes reading it (after a confirmation).",
    ] {
        out.push(Box::new(text(line, TEXT_DIM, 12.0)));
    }
    out.push(Box::new(section("Hotkeys")));
    for (keys, what) in [("Ctrl+Z / Y", "Undo / redo"), ("Ctrl+S", "Save and regenerate")] {
        out.push(Box::new(hotkey_row(keys, what)));
    }
}

impl TabDoc for ConstsDoc {
    fn asset(&self) -> Option<&AssetId> {
        Some(&self.id)
    }

    fn view(&self) -> ViewKind {
        ViewKind::Consts
    }

    fn base_title(&self) -> String {
        asset_title(&self.id)
    }

    fn icon(&self) -> IconId {
        IconId::Consts
    }

    fn watermark(&self) -> Option<&'static str> {
        Some("consts")
    }

    fn is_dirty(&self) -> bool {
        ConstsDoc::is_dirty(self)
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn save(&mut self, project: &Project) -> std::io::Result<()> {
        ops::save_consts(project, &self.id, &self.table).map_err(io_error)?;
        self.mark_saved();
        Ok(())
    }

    fn changed_on_disk(&self, project: &Project) -> bool {
        assets::load_consts(project, &self.id).is_ok_and(|c| GlobalConsts::from_ron(&self.saved).ok() != Some(c))
    }

    fn reload_from_disk(&mut self, project: &Project) -> Result<(), String> {
        self.reload(assets::load_consts(project, &self.id)?);
        Ok(())
    }

    fn keep_mine(&mut self, project: &Project) {
        if let Ok(on_disk) = assets::load_consts(project, &self.id) {
            self.saved = on_disk.to_ron();
        }
    }

    fn rename(&mut self, _project: &Project, from: &AssetId, to: &AssetId) {
        if self.id != *from {
            return;
        }
        self.id = to.clone();
        self.table.name = to.name().to_string();
        self.mark_saved();
        self.revision += 1;
        self.relayout();
    }

    fn config_key(&self) -> Option<String> {
        Some(self.id.source_rel())
    }

    fn override_into(&self, set: &mut AssetSet) {
        set.consts.insert(self.id.clone(), Ok(self.table.clone()));
    }

    fn status(&self, ctx: &StatusCtx) -> Option<StatusInfo> {
        let counts = vec![format!("{} constants", self.table.consts.len())];
        Some(StatusInfo::new(ConstsDoc::is_dirty(self), ctx, counts))
    }
}
