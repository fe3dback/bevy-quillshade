//! The Material domain's steps of the Migration Tool (techdoc §4.4, ADR 0046 "Registration", ADR 0047 §14).
//!
//! [`MaterialMigrator`] owns every Material source file: the assets under `quillshade/material/` (hidden directories
//! skipped, like [`assets::scan`]) and the user nodes `quillshade/material/.nodes/*.ron`, listed explicitly
//! (ADR 0047 §16). The per-file `format_version` is the idempotency marker; [`FORMATS`] maps a project version to the
//! format every Material kind (user nodes included) has at that version.
//!
//! **Step v3** — "Assign asset UUIDs and reference assets by UUID" (ADR 0047 §14):
//!
//! - `collect` publishes, for every file, `"<suffix>:<path>" -> uuid` (`path` = the domain-relative path without
//!   suffix; a user node has the empty suffix: `":.nodes/soft_clamp"`) and, for a user node, `"user:<ident>" -> uuid`
//!   (the first file in path order wins an ident, as in the registry). The UUID is the file's existing `id` if it has
//!   one, else [`AssetUuid::legacy`] of its `source_rel` — deterministic, so an interrupted run, its rerun and the dry
//!   run agree on every id. A format-2 file without an `id` (hand-written, AE004) publishes nothing: nothing may
//!   reference it until it has one.
//! - `is_fresh(v3)` = the file's `format_version >= 2` (a user node without the key is format 1).
//! - `migrate(v3)` parses the format-1 text without the loaders' checks (`ron` into the current model types for the
//!   asset kinds, into the frozen [`UserNodeV1`] for user nodes, whose current DTO renamed `id` to `ident`), sets
//!   `id` and `format_version: 2`, replaces every reference found in the facts by the target's UUID — function and
//!   user node kinds, and the `Curve` / `GlobalConsts` / `Texture` / `GradientCollection` params of the stdlib nodes
//!   (user nodes and functions have no params; sub-item ids stay) — and writes the canonical text: graphs through
//!   `Asset::to_ron` plus the D27 path comments ([`annotate_refs`], labels from the facts), the other kinds through
//!   their writers. Paths that name no file stay as they are (they were dangling already), as do `@internal/*`
//!   textures and `""`.
//!
//! Generated files are not migrated: the orchestrator's caller regenerates afterwards, and the output is unchanged
//! because the resolve seam ([`crate::resolve`]) feeds codegen the same paths. Writing through the current model
//! types and writers is allowed while v3 is the latest step (techdoc §4.4 "Rules"); a later step that changes one of
//! them moves this step to frozen copies first.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use project_core::asset_index::annotate_refs;
use project_core::{AssetUuid, DomainId, ProjectVersion};
use project_migrate::{DomainMigrator, MigrationError, MigrationFacts, RawProject};
use serde::Deserialize;
use serde::de::{self, DeserializeOwned};
use shader_graph_core::consts::GlobalConsts;
use shader_graph_core::curve::Curve;
use shader_graph_core::features::FeatureTable;
use shader_graph_core::gradient::GradientCollection;
use shader_graph_core::model::Asset;
use shader_graph_core::registry::FUNCTION_PREFIX;
use shader_graph_core::texture::TextureAsset;
use shader_graph_core::{PinType, Value};
use shader_stdlib::user::{USER_PREFIX, UserNode, UserPin};

use crate::assets::{self, AssetId, AssetType};
use crate::layout::{MATERIAL, MATERIAL_DIR, USER_NODES_DIR};
use crate::resolve::{ref_type, stdlib};

/// The project version whose step assigns asset UUIDs (ADR 0047 §14).
pub const V3: ProjectVersion = ProjectVersion(3);

/// `(project version, format_version)`: from that project version on, every Material source (each asset kind and
/// the user nodes) has that `format_version` (ADR 0047 §2: v1, v2 -> 1; v3 -> 2). Ordered by version.
pub const FORMATS: &[(ProjectVersion, u32)] = &[(ProjectVersion::FIRST, 1), (V3, 2)];

