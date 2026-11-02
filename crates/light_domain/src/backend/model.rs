//! The Light domain's files (design §9.2): environments (`quillshade/light/<path>.environment.ron`), transition
//! curves (`quillshade/light/<path>.curve.ron`, the curve kit's model), and cube-map Texture assets
//! (`quillshade/light/<path>.texture.ron`, the texture kit's model under [`LIGHT_TEXTURES`]).
//!
//! Asset UUIDs (ADR 0047 §2–4): every kind carries an `id` (format 2, written by the v3 migration), and an
//! environment's `cubemap` stores the texture's UUID. Readers of references see paths: [`LightAssets::resolve`] is
//! the Light resolve seam (UUID -> current path, anything else passes through), [`LightAssets::index_part`] the
//! Light part of the `AssetIndex`.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use project_core::asset_index::{IndexedAsset, IndexedRef, RefNames, annotate_refs};
use project_core::layout::QUILLSHADE_DIR;
use project_core::{
    AssetIndex, AssetRef, AssetUuid, Diagnostic, DomainIndex, LoadError, Project, check_version, probe_version,
};
use texture_kit::{TextureAsset, TexturePolicy, TextureShape};

use super::LIGHT;

/// Current environment file format version: 2 carries an asset `id` and stores the cube map's UUID (ADR 0047 §2–3,
/// written by the v3 migration); older files are upgraded by the Migration Tool (`shader-editor migrate`), never by
/// the loader.
pub const ENVIRONMENT_FORMAT_VERSION: u32 = 2;

/// File suffix of an environment (`<path>.environment.ron`).
pub const ENVIRONMENT_SUFFIX: &str = "environment";
/// File suffix of a Light Texture asset (`<path>.texture.ron`), as the texture kit names it.
pub const TEXTURE_SUFFIX: &str = "texture";
/// File suffix of a Light transition curve (`<path>.curve.ron`), as the curve kit names it.
pub const CURVE_SUFFIX: &str = "curve";

/// What a Light Texture asset accepts (design §9.2): KTX2 cube maps only, no built-in textures, the plain settings
/// form.
pub const LIGHT_TEXTURES: TexturePolicy =
    TexturePolicy { extensions: &["ktx2"], shape: TextureShape::CubeMap, internal: None, presets: false };

/// Calibrated neutral daylight horizon clear color in linear RGBA (`Color::srgb(0.78, 0.80, 0.84)`).
pub const DEFAULT_CLEAR_COLOR: [f32; 4] = [0.571_125, 0.603_827, 0.672_443, 1.0];

/// Converts a domain-relative asset path (`"day"`, `"outdoor/clear_day"`, `"ease_in_out"`) into its `PascalCase` Rust
/// enum variant identifier (`"Day"`, `"OutdoorClearDay"`, `"EaseInOut"`).
pub fn to_pascal_case(path: &str) -> String {
    let mut out = String::new();
    for seg in path.split(['/', '_']).filter(|s| !s.is_empty()) {
        let mut chars = seg.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            out.extend(chars);
        }
    }
    out
}

/// A directional light slot in an [`Environment`].
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct DirectionalLightDef {
    /// Human-readable slot label (e.g. `"Sun"`, `"Sky"`, `"Back"`).
    #[serde(default = "default_light_name")]
    pub name: String,
    /// Linear RGBA color.
    #[serde(default = "default_white")]
    pub color: [f32; 4],
    /// Illuminance in lux.
    #[serde(default = "default_light_illuminance")]
    pub illuminance: f32,
    /// Horizontal yaw angle in degrees (`0` = +Z towards -Z, rotated around +Y).
    #[serde(default = "default_light_azimuth")]
    pub azimuth_deg: f32,
    /// Elevation angle above the horizon in degrees (`0` = horizon, `90` = straight down from zenith).
    #[serde(default = "default_light_elevation")]
    pub elevation_deg: f32,
    /// Whether cascade shadow maps are enabled (`DirectionalLight::shadow_maps_enabled`).
    #[serde(default)]
    pub shadow_maps_enabled: bool,
    /// Whether screen-space contact shadows are enabled (`DirectionalLight::contact_shadows_enabled`).
    #[serde(default)]
    pub contact_shadows_enabled: bool,
    /// Shadow depth bias (`DirectionalLight::shadow_depth_bias`).
    #[serde(default = "default_shadow_depth_bias")]
    pub shadow_depth_bias: f32,
    /// Shadow normal bias (`DirectionalLight::shadow_normal_bias`).
    #[serde(default = "default_shadow_normal_bias")]
    pub shadow_normal_bias: f32,
    /// Whether this light affects lightmapped meshes (`DirectionalLight::affects_lightmapped_mesh_diffuse`).
    #[serde(default = "default_true")]
    pub affects_lightmapped_mesh_diffuse: bool,
}

