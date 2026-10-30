//! Texture assets (design §7.2, decision 0024): a proxy to an image file under the game's `assets/` directory plus a
//! Bevy-free mirror of Bevy's `ImageLoaderSettings`, the path rules ([`TextureAsset::check_path`], E012) and the KTX2
//! shape check ([`shape_problem`], E014), parametrized by a [`TexturePolicy`]; the file [`load`] / [`save`] and every
//! rule at once ([`problems`], each domain maps them to its codes) shared by the domains' backends and the editor.
//! Not a graph and generates no code.
//!
//! With the `editor` feature (default), [`editor`] is the texture editor every domain with Texture assets composes
//! ([`editor::TextureDomain`]): the Texture tab, the read-only image tab of a game file and "Create Texture Asset";
//! the model above stays Bevy-free for generator backends.
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use project_core::LoadError;

#[cfg(feature = "editor")]
pub mod editor;

/// Current texture asset file format version.
pub const TEXTURE_FORMAT_VERSION: u32 = 1;

/// Which GPU view a texture of a domain must have (design §7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureShape {
    /// A plain 2D texture (Material: a `texture_2d` binding rejects cube / array / 3D views).
    Plain2d,
    /// A cube map (6 faces, no array layers, no depth).
    CubeMap,
}

/// What a domain accepts as a Texture asset's image (design §7.2), and what its texture editor offers; each domain
/// maps the kit's messages to its own diagnostic codes.
#[derive(Debug, Clone, Copy)]
pub struct TexturePolicy {
    /// Allowed image extensions (lowercase; compared case-insensitively).
    pub extensions: &'static [&'static str],
    pub shape: TextureShape,
    /// The domain's built-in textures (`@internal/…` ids, design §2), shown in a virtual Internal folder and opened
    /// read-only; `None`: the domain has none. Only the editor uses it (a backend knows its own list).
    pub internal: Option<fn() -> &'static [InternalTexture]>,
    /// The Texture tab offers Easy mode (decision 0028): use-case presets, with the normal-map preset (decision 0042);
    /// without it the tab shows the plain settings form (design §7.2).
    pub presets: bool,
}

impl TexturePolicy {
    /// The built-in textures ([`TexturePolicy::internal`]; empty without them).
    pub fn internal_textures(&self) -> &'static [InternalTexture] {
        self.internal.map_or(&[], |list| list())
    }

    /// The built-in texture with this `@internal/…` id.
    pub fn internal_texture(&self, id: &str) -> Option<&'static InternalTexture> {
        self.internal_textures().iter().find(|t| t.id == id)
    }
}

/// Prefix of the ids of built-in (internal) textures shipped with a domain's editor (`@internal/default`, design
/// §2); `@` never starts an asset path.
pub const INTERNAL_PREFIX: &str = "@internal/";

/// A built-in texture (design §2): embedded image bytes plus loader settings, referenced by its `@internal/<name>` id
/// like a Texture asset but not part of any project (Material: `shader_stdlib::internal`).
pub struct InternalTexture {
    /// `@internal/<name>` ([`INTERNAL_PREFIX`]).
    pub id: &'static str,
    /// File name inside the domain's generated internal textures directory (`default.png`).
    pub file: &'static str,
    pub bytes: &'static [u8],
    pub settings: fn() -> TextureSettings,
}

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
        // Own typed probe (not `project_core::ron_io::probe_version`): keeps the exact old error text.
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
    /// components, no `#` (Bevy reads it as the asset label separator), extension one of `policy.extensions` (any
    /// case accepted; Bevy matches loader extensions case-sensitively, so lowercase is recommended).
    pub fn check_path(path: &str, policy: &TexturePolicy) -> Result<(), String> {
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
            Some(e) if policy.extensions.contains(&e.as_str()) => Ok(()),
            _ => Err(format!("`{path}`: unsupported image format (supported: {})", policy.extensions.join(", "))),
        }
    }
}

/// Bytes of the KTX2 header [`shape_problem`] reads.
pub const KTX2_HEADER_LEN: usize = 48;

/// The first [`KTX2_HEADER_LEN`] bytes of a `.ktx2` file (`None` for another extension or a shorter file).
pub fn read_ktx2_header(file: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    if !file.extension().is_some_and(|e| e.eq_ignore_ascii_case("ktx2")) {
        return None;
    }
    let mut header = vec![0; KTX2_HEADER_LEN];
    std::fs::File::open(file).ok()?.read_exact(&mut header).ok()?;
    Some(header)
}

