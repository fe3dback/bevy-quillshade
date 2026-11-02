//! The global project version (D28, techdoc §4.4, ADR 0046 "Project version (D28)"): the single number in
//! `quillshade/project.ron` that says which format generation the whole project is in.
//!
//! [`ProjectVersion::CURRENT`] is the version this build writes and opens; every later format change adds one
//! migration step and raises it by one. Version 1 is, by definition, the only version spelled `format_version` in
//! `project.ron`; from v2 on the key is `project_version`. Per-file `format_version`s of the asset formats are a
//! different thing (idempotency markers owned by each domain) and are not handled here.
//!
//! A project without `project.ron` has a version too ([`missing_settings_version`], ADR 0046 "Missing
//! `project.ron`"): a new one is at [`ProjectVersion::CURRENT`], one that already holds sources is v1.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The project version stored in `quillshade/project.ron` (serialized as a bare number).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectVersion(pub u32);

impl ProjectVersion {
    /// The first project version (the one spelled `format_version` in `project.ron`).
    pub const FIRST: ProjectVersion = ProjectVersion(1);
    /// The version this build writes and opens.
    pub const CURRENT: ProjectVersion = ProjectVersion(3);

    /// The raw number.
    pub const fn get(self) -> u32 {
        self.0
    }

    /// The following version (the target of the next migration step).
    pub const fn next(self) -> ProjectVersion {
        ProjectVersion(self.0 + 1)
    }
}

impl std::fmt::Display for ProjectVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// Why [`probe_project_version`] could not read a version. Messages are single-line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VersionProbeError {
    #[error("parse error: {0}")]
    Parse(String),
    #[error("no `project_version` field")]
    Missing,
    #[error("both `project_version` and the legacy `format_version` are set; keep only `project_version`")]
    Both,
    /// The legacy key with a value other than 1: only v1 is spelled `format_version` (ADR 0046).
    #[error(
        "`format_version: {0}` is not a project version: only v1 is spelled `format_version`; from v2 on the key is \
         `project_version`"
    )]
    Legacy(u32),
    /// Version 0: project versions start at 1.
    #[error("`project_version: 0` is not a project version: versions start at 1")]
    Zero,
}

/// What [`probe_version_key`] read: the version and how its key is spelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbedVersion {
    pub version: ProjectVersion,
    /// Spelled with the legacy key `format_version` (then `version` is [`ProjectVersion::FIRST`]).
    pub legacy: bool,
}

/// The legacy (v1) key of the project version.
const LEGACY_KEY: &str = "format_version";
/// The key of the project version from v2 on.
const KEY: &str = "project_version";

/// Struct-name-agnostic probe: a map visitor that keeps both version keys and skips every other field.
struct Probe {
    current: Option<u32>,
    legacy: Option<u32>,
}

impl<'de> Deserialize<'de> for Probe {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Probe;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a struct with `project_version`")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Probe, A::Error> {
                let mut probe = Probe { current: None, legacy: None };
                while let Some(key) = map.next_key::<String>()? {
                    let slot = match key.as_str() {
                        KEY => &mut probe.current,
                        LEGACY_KEY => &mut probe.legacy,
                        _ => {
                            map.next_value::<serde::de::IgnoredAny>()?;
                            continue;
                        }
                    };
                    if slot.is_some() {
                        return Err(serde::de::Error::duplicate_field(if key == KEY { KEY } else { LEGACY_KEY }));
                    }
                    *slot = Some(map.next_value::<u32>()?);
                }
                Ok(probe)
            }
        }
        d.deserialize_any(V)
    }
}

/// Reads only the project version of a `project.ron` text, whatever the struct name: `project_version` if present,
/// else the legacy `format_version`. Both present or neither present is an error, and so are the values no project
/// can have: `format_version` other than 1 (only v1 is spelled that way, ADR 0046) and version 0. Any other number is
/// accepted; the caller compares it with [`ProjectVersion::CURRENT`].
pub fn probe_project_version(text: &str) -> Result<ProjectVersion, VersionProbeError> {
    probe_version_key(text).map(|p| p.version)
}

/// [`probe_project_version`] that also tells whether the version is spelled with the legacy key.
pub fn probe_version_key(text: &str) -> Result<ProbedVersion, VersionProbeError> {
    let probe: Probe = ron::Options::default()
        .from_str(text)
        .map_err(|e| VersionProbeError::Parse(e.to_string().replace('\n', " ")))?;
    match (probe.current, probe.legacy) {
        (Some(0), None) => Err(VersionProbeError::Zero),
        (Some(v), None) => Ok(ProbedVersion { version: ProjectVersion(v), legacy: false }),
        (None, Some(1)) => Ok(ProbedVersion { version: ProjectVersion::FIRST, legacy: true }),
        (None, Some(v)) => Err(VersionProbeError::Legacy(v)),
        (Some(_), Some(_)) => Err(VersionProbeError::Both),
        (None, None) => Err(VersionProbeError::Missing),
    }
}

/// Whether `quillshade_dir` holds a project made before `project.ron` was required: any non-hidden entry (a name
/// not starting with `.`) other than `project.ron`. A missing or unreadable directory holds nothing.
pub fn is_legacy_layout(quillshade_dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(quillshade_dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        let name = e.file_name();
        let name = name.to_string_lossy();
        !name.starts_with('.') && name != crate::layout::SETTINGS_FILE
    })
}

