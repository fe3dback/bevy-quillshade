//! The `Texture Channel Assembler` tool (`Tools -> Texture Channel Assembler`): packs up to 4 independent 2D `.png`,
//! `.jpg`/`.jpeg`, or `.ktx2` images into the `R`, `G`, `B`, and `A` channels of a single output texture (e.g. ORM
//! masks, custom packed masks, or albedo + height/opacity). Each channel slot supports selecting the source channel
//! (`R`, `G`, `B`, `A`, or `Luminance`), optional inversion (`1 - x`), or a constant fallback fill (`0` or `255`) when
//! left empty. Exports a standalone `.ktx2` or `.png` file directly to disk without creating a `.texture.ron` asset.

use std::path::{Path, PathBuf};

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
    ChannelSlot, Ktx2ColorSpace, OutputFormat, PreviewChannel, ResizePreset, RgbaImage2d, SourceChannel,
    TextureExportSettings, assemble_channels, demo_assembler_slots, encode_ktx2, encode_png, rgba_to_bevy_image,
};

/// The `Texture Channel Assembler` tool's id in the Material domain.
pub const CHANNEL_ASSEMBLER_TOOL: ToolId = "channel_assembler";

/// The `Texture Channel Assembler` tool tab's view.
pub const CHANNEL_ASSEMBLER_VIEW: ViewKindId = ViewKindId("material.channel_assembler");

/// Titles for the 4 output channel subpanes (`R`, `G`, `B`, `A`).
pub const CHANNEL_TITLES: [&str; 4] = ["Red Channel (R)", "Green Channel (G)", "Blue Channel (B)", "Alpha Channel (A)"];

const CHANNEL_SHORT: [&str; 4] = ["R", "G", "B", "A"];
const DEFAULT_FILL_LABELS: [&str; 2] = ["0 (Black)", "255 (White)"];

fn default_value_label(v: u8) -> &'static str {
    if v == 0 { DEFAULT_FILL_LABELS[0] } else { DEFAULT_FILL_LABELS[1] }
}

fn default_value_from_label(s: &str) -> Option<u8> {
    match s.trim() {
        "0 (Black)" | "0" => Some(0),
        "255 (White)" | "255" => Some(255),
        _ => None,
    }
}

/// A property widget target in the `Texture Channel Assembler` tool.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelAssemblerProp {
    SlotSourceChannel(usize),
    SlotInvert(usize),
    SlotDefaultValue(usize),
    OutputFormat,
    Resize,
    FlipY,
    Ktx2ColorSpace,
    Mipmaps,
    Zstd,
}

impl PropTarget for ChannelAssemblerProp {}

/// A command dispatched to the active [`ChannelAssemblerDoc`].
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelAssemblerAction {
    ImportSlot(usize),
    ClearSlot(usize),
    ClearAll,
    ExportTexture,
    SetPreviewChannel(PreviewChannel),
}

impl From<ChannelAssemblerAction> for Action {
    fn from(a: ChannelAssemblerAction) -> Self {
        Action::Tab(TabAction::doc(a))
    }
}

/// The `Texture Channel Assembler` tool tab's document.
pub struct ChannelAssemblerDoc {
    /// The 4 output channel slots (`R`, `G`, `B`, `A`).
    pub slots: [ChannelSlot; 4],
    /// Export settings (defaults to `Ktx2ColorSpace::Linear` because channel-packed textures are typically linear masks).
    pub settings: TextureExportSettings,
    /// Active channel filter for the assembled output preview (`All | R | G | B | A`).
    pub preview_channel: PreviewChannel,
    /// Last directory visited by an import or export dialog.
    pub last_dir: Option<PathBuf>,
    /// Last file path written by `export_to_path`.
    pub last_export_path: Option<PathBuf>,
    /// Set by [`ChannelAssemblerAction::ImportSlot`] so the file-dialog polling system opens a native file picker for
    /// that slot (`0..4`).
    pub pick_import_slot: Option<usize>,
    /// Set by [`ChannelAssemblerAction::ExportTexture`] so the file-dialog polling system opens a native save picker.
    pub pick_export: bool,
    /// Bumped whenever any slot, export setting, or preview channel changes so the UI and preview cache know when to
    /// rebuild.
    pub revision: u64,
}

impl Default for ChannelAssemblerDoc {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelAssemblerDoc {
    pub fn new() -> Self {
        let settings =
            TextureExportSettings { ktx2_color_space: Ktx2ColorSpace::Linear, ..TextureExportSettings::default() };
        Self {
            slots: ChannelSlot::default_slots(),
            settings,
            preview_channel: PreviewChannel::All,
            last_dir: None,
            last_export_path: None,
            pick_import_slot: None,
            pick_export: false,
            revision: 1,
        }
    }

    /// Populates all 4 slots with procedural demo patterns (`demo_assembler_slots`) and increments `revision`.
    pub fn load_demo(&mut self) {
        self.slots = demo_assembler_slots();
        self.revision += 1;
    }

    /// Loads a 2D `.png` or `.ktx2` file from `path` into `slots[slot]` (`0..4`) and increments `revision`.
    pub fn import_slot_from_path(&mut self, slot: usize, path: &Path) -> Result<(), String> {
        if slot >= 4 {
            return Err(format!("invalid channel slot index {slot} (expected 0..4)"));
        }
        let img = RgbaImage2d::from_file(path)?;
        self.apply_imported_slot(slot, path.to_path_buf(), img);
        Ok(())
    }

    /// Decodes a 2D `.png` or `.ktx2` byte buffer associated with `path` into `slots[slot]` (`0..4`) and increments
    /// `revision`.
    pub fn import_slot_from_bytes(&mut self, slot: usize, path: PathBuf, bytes: &[u8]) -> Result<(), String> {
        if slot >= 4 {
            return Err(format!("invalid channel slot index {slot} (expected 0..4)"));
        }
        let img = RgbaImage2d::from_bytes(bytes, &path)?;
        self.apply_imported_slot(slot, path, img);
        Ok(())
    }

    fn apply_imported_slot(&mut self, slot: usize, path: PathBuf, img: RgbaImage2d) {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            self.last_dir = Some(parent.to_path_buf());
        }
        self.slots[slot].set_image(path, img);
        self.revision += 1;
    }

    /// Clears the loaded image from `slots[slot]` (if any) and increments `revision`.
    pub fn clear_slot(&mut self, slot: usize) {
        if let Some(s) = self.slots.get_mut(slot)
            && (s.image.is_some() || s.path.is_some())
        {
            s.clear();
            self.revision += 1;
        }
    }

