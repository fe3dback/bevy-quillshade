//! The read-only image tab of a game file (design §4.4): [`ImageDoc`], its editor kind ([`ImageEditor`]) and view.
//! The image shows in the preview shared with the Texture tab ([`super::preview`], design §6.2: zoom, pan, fit,
//! channels, Tile 3x3); the editor topbar has the path, `WxH | format | mips` ([`super::describe`]) and the preview's
//! controls, and a load / decode error is shown as text instead of the image. No sampler settings: those belong to a
//! Texture asset, which the right sidebar names or offers to create.

use std::fmt;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::Arc;

use bevy::prelude::*;
use editor_core::action::{Action, Handled, TabAction};
use editor_core::doc_kind::{DocCtx, StatusCtx, TabDoc};
use editor_core::dynval::{PropKey, PropValue};
use editor_core::editors::{EditorAreas, SidebarKey, ViewOf, bar_row, fill_sidebar_with, sidebar_body};
use editor_core::preview::{PreviewHandle, start_preview};
use editor_core::state::editor::Editor;
use editor_core::tab::ViewKindId;
use editor_core::theme::{TEXT, TEXT_DIM, text};
use editor_core::ui::button;
use editor_core::ui::icons::{IconId, IconRef};
use editor_core::ui::props::hotkey_row;
use editor_core::ui::shell::VIEW_BG;
use editor_core::ui::widgets::subpane_box;
use project_core::{AssetRef, Project};

use super::ops::{preview_edit, prop_of_key};
use super::{
    CUBEMAP_PREVIEW_KIND, Channel, CreateTextureForFile, CubeMapViewMode, FileStamp, PREVIEW_HOTKEYS, PreviewDoc,
    PreviewInfoText, TextureDomain, TextureProp, preview_area, preview_controls, texture_for_file,
};
use crate::{AddressMode, SamplerSettings, TextureSettings};

/// Prefix of a game image's tab key (`@game/textures/rock.png`); `@` never starts an asset's `quillshade/<domain>/…`
/// key.
pub const GAME_KEY_PREFIX: &str = "@game/";

/// A game image of domain `D` (design §4.4) in a read-only tab. Pure data: the view decodes the file through
/// [`super::GameImageCache`] whenever [`ImageDoc::revision`] changes.
pub struct ImageDoc<D> {
    /// Path relative to the game's `assets/` dir, `/`-separated.
    pub path: String,
    /// The file's modification time and size when last checked (`None`: missing).
    stamp: Option<FileStamp>,
    /// Bumped whenever the file changes.
    pub revision: u64,
    /// The preview shows only this channel in grey (editor-only, as on a Texture tab).
    pub channel: Channel,
    /// The preview repeats the image 3x3 (editor-only).
    pub tiled: bool,
    /// How a cube-map game image is shown in the preview (3D sphere or 2D strip; editor-only).
    pub cube_mode: CubeMapViewMode,
    /// RAII 3D cube-map preview session handle (`editor_core::preview::start_preview`).
    pub preview: Arc<PreviewHandle>,
    domain: PhantomData<fn() -> D>,
}

impl<D> Clone for ImageDoc<D> {
    fn clone(&self) -> Self {
        ImageDoc {
            path: self.path.clone(),
            stamp: self.stamp,
            revision: self.revision,
            channel: self.channel,
            tiled: self.tiled,
            cube_mode: self.cube_mode,
            preview: Arc::clone(&self.preview),
            domain: PhantomData,
        }
    }
}

impl<D> fmt::Debug for ImageDoc<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageDoc")
            .field("path", &self.path)
            .field("cube_mode", &self.cube_mode)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

impl<D> ImageDoc<D> {
    pub fn open(project: &Project, path: &str) -> Self {
        let mut doc = ImageDoc {
            path: path.to_string(),
            stamp: None,
            revision: 0,
            channel: None,
            tiled: false,
            cube_mode: CubeMapViewMode::Sphere,
            preview: Arc::new(start_preview(CUBEMAP_PREVIEW_KIND)),
            domain: PhantomData,
        };
        doc.reload(project);
        doc
    }