/// The project version of a project whose `quillshade_dir` has no `project.ron` (ADR 0046 "Missing `project.ron`"):
/// [`ProjectVersion::FIRST`] for a legacy layout ([`is_legacy_layout`]: every project with sources made before the
/// file was required), else [`ProjectVersion::CURRENT`] (a new project; the editor writes the file on open). The
/// single rule behind `Project::open`, `Project::reload_settings` and `project_migrate::RawProject::open`.
pub fn missing_settings_version(quillshade_dir: &Path) -> ProjectVersion {
    if is_legacy_layout(quillshade_dir) { ProjectVersion::FIRST } else { ProjectVersion::CURRENT }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_order() {
        assert_eq!(ProjectVersion::CURRENT.to_string(), "v3");
        assert_eq!(ProjectVersion::FIRST.next().next(), ProjectVersion::CURRENT);
        assert!(ProjectVersion::FIRST < ProjectVersion::CURRENT);
        assert_eq!(ProjectVersion(7).get(), 7);
    }

    #[test]
    fn serializes_as_a_bare_number() {
        assert_eq!(ron::to_string(&ProjectVersion(2)).unwrap(), "2");
        assert_eq!(ron::from_str::<ProjectVersion>("3").unwrap(), ProjectVersion(3));
    }

    #[test]
    fn probes_the_new_key_of_any_struct_name() {
        let probe = probe_project_version;
        assert_eq!(probe("ProjectSettings(project_version: 2, header_date: true)"), Ok(ProjectVersion(2)));
        assert_eq!(probe("(header_date: true, project_version: 5)"), Ok(ProjectVersion(5)));
        assert_eq!(probe("Whatever(project_version: 1)"), Ok(ProjectVersion(1)));
        let key = probe_version_key("(project_version: 3)").unwrap();
        assert_eq!(key, ProbedVersion { version: ProjectVersion(3), legacy: false });
    }

    #[test]
    fn version_zero_is_an_error() {
        let zero = probe_project_version("ProjectSettings(project_version: 0)").unwrap_err();
        assert_eq!(zero, VersionProbeError::Zero);
        assert!(zero.to_string().contains("versions start at 1"), "{zero}");
    }

    /// Only v1 is spelled `format_version` (ADR 0046): any other value is no version at all, not "v2 already".
    #[test]
    fn the_legacy_key_is_only_v1() {
        for v in [0, 2, 3, 7] {
            let err = probe_project_version(&format!("ProjectSettings(format_version: {v})")).unwrap_err();
            assert_eq!(err, VersionProbeError::Legacy(v));
            let text = err.to_string();
            assert!(text.contains(&format!("`format_version: {v}`")) && text.contains("project_version"), "{text}");
            assert!(!text.contains('\n'), "{text}");
        }
        let key = probe_version_key("(format_version: 1)").unwrap();
        assert_eq!(key, ProbedVersion { version: ProjectVersion::FIRST, legacy: true });
    }

    #[test]
    fn probes_the_legacy_key() {
        let text = "ProjectSettings(format_version: 1, domains: {\"light\": (enabled: false)})";
        assert_eq!(probe_project_version(text), Ok(ProjectVersion(1)));
        assert_eq!(probe_project_version("(format_version: 1)"), Ok(ProjectVersion(1)));
    }

    #[test]
    fn both_neither_or_garbage_are_errors() {
        let both = "ProjectSettings(format_version: 1, project_version: 2)";
        assert_eq!(probe_project_version(both), Err(VersionProbeError::Both));
        let neither = probe_project_version("ProjectSettings(header_date: true)").unwrap_err();
        assert_eq!(neither, VersionProbeError::Missing);
        assert!(neither.to_string().contains("project_version"), "{neither}");
        assert!(matches!(probe_project_version("garbage ("), Err(VersionProbeError::Parse(_))));
        assert!(matches!(probe_project_version("(project_version: \"2\")"), Err(VersionProbeError::Parse(_))));
        let dup = probe_project_version("(project_version: 2, project_version: 3)");
        assert!(matches!(dup, Err(VersionProbeError::Parse(_))), "{dup:?}");
    }

    #[test]
    fn a_missing_project_ron_is_v1_only_beside_sources() {
        let dir = tempfile::tempdir().unwrap();
        let q = dir.path().join("quillshade");
        // Missing, empty, or holding only hidden entries / `project.ron` itself: a new project.
        assert_eq!(missing_settings_version(&q), ProjectVersion::CURRENT);
        std::fs::create_dir(&q).unwrap();
        assert_eq!(missing_settings_version(&q), ProjectVersion::CURRENT);
        std::fs::create_dir(q.join(".cache")).unwrap();
        std::fs::write(q.join(".gitkeep"), "").unwrap();
        std::fs::write(q.join("project.ron"), "").unwrap();
        assert!(!is_legacy_layout(&q));
        // Any other entry, a file or a (possibly empty) directory: a project made before `project.ron` was required.
        std::fs::create_dir(q.join("material")).unwrap();
        assert!(is_legacy_layout(&q));
        assert_eq!(missing_settings_version(&q), ProjectVersion::FIRST);
        std::fs::remove_dir(q.join("material")).unwrap();
        std::fs::write(q.join("notes.txt"), "").unwrap();
        assert_eq!(missing_settings_version(&q), ProjectVersion::FIRST);
    }
}