/// The `format_version` of Material sources at project version `v` ([`FORMATS`]).
pub fn format_at(v: ProjectVersion) -> u32 {
    FORMATS.iter().rev().find(|(at, _)| *at <= v).map_or(FORMATS[0].1, |(_, format)| *format)
}

/// Fact key prefix of a user node's ident (`"user:<ident>" -> uuid`); asset keys are `"<suffix>:<path>"`, and no
/// Material suffix is `user`.
const USER_KEY: &str = "user:";

/// The fact key of the asset with `suffix` at `path` (domain-relative, without suffix).
fn asset_key(suffix: &str, path: &str) -> String {
    format!("{suffix}:{path}")
}

/// The Material domain's [`DomainMigrator`] (registered through `EditorDomain::migrator()`).
#[derive(Debug, Clone, Copy, Default)]
pub struct MaterialMigrator;

impl DomainMigrator for MaterialMigrator {
    fn domain(&self) -> DomainId {
        MATERIAL
    }

    /// Every asset under `quillshade/material/` (hidden directories skipped) and every `.nodes/*.ron`.
    fn files(&self, project: &RawProject) -> Vec<PathBuf> {
        let root = project.domain_dir(MATERIAL);
        let mut files: Vec<PathBuf> = assets::scan_dir(&root)
            .into_iter()
            .map(|id| root.join(format!("{}.{}.ron", id.path, id.ty.suffix())))
            .collect();
        files.extend(assets::user_node_paths(&root));
        files
    }

    fn is_fresh(&self, file: &Path, text: &str, target: ProjectVersion) -> Result<bool, MigrationError> {
        let source = Source::locate(file).map_err(|e| fail(file, e))?;
        let header = Header::probe(text).map_err(|e| fail(file, e))?;
        Ok(header.format(&source).map_err(|e| fail(file, e))? >= format_at(target))
    }

    fn collect(&self, file: &Path, text: &str, target: ProjectVersion, facts: &mut MigrationFacts) {
        if target != V3 {
            return;
        }
        // A file that cannot be read publishes nothing; `is_fresh` reports it.
        let (Ok(source), Ok(header)) = (Source::locate(file), Header::probe(text)) else { return };
        let Ok(format) = header.format(&source) else { return };
        let stale = format < format_at(V3);
        let Some(uuid) = v3_uuid(&source, &header, stale) else { return };
        facts.insert(MATERIAL, asset_key(source.suffix(), &source.path), uuid.to_string());
        if source.kind == Kind::UserNode
            && let Some(ident) = header.ident(stale)
            && facts.get(MATERIAL, &format!("{USER_KEY}{ident}")).is_none()
        {
            facts.insert(MATERIAL, format!("{USER_KEY}{ident}"), uuid.to_string());
        }
    }

    fn migrate(
        &self,
        file: &Path,
        text: &str,
        target: ProjectVersion,
        facts: &MigrationFacts,
    ) -> Result<String, MigrationError> {
        let source = Source::locate(file).map_err(|e| fail(file, e))?;
        match target {
            V3 => migrate_v3(&source, text, facts).map_err(|e| fail(file, e)),
            _ => Err(fail(file, format!("the Material domain has no step to {target}"))),
        }
    }
}

fn fail(file: &Path, message: String) -> MigrationError {
    MigrationError::domain(MATERIAL, file, message.replace('\n', " "))
}

/// What a Material source file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Asset(AssetType),
    /// `quillshade/material/.nodes/<name>.ron`.
    UserNode,
}

/// A Material source file located in its project.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Source {
    kind: Kind,
    /// Domain-relative path without suffix (`env/rock_wet`, `.nodes/soft_clamp`): the D27 label.
    path: String,
    /// Project-relative file path (`quillshade/material/env/rock_wet.material.ron`): the input of
    /// [`AssetUuid::legacy`].
    source_rel: String,
}

