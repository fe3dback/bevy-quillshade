//! Texture assets (`shaders/**/*.texture.ron`, design §1.1, decision 0024): a proxy to an image file under the
//! game's `assets/` directory plus a Bevy-free mirror of Bevy's `ImageLoaderSettings`. Not a graph and generates no
//! WESL; one game file is referenced by at most one Texture asset (E011), the path is checked by
//! [`TextureAsset::check_path`] (E012).

use serde::{Deserialize, Serialize};

use crate::ident::to_snake_case;
use crate::model::LoadError;

/// Current texture asset file format version.
pub const TEXTURE_FORMAT_VERSION: u32 = 1;

/// Image file extensions a Texture asset may point at (lowercase; compared case-insensitively). Bevy 0.20 cannot
/// read `.ktx` (v1).
pub const TEXTURE_EXTENSIONS: [&str; 2] = ["ktx2", "png"];

/// Prefix of the ids of built-in (internal) textures shipped with the editor (`@internal/default`, design §2).
pub const INTERNAL_PREFIX: &str = "@internal/";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TextureAsset {
    pub format_version: u32,
    /// Image path relative to the game's `assets/` directory (`textures/env/rock_albedo.ktx2`), `/`-separated.
    pub path: String,
    #[serde(default, skip_serializing_if = "TextureSettings::is_default")]
    pub settings: TextureSettings,
}

/// Mirror of the useful part of Bevy's `ImageLoaderSettings`; defaults are Bevy's.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TextureSettings {
    /// Image data is sRGB when the format does not say otherwise (Bevy: `is_srgb`, default `true`).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub is_srgb: bool,
    /// Expand grayscale images to RGBA (Bevy: `expand_grayscale`, default `true`).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub expand_grayscale: bool,
    /// `None` = Bevy's `ImageSampler::Default` (the app's default sampler); `Some` = `ImageSampler::Descriptor`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampler: Option<SamplerSettings>,
}

impl Default for TextureSettings {
    fn default() -> Self {
        TextureSettings { is_srgb: true, expand_grayscale: true, sampler: None }
    }
}

impl TextureSettings {
    pub fn is_default(&self) -> bool {
        *self == TextureSettings::default()
    }
}

/// Mirror of Bevy's `ImageSamplerDescriptor` (without label, compare function and border color); defaults are
/// Bevy's.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct SamplerSettings {
    #[serde(default, skip_serializing_if = "AddressMode::is_default")]
    pub address_mode_u: AddressMode,
    #[serde(default, skip_serializing_if = "AddressMode::is_default")]
    pub address_mode_v: AddressMode,
    #[serde(default, skip_serializing_if = "AddressMode::is_default")]
    pub address_mode_w: AddressMode,
    #[serde(default, skip_serializing_if = "FilterMode::is_default")]
    pub mag_filter: FilterMode,
    #[serde(default, skip_serializing_if = "FilterMode::is_default")]
    pub min_filter: FilterMode,
    #[serde(default, skip_serializing_if = "FilterMode::is_default")]
    pub mipmap_filter: FilterMode,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub lod_min_clamp: f32,
    #[serde(default = "default_lod_max_clamp", skip_serializing_if = "is_default_lod_max_clamp")]
    pub lod_max_clamp: f32,
    /// At least 1; anything else needs every filter to be `Linear` (wgpu rule).
    #[serde(default = "default_anisotropy_clamp", skip_serializing_if = "is_default_anisotropy_clamp")]
    pub anisotropy_clamp: u16,
}

impl Default for SamplerSettings {
    fn default() -> Self {
        SamplerSettings {
            address_mode_u: AddressMode::default(),
            address_mode_v: AddressMode::default(),
            address_mode_w: AddressMode::default(),
            mag_filter: FilterMode::default(),
            min_filter: FilterMode::default(),
            mipmap_filter: FilterMode::default(),
            lod_min_clamp: 0.0,
            lod_max_clamp: default_lod_max_clamp(),
            anisotropy_clamp: default_anisotropy_clamp(),
        }
    }
}

impl SamplerSettings {
    /// The sampler a texture with `sampler: None` gets in a game with Bevy's `ImagePlugin::default()`:
    /// `ImageSamplerDescriptor::linear()` — Linear mag / min / mipmap filters, Clamp to edge (not Repeat).
    pub fn image_plugin_default() -> Self {
        SamplerSettings {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Linear,
            ..SamplerSettings::default()
        }
    }

