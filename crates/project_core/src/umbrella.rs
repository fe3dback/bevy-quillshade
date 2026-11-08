//! The generated umbrella crate `crates/quillshade` (design §4.2, ADR 0048): `Cargo.toml` as the union of the base
//! `common` requirements and the domains' contributions, `src/lib.rs` with `QuillshadePlugin`, and the shared
//! runtime module `src/common/` (`mod.rs`, `curve.rs`, `gradient.rs`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::GENERATOR_VERSION;
use crate::domain::DomainId;
use crate::layout::{BevyDep, CRATE_NAME, Project};
use crate::marker::{self, insert_date};

/// Bevy features always enabled on the umbrella crate because `src/common/` uses `bevy::color::Color` (ADR 0048 §4).
const COMMON_BEVY_FEATURES: &[&str] = &["bevy_color", "std"];

/// Crate dependencies always present in the umbrella crate because `src/common/` derives `Serialize`/`Deserialize`
/// (ADR 0048 §4).
const COMMON_DEPENDENCIES: &[(&str, &str)] = &[("serde", r#"{ version = "1", features = ["derive"] }"#)];

const COMMON_MOD_BODY: &str = "//! Shared runtime types and sampling (`CurveData`, `GradientData`).\n\n\
pub mod curve;\npub mod gradient;\n\n\
pub use curve::{CurveData, Interp};\npub use gradient::{GradientData, GradientStop};\n";

const CURVE_DOC: &str = "//! 1D transfer curve data (`CurveData`, `Interp`) and sampling.\n\n";
const GRADIENT_DOC: &str = "//! 1D OKLab colour gradient data (`GradientData`, `GradientStop`) and sampling.\n\n";

const CURVE_TEMPLATE: &str = include_str!("../templates/common/curve.rs");
const GRADIENT_TEMPLATE: &str = include_str!("../templates/common/gradient.rs");
const GRADIENT_BEVY_TEMPLATE: &str = include_str!("../templates/common/gradient_bevy.rs");

/// What one domain adds to the umbrella crate.
#[derive(Debug, Clone, Copy)]
pub struct CrateContribution {
    pub domain: DomainId,
    /// Name of the domain's plugin inside its module (`QuillshadeMaterialInternalPlugin`).
    pub plugin: &'static str,
    /// Optional domain loader `SystemParam` type inside its module (`BakedMaterialsLoader`).
    pub loader: Option<&'static str>,
    pub bevy_features: &'static [&'static str],
    /// `(crate name, TOML value)` pairs, e.g. `("serde", r#"{ version = "1", features = ["derive"] }"#)`.
    pub dependencies: &'static [(&'static str, &'static str)],
}

fn with_header(body: &str, date: Option<&str>) -> String {
    let mut out = format!("{}\n{body}", marker::header("//", None));
    if let Some(date) = date {
        insert_date(&mut out, date);
    }
    out
}

/// Umbrella crate files as absolute paths: `Cargo.toml`, `src/lib.rs`, and `src/common/{mod,curve,gradient}.rs`.
pub fn files(project: &Project, crates: &[CrateContribution], date: Option<&str>) -> Vec<(PathBuf, String)> {
    debug_assert!(
        crates.iter().all(|c| c.domain.as_str() != "common"),
        "domain id 'common' is reserved for quillshade::common"
    );
    let mut cargo = cargo_toml(project.bevy_dep(), crates);
    let mut lib = lib_rs(project, crates);
    if let Some(date) = date {
        insert_date(&mut lib, date);
        // `insert_date` writes a `//` comment, which TOML does not accept.
        if let Some(pos) = cargo.find('\n') {
            cargo.insert_str(pos + 1, &format!("# generated: {date}\n"));
        }
    }
    let common_mod = with_header(COMMON_MOD_BODY, date);
    let common_curve = with_header(&format!("{CURVE_DOC}{CURVE_TEMPLATE}"), date);
    let common_gradient = with_header(&format!("{GRADIENT_DOC}{GRADIENT_TEMPLATE}\n{GRADIENT_BEVY_TEMPLATE}"), date);
    let dir = project.crate_dir();
    let src = dir.join("src");
    let common = src.join("common");
    vec![
        (dir.join("Cargo.toml"), cargo),
        (src.join("lib.rs"), lib),
        (common.join("mod.rs"), common_mod),
        (common.join("curve.rs"), common_curve),
        (common.join("gradient.rs"), common_gradient),
    ]
}

fn cargo_toml(bevy: BevyDep, crates: &[CrateContribution]) -> String {
    let features: BTreeSet<&str> = COMMON_BEVY_FEATURES
        .iter()
        .copied()
        .chain(crates.iter().flat_map(|c| c.bevy_features.iter().copied()))
        .collect();
    let features = format!("[{}]", features.iter().map(|f| format!("\"{f}\"")).collect::<Vec<_>>().join(", "));
    let bevy = match bevy {
        BevyDep::Version => format!(r#"{{ version = "0.20", default-features = false, features = {features} }}"#),
        BevyDep::Workspace => format!("{{ workspace = true, features = {features} }}"),
    };
    let mut deps: BTreeMap<&str, String> = BTreeMap::new();
    for &(name, value) in COMMON_DEPENDENCIES.iter().chain(crates.iter().flat_map(|c| c.dependencies.iter())) {
        deps.entry(name)
            .and_modify(|existing| *existing = merge_dep_values(existing, value))
            .or_insert_with(|| value.to_string());
    }
    let mut out = format!(
        "{}\n[package]\nname = \"{CRATE_NAME}\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[dependencies]\nbevy = {bevy}\n",
        marker::header("#", None).replace(" — do not edit", " — do not edit (rewritten on every regeneration)"),
    );
    for (name, value) in deps {
        out.push_str(&format!("{name} = {value}\n"));
    }
    out
}

fn parse_dep_table(spec: &str) -> Option<toml::Table> {
    let doc: toml::Table = format!("d = {spec}").parse().ok()?;
    match doc.get("d")? {
        toml::Value::Table(t) => Some(t.clone()),
        toml::Value::String(v) => {
            let mut t = toml::Table::new();
            t.insert("version".into(), toml::Value::String(v.clone()));
            Some(t)
        }
        _ => None,
    }
}

/// Merges two dependency TOML specifications for the same crate, combining their `features` lists deterministically
/// (preserving first-seen order without duplicates).
fn merge_dep_values(existing: &str, incoming: &str) -> String {
    if existing == incoming {
        return existing.to_string();
    }
    let (Some(mut a), Some(b)) = (parse_dep_table(existing), parse_dep_table(incoming)) else {
        return existing.to_string();
    };
    let features_of = |t: &toml::Table| -> Vec<String> {
        t.get("features")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    };
    let mut features = Vec::new();
    for f in features_of(&a).into_iter().chain(features_of(&b)) {
        if !features.contains(&f) {
            features.push(f);
        }
    }
    for (k, v) in b {
        if k != "features" {
            a.entry(k).or_insert(v);
        }
    }
    a.remove("features");
    let mut parts: Vec<String> = Vec::new();
    for key in ["version", "workspace", "path", "default-features"] {
        if let Some(v) = a.remove(key) {
            parts.push(format!("{key} = {v}"));
        }
    }
    for (k, v) in a {
        parts.push(format!("{k} = {v}"));
    }
    if !features.is_empty() {
        let list = features.iter().map(|f| format!("\"{f}\"")).collect::<Vec<_>>().join(", ");
        parts.push(format!("features = [{list}]"));
    }
    format!("{{ {} }}", parts.join(", "))
}

fn lib_rs(project: &Project, crates: &[CrateContribution]) -> String {
    let enabled: Vec<&CrateContribution> = crates.iter().filter(|c| project.domain_enabled(c.domain)).collect();
    let loaders: Vec<(&CrateContribution, &'static str)> =
        enabled.iter().copied().filter_map(|c| c.loader.map(|l| (c, l))).collect();
    let mut out = format!("{}\n", marker::header("//", None));
    out.push_str(
        "//! Quillshade umbrella crate: add [`QuillshadePlugin`] to the app.\n\n\
         use bevy::ecs::system::SystemParam;\n\
         use bevy::prelude::*;\n\n",
    );
    out.push_str("pub mod common;\n");
    for c in &enabled {
        out.push_str(&format!("pub mod {};\n", c.domain));
    }
    out.push_str(&format!("\npub const AUTOGEN_VERSION: u32 = {GENERATOR_VERSION};\n\n"));

    out.push_str(
        "/// Aggregated loading progress across all enabled Quillshade domains.\n\
         ///\n\
         /// Emitted once per change while [`QuillshadeLoader::load`] (or automatic first-frame loading) is in flight;\n\
         /// loading finishes when `loaded + failed == total` ([`QuillshadeReady`]).\n\
         #[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]\n\
         pub struct QuillshadeProgress {\n\
         \x20   /// Assets that have finished loading successfully across all enabled domains.\n\
         \x20   pub loaded: u32,\n\
         \x20   /// Assets that failed to load across all enabled domains.\n\
         \x20   pub failed: u32,\n\
         \x20   /// Total assets tracked across all enabled domains.\n\
         \x20   pub total: u32,\n\
         }\n\n\
         /// Emitted once when all enabled Quillshade domains have finished loading their baked assets.\n\
         #[derive(Message, Debug, Clone, Copy, PartialEq, Eq, Default)]\n\
         pub struct QuillshadeReady;\n\n\
         #[derive(Resource, Default, Debug)]\n\
         struct QuillshadeLoaderState {\n\
         \x20   ready: bool,\n\
         \x20   sent_progress: Option<QuillshadeProgress>,\n\
         }\n\n\
         /// Unified [`SystemParam`] that starts or queries asset loading across all enabled Quillshade domains.\n\
         #[derive(SystemParam)]\n\
         pub struct QuillshadeLoader<'w> {\n\
         \x20   state: ResMut<'w, QuillshadeLoaderState>,\n",
    );
    for (c, loader) in &loaders {
        out.push_str(&format!("    {dom}: {dom}::{loader}<'w>,\n", dom = c.domain));
    }
    out.push_str("}\n\nimpl QuillshadeLoader<'_> {\n");
    out.push_str(
        "    /// Starts loading baked assets for all enabled domains if not already started.\n\
         \x20   ///\n\
         \x20   /// Returns the total number of tracked assets across all enabled domains.\n\
         \x20   pub fn load(&mut self) -> u32 {\n",
    );
    if loaders.is_empty() {
        out.push_str("        0\n");
    } else {
        let sum = loaders.iter().map(|(c, _)| format!("self.{}.load()", c.domain)).collect::<Vec<_>>().join(" + ");
        out.push_str(&format!("        {sum}\n"));
    }
    out.push_str("    }\n\n");
    if loaders.iter().any(|(c, _)| c.domain.as_str() == "material") {
        out.push_str(
            "    /// Loads the Material domain from `manifest` instead of [`material::COMPILED_MANIFEST`] (tests);\n\
             \x20   /// ignored once loading has started.\n\
             \x20   #[doc(hidden)]\n\
             \x20   pub fn load_from_str(&mut self, manifest: &str) {\n\
             \x20       self.material.load_from_str(manifest);\n\
             \x20   }\n\n",
        );
    }
    out.push_str(
        "    /// Current aggregated `(loaded, failed, total)` progress across all enabled domains.\n\
         \x20   pub fn progress(&self) -> QuillshadeProgress {\n",
    );
    if loaders.is_empty() {
        out.push_str("        QuillshadeProgress { loaded: 0, failed: 0, total: 0 }\n");
    } else {
        out.push_str("        let mut loaded = 0u32;\n        let mut failed = 0u32;\n        let mut total = 0u32;\n");
        for (c, _) in &loaders {
            out.push_str(&format!(
                "        let p = self.{dom}.progress();\n        loaded += p.loaded;\n        failed += p.failed;\n        total += p.total;\n",
                dom = c.domain
            ));
        }
        out.push_str("        QuillshadeProgress { loaded, failed, total }\n");
    }
    out.push_str(
        "    }\n\n\
         \x20   /// `true` once every enabled domain has finished loading its baked assets.\n\
         \x20   pub fn is_ready(&self) -> bool {\n\
         \x20       self.state.ready || self.domains_ready()\n\
         \x20   }\n\n\
         \x20   fn domains_started(&self) -> bool {\n",
    );
    if loaders.is_empty() {
        out.push_str("        true\n");
    } else {
        let all =
            loaders.iter().map(|(c, _)| format!("self.{}.is_started()", c.domain)).collect::<Vec<_>>().join(" && ");
        out.push_str(&format!("        {all}\n"));
    }
    out.push_str(
        "    }\n\n\
         \x20   fn domains_ready(&self) -> bool {\n",
    );
    if loaders.is_empty() {
        out.push_str("        true\n");
    } else {
        let all = loaders.iter().map(|(c, _)| format!("self.{}.is_ready()", c.domain)).collect::<Vec<_>>().join(" && ");
        out.push_str(&format!("        {all}\n"));
    }
    out.push_str(
        "    }\n\
         }\n\n\
         fn track_quillshade_progress(\n\
         \x20   mut loader: QuillshadeLoader,\n\
         \x20   mut progress_out: MessageWriter<QuillshadeProgress>,\n\
         \x20   mut ready_out: MessageWriter<QuillshadeReady>,\n\
         ) {\n\
         \x20   if loader.state.ready || !loader.domains_started() {\n\
         \x20       return;\n\
         \x20   }\n\
         \x20   let progress = loader.progress();\n\
         \x20   if loader.state.sent_progress != Some(progress) {\n\
         \x20       loader.state.sent_progress = Some(progress);\n\
         \x20       progress_out.write(progress);\n\
         \x20   }\n\
         \x20   if loader.domains_ready() && progress.loaded + progress.failed >= progress.total {\n\
         \x20       loader.state.ready = true;\n\
         \x20       ready_out.write(QuillshadeReady);\n\
         \x20   }\n\
         }\n\n",
    );

    let after_sets: String = loaders.iter().map(|(c, _)| format!(".after({}::LoadSet)", c.domain)).collect();
    out.push_str(&format!(
        "/// Registers [`QuillshadeProgress`], [`QuillshadeReady`], and [`QuillshadeLoader`] progress tracking.\n\
         #[derive(Default)]\n\
         pub struct QuillshadeLoaderPlugin;\n\n\
         impl Plugin for QuillshadeLoaderPlugin {{\n\
         \x20   fn build(&self, app: &mut App) {{\n\
         \x20       app.add_message::<QuillshadeProgress>()\n\
         \x20           .add_message::<QuillshadeReady>()\n\
         \x20           .init_resource::<QuillshadeLoaderState>()\n\
         \x20           .add_systems(PreUpdate, track_quillshade_progress{after_sets});\n\
         \x20   }}\n\
         }}\n\n\
         /// Adds every enabled domain's plugin.\n\
         pub struct QuillshadePlugin;\n\n\
         impl Plugin for QuillshadePlugin {{\n\
         \x20   fn build(&self, app: &mut App) {{\n",
    ));
    for c in enabled {
        out.push_str(&format!("        app.add_plugins({}::{});\n", c.domain, c.plugin));
    }
    out.push_str("        app.add_plugins(QuillshadeLoaderPlugin);\n");
    out.push_str("    }\n");
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marker::has_marker;

    fn project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
        std::fs::create_dir(dir.path().join("quillshade")).unwrap();
        std::fs::write(
            dir.path().join("quillshade/project.ron"),
            "(project_version: 3, domains: {\"light\": (enabled: false)})",
        )
        .unwrap();
        let p = Project::open(dir.path()).unwrap();
        (dir, p)
    }

    fn contrib(domain: &'static str, plugin: &'static str, features: &'static [&'static str]) -> CrateContribution {
        CrateContribution {
            domain: DomainId::new(domain),
            plugin,
            loader: None,
            bevy_features: features,
            dependencies: &[("serde", r#"{ version = "1", features = ["derive"] }"#)],
        }
    }

    fn get(files: &[(PathBuf, String)], name: &str) -> String {
        files.iter().find(|(p, _)| p.ends_with(name)).unwrap().1.clone()
    }

    #[test]
    fn lib_rs_adds_enabled_plugins_only() {
        let (_d, p) = project();
        let cs = [
            CrateContribution {
                loader: Some("BakedMaterialsLoader"),
                ..contrib("material", "QuillshadeMaterialInternalPlugin", &["bevy_pbr"])
            },
            CrateContribution {
                loader: Some("BakedLightLoader"),
                ..contrib("light", "QuillshadeLightInternalPlugin", &[])
            },
        ];
        let lib = get(&files(&p, &cs, None), "src/lib.rs");
        assert!(lib.contains("pub mod common;\npub mod material;\n") && !lib.contains("pub mod light;"), "{lib}");
        assert!(lib.contains("material::QuillshadeMaterialInternalPlugin"), "{lib}");
        assert!(!lib.contains("light::QuillshadeLightInternalPlugin"), "{lib}");
        assert!(lib.contains("    material: material::BakedMaterialsLoader<'w>,"), "{lib}");
        assert!(!lib.contains("pub material:"), "{lib}");
        assert!(!lib.contains("BakedLightLoader"), "{lib}");
        assert!(lib.contains("QuillshadePlugin"));
        assert!(lib.contains("QuillshadeLoader"));
        assert!(lib.contains("QuillshadeProgress"));
        assert!(lib.contains("QuillshadeReady"));

        // Zero contributions: `pub mod common;` and `QuillshadeLoader` are still emitted.
        let none = get(&files(&p, &[], None), "src/lib.rs");
        assert!(none.contains("pub mod common;\n"), "{none}");
        assert!(none.contains("pub struct QuillshadeLoader<'w>"), "{none}");
        assert!(none.contains("app.add_plugins(QuillshadeLoaderPlugin);"), "{none}");

        // All contributed domains disabled: `pub mod common;` is still emitted.
        let all_disabled =
            get(&files(&p, &[contrib("light", "QuillshadeLightInternalPlugin", &[])], None), "src/lib.rs");
        assert!(
            all_disabled.contains("pub mod common;\n") && !all_disabled.contains("pub mod light;"),
            "{all_disabled}"
        );
        assert!(!all_disabled.contains("QuillshadeLightInternalPlugin"), "{all_disabled}");
    }

    #[test]
    fn cargo_toml_unions_features_and_deps() {
        let (_d, p) = project();
        // Zero contributions still include the base `common` features (`bevy_color`, `std`) and `serde`.
        let base_toml = get(&files(&p, &[], None), "Cargo.toml");
        assert!(base_toml.contains(r#"features = ["bevy_color", "std"]"#), "{base_toml}");
        assert!(base_toml.contains(r#"serde = { version = "1", features = ["derive"] }"#), "{base_toml}");
        assert!(base_toml.parse::<toml::Table>().is_ok(), "{base_toml}");

        let cs = [contrib("material", "A", &["bevy_pbr", "bevy_asset"]), contrib("light", "B", &["bevy_asset", "x11"])];
        let toml = get(&files(&p, &cs, None), "Cargo.toml");
        assert_eq!(toml.matches("\"bevy_asset\"").count(), 1, "{toml}");
        assert!(toml.contains(r#"features = ["bevy_asset", "bevy_color", "bevy_pbr", "std", "x11"]"#), "{toml}");
        assert_eq!(toml.matches("serde =").count(), 1, "{toml}");
        assert!(toml.parse::<toml::Table>().is_ok());

        let c1 = CrateContribution {
            domain: DomainId::new("material"),
            plugin: "A",
            loader: None,
            bevy_features: &[],
            dependencies: &[("serde", r#"{ version = "1", features = ["derive", "std"] }"#)],
        };
        let c2 = CrateContribution {
            domain: DomainId::new("light"),
            plugin: "B",
            loader: None,
            bevy_features: &[],
            dependencies: &[("serde", r#"{ version = "1", features = ["rc", "derive"] }"#)],
        };
        let merged = get(&files(&p, &[c1, c2], None), "Cargo.toml");
        assert!(merged.contains(r#"serde = { version = "1", features = ["derive", "std", "rc"] }"#), "{merged}");
        assert!(merged.parse::<toml::Table>().is_ok());
    }

    #[test]
    fn common_files_match_templates_byte_for_byte() {
        let (_d, p) = project();
        let fs = files(&p, &[], None);
        let hdr = marker::header("//", None);
        assert_eq!(get(&fs, "src/common/mod.rs"), format!("{hdr}\n{COMMON_MOD_BODY}"));
        assert_eq!(get(&fs, "src/common/curve.rs"), format!("{hdr}\n{CURVE_DOC}{CURVE_TEMPLATE}"));
        assert_eq!(
            get(&fs, "src/common/gradient.rs"),
            format!("{hdr}\n{GRADIENT_DOC}{GRADIENT_TEMPLATE}\n{GRADIENT_BEVY_TEMPLATE}")
        );
    }

    #[test]
    fn umbrella_files_carry_the_marker() {
        let (_d, p) = project();
        let fs = files(&p, &[contrib("material", "A", &[])], Some("2026-01-02"));
        assert_eq!(fs.len(), 5);
        for (_, text) in &fs {
            assert!(has_marker(text), "{text}");
            assert_eq!(
                text.lines().nth(1),
                Some(if text.starts_with('#') { "# generated: 2026-01-02" } else { "// generated: 2026-01-02" })
            );
        }
    }

    #[test]
    #[should_panic(expected = "domain id 'common' is reserved")]
    fn reserved_common_domain_id_is_rejected() {
        let (_d, p) = project();
        let _ = files(&p, &[contrib("common", "CommonPlugin", &[])], None);
    }
}