/// E014-style check of a KTX2 file's first [`KTX2_HEADER_LEN`] bytes against the domain's [`TextureShape`]. For
/// `Plain2d`, a cube map, an array or a 3D texture is refused: Bevy gives such an image a cube / array / 3D view,
/// which a `texture_2d` binding rejects at bind group creation (the game quits). `None` when the shape fits or the
/// header is not KTX2 (the loader reports that).
pub fn shape_problem(header: &[u8], shape: TextureShape) -> Option<String> {
    const IDENTIFIER: [u8; 12] = [0xAB, b'K', b'T', b'X', b' ', b'2', b'0', 0xBB, b'\r', b'\n', 0x1A, b'\n'];
    if header.len() < KTX2_HEADER_LEN || header[..12] != IDENTIFIER {
        return None;
    }
    let field = |at: usize| u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]]);
    let (depth, layers, faces) = (field(28), field(32), field(36));
    match shape {
        TextureShape::Plain2d => {
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
        TextureShape::CubeMap => {
            if faces == 6 && layers <= 1 && depth <= 1 {
                return None;
            }
            let what = if faces > 1 {
                format!("{faces} faces")
            } else if layers > 1 {
                format!("a texture array ({layers} layers)")
            } else if depth > 1 {
                format!("a 3D texture (depth {depth})")
            } else {
                "a 2D texture".to_string()
            };
            Some(format!("the image is not a cube map ({what})"))
        }
    }
}

/// Reads a Texture asset file (`quillshade/<domain>/<path>.texture.ron`). The image path is not checked here
/// ([`problems`] reports it, the asset still loads).
pub fn load(file: &Path) -> Result<TextureAsset, String> {
    let text = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    TextureAsset::from_ron(&text)
}

/// Writes a Texture asset file through a temporary file next to it, creating the parent directories. No checks: an
/// invalid or shared image path is a [`problems`] entry.
pub fn save(file: &Path, texture: &TextureAsset) -> io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("ron.tmp");
    std::fs::write(&tmp, texture.to_ron())?;
    std::fs::rename(&tmp, file)
}

/// Which rule a [`TextureProblem`] breaks; each domain maps it to its own diagnostic code (Material: E013, E012,
/// E011, W109, E014).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProblemKind {
    /// [`SamplerSettings::check`] refuses the sampler (an error).
    Sampler,
    /// [`TextureAsset::check_path`] refuses the image path (an error).
    Path,
    /// Another Texture asset of the domain has the same image (an error: one game file, one asset, design §1.1).
    SharedFile,
    /// The image file does not exist (a warning).
    Missing,
    /// The KTX2 image does not have the policy's [`TextureShape`] (an error, [`shape_problem`]).
    Shape,
}

/// One problem of a loaded Texture asset ([`problems`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureProblem {
    pub kind: ProblemKind,
    pub message: String,
}

/// The problems of a loaded Texture asset under `policy`, in this order: sampler, then path; with a valid path, the
/// other assets of the domain on the same image (`others`: their names, e.g. source paths), then a missing image
/// under `assets_dir` (the game's `assets/`) or its KTX2 shape.
pub fn problems(
    texture: &TextureAsset,
    others: &[String],
    assets_dir: &Path,
    policy: &TexturePolicy,
) -> Vec<TextureProblem> {
    let mut out = Vec::new();
    let mut push = |kind, message| out.push(TextureProblem { kind, message });
    if let Some(Err(e)) = texture.settings.sampler.map(|s| s.check()) {
        push(ProblemKind::Sampler, format!("invalid sampler settings: {e}"));
    }
    if let Err(e) = TextureAsset::check_path(&texture.path, policy) {
        push(ProblemKind::Path, format!("invalid texture path: {e}"));
        return out;
    }
    let path = &texture.path;
    if !others.is_empty() {
        let message =
            format!("`{path}` is also used by {}; a game file can have only one Texture asset", others.join(", "));
        push(ProblemKind::SharedFile, message);
    }
    let file = assets_dir.join(path);
    if !file.is_file() {
        push(ProblemKind::Missing, format!("`assets/{path}` does not exist"));
    } else if let Some(e) = read_ktx2_header(&file).and_then(|h| shape_problem(&h, policy.shape)) {
        push(ProblemKind::Shape, format!("`assets/{path}`: {e}"));
    }
    out
}

/// Folder under `quillshade/<domain>/` that Texture assets made for game files go to (design §4.3).
pub const TEXTURES_DIR: &str = "textures";