    /// Re-checks the file; returns whether it changed (the view then decodes it again).
    pub fn reload(&mut self, project: &Project) -> bool {
        let stamp = FileStamp::of(&project.assets_dir().join(&self.path));
        if stamp == self.stamp && self.revision > 0 {
            return false;
        }
        self.stamp = stamp;
        self.revision += 1;
        true
    }

    /// Whether the file existed when last checked.
    pub fn exists(&self) -> bool {
        self.stamp.is_some()
    }
}

impl<D: TextureDomain> PreviewDoc for ImageDoc<D> {
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

    fn cube_mode(&self) -> CubeMapViewMode {
        self.cube_mode
    }

    fn set_cube_mode(&mut self, mode: CubeMapViewMode) {
        self.cube_mode = mode;
    }

    fn preview_handle(&self) -> Option<&PreviewHandle> {
        Some(&self.preview)
    }
}

impl<D: TextureDomain> TabDoc for ImageDoc<D> {
    fn apply(&mut self, _ctx: &mut DocCtx, action: &TabAction) -> Handled {
        preview_edit::<D>(self, action)
    }

    fn current_value(&self, key: &PropKey, _ctx: &StatusCtx) -> Option<PropValue> {
        (prop_of_key::<D>(key)? == &TextureProp::PreviewTiled).then_some(PropValue::Bool(self.tiled))
    }

    fn asset(&self) -> Option<&AssetRef> {
        None
    }

    fn view(&self) -> ViewKindId {
        D::IMAGE_VIEW
    }

    fn base_title(&self) -> String {
        format!("assets/{}", self.path)
    }

    fn icon(&self) -> IconRef {
        IconId::Image.into()
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

    fn save(&mut self, _ctx: &mut DocCtx) -> std::io::Result<()> {
        Ok(())
    }

    fn changed_on_disk(&self, _project: &Project) -> bool {
        false
    }

    fn reload_from_disk(&mut self, ctx: &mut DocCtx) -> Result<(), String> {
        let project = ctx.project.ok_or("no project is open")?;
        self.reload(project);
        if self.exists() { Ok(()) } else { Err(format!("assets/{} does not exist", self.path)) }
    }

    fn keep_mine(&mut self, _project: &Project) {}

    /// Game files are not assets: nothing to follow.
    fn rename(&mut self, _project: &Project, _from: &AssetRef, _to: &AssetRef) {}

    fn config_key(&self) -> Option<String> {
        Some(format!("{GAME_KEY_PREFIX}{}", self.path))
    }

    fn preview_handle(&self) -> Option<&PreviewHandle> {
        Some(&self.preview)
    }
}

/// `D`'s image editor kind.
pub struct ImageEditor<D>(PhantomData<fn() -> D>);

marker! {
    /// The editor topbar row of `D`'s image tab (path, info, the preview's controls), rebuilt with the image's tab.
    ImageViewBar
}

marker! {
    /// The right sidebar body of `D`'s image editor.
    ImageSidebarBody
}

impl<D: TextureDomain> EditorAreas for ImageEditor<D> {
    const VIEW: ViewKindId = D::IMAGE_VIEW;
    const SIDEBAR_TITLE: &'static str = "Image";

    /// The bar row, filled by [`redraw_bar`].
    fn topbar() -> Option<impl Scene> {
        let bar = ImageViewBar::<D>::default();
        Some(bsn! { @bar_row() bar })
    }

    /// The file, the Texture asset pointing at it, the preview's hotkeys.
    fn sidebar() -> Option<impl Scene> {
        let body = ImageSidebarBody::<D>::default();
        Some(bsn! { @sidebar_body() body })
    }

