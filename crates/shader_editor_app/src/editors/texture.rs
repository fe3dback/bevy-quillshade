//! Texture editor kind: a Texture asset's settings, or a read-only built-in texture (decision 0024,
//! [`TextureDoc`]).

use bevy::prelude::*;
use shader_graph_core::texture::TextureAsset;
use shader_project::{AssetId, AssetSet, Project, assets, ops};

use super::{EditorAreas, SidebarKey, fill_sidebar};
use crate::doc_kind::{TabDoc, asset_title, io_error};
use crate::state::Editor;
use crate::tab::ViewKind;
use crate::texture_doc::TextureDoc;
use crate::ui::icons::IconId;
use crate::ui::props::{hotkey_row, section};
use crate::ui::texture_view::{TextureInfoText, TextureViewBar};
use crate::ui::{TEXT, TEXT_DIM, text};

/// The texture editor's window areas.
pub struct TextureEditor;

impl EditorAreas for TextureEditor {
    const VIEW: ViewKind = ViewKind::Texture;
    const SIDEBAR_TITLE: &'static str = "Texture";

    /// The bar row, filled by `ui::texture_view::redraw_form`.
    fn topbar() -> Option<impl Scene> {
        Some(bsn! { @super::bar_row() TextureViewBar Node { flex_wrap: FlexWrap::Wrap } })
    }

    /// What the texture is, its image (`TextureInfoText`, filled by `ui::texture_view::redraw_preview`), its
    /// problems, the hotkeys.
    fn sidebar() -> Option<impl Scene> {
        Some(bsn! { @super::sidebar_body() TextureSidebarBody })
    }

    fn build(app: &mut App) {
        app.add_systems(Update, redraw_sidebar);
    }
}

/// The texture editor's right sidebar body.
#[derive(Component, Default, Clone)]
pub struct TextureSidebarBody;

fn redraw_sidebar(
    editor: Res<Editor>,
    body: Query<Entity, With<TextureSidebarBody>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some(t) = editor.texture_doc() else { return key.forget() };
    let Some(body) = key.changed(&editor, &body) else { return };
    let mut rows: Vec<Box<dyn Scene>> = Vec::new();
    texture_rows(&editor, t, &mut rows);
    fill_sidebar(&mut commands, body, rows);
}

/// Hotkeys of the Texture tab's preview.
const TEXTURE_HOTKEYS: [(&str, &str); 5] = [
    ("Wheel", "Zoom the preview"),
    ("Drag", "Pan the preview"),
    ("Double-click", "Fit the preview"),
    ("Ctrl+Z / Y", "Undo / redo"),
    ("Ctrl+S", "Save and regenerate"),
];

/// A Texture tab (design §4.4): what it is, its image, its problems (the form and preview are the view).
fn texture_rows(editor: &Editor, t: &TextureDoc, out: &mut Vec<Box<dyn Scene>>) {
    out.push(Box::new(text(t.base_title(), TEXT, 15.0)));
    let kind = if t.is_internal() { "Built-in texture, read-only" } else { "Type: Texture asset" };
    out.push(Box::new(text(kind, TEXT_DIM, 12.0)));
    out.push(Box::new(text(format!("Image: assets/{}", t.asset.path), TEXT_DIM, 12.0)));
    out.push(Box::new(bsn! { @text("", TEXT_DIM, 12.0) TextureInfoText }));
    if !t.is_internal() {
        out.push(Box::new(text(
            "Settings reach the game through the baked manifest (hot-reloaded); sRGB and grayscale expansion need a \
             rebuild.",
            TEXT_DIM,
            12.0,
        )));
    }
    if let Some(diags) = editor.diags.get(&t.id).filter(|d| !d.is_empty() && !t.is_internal()) {
        out.push(Box::new(section("Problems")));
        for diag in diags {
            let c = if diag.is_error() { Color::srgb(0.95, 0.42, 0.42) } else { Color::srgb(0.95, 0.78, 0.35) };
            out.push(Box::new(text(format!("{}: {}", diag.code, diag.message), c, 12.0)));
        }
    }
    out.push(Box::new(section("Hotkeys")));
    let hotkeys = if t.is_internal() { &TEXTURE_HOTKEYS[..3] } else { &TEXTURE_HOTKEYS[..] };
    for (keys, what) in hotkeys {
        out.push(Box::new(hotkey_row(keys, what)));
    }
}

impl TabDoc for TextureDoc {
    fn asset(&self) -> Option<&AssetId> {
        (!self.is_internal()).then_some(&self.id)
    }

    fn view(&self) -> ViewKind {
        ViewKind::Texture
    }

    fn base_title(&self) -> String {
        if self.is_internal() { self.id.path.clone() } else { asset_title(&self.id) }
    }

    fn icon(&self) -> IconId {
        IconId::Image
    }

    fn watermark(&self) -> Option<&'static str> {
        Some("texture")
    }

    fn is_dirty(&self) -> bool {
        TextureDoc::is_dirty(self)
    }

    fn is_read_only(&self) -> bool {
        self.is_internal()
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn save(&mut self, project: &Project) -> std::io::Result<()> {
        if self.is_internal() {
            return Ok(());
        }
        ops::save_texture(project, &self.id, &self.asset).map_err(io_error)?;
        self.mark_saved();
        Ok(())
    }

    fn changed_on_disk(&self, project: &Project) -> bool {
        !self.is_internal()
            && assets::load_texture(project, &self.id)
                .is_ok_and(|t| TextureAsset::from_ron(&self.saved).ok() != Some(t))
    }

    /// A built-in texture never changes (read-only documents are "reloaded" on activation).
    fn reload_from_disk(&mut self, project: &Project) -> Result<(), String> {
        if !self.is_internal() {
            self.reload(assets::load_texture(project, &self.id)?);
        }
        Ok(())
    }

    fn keep_mine(&mut self, project: &Project) {
        if self.is_internal() {
            return;
        }
        if let Ok(on_disk) = assets::load_texture(project, &self.id) {
            self.saved = on_disk.to_ron();
        }
    }

    fn rename(&mut self, _project: &Project, from: &AssetId, to: &AssetId) {
        if self.is_internal() || self.id != *from {
            return;
        }
        self.id = to.clone();
        self.revision += 1;
        self.relayout();
    }

    /// `shaders/….texture.ron`, or the `@internal/…` id of a built-in texture.
    fn config_key(&self) -> Option<String> {
        Some(if self.is_internal() { self.id.path.clone() } else { self.id.source_rel() })
    }

    fn override_into(&self, set: &mut AssetSet) {
        if !self.is_internal() {
            set.textures.insert(self.id.clone(), Ok(self.asset.clone()));
        }
    }
}
