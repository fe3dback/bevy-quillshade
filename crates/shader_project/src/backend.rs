//! The Material domain's backend (design §4.2, §6.5): what it contributes to the umbrella crate `crates/quillshade`
//! and its [`DomainBackend`] for `project_core::regen::run`.

use project_core::DomainId;
use project_core::layout::Project;
use project_core::regen::{DomainBackend, DomainPlan};
use project_core::umbrella::CrateContribution;
use shader_codegen_wesl::check::BevySources;

use crate::assets::AssetSet;
use crate::layout::MATERIAL;
use crate::regen::{RegenOptions, plan_set};

/// Bevy features the generated code needs; kept with `workspace = true` too (features are additive). `png` / `ktx2`
/// / `zstd_rust`: the image formats of Texture assets (design §3.4); `bevy_log`: warnings of the baked loader.
const BEVY_FEATURES: &[&str] = &[
    "std",
    "bevy_pbr",
    "bevy_sprite_render",
    "bevy_asset",
    "bevy_color",
    "bevy_mesh",
    "bevy_log",
    "png",
    "ktx2",
    "zstd_rust",
];

/// The Material module of the umbrella crate: `material::QuillshadeMaterialInternalPlugin`, plus serde / ron for the
/// baked manifest.
pub const MATERIAL_CRATE: CrateContribution = CrateContribution {
    domain: MATERIAL,
    plugin: "QuillshadeMaterialInternalPlugin",
    bevy_features: BEVY_FEATURES,
    dependencies: &[("serde", r#"{ version = "1", features = ["derive"] }"#), ("ron", r#""0.12""#)],
};

/// The Material domain as a [`DomainBackend`]: the whole project, orphan cleanup included.
#[derive(Default)]
pub struct MaterialBackend {
    /// Bevy WESL sources for the compile check (wesl + naga); `None` skips the check even when it is asked for,
    /// unless [`MaterialBackend::discover`] is set.
    pub compile_check: Option<BevySources>,
    /// Without [`MaterialBackend::compile_check`], a plan asked to compile finds the sources in the cargo registry
    /// itself (once), with a warning on stderr when they are missing: the CLI's `--compile`.
    pub discover: bool,
    found: std::sync::OnceLock<Option<BevySources>>,
}

impl MaterialBackend {
    /// A backend that finds the Bevy sources when a compile check is asked for (the CLI's, design §10).
    pub fn discovering() -> MaterialBackend {
        MaterialBackend { discover: true, ..MaterialBackend::default() }
    }

    fn sources(&self) -> Option<&BevySources> {
        if self.compile_check.is_some() || !self.discover {
            return self.compile_check.as_ref();
        }
        let found = self.found.get_or_init(|| {
            let s = BevySources::discover();
            if s.is_none() {
                eprintln!("warning: Bevy 0.20 WESL sources not found in the cargo registry; skipping compile check");
            }
            s
        });
        found.as_ref()
    }
}

impl DomainBackend for MaterialBackend {
    fn id(&self) -> DomainId {
        MATERIAL
    }

    fn crate_contribution(&self) -> CrateContribution {
        MATERIAL_CRATE
    }

    fn plan(&self, project: &Project, compile: bool) -> DomainPlan {
        let set = AssetSet::load_all(project);
        let compile_check = if compile { self.sources() } else { None };
        let (report, output) = plan_set(project, &set, &RegenOptions { compile_check, ..Default::default() });
        DomainPlan { diagnostics: report.diagnostics().map(|d| d.to_generic(MATERIAL)).collect(), output }
    }
}
