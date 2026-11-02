//! The built-in project migrator (ADR 0046 "Crate `project_migrate` (D38)": "`project.ron` is owned by the built-in
//! project domain"). The orchestrator appends it itself and runs it last in every step, so rewriting
//! `quillshade/project.ron` is the step's commit point (D39).
//!
//! The v2 step renames the legacy `format_version` key to `project_version`; every later step is a pure version
//! bump. The file is read into `project_core::ProjectSettings` with the same parser and rules as `Project::open`,
//! except that the top-level version key is accepted in either spelling and its value is replaced by the step's
//! target; the result is written in the canonical `project.ron` text (`project_core::layout::settings_text`). Keys
//! `ProjectSettings` does not know are dropped, as the editor's Project Settings tool and `Project::open` ignore them
//! too. When a later version changes `ProjectSettings` incompatibly, the older steps of this migrator move to a
//! frozen copy (techdoc §4.4 "Rules").
//!
//! A project without `project.ron` that the orchestrator migrates is a v1 project with sources (ADR 0046 "Missing
//! `project.ron`"): [`ProjectMigrator::files`] lists the file anyway, the orchestrator reads it as empty text, which is
//! never fresh, and `migrate` creates it from `ProjectSettings::default()` at the step's target.

use std::path::{Path, PathBuf};

use project_core::layout::settings_text;
use project_core::{DomainId, ProjectSettings, ProjectVersion, probe_project_version, probe_version_key};
use serde::Deserialize;
use serde::de;

use crate::{DomainMigrator, MigrationError, MigrationFacts, RawProject};

/// The legacy (v1) key of the project version.
const LEGACY_KEY: &str = "format_version";
/// The key of the project version from v2 on.
const KEY: &str = "project_version";

/// Migrates `quillshade/project.ron` (domain [`DomainId::PROJECT`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct ProjectMigrator;

impl DomainMigrator for ProjectMigrator {
    fn domain(&self) -> DomainId {
        DomainId::PROJECT
    }

    /// Always `quillshade/project.ron`, also when it is missing (the orchestrator reads a missing one as `""`).
    fn files(&self, project: &RawProject) -> Vec<PathBuf> {
        vec![project.settings_path()]
    }

    /// Fresh when `project_version` is at least `target`. Never while the legacy `format_version` key is present:
    /// only v1 is spelled that way (ADR 0046), so the file still needs the v2 rename whatever its value says (the
    /// probe refuses a legacy value other than 1 anyway).
    fn is_fresh(&self, file: &Path, text: &str, target: ProjectVersion) -> Result<bool, MigrationError> {
        if text.is_empty() {
            return Ok(false);
        }
        let probed = probe_version_key(text).map_err(|e| fail(file, e.to_string()))?;
        Ok(!probed.legacy && probed.version >= target)
    }

    fn migrate(
        &self,
        file: &Path,
        text: &str,
        target: ProjectVersion,
        _facts: &MigrationFacts,
    ) -> Result<String, MigrationError> {
        migrate_settings(text, target).map_err(|message| fail(file, message))
    }
}

fn fail(file: &Path, message: String) -> MigrationError {
    MigrationError::domain(DomainId::PROJECT, file, message.replace('\n', " "))
}

/// `project.ron`'s text at `target`: the version key (either spelling) replaced by `project_version: <target>`,
/// every other known setting kept, in the canonical text format. A missing file (`""`) becomes the default settings.
fn migrate_settings(text: &str, target: ProjectVersion) -> Result<String, String> {
    if text.is_empty() {
        let settings = ProjectSettings { project_version: target, ..ProjectSettings::default() };
        return settings_text(&settings).map_err(|e| e.to_string());
    }
    // Clear messages for "both keys" / "no key" before the full parse.
    probe_project_version(text).map_err(|e| e.to_string())?;
    let settings: ProjectSettings = ron::Options::default()
        .from_str_seed(text, AtVersion(target.get()))
        .map_err(|e| format!("invalid settings: {e}"))?;
    settings_text(&settings).map_err(|e| e.to_string())
}

