//! Compile check of generated WESL with `wesl` 0.6 (the compiler Bevy 0.20 uses) against the real
//! Bevy WESL sources, followed by naga validation (spec §10.2, 15).

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use wesl::syntax::{ModulePath, PathOrigin};
use wesl::{CompileOptions, Feature, Features, Resolver};

/// Bevy crates whose WESL modules are reachable from materials.
const BEVY_CRATES: [&str; 4] = ["bevy_pbr", "bevy_render", "bevy_core_pipeline", "bevy_sprite_render"];
const BEVY_VERSION: &str = "0.20";

/// Bevy WESL sources indexed by module path (`bevy_pbr::render::forward_io`).
#[derive(Clone, Default)]
pub struct BevySources {
    modules: HashMap<ModulePath, String>,
}

impl BevySources {
    /// Finds Bevy 0.20 sources in the cargo registry (`$CARGO_HOME/registry/src/*/bevy_pbr-0.20.*`).
    pub fn discover() -> Option<Self> {
        let home = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))?;
        let registry = home.join("registry").join("src");
        for index in std::fs::read_dir(&registry).ok()?.flatten() {
            let mut best: Option<(String, PathBuf)> = None;
            for entry in std::fs::read_dir(index.path()).ok()?.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Some(ver) = name.strip_prefix("bevy_pbr-")
                    && ver.starts_with(BEVY_VERSION)
                    && best.as_ref().is_none_or(|(b, _)| ver > b.as_str())
                {
                    best = Some((ver.to_string(), index.path()));
                }
            }
            if let Some((ver, dir)) = best {
                return Self::load(&dir, &ver);
            }
        }
        None
    }

    /// Loads `{crate}-{version}/src/**/*.wesl` for every Bevy crate in `dir`.
    pub fn load(dir: &Path, version: &str) -> Option<Self> {
        let mut modules = HashMap::new();
        for krate in BEVY_CRATES {
            let src = dir.join(format!("{krate}-{version}")).join("src");
            if !src.is_dir() {
                return None;
            }
            let mut stack = vec![src.clone()];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(&d).ok()?.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if p.extension().is_some_and(|x| x == "wesl") {
                        let rel = p.strip_prefix(&src).ok()?.with_extension("");
                        let components =
                            rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
                        let text = std::fs::read_to_string(&p).ok()?;
                        modules.insert(ModulePath::new(PathOrigin::Package(krate.to_string()), components), text);
                    }
                }
            }
        }
        Some(BevySources { modules })
    }
}

/// Pipeline variant to compile for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// 3D forward main pass, mesh with UVs/tangents.
    Forward3d,
    /// 3D forward pass, mesh without UVs/tangents/colors.
    Forward3dBare,
    /// 3D deferred (prepass pipeline).
    Deferred3d,
    /// 2D sprite / mesh2d.
    Sprite2d,
}

impl Variant {
    fn flags(self) -> (Vec<&'static str>, u32) {
        let mut f = vec![
            "VERTEX_POSITIONS",
            "VERTEX_NORMALS",
            "VERTEX_OUTPUT_INSTANCE_INDEX",
            "AVAILABLE_STORAGE_BUFFER_BINDINGS__GE_3",
            "AVAILABLE_STORAGE_BUFFER_BINDINGS__GE_6",
            "VIEW_PROJECTION_PERSPECTIVE",
        ];
        match self {
            Variant::Forward3d => f.extend(["VERTEX_UVS", "VERTEX_UVS_A", "VERTEX_TANGENTS", "VERTEX_COLORS"]),
            Variant::Forward3dBare => {}
            Variant::Deferred3d => f.extend([
                "VERTEX_UVS",
                "VERTEX_UVS_A",
                "VERTEX_TANGENTS",
                "PREPASS_PIPELINE",
                "PREPASS_FRAGMENT",
                "DEFERRED_PREPASS",
                "NORMAL_PREPASS_OR_DEFERRED_PREPASS",
            ]),
            Variant::Sprite2d => f.extend(["VERTEX_UVS", "VERTEX_COLORS"]),
        }
        let group = if self == Variant::Sprite2d { 2 } else { 3 };
        (f, group)
    }
}

struct CheckResolver<'a> {
    bevy: &'a BevySources,
    files: &'a HashMap<ModulePath, String>,
    constants: String,
}

