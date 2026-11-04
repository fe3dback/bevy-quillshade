//! The `Texture Converter` tool (`Tools -> Texture Converter`): imports a 2D `.png`, `.jpg`/`.jpeg`, or `.ktx2` image,
//! previews both the source thumbnail and the transformed / channel-filtered output (`All | R | G | B | A`), lets the
//! user configure output format (`KTX2` or `PNG`), resolution preset, vertical flip (`Flip Y`), KTX2 color space
//! (`sRGB` / `Linear`), mipmap generation, and Zstd supercompression, and exports a standalone `.ktx2` or `.png` file
//! directly to disk without creating a `.texture.ron` asset.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::feathers::containers::{subpane, subpane_body, subpane_header};
use bevy::feathers::controls::{ButtonVariant, FeathersToolButton};
use bevy::feathers::display::caption;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::InteractionDisabled;
use editor_core::action::{Action, Handled, TabAction};
use editor_core::doc_kind::{DocCtx, StatusCtx, StatusInfo, TabDoc};
use editor_core::domain::ToolId;
use editor_core::dynval::{PropKey, PropTarget, PropValue};
use editor_core::editors::{EditorAreas, SidebarKey, ViewOf, bar_row, fill_sidebar_with, sidebar_body};
use editor_core::message::Level;
use editor_core::state::editor::Editor;
use editor_core::tab::ViewKindId;
use editor_core::theme::{ACCENT, BORDER, TEXT, TEXT_DIM, text};
use editor_core::tooltip::Tooltip;
use editor_core::ui::icons::{IconId, IconRef, icon};
use editor_core::ui::props::{checkbox, checkbox_row, labeled, select_values};
use editor_core::ui::shell::VIEW_BG;
use editor_core::ui::widgets::{Caption, DISABLED_ICON_ALPHA, GroupButton, button_group, scroll_column, subpane_box};
use editor_core::ui::{OnActivate, icon_tool_button};
use project_core::{AssetRef, Project};
use shader_project::layout::MATERIAL;

use crate::texture_tools_common::{
    Ktx2ColorSpace, OutputFormat, PreviewChannel, ResizePreset, RgbaImage2d, TextureExportSettings,
    demo_converter_image, encode_texture, prepare_output_image, rgba_to_bevy_image,
};

/// The `Texture Converter` tool's id in the Material domain.
pub const TEXTURE_CONVERTER_TOOL: ToolId = "texture_converter";

/// The `Texture Converter` tool tab's view.
pub const TEXTURE_CONVERTER_VIEW: ViewKindId = ViewKindId("material.texture_converter");

/// A property widget target in the `Texture Converter` tool.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureConverterProp {
    OutputFormat,
    Resize,
    FlipY,
    Ktx2ColorSpace,
    Mipmaps,
    Zstd,
}

impl PropTarget for TextureConverterProp {}

/// A command dispatched to the active [`TextureConverterDoc`].
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureConverterAction {
    ImportImage,
    ExportTexture,
    SetPreviewChannel(PreviewChannel),
}

impl From<TextureConverterAction> for Action {
    fn from(a: TextureConverterAction) -> Self {
        Action::Tab(TabAction::doc(a))
    }
}

/// The `Texture Converter` tool tab's document.
pub struct TextureConverterDoc {
    /// Path of the imported `.png` or `.ktx2` file on disk (`None` before import).
    pub source_path: Option<PathBuf>,
    /// Decoded RGBA8 source image.
    pub source_image: Option<Arc<RgbaImage2d>>,
    /// Export settings (format, resize preset, flip Y, KTX2 color space, mipmaps, Zstd).
    pub settings: TextureExportSettings,
    /// Active channel filter for the converted output preview (`All | R | G | B | A`).
    pub preview_channel: PreviewChannel,
    /// Last directory visited by the import or export dialog.
    pub last_dir: Option<PathBuf>,
    /// Last file path written by `export_to_path`.
    pub last_export_path: Option<PathBuf>,
    /// Set by [`TextureConverterAction::ImportImage`] so the file-dialog polling system opens a native file picker.
    pub pick_import: bool,
    /// Set by [`TextureConverterAction::ExportTexture`] so the file-dialog polling system opens a native save picker.
    pub pick_export: bool,
    /// Bumped whenever source image, settings, or preview channel changes so the UI and preview cache know when to
    /// rebuild.
    pub revision: u64,
}

impl Default for TextureConverterDoc {
    fn default() -> Self {
        Self::new()
    }
}

impl TextureConverterDoc {
    pub fn new() -> Self {
        Self {
            source_path: None,
            source_image: None,
            settings: TextureExportSettings::default(),
            preview_channel: PreviewChannel::All,
            last_dir: None,
            last_export_path: None,
            pick_import: false,
            pick_export: false,
            revision: 1,
        }
    }

    /// Loads the procedural demo texture (`demo_texture.png`) and increments `revision`.
    pub fn load_demo(&mut self) {
        let img = demo_converter_image();
        self.settings.ktx2_color_space = if img.source_is_srgb { Ktx2ColorSpace::Srgb } else { Ktx2ColorSpace::Linear };
        self.source_path = Some(PathBuf::from("demo_texture.png"));
        self.source_image = Some(Arc::new(img));
        self.revision += 1;
    }

    /// Loads a 2D `.png` or `.ktx2` file from `path`, auto-configures `settings.ktx2_color_space` from the source
    /// image's color space, and increments `revision`.
    pub fn import_from_path(&mut self, path: &Path) -> Result<(), String> {
        let img = RgbaImage2d::from_file(path)?;
        self.apply_imported(path.to_path_buf(), img);
        Ok(())
    }

    /// Decodes a 2D `.png` or `.ktx2` byte buffer associated with `path`, auto-configures `settings.ktx2_color_space`
    /// from the source image's color space, and increments `revision`.
    pub fn import_from_bytes(&mut self, path: PathBuf, bytes: &[u8]) -> Result<(), String> {
        let img = RgbaImage2d::from_bytes(bytes, &path)?;
        self.apply_imported(path, img);
        Ok(())
    }

