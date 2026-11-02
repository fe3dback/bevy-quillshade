//! Quillshade project layout (design §4, §4.1): `quillshade/project.ron`, per-domain source / output / module
//! directories, and the generated umbrella crate location.
//!
//! Opening checks the project version first (D28, techdoc §4.4, ADR 0046 "Project version (D28)"): only a project
//! at [`ProjectVersion::CURRENT`] opens; an older one asks for migration, a newer one is refused. A project without
//! `project.ron` is new (CURRENT) only while `quillshade/` holds nothing else; beside sources it is v1
//! ([`crate::version::missing_settings_version`], ADR 0046 "Missing `project.ron`").

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::DomainId;
use crate::version::{ProjectVersion, missing_settings_version, probe_project_version};

/// Source root inside the game project (`quillshade/<domain>/…`).
pub const QUILLSHADE_DIR: &str = "quillshade";
/// Settings file inside [`QUILLSHADE_DIR`].
pub const SETTINGS_FILE: &str = "project.ron";
/// Generated asset output root (`assets/quillshade/<domain>/…`).
pub const OUTPUT_DIR: &str = "assets/quillshade";
/// Generated umbrella crate location.
pub const CRATE_DIR: &str = "crates/quillshade";
/// Name of the generated umbrella crate.
pub const CRATE_NAME: &str = "quillshade";

fn yes() -> bool {
    true
}

/// Per-domain entry of `project.ron`. Domain settings sections come later (design §4.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainEntry {
    #[serde(default = "yes")]
    pub enabled: bool,
}

/// Project-level options stored in `quillshade/project.ron`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSettings {
    /// The global project version (D28, ADR 0046); v1 spelled this key `format_version`.
    pub project_version: ProjectVersion,
    /// Add the generation date to autogen headers (breaks byte-for-byte determinism across days).
    #[serde(default)]
    pub header_date: bool,
    /// Domain entries by id; unknown ids are kept, a missing id means "enabled".
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub domains: BTreeMap<String, DomainEntry>,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        ProjectSettings { project_version: ProjectVersion::CURRENT, header_date: false, domains: BTreeMap::new() }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("`{0}` is not a directory")]
    NotADirectory(PathBuf),
    #[error("`{0}` is not a project: Cargo.toml is missing")]
    NoCargoToml(PathBuf),
    #[error("`{0}` is not a project: assets/ directory is missing")]
    NoAssets(PathBuf),
    #[error("cannot set up `{0}`: {1}")]
    Io(PathBuf, std::io::Error),
    /// `quillshade/project.ron` exists but cannot be read or parsed: falling back to defaults would silently
    /// re-enable disabled domains and drop options (design §4.1).
    #[error("cannot read project settings `{}`: {message}", path.display())]
    Settings { path: PathBuf, message: String },
    /// The project is at an older project version than this build (`to` = [`ProjectVersion::CURRENT`]): the
    /// Migration Tool upgrades it (techdoc §4.4).
    #[error("project version {from} is older than this editor ({to}); migrate it first")]
    NeedsMigration { from: ProjectVersion, to: ProjectVersion },
    /// The project was written by a newer build (`supported` = [`ProjectVersion::CURRENT`]): refuse to open.
    #[error("project version {found} is newer than this editor supports ({supported}); update the editor")]
    TooNew { found: ProjectVersion, supported: ProjectVersion },
}

/// How the generated `Cargo.toml` depends on Bevy (decision 0019).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BevyDep {
    /// `bevy = { version = "0.20", default-features = false, features = [..] }`.
    #[default]
    Version,
    /// `bevy = { workspace = true, features = [..] }`: the game declares Bevy in `[workspace.dependencies]`.
    Workspace,
}

/// Opened game project.
#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub settings: ProjectSettings,
}

impl Project {
    /// Opens `root`, which must contain `Cargo.toml` and `assets/`.
    pub fn open(root: impl AsRef<Path>) -> Result<Project, OpenError> {
        let root = root.as_ref();
        let root = root.canonicalize().map_or_else(|_| root.to_path_buf(), without_verbatim_prefix);
        if !root.is_dir() {
            return Err(OpenError::NotADirectory(root));
        }
        if !root.join("Cargo.toml").is_file() {
            return Err(OpenError::NoCargoToml(root));
        }
        if !root.join("assets").is_dir() {
            return Err(OpenError::NoAssets(root));
        }
        let settings = read_settings(&root.join(QUILLSHADE_DIR).join(SETTINGS_FILE))?;
        Ok(Project { root, settings })
    }

