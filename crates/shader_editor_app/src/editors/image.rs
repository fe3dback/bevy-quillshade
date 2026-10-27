//! Image editor kind: a read-only game image (design §4.4, [`ImageDoc`]).

use bevy::prelude::*;
use shader_project::{AssetId, Project};

use super::{EditorAreas, SidebarKey, fill_sidebar};
use crate::doc_kind::TabDoc;
use crate::state::{Action, Editor};
use crate::tab::{GAME_KEY_PREFIX, ImageDoc, Tab, ViewKind};
use crate::ui::icons::IconId;
use crate::ui::image_view::ImageViewBar;
use crate::ui::{TEXT, TEXT_DIM, button, text};

/// The image editor's window areas.
pub struct ImageEditor;

impl EditorAreas for ImageEditor {
    const VIEW: ViewKind = ViewKind::Image;
    const SIDEBAR_TITLE: &'static str = "Image";

    /// The bar row, filled by `ui::image_view::redraw`.
    fn topbar() -> Option<impl Scene> {
        Some(bsn! { @super::bar_row() ImageViewBar })
    }

    /// The file and the Texture asset pointing at it.
    fn sidebar() -> Option<impl Scene> {
        Some(bsn! { @super::sidebar_body() ImageSidebarBody })
    }

    fn build(app: &mut App) {
        app.add_systems(Update, redraw_sidebar);
    }
}

/// The image editor's right sidebar body.
#[derive(Component, Default, Clone)]
pub struct ImageSidebarBody;

fn redraw_sidebar(
    editor: Res<Editor>,
    body: Query<Entity, With<ImageSidebarBody>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some(Tab::Image(i)) = editor.tab() else { return key.forget() };
    let Some(body) = key.changed(&editor, &body) else { return };
    let mut rows: Vec<Box<dyn Scene>> = Vec::new();
    image_rows(&editor, i, &mut rows);
    fill_sidebar(&mut commands, body, rows);
}

/// A game image tab (design §4.4): the file and the Texture asset pointing at it (size and format are in the view's
/// header, they come from the decoded image).
fn image_rows(editor: &Editor, i: &ImageDoc, out: &mut Vec<Box<dyn Scene>>) {
    out.push(Box::new(text(i.base_title(), TEXT, 15.0)));
    out.push(Box::new(text("Game image, read-only", TEXT_DIM, 12.0)));
    match editor.texture_for_file(&i.path) {
        Some(id) => out.push(Box::new(text(format!("Texture asset: {}", id.source_rel()), TEXT_DIM, 12.0))),
        None => out.push(Box::new(button("Create Texture Asset", Action::CreateTextureForFile(i.path.clone()), false))),
    }
}

impl TabDoc for ImageDoc {
    fn asset(&self) -> Option<&AssetId> {
        None
    }

    fn view(&self) -> ViewKind {
        ViewKind::Image
    }

    fn base_title(&self) -> String {
        format!("assets/{}", self.path)
    }

    fn icon(&self) -> IconId {
        IconId::Image
    }

    fn watermark(&self) -> Option<&'static str> {
        Some("image")
    }

    fn is_dirty(&self) -> bool {
        false
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn save(&mut self, _project: &Project) -> std::io::Result<()> {
        Ok(())
    }

    fn changed_on_disk(&self, _project: &Project) -> bool {
        false
    }

    fn reload_from_disk(&mut self, project: &Project) -> Result<(), String> {
        self.reload(project);
        if self.exists() { Ok(()) } else { Err(format!("assets/{} does not exist", self.path)) }
    }

    fn keep_mine(&mut self, _project: &Project) {}

    /// Game files are not assets: nothing to follow.
    fn rename(&mut self, _project: &Project, _from: &AssetId, _to: &AssetId) {}

    fn config_key(&self) -> Option<String> {
        Some(format!("{GAME_KEY_PREFIX}{}", self.path))
    }
}
