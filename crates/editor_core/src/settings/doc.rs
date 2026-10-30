//! The Project Settings document (design §14.2): the edited [`ProjectSettings`] against the last saved (or loaded)
//! copy, with undo / redo. Saving writes `quillshade/project.ron` and the umbrella crate files.

use project_core::layout::{DomainEntry, read_settings, save_settings};
use project_core::regen::write_umbrella;
use project_core::{AssetRef, DomainId, Project, ProjectSettings};

use super::{SETTINGS_TOOL, SETTINGS_VIEW};
use crate::action::{Handled, TabAction};
use crate::doc_kind::{DocCtx, StatusCtx, StatusInfo, TabDoc};
use crate::domain::ToolId;
use crate::dynval::{PropKey, PropTarget, PropValue};
use crate::history::History;
use crate::message::{Level, Message};
use crate::tab::ViewKindId;
use crate::ui::icons::{IconId, IconRef};

/// A property of the Project Settings tab; every one is a checkbox ([`PropValue::Bool`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsProp {
    /// [`ProjectSettings::header_date`].
    HeaderDate,
    /// A domain's "Used in game" (`domains.<id>.enabled`).
    Enabled(DomainId),
}

impl PropTarget for SettingsProp {}

/// The Project Settings tab's document.
pub struct SettingsDoc {
    /// What the tab shows and edits.
    pub settings: ProjectSettings,
    /// The settings last saved or loaded: dirty is `settings != saved`.
    saved: ProjectSettings,
    history: History<ProjectSettings>,
    /// Bumped whenever `settings` changes.
    revision: u64,
}

impl SettingsDoc {
    pub fn new(settings: ProjectSettings) -> SettingsDoc {
        SettingsDoc { saved: settings.clone(), settings, history: History::default(), revision: 1 }
    }

    /// Whether domain `id` is used in the game (a missing entry: yes).
    pub fn enabled(&self, id: DomainId) -> bool {
        self.settings.domains.get(id.as_str()).is_none_or(|e| e.enabled)
    }

    /// Sets a property as one undo step; `false` when nothing changed.
    fn set(&mut self, prop: &SettingsProp, on: bool) -> bool {
        let before = self.settings.clone();
        match prop {
            SettingsProp::HeaderDate => self.settings.header_date = on,
            SettingsProp::Enabled(id) => {
                let key = id.as_str();
                match self.settings.domains.get_mut(key) {
                    // Back to what the file had: no entry stays no entry, so the tab is clean again.
                    Some(_) if on && !self.saved.domains.contains_key(key) => {
                        self.settings.domains.remove(key);
                    }
                    Some(entry) => entry.enabled = on,
                    None if !on => {
                        self.settings.domains.insert(key.to_string(), DomainEntry { enabled: false });
                    }
                    None => {}
                }
            }
        }
        let changed = self.settings != before;
        self.history.record(changed.then_some(before), None);
        if changed {
            self.revision += 1;
        }
        changed
    }

    /// Replaces the settings with the file's: saved, history cleared.
    fn replace(&mut self, settings: ProjectSettings) {
        self.saved = settings.clone();
        self.settings = settings;
        self.history.clear();
        self.revision += 1;
    }
}