    fn apply_imported(&mut self, path: PathBuf, img: RgbaImage2d) {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            self.last_dir = Some(parent.to_path_buf());
        }
        self.settings.ktx2_color_space = if img.source_is_srgb { Ktx2ColorSpace::Srgb } else { Ktx2ColorSpace::Linear };
        self.source_path = Some(path);
        self.source_image = Some(Arc::new(img));
        self.revision += 1;
    }

    /// Encodes the loaded `source_image` according to `self.settings` into `.ktx2` or `.png` bytes.
    pub fn export_bytes(&self) -> Result<Vec<u8>, String> {
        let Some(src) = &self.source_image else {
            return Err("no source image loaded".to_string());
        };
        encode_texture(src, &self.settings)
    }

    /// Encodes the loaded `source_image` and writes the resulting bytes to `path` (creating parent directories if
    /// needed), updating `last_export_path` and `last_dir`.
    pub fn export_to_path(&mut self, path: &Path) -> Result<usize, String> {
        let bytes = self.export_bytes()?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            self.last_dir = Some(parent.to_path_buf());
        }
        std::fs::write(path, &bytes).map_err(|e| e.to_string())?;
        self.last_export_path = Some(path.to_path_buf());
        self.revision += 1;
        Ok(bytes.len())
    }

    /// Default export file name derived from `source_path`'s stem (or `"converted_texture"`) and
    /// `self.settings.output_format.extension()`.
    pub fn suggested_export_filename(&self) -> String {
        let stem = self
            .source_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("converted_texture");
        format!("{stem}.{}", self.settings.output_format.extension())
    }

    /// Human-readable summary of the source image (`"rock_albedo.png (1024x1024, Rgba8UnormSrgb, 1 mip)"` or
    /// `"No image loaded"`).
    pub fn source_summary(&self) -> String {
        let Some(src) = &self.source_image else {
            return "No image loaded".to_string();
        };
        let name = self
            .source_path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("image");
        let mip_word = if src.source_mips == 1 { "mip" } else { "mips" };
        format!("{name} ({}x{}, {}, {} {mip_word})", src.width, src.height, src.source_format, src.source_mips)
    }

    /// Human-readable summary of the target output (`"1024x1024 | KTX2 | RGBA8 (sRGB) | 11 mips | Zstd"` or
    /// `"No output (import a .png, .jpg, or .ktx2 image first)"`).
    pub fn output_summary(&self) -> String {
        let Some(src) = &self.source_image else {
            return "No output (import a .png, .jpg, or .ktx2 image first)".to_string();
        };
        self.settings.output_summary(src.width, src.height)
    }
}

/// Opens the `Texture Converter` tool tab.
pub fn open_texture_converter_tool(_editor: &mut Editor) -> Box<dyn TabDoc> {
    Box::new(TextureConverterDoc::new())
}

