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
    ENVIRONMENT_FORMAT_VERSION, ENVIRONMENT_SUFFIX, Environment, LIGHT_TEXTURES, LightAssets, TEXTURE_SUFFIX,
    environment_ref, load_environment, rewrite_texture_refs, save_environment, scan, texture_ref,
};
pub use validate::validate;

/// The Light domain (`quillshade/light/`, `assets/quillshade/light/`, `crates/quillshade/src/light/`).
pub const LIGHT: DomainId = DomainId::new("light");

/// The Light module of the umbrella crate: `light::QuillshadeLightInternalPlugin`; no Bevy feature, no dependency.
pub const LIGHT_CRATE: CrateContribution =
    CrateContribution { domain: LIGHT, plugin: "QuillshadeLightInternalPlugin", bevy_features: &[], dependencies: &[] };

/// The Light domain as a [`DomainBackend`]: one generated file, `crates/quillshade/src/light/mod.rs`.
pub struct LightBackend;

impl DomainBackend for LightBackend {
    fn id(&self) -> DomainId {
        LIGHT
    }

    fn crate_contribution(&self) -> CrateContribution {
        LIGHT_CRATE
    }

    /// Loads and validates `quillshade/light/`; the module lists the environments without an error. Nothing to
    /// compile: `compile` is ignored.
    fn plan(&self, project: &Project, _compile: bool) -> DomainPlan {
        let assets = LightAssets::load(project);
        plan_assets(project, &assets)
    }
}

/// Validates `assets` and builds the Light domain's plan (`crates/quillshade/src/light/mod.rs` listing the
/// environments without an error).
pub fn plan_assets(project: &Project, assets: &LightAssets) -> DomainPlan {
    let diagnostics = validate(project, assets);
    let failed: BTreeSet<&str> =
        diagnostics.iter().filter(|d| d.is_error()).filter_map(|d| d.asset.as_deref()).collect();
    let sources: Vec<(String, &str)> =
        assets.environments.keys().map(|p| (environment_ref(p).source_rel(), p.as_str())).collect();
    let listed: Vec<&str> = sources.iter().filter(|(rel, _)| !failed.contains(rel.as_str())).map(|(_, p)| *p).collect();
    let module = project.module_dir(LIGHT);
    let file = OutFile { path: module.join("mod.rs"), bytes: codegen::module(&listed).into_bytes(), owner: None };
    let roots = vec![OutRoot { dir: module, binary: false }, OutRoot { dir: project.output_dir(LIGHT), binary: false }];
    DomainPlan {
        diagnostics,
        output: DomainOutput { domain: LIGHT, files: vec![file], keep: BTreeSet::new(), roots, orphan_scan: true },
    }
}
