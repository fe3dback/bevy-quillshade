//! The Light domain's steps of the Migration Tool (techdoc §4.4, ADR 0046 "Registration", ADR 0047 §14).
//!
//! [`LightMigrator`] owns every Light source file: the environments, transition curves and cube-map Texture assets
//! under `quillshade/light/` (hidden files and folders skipped, like [`scan`](super::model::scan)). The per-file
//! `format_version` is the idempotency marker; [`FORMATS`] maps a project version to the format every Light kind has
//! at that version.
//!
//! **Step v3** — "Assign asset UUIDs and reference assets by UUID" (ADR 0047 §14):
//!
//! - `collect` publishes, for every file, `"<suffix>:<path>" -> uuid` (`path` = the domain-relative path without
//!   suffix). The UUID is the file's existing `id` if it has one, else [`AssetUuid::legacy`] of its `source_rel` —
//!   deterministic, so an interrupted run, its rerun and the dry run agree on every id. A format-2 file without an
//!   `id` (hand-written, AE004) publishes nothing: nothing may reference it until it has one.
//! - `is_fresh(v3)` = the file's `format_version >= 2`.
//! - `migrate(v3)` parses the format-1 text without the loaders' checks (`ron` into the current model types), sets
//!   `id` and `format_version: 2`, replaces an environment's `cubemap` path by the texture's UUID when the facts know
//!   it, and writes the canonical text: environments through `Environment::to_ron` plus the D27 path comment
//!   ([`annotate_refs`], labels from the facts), curves and textures through their kits' writers. A cube map path
//!   that names no file stays as it is (it was dangling already). The cube map's game file (`TextureAsset::path`)
//!   stays a path (D26).
//!
//! Generated files are not migrated: the orchestrator's caller regenerates afterwards, and the output is unchanged
//! because the resolve seam ([`LightAssets::resolve`](super::LightAssets::resolve)) feeds codegen the same paths and
//! codegen never copies a source `format_version` (ADR 0047 §15). Writing through the current model types and
//! writers is allowed while v3 is the latest step (techdoc §4.4 "Rules"); a later step that changes one of them
//! moves this step to frozen copies first.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use curve_kit::Curve;
use project_core::asset_index::annotate_refs;
use project_core::layout::QUILLSHADE_DIR;
use project_core::{AssetRef, AssetUuid, DomainId, ProjectVersion};
use project_migrate::{DomainMigrator, MigrationError, MigrationFacts, RawProject};
use serde::de::{self, DeserializeOwned};
use serde::{Deserialize, Deserializer};
use texture_kit::TextureAsset;

use super::LIGHT;
use super::model::{CURVE_SUFFIX, ENVIRONMENT_SUFFIX, Environment, TEXTURE_SUFFIX, scan_root};

/// The project version whose step assigns asset UUIDs (ADR 0047 §14).
pub const V3: ProjectVersion = ProjectVersion(3);

/// `(project version, format_version)`: from that project version on, every Light source (environments, curves,
/// textures) has that `format_version` (ADR 0047 §2: v1, v2 -> 1; v3 -> 2). Ordered by version.
pub const FORMATS: &[(ProjectVersion, u32)] = &[(ProjectVersion::FIRST, 1), (V3, 2)];

/// The `format_version` of Light sources at project version `v` ([`FORMATS`]).
pub fn format_at(v: ProjectVersion) -> u32 {
    FORMATS.iter().rev().find(|(at, _)| *at <= v).map_or(FORMATS[0].1, |(_, format)| *format)
}

/// The fact key of the asset with `suffix` at `path` (domain-relative, without suffix).
fn asset_key(suffix: &str, path: &str) -> String {
    format!("{suffix}:{path}")
}

/// The Light domain's [`DomainMigrator`] (registered through `EditorDomain::migrator()`).
#[derive(Debug, Clone, Copy, Default)]
pub struct LightMigrator;

impl DomainMigrator for LightMigrator {
    fn domain(&self) -> DomainId {
        LIGHT
    }

    /// Every environment, curve and texture under `quillshade/light/` (hidden files and folders skipped).
    fn files(&self, project: &RawProject) -> Vec<PathBuf> {
        scan_root(&project.root).into_iter().map(|r| project.root.join(r.source_rel())).collect()
    }