impl TabDoc for SettingsDoc {
    fn apply(&mut self, _ctx: &mut DocCtx, action: &TabAction) -> Handled {
        match action {
            TabAction::Undo | TabAction::Redo => {
                let done = match action {
                    TabAction::Undo => self.history.undo(&mut self.settings),
                    _ => self.history.redo(&mut self.settings),
                };
                if done {
                    self.revision += 1;
                }
            }
            TabAction::SetValue { key, value, .. } => {
                let Some(prop) = key.downcast::<SettingsProp>() else { return Handled::No };
                self.set(prop, value.as_bool());
            }
            TabAction::CommitInline => self.history.end_merge(),
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn current_value(&self, key: &PropKey, _ctx: &StatusCtx) -> Option<PropValue> {
        Some(PropValue::Bool(match key.downcast::<SettingsProp>()? {
            SettingsProp::HeaderDate => self.settings.header_date,
            SettingsProp::Enabled(id) => self.enabled(*id),
        }))
    }

    fn asset(&self) -> Option<&AssetRef> {
        None
    }

    fn view(&self) -> ViewKindId {
        SETTINGS_VIEW
    }

    fn base_title(&self) -> String {
        "Project Settings".into()
    }

    fn icon(&self) -> IconRef {
        IconId::Settings.into()
    }

    fn watermark(&self) -> Option<&'static str> {
        Some("project.ron")
    }

    fn is_dirty(&self) -> bool {
        self.settings != self.saved
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    /// Writes `project.ron`, then the umbrella crate files from the new settings (design §14.2); the editor's project
    /// takes the new settings ([`DocCtx::saved_settings`]). A foreign umbrella file is reported (E008), not replaced.
    fn save(&mut self, ctx: &mut DocCtx) -> std::io::Result<()> {
        let project = ctx.project.ok_or_else(|| std::io::Error::other("no project is open"))?;
        save_settings(project, &self.settings)?;
        let saved = Project { settings: self.settings.clone(), ..project.clone() };
        // A failing umbrella write leaves the tab dirty, so Save can be retried.
        let blocked = write_umbrella(&saved, ctx.crates)?.blocked;
        self.saved = self.settings.clone();
        ctx.saved_settings = Some(self.settings.clone());
        for blocked in blocked {
            ctx.messages.push(Message::new(Level::Error, format!("E008: {}", blocked.message)));
        }
        Ok(())
    }

    /// The file parses to other settings than the saved ones; an unparsable file is no change (the editor warns).
    fn changed_on_disk(&self, project: &Project) -> bool {
        read_settings(&project.settings_path()).is_ok_and(|s| s != self.saved)
    }

    fn reload_from_disk(&mut self, ctx: &mut DocCtx) -> Result<(), String> {
        let project = ctx.project.ok_or("no project is open")?;
        self.replace(read_settings(&project.settings_path()).map_err(|e| e.to_string())?);
        Ok(())
    }

    fn keep_mine(&mut self, project: &Project) {
        if let Ok(on_disk) = read_settings(&project.settings_path()) {
            self.saved = on_disk;
        }
    }

    fn rename(&mut self, _project: &Project, _from: &AssetRef, _to: &AssetRef) {}

    fn config_key(&self) -> Option<String> {
        None
    }

    fn tool(&self) -> Option<ToolId> {
        Some(SETTINGS_TOOL)
    }

    fn status(&self, ctx: &StatusCtx) -> Option<StatusInfo> {
        Some(StatusInfo::new(self.is_dirty(), ctx, Vec::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIGHT: DomainId = DomainId::new("light");

    fn set(doc: &mut SettingsDoc, prop: SettingsProp, on: bool) {
        let (mut state, mut messages) = ((), Vec::new());
        let mut ctx = DocCtx::new(None, &mut state, &mut messages);
        let action = TabAction::SetValue { key: PropKey::new(prop), value: PropValue::Bool(on), merge: false };
        assert_eq!(doc.apply(&mut ctx, &action), Handled::Yes);
    }

    /// Turning a domain off and on again leaves no `enabled: true` entry the file did not have: the tab is clean.
    #[test]
    fn enabling_again_restores_the_saved_settings() {
        let mut doc = SettingsDoc::new(ProjectSettings::default());
        set(&mut doc, SettingsProp::Enabled(LIGHT), false);
        assert!(doc.is_dirty() && !doc.enabled(LIGHT));
        set(&mut doc, SettingsProp::Enabled(LIGHT), true);
        assert!(!doc.is_dirty() && doc.enabled(LIGHT));
        assert!(doc.settings.domains.is_empty());
        // An entry the file has stays, with its flag.
        let mut off = ProjectSettings::default();
        off.domains.insert(LIGHT.as_str().into(), DomainEntry { enabled: false });
        let mut doc = SettingsDoc::new(off);
        set(&mut doc, SettingsProp::Enabled(LIGHT), true);
        assert_eq!(doc.settings.domains.get("light"), Some(&DomainEntry { enabled: true }));
        assert!(doc.is_dirty());
    }

    /// A failing umbrella write (`crates/quillshade` is read-only) leaves the tab dirty so Save can be retried; once the
    /// obstacle is gone, saving makes it clean.
    #[cfg(unix)]
    #[test]
    fn failed_umbrella_write_keeps_the_tab_dirty() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"game\"\n").unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        let project = Project::open(dir.path()).unwrap();
        let crate_dir = project.crate_dir();
        std::fs::create_dir_all(&crate_dir).unwrap();
        let set_mode = |mode| std::fs::set_permissions(&crate_dir, std::fs::Permissions::from_mode(mode)).unwrap();
        set_mode(0o555);
        if std::fs::File::create(crate_dir.join("probe")).is_ok() {
            return; // permissions are not enforced (root): nothing to force
        }
        let mut doc = SettingsDoc::new(project.settings.clone());
        set(&mut doc, SettingsProp::HeaderDate, !project.settings.header_date);
        assert!(doc.is_dirty());
        let (mut state, mut messages) = ((), Vec::new());
        let mut ctx = DocCtx::new(Some(&project), &mut state, &mut messages);
        assert!(doc.save(&mut ctx).is_err());
        assert!(doc.is_dirty() && ctx.saved_settings.is_none());
        set_mode(0o755);
        doc.save(&mut ctx).unwrap();
        assert!(!doc.is_dirty() && ctx.saved_settings.is_some());
    }
}
