//! Open Texture asset (design §1.1, §4.4): the asset (source of truth), undo/redo history and the form fields that
//! edit its settings. A built-in texture (`@internal/…`, design §2) opens as a read-only document.
//!
//! Pure data, no Bevy: the view rebuilds its form from [`TextureDoc::asset`] when [`TextureDoc::layout`] changes and
//! decodes the preview through [`crate::game_image::GameImageCache`] with the current settings.

use std::sync::atomic::{AtomicU64, Ordering};

use shader_graph_core::Value;
use shader_graph_core::texture::{AddressMode, FilterMode, SamplerSettings, TextureAsset, TextureSettings};
use shader_project::layout::INTERNAL_DIR;
use shader_project::{AssetId, AssetType};
use shader_stdlib::internal::InternalTexture;

use crate::history::History;

/// A fresh value for [`TextureDoc::layout`], unique across documents.
fn next_layout() -> u64 {
    static LAYOUT: AtomicU64 = AtomicU64::new(1);
    LAYOUT.fetch_add(1, Ordering::Relaxed)
}

/// Highest sampler anisotropy wgpu uses (higher values are clamped by wgpu); the form field's range is 1..=16.
pub const MAX_ANISOTROPY: u16 = 16;

#[derive(Debug, Clone)]
pub struct TextureDoc {
    /// The Texture asset; for a built-in texture `Texture` + its `@internal/…` id (not a project asset).
    pub id: AssetId,
    pub asset: TextureAsset,
    /// RON text of the last saved state (dirty check).
    pub saved: String,
    history: History<TextureAsset>,
    /// A built-in texture: read-only, its image is the embedded bytes.
    internal: bool,
    /// The preview repeats the image 3x3, so the address modes show (editor-only, not undoable).
    pub tiled: bool,
    /// Easy mode: only the use-case presets in the header, no form; off = the form with every field (editor-only).
    /// Starts on when the settings fit it ([`fits_easy`]).
    pub easy: bool,
    /// The preview shows only channel 0 / 1 / 2 (R / G / B) as a grey image; `None` = the image (editor-only).
    pub channel: Option<usize>,
    /// Bumped on every change of the asset.
    pub revision: u64,
    /// Changes when the form must be rebuilt: any edit except a dragged number field, undo/redo, reload, a refused
    /// path (the field shows the model again).
    pub layout: u64,
}

impl TextureDoc {
    pub fn new(id: AssetId, asset: TextureAsset) -> Self {
        let saved = asset.to_ron();
        TextureDoc {
            id,
            easy: fits_easy(&asset.settings),
            asset,
            saved,
            history: History::default(),
            internal: false,
            tiled: false,
            channel: None,
            revision: 1,
            layout: next_layout(),
        }
    }

    /// Read-only document of a built-in texture: its settings, and as path the file regeneration copies it to
    /// (`shaders/internal/default.png` under `assets/`).
    pub fn internal(t: &InternalTexture) -> Self {
        let dir = INTERNAL_DIR.strip_prefix("assets/").unwrap_or(INTERNAL_DIR);
        let asset = TextureAsset { settings: (t.settings)(), ..TextureAsset::new(&format!("{dir}/{}", t.file)) };
        TextureDoc { internal: true, ..TextureDoc::new(AssetId::new(AssetType::Texture, t.id), asset) }
    }

    pub fn is_internal(&self) -> bool {
        self.internal
    }

    /// What [`crate::game_image::GameImageCache::get`] decodes: the image path under `assets/`, or the `@internal/…`
    /// id of a built-in texture (its embedded bytes).
    pub fn image_key(&self) -> &str {
        if self.internal { &self.id.path } else { &self.asset.path }
    }

    pub fn is_dirty(&self) -> bool {
        !self.internal && self.asset.to_ron() != self.saved
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.asset.to_ron();
    }

    /// Applies `f` as one undoable step; consecutive edits with the same `key` share one step (a dragged number
    /// field, which also keeps the form). Built-in textures are never changed.
    fn edit(&mut self, key: Option<&str>, f: impl FnOnce(&mut TextureAsset)) {
        if self.internal {
            return;
        }
        let before = self.asset.clone();
        f(&mut self.asset);
        let changed = self.asset != before;
        self.history.record(changed.then_some(before), key);
        if changed {
            self.revision += 1;
            if key.is_none() {
                self.relayout();
            }
        }
    }

