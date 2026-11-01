//! The `HDR Converter` tool (`Tools -> HDR Converter`, design §9.2): imports an equirectangular Radiance `.hdr`
//! panorama, previews it as both a 2D tonemapped equirectangular image (left pane) and an interactive 3D cubemap / 2D
//! 6-face strip (right pane), lets the user configure face resolution, brightness, exposure (EV), texel format,
//! mipmaps, and Zstd supercompression, and exports a standalone `.ktx2` cubemap directly to disk without creating a
//! `.texture.ron` asset.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::{Camera, Projection};
use bevy::core_pipeline::Skybox;
use bevy::feathers::controls::{ButtonVariant, FeathersButton, FeathersToolButton};
use bevy::feathers::display::caption;
use bevy::image::{CompressedImageFormats, Image, ImageSampler, ImageType};
use bevy::input::mouse::MouseScrollPixelsPerLine;
use bevy::light::EnvironmentMapLight;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use bevy::ui::InteractionDisabled;
use editor_core::action::{Action, Actions, Handled, TabAction};
use editor_core::doc_kind::{DocCtx, StatusCtx, StatusInfo, TabDoc};
use editor_core::domain::ToolId;
use editor_core::dynval::{PropKey, PropTarget, PropValue};
use editor_core::editors::{EditorAreas, SidebarKey, ViewOf, bar_row, fill_sidebar_with, sidebar_body};
use editor_core::message::{Level, Message};
use editor_core::preview::{PreviewHandle, start_preview};
use editor_core::state::editor::Editor;
use editor_core::tab::ViewKindId;
use editor_core::theme::{ACCENT, BORDER, TEXT, TEXT_DIM, text};
use editor_core::tooltip::Tooltip;
use editor_core::ui::icons::{IconId, IconRef, icon};
use editor_core::ui::props::{checkbox_row, hotkey_row, labeled, number_ranged, select_values};
use editor_core::ui::shell::VIEW_BG;
use editor_core::ui::widgets::{Caption, DISABLED_ICON_ALPHA, GroupButton, button_group, scroll_column, subpane_box};
use editor_core::ui::{OnActivate, icon_tool_button};
use project_core::{AssetRef, Project};
use texture_kit::editor::{CUBEMAP_PREVIEW_KIND, CubeMapPreviewSessions, CubeMapViewMode, start_cubemap_preview};

use crate::backend::{
    CubemapFormat, FACE_SIZES, HdrConvertSettings, HdrImage, LIGHT, convert_hdr_to_cubemap, convert_hdr_to_ktx2,
};

/// The `HDR Converter` tool's id in the Light domain.
pub const HDR_CONVERTER_TOOL: ToolId = "hdr_converter";

/// The `HDR Converter` tool tab's view.
pub const HDR_CONVERTER_VIEW: ViewKindId = ViewKindId("light.hdr_converter");

const DEFAULT_YAW: f32 = 0.4;
const DEFAULT_PITCH: f32 = 0.2;
const DEFAULT_DISTANCE: f32 = 3.2;
const MIN_DISTANCE: f32 = 1.8;
const MAX_DISTANCE: f32 = 8.0;
const DEFAULT_FOV_DEG: f32 = 55.0;
const MAX_PITCH: f32 = 1.48;

/// A property widget target in the `HDR Converter` tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HdrConverterProp {
    FaceSize,
    Brightness,
    ExposureEv,
    Format,
    Mipmaps,
    Zstd,
}

impl PropTarget for HdrConverterProp {}

/// A command dispatched to the active [`HdrConverterDoc`].
#[derive(Debug, Clone, PartialEq)]
pub enum HdrConverterAction {
    BrowseImport,
    ImportPath(PathBuf),
    BrowseExport,
    ExportPath(PathBuf),
    SetCubeMode(CubeMapViewMode),
}

impl From<HdrConverterAction> for Action {
    fn from(a: HdrConverterAction) -> Self {
        Action::Tab(TabAction::doc(a))
    }
}

/// The `HDR Converter` tool tab's document.
pub struct HdrConverterDoc {
    /// Path of the imported `.hdr` file on disk (`None` before import).
    pub source_path: Option<PathBuf>,
    /// Decoded linear RGB Radiance `.hdr` panorama.
    pub hdr: Option<Arc<HdrImage>>,
    /// Conversion parameters (face resolution, brightness, exposure EV, format, mipmaps, Zstd).
    pub settings: HdrConvertSettings,
    /// Active cubemap preview mode (`3D` sphere inside skybox or `2D` 6-face strip).
    pub cube_mode: CubeMapViewMode,
    /// RAII 3D cubemap preview studio handle (`texture_kit::editor::CUBEMAP_PREVIEW_KIND`).
    pub preview: PreviewHandle,
    /// Set by `BrowseImport` so the file-dialog polling system opens a native file picker.
    pub pick_import: bool,
    /// Set by `BrowseExport` so the file-dialog polling system opens a native save picker.
    pub pick_export: bool,
    /// Bumped whenever any parameter or imported image changes.
    pub revision: u64,
    /// Bumped when the UI panes/topbar must be rebuilt.
    pub layout: u64,
}

impl Default for HdrConverterDoc {
    fn default() -> Self {
        Self::new()
    }
}

impl HdrConverterDoc {
    pub fn new() -> Self {
        Self {
            source_path: None,
            hdr: None,
            settings: HdrConvertSettings::default(),
            cube_mode: CubeMapViewMode::Sphere,
            preview: start_preview(CUBEMAP_PREVIEW_KIND),
            pick_import: false,
            pick_export: false,
            revision: 1,
            layout: 1,
        }
    }

    /// Loads a Radiance `.hdr` file from `path` and updates the document's active panorama.
    pub fn import_file(&mut self, path: &Path) -> Result<(), String> {
        let img = HdrImage::load(path)?;
        self.set_hdr(path.to_path_buf(), img);
        Ok(())
    }

    /// Sets the active `HdrImage` and source path directly.
    pub fn set_hdr(&mut self, path: PathBuf, hdr: HdrImage) {
        self.source_path = Some(path);
        self.hdr = Some(Arc::new(hdr));
        self.revision += 1;
        self.layout += 1;
    }

