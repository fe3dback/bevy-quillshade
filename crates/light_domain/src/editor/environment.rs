//! An open environment (design §9.2): the [`Environment`] (source of truth), its undo history and its [`TabDoc`]
//! side (saving, reloading, renames, the editor statusbar). Edits arrive as core property writes keyed by
//! [`EnvironmentProp`]: the cube-map picker sends the texture's path (`""` for none), the number fields their value.

use editor_core::action::{Handled, TabAction};
use editor_core::doc_kind::{DocCtx, StatusCtx, StatusInfo, TabDoc};
use editor_core::dynval::{PropKey, PropTarget, PropValue};
use editor_core::history::{History, MergeKey};
use editor_core::state::editor::Editor;
use editor_core::tab::{Tab, ViewKindId};
use editor_core::ui::icons::IconRef;
use project_core::{AssetRef, Project};

use super::ENVIRONMENT_ICON;
use crate::backend::{ENVIRONMENT_SUFFIX, Environment, LIGHT, load_environment, save_environment};

/// The environment tab's view.
pub const ENVIRONMENT_VIEW: ViewKindId = ViewKindId("light.environment");

/// A field of the environment form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentProp {
    /// [`Environment::cubemap`]: a Light Texture asset's path, `""` for none ([`PropValue::Str`]).
    Cubemap,
    /// [`Environment::skybox_brightness`] ([`PropValue::Float`]).
    SkyboxBrightness,
    /// [`Environment::env_intensity`] ([`PropValue::Float`]).
    EnvIntensity,
}

impl PropTarget for EnvironmentProp {}

/// An environment tab's document.
pub struct EnvironmentDoc {
    /// The asset; follows renames.
    pub aref: AssetRef,
    /// What the tab shows and edits.
    pub env: Environment,
    /// The environment last saved or loaded: dirty is `env != saved`.
    saved: Environment,
    history: History<Environment>,
    /// Bumped whenever `env` changes.
    pub revision: u64,
    /// Bumped when the form must be rebuilt: any change but a dragged number field (which shows its own value).
    pub layout: u64,
}

impl EnvironmentDoc {
    pub fn new(aref: AssetRef, env: Environment) -> EnvironmentDoc {
        EnvironmentDoc { aref, saved: env.clone(), env, history: History::default(), revision: 1, layout: 1 }
    }

    /// Sets `prop` to `value` (a negative number is clamped to 0: no brightness); `merge` edits of one property are
    /// one undo step. `false` when nothing changed.
    fn set(&mut self, prop: EnvironmentProp, value: &PropValue, merge: Option<MergeKey>) -> bool {
        let before = self.env.clone();
        match prop {
            EnvironmentProp::Cubemap => {
                let path = value.as_str().trim();
                self.env.cubemap = (!path.is_empty()).then(|| path.to_string());
            }
            EnvironmentProp::SkyboxBrightness => self.env.skybox_brightness = value.as_f32().max(0.0),
            EnvironmentProp::EnvIntensity => self.env.env_intensity = value.as_f32().max(0.0),
        }
        let changed = self.env != before;
        if changed {
            let merged = merge.is_some();
            self.history.record(Some(before), merge);
            self.revision += 1;
            if !merged {
                self.layout += 1;
            }
        }
        changed
    }

    fn value(&self, prop: EnvironmentProp) -> PropValue {
        match prop {
            EnvironmentProp::Cubemap => PropValue::Str(self.env.cubemap.clone().unwrap_or_default()),
            EnvironmentProp::SkyboxBrightness => PropValue::Float(self.env.skybox_brightness),
            EnvironmentProp::EnvIntensity => PropValue::Float(self.env.env_intensity),
        }
    }

    /// Replaces the environment with the file's: saved, history cleared.
    fn replace(&mut self, env: Environment) {
        self.saved = env.clone();
        self.env = env;
        self.history.clear();
        self.changed();
    }

    fn changed(&mut self) {
        self.revision += 1;
        self.layout += 1;
    }