    /// Replaces the settings. Values wgpu cannot create are kept (validation reports E013, the preview shows the
    /// error).
    pub fn set_settings(&mut self, settings: TextureSettings, key: Option<&str>) {
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

    fn after_history(&mut self) {
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

/// A form field of the Texture tab (`PropTarget::Texture`).
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
    pub fn get(self, settings: &TextureSettings) -> Value {
        let s = effective_sampler(settings);
        match self {
            TextureField::IsSrgb => Value::Bool(settings.is_srgb),
            TextureField::ExpandGrayscale => Value::Bool(settings.expand_grayscale),
            TextureField::DefaultSampler => Value::Bool(settings.sampler.is_none()),
            TextureField::AddressU => Value::Str(address_label(s.address_mode_u).into()),
            TextureField::AddressV => Value::Str(address_label(s.address_mode_v).into()),
            TextureField::AddressW => Value::Str(address_label(s.address_mode_w).into()),
            TextureField::MagFilter => Value::Str(filter_label(s.mag_filter).into()),
            TextureField::MinFilter => Value::Str(filter_label(s.min_filter).into()),
            TextureField::MipmapFilter => Value::Str(filter_label(s.mipmap_filter).into()),
            TextureField::LodMinClamp => Value::Float(s.lod_min_clamp),
            TextureField::LodMaxClamp => Value::Float(s.lod_max_clamp),
            TextureField::Anisotropy => Value::Int(i32::from(s.anisotropy_clamp)),
        }
    }

    /// `settings` with this field set to `v`; a sampler field (or turning "Default" off) turns the default sampler
    /// into a custom one: what the default sampler is in the game ([`SamplerSettings::image_plugin_default`]) plus
    /// that field. An unknown select label changes nothing.
    pub fn set(self, settings: &TextureSettings, v: &Value) -> TextureSettings {
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
/// the user referred to, decision 0028).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterPreset {
    /// Nearest everywhere, Clamp to edge: crisp texels.
    PixelArt,
    /// Bilinear (Linear mag / min, Nearest mipmap), Clamp to edge.
    Ui,
    /// Trilinear, Repeat.
    ThreeD,
    /// Trilinear + 16x anisotropy, Repeat.
    Ground,
}

impl FilterPreset {
    pub const ALL: [FilterPreset; 4] =
        [FilterPreset::PixelArt, FilterPreset::Ui, FilterPreset::ThreeD, FilterPreset::Ground];

    pub fn label(self) -> &'static str {
        match self {
            FilterPreset::PixelArt => "Pixel art",
            FilterPreset::Ui => "UI",
            FilterPreset::ThreeD => "3D",
            FilterPreset::Ground => "Ground",
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            FilterPreset::PixelArt => "Pixel art: Nearest filters (crisp texels), Clamp to edge",
            FilterPreset::Ui => "UI, sprites: bilinear filtering, Clamp to edge",
            FilterPreset::ThreeD => "3D surfaces: trilinear filtering, Repeat (tiles when UVs leave 0..1)",
            FilterPreset::Ground => "Ground, floors: trilinear + 16x anisotropy (sharp at grazing angles), Repeat",
        }
    }

    /// The preset's sampler (LOD clamps at their defaults).
    pub fn sampler(self) -> SamplerSettings {
        use FilterMode::{Linear, Nearest};
        let (mag, min, mip, wrap, aniso) = match self {
            FilterPreset::PixelArt => (Nearest, Nearest, Nearest, AddressMode::ClampToEdge, 1),
            FilterPreset::Ui => (Linear, Linear, Nearest, AddressMode::ClampToEdge, 1),
            FilterPreset::ThreeD => (Linear, Linear, Linear, AddressMode::Repeat, 1),
            FilterPreset::Ground => (Linear, Linear, Linear, AddressMode::Repeat, MAX_ANISOTROPY),
        };
        SamplerSettings {
            address_mode_u: wrap,
            address_mode_v: wrap,
            address_mode_w: wrap,
            mag_filter: mag,
            min_filter: min,
            mipmap_filter: mip,
            anisotropy_clamp: aniso,
            ..SamplerSettings::default()
        }
    }

    /// The preset whose sampler the settings have (none for the default sampler).
    pub fn of(settings: &TextureSettings) -> Option<Self> {
        FilterPreset::ALL.into_iter().find(|p| settings.sampler == Some(p.sampler()))
    }

    /// `settings` with this preset's sampler (sRGB and grayscale expansion stay).
    pub fn apply(self, settings: &TextureSettings) -> TextureSettings {
        TextureSettings { sampler: Some(self.sampler()), ..settings.clone() }
    }
}

/// Easy mode loses nothing on these settings: the default sampler (nothing chosen yet) or a preset's.
pub fn fits_easy(settings: &TextureSettings) -> bool {
    settings.sampler.is_none() || FilterPreset::of(settings).is_some()
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
mod tests {
    use super::*;
    use shader_graph_core::texture::SamplerSettings;

    fn doc() -> TextureDoc {
        TextureDoc::new(AssetId::new(AssetType::Texture, "textures/a"), TextureAsset::new("textures/a.png"))
    }

    #[test]
    fn fields_round_trip() {
        let d = TextureSettings::default();
        assert_eq!(TextureField::DefaultSampler.get(&d), Value::Bool(true));
        assert_eq!(TextureField::MagFilter.get(&d), Value::Str("Linear".into()), "ImagePlugin's default sampler");
        assert_eq!(TextureField::AddressU.get(&d), Value::Str("Clamp to edge".into()), "not Repeat");
        // Turning "Default" off starts from what the game used.
        let custom = TextureField::DefaultSampler.set(&d, &Value::Bool(false));
        assert_eq!(custom.sampler, Some(SamplerSettings::image_plugin_default()));
        let s = TextureField::MagFilter.set(&d, &Value::Str("Nearest".into()));
        let sampler = s.sampler.unwrap();
        assert_eq!(
            sampler,
            SamplerSettings { mag_filter: FilterMode::Nearest, ..SamplerSettings::image_plugin_default() }
        );
        assert_eq!(TextureField::DefaultSampler.set(&s, &Value::Bool(true)).sampler, None);
        let s = TextureField::AddressV.set(&s, &Value::Str("Mirror repeat".into()));
        assert_eq!(s.sampler.unwrap().address_mode_v, AddressMode::MirrorRepeat);
        assert_eq!(TextureField::AddressV.set(&s, &Value::Str("bogus".into())), s);
        let s = TextureField::Anisotropy.set(&s, &Value::Int(64));
        assert_eq!(s.sampler.unwrap().anisotropy_clamp, MAX_ANISOTROPY);
        // Kept as typed: an inverted range is E013, not silently fixed.
        let s = TextureField::LodMinClamp.set(&s, &Value::Float(40.0));
        assert_eq!((s.sampler.unwrap().lod_min_clamp, s.sampler.unwrap().lod_max_clamp), (40.0, 32.0));
        assert!(s.sampler.unwrap().check().is_err());
        assert!(!TextureField::IsSrgb.set(&d, &Value::Bool(false)).is_srgb);
        for f in TextureField::SAMPLER {
            let v = f.get(&d);
            assert_eq!(f.get(&f.set(&d, &v)), v, "{f:?}");
        }
    }

    #[test]
    fn edits_undo_and_layout() {
        let mut d = doc();
        let layout = d.layout;
        let linear = TextureField::MagFilter.set(&d.asset.settings, &Value::Str("Nearest".into()));
        d.set_settings(linear.clone(), None);
        assert!(d.is_dirty());
        assert!(d.layout > layout, "a select change rebuilds the form");
        let layout = d.layout;
        for x in [2.0, 3.0] {
            let s = TextureField::LodMaxClamp.set(&d.asset.settings, &Value::Float(x));
            d.set_settings(s, Some("lod"));
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

    #[test]
    fn presets() {
        let d = TextureSettings::default();
        assert_eq!(FilterPreset::of(&d), None, "the default sampler is no preset");
        assert!(fits_easy(&d));
        for p in FilterPreset::ALL {
            let s = p.apply(&TextureSettings { is_srgb: false, ..d.clone() });
            assert_eq!(FilterPreset::of(&s), Some(p));
            assert!(!s.is_srgb, "loader settings stay");
            assert!(s.sampler.unwrap().check().is_ok(), "{p:?}");
        }
        let ground = FilterPreset::Ground.sampler();
        assert_eq!((ground.anisotropy_clamp, ground.address_mode_v), (MAX_ANISOTROPY, AddressMode::Repeat));
        let ui = FilterPreset::Ui.sampler();
        assert_eq!((ui.min_filter, ui.mipmap_filter), (FilterMode::Linear, FilterMode::Nearest));
        assert!(!fits_easy(&TextureField::LodMaxClamp.set(&FilterPreset::ThreeD.apply(&d), &Value::Float(4.0))));
    }
}