    /// Clears all 4 channel slots and increments `revision`.
    pub fn clear_all(&mut self) {
        let mut changed = false;
        for s in &mut self.slots {
            if s.image.is_some() || s.path.is_some() {
                s.clear();
                changed = true;
            }
        }
        if changed {
            self.revision += 1;
        }
    }

    /// Returns `true` if at least one channel slot has a loaded image.
    pub fn has_any_slot(&self) -> bool {
        self.slots.iter().any(|s| s.image.is_some())
    }

    /// Number of channel slots (`0..=4`) that currently have a loaded image.
    pub fn loaded_slot_count(&self) -> usize {
        self.slots.iter().filter(|s| s.image.is_some()).count()
    }

    /// Returns the `(width, height)` of the first loaded channel slot, or `None` if all slots are empty.
    pub fn base_dimensions(&self) -> Option<(u32, u32)> {
        self.slots.iter().find_map(|s| s.image.as_ref().map(|img| (img.width, img.height)))
    }

    /// Returns `true` when two or more loaded slots have different `(width, height)` dimensions.
    pub fn has_mixed_sizes(&self) -> bool {
        let Some(first) = self.base_dimensions() else { return false };
        self.slots.iter().filter_map(|s| s.image.as_ref()).any(|img| (img.width, img.height) != first)
    }

    /// Assembles all 4 slots into a single [`RgbaImage2d`] with `self.settings.resize` and `self.settings.flip_y`
    /// applied, returning `(assembled_image, mixed_sizes)` or `None` if no slot has an image.
    pub fn assembled_image(&self) -> Option<(RgbaImage2d, bool)> {
        assemble_channels(&self.slots, self.settings.resize, self.settings.flip_y)
    }

    /// Assembles the channels and encodes the result into `.ktx2` or `.png` bytes according to `self.settings`.
    pub fn export_bytes(&self) -> Result<Vec<u8>, String> {
        let (assembled, _) = self.assembled_image().ok_or_else(|| "no channel images loaded".to_string())?;
        match self.settings.output_format {
            OutputFormat::Ktx2 => encode_ktx2(&assembled, &self.settings),
            OutputFormat::Png => encode_png(&assembled),
        }
    }

    /// Assembles the channels, encodes `.ktx2` or `.png` bytes, and writes them to `path` (creating parent directories
    /// if needed), updating `last_export_path` and `last_dir`.
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

    /// Default export file name (`"packed_channels.ktx2"` or `"packed_channels.png"`).
    pub fn suggested_export_filename(&self) -> String {
        format!("packed_channels.{}", self.settings.output_format.extension())
    }

    /// Human-readable summary of `slots[slot]` (`"ao_dome.png (128x128, from R)"` or `"Empty (fill 0)"`).
    pub fn slot_summary(&self, slot: usize) -> String {
        let Some(s) = self.slots.get(slot) else {
            return "Empty".to_string();
        };
        if let Some(img) = &s.image {
            let name = s
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .filter(|n| !n.is_empty())
                .unwrap_or("image");
            let inv = if s.invert { ", inverted" } else { "" };
            format!("{name} ({}x{}, from {}{inv})", img.width, img.height, s.source_channel.label())
        } else {
            format!("Empty (fill {})", s.default_value)
        }
    }

    /// Human-readable summary of the assembled target output (`"128x128 | KTX2 | RGBA8 (Linear) | 8 mips | Zstd"` or
    /// `"No output (import at least one channel image)"`).
    pub fn output_summary(&self) -> String {
        let Some((w, h)) = self.base_dimensions() else {
            return "No output (import at least one channel image)".to_string();
        };
        self.settings.output_summary(w, h)
    }
}

/// Opens the `Texture Channel Assembler` tool tab.
pub fn open_channel_assembler_tool(_editor: &mut Editor) -> Box<dyn TabDoc> {
    Box::new(ChannelAssemblerDoc::new())
}