    /// Converts the loaded [`HdrImage`] into a 6-face `.ktx2` cubemap using `self.settings` and writes it to `path`.
    /// Does not create or modify any `.texture.ron` asset.
    pub fn export_file(&self, path: &Path) -> Result<usize, String> {
        let Some(hdr) = &self.hdr else {
            return Err("no HDR file loaded".to_string());
        };
        let bytes = convert_hdr_to_ktx2(hdr, &self.settings);
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, &bytes).map_err(|e| e.to_string())?;
        Ok(bytes.len())
    }

    /// Human-readable summary of the source `.hdr` file (`"<filename> (<W>x<H>)"` or `"No .hdr file loaded"`).
    pub fn source_summary(&self) -> String {
        match (&self.source_path, &self.hdr) {
            (Some(path), Some(hdr)) => {
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("panorama.hdr");
                format!("{name} ({}x{})", hdr.width, hdr.height)
            }
            _ => "No .hdr file loaded".to_string(),
        }
    }

    /// Human-readable summary of the target KTX2 output (`"6x 512x512 | RGBA16Float (HDR) | 10 mips"`).
    pub fn output_summary(&self) -> String {
        let mips = if self.settings.mipmaps { self.settings.face_size.max(1).ilog2() + 1 } else { 1 };
        let mip_word = if mips == 1 { "mip" } else { "mips" };
        let zstd = if self.settings.zstd { " | Zstd" } else { "" };
        format!("6x {0}x{0} | {1} | {mips} {mip_word}{zstd}", self.settings.face_size, self.settings.format.label())
    }
}

/// Opens the `HDR Converter` tool tab.
pub(super) fn open(_editor: &mut Editor) -> Box<dyn TabDoc> {
    Box::new(HdrConverterDoc::new())
}

