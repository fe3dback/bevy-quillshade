//! The Data domain's [`DomainMigrator`] for `project_migrate` (ADR 0046, ADR 0049).
//!
//! Data domain sources (`.dto.ron`, `.collection.ron`) were introduced at project version 3 with `format_version: 1`
//! and UUIDv7 `id`s. [`DataMigrator`] checks `format_version >= 1` and a valid UUIDv7 root `id`, and can upgrade any
//! pre-UUID or nil-UUID Data source file to canonical format 1 during a v3 migration step.

use std::path::{Path, PathBuf};

use project_core::layout::QUILLSHADE_DIR;
use project_core::{AssetRef, AssetUuid, DomainId, ProjectVersion};
use project_migrate::{DomainMigrator, MigrationError, MigrationFacts, RawProject};
use serde::de::{self, DeserializeOwned};
use serde::{Deserialize, Deserializer};

use super::DATA;
use super::model::{
    COLLECTION_FORMAT_VERSION, COLLECTION_SUFFIX, CURVE_SUFFIX, CollectionAsset, DTO_FORMAT_VERSION, DTO_SUFFIX,
    DtoAsset, ENUM_FORMAT_VERSION, ENUM_SUFFIX, EnumAsset,
};
use super::ron_io::{collection_text, dto_text, enum_text};
use super::validate::scan_root;

/// Project version 3 (`ProjectVersion::CURRENT`).
pub const V3: ProjectVersion = ProjectVersion(3);

fn current_format(suffix: &str) -> u32 {
    if suffix == CURVE_SUFFIX { curve_kit::CURVE_FORMAT_VERSION } else { 1 }
}

/// The Data domain's [`DomainMigrator`] implementation.
#[derive(Debug, Clone, Copy, Default)]
pub struct DataMigrator;

impl DomainMigrator for DataMigrator {
    fn domain(&self) -> DomainId {
        DATA
    }

    fn files(&self, project: &RawProject) -> Vec<PathBuf> {
        scan_root(&project.root).into_iter().map(|r| project.root.join(r.source_rel())).collect()
    }

    fn is_fresh(&self, file: &Path, text: &str, _target: ProjectVersion) -> Result<bool, MigrationError> {
        let source = locate(file).map_err(|e| fail(file, e))?;
        let header = Header::probe(text).map_err(|e| fail(file, e))?;
        let format = header.format().map_err(|e| fail(file, e))?;
        Ok(format >= current_format(source.suffix) && header.has_v7_id())
    }

    fn collect(&self, file: &Path, text: &str, target: ProjectVersion, facts: &mut MigrationFacts) {
        if target != V3 {
            return;
        }
        let (Ok(source), Ok(header)) = (locate(file), Header::probe(text)) else { return };
        let Ok(format) = header.format() else { return };
        let stale = format < current_format(source.suffix) || !header.has_v7_id();
        if let Some(uuid) = v3_uuid(&source, &header, stale) {
            facts.insert(DATA, format!("{}:{}", source.suffix, source.path), uuid.to_string());
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
        if target > ProjectVersion::CURRENT {
            return Err(fail(file, format!("the Data domain has no step to {target}")));
        }
        migrate_to_current(&source, text, facts).map_err(|e| fail(file, e))
    }
}

fn fail(file: &Path, message: String) -> MigrationError {
    MigrationError::domain(DATA, file, message.replace('\n', " "))
}

fn locate(file: &Path) -> Result<AssetRef, String> {
    let data_dir = Path::new(QUILLSHADE_DIR).join(DATA.as_str());
    let root = file
        .ancestors()
        .skip(1)
        .filter(|dir| dir.ends_with(&data_dir))
        .filter_map(|dir| dir.parent().and_then(Path::parent))
        .find(|root| root.join("Cargo.toml").is_file())
        .ok_or_else(|| format!("not inside a project's `{}/`", data_dir.display()))?;
    let rel = file.strip_prefix(root).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
    project_core::asset_ref::parse(&rel, DATA, &[DTO_SUFFIX, ENUM_SUFFIX, COLLECTION_SUFFIX, CURVE_SUFFIX])
        .ok_or_else(|| format!("`{rel}` is not a Data source"))
}

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

