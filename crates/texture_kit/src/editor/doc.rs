//! An open Texture asset (design §1.1, §4.4): the asset (source of truth), undo/redo history and the form fields that
//! edit its settings, with its [`TabDoc`] side (saving, reloading, renames) and its [`PreviewDoc`] side (the image
//! preview's channel and tiling). A built-in texture (`@internal/…`, design §2, [`TexturePolicy::internal`]) opens as
//! a read-only document.
//!
//! Pure data: the view rebuilds its form from [`TextureDoc::asset`] when [`TextureDoc::layout`] changes and decodes
//! the preview through [`super::GameImageCache`] with the current settings.
//!
//! [`TexturePolicy::internal`]: crate::TexturePolicy::internal

use std::fmt;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

use editor_core::action::{Handled, TabAction};
use editor_core::doc_kind::{DocCtx, StatusCtx, TabDoc};
use editor_core::dynval::{PropKey, PropValue};
use editor_core::history::{History, MergeKey};
use editor_core::tab::ViewKindId;
use editor_core::ui::icons::IconRef;
use project_core::{AssetRef, Project};

use super::{Channel, PreviewDoc, TEXTURE_ICON, TextureDomain, internal_path, load, save};
use crate::{AddressMode, FilterMode, InternalTexture, SamplerSettings, TextureAsset, TextureSettings};

/// A fresh value for [`TextureDoc::layout`], unique across documents.
fn next_layout() -> u64 {
    static LAYOUT: AtomicU64 = AtomicU64::new(1);
    LAYOUT.fetch_add(1, Ordering::Relaxed)
}

/// Highest sampler anisotropy wgpu uses (higher values are clamped by wgpu); the form field's range is 1..=16.
pub const MAX_ANISOTROPY: u16 = 16;

/// A Texture tab of domain `D`.
pub struct TextureDoc<D> {
    /// The Texture asset; for a built-in texture its `@internal/…` id as the path (not a project asset). Follows
    /// renames.
    pub aref: AssetRef,
    pub asset: TextureAsset,
    /// The asset last saved or loaded: dirty is `asset != saved`.
    pub saved: TextureAsset,
    history: History<TextureAsset>,
    /// A built-in texture: read-only, its image is the embedded bytes.
    internal: bool,
    /// The preview repeats the image 3x3, so the address modes show (editor-only, not undoable).
    pub tiled: bool,
    /// Easy mode: only the use-case presets in the header, no form; off = the form with every field (editor-only).
    /// Starts on when the domain has presets ([`crate::TexturePolicy::presets`]) and the settings fit them
    /// ([`fits_easy`]).
    pub easy: bool,
    /// The preview shows only channel 0 / 1 / 2 (R / G / B) as a grey image; `None` = the image (editor-only).
    pub channel: Option<usize>,
    /// Bumped on every change of the asset.
    pub revision: u64,
    /// Changes when the form must be rebuilt: any edit except a dragged number field, undo/redo, reload, a refused
    /// path (the field shows the model again).
    pub layout: u64,
    domain: PhantomData<fn() -> D>,
}

impl<D> Clone for TextureDoc<D> {
    fn clone(&self) -> Self {
        TextureDoc {
            aref: self.aref.clone(),
            asset: self.asset.clone(),
            saved: self.saved.clone(),
            history: self.history.clone(),
            internal: self.internal,
            tiled: self.tiled,
            easy: self.easy,
            channel: self.channel,
            revision: self.revision,
            layout: self.layout,
            domain: PhantomData,
        }
    }
}

impl<D> fmt::Debug for TextureDoc<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextureDoc")
            .field("aref", &self.aref)
            .field("asset", &self.asset)
            .field("internal", &self.internal)
            .field("easy", &self.easy)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

impl<D: TextureDomain> TextureDoc<D> {
    pub fn new(aref: AssetRef, asset: TextureAsset) -> Self {
        let saved = asset.clone();
        TextureDoc {
            aref,
            easy: D::POLICY.presets && fits_easy(&asset.settings),
            asset,
            saved,
            history: History::default(),
            internal: false,
            tiled: false,
            channel: None,
            revision: 1,
            layout: next_layout(),
            domain: PhantomData,
        }
    }

