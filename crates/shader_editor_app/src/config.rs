//! User config (spec §9 "State", decision 0018): window size, recent projects, open tabs, the View menu's flags. Lives
//! outside the project, in `<OS config dir>/fe3dback/bevy-shader-editor/config.ron`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserConfig {
    pub window: (u32, u32),
    pub maximized: bool,
    /// Full (canonical) project roots, most recent first.
    pub recent_projects: Vec<PathBuf>,
    /// Open tabs (`AssetId::source_rel`, e.g. `shaders/env/rock.material.ron`) per project root.
    pub tabs: BTreeMap<PathBuf, ProjectTabs>,
    /// View → Show project (Ctrl+1): the left sidebar is shown.
    pub show_left_sidebar: bool,
    /// View → Show console (Ctrl+2): the console/logs shows its messages, not only its header bar.
    pub show_log: bool,
    /// View → Show properties (Ctrl+3): the right sidebar is shown.
    pub show_properties: bool,
    /// View → Zen mode (`` ` ``): hides the left and right sidebars and the whole console/logs, whatever their own
    /// flags say (design §3). Stored apart from them, so turning it off shows exactly the panels whose flags are on.
    pub zen_mode: bool,
}

/// A panel the View flags show or hide ([`UserConfig::panel_visible`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    /// The left sidebar: project tree and assets tree.
    LeftSidebar,
    /// The console/logs strip itself, its header included: hidden only by Zen mode.
    Console,
    /// The console/logs' messages under its header.
    Log,
    /// The right sidebar.
    Properties,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectTabs {
    pub open: Vec<String>,
    pub active: Option<usize>,
}

impl Default for UserConfig {
    fn default() -> Self {
        UserConfig {
            window: (1600, 960),
            maximized: false,
            recent_projects: Vec::new(),
            tabs: BTreeMap::new(),
            show_left_sidebar: true,
            show_log: true,
            show_properties: true,
            zen_mode: false,
        }
    }
}

/// How many recent projects are remembered (all of them show on the welcome screen).
pub const RECENT_LIMIT: usize = 50;

impl UserConfig {
    /// `<OS config dir>/fe3dback/bevy-shader-editor/config.ron`, see [`os_config_dir`].
    pub fn default_path() -> Option<PathBuf> {
        let base = os_config_dir(std::env::consts::OS, |k| std::env::var_os(k), std::env::home_dir())?;
        Some(base.join("fe3dback").join("bevy-shader-editor").join("config.ron"))
    }

    /// Creates the directory of the config file if it is missing.
    pub fn ensure_dir(path: &Path) -> std::io::Result<()> {
        path.parent().map_or(Ok(()), std::fs::create_dir_all)
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

    /// Whether `panel` is shown: its own flag, folded with Zen mode, which has priority. The one place the layout
    /// systems ask.
    pub fn panel_visible(&self, panel: Panel) -> bool {
        !self.zen_mode
            && match panel {
                Panel::LeftSidebar => self.show_left_sidebar,
                Panel::Console => true,
                Panel::Log => self.show_log,
                Panel::Properties => self.show_properties,
            }
    }

    pub fn touch_recent(&mut self, project: &Path) {
        self.recent_projects.retain(|p| p != project);
        self.recent_projects.insert(0, project.to_path_buf());
        self.recent_projects.truncate(RECENT_LIMIT);
    }

    /// Drops `project` from the recent projects (its "x" button); the project itself is untouched.
    pub fn forget_recent(&mut self, project: &Path) {
        self.recent_projects.retain(|p| p != project);
    }
}

/// The OS config directory: environment variables first, then the platform's usual place under `home`:
/// - Windows: `%APPDATA%`, else `<home>\AppData\Roaming`;
/// - macOS: `$HOME/Library/Application Support`, else the same under `home`;
/// - others: `$XDG_CONFIG_HOME`, `$HOME/.config`, else `<home>/.config`.
pub fn os_config_dir(os: &str, env: impl Fn(&str) -> Option<OsString>, home: Option<PathBuf>) -> Option<PathBuf> {
    let var = |k: &str| env(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    match os {
        "windows" => var("APPDATA").or_else(|| home.map(|h| h.join("AppData").join("Roaming"))),
        "macos" => var("HOME").or(home).map(|h| h.join("Library").join("Application Support")),
        _ => var("XDG_CONFIG_HOME").or_else(|| var("HOME").or(home).map(|h| h.join(".config"))),
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
        assert_eq!(c.recent_projects, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
        c.forget_recent(Path::new("/a"));
        assert_eq!(c.recent_projects, vec![PathBuf::from("/b")]);
        for i in 0..RECENT_LIMIT + 5 {
            c.touch_recent(&PathBuf::from(format!("/p{i}")));
        }
        assert_eq!(c.recent_projects.len(), RECENT_LIMIT);
        c.tabs.insert("/a".into(), ProjectTabs { open: vec!["shaders/env/rock.material.ron".into()], active: Some(0) });
        c.save(&path).unwrap();
        assert_eq!(UserConfig::load(&path), c);
        assert_eq!(UserConfig::load(&dir.path().join("missing.ron")), UserConfig::default());
    }

    /// Review Focus 3, 5: a config file from before the View menu opens with the log and the properties shown; the
    /// flags survive a save and a load.
    #[test]
    fn view_flags_default_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.ron");
        std::fs::write(
            &path,
            "(\n  window: (800, 600),\n  maximized: false,\n  recent_projects: [],\n  tabs: {},\n)\n",
        )
        .unwrap();
        let old = UserConfig::load(&path);
        assert_eq!(old.window, (800, 600), "the old file is read");
        assert!(old.show_log && old.show_properties);
        let c = UserConfig { show_log: false, ..UserConfig::default() };
        c.save(&path).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("show_log: false"));
        let loaded = UserConfig::load(&path);
        assert_eq!(loaded, c);
        assert!(!loaded.show_log && loaded.show_properties);
    }

    /// Design §3, Review Focus 3: a config file from before the panel toggles opens with every panel shown and Zen
    /// mode off.
    #[test]
    fn old_config_shows_all_panels_without_zen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.ron");
        std::fs::write(&path, "(\n  window: (800, 600),\n  show_log: false,\n  show_properties: true,\n)\n").unwrap();
        let old = UserConfig::load(&path);
        assert_eq!(old.window, (800, 600), "the old file is read");
        assert!(old.show_left_sidebar && !old.zen_mode);
        assert!(!old.show_log && old.show_properties);
        assert!(old.panel_visible(Panel::LeftSidebar) && old.panel_visible(Panel::Console));
        assert!(!old.panel_visible(Panel::Log) && old.panel_visible(Panel::Properties));
    }