    fn has_v7_id(&self) -> bool {
        self.id.as_deref().and_then(AssetUuid::parse).is_some_and(|id| id.is_v7())
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

fn v3_uuid(source: &AssetRef, header: &Header, stale: bool) -> Option<AssetUuid> {
    header
        .id
        .as_deref()
        .and_then(AssetUuid::parse)
        .filter(AssetUuid::is_v7)
        .or_else(|| stale.then(|| AssetUuid::legacy(&source.source_rel())))
}

fn parse<T: DeserializeOwned>(text: &str) -> Result<T, String> {
    ron::Options::default().from_str(text).map_err(|e| format!("parse error: {e}"))
}

fn migrate_to_current(source: &AssetRef, text: &str, facts: &MigrationFacts) -> Result<String, String> {
    let header = Header::probe(text)?;
    let uuid = v3_uuid(source, &header, true).expect("migrated file always gets a UUID");
    match source.suffix {
        DTO_SUFFIX => {
            let mut dto: DtoAsset = parse(text)?;
            dto.format_version = DTO_FORMAT_VERSION;
            dto.id = uuid;
            for (i, field) in dto.fields.iter_mut().enumerate() {
                if !field.id.is_v7() {
                    field.id = AssetUuid::legacy(&format!("{}#field:{i}:{}", source.source_rel(), field.name));
                }
            }
            Ok(dto_text(&dto))
        }
        ENUM_SUFFIX => {
            let mut en: EnumAsset = parse(text)?;
            en.format_version = ENUM_FORMAT_VERSION;
            en.id = uuid;
            for (vi, variant) in en.variants.iter_mut().enumerate() {
                if !variant.id.is_v7() {
                    variant.id = AssetUuid::legacy(&format!("{}#variant:{vi}:{}", source.source_rel(), variant.name));
                }
                for (fi, field) in variant.fields.iter_mut().enumerate() {
                    if !field.id.is_v7() {
                        field.id = AssetUuid::legacy(&format!(
                            "{}#variant:{vi}:{}:field:{fi}:{}",
                            source.source_rel(),
                            variant.name,
                            field.name
                        ));
                    }
                }
            }
            Ok(enum_text(&en))
        }
        COLLECTION_SUFFIX => {
            let mut col: CollectionAsset = parse(text)?;
            col.format_version = COLLECTION_FORMAT_VERSION;
            col.id = uuid;
            for (i, row) in col.rows.iter_mut().enumerate() {
                if !row.id.is_v7() {
                    row.id = AssetUuid::legacy(&format!("{}#row:{i}", source.source_rel()));
                }
            }
            let dto_path = facts.iter(DATA).find_map(|(k, v)| {
                let path = k.strip_prefix("dto:")?;
                (AssetUuid::parse(v) == Some(col.dto)).then_some(path)
            });
            Ok(collection_text(&col, dto_path, None))
        }
        CURVE_SUFFIX => {
            let mut curve: curve_kit::Curve = parse(text)?;
            curve.format_version = curve_kit::CURVE_FORMAT_VERSION;
            curve.id = uuid;
            Ok(curve.to_ron())
        }
        _ => Err(format!("`{source}` is not a Data source")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrator_detects_fresh_and_stale_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::create_dir_all(root.join("quillshade/data/library")).unwrap();
        let dto_file = root.join("quillshade/data/library/book.dto.ron");

        let fresh_id = AssetUuid::legacy("quillshade/data/library/book.dto.ron");
        let fresh_text = format!("DtoAsset(\n  format_version: 1,\n  id: \"{fresh_id}\",\n  fields: [],\n)\n");
        std::fs::write(&dto_file, &fresh_text).unwrap();

        let migrator = DataMigrator;
        let raw = RawProject::open(root).unwrap();
        assert_eq!(migrator.files(&raw), vec![dto_file.clone()]);
        assert!(migrator.is_fresh(&dto_file, &fresh_text, V3).unwrap());

        let stale_text = "DtoAsset(\n  format_version: 1,\n  fields: [],\n)\n";
        assert!(!migrator.is_fresh(&dto_file, stale_text, V3).unwrap());
        let mut facts = MigrationFacts::default();
        migrator.collect(&dto_file, stale_text, V3, &mut facts);
        let upgraded = migrator.migrate(&dto_file, stale_text, V3, &facts).unwrap();
        assert!(migrator.is_fresh(&dto_file, &upgraded, V3).unwrap());
    }
}