    /// Read-only document of a built-in texture: its settings, and as path the file the domain's regeneration copies
    /// it to ([`internal_path`]: `quillshade/<domain>/internal/default.png` under `assets/`).
    pub fn internal(t: &InternalTexture) -> Self {
        let asset = TextureAsset { settings: (t.settings)(), ..TextureAsset::new(&internal_path::<D>(t)) };
        let aref = AssetRef { domain: D::DOMAIN, path: t.id.to_string(), suffix: D::SUFFIX };
        TextureDoc { internal: true, ..TextureDoc::new(aref, asset) }
    }
}

impl<D> TextureDoc<D> {
    pub fn is_internal(&self) -> bool {
        self.internal
    }

    /// What [`super::GameImageCache::get`] decodes: the image path under `assets/`, or the `@internal/…` id of a
    /// built-in texture (its embedded bytes).
    pub fn image_key(&self) -> &str {
        if self.internal { &self.aref.path } else { &self.asset.path }
    }

    pub fn is_dirty(&self) -> bool {
        !self.internal && self.asset != self.saved
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.asset.clone();
    }

    /// Applies `f` as one undoable step; consecutive edits with the same `key` share one step (a dragged number
    /// field, which also keeps the form). Built-in textures are never changed.
    fn edit(&mut self, key: Option<MergeKey>, f: impl FnOnce(&mut TextureAsset)) {
        if self.internal {
            return;
        }
        let before = self.asset.clone();
        f(&mut self.asset);
        let changed = self.asset != before;
        let relayout = key.is_none();
        self.history.record(changed.then_some(before), key);
        if changed {
            self.revision += 1;
            if relayout {
                self.relayout();
            }
        }
    }

    /// Replaces the settings. Values wgpu cannot create are kept (validation reports E013, the preview shows the
    /// error).
    pub fn set_settings(&mut self, settings: TextureSettings, key: Option<MergeKey>) {
        self.edit(key, |a| a.settings = settings);
    }

    /// Points the asset at another image; the caller checks the path first (E012 rules, one file — one asset).
    pub fn set_path(&mut self, path: &str) {
        self.edit(None, |a| a.path = path.to_string());
    }

    /// Makes the view rebuild its form from the model.
    pub fn relayout(&mut self) {
        self.layout = next_layout();
    }

    /// The asset an undo would restore.
    pub fn peek_undo(&self) -> Option<&TextureAsset> {
        self.history.peek_undo()
    }

    /// The asset a redo would restore.
    pub fn peek_redo(&self) -> Option<&TextureAsset> {
        self.history.peek_redo()
    }

    pub fn end_merge(&mut self) {
        self.history.end_merge();
    }

    pub fn undo(&mut self) -> bool {
        let done = self.history.undo(&mut self.asset);
        if done {
            self.after_history();
        }
        done
    }

    pub fn redo(&mut self) -> bool {
        let done = self.history.redo(&mut self.asset);
        if done {
            self.after_history();
        }
        done
    }

    /// After undo / redo / reload: a sampler that matches no preset cannot be shown in Easy (no preset selected, the
    /// form hidden), so the tab goes to Advanced (design §6.3); settings that fit keep the mode.
    fn after_history(&mut self) {
        if !fits_easy(&self.asset.settings) {
            self.easy = false;
        }
        self.revision += 1;
        self.relayout();
    }

    /// Replaces the asset (external reload) and clears history.
    pub fn reload(&mut self, asset: TextureAsset) {
        self.asset = asset;
        self.mark_saved();
        self.history.clear();
        self.after_history();
    }
}

impl<D: TextureDomain> PreviewDoc for TextureDoc<D> {
    /// The image with the document's current (unsaved) settings.
    fn preview_image(&self) -> (&str, TextureSettings) {
        (self.image_key(), self.asset.settings.clone())
    }

    fn channel(&self) -> Channel {
        self.channel
    }

    fn set_channel(&mut self, channel: Channel) {
        self.channel = channel;
        // The editor topbar's channel buttons show the choice.
        self.relayout();
    }

    fn tiled(&self) -> bool {
        self.tiled
    }

    fn set_tiled(&mut self, tiled: bool) {
        self.tiled = tiled;
    }
}

impl<D: TextureDomain> TabDoc for TextureDoc<D> {
    fn apply(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        self.apply_edit(ctx, action)
    }