impl TabDoc for TextureConverterDoc {
    fn apply(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        match action {
            TabAction::SetValue { key, value, .. } => {
                let Some(prop) = key.downcast::<TextureConverterProp>() else { return Handled::No };
                match prop {
                    TextureConverterProp::OutputFormat => {
                        if let Some(fmt) = OutputFormat::from_label(value.as_str())
                            && self.settings.output_format != fmt
                        {
                            self.settings.output_format = fmt;
                            self.revision += 1;
                            ctx.redraw_props = true;
                        }
                    }
                    TextureConverterProp::Resize => {
                        if let Some(preset) = ResizePreset::from_label(value.as_str())
                            && self.settings.resize != preset
                        {
                            self.settings.resize = preset;
                            self.revision += 1;
                            ctx.redraw_props = true;
                        }
                    }
                    TextureConverterProp::FlipY => {
                        let v = value.as_bool();
                        if self.settings.flip_y != v {
                            self.settings.flip_y = v;
                            self.revision += 1;
                            ctx.redraw_props = true;
                        }
                    }
                    TextureConverterProp::Ktx2ColorSpace => {
                        if let Some(cs) = Ktx2ColorSpace::from_label(value.as_str())
                            && self.settings.ktx2_color_space != cs
                        {
                            self.settings.ktx2_color_space = cs;
                            self.revision += 1;
                            ctx.redraw_props = true;
                        }
                    }
                    TextureConverterProp::Mipmaps => {
                        let v = value.as_bool();
                        if self.settings.mipmaps != v {
                            self.settings.mipmaps = v;
                            self.revision += 1;
                            ctx.redraw_props = true;
                        }
                    }
                    TextureConverterProp::Zstd => {
                        let v = value.as_bool();
                        if self.settings.zstd != v {
                            self.settings.zstd = v;
                            self.revision += 1;
                            ctx.redraw_props = true;
                        }
                    }
                }
            }
            TabAction::Doc(payload) => {
                let Some(act) = payload.downcast::<TextureConverterAction>() else { return Handled::No };
                match act {
                    TextureConverterAction::ImportImage => {
                        self.pick_import = true;
                    }
                    TextureConverterAction::ExportTexture => {
                        if self.source_image.is_some() {
                            self.pick_export = true;
                        }
                    }
                    TextureConverterAction::SetPreviewChannel(ch) => {
                        if self.preview_channel != *ch {
                            self.preview_channel = *ch;
                            self.revision += 1;
                            ctx.redraw_props = true;
                        }
                    }
                }
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn current_value(&self, key: &PropKey, _ctx: &StatusCtx) -> Option<PropValue> {
        match *key.downcast::<TextureConverterProp>()? {
            TextureConverterProp::OutputFormat => Some(PropValue::Str(self.settings.output_format.label().to_string())),
            TextureConverterProp::Resize => Some(PropValue::Str(self.settings.resize.label().to_string())),
            TextureConverterProp::FlipY => Some(PropValue::Bool(self.settings.flip_y)),
            TextureConverterProp::Ktx2ColorSpace => {
                Some(PropValue::Str(self.settings.ktx2_color_space.label().to_string()))
            }
            TextureConverterProp::Mipmaps => Some(PropValue::Bool(self.settings.mipmaps)),
            TextureConverterProp::Zstd => Some(PropValue::Bool(self.settings.zstd)),
        }
    }

    fn asset(&self) -> Option<&AssetRef> {
        None
    }

    fn view(&self) -> ViewKindId {
        TEXTURE_CONVERTER_VIEW
    }

    fn base_title(&self) -> String {
        "Texture Converter".into()
    }

    fn icon(&self) -> IconRef {
        IconId::Image.into()
    }

    fn watermark(&self) -> Option<&'static str> {
        Some("Texture Converter")
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

    fn reload_from_disk(&mut self, _ctx: &mut DocCtx) -> Result<(), String> {
        Ok(())
    }

    fn keep_mine(&mut self, _project: &Project) {}

    fn rename(&mut self, _project: &Project, _from: &AssetRef, _to: &AssetRef) {}

    fn config_key(&self) -> Option<String> {
        None
    }

    fn tool(&self) -> Option<ToolId> {
        Some(TEXTURE_CONVERTER_TOOL)
    }

    fn status(&self, ctx: &StatusCtx) -> Option<StatusInfo> {
        let mut counts = if let Some(src) = &self.source_image {
            let name = self
                .source_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("image");
            vec![format!("Source: {name} ({}x{})", src.width, src.height), self.output_summary()]
        } else {
            vec!["No image loaded".to_string()]
        };
        if let Some(exported) = &self.last_export_path {
            let name = exported.file_name().and_then(|s| s.to_str()).unwrap_or("exported");
            counts.push(format!("Exported: {name}"));
        }
        Some(StatusInfo::new(false, ctx, counts))
    }
}

/// The `Texture Converter` editor kind.
pub struct TextureConverterEditor;

/// The editor topbar row of the `Texture Converter` tab.
#[derive(Component, Default, Clone)]
pub struct TextureConverterBar;

/// The right sidebar body of the `Texture Converter` tab.
#[derive(Component, Default, Clone)]
pub struct TextureConverterSidebarBody;

/// The left column container (`Source Image` subpane + `Output Settings` subpane).
#[derive(Component, Default, Clone)]
pub struct TextureConverterLeftPane;

/// The right column container (`Converted Output Preview` subpane + footer).
#[derive(Component, Default, Clone)]
pub struct TextureConverterRightPane;

/// Checkerboard frame holding the source image thumbnail.
#[derive(Component, Default, Clone)]
pub struct TextureConverterThumbFrame;

/// The UI [`ImageNode`] displaying the source thumbnail.
#[derive(Component, Default, Clone)]
pub struct TextureConverterThumbImage;

/// The large 2D preview viewport container in the right pane.
#[derive(Component, Default, Clone)]
pub struct TextureConverterPreviewBox;

/// Checkerboard frame inside [`TextureConverterPreviewBox`] sized to preserve the output image's aspect ratio.
#[derive(Component, Default, Clone)]
pub struct TextureConverterOutputFrame;

/// The UI [`ImageNode`] displaying the transformed and channel-filtered output preview.
#[derive(Component, Default, Clone)]
pub struct TextureConverterOutputImage;

/// Caches the source thumbnail [`Image`], the transformed/channel-filtered output preview [`Image`], and a 2x2
/// checkerboard background [`Image`] for the active [`TextureConverterDoc`].
#[derive(Resource, Default)]
pub struct TextureConverterPreviewCache {
    pub key: Option<(usize, u64, PreviewChannel)>,
    pub source_thumb: Option<Handle<Image>>,
    pub output_preview: Option<Handle<Image>>,
    pub checker_handle: Option<Handle<Image>>,
}

impl TextureConverterPreviewCache {
    /// Removes cached `source_thumb` and `output_preview` handles from `images`.
    pub fn clear_images(&mut self, images: &mut Assets<Image>) {
        self.key = None;
        if let Some(h) = self.source_thumb.take() {
            images.remove(h.id());
        }
        if let Some(h) = self.output_preview.take() {
            images.remove(h.id());
        }
    }

    /// Returns the 2x2 nearest-sampled checkerboard [`Handle<Image>`], creating it on first use.
    pub fn ensure_checker(&mut self, images: &mut Assets<Image>) -> Handle<Image> {
        if let Some(h) = &self.checker_handle
            && images.contains(h.id())
        {
            return h.clone();
        }
        let (a, b) = ([58, 59, 64, 255], [44, 45, 50, 255]);
        let mut image = Image::new(
            Extent3d { width: 2, height: 2, depth_or_array_layers: 1 },
            TextureDimension::D2,
            [a, b, b, a].concat(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        image.sampler = ImageSampler::nearest();
        let handle = images.add(image);
        self.checker_handle = Some(handle.clone());
        handle
    }

    /// Returns `(source_thumb, output_preview, checker_handle)`, rebuilding the GPU images only when
    /// `(tab_id, doc.revision, doc.preview_channel)` changes.
    pub fn sync(
        &mut self,
        tab_id: usize,
        doc: &TextureConverterDoc,
        images: &mut Assets<Image>,
    ) -> Option<(Handle<Image>, Handle<Image>, Handle<Image>)> {
        let Some(src) = &doc.source_image else {
            self.clear_images(images);
            return None;
        };
        let checker = self.ensure_checker(images);
        let want_key = (tab_id, doc.revision, doc.preview_channel);
        if self.key == Some(want_key)
            && let (Some(thumb), Some(out)) = (&self.source_thumb, &self.output_preview)
            && images.contains(thumb.id())
            && images.contains(out.id())
        {
            return Some((thumb.clone(), out.clone(), checker));
        }

        self.clear_images(images);
        let thumb_img = rgba_to_bevy_image(src, PreviewChannel::All);
        let prepared = prepare_output_image(src, doc.settings.resize, doc.settings.flip_y);
        let out_img = rgba_to_bevy_image(&prepared, doc.preview_channel);

        let thumb_handle = images.add(thumb_img);
        let out_handle = images.add(out_img);
        self.key = Some(want_key);
        self.source_thumb = Some(thumb_handle.clone());
        self.output_preview = Some(out_handle.clone());
        Some((thumb_handle, out_handle, checker))
    }
}

impl EditorAreas for TextureConverterEditor {
    const VIEW: ViewKindId = TEXTURE_CONVERTER_VIEW;
    const SIDEBAR_TITLE: &'static str = "Texture Converter";

    fn topbar() -> Option<impl Scene> {
        Some(bsn! { @bar_row() TextureConverterBar })
    }

    fn sidebar() -> Option<impl Scene> {
        Some(bsn! { @sidebar_body() TextureConverterSidebarBody })
    }

    fn build(app: &mut App) {
        app.init_resource::<TextureConverterPreviewCache>().add_systems(
            Update,
            (
                redraw_bar,
                redraw_left,
                redraw_right,
                redraw_sidebar,
                ApplyDeferred,
                update_texture_converter_preview,
                poll_texture_converter_dialogs,
            )
                .chain()
                .after(editor_core::apply_actions),
        );
    }
}

fn active(editor: &Editor) -> Option<(usize, &TextureConverterDoc)> {
    let i = editor.active?;
    Some((i, editor.tabs.get(i)?.get::<TextureConverterDoc>()?))
}

/// The `Texture Converter` 2-column view in the editor content (`{left: Source Image + Output Settings} | {right: Converted Output Preview}`).
pub fn texture_converter_view() -> impl Scene {
    bsn! {
        ViewOf(TEXTURE_CONVERTER_VIEW)
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            flex_direction: FlexDirection::Row,
            padding: UiRect::new(px(12), px(12), px(12), px(56)),
            column_gap: px(12),
        }
        BackgroundColor(VIEW_BG)
        Visibility::Hidden
        Children [
            TextureConverterLeftPane
            Node {
                width: px(350),
                flex_shrink: 0.0,
                min_height: px(0),
                flex_direction: FlexDirection::Column,
                row_gap: px(10),
            }
            --
            TextureConverterRightPane
            Node {
                flex_grow: 1.15,
                flex_basis: px(0),
                min_width: px(0),
                height: percent(100),
                min_height: px(0),
                flex_direction: FlexDirection::Column,
                row_gap: px(10),
            }
        ]
    }
}

fn preview_channel_group(current: PreviewChannel, enabled: bool) -> impl Scene {
    let items = PreviewChannel::ALL.map(|ch| GroupButton {
        caption: Caption::Text(ch.label().into()),
        action: TextureConverterAction::SetPreviewChannel(ch).into(),
        selected: current == ch,
        disabled: !enabled,
        tip: ch.tip().into(),
    });
    button_group(items.into())
}

fn export_button_label(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Ktx2 => "Export KTX2...",
        OutputFormat::Png => "Export PNG...",
    }
}

fn export_button_tip(format: OutputFormat, enabled: bool) -> String {
    if enabled {
        format!("Convert and export the 2D texture to a standalone .{} file", format.extension())
    } else {
        "Import a .png, .jpg, or .ktx2 image first to export".into()
    }
}

fn export_tool_button(format: OutputFormat, enabled: bool) -> Box<dyn Scene> {
    let tint = if enabled { TEXT } else { TEXT.with_alpha(DISABLED_ICON_ALPHA) };
    let label = export_button_label(format);
    let act = OnActivate(TextureConverterAction::ExportTexture.into());
    let tip = Tooltip(export_button_tip(format, enabled));
    if enabled {
        Box::new(bsn! {
            @FeathersToolButton {
                @variant: ButtonVariant::Plain,
                @caption: bsn_list! { @icon(IconId::Save, 16.0, tint) Node { margin: UiRect::right(px(4)) } -- @caption(label) },
            }
            Node { flex_shrink: 0.0 }
            act
            tip
        })
    } else {
        Box::new(bsn! {
            @FeathersToolButton {
                @variant: ButtonVariant::Plain,
                @caption: bsn_list! { @icon(IconId::Save, 16.0, tint) Node { margin: UiRect::right(px(4)) } -- @caption(label) },
            }
            Node { flex_shrink: 0.0 }
            InteractionDisabled
            act
            tip
        })
    }
}

fn checkbox_row_tip(label: &str, target: impl Into<PropKey>, checked: bool, tip: &str) -> Box<dyn Scene> {
    let cb = checkbox(label, target, checked);
    let tip = Tooltip(tip.to_string());
    Box::new(bsn! {
        Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, min_height: px(26) }
        tip
        Children [ @{cb} ]
    })
}

fn subpane_fill(title: &str, body: impl Scene) -> impl Scene {
    let title = title.to_string();
    bsn! {
        @subpane()
        Node { flex_grow: 1.0, min_height: px(0) }
        Children [
            @subpane_header()
            Children [ @caption(title) ]
            --
            @subpane_body()
            Node { flex_grow: 1.0, min_height: px(0), row_gap: px(10) }
            Children [ @{body} ]
        ]
    }
}

fn redraw_bar(
    editor: Res<Editor>,
    bar: Query<Entity, With<TextureConverterBar>>,
    mut last: Local<Option<(u64, usize, u64)>>,
    mut commands: Commands,
) {
    let Some((i, doc)) = active(&editor) else {
        *last = None;
        return;
    };
    let key = (editor.project_generation, i, doc.revision);
    if *last == Some(key) {
        return;
    }
    let Ok(bar) = bar.single() else { return };
    *last = Some(key);
    commands.entity(bar).despawn_children();

    let has_img = doc.source_image.is_some();
    let ch_group = preview_channel_group(doc.preview_channel, has_img);
    let import_btn = icon_tool_button(
        IconId::FolderOpen,
        "Import Image...",
        Action::from(TextureConverterAction::ImportImage),
        "Open a 2D .png, .jpg, or .ktx2 image from disk",
    );
    let export_btn = export_tool_button(doc.settings.output_format, has_img);

    commands.spawn_scene(bsn! { @{ch_group} ChildOf(bar) });
    commands.spawn_scene(bsn! { @{import_btn} ChildOf(bar) });
    commands.spawn_scene(bsn! { @{export_btn} ChildOf(bar) });
}

fn redraw_left(
    editor: Res<Editor>,
    left: Query<Entity, With<TextureConverterLeftPane>>,
    mut last: Local<Option<(u64, usize, u64)>>,
    mut commands: Commands,
) {
    let Some((i, doc)) = active(&editor) else {
        *last = None;
        return;
    };
    let key = (editor.project_generation, i, doc.revision);
    if *last == Some(key) {
        return;
    };
    let Ok(left) = left.single() else { return };
    *last = Some(key);
    commands.entity(left).despawn_children();

    let src_color = if doc.source_image.is_some() { TEXT } else { TEXT_DIM };
    let src_label = doc.source_summary();

    let thumb_content: Box<dyn Scene> = if let Some(src) = &doc.source_image {
        let (box_w, box_h) = (314.0f32, 134.0f32);
        let aspect = src.width.max(1) as f32 / src.height.max(1) as f32;
        let (fit_w, fit_h) = if box_w / box_h > aspect { (box_h * aspect, box_h) } else { (box_w, box_w / aspect) };
        Box::new(bsn! {
            Node {
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
            }
            Children [
                ImageNode
                TextureConverterThumbFrame
                Node {
                    width: px(fit_w.round()),
                    height: px(fit_h.round()),
                    border_radius: BorderRadius::all(px(4)),
                    overflow: Overflow::clip(),
                }
                Pickable::IGNORE
                Children [
                    ImageNode
                    TextureConverterThumbImage
                    Node {
                        width: percent(100),
                        height: percent(100),
                    }
                    Pickable::IGNORE
                ]
            ]
        })
    } else {
        Box::new(bsn! {
            @text("Use 'Import Image...' in the topbar to select a .png, .jpg, or .ktx2 file.", TEXT_DIM, 12.0)
        })
    };

    let source_rows: Vec<Box<dyn Scene>> = vec![
        Box::new(bsn! {
            Node { min_width: px(0), overflow: Overflow::clip() }
            Children [ @text(src_label, src_color, 12.0) ]
        }),
        Box::new(bsn! {
            Node {
                width: percent(100),
                height: px(150),
                flex_shrink: 0.0,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(6)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                overflow: Overflow::clip(),
            }
            BorderColor::all(BORDER)
            BackgroundColor(Color::srgb_u8(14, 16, 21))
            Children [ @{thumb_content} ]
        }),
    ];
    let source_pane = subpane_box("Source Image", source_rows);

    let formats: Vec<String> = OutputFormat::ALL.iter().map(|f| f.label().to_string()).collect();
    let fmt_idx = OutputFormat::ALL.iter().position(|&f| f == doc.settings.output_format).unwrap_or(0);
    let fmt_select = select_values(TextureConverterProp::OutputFormat, formats.clone(), formats, fmt_idx);

    let resizes: Vec<String> = ResizePreset::ALL.iter().map(|r| r.label().to_string()).collect();
    let resize_idx = ResizePreset::ALL.iter().position(|&r| r == doc.settings.resize).unwrap_or(0);
    let resize_select = select_values(TextureConverterProp::Resize, resizes.clone(), resizes, resize_idx);

    let mut settings_rows: Vec<Box<dyn Scene>> = vec![
        Box::new(labeled("Output Format".into(), fmt_select)),
        Box::new(labeled("Resolution".into(), resize_select)),
        checkbox_row("Flip Vertically (Y)", PropKey::new(TextureConverterProp::FlipY), doc.settings.flip_y),
    ];

    if doc.settings.output_format == OutputFormat::Ktx2 {
        let spaces: Vec<String> = Ktx2ColorSpace::ALL.iter().map(|c| c.label().to_string()).collect();
        let space_idx = Ktx2ColorSpace::ALL.iter().position(|&c| c == doc.settings.ktx2_color_space).unwrap_or(0);
        let space_select = select_values(TextureConverterProp::Ktx2ColorSpace, spaces.clone(), spaces, space_idx);
        settings_rows.push(Box::new(labeled("Color Space".into(), space_select)));
        settings_rows.push(checkbox_row_tip(
            "Generate Mipmaps",
            PropKey::new(TextureConverterProp::Mipmaps),
            doc.settings.mipmaps,
            "Generate a full 2x2 box-filtered mip chain down to 1x1",
        ));
        settings_rows.push(checkbox_row_tip(
            "Zstd Compression",
            PropKey::new(TextureConverterProp::Zstd),
            doc.settings.zstd,
            "Apply KTX2 Zstandard supercompression (level Fastest) to all mip levels",
        ));
    }

    let settings_pane = subpane_box("Output Settings", settings_rows);

    commands.spawn_scene(bsn! {
        @scroll_column()
        Node { padding: UiRect::all(px(0)), row_gap: px(10) }
        Children [
            @{source_pane}
            --
            @{settings_pane}
        ]
        ChildOf(left)
    });
}

fn redraw_right(
    editor: Res<Editor>,
    right: Query<Entity, With<TextureConverterRightPane>>,
    mut last: Local<Option<(u64, usize, u64)>>,
    mut commands: Commands,
) {
    let Some((i, doc)) = active(&editor) else {
        *last = None;
        return;
    };
    let key = (editor.project_generation, i, doc.revision);
    if *last == Some(key) {
        return;
    }
    let Ok(right) = right.single() else { return };
    *last = Some(key);
    commands.entity(right).despawn_children();

    let viewport_content: Box<dyn Scene> = if let Some(src) = &doc.source_image {
        let (out_w, out_h) = doc.settings.resize.resolve(src.width, src.height);
        let (box_w, box_h) = (380.0f32, 340.0f32);
        let aspect = out_w.max(1) as f32 / out_h.max(1) as f32;
        let (fit_w, fit_h) = if box_w / box_h > aspect { (box_h * aspect, box_h) } else { (box_w, box_w / aspect) };
        Box::new(bsn! {
            Node {
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
            }
            Children [
                ImageNode
                TextureConverterOutputFrame
                Node {
                    width: px(fit_w.round()),
                    height: px(fit_h.round()),
                    border_radius: BorderRadius::all(px(4)),
                    overflow: Overflow::clip(),
                }
                Pickable::IGNORE
                Children [
                    ImageNode
                    TextureConverterOutputImage
                    Node {
                        width: percent(100),
                        height: percent(100),
                    }
                    Pickable::IGNORE
                ]
            ]
        })
    } else {
        Box::new(bsn! {
            @text("Import a .png, .jpg, or .ktx2 image to preview the converted output", TEXT_DIM, 13.0)
        })
    };

    let preview_body = bsn! {
        TextureConverterPreviewBox
        Node {
            flex_grow: 1.0,
            width: percent(100),
            min_height: px(0),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(8)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            overflow: Overflow::clip(),
        }
        BorderColor::all(ACCENT)
        BackgroundColor(Color::srgb_u8(14, 16, 21))
        Children [ @{viewport_content} ]
    };

    let preview_pane = subpane_fill("Converted Output Preview", preview_body);
    commands.spawn_scene(bsn! { @{preview_pane} ChildOf(right) });
}

fn redraw_sidebar(
    editor: Res<Editor>,
    body: Query<Entity, With<TextureConverterSidebarBody>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some((_, doc)) = active(&editor) else { return key.forget() };
    fill_sidebar_with(&mut commands, &body, &mut key, SidebarKey::now(&editor), || sidebar_rows(doc));
}

fn sidebar_rows(doc: &TextureConverterDoc) -> Vec<Box<dyn Scene>> {
    let mut info: Vec<Box<dyn Scene>> = vec![Box::new(text(doc.source_summary(), TEXT, 13.0))];
    if let Some(src) = &doc.source_image {
        info.push(Box::new(text(
            format!("Source dimensions: {}x{} ({})", src.width, src.height, src.source_format),
            TEXT_DIM,
            12.0,
        )));
    }
    info.push(Box::new(text(format!("Output: {}", doc.output_summary()), TEXT_DIM, 12.0)));
    if let Some(path) = &doc.last_export_path {
        info.push(Box::new(text(format!("Last exported: {}", path.display()), TEXT_DIM, 12.0)));
    }
    info.push(Box::new(text(
        "Export writes a standalone .ktx2 or .png file directly to disk without creating a .texture.ron asset.",
        TEXT_DIM,
        12.0,
    )));
    vec![subpane_box("Texture Converter", info)]
}

/// Syncs the active [`TextureConverterDoc`]'s source thumbnail and transformed/channel-filtered output preview
/// [`Image`] handles, and fits the output preview frame inside [`TextureConverterPreviewBox`].
pub fn update_texture_converter_preview(world: &mut World) {
    let has_any_converter_tab = world
        .get_resource::<Editor>()
        .is_some_and(|editor| editor.tabs.iter().any(|t| t.get::<TextureConverterDoc>().is_some()));

    if !has_any_converter_tab {
        if world.get_resource::<TextureConverterPreviewCache>().is_some_and(|c| c.key.is_some()) {
            world.resource_scope(|world, mut cache: Mut<TextureConverterPreviewCache>| {
                if let Some(mut images) = world.get_resource_mut::<Assets<Image>>() {
                    cache.clear_images(&mut images);
                }
            });
        }
        return;
    }

    if !world.contains_resource::<TextureConverterPreviewCache>() {
        world.init_resource::<TextureConverterPreviewCache>();
    }
    if !world.contains_resource::<Assets<Image>>() {
        world.init_resource::<Assets<Image>>();
    }

    let handles_and_dims = world.resource_scope(|world, mut cache: Mut<TextureConverterPreviewCache>| {
        world.resource_scope(|world, mut images: Mut<Assets<Image>>| {
            let editor = world.get_resource::<Editor>()?;
            let (tab_id, doc) = active(editor)?;
            let src = doc.source_image.as_ref()?;
            let out_dims = doc.settings.resize.resolve(src.width, src.height);
            let synced = cache.sync(tab_id, doc, &mut images)?;
            Some((out_dims, synced))
        })
    });

    let Some(((out_w, out_h), (thumb_handle, out_handle, checker_handle))) = handles_and_dims else {
        return;
    };

    let tiled = NodeImageMode::Tiled { tile_x: true, tile_y: true, stretch_value: 6.0 };

    {
        let mut q = world.query_filtered::<&mut ImageNode, With<TextureConverterThumbFrame>>();
        for mut img_node in q.iter_mut(world) {
            if img_node.image != checker_handle {
                img_node.image = checker_handle.clone();
                img_node.image_mode = tiled.clone();
            }
        }
    }
    {
        let mut q = world.query_filtered::<&mut ImageNode, With<TextureConverterThumbImage>>();
        for mut img_node in q.iter_mut(world) {
            if img_node.image != thumb_handle {
                img_node.image = thumb_handle.clone();
            }
        }
    }
    {
        let mut q = world.query_filtered::<&mut ImageNode, With<TextureConverterOutputFrame>>();
        for mut img_node in q.iter_mut(world) {
            if img_node.image != checker_handle {
                img_node.image = checker_handle.clone();
                img_node.image_mode = tiled.clone();
            }
        }
    }
    {
        let mut q = world.query_filtered::<&mut ImageNode, With<TextureConverterOutputImage>>();
        for mut img_node in q.iter_mut(world) {
            if img_node.image != out_handle {
                img_node.image = out_handle.clone();
            }
        }
    }

    // Fit the output preview frame inside TextureConverterPreviewBox while preserving aspect ratio.
    let box_size: Option<Vec2> = {
        let mut q = world.query_filtered::<&ComputedNode, With<TextureConverterPreviewBox>>();
        q.iter(world).find_map(|node| {
            let sz = node.size() * node.inverse_scale_factor();
            (sz.x > 48.0 && sz.y > 48.0).then_some(sz)
        })
    };
    if let Some(sz) = box_size {
        let avail_w = (sz.x - 24.0).max(32.0);
        let avail_h = (sz.y - 24.0).max(32.0);
        let aspect = out_w.max(1) as f32 / out_h.max(1) as f32;
        let (fit_w, fit_h) =
            if avail_w / avail_h > aspect { (avail_h * aspect, avail_h) } else { (avail_w, avail_w / aspect) };
        let w = px(fit_w.round());
        let h = px(fit_h.round());
        let mut q = world.query_filtered::<&mut Node, With<TextureConverterOutputFrame>>();
        for mut node in q.iter_mut(world) {
            if node.width != w || node.height != h {
                node.width = w;
                node.height = h;
            }
        }
    }
}

enum PickedConverterDialog {
    Import(Option<PathBuf>),
    Export(Option<PathBuf>),
}

fn poll_texture_converter_dialogs(
    mut editor: ResMut<Editor>,
    mut pending: Local<Option<std::sync::mpsc::Receiver<PickedConverterDialog>>>,
) {
    if let Some(rx) = pending.as_ref() {
        match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if editor.active_doc::<TextureConverterDoc>().is_some_and(|d| d.pick_import || d.pick_export)
                    && let Some(doc) = editor.active_doc_mut::<TextureConverterDoc>()
                {
                    doc.pick_import = false;
                    doc.pick_export = false;
                }
                return;
            }
            Ok(PickedConverterDialog::Import(Some(path))) => {
                let res = editor
                    .active_doc_mut::<TextureConverterDoc>()
                    .map(|doc| doc.import_from_path(&path).map(|()| doc.source_summary()));
                if let Some(res) = res {
                    match res {
                        Ok(summary) => {
                            editor.domain_msg(
                                Some(MATERIAL),
                                Level::Info,
                                format!("Imported texture {summary} from {}", path.display()),
                            );
                            editor.ui.props += 1;
                            editor.ui.tabs += 1;
                        }
                        Err(e) => {
                            editor.domain_msg(
                                Some(MATERIAL),
                                Level::Error,
                                format!("Cannot import texture {}: {e}", path.display()),
                            );
                        }
                    }
                }
            }
            Ok(PickedConverterDialog::Export(Some(path))) => {
                let res = editor.active_doc_mut::<TextureConverterDoc>().map(|doc| doc.export_to_path(&path));
                if let Some(res) = res {
                    match res {
                        Ok(bytes) => {
                            editor.domain_msg(
                                Some(MATERIAL),
                                Level::Info,
                                format!("Exported texture to {} ({bytes} bytes)", path.display()),
                            );
                            editor.ui.props += 1;
                            editor.ui.tabs += 1;
                        }
                        Err(e) => {
                            editor.domain_msg(
                                Some(MATERIAL),
                                Level::Error,
                                format!("Cannot export texture to {}: {e}", path.display()),
                            );
                        }
                    }
                }
            }
            Ok(PickedConverterDialog::Import(None) | PickedConverterDialog::Export(None)) | Err(_) => {}
        }
        *pending = None;
    }