/// Reads [`ProjectSettings`] exactly like `Project::open` does, except that the top-level version key is accepted in
/// either spelling and its value is replaced by `.0` (so any target, also past [`ProjectVersion::CURRENT`], works).
struct AtVersion(u32);

impl<'de> de::DeserializeSeed<'de> for AtVersion {
    type Value = ProjectSettings;

    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<ProjectSettings, D::Error> {
        ProjectSettings::deserialize(VersionKey { inner: d, version: self.0 })
    }
}

/// A deserializer whose top-level struct presents `format_version` / `project_version` as `project_version` with
/// the value `version`; everything below is the inner deserializer's.
struct VersionKey<D> {
    inner: D,
    version: u32,
}

impl<'de, D: de::Deserializer<'de>> de::Deserializer<'de> for VersionKey<D> {
    type Error = D::Error;

    fn deserialize_any<V: de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
        self.inner.deserialize_any(VersionVisitor { inner: visitor, version: self.version })
    }

    fn deserialize_struct<V: de::Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.inner.deserialize_struct(name, fields, VersionVisitor { inner: visitor, version: self.version })
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf option unit unit_struct
        newtype_struct seq tuple tuple_struct map enum identifier ignored_any
    }
}

struct VersionVisitor<V> {
    inner: V,
    version: u32,
}

impl<'de, V: de::Visitor<'de>> de::Visitor<'de> for VersionVisitor<V> {
    type Value = V::Value;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.expecting(f)
    }

    fn visit_map<A: de::MapAccess<'de>>(self, map: A) -> Result<V::Value, A::Error> {
        self.inner.visit_map(VersionMap { inner: map, version: self.version, replace_next: false })
    }
}

struct VersionMap<A> {
    inner: A,
    version: u32,
    /// The key just returned was a version key: replace its value.
    replace_next: bool,
}