    /// The Light Texture asset `from` was renamed to `to` (`None`: deleted) and the environment files on disk were
    /// rewritten: the edited and the saved environment follow, so the unsaved edits stay and the tab is not dirty for
    /// it, and so do the undo / redo states, so an undo never brings `from` back. `false` when neither the edited nor
    /// the saved environment showed `from`.
    pub fn retarget(&mut self, from: &str, to: Option<&str>) -> bool {
        let follow = |env: &mut Environment| {
            let hit = env.cubemap.as_deref() == Some(from);
            if hit {
                env.cubemap = to.map(str::to_string);
            }
            hit
        };
        self.history.for_each_snapshot(|env| {
            follow(env);
        });
        let hit = [follow(&mut self.env), follow(&mut self.saved)].contains(&true);
        if hit {
            self.changed();
        }
        hit
    }

    /// Whether the edited or the saved environment shows the Light Texture asset `path`.
    pub fn shows(&self, path: &str) -> bool {
        [&self.env, &self.saved].iter().any(|e| e.cubemap.as_deref() == Some(path))
    }
}

/// The asset kind's `open`: the environment `r` loaded from disk; `None` with an error dialog when it does not load.
pub(super) fn open(e: &mut Editor, r: &AssetRef) -> Option<Tab> {
    let project = e.project.as_ref()?;
    match load_environment(&project.root.join(r.source_rel())) {
        Ok(env) => Some(Tab::new(LIGHT, EnvironmentDoc::new(r.clone(), env))),
        Err(err) => {
            e.io_error(&format!("cannot open {}", r.source_rel()), err);
            None
        }
    }
}

fn load(project: &Project, r: &AssetRef) -> Result<Environment, String> {
    load_environment(&project.root.join(r.source_rel()))
}