impl Default for DirectionalLightDef {
    fn default() -> Self {
        DirectionalLightDef {
            name: default_light_name(),
            color: default_white(),
            illuminance: default_light_illuminance(),
            azimuth_deg: default_light_azimuth(),
            elevation_deg: default_light_elevation(),
            shadow_maps_enabled: false,
            contact_shadows_enabled: false,
            shadow_depth_bias: default_shadow_depth_bias(),
            shadow_normal_bias: default_shadow_normal_bias(),
            affects_lightmapped_mesh_diffuse: true,
        }
    }
}

/// The three calibrated neutral daylight directional lights (`Sun` key, `Sky` fill, `Back` rim).
pub fn default_lights() -> Vec<DirectionalLightDef> {
    vec![
        DirectionalLightDef {
            name: "Sun".into(),
            color: [1.0, 1.0, 1.0, 1.0],
            illuminance: 15_000.0,
            azimuth_deg: 35.0,
            elevation_deg: 50.0,
            shadow_maps_enabled: false,
            contact_shadows_enabled: false,
            shadow_depth_bias: 0.02,
            shadow_normal_bias: 1.8,
            affects_lightmapped_mesh_diffuse: true,
        },
        DirectionalLightDef {
            name: "Sky".into(),
            color: [1.0, 1.0, 1.0, 1.0],
            illuminance: 3_000.0,
            azimuth_deg: 180.0,
            elevation_deg: 80.0,
            shadow_maps_enabled: false,
            contact_shadows_enabled: false,
            shadow_depth_bias: 0.02,
            shadow_normal_bias: 1.8,
            affects_lightmapped_mesh_diffuse: true,
        },
        DirectionalLightDef {
            name: "Back".into(),
            color: [1.0, 1.0, 1.0, 1.0],
            illuminance: 2_500.0,
            azimuth_deg: 215.0,
            elevation_deg: 25.0,
            shadow_maps_enabled: false,
            contact_shadows_enabled: false,
            shadow_depth_bias: 0.02,
            shadow_normal_bias: 1.8,
            affects_lightmapped_mesh_diffuse: true,
        },
    ]
}

/// Camera tonemapping operator (`bevy::core_pipeline::tonemapping::Tonemapping`).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TonemappingMode {
    None,
    Reinhard,
    ReinhardLuminance,
    AcesFitted,
    AgX,
    SomewhatBoringDisplayTransform,
    #[default]
    TonyMcMapface,
    BlenderFilmic,
}

impl TonemappingMode {
    pub const ALL: [TonemappingMode; 8] = [
        TonemappingMode::None,
        TonemappingMode::Reinhard,
        TonemappingMode::ReinhardLuminance,
        TonemappingMode::AcesFitted,
        TonemappingMode::AgX,
        TonemappingMode::SomewhatBoringDisplayTransform,
        TonemappingMode::TonyMcMapface,
        TonemappingMode::BlenderFilmic,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TonemappingMode::None => "None",
            TonemappingMode::Reinhard => "Reinhard",
            TonemappingMode::ReinhardLuminance => "Reinhard Luminance",
            TonemappingMode::AcesFitted => "ACES Fitted",
            TonemappingMode::AgX => "AgX",
            TonemappingMode::SomewhatBoringDisplayTransform => "Somewhat Boring",
            TonemappingMode::TonyMcMapface => "TonyMcMapface",
            TonemappingMode::BlenderFilmic => "Blender Filmic",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TonemappingMode::None => "None",
            TonemappingMode::Reinhard => "Reinhard",
            TonemappingMode::ReinhardLuminance => "ReinhardLuminance",
            TonemappingMode::AcesFitted => "AcesFitted",
            TonemappingMode::AgX => "AgX",
            TonemappingMode::SomewhatBoringDisplayTransform => "SomewhatBoringDisplayTransform",
            TonemappingMode::TonyMcMapface => "TonyMcMapface",
            TonemappingMode::BlenderFilmic => "BlenderFilmic",
        }
    }

    pub fn from_str_name(s: &str) -> Option<Self> {
        TonemappingMode::ALL.into_iter().find(|m| m.as_str() == s || m.label() == s)
    }
}