impl TabDoc for HdrConverterDoc {
    fn apply(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        match action {
            TabAction::SetValue { key, value, .. } => {
                let Some(prop) = key.downcast::<HdrConverterProp>() else { return Handled::No };
                match prop {
                    HdrConverterProp::FaceSize => {
                        let parsed = value.as_str().parse::<u32>().unwrap_or(512);
                        let sz = if FACE_SIZES.contains(&parsed) { parsed } else { 512 };
                        self.settings.face_size = sz;
                        self.revision += 1;
                        self.layout += 1;
                        ctx.redraw_props = true;
                    }
                    HdrConverterProp::Brightness => {
                        self.settings.brightness = value.as_f32().clamp(0.0, 100.0);
                        self.revision += 1;
                        self.layout += 1;
                        ctx.redraw_props = true;
                    }
                    HdrConverterProp::ExposureEv => {
                        self.settings.exposure_ev = value.as_f32().clamp(-10.0, 10.0);
                        self.revision += 1;
                        self.layout += 1;
                        ctx.redraw_props = true;
                    }
                    HdrConverterProp::Format => {
                        if let Some(fmt) = CubemapFormat::from_label(value.as_str()) {
                            self.settings.format = fmt;
                            self.revision += 1;
                            self.layout += 1;
                            ctx.redraw_props = true;
                        }
                    }
                    HdrConverterProp::Mipmaps => {
                        self.settings.mipmaps = value.as_bool();
                        self.revision += 1;
                        self.layout += 1;
                        ctx.redraw_props = true;
                    }
                    HdrConverterProp::Zstd => {
                        self.settings.zstd = value.as_bool();
                        self.revision += 1;
                        self.layout += 1;
                        ctx.redraw_props = true;
                    }
                }
            }
            TabAction::SetComponent { key, value, is_final, .. } => {
                let Some(prop) = key.downcast::<HdrConverterProp>() else { return Handled::No };
                match prop {
                    HdrConverterProp::Brightness => {
                        let clamped = value.clamp(0.0, 100.0);
                        self.settings.brightness = clamped;
                        self.revision += 1;
                        if *is_final && clamped != *value {
                            self.layout += 1;
                        }
                        ctx.redraw_props = *is_final;
                    }
                    HdrConverterProp::ExposureEv => {
                        let clamped = value.clamp(-10.0, 10.0);
                        self.settings.exposure_ev = clamped;
                        self.revision += 1;
                        if *is_final && clamped != *value {
                            self.layout += 1;
                        }
                        ctx.redraw_props = *is_final;
                    }
                    HdrConverterProp::FaceSize => {
                        let raw = *value as u32;
                        let sz = if FACE_SIZES.contains(&raw) { raw } else { 512 };
                        self.settings.face_size = sz;
                        self.revision += 1;
                        self.layout += 1;
                        ctx.redraw_props = true;
                    }
                    _ => return Handled::No,
                }
            }
            TabAction::Doc(payload) => {
                let Some(act) = payload.downcast::<HdrConverterAction>() else { return Handled::No };
                match act {
                    HdrConverterAction::BrowseImport => {
                        self.pick_import = true;
                    }
                    HdrConverterAction::ImportPath(path) => match self.import_file(path) {
                        Ok(()) => {
                            if let Some(hdr) = &self.hdr {
                                ctx.messages.push(Message {
                                    domain: Some(LIGHT),
                                    level: Level::Info,
                                    text: format!("Imported HDR {} ({}x{})", path.display(), hdr.width, hdr.height),
                                });
                            }
                            ctx.redraw_props = true;
                        }
                        Err(e) => {
                            ctx.messages.push(Message {
                                domain: Some(LIGHT),
                                level: Level::Error,
                                text: format!("Cannot import HDR {}: {e}", path.display()),
                            });
                        }
                    },
                    HdrConverterAction::BrowseExport => {
                        if self.hdr.is_some() {
                            self.pick_export = true;
                        }
                    }
                    HdrConverterAction::ExportPath(path) => match self.export_file(path) {
                        Ok(bytes) => {
                            ctx.messages.push(Message {
                                domain: Some(LIGHT),
                                level: Level::Info,
                                text: format!("Exported cubemap KTX2 to {} ({bytes} bytes)", path.display()),
                            });
                        }
                        Err(e) => {
                            ctx.messages.push(Message {
                                domain: Some(LIGHT),
                                level: Level::Error,
                                text: format!("Cannot export KTX2 to {}: {e}", path.display()),
                            });
                        }
                    },
                    HdrConverterAction::SetCubeMode(mode) => {
                        self.cube_mode = *mode;
                        self.revision += 1;
                        self.layout += 1;
                        ctx.redraw_props = true;
                    }
                }
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn current_value(&self, key: &PropKey, _ctx: &StatusCtx) -> Option<PropValue> {
        match *key.downcast::<HdrConverterProp>()? {
            HdrConverterProp::FaceSize => Some(PropValue::Str(self.settings.face_size.to_string())),
            HdrConverterProp::Brightness => Some(PropValue::Float(self.settings.brightness)),
            HdrConverterProp::ExposureEv => Some(PropValue::Float(self.settings.exposure_ev)),
            HdrConverterProp::Format => Some(PropValue::Str(self.settings.format.label().to_string())),
            HdrConverterProp::Mipmaps => Some(PropValue::Bool(self.settings.mipmaps)),
            HdrConverterProp::Zstd => Some(PropValue::Bool(self.settings.zstd)),
        }
    }

    fn asset(&self) -> Option<&AssetRef> {
        None
    }

    fn view(&self) -> ViewKindId {
        HDR_CONVERTER_VIEW
    }

    fn base_title(&self) -> String {
        "HDR Converter".into()
    }

    fn icon(&self) -> IconRef {
        super::LIGHT_ICON
    }

    fn watermark(&self) -> Option<&'static str> {
        Some("HDR Converter")
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
        Some(HDR_CONVERTER_TOOL)
    }

    fn status(&self, ctx: &StatusCtx) -> Option<StatusInfo> {
        let src = if self.hdr.is_some() { self.source_summary() } else { "No HDR loaded".to_string() };
        Some(StatusInfo::new(false, ctx, vec![src, self.output_summary()]))
    }

    fn preview_handle(&self) -> Option<&PreviewHandle> {
        Some(&self.preview)
    }
}

/// The `HDR Converter` editor kind.
pub struct HdrConverterEditor;

/// The editor topbar row of the `HDR Converter` tab.
#[derive(Component, Default, Clone)]
pub struct HdrConverterBar;

/// The right sidebar body of the `HDR Converter` tab.
#[derive(Component, Default, Clone)]
pub struct HdrConverterSidebarBody;

/// The left zone container (`Import HDR...` bar, 2D equirectangular preview, and `Settings` subpane).
#[derive(Component, Default, Clone)]
pub struct HdrLeftPane;

/// The right zone container (3D/2D cubemap preview viewport and `Export KTX2...` footer).
#[derive(Component, Default, Clone)]
pub struct HdrRightPane;

/// Framed container for the 2D equirectangular preview in the left zone.
#[derive(Component, Default, Clone)]
pub struct Hdr2dPreviewBox;

/// The UI [`ImageNode`] displaying the tonemapped 2D equirectangular panorama.
#[derive(Component, Default, Clone)]
pub struct Hdr2dPreviewImage;

/// Interactive 3D / 2D cubemap preview viewport in the right zone.
#[derive(Component, Default, Clone)]
pub struct HdrCubePreviewViewport;

/// The UI [`ImageNode`] inside [`HdrCubePreviewViewport`] bound to the 3D [`CubeMapPreviewSession`]'s RTT image.
#[derive(Component, Default, Clone)]
pub struct HdrCubePreviewImage;

/// The UI [`ImageNode`] inside [`HdrCubePreviewViewport`] displaying all 6 cube faces stacked as a 2D strip.
#[derive(Component, Default, Clone)]
pub struct HdrCubeStripImage;

/// Output summary label updated live when settings change.
#[derive(Component, Default, Clone)]
pub struct HdrConverterSummaryText;

struct Cached2dPreview {
    hdr: Arc<HdrImage>,
    brightness_bits: u32,
    exposure_bits: u32,
    handle: Handle<Image>,
}

struct CachedCubePreview {
    hdr: Arc<HdrImage>,
    face_size: u32,
    brightness_bits: u32,
    exposure_bits: u32,
    format: CubemapFormat,
    mipmaps: bool,
    cube_handle: Handle<Image>,
    strip_handle: Handle<Image>,
}

/// Caches the tonemapped 2D equirectangular preview [`Image`] and the converted preview cubemap / 2D strip [`Image`]s
/// for the active [`HdrConverterDoc`].
#[derive(Resource, Default)]
pub struct HdrConverterPreviewCache {
    preview_2d: Option<Cached2dPreview>,
    preview_cube: Option<CachedCubePreview>,
}

impl HdrConverterPreviewCache {
    /// Returns the tonemapped 2D equirectangular preview [`Handle<Image>`], recomputing only when the source
    /// [`HdrImage`], `brightness`, or `exposure_ev` changes.
    pub fn get_2d(
        &mut self,
        hdr: &Arc<HdrImage>,
        brightness: f32,
        exposure_ev: f32,
        images: &mut Assets<Image>,
    ) -> Handle<Image> {
        let b_bits = brightness.to_bits();
        let e_bits = exposure_ev.to_bits();
        if let Some(cached) = &self.preview_2d
            && Arc::ptr_eq(&cached.hdr, hdr)
            && cached.brightness_bits == b_bits
            && cached.exposure_bits == e_bits
            && images.contains(cached.handle.id())
        {
            return cached.handle.clone();
        }
        if let Some(old) = self.preview_2d.take() {
            images.remove(old.handle.id());
        }
        let (w, h, rgba) = hdr.render_2d_preview_rgba8(512, brightness, exposure_ev);
        let img = Image::new(
            Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            TextureDimension::D2,
            rgba,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        let handle = images.add(img);
        self.preview_2d = Some(Cached2dPreview {
            hdr: hdr.clone(),
            brightness_bits: b_bits,
            exposure_bits: e_bits,
            handle: handle.clone(),
        });
        handle
    }

    /// Returns `(cube_handle, strip_handle)` for the live 3D/2D cubemap preview (`face_size` capped to `128` for
    /// instant interactive scrubbing while full resolution is used on export).
    pub fn get_cubemap(
        &mut self,
        hdr: &Arc<HdrImage>,
        settings: &HdrConvertSettings,
        images: &mut Assets<Image>,
    ) -> (Handle<Image>, Handle<Image>) {
        let preview_face_size = settings.face_size.min(128);
        let b_bits = settings.brightness.to_bits();
        let e_bits = settings.exposure_ev.to_bits();
        if let Some(cached) = &self.preview_cube
            && Arc::ptr_eq(&cached.hdr, hdr)
            && cached.face_size == preview_face_size
            && cached.brightness_bits == b_bits
            && cached.exposure_bits == e_bits
            && cached.format == settings.format
            && cached.mipmaps == settings.mipmaps
            && images.contains(cached.cube_handle.id())
            && images.contains(cached.strip_handle.id())
        {
            return (cached.cube_handle.clone(), cached.strip_handle.clone());
        }
        if let Some(old) = self.preview_cube.take() {
            images.remove(old.cube_handle.id());
            images.remove(old.strip_handle.id());
        }

        let preview_settings = HdrConvertSettings { face_size: preview_face_size, zstd: false, ..settings.clone() };
        let cubemap = convert_hdr_to_cubemap(hdr, &preview_settings);
        let ktx2_bytes = cubemap.to_ktx2_bytes(false);
        let mut cube_img = Image::from_buffer(
            &ktx2_bytes,
            ImageType::Extension("ktx2"),
            CompressedImageFormats::NONE,
            cubemap.format == CubemapFormat::Rgba8Srgb,
            ImageSampler::Default,
            RenderAssetUsages::default(),
        )
        .expect("decode preview KTX2 cubemap");
        cube_img.texture_view_descriptor =
            Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });

        let bevy_format = match cubemap.format {
            CubemapFormat::Rgba16Float => TextureFormat::Rgba16Float,
            CubemapFormat::Rgba8Srgb => TextureFormat::Rgba8UnormSrgb,
        };
        let strip_img = Image::new(
            Extent3d { width: cubemap.face_size, height: cubemap.face_size * 6, depth_or_array_layers: 1 },
            TextureDimension::D2,
            cubemap.levels[0].clone(),
            bevy_format,
            RenderAssetUsages::default(),
        );

        let cube_handle = images.add(cube_img);
        let strip_handle = images.add(strip_img);
        self.preview_cube = Some(CachedCubePreview {
            hdr: hdr.clone(),
            face_size: preview_face_size,
            brightness_bits: b_bits,
            exposure_bits: e_bits,
            format: settings.format,
            mipmaps: settings.mipmaps,
            cube_handle: cube_handle.clone(),
            strip_handle: strip_handle.clone(),
        });
        (cube_handle, strip_handle)
    }
}

impl EditorAreas for HdrConverterEditor {
    const VIEW: ViewKindId = HDR_CONVERTER_VIEW;
    const SIDEBAR_TITLE: &'static str = "HDR Converter";

