//! Image editor kind: a read-only game image (design §4.4, [`ImageDoc`]).

use bevy::prelude::*;
use shader_graph_core::texture::{AddressMode, SamplerSettings, TextureSettings};
use shader_project::{AssetId, Project};

use super::{EditorAreas, SidebarKey, fill_sidebar_with};
use crate::doc_kind::{PreviewDoc, TabDoc};
use crate::game_image::Channel;
use crate::state::{Action, Editor};
use crate::tab::{GAME_KEY_PREFIX, ImageDoc, Tab, ViewKind};
use crate::ui::icons::IconId;
use crate::ui::image_preview::PREVIEW_HOTKEYS;
use crate::ui::image_view::ImageViewBar;
use crate::ui::props::hotkey_row;
use crate::ui::widgets::subpane_box;
use crate::ui::{TEXT, TEXT_DIM, button, text};

/// The image editor's window areas.
pub struct ImageEditor;

impl EditorAreas for ImageEditor {
    const VIEW: ViewKind = ViewKind::Image;
    const SIDEBAR_TITLE: &'static str = "Image";

    /// The bar row, filled by `ui::image_view::redraw_bar`.
    fn topbar() -> Option<impl Scene> {
        Some(bsn! { @super::bar_row() ImageViewBar })
    }

    /// The file, the Texture asset pointing at it, the preview's hotkeys.
    fn sidebar() -> Option<impl Scene> {
        Some(bsn! { @super::sidebar_body() ImageSidebarBody })
    }

    fn build(app: &mut App) {
        app.add_systems(Update, redraw_sidebar);
    }
}

/// The image editor's right sidebar body.
#[derive(Component, Default, Clone)]
struct ImageSidebarBody;

fn redraw_sidebar(
    editor: Res<Editor>,
    body: Query<Entity, With<ImageSidebarBody>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some(Tab::Image(i)) = editor.tab() else { return key.forget() };
    fill_sidebar_with(&mut commands, &body, &mut key, SidebarKey::now(&editor), || image_rows(&editor, i));
}

/// A game image tab (design §4.4): the file and the Texture asset pointing at it (size and format are in the editor
/// topbar, they come from the decoded image), the preview's hotkeys.
fn image_rows(editor: &Editor, i: &ImageDoc) -> Vec<Box<dyn Scene>> {
    let mut out: Vec<Box<dyn Scene>> = Vec::new();
    out.push(Box::new(text(i.base_title(), TEXT, 15.0)));
    out.push(Box::new(text("Game image, read-only", TEXT_DIM, 12.0)));
    match editor.texture_for_file(&i.path) {
        Some(id) => out.push(Box::new(text(format!("Texture asset: {}", id.source_rel()), TEXT_DIM, 12.0))),
        None => out.push(Box::new(button("Create Texture Asset", Action::CreateTextureForFile(i.path.clone()), false))),
    }
    let hotkeys = PREVIEW_HOTKEYS.iter().map(|&(keys, what)| -> Box<dyn Scene> { Box::new(hotkey_row(keys, what)) });
    vec![subpane_box("Image", out), subpane_box("Hotkeys", hotkeys.collect())]
}

impl PreviewDoc for ImageDoc {
    /// The file with Bevy's default loader settings; tiled, with a Repeat sampler, so the 3x3 shows whether the image
    /// tiles (a game image has no sampler of its own; the default one clamps).
    fn preview_image(&self) -> (&str, TextureSettings) {
        let repeat = SamplerSettings {
            address_mode_u: AddressMode::Repeat,
            address_mode_v: AddressMode::Repeat,
            address_mode_w: AddressMode::Repeat,
            ..SamplerSettings::image_plugin_default()
        };
        (&self.path, TextureSettings { sampler: self.tiled.then_some(repeat), ..TextureSettings::default() })
    }

    fn channel(&self) -> Channel {
        self.channel
    }

    fn set_channel(&mut self, channel: Channel) {
        self.channel = channel;
    }

    fn tiled(&self) -> bool {
        self.tiled
    }

    fn set_tiled(&mut self, tiled: bool) {
        self.tiled = tiled;
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