    fn current_value(&self, key: &PropKey, _ctx: &StatusCtx) -> Option<PropValue> {
        self.prop_of(super::ops::prop_of_key::<D>(key)?)
    }

    fn asset(&self) -> Option<&AssetRef> {
        (!self.is_internal()).then_some(&self.aref)
    }

    fn view(&self) -> ViewKindId {
        D::VIEW
    }

    /// The source path without `.ron`, or the `@internal/…` id of a built-in texture.
    fn base_title(&self) -> String {
        if self.is_internal() {
            self.aref.path.clone()
        } else {
            self.aref.source_rel().trim_end_matches(".ron").to_string()
        }
    }

    fn icon(&self) -> IconRef {
        TEXTURE_ICON
    }

    fn watermark(&self) -> Option<&'static str> {
        Some(D::SUFFIX)
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

    fn save(&mut self, ctx: &mut DocCtx) -> std::io::Result<()> {
        let project = ctx.project.ok_or_else(|| std::io::Error::other("no project is open"))?;
        if self.is_internal() {
            return Ok(());
        }
        save(project, &self.aref, &self.asset)?;
        self.mark_saved();
        Ok(())
    }

    fn changed_on_disk(&self, project: &Project) -> bool {
        !self.is_internal() && load(project, &self.aref).is_ok_and(|t| t != self.saved)
    }

    /// A built-in texture never changes (read-only documents are "reloaded" on activation).
    fn reload_from_disk(&mut self, ctx: &mut DocCtx) -> Result<(), String> {
        let project = ctx.project.ok_or("no project is open")?;
        if !self.is_internal() {
            self.reload(load(project, &self.aref)?);
        }
        Ok(())
    }

    fn keep_mine(&mut self, project: &Project) {
        if self.is_internal() {
            return;
        }
        if let Ok(on_disk) = load(project, &self.aref) {
            self.saved = on_disk;
        }
    }

    fn rename(&mut self, _project: &Project, from: &AssetRef, to: &AssetRef) {
        if self.is_internal() || self.aref != *from {
            return;
        }
        self.aref = to.clone();
        self.revision += 1;
        self.relayout();
    }

    /// `quillshade/<domain>/….texture.ron`, or the `@internal/…` id of a built-in texture.
    fn config_key(&self) -> Option<String> {
        Some(if self.is_internal() { self.aref.path.clone() } else { self.aref.source_rel() })
    }
}

/// A form field of the Texture tab (`TextureProp::Field`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureField {
    IsSrgb,
    ExpandGrayscale,
    /// `sampler: None` (Bevy's `ImageSampler::Default`, the app's default sampler).
    DefaultSampler,
    AddressU,
    AddressV,
    AddressW,
    MagFilter,
    MinFilter,
    MipmapFilter,
    LodMinClamp,
    LodMaxClamp,
    Anisotropy,
}