    fn topbar() -> Option<impl Scene> {
        Some(bsn! { @bar_row() HdrConverterBar })
    }

    fn sidebar() -> Option<impl Scene> {
        Some(bsn! { @sidebar_body() HdrConverterSidebarBody })
    }

    fn build(app: &mut App) {
        app.init_resource::<HdrConverterPreviewCache>()
            .add_systems(
                Update,
                (
                    redraw_bar,
                    redraw_left,
                    redraw_right,
                    redraw_sidebar,
                    ApplyDeferred,
                    update_hdr_converter_preview,
                    poll_hdr_file_dialogs,
                )
                    .chain()
                    .after(editor_core::apply_actions)
                    .after(editor_core::preview::sync_previews),
            )
            .add_observer(hdr_cubemap_drag)
            .add_observer(hdr_cubemap_scroll)
            .add_observer(hdr_cubemap_double_click);
    }
}

fn active(editor: &Editor) -> Option<(usize, &HdrConverterDoc)> {
    let i = editor.active?;
    Some((i, editor.tabs.get(i)?.get::<HdrConverterDoc>()?))
}

/// The `HDR Converter` 2-zone split view in the editor content (`{left: import + 2D preview + settings} | {right: 3D/2D cubemap preview + export}`).
pub fn hdr_converter_view() -> impl Scene {
    bsn! {
        ViewOf(HDR_CONVERTER_VIEW)
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
            HdrLeftPane
            Node {
                flex_grow: 1.0,
                flex_basis: px(0),
                min_width: px(280),
                max_width: px(360),
                min_height: px(0),
                flex_direction: FlexDirection::Column,
                row_gap: px(10),
            }
            --
            HdrRightPane
            Node {
                flex_grow: 1.15,
                flex_basis: px(0),
                min_width: px(0),
                min_height: px(0),
                flex_direction: FlexDirection::Column,
                row_gap: px(10),
            }
        ]
    }
}

fn cube_mode_group(current: CubeMapViewMode, enabled: bool) -> impl Scene {
    let items = CubeMapViewMode::ALL.map(|mode| GroupButton {
        caption: Caption::Text(mode.label().into()),
        action: HdrConverterAction::SetCubeMode(mode).into(),
        selected: current == mode,
        disabled: !enabled,
        tip: mode.tip().into(),
    });
    button_group(items.into())
}

fn export_tool_button(enabled: bool) -> Box<dyn Scene> {
    let tint = if enabled { TEXT } else { TEXT.with_alpha(DISABLED_ICON_ALPHA) };
    let act = OnActivate(HdrConverterAction::BrowseExport.into());
    let tip = Tooltip(if enabled {
        "Convert and export the 6-face cubemap to a standalone .ktx2 file".into()
    } else {
        "Import an .hdr file first to export a .ktx2 cubemap".into()
    });
    if enabled {
        Box::new(bsn! {
            @FeathersToolButton {
                @variant: ButtonVariant::Plain,
                @caption: bsn_list! { @icon(IconId::Save, 16.0, tint) Node { margin: UiRect::right(px(4)) } -- @caption("Export KTX2...") },
            }
            Node { flex_shrink: 0.0 }
            act
            tip
        })
    } else {
        Box::new(bsn! {
            @FeathersToolButton {
                @variant: ButtonVariant::Plain,
                @caption: bsn_list! { @icon(IconId::Save, 16.0, tint) Node { margin: UiRect::right(px(4)) } -- @caption("Export KTX2...") },
            }
            Node { flex_shrink: 0.0 }
            InteractionDisabled
            act
            tip
        })
    }
}

fn export_primary_button(enabled: bool) -> Box<dyn Scene> {
    let tint = if enabled { TEXT } else { TEXT.with_alpha(DISABLED_ICON_ALPHA) };
    let variant = if enabled { ButtonVariant::Primary } else { ButtonVariant::Normal };
    let act = OnActivate(HdrConverterAction::BrowseExport.into());
    let tip = Tooltip(if enabled {
        "Convert and export the 6-face cubemap to a standalone .ktx2 file".into()
    } else {
        "Import an .hdr file first to export a .ktx2 cubemap".into()
    });
    if enabled {
        Box::new(bsn! {
            @FeathersButton {
                @variant: variant,
                @caption: bsn_list! { @icon(IconId::Save, 16.0, tint) Node { margin: UiRect::right(px(6)) } -- @caption("Export KTX2...") },
            }
            Node { flex_shrink: 0.0 }
            act
            tip
        })
    } else {
        Box::new(bsn! {
            @FeathersButton {
                @variant: variant,
                @caption: bsn_list! { @icon(IconId::Save, 16.0, tint) Node { margin: UiRect::right(px(6)) } -- @caption("Export KTX2...") },
            }
            Node { flex_shrink: 0.0 }
            InteractionDisabled
            act
            tip
        })
    }
}

fn redraw_bar(
    editor: Res<Editor>,
    bar: Query<Entity, With<HdrConverterBar>>,
    mut last: Local<Option<(u64, usize, u64)>>,
    mut commands: Commands,
) {
    let Some((i, doc)) = active(&editor) else {
        *last = None;
        return;
    };
    let key = (editor.project_generation, i, doc.layout);
    if *last == Some(key) {
        return;
    }
    let Ok(bar) = bar.single() else { return };
    *last = Some(key);
    commands.entity(bar).despawn_children();

    let has_hdr = doc.hdr.is_some();
    let mode_group = cube_mode_group(doc.cube_mode, has_hdr);
    let import_btn = icon_tool_button(
        IconId::FolderOpen,
        "Import HDR...",
        Action::from(HdrConverterAction::BrowseImport),
        "Open a Radiance .hdr panorama file from disk",
    );
    let export_btn = export_tool_button(has_hdr);
    let summary_str = doc.output_summary();

    commands.spawn_scene(bsn! { @{mode_group} ChildOf(bar) });
    commands.spawn_scene(bsn! { @{import_btn} ChildOf(bar) });
    commands.spawn_scene(bsn! { @{export_btn} ChildOf(bar) });
    commands.spawn_scene(bsn! {
        HdrConverterSummaryText
        @text(summary_str, TEXT_DIM, 12.0)
        ChildOf(bar)
    });
}

