//! User config (spec §9 "State"): window size, recent projects, open tabs. Lives outside the project.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserConfig {
    pub window: (u32, u32),
    pub maximized: bool,
    /// Most recent first.
    pub recent: Vec<PathBuf>,
    /// Open tabs (`AssetId::source_rel`, e.g. `shaders/env/rock.material.ron`) per project root.
    pub tabs: BTreeMap<PathBuf, ProjectTabs>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectTabs {
    pub open: Vec<String>,
    pub active: Option<usize>,
}

impl Default for UserConfig {
    fn default() -> Self {
        UserConfig { window: (1600, 960), maximized: false, recent: Vec::new(), tabs: BTreeMap::new() }
    }
}

const RECENT_LIMIT: usize = 10;

impl UserConfig {
    /// `$XDG_CONFIG_HOME/shader-editor/config.ron` (or `~/.config/…`, `%APPDATA%\…`).
    pub fn default_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("shader-editor").join("config.ron"))
    }

    pub fn load(path: &Path) -> UserConfig {
        std::fs::read_to_string(path).ok().and_then(|t| ron::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default().indentor("  "))
            .map_err(std::io::Error::other)?;
        if std::fs::read_to_string(path).ok().as_deref() == Some(text.as_str()) {
            return Ok(());
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("ron.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }

    pub fn touch_recent(&mut self, project: &Path) {
        self.recent.retain(|p| p != project);
        self.recent.insert(0, project.to_path_buf());
        self.recent.truncate(RECENT_LIMIT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_recent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.ron");
        let mut c = UserConfig::default();
        c.touch_recent(Path::new("/a"));
        c.touch_recent(Path::new("/b"));
        c.touch_recent(Path::new("/a"));
        assert_eq!(c.recent, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
        c.tabs.insert("/a".into(), ProjectTabs { open: vec!["shaders/env/rock.material.ron".into()], active: Some(0) });
        c.save(&path).unwrap();
        assert_eq!(UserConfig::load(&path), c);
        assert_eq!(UserConfig::load(&dir.path().join("missing.ron")), UserConfig::default());
    }
}