    /// All four View flags survive a save and a load.
    #[test]
    fn view_flags_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.ron");
        for (left, log, props, zen) in [(false, true, false, true), (true, false, true, false)] {
            let c = UserConfig {
                show_left_sidebar: left,
                show_log: log,
                show_properties: props,
                zen_mode: zen,
                ..UserConfig::default()
            };
            c.save(&path).unwrap();
            let loaded = UserConfig::load(&path);
            assert_eq!(loaded, c);
            assert_eq!(
                (loaded.show_left_sidebar, loaded.show_log, loaded.show_properties, loaded.zen_mode),
                (left, log, props, zen)
            );
        }
    }

    /// Zen mode hides every panel, whatever its own flag says (design §3); off, each panel follows its flag. The
    /// console strip (its header) is hidden only by Zen.
    #[test]
    fn zen_folds_into_panel_visibility() {
        let panels = [Panel::LeftSidebar, Panel::Console, Panel::Log, Panel::Properties];
        let shown = |c: &UserConfig| panels.map(|p| c.panel_visible(p));
        let mut c = UserConfig::default();
        assert_eq!(shown(&c), [true, true, true, true]);
        c.show_log = false;
        c.show_left_sidebar = false;
        assert_eq!(shown(&c), [false, true, false, true]);
        c.zen_mode = true;
        assert_eq!(shown(&c), [false; 4]);
        c.show_log = true;
        c.show_left_sidebar = true;
        assert_eq!(shown(&c), [false; 4], "the flags change, Zen still hides");
        c.zen_mode = false;
        assert_eq!(shown(&c), [true; 4]);
    }

    #[test]
    fn os_config_dirs() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| OsString::from(v))
        };
        let home = Some(PathBuf::from("/home/u"));
        let dir = |os, vars, home: &Option<PathBuf>| os_config_dir(os, env(vars), home.clone());
        assert_eq!(dir("linux", &[("XDG_CONFIG_HOME", "/x"), ("HOME", "/h")], &home), Some("/x".into()));
        assert_eq!(dir("linux", &[("XDG_CONFIG_HOME", ""), ("HOME", "/h")], &home), Some("/h/.config".into()));
        assert_eq!(dir("freebsd", &[], &home), Some("/home/u/.config".into()));
        assert_eq!(dir("linux", &[], &None), None);
        assert_eq!(dir("macos", &[("HOME", "/h")], &home), Some("/h/Library/Application Support".into()));
        assert_eq!(dir("macos", &[], &home), Some("/home/u/Library/Application Support".into()));
        assert_eq!(dir("windows", &[], &home), Some(PathBuf::from("/home/u").join("AppData").join("Roaming")));
        assert_eq!(dir("windows", &[("APPDATA", "/appdata")], &home), Some("/appdata".into()));
    }
}