/// Global color grading parameters (`bevy::render::view::ColorGradingGlobal`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ColorGradingGlobalDef {
    #[serde(default)]
    pub exposure: f32,
    #[serde(default)]
    pub temperature: f32,
    #[serde(default)]
    pub tint: f32,
    #[serde(default)]
    pub hue: f32,
    #[serde(default = "default_one")]
    pub post_saturation: f32,
    #[serde(default = "default_midtones_range")]
    pub midtones_range: [f32; 2],
}

impl Default for ColorGradingGlobalDef {
    fn default() -> Self {
        ColorGradingGlobalDef {
            exposure: 0.0,
            temperature: 0.0,
            tint: 0.0,
            hue: 0.0,
            post_saturation: 1.0,
            midtones_range: default_midtones_range(),
        }
    }
}

/// Per-luminance-band color grading parameters (`bevy::render::view::ColorGradingSection`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ColorGradingSectionDef {
    #[serde(default = "default_one")]
    pub saturation: f32,
    #[serde(default = "default_one")]
    pub contrast: f32,
    #[serde(default = "default_one")]
    pub gamma: f32,
    #[serde(default = "default_one")]
    pub gain: f32,
    #[serde(default)]
    pub lift: f32,
}

impl Default for ColorGradingSectionDef {
    fn default() -> Self {
        ColorGradingSectionDef { saturation: 1.0, contrast: 1.0, gamma: 1.0, gain: 1.0, lift: 0.0 }
    }
}

/// Full camera color grading (`bevy::render::view::ColorGrading`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct ColorGradingDef {
    #[serde(default)]
    pub global: ColorGradingGlobalDef,
    #[serde(default)]
    pub shadows: ColorGradingSectionDef,
    #[serde(default)]
    pub midtones: ColorGradingSectionDef,
    #[serde(default)]
    pub highlights: ColorGradingSectionDef,
}

/// Distance fog falloff curve (`bevy::pbr::FogFalloff`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum FogFalloffDef {
    Linear {
        #[serde(default = "default_fog_start")]
        start: f32,
        #[serde(default = "default_fog_end")]
        end: f32,
    },
    Exponential {
        #[serde(default = "default_fog_density")]
        density: f32,
    },
    ExponentialSquared {
        #[serde(default = "default_fog_density")]
        density: f32,
    },
    Atmospheric {
        #[serde(default = "default_fog_atmospheric")]
        extinction: [f32; 3],
        #[serde(default = "default_fog_atmospheric")]
        inscattering: [f32; 3],
    },
}

impl Default for FogFalloffDef {
    fn default() -> Self {
        FogFalloffDef::Linear { start: default_fog_start(), end: default_fog_end() }
    }
}

/// Camera distance fog configuration (`bevy::pbr::DistanceFog`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct FogDef {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_clear_color")]
    pub color: [f32; 4],
    #[serde(default = "default_transparent_black")]
    pub directional_light_color: [f32; 4],
    #[serde(default = "default_fog_exponent")]
    pub directional_light_exponent: f32,
    #[serde(default)]
    pub falloff: FogFalloffDef,
}

impl Default for FogDef {
    fn default() -> Self {
        FogDef {
            enabled: true,
            color: default_clear_color(),
            directional_light_color: default_transparent_black(),
            directional_light_exponent: default_fog_exponent(),
            falloff: FogFalloffDef::default(),
        }
    }
}

