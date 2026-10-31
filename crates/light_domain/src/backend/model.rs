//! The Light domain's files (design §9.2): environments (`quillshade/light/<path>.environment.ron`), transition
//! curves (`quillshade/light/<path>.curve.ron`, the curve kit's model), and cube-map Texture assets
//! (`quillshade/light/<path>.texture.ron`, the texture kit's model under [`LIGHT_TEXTURES`]).

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use project_core::{AssetRef, Diagnostic, LoadError, Project, probe_version};
use texture_kit::{TextureAsset, TexturePolicy, TextureShape};

use super::LIGHT;

/// Current environment file format version.
pub const ENVIRONMENT_FORMAT_VERSION: u32 = 1;

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
    /// Camera clear color in linear RGBA (`ClearColor`).
    #[serde(default = "default_clear_color")]
    pub clear_color: [f32; 4],
    /// The Light Texture asset of the cube map: its path, domain-relative and without suffix (`textures/sky`).
    #[serde(default)]
    pub cubemap: Option<String>,
    /// Brightness of the skybox (Bevy's `Skybox::brightness`, cd/m²).
    #[serde(default = "default_skybox_brightness")]
    pub skybox_brightness: f32,
    /// Yaw rotation of the skybox in degrees (`Skybox::rotation`).
    #[serde(default)]
    pub skybox_rotation: f32,
    /// Scale of the environment light (Bevy's `EnvironmentMapLight::intensity`, cd/m²).
    #[serde(default = "default_env_intensity")]
    pub env_intensity: f32,
    /// Yaw rotation of the environment light in degrees (`EnvironmentMapLight::rotation`).
    #[serde(default)]
    pub env_rotation: f32,
    /// Procedural hemispherical IBL zenith color in linear RGBA (used when `cubemap` is `None`).
    #[serde(default = "default_sky_top_color")]
    pub sky_top_color: [f32; 4],
    /// Procedural hemispherical IBL horizon color in linear RGBA (used when `cubemap` is `None`).
    #[serde(default = "default_sky_horizon_color")]
    pub sky_horizon_color: [f32; 4],
    /// Procedural hemispherical IBL nadir/ground color in linear RGBA (used when `cubemap` is `None`).
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
            clear_color: default_clear_color(),
            cubemap: None,
            skybox_brightness: default_skybox_brightness(),
            skybox_rotation: 0.0,
            env_intensity: default_env_intensity(),
            env_rotation: 0.0,
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
    /// Parses an environment file (older format versions would migrate here).
    pub fn from_ron(text: &str) -> Result<Self, String> {
        match probe_version(text).map_err(|e| e.to_string())? {
            v if v > ENVIRONMENT_FORMAT_VERSION => {
                Err(LoadError::TooNew { found: v, supported: ENVIRONMENT_FORMAT_VERSION }.to_string())
            }
            ENVIRONMENT_FORMAT_VERSION => {
                ron::Options::default().from_str(text).map_err(|e| LoadError::Parse(e.to_string()).to_string())
            }
            v => Err(LoadError::Unsupported(v).to_string()),
        }
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

/// Writes an environment file through a temporary file next to it, creating the parent directories.
pub fn save_environment(file: &Path, env: &Environment) -> io::Result<()> {
    project_core::writer::write_atomic(file, env.to_ron())
}

/// Reads a curve file.
pub fn load_curve(file: &Path) -> Result<curve_kit::Curve, String> {
    let text = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    curve_kit::Curve::from_ron(&text).map_err(|e| e.to_string())
}

/// Writes a curve file through a temporary file next to it, creating the parent directories.
pub fn save_curve(file: &Path, curve: &curve_kit::Curve) -> io::Result<()> {
    project_core::writer::write_atomic(file, curve.to_ron())
}

/// Every Light asset file of the project (`quillshade/light/**`), in path order; files and folders whose name starts
/// with `.` are skipped.
pub fn scan(project: &Project) -> Vec<AssetRef> {
    let dir = project.domain_dir(LIGHT);
    let mut out: Vec<AssetRef> = project_core::files::walk_files(&dir)
        .into_iter()
        .filter(|f| f.strip_prefix(&dir).is_ok_and(|rel| !rel.iter().any(|c| c.to_string_lossy().starts_with('.'))))
        .filter_map(|f| {
            project_core::asset_ref::parse(&project.rel(&f), LIGHT, &[ENVIRONMENT_SUFFIX, TEXTURE_SUFFIX, CURVE_SUFFIX])
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
}

/// Rewrites (`Some`) or clears (`None`) the cube map of every environment showing the Light Texture asset `from`
/// (a texture renamed or deleted); the paths of the rewritten environments, in path order. An environment that does
/// not load is left alone.
pub fn rewrite_texture_refs(project: &Project, from: &str, to: Option<&str>) -> io::Result<Vec<String>> {
    let mut rewritten = Vec::new();
    for r in scan(project).into_iter().filter(|r| r.suffix == ENVIRONMENT_SUFFIX) {
        let file = project.root.join(r.source_rel());
        let Ok(mut env) = load_environment(&file) else { continue };
        if env.cubemap.as_deref() != Some(from) {
            continue;
        }
        env.cubemap = to.map(str::to_string);
        save_environment(&file, &env)?;
        rewritten.push(r.path);
    }
    Ok(rewritten)
}