    fn build(app: &mut App) {
        app.add_systems(Update, redraw_sidebar::<D>);
    }
}

/// The image tab of `D` when it is the active one.
fn active_image<D: TextureDomain>(editor: &Editor) -> Option<&ImageDoc<D>> {
    editor.active_doc()
}

fn redraw_sidebar<D: TextureDomain>(
    editor: Res<Editor>,
    body: Query<Entity, With<ImageSidebarBody<D>>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some(i) = active_image::<D>(&editor) else { return key.forget() };
    fill_sidebar_with(&mut commands, &body, &mut key, SidebarKey::now(&editor), || image_rows::<D>(&editor, i));
}

/// A game image tab (design §4.4): the file and the Texture asset pointing at it (size and format are in the editor
/// topbar, they come from the decoded image), the preview's hotkeys.
fn image_rows<D: TextureDomain>(editor: &Editor, i: &ImageDoc<D>) -> Vec<Box<dyn Scene>> {
    let mut out: Vec<Box<dyn Scene>> = Vec::new();
    out.push(Box::new(text(i.base_title(), TEXT, 15.0)));
    out.push(Box::new(text("Game image, read-only", TEXT_DIM, 12.0)));
    match texture_for_file::<D>(editor, &i.path) {
        Some(r) => out.push(Box::new(text(format!("Texture asset: {}", r.source_rel()), TEXT_DIM, 12.0))),
        None => {
            let create =
                Action::Domain(D::DOMAIN, editor_core::dynval::Payload::new(CreateTextureForFile(i.path.clone())));
            out.push(Box::new(button("Create Texture Asset", create, false)));
        }
    }
    let hotkeys = PREVIEW_HOTKEYS.iter().map(|&(keys, what)| -> Box<dyn Scene> { Box::new(hotkey_row(keys, what)) });
    vec![subpane_box("Image", out), subpane_box("Hotkeys", hotkeys.collect())]
}

/// `D`'s image tab's window systems: its editor topbar.
pub struct ImageViewPlugin<D>(PhantomData<fn() -> D>);

impl<D> Default for ImageViewPlugin<D> {
    fn default() -> Self {
        ImageViewPlugin(PhantomData)
    }
}

impl<D: TextureDomain> Plugin for ImageViewPlugin<D> {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, redraw_bar::<D>.after(editor_core::apply_actions));
    }
}

/// `D`'s image view in the editor content.
pub fn image_view<D: TextureDomain>() -> impl Scene {
    let (view, area): (_, Box<dyn Scene>) = (ViewOf(D::IMAGE_VIEW), Box::new(preview_area::<D>(D::IMAGE_VIEW)));
    bsn! {
        view
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            flex_direction: FlexDirection::Column,
        }
        BackgroundColor(VIEW_BG)
        Visibility::Hidden
        Children [ @{area} ]
    }
}

/// Rebuilds the editor topbar when another image tab is active, its file changed, its channel changed or its cube-map
/// mode changed (the buttons show the choice; Tile 3x3 shows its own state).
fn redraw_bar<D: TextureDomain>(
    editor: Res<Editor>,
    bar: Query<Entity, With<ImageViewBar<D>>>,
    mut last: Local<Option<(PathBuf, String, u64, Option<usize>, CubeMapViewMode)>>,
    mut commands: Commands,
) {
    let Ok(bar) = bar.single() else { return };
    let Some(doc) = active_image::<D>(&editor) else {
        *last = None;
        return;
    };
    let Some(project) = &editor.project else { return };
    // The root too: another project's tab of the same path may be restored without a frame in between.
    let key = (project.root.clone(), doc.path.clone(), doc.revision, doc.channel, doc.cube_mode);
    if last.as_ref() == Some(&key) {
        return;
    }
    *last = Some(key);
    commands.entity(bar).despawn_children();
    let info = PreviewInfoText::<D>::default();
    let mut controls: Vec<Box<dyn Scene>> = vec![
        Box::new(text(doc.base_title(), TEXT, 13.0)),
        Box::new(bsn! { @text("", TEXT_DIM, 12.0) info }),
        Box::new(bsn! { Node { flex_grow: 1.0 } }),
    ];
    controls.extend(preview_controls::<D>(doc));
    for c in controls {
        commands.spawn_scene(bsn! { @{c} ChildOf(bar) });
    }
}
