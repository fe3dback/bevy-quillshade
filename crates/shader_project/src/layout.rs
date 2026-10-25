//! Project detection and directory layout (spec §5).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use shader_graph_core::{Code, Diagnostic};

/// Settings file inside `shaders/`.
pub const SETTINGS_FILE: &str = ".shader-editor.ron";
/// Directory with data-driven user nodes inside `shaders/`.
pub const USER_NODES_DIR: &str = ".nodes";
/// Generated crate location relative to the project root.
pub const CRATE_DIR: &str = "crates/shaders";
/// WESL autogen location relative to the project root.
pub const AUTOGEN_DIR: &str = "assets/shaders/autogen";

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("`{0}` is not a directory")]
    NotADirectory(PathBuf),
    #[error("`{0}` is not a project: Cargo.toml is missing")]
    NoCargoToml(PathBuf),
    #[error("`{0}` is not a project: assets/ directory is missing")]
    NoAssets(PathBuf),
}

/// Project-level options stored in `shaders/.shader-editor.ron`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSettings {
    pub format_version: u32,
    /// Add the generation date to autogen headers (breaks byte-for-byte determinism across days).
    #[serde(default)]
    pub header_date: bool,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        ProjectSettings { format_version: 1, header_date: false }
    }
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
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        if !root.is_dir() {
            return Err(OpenError::NotADirectory(root));
        }
        if !root.join("Cargo.toml").is_file() {
            return Err(OpenError::NoCargoToml(root));
        }
        if !root.join("assets").is_dir() {
            return Err(OpenError::NoAssets(root));
        }
        let settings = std::fs::read_to_string(root.join("shaders").join(SETTINGS_FILE))
            .ok()
            .and_then(|t| ron::from_str(&t).ok())
            .unwrap_or_default();
        Ok(Project { root, settings })
    }

    pub fn shaders_dir(&self) -> PathBuf {
        self.root.join("shaders")
    }

    pub fn autogen_dir(&self) -> PathBuf {
        self.root.join(AUTOGEN_DIR)
    }

    pub fn crate_dir(&self) -> PathBuf {
        self.root.join(CRATE_DIR)
    }

    pub fn settings_path(&self) -> PathBuf {
        self.shaders_dir().join(SETTINGS_FILE)
    }

    /// Source file of an asset (`shaders/{path}.{suffix}.ron`).
    pub fn asset_file(&self, path: &str, suffix: &str) -> PathBuf {
        self.shaders_dir().join(format!("{path}.{suffix}.ron"))
    }

    /// Project-relative display path.
    pub fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.root).unwrap_or(p).to_string_lossy().replace('\\', "/")
    }

    /// Creates `shaders/` (with default settings) if missing.
    pub fn ensure_shaders_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.shaders_dir())?;
        if !self.settings_path().exists() {
            let text = ron::ser::to_string_pretty(&self.settings, ron::ser::PrettyConfig::new().struct_names(true))
                .expect("settings serialization");
            std::fs::write(self.settings_path(), text + "\n")?;
        }
        Ok(())
    }

    /// W105: `crates/shaders` missing from `[workspace.members]` of the root `Cargo.toml`.
    pub fn workspace_warning(&self) -> Option<Diagnostic> {
        let text = std::fs::read_to_string(self.root.join("Cargo.toml")).ok()?;
        let doc: toml::Table = text.parse().ok()?;
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
                "add to the root Cargo.toml:\n[workspace]\nmembers = [\"{CRATE_DIR}\"]\n\nand to [dependencies]: shaders = {{ path = \"{CRATE_DIR}\" }}"
            )
        };
        Some(
            Diagnostic::new(Code::W105, format!("{CRATE_DIR} is not in [workspace.members]; {hint}"))
                .in_asset("Cargo.toml"),
        )
    }
}

/// Minimal cargo-style glob: `*` matches one path segment.
fn glob_match(pattern: &str, path: &str) -> bool {
    let p: Vec<&str> = pattern.split('/').collect();
    let s: Vec<&str> = path.split('/').collect();
    p.len() == s.len()
        && p.iter().zip(&s).all(|(a, b)| {
            if let Some((pre, post)) = a.split_once('*') { b.starts_with(pre) && b.ends_with(post) } else { a == b }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_match("crates/*", "crates/shaders"));
        assert!(glob_match("crates/shaders", "crates/shaders"));
        assert!(!glob_match("crates/*", "crates/shaders/x"));
        assert!(!glob_match("other/*", "crates/shaders"));
    }
}