/// An environment (design §9.2): skybox, image-based lighting, ambient light, directional lights, camera exposure,
/// tonemapping, color grading, and distance fog.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Environment {
    pub format_version: u32,
    /// The asset's UUID (ADR 0047 §2); nil (omitted from the file) until the asset is first saved.
    #[serde(default, skip_serializing_if = "AssetUuid::is_nil")]
    pub id: AssetUuid,
    /// Camera clear color in linear RGBA (`ClearColor`).
    #[serde(default = "default_clear_color")]
    pub clear_color: [f32; 4],
    /// The Light Texture asset of the cube map: its path, domain-relative and without suffix (`textures/sky`); stored as
    /// its UUID ([`LightAssets::resolve`] turns it back into the path for every reader).
    #[serde(default)]
    pub cubemap: Option<String>,
    /// Brightness of the skybox (Bevy's `Skybox::brightness`, cd/m²).
    #[serde(default = "default_skybox_brightness")]
    pub skybox_brightness: f32,
    /// Yaw rotation of the skybox in degrees (`Skybox::rotation`); the IBL filtered from `cubemap` turns with it
    /// (`GeneratedEnvironmentMapLight::rotation`). Files written before it replaced `env_rotation` still load: the old
    /// field is ignored.
    #[serde(default)]
    pub skybox_rotation: f32,
    /// Scale of the environment light (Bevy's `GeneratedEnvironmentMapLight::intensity`, cd/m²). The light is filtered
    /// from `cubemap` when it has square power-of-two faces (`LW104` otherwise), else from the procedural sky colors.
    #[serde(default = "default_env_intensity")]
    pub env_intensity: f32,
    /// Procedural hemispherical IBL zenith color in linear RGBA (used when `cubemap` is `None` or cannot be filtered).
    #[serde(default = "default_sky_top_color")]
    pub sky_top_color: [f32; 4],
    /// Procedural hemispherical IBL horizon color in linear RGBA (used when `cubemap` is `None` or cannot be filtered).
    #[serde(default = "default_sky_horizon_color")]
    pub sky_horizon_color: [f32; 4],
    /// Procedural hemispherical IBL nadir/ground color in linear RGBA (used when `cubemap` is `None` or cannot be
    /// filtered).
    #[serde(default = "default_ground_color")]
    pub ground_color: [f32; 4],
    /// Whether the environment map light affects lightmapped mesh diffuse (`EnvironmentMapLight::affects_lightmapped_mesh_diffuse`).
    #[serde(default = "default_true")]
    pub env_affects_lightmapped: bool,
    /// Global ambient light color in linear RGBA (`GlobalAmbientLight::color`).
    #[serde(default = "default_ambient_color")]
    pub ambient_color: [f32; 4],
    /// Global ambient light brightness (`GlobalAmbientLight::brightness`).
    #[serde(default = "default_ambient_brightness")]
    pub ambient_brightness: f32,
    /// Whether the global ambient light affects lightmapped meshes (`GlobalAmbientLight::affects_lightmapped_meshes`).
    #[serde(default = "default_true")]
    pub ambient_affects_lightmapped: bool,
    /// Directional light sources managed by this environment.
    #[serde(default = "default_lights")]
    pub lights: Vec<DirectionalLightDef>,
    /// Camera exposure at ISO 100 (`Exposure::ev100`).
    #[serde(default = "default_ev100")]
    pub ev100: f32,
    /// Camera tonemapping mode (`Tonemapping`).
    #[serde(default)]
    pub tonemapping: TonemappingMode,
    /// Whether deband dithering is enabled on the camera (`DebandDither`).
    #[serde(default = "default_true")]
    pub deband_dither: bool,
    /// Camera color grading (`ColorGrading`).
    #[serde(default)]
    pub color_grading: ColorGradingDef,
    /// Camera distance fog (`DistanceFog`).
    #[serde(default)]
    pub fog: FogDef,
}

fn default_clear_color() -> [f32; 4] {
    DEFAULT_CLEAR_COLOR
}

fn default_skybox_brightness() -> f32 {
    2500.0
}

fn default_env_intensity() -> f32 {
    2500.0
}

fn default_sky_top_color() -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}

fn default_sky_horizon_color() -> [f32; 4] {
    [0.9, 0.9, 0.9, 1.0]
}

fn default_ground_color() -> [f32; 4] {
    [0.35, 0.35, 0.35, 1.0]
}

fn default_ambient_color() -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}

fn default_ambient_brightness() -> f32 {
    300.0
}

fn default_ev100() -> f32 {
    13.0
}

fn default_true() -> bool {
    true
}

fn default_one() -> f32 {
    1.0
}

fn default_white() -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}

fn default_transparent_black() -> [f32; 4] {
    [0.0, 0.0, 0.0, 0.0]
}

