//! Quillshade project layout (design §4, §4.1): `quillshade/project.ron`, per-domain source / output / module
//! directories, and the generated umbrella crate location.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::DomainId;

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
    pub format_version: u32,
    /// Add the generation date to autogen headers (breaks byte-for-byte determinism across days).
    #[serde(default)]
    pub header_date: bool,
    /// Domain entries by id; unknown ids are kept, a missing id means "enabled".
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub domains: BTreeMap<String, DomainEntry>,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        ProjectSettings { format_version: 1, header_date: false, domains: BTreeMap::new() }
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
    /// `assets/` and `quillshade/` (with default settings) are created if missing.
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

    /// Creates `quillshade/` and a default `project.ron` if missing.
    pub fn ensure_layout(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.quillshade_dir())?;
        if !self.settings_path().exists() {
            std::fs::write(self.settings_path(), settings_text(&self.settings)?)?;
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

/// Reads `project.ron`: a missing file means defaults, an unreadable or malformed one is an error.
pub fn read_settings(path: &Path) -> Result<ProjectSettings, OpenError> {
    let err = |message: String| OpenError::Settings { path: path.to_path_buf(), message };
    match std::fs::read_to_string(path) {
        Ok(text) => ron::from_str(&text).map_err(|e| err(e.to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ProjectSettings::default()),
        Err(e) => Err(err(e.to_string())),
    }
}

/// Writes `settings` to the project's `quillshade/project.ron` atomically (temp file, then a rename: an editor or
/// watcher never reads half a file), in the style [`Project::ensure_layout`] writes. `project` is not changed: the
/// caller adopts `settings` as [`Project::settings`] once this succeeded (the Project Settings tool, design §14.2).
pub fn save_settings(project: &Project, settings: &ProjectSettings) -> std::io::Result<()> {
    let batch = crate::writer::WriteBatch {
        writes: vec![(project.settings_path(), settings_text(settings)?.into_bytes())],
        deletes: Vec::new(),
    };
    batch.apply(&[])
}

/// `project.ron`'s text: struct names, 2-space indent, `\n` line ends, a final newline.
fn settings_text(settings: &ProjectSettings) -> std::io::Result<String> {
    let cfg = ron::ser::PrettyConfig::new().struct_names(true).indentor("  ".to_string()).new_line("\n".to_string());
    Ok(ron::ser::to_string_pretty(settings, cfg).map_err(std::io::Error::other)? + "\n")
}

/// Minimal cargo-style glob: `*` matches one path segment.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let p: Vec<&str> = pattern.split('/').collect();
    let s: Vec<&str> = path.split('/').collect();
    p.len() == s.len()
        && p.iter().zip(&s).all(|(a, b)| {
            if let Some((pre, post)) = a.split_once('*') { b.starts_with(pre) && b.ends_with(post) } else { a == b }
        })
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
            "(format_version: 1, header_date: true, domains: {\"light\": (enabled: false), \"nope\": (enabled: false)})",
        )
        .unwrap();
        let p = Project::open(dir.path()).unwrap();
        assert!(p.settings.header_date);
        assert!(!p.domain_enabled(DomainId::new("light")));
        assert!(p.domain_enabled(DomainId::new("material")));
    }

    #[test]
    fn open_without_project_ron_uses_defaults() {
        let dir = game("[package]\nname = \"g\"\n");
        let p = Project::open(dir.path()).unwrap();
        assert_eq!(p.settings, ProjectSettings::default());
        assert_eq!(p.settings.format_version, 1);
        assert!(!p.settings.header_date);
    }

    #[test]
    fn malformed_project_ron_is_an_open_error() {
        let dir = game("[package]\nname = \"g\"\n");
        std::fs::create_dir(dir.path().join("quillshade")).unwrap();
        let path = dir.path().join("quillshade/project.ron");
        std::fs::write(&path, "(format_version: 1, header_date: tru)").unwrap();
        let err = Project::open(dir.path()).unwrap_err();
        assert!(matches!(&err, OpenError::Settings { path, .. } if path.ends_with("quillshade/project.ron")), "{err}");
        let text = err.to_string();
        assert!(text.contains("project.ron") && !text.contains('\n'), "{text}");
        // "New project" on such a directory fails too and leaves the file alone.
        assert!(matches!(Project::create(dir.path()), Err(OpenError::Settings { .. })));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(format_version: 1, header_date: tru)");
    }

    #[test]
    fn reload_settings_keeps_the_old_ones_on_error() {
        let dir = game("[package]\nname = \"g\"\n");
        let mut p = Project::create(dir.path()).unwrap();
        std::fs::write(p.settings_path(), "(format_version: 1, header_date: true)").unwrap();
        p.reload_settings().unwrap();
        assert!(p.settings.header_date);
        std::fs::write(p.settings_path(), "(format_version: 1, header_date: ").unwrap();
        assert!(matches!(p.reload_settings(), Err(OpenError::Settings { .. })));
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
        assert!(text.starts_with("ProjectSettings(\n  format_version: 1,\n"), "{text}");
        assert!(!text.contains('\r'), "{text}");
        assert_eq!(Project::open(dir.path()).unwrap().settings, p.settings);
        // An existing (hand-edited) file is never rewritten.
        std::fs::write(p.settings_path(), "ProjectSettings(format_version: 1, header_date: true)\n").unwrap();
        Project::create(dir.path()).unwrap();
        let kept = std::fs::read_to_string(p.settings_path()).unwrap();
        assert_eq!(kept, "ProjectSettings(format_version: 1, header_date: true)\n");
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
        assert!(text.starts_with("ProjectSettings(\n  format_version: 1,\n  header_date: true,\n"), "{text}");
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