impl TextureField {
    /// Fields of a custom sampler (shown while "Default" is off), in form order.
    pub const SAMPLER: [TextureField; 9] = [
        TextureField::AddressU,
        TextureField::AddressV,
        TextureField::AddressW,
        TextureField::MagFilter,
        TextureField::MinFilter,
        TextureField::MipmapFilter,
        TextureField::LodMinClamp,
        TextureField::LodMaxClamp,
        TextureField::Anisotropy,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TextureField::IsSrgb => "sRGB",
            TextureField::ExpandGrayscale => "Expand grayscale",
            TextureField::DefaultSampler => "Default",
            TextureField::AddressU => "Address U",
            TextureField::AddressV => "Address V",
            TextureField::AddressW => "Address W",
            TextureField::MagFilter => "Mag filter",
            TextureField::MinFilter => "Min filter",
            TextureField::MipmapFilter => "Mipmap filter",
            TextureField::LodMinClamp => "LOD min clamp",
            TextureField::LodMaxClamp => "LOD max clamp",
            TextureField::Anisotropy => "Anisotropy",
        }
    }

    /// Option labels of a select field (address modes, filters); empty for the others.
    pub fn options(self) -> Vec<&'static str> {
        match self {
            TextureField::AddressU | TextureField::AddressV | TextureField::AddressW => {
                AddressMode::ALL.iter().map(|m| address_label(*m)).collect()
            }
            TextureField::MagFilter | TextureField::MinFilter | TextureField::MipmapFilter => {
                FilterMode::ALL.iter().map(|f| filter_label(*f)).collect()
            }
            _ => Vec::new(),
        }
    }

    /// The field's value in `settings` (`Bool` toggles, `Str` select labels, `Float` / `Int` numbers); sampler
    /// fields of the default sampler read what a game with Bevy's `ImagePlugin::default()` uses
    /// ([`SamplerSettings::image_plugin_default`]).
    pub fn get(self, settings: &TextureSettings) -> PropValue {
        let s = effective_sampler(settings);
        match self {
            TextureField::IsSrgb => PropValue::Bool(settings.is_srgb),
            TextureField::ExpandGrayscale => PropValue::Bool(settings.expand_grayscale),
            TextureField::DefaultSampler => PropValue::Bool(settings.sampler.is_none()),
            TextureField::AddressU => PropValue::Str(address_label(s.address_mode_u).into()),
            TextureField::AddressV => PropValue::Str(address_label(s.address_mode_v).into()),
            TextureField::AddressW => PropValue::Str(address_label(s.address_mode_w).into()),
            TextureField::MagFilter => PropValue::Str(filter_label(s.mag_filter).into()),
            TextureField::MinFilter => PropValue::Str(filter_label(s.min_filter).into()),
            TextureField::MipmapFilter => PropValue::Str(filter_label(s.mipmap_filter).into()),
            TextureField::LodMinClamp => PropValue::Float(s.lod_min_clamp),
            TextureField::LodMaxClamp => PropValue::Float(s.lod_max_clamp),
            TextureField::Anisotropy => PropValue::Int(i32::from(s.anisotropy_clamp)),
        }
    }

    /// `settings` with this field set to `v`; a sampler field (or turning "Default" off) turns the default sampler
    /// into a custom one: what the default sampler is in the game ([`SamplerSettings::image_plugin_default`]) plus
    /// that field. An unknown select label changes nothing.
    pub fn set(self, settings: &TextureSettings, v: &PropValue) -> TextureSettings {
        let mut out = settings.clone();
        let address = || AddressMode::ALL.into_iter().find(|m| address_label(*m) == v.as_str());
        let filter = || FilterMode::ALL.into_iter().find(|f| filter_label(*f) == v.as_str());
        let mut s = effective_sampler(&out);
        match self {
            TextureField::IsSrgb => return TextureSettings { is_srgb: v.as_bool(), ..out },
            TextureField::ExpandGrayscale => return TextureSettings { expand_grayscale: v.as_bool(), ..out },
            TextureField::DefaultSampler => {
                out.sampler = if v.as_bool() { None } else { Some(s) };
                return out;
            }
            TextureField::AddressU => s.address_mode_u = address().unwrap_or(s.address_mode_u),
            TextureField::AddressV => s.address_mode_v = address().unwrap_or(s.address_mode_v),
            TextureField::AddressW => s.address_mode_w = address().unwrap_or(s.address_mode_w),
            TextureField::MagFilter => s.mag_filter = filter().unwrap_or(s.mag_filter),
            TextureField::MinFilter => s.min_filter = filter().unwrap_or(s.min_filter),
            TextureField::MipmapFilter => s.mipmap_filter = filter().unwrap_or(s.mipmap_filter),
            TextureField::LodMinClamp => s.lod_min_clamp = v.as_f32(),
            TextureField::LodMaxClamp => s.lod_max_clamp = v.as_f32(),
            TextureField::Anisotropy => s.anisotropy_clamp = v.as_i32().clamp(1, i32::from(MAX_ANISOTROPY)) as u16,
        }
        out.sampler = Some(s);
        out
    }
}

/// The sampler `settings` stand for: the custom one, or the game's default ([`SamplerSettings::image_plugin_default`]).
pub fn effective_sampler(settings: &TextureSettings) -> SamplerSettings {
    settings.sampler.unwrap_or_else(SamplerSettings::image_plugin_default)
}

