//! The Light domain's files (design §9.2): environments (`quillshade/light/<path>.environment.ron`) and cube-map
//! Texture assets (`quillshade/light/<path>.texture.ron`, the texture kit's model under [`LIGHT_TEXTURES`]).

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

/// What a Light Texture asset accepts (design §9.2): KTX2 cube maps only, no built-in textures, the plain settings
/// form.
pub const LIGHT_TEXTURES: TexturePolicy =
    TexturePolicy { extensions: &["ktx2"], shape: TextureShape::CubeMap, internal: None, presets: false };

/// An environment (design §9.2): the cube map of the sky and the environment light, and how bright they are.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Environment {
    pub format_version: u32,
    /// The Light Texture asset of the cube map: its path, domain-relative and without suffix (`textures/sky`).
    #[serde(default)]
    pub cubemap: Option<String>,
    /// Brightness of the skybox (Bevy's `Skybox::brightness`, cd/m²).
    #[serde(default = "default_skybox_brightness")]
    pub skybox_brightness: f32,
    /// Scale of the environment light (Bevy's `EnvironmentMapLight::intensity`).
    #[serde(default = "default_env_intensity")]
    pub env_intensity: f32,
}

fn default_skybox_brightness() -> f32 {
    1000.0
}

fn default_env_intensity() -> f32 {
    1.0
}

impl Default for Environment {
    fn default() -> Self {
        Environment {
            format_version: ENVIRONMENT_FORMAT_VERSION,
            cubemap: None,
            skybox_brightness: default_skybox_brightness(),
            env_intensity: default_env_intensity(),
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

/// Reads an environment file.
pub fn load_environment(file: &Path) -> Result<Environment, String> {
    let text = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    Environment::from_ron(&text)
}

/// Writes an environment file through a temporary file next to it, creating the parent directories.
pub fn save_environment(file: &Path, env: &Environment) -> io::Result<()> {
    project_core::writer::write_atomic(file, env.to_ron())
}

/// Every Light asset file of the project (`quillshade/light/**`), in path order; files and folders whose name starts
/// with `.` are skipped.
pub fn scan(project: &Project) -> Vec<AssetRef> {
    let dir = project.domain_dir(LIGHT);
    let mut out: Vec<AssetRef> = project_core::files::walk_files(&dir)
        .into_iter()
        .filter(|f| f.strip_prefix(&dir).is_ok_and(|rel| !rel.iter().any(|c| c.to_string_lossy().starts_with('.'))))
        .filter_map(|f| project_core::asset_ref::parse(&project.rel(&f), LIGHT, &[ENVIRONMENT_SUFFIX, TEXTURE_SUFFIX]))
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