impl<'de, A: de::MapAccess<'de>> de::MapAccess<'de> for VersionMap<A> {
    type Error = A::Error;

    fn next_key_seed<K: de::DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, A::Error> {
        let Some(key) = self.inner.next_key::<String>()? else {
            return Ok(None);
        };
        self.replace_next = key == KEY || key == LEGACY_KEY;
        let key = if self.replace_next { KEY.to_string() } else { key };
        seed.deserialize(de::value::StringDeserializer::<A::Error>::new(key)).map(Some)
    }

    fn next_value_seed<S: de::DeserializeSeed<'de>>(&mut self, seed: S) -> Result<S::Value, A::Error> {
        if std::mem::take(&mut self.replace_next) {
            self.inner.next_value::<de::IgnoredAny>()?;
            seed.deserialize(de::value::U32Deserializer::<A::Error>::new(self.version))
        } else {
            self.inner.next_value_seed(seed)
        }
    }

    fn size_hint(&self) -> Option<usize> {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn migrate(text: &str, target: u32) -> String {
        let path = Path::new("/game/quillshade/project.ron");
        ProjectMigrator.migrate(path, text, ProjectVersion(target), &MigrationFacts::default()).unwrap()
    }

    fn fresh(text: &str, target: u32) -> bool {
        ProjectMigrator.is_fresh(Path::new("project.ron"), text, ProjectVersion(target)).unwrap()
    }

    #[test]
    fn golden_minimal_v1_to_v2() {
        let v1 = "ProjectSettings(\n    format_version: 1,\n)";
        assert!(!fresh(v1, 2));
        let v2 = migrate(v1, 2);
        assert_eq!(v2, "ProjectSettings(\n  project_version: 2,\n  header_date: false,\n)\n");
        assert!(fresh(&v2, 2));
        assert!(!fresh(&v2, 3));
    }

    #[test]
    fn golden_full_v1_to_v2() {
        let v1 = "ProjectSettings(\n    format_version: 1,\n    header_date: true,\n    domains: {\n        \
                  \"material\": (),\n        \"light\": (enabled: false),\n    },\n)\n";
        let v2 = migrate(v1, 2);
        let expected = "ProjectSettings(\n  project_version: 2,\n  header_date: true,\n  domains: {\n    \
                        \"light\": DomainEntry(\n      enabled: false,\n    ),\n    \"material\": DomainEntry(\n      \
                        enabled: true,\n    ),\n  },\n)\n";
        assert_eq!(v2, expected);
        assert!(fresh(&v2, 2));
        let settings: ProjectSettings = ron::from_str(&v2).unwrap();
        assert!(settings.header_date && !settings.domains["light"].enabled);
    }

    #[test]
    fn parses_like_project_open_and_is_idempotent() {
        // Unnamed struct and the canonical name both work; another name is refused like `Project::open` does.
        assert_eq!(
            migrate("(format_version: 1, header_date: true)", 2),
            migrate("ProjectSettings(format_version: 1, header_date: true)", 2)
        );
        let path = Path::new("project.ron");
        let other =
            ProjectMigrator.migrate(path, "Whatever(format_version: 1)", ProjectVersion(2), &Default::default());
        assert!(other.is_err());
        // Unknown keys are dropped (ignored on open as well); only the top-level key is the version.
        let v2 = migrate("(format_version: 1, some_future_option: 3, domains: {\"format_version\": ()})", 2);
        assert!(!v2.contains("some_future_option") && v2.contains("\"format_version\": DomainEntry("), "{v2}");
        let v2 = migrate("(format_version: 1)", 2);
        assert_eq!(migrate(&v2, 2), v2, "re-migrating a canonical file at the same version is a no-op");
    }

    #[test]
    fn a_missing_file_is_stale_and_created_from_defaults() {
        assert!(!fresh("", 2) && !fresh("", 3));
        assert_eq!(migrate("", 2), "ProjectSettings(\n  project_version: 2,\n  header_date: false,\n)\n");
        assert_eq!(migrate("", 3), "ProjectSettings(\n  project_version: 3,\n  header_date: false,\n)\n");
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        let project = RawProject::open(dir.path()).unwrap();
        assert_eq!(ProjectMigrator.files(&project), [project.settings_path()], "listed while missing");
    }

    #[test]
    fn later_steps_are_pure_bumps() {
        let v2 = migrate("ProjectSettings(format_version: 1, header_date: true)", 2);
        let v3 = migrate(&v2, 3);
        assert_eq!(v3, v2.replace("project_version: 2", "project_version: 3"));
        assert!(fresh(&v3, 3) && !fresh(&v3, 4));
        let v4 = migrate(&v3, 4);
        assert_eq!(v4, v2.replace("project_version: 2", "project_version: 4"));
    }

    #[test]
    fn errors_name_the_file_and_are_single_line() {
        let path = Path::new("/game/quillshade/project.ron");
        let facts = MigrationFacts::default();
        for bad in ["(format_version: 1, project_version: 2)", "(header_date: true)", "garbage (", "[1, 2]"] {
            let err = ProjectMigrator.migrate(path, bad, ProjectVersion(2), &facts).unwrap_err();
            assert!(matches!(&err, MigrationError::Domain { domain, .. } if *domain == DomainId::PROJECT), "{err}");
            let text = err.to_string();
            assert!(text.contains("project.ron") && !text.contains('\n'), "{text}");
        }
        let err = ProjectMigrator.is_fresh(path, "(header_date: true)", ProjectVersion(2)).unwrap_err();
        assert!(err.to_string().contains("project_version"), "{err}");
    }

    /// `format_version: N` with N != 1 is no version (only v1 is spelled that way): never fresh, never "up to date",
    /// an error naming the key instead; the same for version 0.
    #[test]
    fn the_legacy_key_is_never_fresh() {
        let path = Path::new("/game/quillshade/project.ron");
        assert!(!fresh("(format_version: 1)", 1), "the legacy spelling is stale even at its own version");
        for bad in ["(format_version: 2)", "(format_version: 3)", "(project_version: 0)"] {
            let err = ProjectMigrator.is_fresh(path, bad, ProjectVersion(2)).unwrap_err();
            let text = err.to_string();
            assert!(text.contains("project.ron") && text.contains("is not a project version"), "{text}");
            let err = ProjectMigrator.migrate(path, bad, ProjectVersion(2), &MigrationFacts::default()).unwrap_err();
            assert!(err.to_string().contains("is not a project version"), "{err}");
        }
    }
}