    if !editor.active_doc::<TextureConverterDoc>().is_some_and(|d| d.pick_import || d.pick_export) {
        return;
    }
    let project_root = editor.project.as_ref().map(|p| p.root.clone());
    let Some(doc) = editor.active_doc_mut::<TextureConverterDoc>() else { return };
    let want_import = std::mem::take(&mut doc.pick_import);
    let want_export = std::mem::take(&mut doc.pick_export) && doc.source_image.is_some();
    if !want_import && !want_export {
        return;
    }
    if std::env::var_os("SHADER_EDITOR_SCREENSHOT").is_some() {
        return;
    }

    let last_dir = doc.last_dir.clone();
    let source_dir = doc.source_path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf));
    let default_name = doc.suggested_export_filename();
    let out_fmt = doc.settings.output_format;
    let (tx, rx) = std::sync::mpsc::channel();
    *pending = Some(rx);

    if want_import {
        let start = last_dir
            .or(source_dir)
            .or_else(|| project_root.as_ref().map(|r| r.join("assets/textures")).filter(|d| d.is_dir()))
            .or_else(|| project_root.as_ref().map(|r| r.join("assets")).filter(|d| d.is_dir()))
            .or(project_root)
            .or_else(std::env::home_dir);
        std::thread::spawn(move || {
            let mut dialog = rfd::AsyncFileDialog::new()
                .set_title("Import 2D Texture")
                .add_filter("2D Texture (.png, .jpg, .jpeg, .ktx2)", &["png", "jpg", "jpeg", "ktx2"]);
            if let Some(dir) = start {
                dialog = dialog.set_directory(dir);
            }
            let picked = bevy::tasks::block_on(dialog.pick_file());
            drop(tx.send(PickedConverterDialog::Import(picked.map(|h| h.path().to_path_buf()))));
        });
    } else if want_export {
        let start = last_dir
            .or_else(|| project_root.as_ref().map(|r| r.join("assets/textures")).filter(|d| d.is_dir()))
            .or_else(|| project_root.as_ref().map(|r| r.join("assets")).filter(|d| d.is_dir()))
            .or(project_root)
            .or(source_dir)
            .or_else(std::env::home_dir);
        let ext = out_fmt.extension();
        let (filter_name, title) = match out_fmt {
            OutputFormat::Ktx2 => ("KTX2 Texture (.ktx2)", "Export KTX2 Texture"),
            OutputFormat::Png => ("PNG Image (.png)", "Export PNG Texture"),
        };
        std::thread::spawn(move || {
            let mut dialog = rfd::AsyncFileDialog::new()
                .set_title(title)
                .add_filter(filter_name, &[ext])
                .set_file_name(&default_name);
            if let Some(dir) = start {
                dialog = dialog.set_directory(dir);
            }
            let picked = bevy::tasks::block_on(dialog.save_file());
            drop(tx.send(PickedConverterDialog::Export(picked.map(|h| {
                let mut p = h.path().to_path_buf();
                if p.extension().is_none() {
                    p.set_extension(ext);
                }
                p
            }))));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texture_tools_common::encode_png;

    #[test]
    fn load_demo_populates_source_and_summaries() {
        let mut doc = TextureConverterDoc::new();
        assert_eq!(doc.source_summary(), "No image loaded");
        assert_eq!(doc.output_summary(), "No output (import a .png, .jpg, or .ktx2 image first)");
        assert_eq!(doc.suggested_export_filename(), "converted_texture.ktx2");
        assert_eq!(doc.status(&StatusCtx::default()).unwrap().counts, vec!["No image loaded".to_string()]);

        let rev0 = doc.revision;
        doc.load_demo();
        assert!(doc.revision > rev0);
        assert!(doc.source_image.is_some());
        assert_eq!(doc.source_summary(), "demo_texture.png (128x128, Rgba8UnormSrgb, 1 mip)");
        assert_eq!(doc.output_summary(), "128x128 | KTX2 | RGBA8 (sRGB) | 8 mips | Zstd");
        assert_eq!(doc.suggested_export_filename(), "demo_texture.ktx2");

        doc.settings.output_format = OutputFormat::Png;
        assert_eq!(doc.suggested_export_filename(), "demo_texture.png");
        assert_eq!(doc.output_summary(), "128x128 | PNG | RGBA8");
    }

    #[test]
    fn import_from_bytes_and_export_to_path_roundtrip_ktx2_png_and_jpeg() {
        let mut doc = TextureConverterDoc::new();
        let src = RgbaImage2d::new(8, 4, vec![[200, 100, 50, 255]; 32]);
        let png_bytes = encode_png(&src).expect("encode png");

        doc.import_from_bytes(PathBuf::from("textures/brick_albedo.png"), &png_bytes).expect("import_from_bytes");
        assert_eq!(doc.source_summary(), "brick_albedo.png (8x4, Rgba8UnormSrgb, 1 mip)");
        assert_eq!(doc.settings.ktx2_color_space, Ktx2ColorSpace::Srgb);
        assert_eq!(doc.last_dir, Some(PathBuf::from("textures")));
        assert_eq!(doc.suggested_export_filename(), "brick_albedo.ktx2");

        let dir = tempfile::tempdir().unwrap();

        // 1. Export KTX2 (with Linear color space so we can also test auto-detection on re-import).
        doc.settings.output_format = OutputFormat::Ktx2;
        doc.settings.ktx2_color_space = Ktx2ColorSpace::Linear;
        doc.settings.mipmaps = true;
        doc.settings.zstd = true;
        let ktx2_path = dir.path().join("out/brick_albedo.ktx2");
        let ktx2_len = doc.export_to_path(&ktx2_path).expect("export KTX2");
        assert!(ktx2_len > 80);
        assert_eq!(doc.last_export_path.as_deref(), Some(ktx2_path.as_path()));
        assert!(doc.status(&StatusCtx::default()).unwrap().counts.contains(&"Exported: brick_albedo.ktx2".to_string()));

        // Re-import the exported KTX2 from disk and verify Linear color space and mip count are detected.
        let mut reimported = TextureConverterDoc::new();
        reimported.import_from_path(&ktx2_path).expect("re-import KTX2");
        assert_eq!(reimported.settings.ktx2_color_space, Ktx2ColorSpace::Linear);
        assert_eq!(reimported.source_image.as_ref().unwrap().source_mips, 4); // 8x4 -> 4 mips
        assert_eq!(reimported.source_image.as_ref().unwrap().pixels, src.pixels);

        // 2. Export PNG with resize and flip_y.
        doc.settings.output_format = OutputFormat::Png;
        doc.settings.resize = ResizePreset::Size256;
        doc.settings.flip_y = true;
        let png_path = dir.path().join("out/brick_albedo.png");
        let png_len = doc.export_to_path(&png_path).expect("export PNG");
        assert!(png_len > 0);
        let decoded_png = RgbaImage2d::from_file(&png_path).expect("decode exported PNG");
        assert_eq!((decoded_png.width, decoded_png.height), (256, 256));

        // 3. Import a JPEG (.jpg) byte buffer and verify source metadata and suggested export filename.
        let mut jpg_bytes = Vec::new();
        {
            let rgb = [160u8, 80, 40].repeat(8 * 4);
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg_bytes, 100);
            enc.encode(&rgb, 8, 4, image::ExtendedColorType::Rgb8).expect("encode JPEG");
        }
        let mut jpg_doc = TextureConverterDoc::new();
        jpg_doc.import_from_bytes(PathBuf::from("textures/photo.jpg"), &jpg_bytes).expect("import JPEG");
        assert_eq!(jpg_doc.source_summary(), "photo.jpg (8x4, Rgba8UnormSrgb, 1 mip)");
        assert_eq!(jpg_doc.suggested_export_filename(), "photo.ktx2");
    }

    #[test]
    fn tab_doc_properties_and_actions_update_settings_and_revision() {
        let mut editor = crate::test_editor();
        editor.open_tool(MATERIAL, TEXTURE_CONVERTER_TOOL, None);
        {
            let doc = editor.active_doc_mut::<TextureConverterDoc>().expect("active converter doc");
            doc.load_demo();
        }
        let rev_before = editor.active_doc::<TextureConverterDoc>().unwrap().revision;

        editor.apply(Action::from(TextureConverterAction::SetPreviewChannel(PreviewChannel::A)));
        assert_eq!(editor.active_doc::<TextureConverterDoc>().unwrap().preview_channel, PreviewChannel::A);
        assert!(editor.active_doc::<TextureConverterDoc>().unwrap().revision > rev_before);

        editor.apply(Action::Tab(TabAction::SetValue {
            key: PropKey::new(TextureConverterProp::Resize),
            value: PropValue::Str(ResizePreset::Size512.label().into()),
            merge: false,
        }));
        assert_eq!(editor.active_doc::<TextureConverterDoc>().unwrap().settings.resize, ResizePreset::Size512);
        assert_eq!(
            editor.active_doc::<TextureConverterDoc>().unwrap().output_summary(),
            "512x512 | KTX2 | RGBA8 (sRGB) | 10 mips | Zstd"
        );
    }

    #[test]
    fn preview_cache_rebuilds_on_revision_and_cleans_up_on_close() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<TextureConverterPreviewCache>();
        let mut editor = crate::test_editor();
        editor.open_tool(MATERIAL, TEXTURE_CONVERTER_TOOL, None);
        editor.active_doc_mut::<TextureConverterDoc>().unwrap().load_demo();
        world.insert_resource(editor);

        update_texture_converter_preview(&mut world);
        let (thumb_id_1, out_id_1) = {
            let cache = world.resource::<TextureConverterPreviewCache>();
            let images = world.resource::<Assets<Image>>();
            let thumb = cache.source_thumb.as_ref().expect("source_thumb").id();
            let out = cache.output_preview.as_ref().expect("output_preview").id();
            let checker = cache.checker_handle.as_ref().expect("checker_handle").id();
            assert!(images.contains(thumb));
            assert!(images.contains(out));
            assert!(images.contains(checker));
            (thumb, out)
        };

        // Switching channel rebuilds and removes the previous handles from Assets<Image>.
        world
            .resource_mut::<Editor>()
            .apply(Action::from(TextureConverterAction::SetPreviewChannel(PreviewChannel::R)));
        update_texture_converter_preview(&mut world);
        let (thumb_id_2, out_id_2) = {
            let cache = world.resource::<TextureConverterPreviewCache>();
            let images = world.resource::<Assets<Image>>();
            let thumb = cache.source_thumb.as_ref().unwrap().id();
            let out = cache.output_preview.as_ref().unwrap().id();
            assert!(!images.contains(thumb_id_1));
            assert!(!images.contains(out_id_1));
            assert!(images.contains(thumb));
            assert!(images.contains(out));
            (thumb, out)
        };

        // Closing the tab cleans up the cached preview images.
        world.resource_mut::<Editor>().tabs.clear();
        world.resource_mut::<Editor>().active = None;
        update_texture_converter_preview(&mut world);
        {
            let cache = world.resource::<TextureConverterPreviewCache>();
            let images = world.resource::<Assets<Image>>();
            assert!(cache.source_thumb.is_none());
            assert!(cache.output_preview.is_none());
            assert!(!images.contains(thumb_id_2));
            assert!(!images.contains(out_id_2));
        }
    }
}