impl Source {
    /// Locates `file` (absolute) below the innermost `<root>/quillshade/material/` whose `<root>` holds a
    /// `Cargo.toml` (every project root does), so nested `quillshade/material/` directories on either side of the
    /// project root cannot confuse it.
    fn locate(file: &Path) -> Result<Source, String> {
        let root = file
            .ancestors()
            .skip(1)
            .find(|dir| {
                dir.ends_with(MATERIAL_DIR)
                    && dir.parent().and_then(Path::parent).is_some_and(|r| r.join("Cargo.toml").is_file())
            })
            .ok_or_else(|| format!("not inside a project's `{MATERIAL_DIR}/`"))?;
        let rel = file.strip_prefix(root).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
        if let Some(name) = rel
            .strip_prefix(USER_NODES_DIR)
            .and_then(|r| r.strip_prefix('/'))
            .and_then(|r| r.strip_suffix(".ron"))
            .filter(|n| !n.contains('/'))
        {
            let r = assets::user_node_ref(name);
            return Ok(Source { kind: Kind::UserNode, source_rel: r.source_rel(), path: r.path });
        }
        let id: AssetId = assets::id_from_rel(&rel).ok_or_else(|| format!("`{rel}` is not a Material source"))?;
        Ok(Source { kind: Kind::Asset(id.ty), source_rel: id.source_rel(), path: id.path })
    }

    fn suffix(&self) -> &'static str {
        match self.kind {
            Kind::Asset(ty) => ty.suffix(),
            Kind::UserNode => "",
        }
    }
}

/// The top-level keys of a source file that the step needs, read struct-name agnostic (any kind, any format).
#[derive(Debug, Default)]
struct Header {
    format_version: Option<u32>,
    /// The asset UUID from format 2 on; a format-1 user node's code identifier.
    id: Option<String>,
    /// A format-2 user node's code identifier.
    ident: Option<String>,
}

impl Header {
    fn probe(text: &str) -> Result<Header, String> {
        ron::Options::default().from_str(text).map_err(|e| format!("parse error: {e}"))
    }

    /// The file's `format_version`; a user node without the key is format 1 (ADR 0047 §16), any other kind must
    /// have it.
    fn format(&self, source: &Source) -> Result<u32, String> {
        match (self.format_version, source.kind) {
            (Some(v), _) => Ok(v),
            (None, Kind::UserNode) => Ok(format_at(ProjectVersion::FIRST)),
            (None, Kind::Asset(_)) => Err("no `format_version` field".into()),
        }
    }

    /// A user node's code identifier: `id` in format 1 (`stale`), `ident` from format 2 on.
    fn ident(&self, stale: bool) -> Option<&str> {
        if stale { self.id.as_deref() } else { self.ident.as_deref() }.filter(|i| !i.is_empty())
    }
}