    /// Re-reads `project.ron` after an external change. On error the current settings are kept.
    pub fn reload_settings(&mut self) -> Result<(), OpenError> {
        self.settings = read_settings(&self.settings_path())?;
        Ok(())
    }

    /// Sets up a game crate as a project ("New project" in the editor): `root` must contain `Cargo.toml`;
    /// `assets/` and `quillshade/` (with default settings) are created if missing. Opens first, so a project that is
    /// not at [`ProjectVersion::CURRENT`] (also a legacy one without `project.ron`) is refused and never stamped.
    pub fn create(root: impl AsRef<Path>) -> Result<Project, OpenError> {
        let root = root.as_ref();
        let io = |e| OpenError::Io(root.to_path_buf(), e);
        if root.join("Cargo.toml").is_file() {
            std::fs::create_dir_all(root.join("assets")).map_err(io)?;
        }
        let project = Project::open(root)?;
        project.ensure_layout().map_err(io)?;
        Ok(project)
    }

    pub fn quillshade_dir(&self) -> PathBuf {
        self.root.join(QUILLSHADE_DIR)
    }

    /// Source root of a domain (`quillshade/<id>`).
    pub fn domain_dir(&self, domain: DomainId) -> PathBuf {
        self.quillshade_dir().join(domain.as_str())
    }

    /// Generated asset output root of a domain (`assets/quillshade/<id>`).
    pub fn output_dir(&self, domain: DomainId) -> PathBuf {
        self.root.join(OUTPUT_DIR).join(domain.as_str())
    }

    /// The game's `assets/` directory.
    pub fn assets_dir(&self) -> PathBuf {
        self.root.join("assets")
    }

    pub fn crate_dir(&self) -> PathBuf {
        self.root.join(CRATE_DIR)
    }

    /// Generated module of a domain inside the umbrella crate (`crates/quillshade/src/<id>`).
    pub fn module_dir(&self, domain: DomainId) -> PathBuf {
        self.crate_dir().join("src").join(domain.as_str())
    }

    pub fn settings_path(&self) -> PathBuf {
        self.quillshade_dir().join(SETTINGS_FILE)
    }

    /// Project-relative display path.
    pub fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.root).unwrap_or(p).to_string_lossy().replace('\\', "/")
    }

    /// Creates `quillshade/` and `project.ron` (from [`Project::settings`]) if missing. The settings are at
    /// [`ProjectVersion::CURRENT`]: [`Project::open`] refuses every other project, including a legacy layout without
    /// `project.ron` (ADR 0046), so this never stamps an old project current.
    pub fn ensure_layout(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.quillshade_dir())?;
        let path = self.settings_path();
        if !path.exists() {
            crate::writer::write_atomic(&path, settings_text(&self.settings)?)?;
        }
        Ok(())
    }

    /// Whether a domain is used in the game; a missing entry means `true`.
    pub fn domain_enabled(&self, domain: DomainId) -> bool {
        self.settings.domains.get(domain.as_str()).is_none_or(|e| e.enabled)
    }

    /// How the generated crate depends on Bevy (decision 0019): through the workspace when the root `Cargo.toml`
    /// declares `[workspace.dependencies] bevy`, or itself takes `bevy = { workspace = true }` (the game is a member
    /// of an outer workspace).
    pub fn bevy_dep(&self) -> BevyDep {
        let Some(doc) = self.root_manifest() else {
            return BevyDep::Version;
        };
        let declared = doc.get("workspace").and_then(|w| w.get("dependencies")).and_then(|d| d.get("bevy")).is_some();
        let inherited = doc
            .get("dependencies")
            .and_then(|d| d.get("bevy"))
            .and_then(|b| b.get("workspace"))
            .and_then(|w| w.as_bool())
            .unwrap_or(false);
        if declared || inherited { BevyDep::Workspace } else { BevyDep::Version }
    }

    /// W105 text: `crates/quillshade` missing from `[workspace.members]` of the root `Cargo.toml`. The caller wraps
    /// it into its own diagnostic.
    pub fn workspace_problem(&self) -> Option<String> {
        let doc = self.root_manifest()?;
        let members: Vec<String> = doc
            .get("workspace")
            .and_then(|w| w.get("members"))
            .and_then(|m| m.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        if members.iter().any(|m| glob_match(m.trim_end_matches('/'), CRATE_DIR)) {
            return None;
        }
        let hint = if doc.contains_key("workspace") {
            format!("add \"{CRATE_DIR}\" to [workspace] members, e.g. members = [\"{CRATE_DIR}\"]")
        } else {
            format!(
                "add to the root Cargo.toml:\n[workspace]\nmembers = [\"{CRATE_DIR}\"]\n\nand to [dependencies]: {CRATE_NAME} = {{ path = \"{CRATE_DIR}\" }}"
            )
        };
        Some(format!("{CRATE_DIR} is not in [workspace.members]; {hint}"))
    }

    /// The game's root `Cargo.toml`, parsed (`None` when missing or not TOML).
    fn root_manifest(&self) -> Option<toml::Table> {
        std::fs::read_to_string(self.root.join("Cargo.toml")).ok()?.parse().ok()
    }
}