fn default_midtones_range() -> [f32; 2] {
    [0.2, 0.7]
}

fn default_light_name() -> String {
    "Sun".to_string()
}

fn default_light_illuminance() -> f32 {
    15_000.0
}

fn default_light_azimuth() -> f32 {
    35.0
}

fn default_light_elevation() -> f32 {
    50.0
}

fn default_shadow_depth_bias() -> f32 {
    0.02
}

fn default_shadow_normal_bias() -> f32 {
    1.8
}

fn default_fog_start() -> f32 {
    60.0
}

fn default_fog_end() -> f32 {
    400.0
}

fn default_fog_density() -> f32 {
    0.01
}

fn default_fog_atmospheric() -> [f32; 3] {
    [0.01, 0.01, 0.01]
}

fn default_fog_exponent() -> f32 {
    8.0
}

impl Default for Environment {
    fn default() -> Self {
        Environment {
            format_version: ENVIRONMENT_FORMAT_VERSION,
            id: AssetUuid::NIL,
            clear_color: default_clear_color(),
            cubemap: None,
            skybox_brightness: default_skybox_brightness(),
            skybox_rotation: 0.0,
            env_intensity: default_env_intensity(),
            sky_top_color: default_sky_top_color(),
            sky_horizon_color: default_sky_horizon_color(),
            ground_color: default_ground_color(),
            env_affects_lightmapped: true,
            ambient_color: default_ambient_color(),
            ambient_brightness: default_ambient_brightness(),
            ambient_affects_lightmapped: true,
            lights: default_lights(),
            ev100: default_ev100(),
            tonemapping: TonemappingMode::default(),
            deband_dither: true,
            color_grading: ColorGradingDef::default(),
            fog: FogDef::default(),
        }
    }
}

impl Environment {
    /// Parses an environment file of the current format ([`ENVIRONMENT_FORMAT_VERSION`]; any other is refused).
    pub fn from_ron(text: &str) -> Result<Self, String> {
        check_version(probe_version(text).map_err(|e| e.to_string())?, ENVIRONMENT_FORMAT_VERSION)
            .map_err(|e| e.to_string())?;
        ron::Options::default().from_str(text).map_err(|e| LoadError::Parse(e.to_string()).to_string())
    }

    /// Mints the asset's UUID when it has none (ADR 0047 §6); returns it.
    pub fn ensure_id(&mut self) -> AssetUuid {
        self.id.ensure()
    }

    /// Struct names, 2-space indent, `\n` line ends, a final newline (like Material assets).
    pub fn to_ron(&self) -> String {
        let cfg =
            ron::ser::PrettyConfig::new().struct_names(true).indentor("  ".to_string()).new_line("\n".to_string());
        let mut s = ron::ser::to_string_pretty(self, cfg).expect("environment serialization");
        s.push('\n');
        s
    }
}

/// The environment `path` (domain-relative, no suffix).
pub fn environment_ref(path: &str) -> AssetRef {
    AssetRef { domain: LIGHT, path: path.to_string(), suffix: ENVIRONMENT_SUFFIX }
}

/// The Light Texture asset `path` (domain-relative, no suffix).
pub fn texture_ref(path: &str) -> AssetRef {
    AssetRef { domain: LIGHT, path: path.to_string(), suffix: TEXTURE_SUFFIX }
}

/// The Light Curve asset `path` (domain-relative, no suffix).
pub fn curve_ref(path: &str) -> AssetRef {
    AssetRef { domain: LIGHT, path: path.to_string(), suffix: CURVE_SUFFIX }
}

/// Reads an environment file.
pub fn load_environment(file: &Path) -> Result<Environment, String> {
    let text = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    Environment::from_ron(&text)
}

/// Writes an environment file through a temporary file next to it, creating the parent directories. The environment
/// gets its asset id first when it has none (ADR 0047 §6), and its cube map UUID the D27 path comment labelled by
/// `names` (the project's `AssetIndex`, ADR 0047 §5).
pub fn save_environment(file: &Path, env: &mut Environment, names: &dyn RefNames) -> io::Result<()> {
    env.ensure_id();
    project_core::writer::write_atomic(file, annotate_refs(&env.to_ron(), names, LIGHT))
}

/// Reads a curve file.
pub fn load_curve(file: &Path) -> Result<curve_kit::Curve, String> {
    let text = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    curve_kit::Curve::from_ron(&text).map_err(|e| e.to_string())
}