impl<'de> Deserialize<'de> for Header {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Header, D::Error> {
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
                        "ident" => h.ident = Some(map.next_value()?),
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

/// The UUID a file has at v3: its existing `id` (not a format-1 user node's, which is its code identifier), else
/// [`AssetUuid::legacy`] when the step migrates it (`stale`), else none (a format-2 file without an id is AE004).
fn v3_uuid(source: &Source, header: &Header, stale: bool) -> Option<AssetUuid> {
    let existing = if source.kind == Kind::UserNode && stale { None } else { header.id.as_deref() };
    existing
        .and_then(AssetUuid::parse)
        .filter(|id| !id.is_nil())
        .or_else(|| stale.then(|| AssetUuid::legacy(&source.source_rel)))
}

/// Parses `text` into `T` without any loader's version dispatch or checks.
fn parse<T: DeserializeOwned>(text: &str) -> Result<T, String> {
    ron::Options::default().from_str(text).map_err(|e| format!("parse error: {e}"))
}

/// One format-1 file in its format-2 text (see the module docs).
fn migrate_v3(source: &Source, text: &str, facts: &MigrationFacts) -> Result<String, String> {
    let header = Header::probe(text)?;
    let uuid = v3_uuid(source, &header, true).expect("a migrated file always gets a UUID");
    let format_version = format_at(V3);
    Ok(match source.kind {
        Kind::Asset(AssetType::Material | AssetType::Function) => {
            let mut a: Asset = parse(text)?;
            (a.format_version, a.id) = (format_version, uuid);
            rewrite_refs(&mut a, facts);
            annotate_refs(&a.to_ron(), &labels(facts), MATERIAL)
        }
        Kind::Asset(AssetType::Curve) => {
            let mut c: Curve = parse(text)?;
            (c.format_version, c.id) = (format_version, uuid);
            c.to_ron()
        }
        Kind::Asset(AssetType::Consts) => {
            let mut t: GlobalConsts = parse(text)?;
            (t.format_version, t.id) = (format_version, uuid);
            t.to_ron()
        }
        Kind::Asset(AssetType::Features) => {
            let mut t: FeatureTable = parse(text)?;
            (t.format_version, t.id) = (format_version, uuid);
            t.to_ron()
        }
        Kind::Asset(AssetType::Texture) => {
            let mut t: TextureAsset = parse(text)?;
            (t.format_version, t.id) = (format_version, uuid);
            t.to_ron()
        }
        Kind::Asset(AssetType::Gradients) => {
            let mut g: GradientCollection = parse(text)?;
            (g.format_version, g.id) = (format_version, uuid);
            g.to_ron()
        }
        Kind::UserNode => parse::<UserNodeV1>(text)?.into_v2(format_version, uuid)?.to_ron(),
    })
}

/// Replaces every reference of `a` that the facts know by the target's UUID: `function::<path>` and
/// `user::<ident>` kinds, and the asset reference params of the stdlib nodes (found through the stdlib registry; the
/// project's user nodes and functions have no params).
fn rewrite_refs(a: &mut Asset, facts: &MigrationFacts) {
    for n in a.nodes.values_mut() {
        if let Some(path) = n.kind.strip_prefix(FUNCTION_PREFIX)
            && let Some(to) = facts.get(MATERIAL, &asset_key(AssetType::Function.suffix(), path))
        {
            n.kind = format!("{FUNCTION_PREFIX}{to}");
        }
        if let Some(ident) = n.kind.strip_prefix(USER_PREFIX)
            && let Some(to) = facts.get(MATERIAL, &format!("{USER_KEY}{ident}"))
        {
            n.kind = format!("{USER_PREFIX}{to}");
        }
        let Some(def) = stdlib().get(&n.kind) else { continue };
        for p in def.params() {
            if let Some(ty) = ref_type(&p.kind)
                && let Some(Value::Str(v)) = n.params.get_mut(&p.id)
                && let Some(to) = facts.get(MATERIAL, &asset_key(ty.suffix(), v))
            {
                *v = to.to_string();
            }
        }
    }
}

/// The D27 labels of every published asset: UUID -> (Material, domain-relative path without suffix).
fn labels(facts: &MigrationFacts) -> BTreeMap<AssetUuid, (DomainId, String)> {
    facts
        .iter(MATERIAL)
        .filter(|(key, _)| !key.starts_with(USER_KEY))
        .filter_map(|(key, uuid)| Some((AssetUuid::parse(uuid)?, (MATERIAL, key.split_once(':')?.1.to_string()))))
        .collect()
}

/// A user node file in format 1 (frozen: the current [`UserNode`] reads `ident` where format 1 has `id`): no
/// `format_version` key, the code identifier in `id`.
#[derive(Deserialize)]
#[serde(rename = "UserNode")]
struct UserNodeV1 {
    #[serde(default)]
    id: String,
    name: String,
    #[serde(default = "default_category")]
    category: String,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    description: String,
    inputs: Vec<UserPinV1>,
    output: PinType,
    template: String,
}

/// A user node pin in format 1 (frozen with [`UserNodeV1`]).
#[derive(Deserialize)]
#[serde(rename = "UserPin")]
struct UserPinV1 {
    name: String,
    ty: PinType,
    #[serde(default)]
    default: Option<Value>,
}

fn default_category() -> String {
    "User".into()
}

impl UserNodeV1 {
    fn into_v2(self, format_version: u32, id: AssetUuid) -> Result<UserNode, String> {
        if self.id.is_empty() {
            return Err("the user node has no `id` (its code identifier in this format)".into());
        }
        Ok(UserNode {
            format_version,
            id,
            ident: self.id,
            name: self.name,
            category: self.category,
            keywords: self.keywords,
            description: self.description,
            inputs: self.inputs.into_iter().map(|p| UserPin { name: p.name, ty: p.ty, default: p.default }).collect(),
            output: self.output,
            template: self.template,
        })
    }
}

#[cfg(test)]
mod tests {
    use shader_graph_core::consts::CONSTS_FORMAT_VERSION;
    use shader_graph_core::curve::CURVE_FORMAT_VERSION;
    use shader_graph_core::gradient::GRADIENTS_FORMAT_VERSION;
    use shader_graph_core::model::FORMAT_VERSION;
    use shader_graph_core::texture::TEXTURE_FORMAT_VERSION;
    use shader_stdlib::user::USER_NODE_FORMAT_VERSION;