/// Reads `project.ron` at `path` (`quillshade/project.ron`): an unreadable or malformed one, or one without a
/// version key, is [`OpenError::Settings`]. The version is probed before the full parse (ADR 0046): an older project
/// is [`OpenError::NeedsMigration`], a newer one [`OpenError::TooNew`]. A missing file takes its version from what
/// `quillshade/` holds ([`missing_settings_version`]): defaults for a new project, `NeedsMigration` from v1 beside
/// sources.
pub fn read_settings(path: &Path) -> Result<ProjectSettings, OpenError> {
    let err = |message: String| OpenError::Settings { path: path.to_path_buf(), message };
    let (version, text) = match std::fs::read_to_string(path) {
        Ok(text) => (probe_project_version(&text).map_err(|e| err(e.to_string()))?, Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            (missing_settings_version(path.parent().unwrap_or(Path::new(""))), None)
        }
        Err(e) => return Err(err(e.to_string())),
    };
    match version.cmp(&ProjectVersion::CURRENT) {
        std::cmp::Ordering::Less => Err(OpenError::NeedsMigration { from: version, to: ProjectVersion::CURRENT }),
        std::cmp::Ordering::Greater => Err(OpenError::TooNew { found: version, supported: ProjectVersion::CURRENT }),
        std::cmp::Ordering::Equal => match text {
            Some(text) => ron::from_str(&text).map_err(|e| err(e.to_string())),
            None => Ok(ProjectSettings::default()),
        },
    }
}

/// Writes `settings` to the project's `quillshade/project.ron` atomically (temp file, then a rename: an editor or
/// watcher never reads half a file), in the style [`Project::ensure_layout`] writes. `project` is not changed: the
/// caller adopts `settings` as [`Project::settings`] once this succeeded (the Project Settings tool, design §14.2).
pub fn save_settings(project: &Project, settings: &ProjectSettings) -> std::io::Result<()> {
    crate::writer::write_atomic(&project.settings_path(), settings_text(settings)?)
}

/// `project.ron`'s canonical text: struct names, 2-space indent, `\n` line ends, a final newline. Shared with the
/// project migrator (`project_migrate`, ADR 0046), which rewrites `project.ron` in exactly this format.
pub fn settings_text(settings: &ProjectSettings) -> std::io::Result<String> {
    let cfg = ron::ser::PrettyConfig::new().struct_names(true).indentor("  ".to_string()).new_line("\n".to_string());
    Ok(ron::ser::to_string_pretty(settings, cfg).map_err(std::io::Error::other)? + "\n")
}

/// Cargo-style glob: `**` matches zero or more path segments; `*` matches zero or more characters within a segment
/// (multiple `*` per segment supported).
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let p: Vec<&str> = pattern.split('/').collect();
    let s: Vec<&str> = path.split('/').collect();
    match_segments(&p, &s)
}

