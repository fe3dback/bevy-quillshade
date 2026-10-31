//! The Light domain's backend (design §9.2, §6.5), Bevy-free: its files ([`model`]), their diagnostics
//! ([`validate()`]), the module it contributes to the umbrella crate ([`codegen`]) and its [`DomainBackend`] for
//! `project_core::regen::run`.

use std::collections::BTreeSet;

use project_core::regen::{DomainBackend, DomainOutput, DomainPlan, OutFile, OutRoot};
use project_core::umbrella::CrateContribution;
use project_core::{DomainId, Project};

pub mod codegen;
pub mod model;
pub mod validate;

pub use model::{
    CURVE_SUFFIX, ColorGradingDef, ColorGradingGlobalDef, ColorGradingSectionDef, DEFAULT_CLEAR_COLOR,
    DirectionalLightDef, ENVIRONMENT_FORMAT_VERSION, ENVIRONMENT_SUFFIX, Environment, FogDef, FogFalloffDef,
    LIGHT_TEXTURES, LightAssets, TEXTURE_SUFFIX, TonemappingMode, curve_ref, default_lights, environment_ref,
    load_curve, load_environment, rewrite_texture_refs, save_curve, save_environment, scan, texture_ref,
    to_pascal_case,
};
pub use validate::{scan_game_conflicts, validate, validate_in_memory, validate_texture, validate_textures};

/// The Light domain (`quillshade/light/`, `assets/quillshade/light/`, `crates/quillshade/src/light/`).
pub const LIGHT: DomainId = DomainId::new("light");

/// Bevy features required by the generated `quillshade::light` runtime module.
const LIGHT_BEVY_FEATURES: &[&str] = &[
    "std",
    "bevy_asset",
    "bevy_color",
    "bevy_core_pipeline",
    "bevy_curve",
    "bevy_image",
    "bevy_light",
    "bevy_log",
    "bevy_pbr",
    "bevy_render",
    "bevy_shader",
    "bevy_state",
    "ktx2",
    "zstd_rust",
];

/// The Light module of the umbrella crate: `light::QuillshadeLightInternalPlugin`, plus serde / ron for the baked
/// manifest.
pub const LIGHT_CRATE: CrateContribution = CrateContribution {
    domain: LIGHT,
    plugin: "QuillshadeLightInternalPlugin",
    bevy_features: LIGHT_BEVY_FEATURES,
    dependencies: &[("serde", r#"{ version = "1", features = ["derive"] }"#), ("ron", r#""0.12""#)],
};

/// The Light domain as a [`DomainBackend`]: generates `crates/quillshade/src/light/mod.rs` and
/// `assets/quillshade/light/autogen/baked.ron`.
pub struct LightBackend;

impl DomainBackend for LightBackend {
    fn id(&self) -> DomainId {
        LIGHT
    }

    fn crate_contribution(&self) -> CrateContribution {
        LIGHT_CRATE
    }

    /// Loads and validates `quillshade/light/`; generates the module and manifest for error-free assets. Nothing to
    /// compile: `compile` is ignored.
    fn plan(&self, project: &Project, _compile: bool) -> DomainPlan {
        let assets = LightAssets::load(project);
        plan_assets(project, &assets)
    }
}

/// Validates `assets` and builds the Light domain's plan (`crates/quillshade/src/light/mod.rs` and
/// `assets/quillshade/light/autogen/baked.ron` listing the environments, curves, and textures without an error).
pub fn plan_assets(project: &Project, assets: &LightAssets) -> DomainPlan {
    let diagnostics = validate(project, assets);
    let failed: BTreeSet<&str> =
        diagnostics.iter().filter(|d| d.is_error()).filter_map(|d| d.asset.as_deref()).collect();

    let valid_textures = assets
        .textures
        .iter()
        .filter(|(p, _)| !failed.contains(texture_ref(p).source_rel().as_str()))
        .map(|(p, t)| (p.clone(), t.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();

    let valid_environments = assets
        .environments
        .iter()
        .filter(|(p, _)| !failed.contains(environment_ref(p).source_rel().as_str()))
        .map(|(p, e)| {
            let mut env = e.clone();
            if env.cubemap.as_deref().is_some_and(|c| !valid_textures.contains_key(c)) {
                env.cubemap = None;
            }
            (p.clone(), env)
        })
        .collect();

    let valid_curves = assets
        .curves
        .iter()
        .filter(|(p, _)| !failed.contains(curve_ref(p).source_rel().as_str()))
        .map(|(p, c)| (p.clone(), c.clone()))
        .collect();

    let valid_assets = LightAssets {
        environments: valid_environments,
        textures: valid_textures,
        curves: valid_curves,
        load_errors: Vec::new(),
    };

    let date = project.settings.header_date.then(project_core::marker::civil_today);
    let mut mod_text = codegen::module(&valid_assets);
    let mut manifest_text = codegen::manifest(&valid_assets);
    if let Some(d) = &date {
        project_core::marker::insert_date(&mut mod_text, d);
        project_core::marker::insert_date(&mut manifest_text, d);
    }

    let module = project.module_dir(LIGHT);
    let output_dir = project.output_dir(LIGHT);
    let files = vec![
        OutFile { path: module.join("mod.rs"), bytes: mod_text.into_bytes(), owner: None },
        OutFile { path: output_dir.join("autogen/baked.ron"), bytes: manifest_text.into_bytes(), owner: None },
    ];
    let roots = vec![OutRoot { dir: module, binary: false }, OutRoot { dir: output_dir, binary: false }];
    DomainPlan {
        diagnostics,
        output: DomainOutput { domain: LIGHT, files, keep: BTreeSet::new(), roots, orphan_scan: true },
    }
}
