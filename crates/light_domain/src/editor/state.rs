//! The Light domain's state in the editor (design §6.3, §9.2): the assets of the last scan, the diagnostics of what
//! the open tabs show, generation (Generate, after a save) and what follows a renamed or deleted Light texture.

use std::path::Path;

use editor_core::domain::DomainState;
use editor_core::message::Level;
use editor_core::state::editor::Editor;
use editor_core::tab::Tab;
use editor_core::util::text::plural;
use project_core::regen::commit;
use project_core::writer::Change;
use project_core::{AssetRef, Diagnostic, DomainId};
use texture_kit::editor::TextureDoc;

use super::{EnvironmentDoc, LightTexture};
use crate::backend::{
    ENVIRONMENT_SUFFIX, LIGHT, LightAssets, TEXTURE_SUFFIX, plan_assets, rewrite_texture_refs, validate,
};

/// What the Light domain keeps between frames.
#[derive(Default)]
pub struct LightState {
    /// The Light assets on disk, as of the last rescan or save.
    pub assets: LightAssets,
    /// [`validate`] of `assets` with the open tabs' (unsaved) environments and Texture assets in place of their files.
    pub diagnostics: Vec<Diagnostic>,
}

impl LightState {
    /// Re-reads the assets from disk, then [`LightState::revalidate`].
    fn reload(editor: &mut Editor) {
        let Some(project) = editor.project.as_ref().filter(|p| p.domain_enabled(LIGHT)) else {
            *editor.domains.get_mut::<Self>().expect("Light domain not registered") = Self::default();
            return;
        };
        let assets = LightAssets::load(project);
        editor.domains.get_mut::<Self>().expect("Light domain not registered").assets = assets;
        Self::revalidate(editor);
    }

    /// The diagnostics of the assets as the open tabs show them; the console/logs, the trees' error colours and the
    /// Texture tabs' problems redraw when they changed.
    fn revalidate(editor: &mut Editor) {
        let diagnostics = match editor.project.as_ref().filter(|p| p.domain_enabled(LIGHT)) {
            Some(project) => {
                let assets = &editor.domains.get::<Self>().expect("Light domain not registered").assets;
                validate(project, &shown(assets, &editor.tabs))
            }
            None => Vec::new(),
        };
        let state = editor.domains.get_mut::<Self>().expect("Light domain not registered");
        if diagnostics != state.diagnostics {
            state.diagnostics = diagnostics;
            editor.ui.log += 1;
            editor.ui.browser += 1;
            editor.ui.props += 1;
        }
    }

    /// Writes the Light module and the umbrella crate files from what is saved (design §6.5): every registered
    /// domain's contribution, so the umbrella keeps every domain's module. Logs what was written and blocked files
    /// (E008); a changed Rust file needs a rebuild of the game.
    fn regenerate(editor: &mut Editor) {
        let Some(project) = editor.project.clone().filter(|p| p.domain_enabled(LIGHT)) else { return };
        let list = editor.domain_list.clone();
        let assets = &editor.domains.get::<Self>().expect("Light domain not registered").assets;
        let plan = plan_assets(&project, assets);
        let committed = match commit(&project, &[plan.output], list.crate_contributions(), false) {
            Ok(c) => c,
            Err(e) => return editor.io_error("Regeneration failed", e),
        };
        for blocked in committed.blocked {
            editor.push_msg(Level::Error, format!("E008: {}", blocked.message));
        }
        let n = committed.changes.len();
        match n {
            0 => editor.info("Regenerated: no changes"),
            _ => editor.info(format!("Regenerated: {} written", plural(n, "file", "files"))),
        }
        let crate_dir = project.crate_dir();
        if committed.changes.iter().map(Change::path).any(|p| is_rust_in(p, &crate_dir)) {
            let text = "The generated Rust crate changed: rebuild the game to apply it";
            editor.push_msg(Level::Warning, text.into());
        }
    }

    /// The Light texture `from` was renamed to `to` (`None`: deleted) and the environment files on disk naming it were
    /// rewritten (`rewritten`): the open environment tabs follow ([`EnvironmentDoc::retarget`]: their edits and undo
    /// history stay), then the assets reload and the module regenerates. Returns the environments that showed it.
    fn texture_moved(editor: &mut Editor, from: &str, to: Option<&str>, rewritten: Vec<String>) -> Vec<String> {
        let mut shown = rewritten;
        for doc in editor.tabs.iter_mut().filter_map(|t| t.get_mut::<EnvironmentDoc>()) {
            // Edited, saved and undo / redo states alike: a clean tab stays clean, a dirty one keeps its edits.
            if doc.retarget(from, to) && !shown.contains(&doc.aref.path) {
                shown.push(doc.aref.path.clone());
            }
        }
        shown.sort();
        editor.ui.props += 1;
        editor.ui.tabs += 1;
        Self::reload(editor);
        Self::regenerate(editor);
        shown
    }
}