fn redraw_left(
    editor: Res<Editor>,
    left: Query<Entity, With<HdrLeftPane>>,
    mut last: Local<Option<(u64, usize, u64)>>,
    mut commands: Commands,
) {
    let Some((i, doc)) = active(&editor) else {
        *last = None;
        return;
    };
    let key = (editor.project_generation, i, doc.layout);
    if *last == Some(key) {
        return;
    }
    let Ok(left) = left.single() else { return };
    *last = Some(key);
    commands.entity(left).despawn_children();

    let import_btn = icon_tool_button(
        IconId::FolderOpen,
        "Import HDR...",
        Action::from(HdrConverterAction::BrowseImport),
        "Open a Radiance .hdr panorama file from disk",
    );
    let src_color = if doc.hdr.is_some() { TEXT } else { TEXT_DIM };
    let src_label = doc.source_summary();

    let preview_2d_content: Box<dyn Scene> = if let Some(hdr) = &doc.hdr {
        // Fit the 2D panorama within the preview box while preserving aspect ratio.
        let (box_w, box_h) = (304.0f32, 152.0f32);
        let aspect = hdr.width.max(1) as f32 / hdr.height.max(1) as f32;
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
                Hdr2dPreviewImage
                Node {
                    width: px(fit_w.round()),
                    height: px(fit_h.round()),
                    border_radius: BorderRadius::all(px(4)),
                }
                Pickable::IGNORE
                --
                Node {
                    position_type: PositionType::Absolute,
                    left: px(10),
                    top: px(8),
                }
                Pickable::IGNORE
                Children [
                    @text("2D Panorama", TEXT_DIM, 11.0)
                ]
            ]
        })
    } else {
        Box::new(bsn! {
            @text("2D Equirectangular Preview (import .hdr above)", TEXT_DIM, 12.0)
        })
    };

    let sizes: Vec<String> = FACE_SIZES.iter().map(u32::to_string).collect();
    let size_idx = FACE_SIZES.iter().position(|&s| s == doc.settings.face_size).unwrap_or(1);
    let face_select = select_values(HdrConverterProp::FaceSize, sizes.clone(), sizes, size_idx);

    let formats: Vec<String> = CubemapFormat::ALL.iter().map(|f| f.label().to_string()).collect();
    let fmt_idx = CubemapFormat::ALL.iter().position(|&f| f == doc.settings.format).unwrap_or(0);
    let fmt_select = select_values(HdrConverterProp::Format, formats.clone(), formats, fmt_idx);

    let settings_rows: Vec<Box<dyn Scene>> = vec![
        Box::new(labeled("Face Resolution".into(), face_select)),
        Box::new(labeled(
            "Brightness".into(),
            number_ranged(
                HdrConverterProp::Brightness,
                0,
                doc.settings.brightness,
                Some((0.0, 5.0)),
                Some((0.0, 100.0)),
            ),
        )),
        Box::new(labeled(
            "Exposure (EV)".into(),
            number_ranged(
                HdrConverterProp::ExposureEv,
                0,
                doc.settings.exposure_ev,
                Some((-5.0, 5.0)),
                Some((-10.0, 10.0)),
            ),
        )),
        Box::new(labeled("Output Format".into(), fmt_select)),
        checkbox_row("Generate Mipmaps", PropKey::new(HdrConverterProp::Mipmaps), doc.settings.mipmaps),
        checkbox_row("Zstd Compression", PropKey::new(HdrConverterProp::Zstd), doc.settings.zstd),
    ];
    let settings_pane = subpane_box("Settings", settings_rows);

    commands.spawn_scene(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(10),
            min_height: px(28),
        }
        Children [
            @{import_btn}
            --
            Node { flex_grow: 1.0, min_width: px(0), overflow: Overflow::clip() }
            Children [ @text(src_label, src_color, 12.0) ]
        ]
        ChildOf(left)
    });

    commands.spawn_scene(bsn! {
        Hdr2dPreviewBox
        Node {
            width: percent(100),
            height: px(164),
            flex_shrink: 0.0,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            overflow: Overflow::clip(),
        }
        BorderColor::all(BORDER)
        BackgroundColor(Color::srgb_u8(20, 22, 26))
        Children [ @{preview_2d_content} ]
        ChildOf(left)
    });

    commands.spawn_scene(bsn! {
        @scroll_column()
        Node { padding: UiRect::all(px(0)), row_gap: px(10) }
        Children [ @{settings_pane} ]
        ChildOf(left)
    });
}

fn redraw_right(
    editor: Res<Editor>,
    right: Query<Entity, With<HdrRightPane>>,
    mut last: Local<Option<(u64, usize, u64)>>,
    mut commands: Commands,
) {
    let Some((i, doc)) = active(&editor) else {
        *last = None;
        return;
    };
    let key = (editor.project_generation, i, doc.layout);
    if *last == Some(key) {
        return;
    }
    let Ok(right) = right.single() else { return };
    *last = Some(key);
    commands.entity(right).despawn_children();

    let has_hdr = doc.hdr.is_some();
    let show_3d = doc.cube_mode == CubeMapViewMode::Sphere;
    let display_3d = if show_3d { Display::Flex } else { Display::None };
    let display_strip = if show_3d { Display::None } else { Display::Flex };
    let mode_group = cube_mode_group(doc.cube_mode, has_hdr);
    let caption_str =
        if show_3d { "3D Cubemap (drag: orbit, wheel: zoom)" } else { "2D Faces (+X, -X, +Y, -Y, +Z, -Z)" };

    let viewport_content: Box<dyn Scene> = if has_hdr {
        Box::new(bsn! {
            Node {
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
            }
            Children [
                ImageNode
                HdrCubePreviewImage
                Node {
                    width: percent(100),
                    height: percent(100),
                    display: (display_3d),
                }
                Pickable::IGNORE
                --
                ImageNode
                HdrCubeStripImage
                Node {
                    width: px(96),
                    height: px(576),
                    max_height: percent(92),
                    display: (display_strip),
                }
                Pickable::IGNORE
                --
                Node {
                    position_type: PositionType::Absolute,
                    left: px(12),
                    top: px(10),
                }
                Pickable::IGNORE
                Children [
                    @text(caption_str, TEXT_DIM, 11.0)
                ]
                --
                Node {
                    position_type: PositionType::Absolute,
                    right: px(10),
                    top: px(8),
                }
                Children [ @{mode_group} ]
            ]
        })
    } else {
        Box::new(bsn! {
            @text("Import an .hdr file on the left to preview the 3D cubemap", TEXT_DIM, 13.0)
        })
    };

    let summary_str = doc.output_summary();
    let export_btn = export_primary_button(has_hdr);

    commands.spawn_scene(bsn! {
        HdrCubePreviewViewport
        Node {
            flex_grow: 1.0,
            width: percent(100),
            min_height: px(0),
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::all(px(8)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            overflow: Overflow::clip(),
        }
        BorderColor::all(ACCENT)
        BackgroundColor(Color::srgb_u8(20, 22, 26))
        Children [ @{viewport_content} ]
        ChildOf(right)
    });

    commands.spawn_scene(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            column_gap: px(10),
            min_height: px(30),
        }
        Children [
            Node { flex_grow: 1.0, min_width: px(0), overflow: Overflow::clip() }
            Children [
                HdrConverterSummaryText
                @text(summary_str, TEXT_DIM, 11.0)
            ]
            --
            @{export_btn}
        ]
        ChildOf(right)
    });
}