    fn is_fresh(&self, file: &Path, text: &str, target: ProjectVersion) -> Result<bool, MigrationError> {
        locate(file).map_err(|e| fail(file, e))?;
        let header = Header::probe(text).map_err(|e| fail(file, e))?;
        Ok(header.format().map_err(|e| fail(file, e))? >= format_at(target))
    }

    fn collect(&self, file: &Path, text: &str, target: ProjectVersion, facts: &mut MigrationFacts) {
        if target != V3 {
            return;
        }
        // A file that cannot be read publishes nothing; `is_fresh` reports it.
        let (Ok(source), Ok(header)) = (locate(file), Header::probe(text)) else { return };
        let Ok(format) = header.format() else { return };
        if let Some(uuid) = v3_uuid(&source, &header, format < format_at(V3)) {
            facts.insert(LIGHT, asset_key(source.suffix, &source.path), uuid.to_string());
        }
    }

    fn migrate(
        &self,
        file: &Path,
        text: &str,
        target: ProjectVersion,
        facts: &MigrationFacts,
    ) -> Result<String, MigrationError> {
        let source = locate(file).map_err(|e| fail(file, e))?;
        match target {
            V3 => migrate_v3(&source, text, facts).map_err(|e| fail(file, e)),
            _ => Err(fail(file, format!("the Light domain has no step to {target}"))),
        }
    }
}

fn fail(file: &Path, message: String) -> MigrationError {
    MigrationError::domain(LIGHT, file, message.replace('\n', " "))
}

/// Locates `file` (absolute) below the innermost `<root>/quillshade/light/` whose `<root>` holds a `Cargo.toml`
/// (every project root does), so nested `quillshade/light/` directories on either side of the project root cannot
/// confuse it.
fn locate(file: &Path) -> Result<AssetRef, String> {
    let light_dir = Path::new(QUILLSHADE_DIR).join(LIGHT.as_str());
    let root = file
        .ancestors()
        .skip(1)
        .filter(|dir| dir.ends_with(&light_dir))
        .filter_map(|dir| dir.parent().and_then(Path::parent))
        .find(|root| root.join("Cargo.toml").is_file())
        .ok_or_else(|| format!("not inside a project's `{}/`", light_dir.display()))?;
    let rel = file.strip_prefix(root).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
    project_core::asset_ref::parse(&rel, LIGHT, &[ENVIRONMENT_SUFFIX, TEXTURE_SUFFIX, CURVE_SUFFIX])
        .ok_or_else(|| format!("`{rel}` is not a Light source"))
}

/// The top-level keys of a source file that the step needs, read struct-name agnostic (any kind, any format).
#[derive(Debug, Default)]
struct Header {
    format_version: Option<u32>,
    id: Option<String>,
}

impl Header {
    fn probe(text: &str) -> Result<Header, String> {
        ron::Options::default().from_str(text).map_err(|e| format!("parse error: {e}"))
    }

    fn format(&self) -> Result<u32, String> {
        self.format_version.ok_or_else(|| "no `format_version` field".into())
    }
}

impl<'de> Deserialize<'de> for Header {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Header, D::Error> {
        struct V;
        impl<'de> de::Visitor<'de> for V {
            type Value = Header;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a struct")
            }

            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Header, A::Error> {
                let mut h = Header::default();
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "format_version" => h.format_version = Some(map.next_value()?),
                        "id" => h.id = Some(map.next_value()?),
                        _ => {
                            map.next_value::<de::IgnoredAny>()?;
                        }
                    }
                }
                Ok(h)
            }
        }
        d.deserialize_any(V)
    }
}

/// The UUID a file has at v3: its existing `id`, else [`AssetUuid::legacy`] when the step migrates it (`stale`), else
/// none (a format-2 file without an id is AE004).
fn v3_uuid(source: &AssetRef, header: &Header, stale: bool) -> Option<AssetUuid> {
    header
        .id
        .as_deref()
        .and_then(AssetUuid::parse)
        .filter(|id| !id.is_nil())
        .or_else(|| stale.then(|| AssetUuid::legacy(&source.source_rel())))
}

/// Parses `text` into `T` without any loader's version dispatch or checks.
fn parse<T: DeserializeOwned>(text: &str) -> Result<T, String> {
    ron::Options::default().from_str(text).map_err(|e| format!("parse error: {e}"))
}

