//! [`RawProject`]: the project handle of the Migration Tool (techdoc §4.4, ADR 0046 "Crate `project_migrate`
//! (D38)"). Unlike `project_core::Project::open` it does not require a current-version `project.ron`: it runs the
//! same directory checks and the version probe, but never compares the version (the orchestrator does).

use std::path::{Path, PathBuf};

use project_core::layout::{QUILLSHADE_DIR, SETTINGS_FILE, without_verbatim_prefix};
use project_core::{DomainId, OpenError, ProjectVersion, missing_settings_version, probe_project_version};

use crate::MigrationError;

/// A game project at any project version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawProject {
    /// The project root (canonicalized when possible, like `Project::root`).
    pub root: PathBuf,
    /// The probed project version; without `project.ron` it is v1 beside sources, else [`ProjectVersion::CURRENT`]
    /// (ADR 0046 "Missing `project.ron`", [`missing_settings_version`]).
    pub version: ProjectVersion,
}

impl RawProject {
    /// Opens `root`, which must contain `Cargo.toml` and `assets/`, and probes `quillshade/project.ron`'s version
    /// (either key). Old, current and too-new projects all open; an unreadable `project.ron` or one without a
    /// readable version is [`OpenError::Settings`].
    pub fn open(root: impl AsRef<Path>) -> Result<RawProject, MigrationError> {
        let root = root.as_ref();
        let root = root.canonicalize().map_or_else(|_| root.to_path_buf(), without_verbatim_prefix);
        if !root.is_dir() {
            return Err(OpenError::NotADirectory(root).into());
        }
        if !root.join("Cargo.toml").is_file() {
            return Err(OpenError::NoCargoToml(root).into());
        }
        if !root.join("assets").is_dir() {
            return Err(OpenError::NoAssets(root).into());
        }
        let version = read_version(&root.join(QUILLSHADE_DIR).join(SETTINGS_FILE))?;
        Ok(RawProject { root, version })
    }

    /// `quillshade/`.
    pub fn quillshade_dir(&self) -> PathBuf {
        self.root.join(QUILLSHADE_DIR)
    }

    /// Source root of a domain (`quillshade/<id>`).
    pub fn domain_dir(&self, domain: DomainId) -> PathBuf {
        self.quillshade_dir().join(domain.as_str())
    }

    /// `quillshade/project.ron`.
    pub fn settings_path(&self) -> PathBuf {
        self.quillshade_dir().join(SETTINGS_FILE)
    }

    /// Project-relative display path with `/` separators.
    pub fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.root).unwrap_or(p).to_string_lossy().replace('\\', "/")
    }
}

/// The version of the `project.ron` at `path`; a missing file takes it from what `quillshade/` holds, by the same
/// rule as `Project::open` ([`missing_settings_version`]).
fn read_version(path: &Path) -> Result<ProjectVersion, MigrationError> {
    let settings = |message: String| OpenError::Settings { path: path.to_path_buf(), message };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(missing_settings_version(path.parent().unwrap_or(Path::new(""))));
        }
        Err(e) => return Err(settings(e.to_string()).into()),
    };
    Ok(probe_project_version(&text).map_err(|e| settings(e.to_string()))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
        dir
    }

    #[test]
    fn opens_any_version_and_defaults_to_current() {
        let dir = game();
        assert_eq!(RawProject::open(dir.path()).unwrap().version, ProjectVersion::CURRENT);
        std::fs::create_dir(dir.path().join("quillshade")).unwrap();
        for (text, version) in [
            ("ProjectSettings(format_version: 1)", 1),
            ("ProjectSettings(project_version: 2)", 2),
            ("ProjectSettings(project_version: 9, future: true)", 9),
        ] {
            std::fs::write(dir.path().join("quillshade/project.ron"), text).unwrap();
            assert_eq!(RawProject::open(dir.path()).unwrap().version, ProjectVersion(version), "{text}");
        }
    }

    /// ADR 0046 "Missing `project.ron`": sources without `project.ron` are v1, an empty `quillshade/` is new.
    #[test]
    fn legacy_layout_is_v1() {
        let dir = game();
        std::fs::create_dir_all(dir.path().join("quillshade/.hidden")).unwrap();
        assert_eq!(RawProject::open(dir.path()).unwrap().version, ProjectVersion::CURRENT);
        std::fs::create_dir_all(dir.path().join("quillshade/light")).unwrap();
        std::fs::write(dir.path().join("quillshade/light/day.environment.ron"), "()").unwrap();
        assert_eq!(RawProject::open(dir.path()).unwrap().version, ProjectVersion::FIRST);
        assert!(!dir.path().join("quillshade/project.ron").exists());
    }

    #[test]
    fn open_errors() {
        let dir = tempfile::tempdir().unwrap();
        let err = RawProject::open(dir.path()).unwrap_err();
        assert!(matches!(err, MigrationError::Open(OpenError::NoCargoToml(_))), "{err}");
        std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        let err = RawProject::open(dir.path()).unwrap_err();
        assert!(matches!(err, MigrationError::Open(OpenError::NoAssets(_))), "{err}");
        let err = RawProject::open(dir.path().join("missing")).unwrap_err();
        assert!(matches!(err, MigrationError::Open(OpenError::NotADirectory(_))), "{err}");
        let dir = game();
        std::fs::create_dir(dir.path().join("quillshade")).unwrap();
        std::fs::write(dir.path().join("quillshade/project.ron"), "ProjectSettings(header_date: true)").unwrap();
        let err = RawProject::open(dir.path()).unwrap_err();
        assert!(matches!(err, MigrationError::Open(OpenError::Settings { .. })), "{err}");
        assert!(err.to_string().contains("project_version") && !err.to_string().contains('\n'), "{err}");
    }

    #[test]
    fn paths() {
        let dir = game();
        let p = RawProject::open(dir.path()).unwrap();
        assert!(p.quillshade_dir().ends_with("quillshade"));
        assert!(p.domain_dir(DomainId::new("light")).ends_with("quillshade/light"));
        assert!(p.settings_path().ends_with("quillshade/project.ron"));
        assert_eq!(p.rel(&p.settings_path()), "quillshade/project.ron");
    }
}