    use super::*;

    #[test]
    fn formats_table() {
        assert_eq!(format_at(ProjectVersion::FIRST), 1);
        assert_eq!(format_at(ProjectVersion(2)), 1);
        assert_eq!(format_at(V3), 2);
        assert_eq!(format_at(ProjectVersion(9)), 2);
        assert!(FORMATS.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 < w[1].1));
        // The latest step writes the current formats.
        for v in [
            FORMAT_VERSION,
            CURVE_FORMAT_VERSION,
            CONSTS_FORMAT_VERSION,
            GRADIENTS_FORMAT_VERSION,
            TEXTURE_FORMAT_VERSION,
            USER_NODE_FORMAT_VERSION,
        ] {
            assert_eq!(v, format_at(V3));
        }
    }

    #[test]
    fn locate_sources() {
        let dir = tempfile::tempdir().unwrap();
        // The project root itself lies below a `quillshade/material/` directory without a `Cargo.toml`.
        let root = dir.path().join("quillshade/material/game");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("Cargo.toml"), "").unwrap();
        let material = root.join("quillshade/material");
        let at = |rel: &str| Source::locate(&material.join(rel));
        assert_eq!(
            at("env/rock.material.ron").unwrap(),
            Source {
                kind: Kind::Asset(AssetType::Material),
                path: "env/rock".into(),
                source_rel: "quillshade/material/env/rock.material.ron".into(),
            }
        );
        // A nested `quillshade/material/` inside the domain is just a directory of assets.
        assert_eq!(at("quillshade/material/a.curve.ron").unwrap().path, "quillshade/material/a");
        assert_eq!(
            at(".nodes/soft_clamp.ron").unwrap(),
            Source {
                kind: Kind::UserNode,
                path: ".nodes/soft_clamp".into(),
                source_rel: "quillshade/material/.nodes/soft_clamp.ron".into(),
            }
        );
        assert!(at("env/readme.txt").is_err());
        assert!(at(".nodes/deeper/x.ron").is_err());
        assert!(Source::locate(&dir.path().join("elsewhere/a.curve.ron")).is_err());
    }

    #[test]
    fn header_probe_and_ident() {
        let v1 =
            Header::probe("UserNode(id: \"soft\", name: \"S\", inputs: [], output: Float, template: \"1.0\")").unwrap();
        assert_eq!((v1.format_version, v1.ident(true)), (None, Some("soft")));
        let v2 = Header::probe("UserNode(format_version: 2, ident: \"soft\", nested: (id: 3))").unwrap();
        assert_eq!((v2.format_version, v2.id.as_deref(), v2.ident(false)), (Some(2), None, Some("soft")));
        assert!(Header::probe("not ron").is_err());
    }
}