fn redraw_sidebar(
    editor: Res<Editor>,
    body: Query<Entity, With<HdrConverterSidebarBody>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some((_, doc)) = active(&editor) else { return key.forget() };
    fill_sidebar_with(&mut commands, &body, &mut key, SidebarKey::now(&editor), || sidebar_rows(doc));
}

const HOTKEYS: [(&str, &str); 3] =
    [("Drag", "Orbit 3D preview"), ("Wheel", "Zoom 3D preview"), ("Double-click", "Reset camera")];

fn sidebar_rows(doc: &HdrConverterDoc) -> Vec<Box<dyn Scene>> {
    let mut info: Vec<Box<dyn Scene>> = vec![Box::new(text(doc.source_summary(), TEXT, 13.0))];
    if let Some(hdr) = &doc.hdr {
        info.push(Box::new(text(format!("Source dimensions: {}x{}", hdr.width, hdr.height), TEXT_DIM, 12.0)));
    }
    info.push(Box::new(text(format!("Output: {}", doc.output_summary()), TEXT_DIM, 12.0)));
    info.push(Box::new(text(format!("Effective scale: {:.3}x", doc.settings.scale()), TEXT_DIM, 12.0)));
    info.push(Box::new(text(
        "Converts an equirectangular .hdr panorama directly to a Bevy .ktx2 cubemap without creating a Texture asset.",
        TEXT_DIM,
        12.0,
    )));
    let hotkeys = HOTKEYS.map(|(keys, what)| -> Box<dyn Scene> { Box::new(hotkey_row(keys, what)) });
    vec![subpane_box("HDR Converter", info), subpane_box("Hotkeys", hotkeys.into())]
}