/// Easy mode's presets: what the texture is for, each the whole sampler that suits it best (after the filtering guide
/// the user referred to, decision 0028) plus the sRGB flag (decision 0042: only Normal map is linear data). Easy mode
/// always has one selected: the file stores it through the sampler and sRGB flag it writes, and [`FilterPreset::of`]
/// reads it back (design §6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterPreset {
    /// No sampler stored: the game's default (with Bevy's `ImagePlugin::default()`: Linear, Clamp to edge).
    Default,
    /// Nearest everywhere, Clamp to edge: crisp texels.
    PixelArt,
    /// Bilinear (Linear mag / min, Nearest mipmap), Clamp to edge.
    Ui,
    /// Trilinear, Repeat.
    ThreeD,
    /// 3D's sampler on linear data (sRGB off).
    NormalMap,
    /// Trilinear + 16x anisotropy, Repeat.
    Ground,
}

impl FilterPreset {
    pub const ALL: [FilterPreset; 6] = [
        FilterPreset::Default,
        FilterPreset::PixelArt,
        FilterPreset::Ui,
        FilterPreset::ThreeD,
        FilterPreset::NormalMap,
        FilterPreset::Ground,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FilterPreset::Default => "Default",
            FilterPreset::PixelArt => "Pixel art",
            FilterPreset::Ui => "UI",
            FilterPreset::ThreeD => "3D",
            FilterPreset::NormalMap => "Normal map",
            FilterPreset::Ground => "Ground",
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            FilterPreset::Default => {
                "Bevy's default sampler (ImagePlugin::default(): Linear, Clamp to edge); no sampler stored"
            }
            FilterPreset::PixelArt => "Pixel art: Nearest filters (crisp texels), Clamp to edge",
            FilterPreset::Ui => "UI, sprites: bilinear filtering, Clamp to edge",
            FilterPreset::ThreeD => "3D surfaces: trilinear filtering, Repeat (tiles when UVs leave 0..1)",
            FilterPreset::NormalMap => "Normal maps, other linear data: sRGB off, trilinear filtering, Repeat",
            FilterPreset::Ground => "Ground, floors: trilinear + 16x anisotropy (sharp at grazing angles), Repeat",
        }
    }

    /// The preset's sampler (LOD clamps at their defaults); `None` for [`FilterPreset::Default`].
    pub fn sampler(self) -> Option<SamplerSettings> {
        use FilterMode::{Linear, Nearest};
        let (mag, min, mip, wrap, aniso) = match self {
            FilterPreset::Default => return None,
            FilterPreset::PixelArt => (Nearest, Nearest, Nearest, AddressMode::ClampToEdge, 1),
            FilterPreset::Ui => (Linear, Linear, Nearest, AddressMode::ClampToEdge, 1),
            FilterPreset::ThreeD | FilterPreset::NormalMap => (Linear, Linear, Linear, AddressMode::Repeat, 1),
            FilterPreset::Ground => (Linear, Linear, Linear, AddressMode::Repeat, MAX_ANISOTROPY),
        };
        Some(SamplerSettings {
            address_mode_u: wrap,
            address_mode_v: wrap,
            address_mode_w: wrap,
            mag_filter: mag,
            min_filter: min,
            mipmap_filter: mip,
            anisotropy_clamp: aniso,
            ..SamplerSettings::default()
        })
    }

    /// The preset's sRGB flag: color data except for [`FilterPreset::NormalMap`].
    pub fn is_srgb(self) -> bool {
        self != FilterPreset::NormalMap
    }

    /// The preset whose sampler and sRGB flag the settings have: [`FilterPreset::Default`] for no sampler on sRGB
    /// data, `None` for a custom sampler (also one equal to the game's default: it is stored, so it is not
    /// "Default") or linear data under a sampler other than Normal map's.
    pub fn of(settings: &TextureSettings) -> Option<Self> {
        FilterPreset::ALL.into_iter().find(|p| settings.sampler == p.sampler() && settings.is_srgb == p.is_srgb())
    }

    /// `settings` with this preset's sampler and sRGB flag (grayscale expansion stays).
    pub fn apply(self, settings: &TextureSettings) -> TextureSettings {
        TextureSettings { sampler: self.sampler(), is_srgb: self.is_srgb(), ..settings.clone() }
    }
}

/// Easy mode loses nothing on these settings: their sampler and sRGB flag are a preset's ([`FilterPreset::Default`]:
/// no sampler, sRGB).
pub fn fits_easy(settings: &TextureSettings) -> bool {
    FilterPreset::of(settings).is_some()
}