impl TabDoc for ChannelAssemblerDoc {
    fn apply(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        match action {
            TabAction::SetValue { key, value, .. } => {
                let Some(prop) = key.downcast::<ChannelAssemblerProp>() else { return Handled::No };
                match *prop {
                    ChannelAssemblerProp::SlotSourceChannel(slot) => {
                        if let Some(s) = self.slots.get_mut(slot)
                            && let Some(ch) = SourceChannel::from_label(value.as_str())
                            && s.source_channel != ch
                        {
                            s.source_channel = ch;
                            self.revision += 1;
                            ctx.redraw_props = true;
                        }
                    }
                    ChannelAssemblerProp::SlotInvert(slot) => {
                        let v = value.as_bool();
                        if let Some(s) = self.slots.get_mut(slot)
                            && s.invert != v
                        {
                            s.invert = v;
                            self.revision += 1;
                            ctx.redraw_props = true;
                        }
                    }
                    ChannelAssemblerProp::SlotDefaultValue(slot) => {
                        if let Some(s) = self.slots.get_mut(slot)
                            && let Some(def_val) = default_value_from_label(value.as_str())
                            && s.default_value != def_val
                        {
                            s.default_value = def_val;
                            self.revision += 1;
                            ctx.redraw_props = true;
                        }
                    }
                    ChannelAssemblerProp::OutputFormat => {
                        if let Some(fmt) = OutputFormat::from_label(value.as_str())
                            && self.settings.output_format != fmt
                        {
                            self.settings.output_format = fmt;
                            self.revision += 1;
                            ctx.redraw_props = true;
                        }
                    }
                    ChannelAssemblerProp::Resize => {
                        if let Some(preset) = ResizePreset::from_label(value.as_str())
                            && self.settings.resize != preset
                        {
                            self.settings.resize = preset;
                            self.revision += 1;
                            ctx.redraw_props = true;
                        }
                    }
                    ChannelAssemblerProp::FlipY => {
                        let v = value.as_bool();
                        if self.settings.flip_y != v {
                            self.settings.flip_y = v;
                            self.revision += 1;
                            ctx.redraw_props = true;
                        }
                    }
                    ChannelAssemblerProp::Ktx2ColorSpace => {
                        if let Some(cs) = Ktx2ColorSpace::from_label(value.as_str())
                            && self.settings.ktx2_color_space != cs
                        {
                            self.settings.ktx2_color_space = cs;
                            self.revision += 1;
                            ctx.redraw_props = true;
                        }
                    }
                    ChannelAssemblerProp::Mipmaps => {
                        let v = value.as_bool();
                        if self.settings.mipmaps != v {
                            self.settings.mipmaps = v;
                            self.revision += 1;
                            ctx.redraw_props = true;
                        }
                    }
                    ChannelAssemblerProp::Zstd => {
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
                let Some(act) = payload.downcast::<ChannelAssemblerAction>() else { return Handled::No };
                match *act {
                    ChannelAssemblerAction::ImportSlot(slot) => {
                        if slot < 4 {
                            self.pick_import_slot = Some(slot);
                        }
                    }
                    ChannelAssemblerAction::ClearSlot(slot) => {
                        self.clear_slot(slot);
                        ctx.redraw_props = true;
                    }
                    ChannelAssemblerAction::ClearAll => {
                        self.clear_all();
                        ctx.redraw_props = true;
                    }
                    ChannelAssemblerAction::ExportTexture => {
                        if self.has_any_slot() {
                            self.pick_export = true;
                        }
                    }
                    ChannelAssemblerAction::SetPreviewChannel(ch) => {
                        if self.preview_channel != ch {
                            self.preview_channel = ch;
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
        match *key.downcast::<ChannelAssemblerProp>()? {
            ChannelAssemblerProp::SlotSourceChannel(slot) => {
                self.slots.get(slot).map(|s| PropValue::Str(s.source_channel.label().to_string()))
            }
            ChannelAssemblerProp::SlotInvert(slot) => self.slots.get(slot).map(|s| PropValue::Bool(s.invert)),
            ChannelAssemblerProp::SlotDefaultValue(slot) => {
                self.slots.get(slot).map(|s| PropValue::Str(default_value_label(s.default_value).to_string()))
            }
            ChannelAssemblerProp::OutputFormat => Some(PropValue::Str(self.settings.output_format.label().to_string())),
            ChannelAssemblerProp::Resize => Some(PropValue::Str(self.settings.resize.label().to_string())),
            ChannelAssemblerProp::FlipY => Some(PropValue::Bool(self.settings.flip_y)),
            ChannelAssemblerProp::Ktx2ColorSpace => {
                Some(PropValue::Str(self.settings.ktx2_color_space.label().to_string()))
            }
            ChannelAssemblerProp::Mipmaps => Some(PropValue::Bool(self.settings.mipmaps)),
            ChannelAssemblerProp::Zstd => Some(PropValue::Bool(self.settings.zstd)),
        }
    }

    fn asset(&self) -> Option<&AssetRef> {
        None
    }

    fn view(&self) -> ViewKindId {
        CHANNEL_ASSEMBLER_VIEW
    }

    fn base_title(&self) -> String {
        "Texture Channel Assembler".into()
    }

    fn icon(&self) -> IconRef {
        IconId::Image.into()
    }

    fn watermark(&self) -> Option<&'static str> {
        Some("Texture Channel Assembler")
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
        Some(CHANNEL_ASSEMBLER_TOOL)
    }

    fn status(&self, ctx: &StatusCtx) -> Option<StatusInfo> {
        let mut counts = if self.has_any_slot() {
            let mut c = vec![format!("{}/4 channels loaded", self.loaded_slot_count()), self.output_summary()];
            if self.has_mixed_sizes() {
                c.push("Inputs resampled".to_string());
            }
            c
        } else {
            vec!["No channel images loaded".to_string()]
        };
        if let Some(exported) = &self.last_export_path {
            let name = exported.file_name().and_then(|s| s.to_str()).unwrap_or("exported");
            counts.push(format!("Exported: {name}"));
        }
        Some(StatusInfo::new(false, ctx, counts))
    }
}

/// The `Texture Channel Assembler` editor kind.
pub struct ChannelAssemblerEditor;

/// The editor topbar row of the `Texture Channel Assembler` tab.
#[derive(Component, Default, Clone)]
pub struct ChannelAssemblerBar;

/// The right sidebar body of the `Texture Channel Assembler` tab.
#[derive(Component, Default, Clone)]
pub struct ChannelAssemblerSidebarBody;

/// The left column container (4 channel slot subpanes + `Output Settings` subpane).
#[derive(Component, Default, Clone)]
pub struct ChannelAssemblerLeftPane;

/// The right column container (`Assembled Output Preview` subpane + footer).
#[derive(Component, Default, Clone)]
pub struct ChannelAssemblerRightPane;

/// Checkerboard frame holding the thumbnail for channel slot `0..4`.
#[derive(Component, Default, Clone, Copy)]
pub struct ChannelAssemblerSlotThumbFrame(pub usize);

/// The UI [`ImageNode`] displaying the thumbnail for channel slot `0..4`.
#[derive(Component, Default, Clone, Copy)]
pub struct ChannelAssemblerSlotThumbImage(pub usize);

/// The large 2D preview viewport container in the right pane.
#[derive(Component, Default, Clone)]
pub struct ChannelAssemblerPreviewBox;

/// Checkerboard frame inside [`ChannelAssemblerPreviewBox`] sized to preserve the assembled image's aspect ratio.
#[derive(Component, Default, Clone)]
pub struct ChannelAssemblerOutputFrame;

/// The UI [`ImageNode`] displaying the assembled and channel-filtered output preview.
#[derive(Component, Default, Clone)]
pub struct ChannelAssemblerOutputImage;

/// Caches per-slot thumbnails, the assembled/channel-filtered output preview [`Image`], and a 2x2 checkerboard
/// background [`Image`] for the active [`ChannelAssemblerDoc`].
#[derive(Resource, Default)]
pub struct ChannelAssemblerPreviewCache {
    pub key: Option<(usize, u64, PreviewChannel)>,
    pub slot_thumbs: [Option<Handle<Image>>; 4],
    pub output_preview: Option<Handle<Image>>,
    pub mixed_sizes: bool,
    pub out_dims: (u32, u32),
    pub checker_handle: Option<Handle<Image>>,
}

impl ChannelAssemblerPreviewCache {
    /// Removes cached `slot_thumbs` and `output_preview` handles from `images`.
    pub fn clear_images(&mut self, images: &mut Assets<Image>) {
        self.key = None;
        for h in &mut self.slot_thumbs {
            if let Some(handle) = h.take() {
                images.remove(handle.id());
            }
        }
        if let Some(h) = self.output_preview.take() {
            images.remove(h.id());
        }
        self.mixed_sizes = false;
        self.out_dims = (0, 0);
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

    /// Synchronizes cached slot thumbnails and assembled output preview, rebuilding GPU [`Image`] assets only when
    /// `(tab_id, doc.revision, doc.preview_channel)` changes.
    pub fn sync(&mut self, tab_id: usize, doc: &ChannelAssemblerDoc, images: &mut Assets<Image>) -> bool {
        if !doc.has_any_slot() {
            self.clear_images(images);
            return false;
        }
        let _ = self.ensure_checker(images);
        let want_key = (tab_id, doc.revision, doc.preview_channel);
        if self.key == Some(want_key)
            && let Some(out) = &self.output_preview
            && images.contains(out.id())
        {
            return true;
        }

        self.clear_images(images);
        let Some((assembled, mixed_sizes)) = doc.assembled_image() else {
            return false;
        };

        for (i, slot) in doc.slots.iter().enumerate() {
            if let Some(src) = &slot.image {
                let thumb = rgba_to_bevy_image(src, PreviewChannel::All);
                self.slot_thumbs[i] = Some(images.add(thumb));
            }
        }

        let out_dims = (assembled.width, assembled.height);
        let out_img = rgba_to_bevy_image(&assembled, doc.preview_channel);
        self.output_preview = Some(images.add(out_img));
        self.mixed_sizes = mixed_sizes;
        self.out_dims = out_dims;
        self.key = Some(want_key);
        true
    }
}

impl EditorAreas for ChannelAssemblerEditor {
    const VIEW: ViewKindId = CHANNEL_ASSEMBLER_VIEW;
    const SIDEBAR_TITLE: &'static str = "Texture Channel Assembler";

    fn topbar() -> Option<impl Scene> {
        Some(bsn! { @bar_row() ChannelAssemblerBar })
    }

    fn sidebar() -> Option<impl Scene> {
        Some(bsn! { @sidebar_body() ChannelAssemblerSidebarBody })
    }

    fn build(app: &mut App) {
        app.init_resource::<ChannelAssemblerPreviewCache>().add_systems(
            Update,
            (
                redraw_bar,
                redraw_left,
                redraw_right,
                redraw_sidebar,
                ApplyDeferred,
                update_channel_assembler_preview,
                poll_channel_assembler_dialogs,
            )
                .chain()
                .after(editor_core::apply_actions),
        );
    }
}

fn active(editor: &Editor) -> Option<(usize, &ChannelAssemblerDoc)> {
    let i = editor.active?;
    Some((i, editor.tabs.get(i)?.get::<ChannelAssemblerDoc>()?))
}

/// The `Texture Channel Assembler` 2-column view in the editor content
/// (`{left: R/G/B/A slots + Output Settings} | {right: Assembled Output Preview}`).
pub fn channel_assembler_view() -> impl Scene {
    bsn! {
        ViewOf(CHANNEL_ASSEMBLER_VIEW)
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
            ChannelAssemblerLeftPane
            Node {
                width: px(380),
                flex_shrink: 0.0,
                min_height: px(0),
                flex_direction: FlexDirection::Column,
                row_gap: px(10),
            }
            --
            ChannelAssemblerRightPane
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
        action: ChannelAssemblerAction::SetPreviewChannel(ch).into(),
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
        format!("Assemble channels and export the 2D texture to a standalone .{} file", format.extension())
    } else {
        "Import at least one channel image first to export".into()
    }
}

fn tool_button_with_state(
    icon_id: IconId,
    label: &'static str,
    action: Action,
    tip_text: String,
    enabled: bool,
) -> Box<dyn Scene> {
    let tint = if enabled { TEXT } else { TEXT.with_alpha(DISABLED_ICON_ALPHA) };
    let act = OnActivate(action);
    let tip = Tooltip(tip_text);
    if enabled {
        Box::new(bsn! {
            @FeathersToolButton {
                @variant: ButtonVariant::Plain,
                @caption: bsn_list! { @icon(icon_id, 16.0, tint) Node { margin: UiRect::right(px(4)) } -- @caption(label) },
            }
            Node { flex_shrink: 0.0 }
            act
            tip
        })
    } else {
        Box::new(bsn! {
            @FeathersToolButton {
                @variant: ButtonVariant::Plain,
                @caption: bsn_list! { @icon(icon_id, 16.0, tint) Node { margin: UiRect::right(px(4)) } -- @caption(label) },
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
    bar: Query<Entity, With<ChannelAssemblerBar>>,
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

    let has_any = doc.has_any_slot();
    let ch_group = preview_channel_group(doc.preview_channel, has_any);
    let clear_tip =
        if has_any { "Clear all loaded channel images".to_string() } else { "No channel images loaded".to_string() };
    let clear_btn = tool_button_with_state(
        IconId::Close,
        "Clear All",
        Action::from(ChannelAssemblerAction::ClearAll),
        clear_tip,
        has_any,
    );
    let export_btn = tool_button_with_state(
        IconId::Save,
        export_button_label(doc.settings.output_format),
        Action::from(ChannelAssemblerAction::ExportTexture),
        export_button_tip(doc.settings.output_format, has_any),
        has_any,
    );

    commands.spawn_scene(bsn! { @{ch_group} ChildOf(bar) });
    commands.spawn_scene(bsn! { @{clear_btn} ChildOf(bar) });
    commands.spawn_scene(bsn! { @{export_btn} ChildOf(bar) });
}

fn build_slot_subpane(doc: &ChannelAssemblerDoc, slot_idx: usize) -> Box<dyn Scene> {
    let slot = &doc.slots[slot_idx];
    let has_img = slot.image.is_some();
    let import_btn = icon_tool_button(
        IconId::FolderOpen,
        "Import...",
        Action::from(ChannelAssemblerAction::ImportSlot(slot_idx)),
        format!("Import a 2D .png, .jpg, or .ktx2 image for {}", CHANNEL_TITLES[slot_idx]),
    );
    let clear_btn: Box<dyn Scene> = if has_img {
        Box::new(icon_tool_button(
            IconId::Close,
            "Clear",
            Action::from(ChannelAssemblerAction::ClearSlot(slot_idx)),
            format!("Clear the loaded image from {}", CHANNEL_TITLES[slot_idx]),
        ))
    } else {
        Box::new(bsn! {})
    };

    let thumb_inner: Box<dyn Scene> = if has_img {
        Box::new(bsn! {
            ImageNode
            ChannelAssemblerSlotThumbFrame(slot_idx)
            Node {
                width: percent(100),
                height: percent(100),
            }
            Pickable::IGNORE
            Children [
                ImageNode
                ChannelAssemblerSlotThumbImage(slot_idx)
                Node {
                    width: percent(100),
                    height: percent(100),
                }
                Pickable::IGNORE
            ]
        })
    } else {
        let short = CHANNEL_SHORT[slot_idx];
        Box::new(bsn! {
            @text(short, TEXT_DIM, 11.0)
        })
    };

    let summary_color = if has_img { TEXT } else { TEXT_DIM };
    let summary_label = doc.slot_summary(slot_idx);

    let header_row: Box<dyn Scene> = Box::new(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            column_gap: px(8),
        }
        Children [
            Node {
                flex_grow: 1.0,
                min_width: px(0),
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
            }
            Children [
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(6),
                }
                Children [
                    @{import_btn}
                    --
                    @{clear_btn}
                ]
                --
                Node {
                    min_width: px(0),
                    overflow: Overflow::clip(),
                }
                Children [
                    @text(summary_label, summary_color, 11.5)
                ]
            ]
            --
            Node {
                width: px(44),
                height: px(44),
                flex_shrink: 0.0,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(4)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                overflow: Overflow::clip(),
            }
            BorderColor::all(BORDER)
            BackgroundColor(Color::srgb_u8(14, 16, 21))
            Children [ @{thumb_inner} ]
        ]
    });

    let mut rows: Vec<Box<dyn Scene>> = vec![header_row];

    if has_img {
        let src_channels: Vec<String> = SourceChannel::ALL.iter().map(|c| c.label().to_string()).collect();
        let src_idx = SourceChannel::ALL.iter().position(|&c| c == slot.source_channel).unwrap_or(0);
        let src_select = select_values(
            ChannelAssemblerProp::SlotSourceChannel(slot_idx),
            src_channels.clone(),
            src_channels,
            src_idx,
        );
        rows.push(Box::new(labeled("Take From".into(), src_select)));
        rows.push(checkbox_row(
            "Invert (1 - x)",
            PropKey::new(ChannelAssemblerProp::SlotInvert(slot_idx)),
            slot.invert,
        ));
    } else {
        let fill_options: Vec<String> = DEFAULT_FILL_LABELS.iter().map(|s| (*s).to_string()).collect();
        let fill_idx = if slot.default_value == 0 { 0 } else { 1 };
        let fill_select = select_values(
            ChannelAssemblerProp::SlotDefaultValue(slot_idx),
            fill_options.clone(),
            fill_options,
            fill_idx,
        );
        rows.push(Box::new(labeled("Empty Fill".into(), fill_select)));
    }

    subpane_box(CHANNEL_TITLES[slot_idx], rows)
}

fn redraw_left(
    editor: Res<Editor>,
    left: Query<Entity, With<ChannelAssemblerLeftPane>>,
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
    let Ok(left) = left.single() else { return };
    *last = Some(key);
    commands.entity(left).despawn_children();

    let slot0 = build_slot_subpane(doc, 0);
    let slot1 = build_slot_subpane(doc, 1);
    let slot2 = build_slot_subpane(doc, 2);
    let slot3 = build_slot_subpane(doc, 3);

    let formats: Vec<String> = OutputFormat::ALL.iter().map(|f| f.label().to_string()).collect();
    let fmt_idx = OutputFormat::ALL.iter().position(|&f| f == doc.settings.output_format).unwrap_or(0);
    let fmt_select = select_values(ChannelAssemblerProp::OutputFormat, formats.clone(), formats, fmt_idx);

    let resizes: Vec<String> = ResizePreset::ALL.iter().map(|r| r.label().to_string()).collect();
    let resize_idx = ResizePreset::ALL.iter().position(|&r| r == doc.settings.resize).unwrap_or(0);
    let resize_select = select_values(ChannelAssemblerProp::Resize, resizes.clone(), resizes, resize_idx);

    let mut settings_rows: Vec<Box<dyn Scene>> = vec![
        Box::new(labeled("Output Format".into(), fmt_select)),
        Box::new(labeled("Resolution".into(), resize_select)),
        checkbox_row("Flip Vertically (Y)", PropKey::new(ChannelAssemblerProp::FlipY), doc.settings.flip_y),
    ];

    if doc.settings.output_format == OutputFormat::Ktx2 {
        let spaces: Vec<String> = Ktx2ColorSpace::ALL.iter().map(|c| c.label().to_string()).collect();
        let space_idx = Ktx2ColorSpace::ALL.iter().position(|&c| c == doc.settings.ktx2_color_space).unwrap_or(0);
        let space_select = select_values(ChannelAssemblerProp::Ktx2ColorSpace, spaces.clone(), spaces, space_idx);
        settings_rows.push(Box::new(labeled("Color Space".into(), space_select)));
        settings_rows.push(checkbox_row_tip(
            "Generate Mipmaps",
            PropKey::new(ChannelAssemblerProp::Mipmaps),
            doc.settings.mipmaps,
            "Generate a full 2x2 box-filtered mip chain down to 1x1",
        ));
        settings_rows.push(checkbox_row_tip(
            "Zstd Compression",
            PropKey::new(ChannelAssemblerProp::Zstd),
            doc.settings.zstd,
            "Apply KTX2 Zstandard supercompression (level Fastest) to all mip levels",
        ));
    }

    let settings_pane = subpane_box("Output Settings", settings_rows);

    commands.spawn_scene(bsn! {
        @scroll_column()
        Node { padding: UiRect::all(px(0)), row_gap: px(10) }
        Children [
            @{slot0}
            --
            @{slot1}
            --
            @{slot2}
            --
            @{slot3}
            --
            @{settings_pane}
        ]
        ChildOf(left)
    });
}

fn redraw_right(
    editor: Res<Editor>,
    right: Query<Entity, With<ChannelAssemblerRightPane>>,
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

    let viewport_content: Box<dyn Scene> = if let Some((base_w, base_h)) = doc.base_dimensions() {
        let (out_w, out_h) = doc.settings.resize.resolve(base_w, base_h);
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
                ChannelAssemblerOutputFrame
                Node {
                    width: px(fit_w.round()),
                    height: px(fit_h.round()),
                    border_radius: BorderRadius::all(px(4)),
                    overflow: Overflow::clip(),
                }
                Pickable::IGNORE
                Children [
                    ImageNode
                    ChannelAssemblerOutputImage
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
            @text("Import at least one channel image on the left to preview the assembled output", TEXT_DIM, 13.0)
        })
    };

    let preview_body = bsn! {
        ChannelAssemblerPreviewBox
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

    let preview_pane = subpane_fill("Assembled Output Preview", preview_body);
    commands.spawn_scene(bsn! { @{preview_pane} ChildOf(right) });
}

fn redraw_sidebar(
    editor: Res<Editor>,
    body: Query<Entity, With<ChannelAssemblerSidebarBody>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some((_, doc)) = active(&editor) else { return key.forget() };
    fill_sidebar_with(&mut commands, &body, &mut key, SidebarKey::now(&editor), || sidebar_rows(doc));
}

fn sidebar_rows(doc: &ChannelAssemblerDoc) -> Vec<Box<dyn Scene>> {
    let mut info: Vec<Box<dyn Scene>> = Vec::with_capacity(7);
    for (idx, short) in CHANNEL_SHORT.iter().enumerate() {
        let color = if doc.slots[idx].image.is_some() { TEXT } else { TEXT_DIM };
        info.push(Box::new(text(format!("{short}: {}", doc.slot_summary(idx)), color, 12.0)));
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
    vec![subpane_box("Texture Channel Assembler", info)]
}

/// Syncs the active [`ChannelAssemblerDoc`]'s per-slot thumbnails and assembled/channel-filtered output preview
/// [`Image`] handles, and fits the output preview frame inside [`ChannelAssemblerPreviewBox`].
pub fn update_channel_assembler_preview(world: &mut World) {
    let has_any_assembler_tab = world
        .get_resource::<Editor>()
        .is_some_and(|editor| editor.tabs.iter().any(|t| t.get::<ChannelAssemblerDoc>().is_some()));

    if !has_any_assembler_tab {
        if world.get_resource::<ChannelAssemblerPreviewCache>().is_some_and(|c| c.key.is_some()) {
            world.resource_scope(|world, mut cache: Mut<ChannelAssemblerPreviewCache>| {
                if let Some(mut images) = world.get_resource_mut::<Assets<Image>>() {
                    cache.clear_images(&mut images);
                }
            });
        }
        return;
    }

    if !world.contains_resource::<ChannelAssemblerPreviewCache>() {
        world.init_resource::<ChannelAssemblerPreviewCache>();
    }
    if !world.contains_resource::<Assets<Image>>() {
        world.init_resource::<Assets<Image>>();
    }

    let synced_state = world.resource_scope(|world, mut cache: Mut<ChannelAssemblerPreviewCache>| {
        world.resource_scope(|world, mut images: Mut<Assets<Image>>| {
            let editor = world.get_resource::<Editor>()?;
            let (tab_id, doc) = active(editor)?;
            if !cache.sync(tab_id, doc, &mut images) {
                return None;
            }
            Some((
                cache.out_dims,
                cache.slot_thumbs.clone(),
                cache.output_preview.clone()?,
                cache.checker_handle.clone()?,
            ))
        })
    });

    let Some(((out_w, out_h), slot_thumbs, out_handle, checker_handle)) = synced_state else {
        return;
    };

    let tiled = NodeImageMode::Tiled { tile_x: true, tile_y: true, stretch_value: 6.0 };

    {
        let mut q = world.query::<(&ChannelAssemblerSlotThumbFrame, &mut ImageNode)>();
        for (_, mut img_node) in q.iter_mut(world) {
            if img_node.image != checker_handle {
                img_node.image = checker_handle.clone();
                img_node.image_mode = tiled.clone();
            }
        }
    }
    {
        let mut q = world.query::<(&ChannelAssemblerSlotThumbImage, &mut ImageNode)>();
        for (slot, mut img_node) in q.iter_mut(world) {
            if let Some(Some(thumb)) = slot_thumbs.get(slot.0)
                && img_node.image != *thumb
            {
                img_node.image = thumb.clone();
            }
        }
    }
    {
        let mut q = world.query_filtered::<&mut ImageNode, With<ChannelAssemblerOutputFrame>>();
        for mut img_node in q.iter_mut(world) {
            if img_node.image != checker_handle {
                img_node.image = checker_handle.clone();
                img_node.image_mode = tiled.clone();
            }
        }
    }
    {
        let mut q = world.query_filtered::<&mut ImageNode, With<ChannelAssemblerOutputImage>>();
        for mut img_node in q.iter_mut(world) {
            if img_node.image != out_handle {
                img_node.image = out_handle.clone();
            }
        }
    }

    // Fit the output preview frame inside ChannelAssemblerPreviewBox while preserving aspect ratio.
    let box_size: Option<Vec2> = {
        let mut q = world.query_filtered::<&ComputedNode, With<ChannelAssemblerPreviewBox>>();
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
        let mut q = world.query_filtered::<&mut Node, With<ChannelAssemblerOutputFrame>>();
        for mut node in q.iter_mut(world) {
            if node.width != w || node.height != h {
                node.width = w;
                node.height = h;
            }
        }
    }
}

enum PickedAssemblerDialog {
    ImportSlot(usize, Option<PathBuf>),
    Export(Option<PathBuf>),
}

fn poll_channel_assembler_dialogs(
    mut editor: ResMut<Editor>,
    mut pending: Local<Option<std::sync::mpsc::Receiver<PickedAssemblerDialog>>>,
) {
    if let Some(rx) = pending.as_ref() {
        match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if editor
                    .active_doc::<ChannelAssemblerDoc>()
                    .is_some_and(|d| d.pick_import_slot.is_some() || d.pick_export)
                    && let Some(doc) = editor.active_doc_mut::<ChannelAssemblerDoc>()
                {
                    doc.pick_import_slot = None;
                    doc.pick_export = false;
                }
                return;
            }
            Ok(PickedAssemblerDialog::ImportSlot(slot, Some(path))) => {
                let res = editor
                    .active_doc_mut::<ChannelAssemblerDoc>()
                    .map(|doc| doc.import_slot_from_path(slot, &path).map(|()| doc.slot_summary(slot)));
                if let Some(res) = res {
                    let ch_name = CHANNEL_SHORT.get(slot).copied().unwrap_or("?");
                    match res {
                        Ok(summary) => {
                            editor.domain_msg(
                                Some(MATERIAL),
                                Level::Info,
                                format!("Imported channel {ch_name} ({summary}) from {}", path.display()),
                            );
                            editor.ui.props += 1;
                            editor.ui.tabs += 1;
                        }
                        Err(e) => {
                            editor.domain_msg(
                                Some(MATERIAL),
                                Level::Error,
                                format!("Cannot import channel {ch_name} from {}: {e}", path.display()),
                            );
                        }
                    }
                }
            }
            Ok(PickedAssemblerDialog::Export(Some(path))) => {
                let res = editor.active_doc_mut::<ChannelAssemblerDoc>().map(|doc| doc.export_to_path(&path));
                if let Some(res) = res {
                    match res {
                        Ok(bytes) => {
                            editor.domain_msg(
                                Some(MATERIAL),
                                Level::Info,
                                format!("Exported assembled texture to {} ({bytes} bytes)", path.display()),
                            );
                            editor.ui.props += 1;
                            editor.ui.tabs += 1;
                        }
                        Err(e) => {
                            editor.domain_msg(
                                Some(MATERIAL),
                                Level::Error,
                                format!("Cannot export assembled texture to {}: {e}", path.display()),
                            );
                        }
                    }
                }
            }
            Ok(PickedAssemblerDialog::ImportSlot(_, None) | PickedAssemblerDialog::Export(None)) | Err(_) => {}
        }
        *pending = None;
    }

    if !editor.active_doc::<ChannelAssemblerDoc>().is_some_and(|d| d.pick_import_slot.is_some() || d.pick_export) {
        return;
    }
    let project_root = editor.project.as_ref().map(|p| p.root.clone());
    let Some(doc) = editor.active_doc_mut::<ChannelAssemblerDoc>() else { return };
    let want_import = doc.pick_import_slot.take();
    let want_export = std::mem::take(&mut doc.pick_export) && doc.has_any_slot();
    if want_import.is_none() && !want_export {
        return;
    }
    if std::env::var_os("SHADER_EDITOR_SCREENSHOT").is_some() {
        return;
    }

    let last_dir = doc.last_dir.clone();
    let slot_dir = doc.slots.iter().find_map(|s| s.path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)));
    let default_name = doc.suggested_export_filename();
    let out_fmt = doc.settings.output_format;
    let (tx, rx) = std::sync::mpsc::channel();
    *pending = Some(rx);

    if let Some(slot) = want_import {
        let start = last_dir
            .or(slot_dir)
            .or_else(|| project_root.as_ref().map(|r| r.join("assets/textures")).filter(|d| d.is_dir()))
            .or_else(|| project_root.as_ref().map(|r| r.join("assets")).filter(|d| d.is_dir()))
            .or(project_root)
            .or_else(std::env::home_dir);
        let title = format!("Import {} Image", CHANNEL_TITLES.get(slot).copied().unwrap_or("Channel"));
        std::thread::spawn(move || {
            let mut dialog = rfd::AsyncFileDialog::new()
                .set_title(&title)
                .add_filter("2D Texture (.png, .jpg, .jpeg, .ktx2)", &["png", "jpg", "jpeg", "ktx2"]);
            if let Some(dir) = start {
                dialog = dialog.set_directory(dir);
            }
            let picked = bevy::tasks::block_on(dialog.pick_file());
            drop(tx.send(PickedAssemblerDialog::ImportSlot(slot, picked.map(|h| h.path().to_path_buf()))));
        });
    } else if want_export {
        let start = last_dir
            .or_else(|| project_root.as_ref().map(|r| r.join("assets/textures")).filter(|d| d.is_dir()))
            .or_else(|| project_root.as_ref().map(|r| r.join("assets")).filter(|d| d.is_dir()))
            .or(project_root)
            .or(slot_dir)
            .or_else(std::env::home_dir);
        let ext = out_fmt.extension();
        let (filter_name, title) = match out_fmt {
            OutputFormat::Ktx2 => ("KTX2 Texture (.ktx2)", "Export Assembled KTX2 Texture"),
            OutputFormat::Png => ("PNG Image (.png)", "Export Assembled PNG Texture"),
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
            drop(tx.send(PickedAssemblerDialog::Export(picked.map(|h| {
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

    #[test]
    fn default_doc_and_load_demo_summaries() {
        let mut doc = ChannelAssemblerDoc::new();
        assert!(!doc.has_any_slot());
        assert_eq!(doc.loaded_slot_count(), 0);
        assert_eq!(doc.settings.ktx2_color_space, Ktx2ColorSpace::Linear);
        assert_eq!(doc.slot_summary(0), "Empty (fill 0)");
        assert_eq!(doc.slot_summary(1), "Empty (fill 0)");
        assert_eq!(doc.slot_summary(2), "Empty (fill 0)");
        assert_eq!(doc.slot_summary(3), "Empty (fill 255)");
        assert_eq!(doc.output_summary(), "No output (import at least one channel image)");
        assert_eq!(doc.suggested_export_filename(), "packed_channels.ktx2");
        assert_eq!(doc.status(&StatusCtx::default()).unwrap().counts, vec!["No channel images loaded".to_string()]);

        let rev0 = doc.revision;
        doc.load_demo();
        assert!(doc.revision > rev0);
        assert!(doc.has_any_slot());
        assert_eq!(doc.loaded_slot_count(), 4);
        assert!(!doc.has_mixed_sizes());
        assert_eq!(doc.slot_summary(0), "ao_dome.png (128x128, from R)");
        assert_eq!(doc.output_summary(), "128x128 | KTX2 | RGBA8 (Linear) | 8 mips | Zstd");
        assert_eq!(
            doc.status(&StatusCtx::default()).unwrap().counts,
            vec!["4/4 channels loaded".to_string(), "128x128 | KTX2 | RGBA8 (Linear) | 8 mips | Zstd".to_string()]
        );
    }

    #[test]
    fn slot_import_clear_extraction_invert_default_fill_and_export_roundtrip() {
        let mut doc = ChannelAssemblerDoc::new();

        // Slot 0 (R): 4x4 image where G channel is 120, inverted -> 255 - 120 = 135.
        let img_r = RgbaImage2d::new(4, 4, vec![[10, 120, 30, 255]; 16]);
        let png_r = encode_png(&img_r).unwrap();
        doc.import_slot_from_bytes(0, PathBuf::from("masks/ao.png"), &png_r).unwrap();

        // Slot 2 (B): 8x8 image (mixed size!) where B channel is 200.
        let img_b = RgbaImage2d::new(8, 8, vec![[0, 0, 200, 255]; 64]);
        let png_b = encode_png(&img_b).unwrap();
        doc.import_slot_from_bytes(2, PathBuf::from("masks/metallic.png"), &png_b).unwrap();

        assert_eq!(doc.loaded_slot_count(), 2);
        assert!(doc.has_mixed_sizes());
        assert_eq!(doc.base_dimensions(), Some((4, 4)));
        assert!(doc.status(&StatusCtx::default()).unwrap().counts.contains(&"Inputs resampled".to_string()));

        // Configure Slot 0: Take From G + Invert; Slot 1 (empty): Empty Fill 255 (White); Slot 2: Take From B.
        doc.slots[0].source_channel = SourceChannel::G;
        doc.slots[0].invert = true;
        doc.slots[1].default_value = 255;
        doc.slots[2].source_channel = SourceChannel::B;
        assert_eq!(doc.slot_summary(0), "ao.png (4x4, from G, inverted)");
        assert_eq!(doc.slot_summary(1), "Empty (fill 255)");

        let dir = tempfile::tempdir().unwrap();

        // Export KTX2 (Linear, with mipmaps and Zstd).
        let ktx2_path = dir.path().join("packed/orm.ktx2");
        let ktx2_len = doc.export_to_path(&ktx2_path).expect("export KTX2");
        assert!(ktx2_len > 80);
        assert!(doc.status(&StatusCtx::default()).unwrap().counts.contains(&"Exported: orm.ktx2".to_string()));
        let decoded_ktx2 = RgbaImage2d::from_file(&ktx2_path).expect("decode exported KTX2");
        assert_eq!((decoded_ktx2.width, decoded_ktx2.height), (4, 4));
        assert!(!decoded_ktx2.source_is_srgb);
        assert_eq!(decoded_ktx2.source_mips, 3); // 4x4 -> 3 mips
        assert!(decoded_ktx2.pixels.iter().all(|&px| px == [135, 255, 200, 255]));

        // Export PNG with ResizePreset::Size256.
        doc.settings.output_format = OutputFormat::Png;
        doc.settings.resize = ResizePreset::Size256;
        assert_eq!(doc.suggested_export_filename(), "packed_channels.png");
        let png_path = dir.path().join("packed/orm.png");
        let png_len = doc.export_to_path(&png_path).expect("export PNG");
        assert!(png_len > 0);
        let decoded_png = RgbaImage2d::from_file(&png_path).expect("decode exported PNG");
        assert_eq!((decoded_png.width, decoded_png.height), (256, 256));
        assert!(decoded_png.pixels.iter().all(|&px| px == [135, 255, 200, 255]));

        // Import a JPEG (.jpg) into Slot 1 (G) and verify slot summary.
        let mut jpg_bytes = Vec::new();
        {
            let rgb = [90u8, 180, 45].repeat(4 * 4);
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg_bytes, 100);
            enc.encode(&rgb, 4, 4, image::ExtendedColorType::Rgb8).expect("encode JPEG");
        }
        doc.import_slot_from_bytes(1, PathBuf::from("masks/roughness.jpg"), &jpg_bytes).expect("import JPEG slot");
        assert_eq!(doc.slot_summary(1), "roughness.jpg (4x4, from R)");
        doc.clear_slot(1);

        // Clear slot 2 -> mixed_sizes becomes false.
        doc.clear_slot(2);
        assert_eq!(doc.loaded_slot_count(), 1);
        assert!(!doc.has_mixed_sizes());

        // Clear all -> no slots loaded.
        doc.clear_all();
        assert!(!doc.has_any_slot());
        assert!(doc.export_bytes().is_err());
    }

    #[test]
    fn tab_doc_properties_and_actions_update_slots_and_settings() {
        let mut editor = crate::test_editor();
        editor.open_tool(MATERIAL, CHANNEL_ASSEMBLER_TOOL, None);
        {
            let doc = editor.active_doc_mut::<ChannelAssemblerDoc>().expect("active assembler doc");
            doc.load_demo();
        }

        // Change Slot 0 source channel to Luminance and invert it.
        editor.apply(Action::Tab(TabAction::SetValue {
            key: PropKey::new(ChannelAssemblerProp::SlotSourceChannel(0)),
            value: PropValue::Str(SourceChannel::Luminance.label().into()),
            merge: false,
        }));
        editor.apply(Action::Tab(TabAction::SetValue {
            key: PropKey::new(ChannelAssemblerProp::SlotInvert(0)),
            value: PropValue::Bool(true),
            merge: false,
        }));
        {
            let doc = editor.active_doc::<ChannelAssemblerDoc>().unwrap();
            assert_eq!(doc.slots[0].source_channel, SourceChannel::Luminance);
            assert!(doc.slots[0].invert);
        }

        // Clear Slot 1 and set its Empty Fill to 255 (White).
        editor.apply(Action::from(ChannelAssemblerAction::ClearSlot(1)));
        editor.apply(Action::Tab(TabAction::SetValue {
            key: PropKey::new(ChannelAssemblerProp::SlotDefaultValue(1)),
            value: PropValue::Str("255 (White)".into()),
            merge: false,
        }));
        {
            let doc = editor.active_doc::<ChannelAssemblerDoc>().unwrap();
            assert!(doc.slots[1].image.is_none());
            assert_eq!(doc.slots[1].default_value, 255);
        }

        // Set preview channel to G and ClearAll.
        editor.apply(Action::from(ChannelAssemblerAction::SetPreviewChannel(PreviewChannel::G)));
        assert_eq!(editor.active_doc::<ChannelAssemblerDoc>().unwrap().preview_channel, PreviewChannel::G);

        editor.apply(Action::from(ChannelAssemblerAction::ClearAll));
        assert!(!editor.active_doc::<ChannelAssemblerDoc>().unwrap().has_any_slot());
    }

    #[test]
    fn preview_cache_rebuilds_and_cleans_up_slot_and_output_handles() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<ChannelAssemblerPreviewCache>();
        let mut editor = crate::test_editor();
        editor.open_tool(MATERIAL, CHANNEL_ASSEMBLER_TOOL, None);
        editor.active_doc_mut::<ChannelAssemblerDoc>().unwrap().load_demo();
        world.insert_resource(editor);

        update_channel_assembler_preview(&mut world);
        let (slot0_id, out_id_1) = {
            let cache = world.resource::<ChannelAssemblerPreviewCache>();
            let images = world.resource::<Assets<Image>>();
            let s0 = cache.slot_thumbs[0].as_ref().expect("slot 0 thumb").id();
            let out = cache.output_preview.as_ref().expect("output_preview").id();
            assert!(images.contains(s0));
            assert!(images.contains(out));
            (s0, out)
        };

        // Switching preview channel rebuilds handles and removes old ones.
        world
            .resource_mut::<Editor>()
            .apply(Action::from(ChannelAssemblerAction::SetPreviewChannel(PreviewChannel::B)));
        update_channel_assembler_preview(&mut world);
        let out_id_2 = {
            let cache = world.resource::<ChannelAssemblerPreviewCache>();
            let images = world.resource::<Assets<Image>>();
            let out = cache.output_preview.as_ref().unwrap().id();
            assert!(!images.contains(slot0_id));
            assert!(!images.contains(out_id_1));
            assert!(images.contains(out));
            out
        };

        // Closing the tab removes all cached preview handles.
        world.resource_mut::<Editor>().tabs.clear();
        world.resource_mut::<Editor>().active = None;
        update_channel_assembler_preview(&mut world);
        {
            let cache = world.resource::<ChannelAssemblerPreviewCache>();
            let images = world.resource::<Assets<Image>>();
            assert!(cache.output_preview.is_none());
            assert!(cache.slot_thumbs.iter().all(Option::is_none));
            assert!(!images.contains(out_id_2));
        }
    }
}