/// Syncs the active [`HdrConverterDoc`]'s 2D preview image, 2D strip image, and 3D [`CubeMapPreviewSession`] camera /
/// skybox / IBL light.
pub fn update_hdr_converter_preview(world: &mut World) {
    let hdr_tab_ids: Vec<u64> = world
        .get_resource::<Editor>()
        .map(|editor| {
            editor
                .tabs
                .iter()
                .filter_map(|t| t.get::<HdrConverterDoc>())
                .filter(|d| d.preview.is_alive())
                .map(|d| d.preview.id())
                .collect()
        })
        .unwrap_or_default();

    let active_info = world.get_resource::<Editor>().and_then(|editor| {
        let doc = editor.active_doc::<HdrConverterDoc>()?;
        let id = doc.preview.is_alive().then(|| doc.preview.id())?;
        Some((id, doc.hdr.clone(), doc.settings.clone(), doc.cube_mode, doc.output_summary()))
    });

    let Some((active_id, hdr_opt, settings, cube_mode, summary)) = active_info else {
        if let Some(sessions) = world.get_resource::<CubeMapPreviewSessions>() {
            let cams: Vec<Entity> =
                hdr_tab_ids.iter().filter_map(|id| sessions.sessions.get(id).map(|s| s.camera)).collect();
            for cam_entity in cams {
                if let Ok(mut e) = world.get_entity_mut(cam_entity)
                    && let Some(mut cam) = e.get_mut::<Camera>()
                    && cam.is_active
                {
                    cam.is_active = false;
                }
            }
        }
        return;
    };

    {
        let mut q = world.query_filtered::<&mut Text, With<HdrConverterSummaryText>>();
        for mut t in q.iter_mut(world) {
            if t.0 != summary {
                t.0 = summary.clone();
            }
        }
    }

    let Some(hdr) = hdr_opt else {
        if let Some(sessions) = world.get_resource::<CubeMapPreviewSessions>() {
            let cams: Vec<Entity> =
                hdr_tab_ids.iter().filter_map(|id| sessions.sessions.get(id).map(|s| s.camera)).collect();
            for cam_entity in cams {
                if let Ok(mut e) = world.get_entity_mut(cam_entity)
                    && let Some(mut cam) = e.get_mut::<Camera>()
                    && cam.is_active
                {
                    cam.is_active = false;
                }
            }
        }
        return;
    };

    if !world.contains_resource::<HdrConverterPreviewCache>() {
        world.init_resource::<HdrConverterPreviewCache>();
    }
    if !world.contains_resource::<Assets<Image>>() {
        world.init_resource::<Assets<Image>>();
    }

    let (preview_2d_handle, cube_handle, strip_handle) =
        world.resource_scope(|world, mut cache: Mut<HdrConverterPreviewCache>| {
            let mut images = world.resource_mut::<Assets<Image>>();
            let h2d = cache.get_2d(&hdr, settings.brightness, settings.exposure_ev, &mut images);
            let (hcube, hstrip) = cache.get_cubemap(&hdr, &settings, &mut images);
            (h2d, hcube, hstrip)
        });

    {
        let mut q = world.query_filtered::<&mut ImageNode, With<Hdr2dPreviewImage>>();
        for mut img_node in q.iter_mut(world) {
            if img_node.image != preview_2d_handle {
                img_node.image = preview_2d_handle.clone();
            }
        }
    }
    {
        let mut q = world.query_filtered::<&mut ImageNode, With<HdrCubeStripImage>>();
        for mut img_node in q.iter_mut(world) {
            if img_node.image != strip_handle {
                img_node.image = strip_handle.clone();
            }
        }
    }

    let show_3d = cube_mode == CubeMapViewMode::Sphere;

    if let Some(sessions) = world.get_resource::<CubeMapPreviewSessions>() {
        let inactive_cams: Vec<Entity> = hdr_tab_ids
            .iter()
            .filter(|&&id| id != active_id || !show_3d)
            .filter_map(|id| sessions.sessions.get(id).map(|s| s.camera))
            .collect();
        for cam_entity in inactive_cams {
            if let Ok(mut e) = world.get_entity_mut(cam_entity)
                && let Some(mut cam) = e.get_mut::<Camera>()
                && cam.is_active
            {
                cam.is_active = false;
            }
        }
    }

    if !show_3d {
        if let Some(mut sessions) = world.get_resource_mut::<CubeMapPreviewSessions>()
            && let Some(sess) = sessions.sessions.get_mut(&active_id)
        {
            sess.mode = cube_mode;
        }
        return;
    }

    if !world.contains_resource::<CubeMapPreviewSessions>()
        || !world.resource::<CubeMapPreviewSessions>().sessions.contains_key(&active_id)
    {
        start_cubemap_preview(world, active_id);
    }

    let viewport_size: Option<UVec2> = {
        let mut q = world.query_filtered::<&ComputedNode, With<HdrCubePreviewViewport>>();
        q.iter(world).find_map(|node| {
            let sz = (node.size() * node.inverse_scale_factor()).round().as_uvec2();
            (sz.x >= 64 && sz.y >= 64).then_some(UVec2::new(sz.x.min(1920), sz.y.min(1440)))
        })
    };

    let (camera_entity, sphere_entity, rtt_handle, cam_tf, fov_rad) = {
        let mut sessions = world.resource_mut::<CubeMapPreviewSessions>();
        let Some(sess) = sessions.sessions.get_mut(&active_id) else { return };
        sess.mode = cube_mode;
        let tf = sess.camera_transform();
        let fov = DEFAULT_FOV_DEG.to_radians();
        (sess.camera, sess.sphere, sess.rtt_image.clone(), tf, fov)
    };

    if let (Some(rtt), Some(want_size)) = (rtt_handle.as_ref(), viewport_size)
        && let Some(mut images) = world.get_resource_mut::<Assets<Image>>()
        && images.get(rtt.id()).is_some_and(|img| img.size() != want_size)
        && let Some(mut img) = images.get_mut(rtt.id())
    {
        img.resize(Extent3d { width: want_size.x, height: want_size.y, depth_or_array_layers: 1 });
    }

    if let Ok(mut sphere_ent) = world.get_entity_mut(sphere_entity)
        && let Some(mut vis) = sphere_ent.get_mut::<Visibility>()
        && *vis != Visibility::Inherited
    {
        *vis = Visibility::Inherited;
    }

    if let Ok(mut cam_ent) = world.get_entity_mut(camera_entity) {
        if let Some(mut cam) = cam_ent.get_mut::<Camera>()
            && !cam.is_active
        {
            cam.is_active = true;
        }
        if let Some(mut tf) = cam_ent.get_mut::<Transform>()
            && *tf != cam_tf
        {
            *tf = cam_tf;
        }
        if let Some(mut proj) = cam_ent.get_mut::<Projection>()
            && let Projection::Perspective(ref mut p) = *proj
            && (p.fov - fov_rad).abs() > 1e-5
        {
            p.fov = fov_rad;
        }
        let skybox = Skybox { image: Some(cube_handle.clone()), brightness: 1000.0, rotation: Quat::IDENTITY };
        if let Some(mut existing) = cam_ent.get_mut::<Skybox>() {
            if existing.image != skybox.image || existing.brightness != skybox.brightness {
                *existing = skybox;
            }
        } else {
            cam_ent.insert(skybox);
        }
        let ibl = EnvironmentMapLight {
            diffuse_map: cube_handle.clone(),
            specular_map: cube_handle,
            intensity: 1000.0,
            rotation: Quat::IDENTITY,
            affects_lightmapped_mesh_diffuse: true,
        };
        if let Some(mut existing) = cam_ent.get_mut::<EnvironmentMapLight>() {
            if existing.diffuse_map != ibl.diffuse_map || existing.specular_map != ibl.specular_map {
                *existing = ibl;
            }
        } else {
            cam_ent.insert(ibl);
        }
    }

    if let Some(rtt) = rtt_handle {
        let mut q = world.query_filtered::<&mut ImageNode, With<HdrCubePreviewImage>>();
        for mut img_node in q.iter_mut(world) {
            if img_node.image != rtt {
                img_node.image = rtt.clone();
            }
        }
    }
}

fn active_3d_hdr_session(editor: &Editor) -> Option<u64> {
    let doc = editor.active_doc::<HdrConverterDoc>()?;
    if doc.hdr.is_none() || doc.cube_mode == CubeMapViewMode::Strip2d || !doc.preview.is_alive() {
        return None;
    }
    Some(doc.preview.id())
}

fn hdr_cubemap_drag(
    mut ev: On<PointerDrag>,
    viewports: Query<(), With<HdrCubePreviewViewport>>,
    editor: Res<Editor>,
    mut sessions: ResMut<CubeMapPreviewSessions>,
) {
    if !viewports.contains(ev.entity) {
        return;
    }
    let Some(id) = active_3d_hdr_session(&editor) else { return };
    let Some(sess) = sessions.sessions.get_mut(&id) else { return };
    ev.propagate(false);
    sess.yaw -= ev.delta.x * 0.008;
    sess.pitch = (sess.pitch + ev.delta.y * 0.008).clamp(-MAX_PITCH, MAX_PITCH);
}

fn hdr_cubemap_scroll(
    mut ev: On<PointerScroll>,
    viewports: Query<(), With<HdrCubePreviewViewport>>,
    ratio: Res<MouseScrollPixelsPerLine>,
    editor: Res<Editor>,
    mut sessions: ResMut<CubeMapPreviewSessions>,
) {
    if !viewports.contains(ev.entity) {
        return;
    }
    let Some(id) = active_3d_hdr_session(&editor) else { return };
    let Some(sess) = sessions.sessions.get_mut(&id) else { return };
    ev.propagate(false);
    let notches = ev.to_lines(&ratio).y;
    if notches == 0.0 {
        return;
    }
    sess.distance = (sess.distance * 0.9f32.powf(notches)).clamp(MIN_DISTANCE, MAX_DISTANCE);
}

fn hdr_cubemap_double_click(
    ev: On<PointerClick>,
    viewports: Query<(), With<HdrCubePreviewViewport>>,
    editor: Res<Editor>,
    mut sessions: ResMut<CubeMapPreviewSessions>,
) {
    if !viewports.contains(ev.entity) || ev.button != PointerButton::Primary || ev.count < 2 {
        return;
    }
    let Some(id) = active_3d_hdr_session(&editor) else { return };
    let Some(sess) = sessions.sessions.get_mut(&id) else { return };
    sess.yaw = DEFAULT_YAW;
    sess.pitch = DEFAULT_PITCH;
    sess.distance = DEFAULT_DISTANCE;
}