fn match_segments(p: &[&str], s: &[&str]) -> bool {
    let Some((&first, rest_p)) = p.split_first() else {
        return s.is_empty();
    };
    if first == "**" {
        match_segments(rest_p, s) || (!s.is_empty() && match_segments(p, &s[1..]))
    } else {
        let Some((&seg, rest_s)) = s.split_first() else {
            return false;
        };
        segment_match(first, seg) && match_segments(rest_p, rest_s)
    }
}

fn segment_match(pat: &str, seg: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == seg;
    }
    let Some(mut rest) = seg.strip_prefix(parts[0]) else {
        return false;
    };
    let Some(middle) = rest.strip_suffix(parts[parts.len() - 1]) else {
        return false;
    };
    rest = middle;
    for part in &parts[1..parts.len() - 1] {
        if part.is_empty() {
            continue;
        }
        let Some(pos) = rest.find(part) else {
            return false;
        };
        rest = &rest[pos + part.len()..];
    }
    true
}

/// Drops the `\\?\` prefix Windows `canonicalize` adds (`\\?\C:\…`, `\\?\UNC\server\…`): a verbatim path takes `/`
/// literally, while layout constants and asset paths joined onto the root use `/` separators. Unlike `dunce`, it strips
/// the prefix even where the plain form means another path (reserved device names, components ending in `.` or a
/// space): acceptable for a project directory.
pub fn without_verbatim_prefix(p: PathBuf) -> PathBuf {
    let Some(s) = p.to_str() else { return p };
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match s.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') && rest.as_bytes()[0].is_ascii_alphabetic() => {
            PathBuf::from(rest)
        }
        _ => p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(manifest: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), manifest).unwrap();
        dir
    }

    #[test]
    fn open_reads_project_ron() {
        let dir = game("[package]\nname = \"g\"\n");
        std::fs::create_dir(dir.path().join("quillshade")).unwrap();
        std::fs::write(
            dir.path().join("quillshade/project.ron"),
            "(project_version: 3, header_date: true, domains: {\"light\": (enabled: false), \"nope\": (enabled: false)})",
        )
        .unwrap();
        let p = Project::open(dir.path()).unwrap();
        assert_eq!(p.settings.project_version, ProjectVersion::CURRENT);
        assert!(p.settings.header_date);
        assert!(!p.domain_enabled(DomainId::new("light")));
        assert!(p.domain_enabled(DomainId::new("material")));
    }

    #[test]
    fn open_without_project_ron_uses_defaults() {
        let dir = game("[package]\nname = \"g\"\n");
        let p = Project::open(dir.path()).unwrap();
        assert_eq!(p.settings, ProjectSettings::default());
        assert_eq!(p.settings.project_version, ProjectVersion::CURRENT);
        assert!(!p.settings.header_date);
    }

    /// A game with `quillshade/project.ron` holding `text`.
    fn game_with_settings(text: &str) -> tempfile::TempDir {
        let dir = game("[package]\nname = \"g\"\n");
        std::fs::create_dir(dir.path().join("quillshade")).unwrap();
        std::fs::write(dir.path().join("quillshade/project.ron"), text).unwrap();
        dir
    }

    #[test]
    fn old_project_needs_migration() {
        // v1 spells the key `format_version`.
        let dir = game_with_settings("ProjectSettings(format_version: 1, header_date: true)\n");
        let err = Project::open(dir.path()).unwrap_err();
        assert!(
            matches!(err, OpenError::NeedsMigration { from: ProjectVersion(1), to: ProjectVersion::CURRENT }),
            "{err}"
        );
        let text = err.to_string();
        assert_eq!(text, "project version v1 is older than this editor (v3); migrate it first");
        // The new key with an old number is old too; "New project" refuses as well and leaves the file alone.
        let dir = game_with_settings("ProjectSettings(project_version: 1)\n");
        let err = Project::open(dir.path()).unwrap_err();
        assert!(matches!(err, OpenError::NeedsMigration { from: ProjectVersion(1), .. }), "{err}");
        assert!(matches!(Project::create(dir.path()), Err(OpenError::NeedsMigration { .. })));
        let kept = std::fs::read_to_string(dir.path().join("quillshade/project.ron")).unwrap();
        assert_eq!(kept, "ProjectSettings(project_version: 1)\n");
    }

    /// ADR 0046 "Missing `project.ron`": sources without `project.ron` are a v1 project; opening, "New project" and
    /// the watcher refuse it and write nothing (the project migrator creates the file).
    #[test]
    fn legacy_layout_without_project_ron_needs_migration() {
        let dir = game("[package]\nname = \"g\"\n");
        let source = dir.path().join("quillshade/material/rock.material.ron");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "()").unwrap();
        let v1 = |r: Result<_, OpenError>| {
            matches!(r, Err(OpenError::NeedsMigration { from: ProjectVersion::FIRST, to: ProjectVersion::CURRENT }))
        };
        assert!(v1(Project::open(dir.path()).map(|_| ())));
        assert!(v1(Project::create(dir.path()).map(|_| ())));
        assert!(v1(read_settings(&dir.path().join("quillshade/project.ron")).map(|_| ())));
        assert!(!dir.path().join("quillshade/project.ron").exists(), "never stamped current");
        // Deleting `project.ron` of an open project with sources: the watcher keeps the old settings.
        let dir = game("[package]\nname = \"g\"\n");
        let mut p = Project::create(dir.path()).unwrap();
        std::fs::write(p.settings_path(), "(project_version: 3, header_date: true)").unwrap();
        p.reload_settings().unwrap();
        std::fs::create_dir(p.domain_dir(DomainId::new("light"))).unwrap();
        std::fs::remove_file(p.settings_path()).unwrap();
        assert!(v1(p.reload_settings()));
        assert!(p.settings.header_date);
    }

    #[test]
    fn empty_or_missing_quillshade_is_a_new_project() {
        // No `quillshade/`, an empty one, or one with only hidden entries: a new project at CURRENT.
        let dir = game("[package]\nname = \"g\"\n");
        assert_eq!(Project::open(dir.path()).unwrap().settings, ProjectSettings::default());
        let q = dir.path().join("quillshade");
        std::fs::create_dir(&q).unwrap();
        assert_eq!(Project::open(dir.path()).unwrap().settings, ProjectSettings::default());
        std::fs::write(q.join(".gitignore"), "").unwrap();
        std::fs::create_dir(q.join(".nodes")).unwrap();
        assert_eq!(Project::open(dir.path()).unwrap().settings, ProjectSettings::default());
        // "New project" still writes CURRENT.
        let p = Project::create(dir.path()).unwrap();
        let text = std::fs::read_to_string(p.settings_path()).unwrap();
        assert_eq!(text, "ProjectSettings(\n  project_version: 3,\n  header_date: false,\n)\n");
        // Once written, sources beside it are an ordinary current project.
        std::fs::create_dir(p.domain_dir(DomainId::new("material"))).unwrap();
        assert_eq!(Project::open(dir.path()).unwrap().settings.project_version, ProjectVersion::CURRENT);
    }

    #[test]
    fn newer_project_is_refused() {
        let dir = game_with_settings("ProjectSettings(project_version: 4, some_future_option: true)\n");
        let err = Project::open(dir.path()).unwrap_err();
        assert!(
            matches!(err, OpenError::TooNew { found: ProjectVersion(4), supported: ProjectVersion::CURRENT }),
            "{err}"
        );
        assert_eq!(err.to_string(), "project version v4 is newer than this editor supports (v3); update the editor");
    }

    #[test]
    fn both_or_no_version_key_is_a_settings_error() {
        let dir = game_with_settings("ProjectSettings(format_version: 1, project_version: 2)\n");
        let err = Project::open(dir.path()).unwrap_err();
        assert!(matches!(&err, OpenError::Settings { .. }), "{err}");
        assert!(err.to_string().contains("format_version") && !err.to_string().contains('\n'), "{err}");
        let dir = game_with_settings("ProjectSettings(header_date: true)\n");
        let err = Project::open(dir.path()).unwrap_err();
        assert!(matches!(&err, OpenError::Settings { .. }), "{err}");
        assert!(err.to_string().contains("project_version"), "{err}");
    }

    /// ADR 0046: only v1 is spelled `format_version`, and versions start at 1. `format_version: 2` or
    /// `project_version: 0` is a settings error with a clear message, never an open (or a migration) as some version.
    #[test]
    fn impossible_versions_are_a_settings_error() {
        for (text, needle) in [
            ("ProjectSettings(format_version: 2)\n", "`format_version: 2` is not a project version"),
            ("ProjectSettings(format_version: 3, header_date: true)\n", "only v1 is spelled `format_version`"),
            ("ProjectSettings(project_version: 0)\n", "versions start at 1"),
        ] {
            let dir = game_with_settings(text);
            let err = Project::open(dir.path()).unwrap_err();
            assert!(matches!(&err, OpenError::Settings { .. }), "{text}: {err}");
            assert!(err.to_string().contains(needle) && !err.to_string().contains('\n'), "{text}: {err}");
        }
    }

    #[test]
    fn malformed_project_ron_is_an_open_error() {
        let dir = game("[package]\nname = \"g\"\n");
        std::fs::create_dir(dir.path().join("quillshade")).unwrap();
        let path = dir.path().join("quillshade/project.ron");
        std::fs::write(&path, "(project_version: 3, header_date: tru)").unwrap();
        let err = Project::open(dir.path()).unwrap_err();
        assert!(matches!(&err, OpenError::Settings { path, .. } if path.ends_with("quillshade/project.ron")), "{err}");
        let text = err.to_string();
        assert!(text.contains("project.ron") && !text.contains('\n'), "{text}");
        // "New project" on such a directory fails too and leaves the file alone.
        assert!(matches!(Project::create(dir.path()), Err(OpenError::Settings { .. })));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(project_version: 3, header_date: tru)");
        // A document the probe cannot read at all is a settings error as well.
        std::fs::write(&path, "(project_version: 3, header_date: ").unwrap();
        assert!(matches!(Project::open(dir.path()), Err(OpenError::Settings { .. })));
    }

    #[test]
    fn reload_settings_keeps_the_old_ones_on_error() {
        let dir = game("[package]\nname = \"g\"\n");
        let mut p = Project::create(dir.path()).unwrap();
        std::fs::write(p.settings_path(), "(project_version: 3, header_date: true)").unwrap();
        p.reload_settings().unwrap();
        assert!(p.settings.header_date);
        std::fs::write(p.settings_path(), "(project_version: 3, header_date: ").unwrap();
        assert!(matches!(p.reload_settings(), Err(OpenError::Settings { .. })));
        assert!(p.settings.header_date);
        std::fs::write(p.settings_path(), "(format_version: 1, header_date: false)").unwrap();
        assert!(matches!(p.reload_settings(), Err(OpenError::NeedsMigration { from: ProjectVersion(1), .. })));
        assert!(p.settings.header_date);
        std::fs::write(p.settings_path(), "(project_version: 4)").unwrap();
        assert!(matches!(p.reload_settings(), Err(OpenError::TooNew { found: ProjectVersion(4), .. })));
        assert!(p.settings.header_date);
        std::fs::remove_file(p.settings_path()).unwrap();
        p.reload_settings().unwrap();
        assert_eq!(p.settings, ProjectSettings::default());
    }

    #[test]
    fn paths() {
        let dir = game("[package]\nname = \"g\"\n");
        let p = Project::open(dir.path()).unwrap();
        let m = DomainId::new("material");
        assert!(p.domain_dir(m).ends_with("quillshade/material"));
        assert!(p.output_dir(m).ends_with("assets/quillshade/material"));
        assert!(p.module_dir(m).ends_with("crates/quillshade/src/material"));
        assert!(p.crate_dir().ends_with("crates/quillshade"));
        assert!(p.settings_path().ends_with("quillshade/project.ron"));
        assert_eq!(p.rel(&p.crate_dir()), "crates/quillshade");
    }

    #[test]
    fn create_writes_project_ron() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(Project::create(dir.path()), Err(OpenError::NoCargoToml(_))));
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
        let p = Project::create(dir.path()).unwrap();
        assert!(p.assets_dir().is_dir() && p.quillshade_dir().is_dir() && p.settings_path().is_file());
        let text = std::fs::read_to_string(p.settings_path()).unwrap();
        assert_eq!(text, "ProjectSettings(\n  project_version: 3,\n  header_date: false,\n)\n");
        assert_eq!(text, settings_text(&ProjectSettings::default()).unwrap());
        assert_eq!(Project::open(dir.path()).unwrap().settings, p.settings);
        // An existing (hand-edited) file is never rewritten.
        std::fs::write(p.settings_path(), "ProjectSettings(project_version: 3, header_date: true)\n").unwrap();
        Project::create(dir.path()).unwrap();
        let kept = std::fs::read_to_string(p.settings_path()).unwrap();
        assert_eq!(kept, "ProjectSettings(project_version: 3, header_date: true)\n");
    }

    #[test]
    fn save_settings_round_trips() {
        let dir = game("[package]\nname = \"g\"\n");
        let p = Project::create(dir.path()).unwrap();
        let mut settings = ProjectSettings { header_date: true, ..ProjectSettings::default() };
        settings.domains.insert("material".into(), DomainEntry { enabled: false });
        save_settings(&p, &settings).unwrap();
        assert_eq!(read_settings(&p.settings_path()).unwrap(), settings);
        let text = std::fs::read_to_string(p.settings_path()).unwrap();
        assert!(text.starts_with("ProjectSettings(\n  project_version: 3,\n  header_date: true,\n"), "{text}");
        assert!(text.contains("\n    \"material\": DomainEntry(\n      enabled: false,\n"), "{text}");
        assert!(text.ends_with(")\n") && !text.contains('\r'), "{text}");
        let leftovers = std::fs::read_dir(p.quillshade_dir()).unwrap().count();
        assert_eq!(leftovers, 1, "no temp file stays next to project.ron");
    }

    #[test]
    fn workspace_problem_names_the_quillshade_crate() {
        let ok = game("[workspace]\nmembers = [\"crates/*\"]\n");
        assert_eq!(Project::open(ok.path()).unwrap().workspace_problem(), None);
        let bad = game("[package]\nname = \"g\"\n");
        let text = Project::open(bad.path()).unwrap().workspace_problem().unwrap();
        assert!(text.contains("crates/quillshade"), "{text}");
        assert!(text.contains(r#"quillshade = { path = "crates/quillshade" }"#), "{text}");
    }

    #[test]
    fn globs() {
        assert!(glob_match("crates/*", "crates/quillshade"));
        assert!(glob_match("crates/quillshade", "crates/quillshade"));
        assert!(!glob_match("crates/*", "crates/quillshade/x"));
        assert!(!glob_match("other/*", "crates/quillshade"));
        assert!(glob_match("**", "crates/quillshade"));
        assert!(glob_match("crates/**", "crates/quillshade"));
        assert!(glob_match("crates/**", "crates/a/b/quillshade"));
        assert!(glob_match("crates/**/quillshade", "crates/quillshade"));
        assert!(glob_match("crates/**/quillshade", "crates/a/b/quillshade"));
        assert!(!glob_match("crates/**/quillshade", "crates/a/b/other"));
        assert!(glob_match("crates/*_*_*", "crates/bevy_shader_editor"));
        assert!(glob_match("crates/*quill*shade*", "crates/quillshade"));
        assert!(!glob_match("crates/a*a", "crates/a"));
    }

    #[test]
    fn bevy_dep_follows_the_root_manifest() {
        let dep = |manifest: &str| Project::open(game(manifest).path()).unwrap().bevy_dep();
        assert_eq!(dep("[package]\nname = \"game\"\n[dependencies]\nbevy = \"0.20\"\n"), BevyDep::Version);
        assert_eq!(dep("[workspace]\n[workspace.dependencies]\nbevy = { version = \"0.20\" }\n"), BevyDep::Workspace);
        assert_eq!(
            dep("[package]\nname = \"game\"\n[dependencies]\nbevy = { workspace = true }\n"),
            BevyDep::Workspace
        );
        assert_eq!(dep("not toml ["), BevyDep::Version);
    }

    #[test]
    fn verbatim_prefix_is_dropped() {
        let strip = |p: &str| without_verbatim_prefix(PathBuf::from(p));
        assert_eq!(strip(r"\\?\C:\games\rock"), PathBuf::from(r"C:\games\rock"));
        assert_eq!(strip(r"\\?\UNC\server\share\rock"), PathBuf::from(r"\\server\share\rock"));
        assert_eq!(strip(r"\\?\Volume{0a1b}\rock"), PathBuf::from(r"\\?\Volume{0a1b}\rock"));
        assert_eq!(strip("/home/u/rock"), PathBuf::from("/home/u/rock"));
    }
}