    /// E013 rules: what wgpu refuses to create (anisotropy 0; anisotropy above 1 unless mag, min and mipmap filters
    /// are all Linear; LOD clamps negative or min > max), plus non-finite LOD clamps, which wgpu accepts but we refuse
    /// (decision 0024). Anisotropy above 16 is clamped by wgpu, not refused.
    pub fn check(&self) -> Result<(), String> {
        if self.anisotropy_clamp == 0 {
            return Err("anisotropy_clamp must be at least 1".into());
        }
        let linear = [self.mag_filter, self.min_filter, self.mipmap_filter].iter().all(|f| *f == FilterMode::Linear);
        if self.anisotropy_clamp > 1 && !linear {
            return Err(format!("anisotropy_clamp {} needs Linear mag, min and mipmap filters", self.anisotropy_clamp));
        }
        let (min, max) = (self.lod_min_clamp, self.lod_max_clamp);
        if !min.is_finite() || !max.is_finite() || min < 0.0 || min > max {
            return Err(format!("LOD clamps must be finite, 0 <= lod_min_clamp <= lod_max_clamp (got {min}..{max})"));
        }
        Ok(())
    }
}

/// Mirror of Bevy's `ImageAddressMode`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum AddressMode {
    #[default]
    ClampToEdge,
    Repeat,
    MirrorRepeat,
    ClampToBorder,
}

impl AddressMode {
    pub const ALL: [AddressMode; 4] =
        [AddressMode::ClampToEdge, AddressMode::Repeat, AddressMode::MirrorRepeat, AddressMode::ClampToBorder];

    pub fn is_default(&self) -> bool {
        *self == AddressMode::default()
    }
}

/// Mirror of Bevy's `ImageFilterMode` (Bevy's descriptor default is `Nearest`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum FilterMode {
    #[default]
    Nearest,
    Linear,
}

impl FilterMode {
    pub const ALL: [FilterMode; 2] = [FilterMode::Nearest, FilterMode::Linear];

    pub fn is_default(&self) -> bool {
        *self == FilterMode::default()
    }
}

fn yes() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

fn default_lod_max_clamp() -> f32 {
    32.0
}

fn is_default_lod_max_clamp(v: &f32) -> bool {
    *v == default_lod_max_clamp()
}

fn default_anisotropy_clamp() -> u16 {
    1
}

fn is_default_anisotropy_clamp(v: &u16) -> bool {
    *v == default_anisotropy_clamp()
}

#[derive(Deserialize)]
#[serde(rename = "TextureAsset")]
struct VersionProbe {
    format_version: u32,
}

impl TextureAsset {
    /// Texture asset for `path` with Bevy's default loader settings.
    pub fn new(path: &str) -> Self {
        TextureAsset { format_version: TEXTURE_FORMAT_VERSION, path: path.to_string(), settings: Default::default() }
    }

    /// Parses a texture asset file (older format versions would migrate here). The path is not checked here (E012
    /// is reported by validation, the asset still loads).
    pub fn from_ron(text: &str) -> Result<Self, String> {
        let parse = |e: ron::error::SpannedError| LoadError::Parse(e.to_string()).to_string();
        let probe: VersionProbe = ron::Options::default().from_str(text).map_err(parse)?;
        match probe.format_version {
            v if v > TEXTURE_FORMAT_VERSION => {
                Err(LoadError::TooNew { found: v, supported: TEXTURE_FORMAT_VERSION }.to_string())
            }
            TEXTURE_FORMAT_VERSION => ron::Options::default().from_str(text).map_err(parse),
            v => Err(LoadError::Unsupported(v).to_string()),
        }
    }

    /// Struct names, 2-space indent, fields at their default omitted.
    pub fn to_ron(&self) -> String {
        let cfg =
            ron::ser::PrettyConfig::new().struct_names(true).indentor("  ".to_string()).new_line("\n".to_string());
        let mut s = ron::ser::to_string_pretty(self, cfg).expect("texture serialization");
        s.push('\n');
        s
    }