/// One format-1 file in its format-2 text (see the module docs).
fn migrate_v3(source: &AssetRef, text: &str, facts: &MigrationFacts) -> Result<String, String> {
    let header = Header::probe(text)?;
    let uuid = v3_uuid(source, &header, true).expect("a migrated file always gets a UUID");
    let format_version = format_at(V3);
    Ok(match source.suffix {
        ENVIRONMENT_SUFFIX => {
            let mut e: Environment = parse(text)?;
            (e.format_version, e.id) = (format_version, uuid);
            if let Some(cubemap) = &mut e.cubemap
                && let Some(to) = facts.get(LIGHT, &asset_key(TEXTURE_SUFFIX, cubemap))
            {
                *cubemap = to.to_string();
            }
            annotate_refs(&e.to_ron(), &labels(facts), LIGHT)
        }
        CURVE_SUFFIX => {
            let mut c: Curve = parse(text)?;
            (c.format_version, c.id) = (format_version, uuid);
            c.to_ron()
        }
        _ => {
            let mut t: TextureAsset = parse(text)?;
            (t.format_version, t.id) = (format_version, uuid);
            t.to_ron()
        }
    })
}

/// The D27 labels of every published asset: UUID -> (Light, domain-relative path without suffix).
fn labels(facts: &MigrationFacts) -> BTreeMap<AssetUuid, (DomainId, String)> {
    facts
        .iter(LIGHT)
        .filter_map(|(key, uuid)| Some((AssetUuid::parse(uuid)?, (LIGHT, key.split_once(':')?.1.to_string()))))
        .collect()
}

#[cfg(test)]
mod tests {
    use curve_kit::CURVE_FORMAT_VERSION;
    use texture_kit::TEXTURE_FORMAT_VERSION;

    use super::super::model::ENVIRONMENT_FORMAT_VERSION;
    use super::*;

    #[test]
    fn formats_table() {
        assert_eq!(format_at(ProjectVersion::FIRST), 1);
        assert_eq!(format_at(ProjectVersion(2)), 1);
        assert_eq!(format_at(V3), 2);
        assert_eq!(format_at(ProjectVersion(9)), 2);
        assert!(FORMATS.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 < w[1].1));
        // The latest step writes the current formats.
        for v in [ENVIRONMENT_FORMAT_VERSION, CURVE_FORMAT_VERSION, TEXTURE_FORMAT_VERSION] {
            assert_eq!(v, format_at(V3));
        }
    }

    #[test]
    fn locate_sources() {
        let dir = tempfile::tempdir().unwrap();
        // The project root itself lies below a `quillshade/light/` directory without a `Cargo.toml`.
        let root = dir.path().join("quillshade/light/game");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("Cargo.toml"), "").unwrap();
        let light = root.join("quillshade/light");
        let at = |rel: &str| locate(&light.join(rel));
        let env = at("outdoor/day.environment.ron").unwrap();
        assert_eq!((env.path.as_str(), env.suffix), ("outdoor/day", ENVIRONMENT_SUFFIX));
        assert_eq!(env.source_rel(), "quillshade/light/outdoor/day.environment.ron");
        assert_eq!(at("sky.texture.ron").unwrap().suffix, TEXTURE_SUFFIX);
        assert_eq!(at("smooth.curve.ron").unwrap().suffix, CURVE_SUFFIX);
        // A nested `quillshade/light/` inside the domain is just a directory of assets.
        assert_eq!(at("quillshade/light/a.curve.ron").unwrap().path, "quillshade/light/a");
        assert!(at("readme.txt").is_err());
        assert!(at("x.material.ron").is_err());
        assert!(locate(&dir.path().join("elsewhere/a.curve.ron")).is_err());
    }

    #[test]
    fn header_probe() {
        let v1 = Header::probe("Environment(format_version: 1, cubemap: Some(\"sky\"))").unwrap();
        assert_eq!((v1.format().unwrap(), v1.id), (1, None));
        let v2 = Header::probe("Curve(format_version: 2, id: \"x\", name: \"c\")").unwrap();
        assert_eq!((v2.format().unwrap(), v2.id.as_deref()), (2, Some("x")));
        assert!(Header::probe("TextureAsset(path: \"a.ktx2\")").unwrap().format().is_err());
        assert!(Header::probe("not ron").is_err());
    }
}