impl TabDoc for EnvironmentDoc {
    fn apply(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        match action {
            TabAction::Undo | TabAction::Redo => {
                let done = match action {
                    TabAction::Undo => self.history.undo(&mut self.env),
                    _ => self.history.redo(&mut self.env),
                };
                if done {
                    self.changed();
                    ctx.redraw_props = true;
                }
            }
            TabAction::SetValue { key, value, merge } => {
                let Some(prop) = key.downcast::<EnvironmentProp>() else { return Handled::No };
                let merge = merge.then(|| MergeKey::Prop(key.clone()));
                ctx.redraw_props |= self.set(*prop, value, merge);
            }
            TabAction::SetComponent { key, comp, value, is_final } => {
                let Some(prop) = key.downcast::<EnvironmentProp>() else { return Handled::No };
                // `None`: a field sent its shown (rounded) text back on focus loss, which is not an edit.
                if let Some(new) = self.value(*prop).with_component(*comp, *value) {
                    self.set(*prop, &new, Some(MergeKey::Prop(key.clone())));
                    // A clamped value (a typed negative number): the field must show the stored one.
                    if *is_final && self.value(*prop) != new {
                        self.layout += 1;
                    }
                }
                if *is_final {
                    self.history.end_merge();
                }
            }
            TabAction::CommitInline => self.history.end_merge(),
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn current_value(&self, key: &PropKey, _ctx: &StatusCtx) -> Option<PropValue> {
        Some(self.value(*key.downcast::<EnvironmentProp>()?))
    }

    fn asset(&self) -> Option<&AssetRef> {
        Some(&self.aref)
    }

    fn view(&self) -> ViewKindId {
        ENVIRONMENT_VIEW
    }

    /// The source path without `.ron`.
    fn base_title(&self) -> String {
        self.aref.source_rel().trim_end_matches(".ron").to_string()
    }

    fn icon(&self) -> IconRef {
        ENVIRONMENT_ICON
    }

    fn watermark(&self) -> Option<&'static str> {
        Some(ENVIRONMENT_SUFFIX)
    }

    fn is_dirty(&self) -> bool {
        self.env != self.saved
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn save(&mut self, ctx: &mut DocCtx) -> std::io::Result<()> {
        let project = ctx.project.ok_or_else(|| std::io::Error::other("no project is open"))?;
        project.ensure_layout()?;
        save_environment(&project.root.join(self.aref.source_rel()), &self.env)?;
        self.saved = self.env.clone();
        self.revision += 1;
        Ok(())
    }

    fn changed_on_disk(&self, project: &Project) -> bool {
        load(project, &self.aref).is_ok_and(|env| env != self.saved)
    }

    fn reload_from_disk(&mut self, ctx: &mut DocCtx) -> Result<(), String> {
        let project = ctx.project.ok_or("no project is open")?;
        self.replace(load(project, &self.aref)?);
        Ok(())
    }

    fn keep_mine(&mut self, project: &Project) {
        if let Ok(on_disk) = load(project, &self.aref) {
            self.saved = on_disk;
        }
    }

    fn rename(&mut self, _project: &Project, from: &AssetRef, to: &AssetRef) {
        if self.aref == *from {
            self.aref = to.clone();
            self.changed();
        }
    }

    fn config_key(&self) -> Option<String> {
        Some(self.aref.source_rel())
    }

    fn status(&self, ctx: &StatusCtx) -> Option<StatusInfo> {
        Some(StatusInfo::new(self.is_dirty(), ctx, Vec::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> EnvironmentDoc {
        EnvironmentDoc::new(crate::backend::environment_ref("day"), Environment::default())
    }

    fn apply(doc: &mut EnvironmentDoc, action: TabAction) -> Handled {
        let (mut state, mut messages) = ((), Vec::new());
        doc.apply(&mut DocCtx::new(None, &mut state, &mut messages), &action)
    }

    fn drag(prop: EnvironmentProp, value: f32, is_final: bool) -> TabAction {
        TabAction::SetComponent { key: PropKey::new(prop), comp: 0, value, is_final }
    }

    /// A drag is one undo step and keeps the form; the end of it, undo and a picked cube map rebuild it.
    #[test]
    fn drags_merge_and_keep_the_form() {
        let mut d = doc();
        let layout = d.layout;
        for v in [1.0, 2.0, 3.0] {
            assert_eq!(apply(&mut d, drag(EnvironmentProp::EnvIntensity, v, false)), Handled::Yes);
        }
        apply(&mut d, drag(EnvironmentProp::EnvIntensity, 4.0, true));
        assert_eq!((d.env.env_intensity, d.layout), (4.0, layout));
        apply(&mut d, TabAction::Undo);
        assert_eq!(d.env.env_intensity, 1.0);
        assert!(d.layout > layout && !d.is_dirty());
        let pick = TabAction::SetValue {
            key: PropKey::new(EnvironmentProp::Cubemap),
            value: PropValue::Str("sky".into()),
            merge: false,
        };
        apply(&mut d, pick);
        assert_eq!(d.env.cubemap.as_deref(), Some("sky"));
        assert_eq!(
            d.current_value(&PropKey::new(EnvironmentProp::Cubemap), &StatusCtx::default()),
            Some(PropValue::Str("sky".into()))
        );
        // Another kind's key is not this document's.
        let other = TabAction::SetValue { key: PropKey::default(), value: PropValue::Float(1.0), merge: false };
        assert_eq!(apply(&mut d, other), Handled::No);
    }

    /// A typed negative number is stored as 0 and the form is rebuilt to show it.
    #[test]
    fn clamped_value_rebuilds_the_form() {
        let mut d = doc();
        let layout = d.layout;
        apply(&mut d, drag(EnvironmentProp::EnvIntensity, -2.0, true));
        assert_eq!(d.env.env_intensity, 0.0);
        assert!(d.layout > layout, "the field shows 0, not -2");
        let layout = d.layout;
        apply(&mut d, drag(EnvironmentProp::EnvIntensity, 3.0, true));
        assert_eq!((d.env.env_intensity, d.layout), (3.0, layout), "an accepted value keeps the form");
    }

    /// A renamed cube map is followed in the edited and the saved environment: the tab stays as dirty as it was.
    #[test]
    fn retarget_keeps_the_dirty_state() {
        let mut d = EnvironmentDoc::new(
            crate::backend::environment_ref("day"),
            Environment { cubemap: Some("sky".into()), ..Environment::default() },
        );
        assert!(d.retarget("sky", Some("blue")));
        assert!(!d.is_dirty() && d.shows("blue"));
        apply(&mut d, drag(EnvironmentProp::SkyboxBrightness, 5.0, true));
        assert!(d.retarget("blue", None));
        assert_eq!(d.env.cubemap, None);
        assert!(d.is_dirty(), "the brightness edit stays");
        assert!(!d.retarget("blue", None));
        apply(&mut d, TabAction::Undo);
        assert_eq!((d.env.skybox_brightness, d.env.cubemap.as_deref()), (1000.0, None), "the undo state follows too");
        assert!(!d.is_dirty());
    }
}