/// Writes a curve file through a temporary file next to it, creating the parent directories. The curve gets its
/// asset id first when it has none (ADR 0047 §6).
pub fn save_curve(file: &Path, curve: &mut curve_kit::Curve) -> io::Result<()> {
    curve.ensure_id();
    project_core::writer::write_atomic(file, curve.to_ron())
}

/// Gives the Light source file `r` the asset id `id` (the UUID quick fixes Regenerate UUID and Assign UUID, ADR 0047
/// §11, §16): the file is loaded, its id set and written back through its kind's canonical writer (an environment
/// with the D27 cube-map comment labelled by `names`), so a hand-written file without an `id` line gets one. A file
/// that does not load is refused untouched.
pub fn set_asset_uuid(project: &Project, r: &AssetRef, id: AssetUuid, names: &dyn RefNames) -> Result<(), String> {
    let file = project.root.join(r.source_rel());
    let invalid = |e: String| format!("`{r}`: {e}");
    let written = match r.suffix {
        ENVIRONMENT_SUFFIX => {
            let mut env = load_environment(&file).map_err(invalid)?;
            env.id = id;
            save_environment(&file, &mut env, names)
        }
        CURVE_SUFFIX => {
            let mut curve = load_curve(&file).map_err(invalid)?;
            curve.id = id;
            save_curve(&file, &mut curve)
        }
        TEXTURE_SUFFIX => {
            let mut texture = texture_kit::load(&file).map_err(invalid)?;
            texture.id = id;
            texture_kit::save(&file, &texture)
        }
        _ => return Err(format!("`{r}` is not a Light asset")),
    };
    written.map_err(|e| e.to_string())
}

/// Every Light asset file of the project (`quillshade/light/**`), in path order; files and folders whose name starts
/// with `.` are skipped.
pub fn scan(project: &Project) -> Vec<AssetRef> {
    scan_root(&project.root)
}

/// [`scan`] of the project at `root` (any project version: the Migration Tool lists the Light sources with it).
pub fn scan_root(root: &Path) -> Vec<AssetRef> {
    let dir = root.join(QUILLSHADE_DIR).join(LIGHT.as_str());
    let mut out: Vec<AssetRef> = project_core::files::walk_files(&dir)
        .into_iter()
        .filter(|f| f.strip_prefix(&dir).is_ok_and(|rel| !rel.iter().any(|c| c.to_string_lossy().starts_with('.'))))
        .filter_map(|f| {
            let rel = f.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
            project_core::asset_ref::parse(&rel, LIGHT, &[ENVIRONMENT_SUFFIX, TEXTURE_SUFFIX, CURVE_SUFFIX])
        })
        .collect();
    out.sort();
    out
}

/// The loaded Light assets of a project, by domain-relative path; a file that does not load is only in
/// [`LightAssets::load_errors`] (LE009).
#[derive(Debug, Clone, Default)]
pub struct LightAssets {
    pub environments: BTreeMap<String, Environment>,
    pub textures: BTreeMap<String, TextureAsset>,
    pub curves: BTreeMap<String, curve_kit::Curve>,
    pub load_errors: Vec<Diagnostic>,
}

impl LightAssets {
    pub fn load(project: &Project) -> LightAssets {
        let mut assets = LightAssets::default();
        for r in scan(project) {
            let file = project.root.join(r.source_rel());
            let loaded = match r.suffix {
                ENVIRONMENT_SUFFIX => load_environment(&file).map(|e| {
                    assets.environments.insert(r.path.clone(), e);
                }),
                CURVE_SUFFIX => load_curve(&file).map(|mut c| {
                    if let Some(stem) = r.path.rsplit('/').next() {
                        c.name = stem.to_string();
                    }
                    assets.curves.insert(r.path.clone(), c);
                }),
                _ => texture_kit::load(&file).map(|t| {
                    assets.textures.insert(r.path.clone(), t);
                }),
            };
            if let Err(e) = loaded {
                let d =
                    Diagnostic::error("LE009", format!("cannot load: {e}")).in_domain(LIGHT).in_asset(r.source_rel());
                assets.load_errors.push(d);
            }
        }
        assets
    }