/// Whether `path` is a Rust file of the generated crate.
fn is_rust_in(path: &Path, crate_dir: &Path) -> bool {
    path.starts_with(crate_dir) && path.extension().is_some_and(|e| e == "rs")
}

/// `assets` with the open Light tabs' environments and Texture assets (unsaved edits included) in place of their
/// files.
fn shown(assets: &LightAssets, tabs: &[Tab]) -> LightAssets {
    let mut out = assets.clone();
    for t in tabs.iter().filter(|t| t.domain == LIGHT) {
        if let Some(d) = t.get::<EnvironmentDoc>() {
            out.environments.insert(d.aref.path.clone(), d.env.clone());
        } else if let Some(d) = t.get::<TextureDoc<LightTexture>>() {
            out.textures.insert(d.aref.path.clone(), d.asset.clone());
        }
    }
    out
}

impl DomainState for LightState {
    fn id(&self) -> DomainId {
        LIGHT
    }

    fn rescan(editor: &mut Editor) {
        Self::reload(editor);
    }

    /// An edit of a Light tab, or a Light command: the diagnostics follow the open documents.
    fn after_apply(editor: &mut Editor) {
        Self::revalidate(editor);
    }

    /// The saved files are read again and the module regenerates.
    fn after_save(editor: &mut Editor, _tabs: &[usize]) {
        Self::reload(editor);
        Self::regenerate(editor);
    }

    fn diagnostics(&self) -> Vec<Diagnostic> {
        self.diagnostics.clone()
    }

    fn generate(editor: &mut Editor) {
        Self::reload(editor);
        Self::regenerate(editor);
    }

    /// A Light texture that environments show: they lose their cube map.
    fn delete_message(&self, _editor: &Editor, r: &AssetRef) -> Option<String> {
        if r.suffix != TEXTURE_SUFFIX {
            return None;
        }
        let users: Vec<&str> = self
            .assets
            .environments
            .iter()
            .filter(|(_, env)| env.cubemap.as_deref() == Some(r.path.as_str()))
            .map(|(path, _)| path.as_str())
            .collect();
        (!users.is_empty()).then(|| {
            format!("Delete {}?\nIt is the cube map of {}: they will have none.", r.source_rel(), users.join(", "))
        })
    }

    /// A renamed Light texture: the environments naming it (files and open tabs) follow. Any rename regenerates (the
    /// module lists the environments); core rescanned first.
    fn renamed(editor: &mut Editor, from: &AssetRef, to: &AssetRef) {
        if from.suffix != TEXTURE_SUFFIX {
            return Self::regenerate(editor);
        }
        let Some(project) = editor.project.clone() else { return };
        let rewritten = match rewrite_texture_refs(&project, &from.path, Some(&to.path)) {
            Ok(r) => r,
            Err(e) => {
                editor.io_error("Cannot update the environments", e);
                Vec::new()
            }
        };
        let shown = Self::texture_moved(editor, &from.path, Some(&to.path), rewritten);
        if !shown.is_empty() {
            let what = plural(shown.len(), "environment", "environments");
            editor.info(format!("Updated the cube map of {what}: {}", shown.join(", ")));
        }
    }

    /// A deleted Light texture: the environments naming it lose their cube map, with a warning.
    fn deleted(editor: &mut Editor, r: &AssetRef) {
        if r.suffix != TEXTURE_SUFFIX {
            return Self::regenerate(editor);
        }
        let Some(project) = editor.project.clone() else { return };
        let rewritten = match rewrite_texture_refs(&project, &r.path, None) {
            Ok(r) => r,
            Err(e) => {
                editor.io_error("Cannot update the environments", e);
                Vec::new()
            }
        };
        let shown = Self::texture_moved(editor, &r.path, None, rewritten);
        if !shown.is_empty() {
            let text = format!(
                "Deleted {} was the cube map of {}: {}; they have none now",
                r.path,
                plural(shown.len(), "environment", "environments"),
                shown.join(", ")
            );
            editor.push_msg(Level::Warning, text);
        }
    }

    /// Opens the diagnostic's asset.
    fn focus_diagnostic(editor: &mut Editor, diag: &Diagnostic) {
        let Some(rel) = diag.asset.as_deref() else { return };
        if let Some(r) = project_core::asset_ref::parse(rel, LIGHT, &[ENVIRONMENT_SUFFIX, TEXTURE_SUFFIX]) {
            editor.open_ref(&r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Environment, environment_ref};

    /// The open tab's environment replaces its file's in what is validated.
    #[test]
    fn open_tabs_override_the_files() {
        let mut assets = LightAssets::default();
        assets.environments.insert("day".into(), Environment::default());
        let env = Environment { cubemap: Some("sky".into()), ..Environment::default() };
        let tabs = [Tab::new(LIGHT, EnvironmentDoc::new(environment_ref("day"), env.clone()))];
        assert_eq!(shown(&assets, &tabs).environments["day"], env);
        assert_eq!(shown(&assets, &[]).environments["day"], Environment::default());
    }
}