enum PickedDialog {
    Import(Option<PathBuf>),
    Export(Option<PathBuf>),
}

fn poll_hdr_file_dialogs(
    mut editor: ResMut<Editor>,
    mut actions: ResMut<Actions>,
    mut pending: Local<Option<std::sync::mpsc::Receiver<PickedDialog>>>,
) {
    if let Some(rx) = pending.as_ref() {
        match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if let Some(doc) = editor.active_doc_mut::<HdrConverterDoc>() {
                    doc.pick_import = false;
                    doc.pick_export = false;
                }
                return;
            }
            Ok(PickedDialog::Import(Some(path))) => actions.push(HdrConverterAction::ImportPath(path)),
            Ok(PickedDialog::Export(Some(path))) => actions.push(HdrConverterAction::ExportPath(path)),
            Ok(PickedDialog::Import(None) | PickedDialog::Export(None)) | Err(_) => {}
        }
        *pending = None;
    }

    let project_root = editor.project.as_ref().map(|p| p.root.clone());
    let Some(doc) = editor.active_doc_mut::<HdrConverterDoc>() else { return };
    let want_import = std::mem::take(&mut doc.pick_import);
    let want_export = std::mem::take(&mut doc.pick_export) && doc.hdr.is_some();
    if !want_import && !want_export {
        return;
    }
    if std::env::var_os("SHADER_EDITOR_SCREENSHOT").is_some() {
        return;
    }

    let source_path = doc.source_path.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    *pending = Some(rx);

    if want_import {
        let start = source_path
            .as_ref()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .or(project_root)
            .or_else(std::env::home_dir);
        std::thread::spawn(move || {
            let mut dialog = rfd::AsyncFileDialog::new()
                .set_title("Import Equirectangular HDR")
                .add_filter("Radiance HDR", &["hdr"]);
            if let Some(dir) = start {
                dialog = dialog.set_directory(dir);
            }
            let picked = bevy::tasks::block_on(dialog.pick_file());
            drop(tx.send(PickedDialog::Import(picked.map(|h| h.path().to_path_buf()))));
        });
    } else if want_export {
        let default_name = source_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .map_or_else(|| "skybox.ktx2".to_string(), |stem| format!("{stem}_cubemap.ktx2"));
        let start = project_root
            .as_ref()
            .map(|r| r.join("assets/textures"))
            .filter(|d| d.is_dir())
            .or_else(|| project_root.as_ref().map(|r| r.join("assets")).filter(|d| d.is_dir()))
            .or(project_root)
            .or_else(|| source_path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)))
            .or_else(std::env::home_dir);
        std::thread::spawn(move || {
            let mut dialog = rfd::AsyncFileDialog::new()
                .set_title("Export Cubemap KTX2")
                .add_filter("KTX2 Texture", &["ktx2"])
                .set_file_name(&default_name);
            if let Some(dir) = start {
                dialog = dialog.set_directory(dir);
            }
            let picked = bevy::tasks::block_on(dialog.save_file());
            drop(tx.send(PickedDialog::Export(picked.map(|h| {
                let mut p = h.path().to_path_buf();
                if p.extension().is_none() {
                    p.set_extension("ktx2");
                }
                p
            }))));
        });
    }
}

/// Generates a synthetic HDR sky panorama with a bright sun disc, blue zenith, golden horizon glow, and lit ground
/// hemisphere (used by `SHADER_EDITOR_OPEN="@tool/hdr_converter:demo"` and tests).
pub fn demo_sky_hdr(width: u32, height: u32) -> HdrImage {
    let w = width.max(1);
    let h = height.max(1);
    let sun_raw = [0.45f32, 0.36, -0.82];
    let sun_len = (sun_raw[0] * sun_raw[0] + sun_raw[1] * sun_raw[1] + sun_raw[2] * sun_raw[2]).sqrt();
    let sun_dir = [sun_raw[0] / sun_len, sun_raw[1] / sun_len, sun_raw[2] / sun_len];

    let mut pixels = Vec::with_capacity(w as usize * h as usize);
    for y in 0..h {
        let v = (y as f32 + 0.5) / h as f32;
        let lat = (0.5 - v) * std::f32::consts::PI;
        let wy = lat.sin();
        let cos_lat = lat.cos();
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;
            let lon = (u - 0.5) * std::f32::consts::TAU;
            let wx = cos_lat * lon.sin();
            let wz = -cos_lat * lon.cos();

            let mut rgb = if wy >= 0.0 {
                let t = wy.powf(0.55);
                let zenith = [0.10f32, 0.28, 1.15];
                let horizon = [1.45f32, 0.78, 0.42];
                let mut sky = [
                    horizon[0] + (zenith[0] - horizon[0]) * t,
                    horizon[1] + (zenith[1] - horizon[1]) * t,
                    horizon[2] + (zenith[2] - horizon[2]) * t,
                ];
                let cloud_band = (1.0 - (wy - 0.22).abs() * 4.5).clamp(0.0, 1.0);
                let wave = ((lon * 3.0).sin() * 0.5 + (lon * 7.0 + 1.2).cos() * 0.5).max(0.0);
                let cloud = cloud_band * wave * 0.45;
                sky[0] += cloud * 1.3;
                sky[1] += cloud * 0.95;
                sky[2] += cloud * 0.75;
                sky
            } else {
                let g = (-wy).powf(0.65);
                let horizon_ground = [0.52f32, 0.36, 0.25];
                let nadir_ground = [0.10f32, 0.09, 0.08];
                [
                    horizon_ground[0] + (nadir_ground[0] - horizon_ground[0]) * g,
                    horizon_ground[1] + (nadir_ground[1] - horizon_ground[1]) * g,
                    horizon_ground[2] + (nadir_ground[2] - horizon_ground[2]) * g,
                ]
            };

            let cos_sun = (wx * sun_dir[0] + wy * sun_dir[1] + wz * sun_dir[2]).clamp(-1.0, 1.0);
            if cos_sun > 0.0 {
                let scatter = cos_sun.powf(5.0) * 0.65;
                rgb[0] += scatter * 1.2;
                rgb[1] += scatter * 0.65;
                rgb[2] += scatter * 0.25;
            }
            if cos_sun > 0.92 {
                let corona = ((cos_sun - 0.92) / 0.08).powf(3.0) * 4.5;
                rgb[0] += corona * 1.4;
                rgb[1] += corona * 0.95;
                rgb[2] += corona * 0.45;
            }
            if cos_sun > 0.995 {
                rgb[0] += 18.0;
                rgb[1] += 15.0;
                rgb[2] += 10.0;
            }
            pixels.push(rgb);
        }
    }
    HdrImage { width: w, height: h, pixels }
}