impl Resolver for CheckResolver<'_> {
    fn resolve_source<'b>(&'b self, path: &ModulePath) -> Result<Cow<'b, str>, wesl::error::ResolveError> {
        // Same normalization as Bevy's resolver: `Package("bevy_pbr/bevy_render")` → `bevy_render`.
        let normalized;
        let path = match &path.origin {
            PathOrigin::Package(p) if p.contains('/') => {
                let last = p.rsplit('/').next().unwrap_or(p).to_string();
                normalized = ModulePath::new(PathOrigin::Package(last), path.components.clone());
                &normalized
            }
            _ => path,
        };
        if path.origin == PathOrigin::Package("constants".into()) && path.components.is_empty() {
            return Ok(Cow::Borrowed(&self.constants));
        }
        if let Some(s) = self.files.get(path).or_else(|| self.bevy.modules.get(path)) {
            return Ok(Cow::Borrowed(s));
        }
        Err(wesl::error::ResolveError::ModuleNotFound(path.clone(), "not found".into()))
    }
}

/// Generated autogen files for the check, keyed by module components (`shaders/autogen/...`).
#[derive(Default)]
pub struct Files {
    modules: HashMap<ModulePath, String>,
}

impl Files {
    pub fn add(&mut self, module: &[String], text: impl Into<String>) {
        self.modules.insert(ModulePath::new(PathOrigin::Absolute, module.to_vec()), text.into());
    }
}

#[derive(Debug, Clone)]
pub struct CheckError {
    pub variant: Variant,
    pub message: String,
}

impl std::fmt::Display for CheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{:?}] {}", self.variant, self.message)
    }
}

/// Compiles module `main` for `variant`; returns the WGSL on success.
pub fn compile(bevy: &BevySources, files: &Files, main: &[String], variant: Variant) -> Result<String, CheckError> {
    compile_with(bevy, files, main, variant, true)
}

/// Like [`compile`]; `strip: false` keeps every declaration (for modules without entry points).
pub fn compile_with(
    bevy: &BevySources,
    files: &Files,
    main: &[String],
    variant: Variant,
    strip: bool,
) -> Result<String, CheckError> {
    let (flags, group) = variant.flags();
    let constants = format!(
        "const MATERIAL_BIND_GROUP = {group};\nconst MAX_DIRECTIONAL_LIGHTS = 10;\nconst MAX_CASCADES_PER_LIGHT = 4;\nconst MAX_RECT_LIGHTS = 8;\nconst AVAILABLE_STORAGE_BUFFER_BINDINGS = 8;\n"
    );
    let resolver = CheckResolver { bevy, files: &files.modules, constants };
    let mut features = Features::default();
    for f in flags.iter().chain(
        [
            "MATERIAL_BIND_GROUP",
            "MAX_DIRECTIONAL_LIGHTS",
            "MAX_CASCADES_PER_LIGHT",
            "MAX_RECT_LIGHTS",
            "AVAILABLE_STORAGE_BUFFER_BINDINGS",
        ]
        .iter(),
    ) {
        features.flags.insert(f.to_string(), Feature::Enable);
    }
    let options =
        CompileOptions { imports: true, condcomp: true, visibility: false, strip, features, ..Default::default() };
    let path = ModulePath::new(PathOrigin::Absolute, main.to_vec());
    let err = |message: String| CheckError { variant, message };
    let wgsl = wesl::compile(&path, &options, &resolver).map_err(|e| err(e.to_string()))?.to_string();
    let module =
        naga::front::wgsl::parse_str(&wgsl).map_err(|e| err(format!("naga parse: {}", e.emit_to_string(&wgsl))))?;
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .map_err(|e| err(format!("naga validate: {}", e.emit_to_string(&wgsl))))?;
    Ok(wgsl)
}

/// Variants a material of the given kind is compiled with by Bevy.
pub fn variants_for(kind: &shader_graph_core::MaterialKind) -> &'static [Variant] {
    match kind {
        shader_graph_core::MaterialKind::Pbr3d(_) => &[Variant::Forward3d, Variant::Forward3dBare, Variant::Deferred3d],
        shader_graph_core::MaterialKind::Unlit3d(_) => &[Variant::Forward3d, Variant::Forward3dBare],
        shader_graph_core::MaterialKind::Sprite2d(_) => &[Variant::Sprite2d],
    }
}

/// Parses a standalone WESL snippet (Custom node fn) and returns the parser error, if any.
pub fn parse_error(source: &str) -> Option<String> {
    source.parse::<wesl::syntax::TranslationUnit>().err().map(|e| e.to_string())
}