/// Asset path of a new Texture asset for the game file `file` (relative to `assets/`, design §4.3):
/// `textures/<folders>/<stem>`, every component made a valid name with [`asset_name_from_file_stem`]. The folders are
/// the file's folders under `assets/`, minus a leading `textures` (so `assets/textures/env/Rock-Albedo.ktx2` →
/// `textures/env/rock_albedo`). A path `taken` (another asset has it) gets `_2`, `_3`, …
pub fn new_texture_path_for_file(file: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut parts: Vec<&str> = file.split('/').collect();
    let name = parts.pop().unwrap_or_default();
    if parts.first() == Some(&TEXTURES_DIR) {
        parts.remove(0);
    }
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    let mut base: Vec<String> = vec![TEXTURES_DIR.to_string()];
    base.extend(parts.iter().map(|d| asset_name_from_file_stem(d)));
    base.push(asset_name_from_file_stem(stem));
    let base = base.join("/");
    let mut candidate = base.clone();
    let mut n = 2;
    while taken(&candidate) {
        candidate = format!("{base}_{n}");
        n += 1;
    }
    candidate
}

/// Asset name for an image file stem (snake_case, design §4.3): `Rock-Albedo` → `rock_albedo`; a stem starting with
/// a digit gets `t_` (`1` → `t_1`); a stem with no ASCII letter or digit becomes `texture`.
pub fn asset_name_from_file_stem(stem: &str) -> String {
    let name = project_core::ident::to_snake_case(stem);
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

    const MATERIAL: TexturePolicy =
        TexturePolicy { extensions: &["ktx2", "png"], shape: TextureShape::Plain2d, internal: None, presets: true };

    #[test]
    fn plain_2d_shape_matches_old_rule() {
        let plain = |h: &[u8]| shape_problem(h, TextureShape::Plain2d);
        assert_eq!(ktx2(0, 0, 1).len(), KTX2_HEADER_LEN);
        assert_eq!(plain(&ktx2(0, 0, 1)), None);
        assert_eq!(plain(&ktx2(1, 1, 1)), None, "a count of 1 is still 2D");
        let cube = plain(&ktx2(0, 0, 6)).unwrap();
        assert_eq!(cube, "the image is a cube map (6 faces); only 2D textures can be sampled");
        assert!(plain(&ktx2(0, 4, 1)).unwrap().contains("a texture array (4 layers)"));
        assert!(plain(&ktx2(8, 0, 1)).unwrap().contains("a 3D texture (depth 8)"));
        assert_eq!(plain(&ktx2(0, 0, 6)[..40]), None, "short");
        assert_eq!(plain(b"\x89PNG\r\n\x1a\n and some more bytes to make it long enough......"), None);
    }

    #[test]
    fn cube_map_shape() {
        let cube = |h: &[u8]| shape_problem(h, TextureShape::CubeMap);
        assert_eq!(cube(&ktx2(0, 0, 6)), None);
        assert!(cube(&ktx2(0, 0, 1)).unwrap().contains("a 2D texture"));
        assert!(cube(&ktx2(0, 4, 6)).is_some());
        assert!(cube(&ktx2(4, 0, 1)).unwrap().contains("3D"));
        assert!(cube(&ktx2(0, 4, 1)).unwrap().contains("4 layers"));
        assert_eq!(cube(&ktx2(0, 0, 6)[..10]), None);
    }

    #[test]
    fn check_path_uses_policy_extensions() {
        let light =
            TexturePolicy { extensions: &["ktx2"], shape: TextureShape::CubeMap, internal: None, presets: false };
        assert!(TextureAsset::check_path("a.png", &light).unwrap_err().contains("ktx2"));
        assert!(TextureAsset::check_path("a.ktx2", &light).is_ok());
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
    fn load_and_save_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("quillshade/light/textures/sky.texture.ron");
        let t = TextureAsset::new("textures/sky.ktx2");
        save(&file, &t).unwrap();
        assert_eq!(load(&file).unwrap(), t);
        assert_eq!(std::fs::read_dir(file.parent().unwrap()).unwrap().count(), 1, "no temp file stays");
        assert!(load(&dir.path().join("none.texture.ron")).is_err());
    }

    /// The rules in their order, each under its kind; a refused path stops there.
    #[test]
    fn problems_follow_the_policy() {
        let dir = tempfile::tempdir().unwrap();
        let assets = dir.path();
        std::fs::create_dir(assets.join("textures")).unwrap();
        std::fs::write(assets.join("textures/flat.ktx2"), ktx2(0, 0, 1)).unwrap();
        std::fs::write(assets.join("textures/sky.ktx2"), ktx2(0, 0, 6)).unwrap();
        let cube =
            TexturePolicy { extensions: &["ktx2"], shape: TextureShape::CubeMap, internal: None, presets: false };
        let kinds = |t: &TextureAsset, others: &[String]| -> Vec<ProblemKind> {
            problems(t, others, assets, &cube).into_iter().map(|p| p.kind).collect()
        };
        assert!(kinds(&TextureAsset::new("textures/sky.ktx2"), &[]).is_empty());
        assert_eq!(kinds(&TextureAsset::new("textures/flat.ktx2"), &[]), [ProblemKind::Shape]);
        assert_eq!(kinds(&TextureAsset::new("textures/gone.ktx2"), &[]), [ProblemKind::Missing]);
        let mut bad = TextureAsset::new("textures/sky.png");
        bad.settings.sampler = Some(SamplerSettings { anisotropy_clamp: 0, ..SamplerSettings::default() });
        assert_eq!(kinds(&bad, &["x".into()]), [ProblemKind::Sampler, ProblemKind::Path]);
        let shared = problems(&TextureAsset::new("textures/sky.ktx2"), &["a".into(), "b".into()], assets, &cube);
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].kind, ProblemKind::SharedFile);
        assert_eq!(
            shared[0].message,
            "`textures/sky.ktx2` is also used by a, b; a game file can have only one Texture asset"
        );
        assert_eq!(problems(&TextureAsset::new("textures/flat.ktx2"), &[], assets, &MATERIAL), []);
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
        assert!(TextureAsset::check_path("a.png", &MATERIAL).is_ok());
        assert!(TextureAsset::check_path("a.KTX2", &MATERIAL).is_ok());
        assert!(TextureAsset::check_path("textures/env/rock_albedo.ktx2", &MATERIAL).is_ok());
        assert!(TextureAsset::check_path("a.ktx", &MATERIAL).is_err());
        assert!(TextureAsset::check_path("a", &MATERIAL).is_err());
        assert!(TextureAsset::check_path("/abs.png", &MATERIAL).is_err());
        assert!(TextureAsset::check_path("C:/abs.png", &MATERIAL).is_err());
        assert!(TextureAsset::check_path("../x.png", &MATERIAL).is_err());
        assert!(TextureAsset::check_path("a/../x.png", &MATERIAL).is_err());
        assert!(TextureAsset::check_path("./x.png", &MATERIAL).is_err());
        assert!(TextureAsset::check_path("a//x.png", &MATERIAL).is_err());
        assert!(TextureAsset::check_path("a\\x.png", &MATERIAL).is_err());
        assert!(TextureAsset::check_path("", &MATERIAL).is_err());
        assert!(TextureAsset::check_path("a#b.png", &MATERIAL).is_err());
        assert!(TextureAsset::check_path("textures/rock.png#Image0", &MATERIAL).is_err());
    }

    /// Moved from `shader_project::game_assets` (the "Create Texture Asset" half, design §4.3).
    #[test]
    fn new_texture_paths_mirror_folders_and_avoid_clashes() {
        let free = |_: &str| false;
        assert_eq!(new_texture_path_for_file("textures/env/Rock-Albedo.ktx2", free), "textures/env/rock_albedo");
        assert_eq!(new_texture_path_for_file("ui/Icons 2/1.png", free), "textures/ui/icons_2/t_1");
        assert_eq!(new_texture_path_for_file("a.png", free), "textures/a");
        let taken = |p: &str| ["textures/a", "textures/a_2"].contains(&p);
        assert_eq!(new_texture_path_for_file("textures/a.ktx2", |p| p == "textures/a"), "textures/a_2");
        assert_eq!(new_texture_path_for_file("a.ktx2", taken), "textures/a_3");
    }

    #[test]
    fn policy_lists_internal_textures() {
        static LIST: [InternalTexture; 1] =
            [InternalTexture { id: "@internal/x", file: "x.png", bytes: &[], settings: TextureSettings::default }];
        let with = TexturePolicy { internal: Some(|| &LIST), ..MATERIAL };
        assert_eq!(with.internal_textures().len(), 1);
        assert!(with.internal_texture("@internal/x").is_some_and(|t| t.file == "x.png"));
        assert!(with.internal_texture("@internal/y").is_none());
        assert!(MATERIAL.internal_textures().is_empty() && MATERIAL.internal_texture("@internal/x").is_none());
        assert!(LIST.iter().all(|t| t.id.starts_with(INTERNAL_PREFIX)));
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