pub fn address_label(m: AddressMode) -> &'static str {
    match m {
        AddressMode::ClampToEdge => "Clamp to edge",
        AddressMode::Repeat => "Repeat",
        AddressMode::MirrorRepeat => "Mirror repeat",
        AddressMode::ClampToBorder => "Clamp to border",
    }
}

pub fn filter_label(f: FilterMode) -> &'static str {
    match f {
        FilterMode::Nearest => "Nearest",
        FilterMode::Linear => "Linear",
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{TexturePolicy, TextureShape};
    use project_core::DomainId;

    /// A domain for the kit's unit tests: Material's policy.
    pub(crate) struct TestTexture;

    impl TextureDomain for TestTexture {
        const DOMAIN: DomainId = DomainId::new("test");
        const VIEW: ViewKindId = ViewKindId("test.texture");
        const IMAGE_VIEW: ViewKindId = ViewKindId("test.image");
        const POLICY: TexturePolicy =
            TexturePolicy { extensions: &["ktx2", "png"], shape: TextureShape::Plain2d, internal: None, presets: true };

        fn disk_owners(_editor: &editor_core::state::editor::Editor, _file: &str) -> Vec<AssetRef> {
            Vec::new()
        }
    }

    /// [`TestTexture`] without presets (a cube map domain).
    struct NoPresets;

    impl TextureDomain for NoPresets {
        const DOMAIN: DomainId = DomainId::new("cube");
        const VIEW: ViewKindId = ViewKindId("cube.texture");
        const IMAGE_VIEW: ViewKindId = ViewKindId("cube.image");
        const POLICY: TexturePolicy = TexturePolicy { presets: false, ..TestTexture::POLICY };

        fn disk_owners(_editor: &editor_core::state::editor::Editor, _file: &str) -> Vec<AssetRef> {
            Vec::new()
        }
    }

    fn aref(path: &str) -> AssetRef {
        AssetRef { domain: TestTexture::DOMAIN, path: path.into(), suffix: "texture" }
    }

    fn doc() -> TextureDoc<TestTexture> {
        TextureDoc::new(aref("textures/a"), TextureAsset::new("textures/a.png"))
    }

    #[test]
    fn easy_needs_presets() {
        assert!(doc().easy);
        let d = TextureDoc::<NoPresets>::new(aref("textures/a"), TextureAsset::new("textures/a.png"));
        assert!(!d.easy, "no presets: the form");
    }

    #[test]
    fn fields_round_trip() {
        let d = TextureSettings::default();
        assert_eq!(TextureField::DefaultSampler.get(&d), PropValue::Bool(true));
        assert_eq!(TextureField::MagFilter.get(&d), PropValue::Str("Linear".into()), "ImagePlugin's default sampler");
        assert_eq!(TextureField::AddressU.get(&d), PropValue::Str("Clamp to edge".into()), "not Repeat");
        // Turning "Default" off starts from what the game used.
        let custom = TextureField::DefaultSampler.set(&d, &PropValue::Bool(false));
        assert_eq!(custom.sampler, Some(SamplerSettings::image_plugin_default()));
        let s = TextureField::MagFilter.set(&d, &PropValue::Str("Nearest".into()));
        let sampler = s.sampler.unwrap();
        assert_eq!(
            sampler,
            SamplerSettings { mag_filter: FilterMode::Nearest, ..SamplerSettings::image_plugin_default() }
        );
        assert_eq!(TextureField::DefaultSampler.set(&s, &PropValue::Bool(true)).sampler, None);
        let s = TextureField::AddressV.set(&s, &PropValue::Str("Mirror repeat".into()));
        assert_eq!(s.sampler.unwrap().address_mode_v, AddressMode::MirrorRepeat);
        assert_eq!(TextureField::AddressV.set(&s, &PropValue::Str("bogus".into())), s);
        let s = TextureField::Anisotropy.set(&s, &PropValue::Int(64));
        assert_eq!(s.sampler.unwrap().anisotropy_clamp, MAX_ANISOTROPY);
        // Kept as typed: an inverted range is E013, not silently fixed.
        let s = TextureField::LodMinClamp.set(&s, &PropValue::Float(40.0));
        assert_eq!((s.sampler.unwrap().lod_min_clamp, s.sampler.unwrap().lod_max_clamp), (40.0, 32.0));
        assert!(s.sampler.unwrap().check().is_err());
        assert!(!TextureField::IsSrgb.set(&d, &PropValue::Bool(false)).is_srgb);
        for f in TextureField::SAMPLER {
            let v = f.get(&d);
            assert_eq!(f.get(&f.set(&d, &v)), v, "{f:?}");
        }
    }

    #[test]
    fn edits_undo_and_layout() {
        let mut d = doc();
        let layout = d.layout;
        let linear = TextureField::MagFilter.set(&d.asset.settings, &PropValue::Str("Nearest".into()));
        d.set_settings(linear.clone(), None);
        assert!(d.is_dirty());
        assert!(d.layout > layout, "a select change rebuilds the form");
        let layout = d.layout;
        for x in [2.0, 3.0] {
            let s = TextureField::LodMaxClamp.set(&d.asset.settings, &PropValue::Float(x));
            d.set_settings(s, Some("lod".into()));
        }
        d.end_merge();
        assert_eq!(d.layout, layout, "a dragged field keeps the form");
        assert!(d.undo());
        assert_eq!(d.asset.settings, linear, "the drag was one step");
        assert!(d.undo());
        assert!(!d.is_dirty());
        d.set_path("textures/b.png");
        assert_eq!(d.asset.path, "textures/b.png");
        assert!(d.undo());
        assert_eq!(d.asset.path, "textures/a.png");
    }

    /// Design §6.3: "Default" is no sampler in the file (Bevy's `ImagePlugin::default()`), not "3D" (Repeat).
    #[test]
    fn default_preset_is_no_sampler() {
        assert_eq!(FilterPreset::of(&TextureSettings::default()), Some(FilterPreset::Default));
        assert_eq!(FilterPreset::ALL[0], FilterPreset::Default, "the first preset");
        assert_eq!(FilterPreset::Default.sampler(), None);
        let s3d =
            FilterPreset::ThreeD.apply(&TextureSettings { expand_grayscale: false, ..TextureSettings::default() });
        let back = FilterPreset::Default.apply(&s3d);
        assert!(back.sampler.is_none() && back.is_srgb);
        assert!(!back.expand_grayscale, "grayscale expansion stays");
        // A custom sampler equal to the game's default is still a custom one: no preset.
        let custom = TextureSettings { sampler: Some(SamplerSettings::image_plugin_default()), ..Default::default() };
        assert_eq!(FilterPreset::of(&custom), None);
        assert!(!fits_easy(&custom));
    }

    #[test]
    fn presets() {
        let d = TextureSettings::default();
        assert!(fits_easy(&d));
        for p in FilterPreset::ALL {
            let s = p.apply(&TextureSettings { is_srgb: !p.is_srgb(), expand_grayscale: false, ..d.clone() });
            assert_eq!(FilterPreset::of(&s), Some(p));
            assert_eq!(s.is_srgb, p.is_srgb(), "{p:?} sets sRGB");
            assert!(!s.expand_grayscale, "grayscale expansion stays");
            assert!(s.sampler.is_none_or(|s| s.check().is_ok()), "{p:?}");
        }
        // Normal map: 3D's sampler on linear data; linear data under any other preset's sampler is no preset.
        assert_eq!(FilterPreset::NormalMap.sampler(), FilterPreset::ThreeD.sampler());
        assert!(!FilterPreset::NormalMap.is_srgb());
        let linear = |p: FilterPreset| TextureSettings { is_srgb: false, ..p.apply(&d) };
        assert_eq!(FilterPreset::of(&linear(FilterPreset::ThreeD)), Some(FilterPreset::NormalMap));
        assert_eq!(FilterPreset::of(&linear(FilterPreset::Default)), None);
        assert!(!fits_easy(&linear(FilterPreset::Ground)));
        let ground = FilterPreset::Ground.sampler().unwrap();
        assert_eq!((ground.anisotropy_clamp, ground.address_mode_v), (MAX_ANISOTROPY, AddressMode::Repeat));
        let ui = FilterPreset::Ui.sampler().unwrap();
        assert_eq!((ui.min_filter, ui.mipmap_filter), (FilterMode::Linear, FilterMode::Nearest));
        assert!(!fits_easy(&TextureField::LodMaxClamp.set(&FilterPreset::ThreeD.apply(&d), &PropValue::Float(4.0))));
    }
}