    /// E012 rules for [`TextureAsset::path`]: not empty, relative, `/`-separated without empty, `.` or `..`
    /// components, no `#` (Bevy reads it as the asset label separator), extension `ktx2` or `png` (any case accepted;
    /// Bevy matches loader extensions case-sensitively, so lowercase is recommended).
    pub fn check_path(path: &str) -> Result<(), String> {
        if path.is_empty() {
            return Err("empty texture path".into());
        }
        if path.contains('\\') {
            return Err(format!("`{path}`: use `/` as the path separator"));
        }
        if path.contains('#') {
            return Err(format!("`{path}` must not contain `#` (Bevy reads it as an asset label separator)"));
        }
        let bytes = path.as_bytes();
        if path.starts_with('/') || (bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()) {
            return Err(format!("`{path}` must be relative to the game's assets/ directory"));
        }
        if path.split('/').any(|c| c.is_empty() || c == "." || c == "..") {
            return Err(format!("`{path}` must not contain empty, `.` or `..` components"));
        }
        let ext = path.rsplit('/').next().and_then(|f| f.rsplit_once('.')).map(|(_, e)| e.to_ascii_lowercase());
        match ext {
            Some(e) if TEXTURE_EXTENSIONS.contains(&e.as_str()) => Ok(()),
            _ => Err(format!("`{path}`: unsupported image format (supported: {})", TEXTURE_EXTENSIONS.join(", "))),
        }
    }
}

/// Bytes of the KTX2 header [`ktx2_not_2d`] reads.
pub const KTX2_HEADER_LEN: usize = 48;

/// E014 for a KTX2 file (its first [`KTX2_HEADER_LEN`] bytes) that is no plain 2D texture: a cube map, an array or a
/// 3D texture. Bevy gives such an image a cube / array / 3D view, which a `texture_2d` binding rejects at bind group
/// creation (the game quits). `None` for a 2D texture or a header that is not KTX2 (the loader reports that).
pub fn ktx2_not_2d(header: &[u8]) -> Option<String> {
    const IDENTIFIER: [u8; 12] = [0xAB, b'K', b'T', b'X', b' ', b'2', b'0', 0xBB, b'\r', b'\n', 0x1A, b'\n'];
    if header.len() < KTX2_HEADER_LEN || header[..12] != IDENTIFIER {
        return None;
    }
    let field = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().expect("4 bytes"));
    let (depth, layers, faces) = (field(28), field(32), field(36));
    let what = if faces > 1 {
        format!("a cube map ({faces} faces)")
    } else if layers > 1 {
        format!("a texture array ({layers} layers)")
    } else if depth > 1 {
        format!("a 3D texture (depth {depth})")
    } else {
        return None;
    };
    Some(format!("the image is {what}; only 2D textures can be sampled"))
}

