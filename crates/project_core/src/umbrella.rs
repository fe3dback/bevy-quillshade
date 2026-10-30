//! The generated umbrella crate `crates/quillshade` (design §4.2): `Cargo.toml` as the union of the domains'
//! contributions and `src/lib.rs` with `QuillshadePlugin`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::GENERATOR_VERSION;
use crate::domain::DomainId;
use crate::layout::{BevyDep, CRATE_NAME, Project};
use crate::marker::{self, insert_date};

/// What one domain adds to the umbrella crate.
#[derive(Debug, Clone, Copy)]
pub struct CrateContribution {
    pub domain: DomainId,
    /// Name of the domain's plugin inside its module (`QuillshadeMaterialInternalPlugin`).
    pub plugin: &'static str,
    pub bevy_features: &'static [&'static str],
    /// `(crate name, TOML value)` pairs, e.g. `("serde", r#"{ version = "1", features = ["derive"] }"#)`.
    pub dependencies: &'static [(&'static str, &'static str)],
}

/// Umbrella crate files as absolute paths: `Cargo.toml` and `src/lib.rs`.
pub fn files(project: &Project, crates: &[CrateContribution], date: Option<&str>) -> Vec<(PathBuf, String)> {
    let mut cargo = cargo_toml(project.bevy_dep(), crates);
    let mut lib = lib_rs(project, crates);
    if let Some(date) = date {
        insert_date(&mut lib, date);
        // `insert_date` writes a `//` comment, which TOML does not accept.
        if let Some(pos) = cargo.find('\n') {
            cargo.insert_str(pos + 1, &format!("# generated: {date}\n"));
        }
    }
    let dir = project.crate_dir();
    vec![(dir.join("Cargo.toml"), cargo), (dir.join("src").join("lib.rs"), lib)]
}

fn cargo_toml(bevy: BevyDep, crates: &[CrateContribution]) -> String {
    let features: BTreeSet<&str> = crates.iter().flat_map(|c| c.bevy_features.iter().copied()).collect();
    let features = format!("[{}]", features.iter().map(|f| format!("\"{f}\"")).collect::<Vec<_>>().join(", "));
    let bevy = match bevy {
        BevyDep::Version => format!(r#"{{ version = "0.20", default-features = false, features = {features} }}"#),
        BevyDep::Workspace => format!("{{ workspace = true, features = {features} }}"),
    };
    let mut deps: BTreeMap<&str, String> = BTreeMap::new();
    for c in crates {
        for &(name, value) in c.dependencies {
            deps.entry(name)
                .and_modify(|existing| *existing = merge_dep_values(existing, value))
                .or_insert_with(|| value.to_string());
        }
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
    let mut out = format!("{}\n", marker::header("//", None));
    out.push_str("//! Quillshade umbrella crate: add [`QuillshadePlugin`] to the app.\n\nuse bevy::prelude::*;\n\n");
    for c in crates {
        out.push_str(&format!("pub mod {};\n", c.domain));
    }
    out.push_str(&format!("\npub const AUTOGEN_VERSION: u32 = {GENERATOR_VERSION};\n\n"));
    out.push_str("/// Adds every enabled domain's plugin.\npub struct QuillshadePlugin;\n\n");
    let enabled: Vec<&CrateContribution> = crates.iter().filter(|c| project.domain_enabled(c.domain)).collect();
    out.push_str("impl Plugin for QuillshadePlugin {\n");
    if enabled.is_empty() {
        out.push_str("    fn build(&self, _app: &mut App) {}\n");
    } else {
        out.push_str("    fn build(&self, app: &mut App) {\n");
        for c in enabled {
            out.push_str(&format!("        app.add_plugins({}::{});\n", c.domain, c.plugin));
        }
        out.push_str("    }\n");
    }
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
            "(format_version: 1, domains: {\"light\": (enabled: false)})",
        )
        .unwrap();
        let p = Project::open(dir.path()).unwrap();
        (dir, p)
    }

    fn contrib(domain: &'static str, plugin: &'static str, features: &'static [&'static str]) -> CrateContribution {
        CrateContribution {
            domain: DomainId::new(domain),
            plugin,
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
            contrib("material", "QuillshadeMaterialInternalPlugin", &["bevy_pbr"]),
            contrib("light", "QuillshadeLightInternalPlugin", &[]),
        ];
        let lib = get(&files(&p, &cs, None), "src/lib.rs");
        assert!(lib.contains("pub mod material;") && lib.contains("pub mod light;"), "{lib}");
        assert!(lib.contains("material::QuillshadeMaterialInternalPlugin"), "{lib}");
        assert!(!lib.contains("light::QuillshadeLightInternalPlugin"), "{lib}");
        assert!(lib.contains("QuillshadePlugin"));
        let none = get(&files(&p, &[], None), "src/lib.rs");
        assert!(none.contains("fn build(&self, _app: &mut App) {}"), "{none}");
    }

    #[test]
    fn cargo_toml_unions_features_and_deps() {
        let (_d, p) = project();
        let cs = [contrib("material", "A", &["bevy_pbr", "bevy_asset"]), contrib("light", "B", &["bevy_asset", "x11"])];
        let toml = get(&files(&p, &cs, None), "Cargo.toml");
        assert_eq!(toml.matches("\"bevy_asset\"").count(), 1, "{toml}");
        assert!(toml.contains(r#"features = ["bevy_asset", "bevy_pbr", "x11"]"#), "{toml}");
        assert_eq!(toml.matches("serde =").count(), 1, "{toml}");
        assert!(toml.parse::<toml::Table>().is_ok());

        let c1 = CrateContribution {
            domain: DomainId::new("material"),
            plugin: "A",
            bevy_features: &[],
            dependencies: &[("serde", r#"{ version = "1", features = ["derive", "std"] }"#)],
        };
        let c2 = CrateContribution {
            domain: DomainId::new("light"),
            plugin: "B",
            bevy_features: &[],
            dependencies: &[("serde", r#"{ version = "1", features = ["rc", "derive"] }"#)],
        };
        let merged = get(&files(&p, &[c1, c2], None), "Cargo.toml");
        assert!(merged.contains(r#"serde = { version = "1", features = ["derive", "std", "rc"] }"#), "{merged}");
        assert!(merged.parse::<toml::Table>().is_ok());
    }

    #[test]
    fn umbrella_files_carry_the_marker() {
        let (_d, p) = project();
        let fs = files(&p, &[contrib("material", "A", &[])], Some("2026-01-02"));
        assert_eq!(fs.len(), 2);
        for (_, text) in &fs {
            assert!(has_marker(text), "{text}");
            assert_eq!(
                text.lines().nth(1),
                Some(if text.starts_with('#') { "# generated: 2026-01-02" } else { "// generated: 2026-01-02" })
            );
        }
    }
}