    /// The Light domain's part of the `AssetIndex` (ADR 0047 §7): every loaded asset (one that does not load is not
    /// listed) with its `id`; an environment's cube map is a reference located at `cubemap` when it is a UUID (a
    /// hand-written path is not an edge).
    pub fn index_part(&self) -> DomainIndex {
        let mut assets = Vec::new();
        for (path, e) in &self.environments {
            let refs = e
                .cubemap
                .as_deref()
                .and_then(AssetUuid::parse)
                .map(|to| IndexedRef { to, sub: None, location: Some("cubemap".into()) })
                .into_iter()
                .collect();
            assets.push(IndexedAsset { asset: environment_ref(path), id: Ok(e.id), sub_items: Vec::new(), refs });
        }
        for (path, c) in &self.curves {
            assets.push(IndexedAsset { asset: curve_ref(path), id: Ok(c.id), sub_items: Vec::new(), refs: Vec::new() });
        }
        for (path, t) in &self.textures {
            assets.push(IndexedAsset {
                asset: texture_ref(path),
                id: Ok(t.id),
                sub_items: Vec::new(),
                refs: Vec::new(),
            });
        }
        assets.sort_by(|a, b| a.asset.cmp(&b.asset));
        DomainIndex { domain: LIGHT, assets }
    }

    /// The Light resolve seam (ADR 0047 §4): every environment's cube map that is the UUID of a Light Texture asset in
    /// `index` becomes that texture's current path, so validation, planning and codegen see the path form (generated
    /// files are identical). Anything else — a path (hand-written), a dangling UUID, the UUID of another kind —
    /// passes through unchanged (a dangling one is then `LE001`, plus `AE002` from the index).
    pub fn resolve(&mut self, index: &AssetIndex) {
        for env in self.environments.values_mut() {
            if let Some(path) = env.cubemap.as_deref().and_then(|c| resolve_texture(c, index)) {
                env.cubemap = Some(path.to_string());
            }
        }
    }

    /// [`LightAssets::resolve`] against the index of these assets alone ([`LightAssets::index_part`]): Light
    /// references stay inside the domain, so this is enough where no project-wide index is at hand.
    pub fn resolve_locally(&mut self) {
        let index = AssetIndex::build(vec![self.index_part()]);
        self.resolve(&index);
    }
}

/// The current path of the Light Texture asset whose UUID is `value`; `None` when `value` is not a UUID or names no
/// such asset (unknown, another domain or another kind).
pub fn resolve_texture<'a>(value: &str, index: &'a AssetIndex) -> Option<&'a str> {
    let entry = index.get(AssetUuid::parse(value)?)?;
    (entry.asset.domain == LIGHT && entry.asset.suffix == TEXTURE_SUFFIX).then_some(entry.asset.path.as_str())
}

/// Clears the cube map of every environment showing the deleted Light Texture asset named by `names` (the values a
/// cube map stores for it: its UUID, or its path in a hand-written file; ADR 0047 §13); the paths of the rewritten
/// environments, in path order. An environment that does not load is left alone. A rename needs nothing: cube maps
/// store the texture's UUID.
pub fn clear_texture_refs(project: &Project, names: &[String]) -> io::Result<Vec<String>> {
    let mut rewritten = Vec::new();
    for r in scan(project).into_iter().filter(|r| r.suffix == ENVIRONMENT_SUFFIX) {
        let file = project.root.join(r.source_rel());
        let Ok(mut env) = load_environment(&file) else { continue };
        if !env.cubemap.as_ref().is_some_and(|c| names.contains(c)) {
            continue;
        }
        env.cubemap = None;
        // Nothing is left to label: the cube map was the environment's only reference.
        save_environment(&file, &mut env, &AssetIndex::default())?;
        rewritten.push(r.path);
    }
    Ok(rewritten)
}

/// The values a cube map naming Light Texture asset `path` stores, as `index` knows it: its UUID (ADR 0047 §4) and
/// its path (a hand-written file may still name it so). A duplicate loser (a copied file, AE001) has no UUID here:
/// cube maps storing its id name the original ([`AssetIndex::ref_id_of`]).
pub fn texture_ref_names(path: &str, index: &AssetIndex) -> Vec<String> {
    let uuid = index.ref_id_of(&texture_ref(path)).map(|u| u.to_string());
    std::iter::once(path.to_string()).chain(uuid).collect()
}