/// Asset name for an image file stem (snake_case, design §4.3): `Rock-Albedo` → `rock_albedo`; a stem starting with
/// a digit gets `t_` (`1` → `t_1`); a stem with no ASCII letter or digit becomes `texture`.
pub fn asset_name_from_file_stem(stem: &str) -> String {
    let name = to_snake_case(stem);
    match name.chars().next() {
        None => "texture".to_string(),
        Some(c) if c.is_ascii_digit() => format!("t_{name}"),
        Some(_) => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// KTX2 header with the given depth, layer and face counts.
    fn ktx2(depth: u32, layers: u32, faces: u32) -> Vec<u8> {
        let mut h = vec![0xAB, b'K', b'T', b'X', b' ', b'2', b'0', 0xBB, b'\r', b'\n', 0x1A, b'\n'];
        for v in [37, 1, 64, 64, depth, layers, faces, 1, 0] {
            h.extend(u32::to_le_bytes(v));
        }
        h
    }

    #[test]
    fn ktx2_shapes() {
        assert_eq!(ktx2(0, 0, 1).len(), KTX2_HEADER_LEN);
        assert_eq!(ktx2_not_2d(&ktx2(0, 0, 1)), None);
        assert_eq!(ktx2_not_2d(&ktx2(1, 1, 1)), None, "a count of 1 is still 2D");
        let cube = ktx2_not_2d(&ktx2(0, 0, 6)).unwrap();
        assert_eq!(cube, "the image is a cube map (6 faces); only 2D textures can be sampled");
        assert!(ktx2_not_2d(&ktx2(0, 4, 1)).unwrap().contains("a texture array (4 layers)"));
        assert!(ktx2_not_2d(&ktx2(8, 0, 1)).unwrap().contains("a 3D texture (depth 8)"));
        assert_eq!(ktx2_not_2d(&ktx2(0, 0, 6)[..40]), None, "short");
        assert_eq!(ktx2_not_2d(b"\x89PNG\r\n\x1a\n and some more bytes to make it long enough......"), None);
    }

    #[test]
    fn ron_round_trip_omits_defaults() {
        let t = TextureAsset::new("textures/a.png");
        let text = t.to_ron();
        assert!(text.starts_with("TextureAsset("), "{text}");
        assert!(text.contains("path: \"textures/a.png\""), "{text}");
        assert!(!text.contains("settings:"), "{text}");
        assert_eq!(TextureAsset::from_ron(&text).unwrap(), t);
    }

    #[test]
    fn sampler_settings_round_trip() {
        let mut t = TextureAsset::new("textures/b.ktx2");
        t.settings.is_srgb = false;
        t.settings.sampler = Some(SamplerSettings {
            address_mode_u: AddressMode::Repeat,
            address_mode_v: AddressMode::MirrorRepeat,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            anisotropy_clamp: 8,
            ..SamplerSettings::default()
        });
        let text = t.to_ron();
        assert!(text.contains("is_srgb: false"), "{text}");
        assert!(!text.contains("expand_grayscale"), "default omitted: {text}");
        assert!(text.contains("address_mode_u: Repeat"), "{text}");
        assert!(text.contains("mag_filter: Linear"), "{text}");
        assert!(!text.contains("address_mode_w"), "default omitted: {text}");
        assert!(!text.contains("lod_max_clamp"), "default omitted: {text}");
        let back = TextureAsset::from_ron(&text).unwrap();
        assert_eq!(back, t);
        let s = back.settings.sampler.unwrap();
        assert_eq!((s.address_mode_u, s.mag_filter), (AddressMode::Repeat, FilterMode::Linear));
        assert_eq!(
            (s.address_mode_w, s.mipmap_filter, s.lod_max_clamp),
            (AddressMode::ClampToEdge, FilterMode::Nearest, 32.0)
        );
        // A sampler written with every field at its default still means "custom sampler", not Bevy's default one.
        let mut d = TextureAsset::new("a.png");
        d.settings.sampler = Some(SamplerSettings::default());
        assert_eq!(TextureAsset::from_ron(&d.to_ron()).unwrap(), d);
        let too_new = "TextureAsset(format_version: 9, path: \"a.png\")";
        assert!(TextureAsset::from_ron(too_new).unwrap_err().contains("newer"));
    }

    #[test]
    fn sampler_check_rules() {
        let ok = SamplerSettings::default();
        assert!(ok.check().is_ok());
        assert!(SamplerSettings { anisotropy_clamp: 0, ..ok }.check().unwrap_err().contains("at least 1"));
        let aniso = SamplerSettings { anisotropy_clamp: 8, ..ok };
        assert!(aniso.check().unwrap_err().contains("Linear"));
        let linear = SamplerSettings {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Linear,
            ..aniso
        };
        assert!(linear.check().is_ok());
        assert!(SamplerSettings { anisotropy_clamp: 64, ..linear }.check().is_ok(), "wgpu clamps to 16");
        for (min, max) in [(-1.0, 32.0), (5.0, 4.0), (f32::NAN, 32.0), (0.0, f32::INFINITY)] {
            let bad = SamplerSettings { lod_min_clamp: min, lod_max_clamp: max, ..ok };
            assert!(bad.check().unwrap_err().contains("LOD"), "{min}..{max}");
        }
    }

    #[test]
    fn check_path_rules() {
        assert!(TextureAsset::check_path("a.png").is_ok());
        assert!(TextureAsset::check_path("a.KTX2").is_ok());
        assert!(TextureAsset::check_path("textures/env/rock_albedo.ktx2").is_ok());
        assert!(TextureAsset::check_path("a.ktx").is_err());
        assert!(TextureAsset::check_path("a").is_err());
        assert!(TextureAsset::check_path("/abs.png").is_err());
        assert!(TextureAsset::check_path("C:/abs.png").is_err());
        assert!(TextureAsset::check_path("../x.png").is_err());
        assert!(TextureAsset::check_path("a/../x.png").is_err());
        assert!(TextureAsset::check_path("./x.png").is_err());
        assert!(TextureAsset::check_path("a//x.png").is_err());
        assert!(TextureAsset::check_path("a\\x.png").is_err());
        assert!(TextureAsset::check_path("").is_err());
        assert!(TextureAsset::check_path("a#b.png").is_err());
        assert!(TextureAsset::check_path("textures/rock.png#Image0").is_err());
    }

    #[test]
    fn asset_name_from_file_stem_cases() {
        assert_eq!(asset_name_from_file_stem("Rock-Albedo"), "rock_albedo");
        assert_eq!(asset_name_from_file_stem("1"), "t_1");
        assert_eq!(asset_name_from_file_stem("Ёж"), "texture");
        assert_eq!(asset_name_from_file_stem("a__b"), "a_b");
        assert_eq!(asset_name_from_file_stem("rockAlbedo 2"), "rock_albedo_2");
    }
}
