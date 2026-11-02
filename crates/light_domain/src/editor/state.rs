//! The Light domain's state in the editor (design §6.3, §9.2): the assets of the last scan, the diagnostics of what
//! the open tabs show, generation (Generate, after a save) and what follows a deleted Light texture.

use std::collections::BTreeMap;
use std::path::Path;

use curve_kit::editor::CurveDoc;
use editor_core::domain::DomainState;
use editor_core::message::Level;
use editor_core::state::editor::Editor;
use editor_core::tab::{Tab, TextDoc, parse_highlights};
use editor_core::util::text::plural;
use project_core::regen::commit;
use project_core::writer::Change;
use project_core::{AssetRef, AssetUuid, Diagnostic, DomainId};
use texture_kit::TextureAsset;
use texture_kit::editor::TextureDoc;

use super::{EnvironmentDoc, LightCurve, LightTexture};
use crate::backend::{
    CURVE_SUFFIX, ENVIRONMENT_SUFFIX, LIGHT, LightAssets, TEXTURE_SUFFIX, clear_texture_refs, plan_assets,
    resolve_texture, scan_game_conflicts, set_asset_uuid, texture_ref, texture_ref_names, validate_in_memory,
    validate_texture, validate_textures,
};

/// What the Light domain keeps between frames.
#[derive(Default)]
pub struct LightState {
    /// The Light assets on disk, as of the last rescan or save.
    pub assets: LightAssets,
    /// [`crate::backend::validate`] of `assets` with the open tabs' (unsaved) environments and Texture assets in place
    /// of their files.
    pub diagnostics: Vec<Diagnostic>,
    /// Cached per-texture diagnostics (`(asset, diagnostics)`) so curve and environment edits do not touch the disk.
    pub texture_diags: BTreeMap<String, (TextureAsset, Vec<Diagnostic>)>,
    /// Cached game Rust conflict warnings (`LW102`) from the last rescan or save.
    pub game_conflicts: Vec<Diagnostic>,
    /// Incremented on every `reload` so [`super::PreviewCubemapCache`] knows when to drop cached `.ktx2` handles.
    pub reload_generation: u64,
    /// What a cube map naming the Light texture being deleted stores (its UUID and path), from `before_delete` to
    /// `deleted` (the index forgets the UUID when core rescans after the delete).
    pub deleting: Vec<String>,
}

impl LightState {
    /// Re-reads the assets from disk, then [`LightState::revalidate`].
    fn reload(editor: &mut Editor) {
        let project = editor.project.clone().filter(|p| p.domain_enabled(LIGHT));
        let game_conflicts = project.as_ref().map(scan_game_conflicts).unwrap_or_default();
        for doc in editor.tabs.iter_mut().filter_map(|t| t.get_mut::<super::InspectGameDoc>()) {
            doc.set_conflicts(game_conflicts.clone());
        }
        let Some(project) = project else {
            let state = editor.domains.get_mut::<Self>().expect("Light domain not registered");
            let next_gen = state.reload_generation + 1;
            *state = Self { reload_generation: next_gen, ..Self::default() };
            return;
        };
        let mut assets = LightAssets::load(&project);
        // Cube map UUIDs read as paths everywhere in the editor's checks (ADR 0047 §4).
        assets.resolve_locally();
        let texture_diags = validate_textures(&project, &assets.textures)
            .into_iter()
            .filter_map(|(k, d)| Some((k.clone(), (assets.textures.get(&k)?.clone(), d))))
            .collect();
        let state = editor.domains.get_mut::<Self>().expect("Light domain not registered");
        state.assets = assets;
        state.texture_diags = texture_diags;
        state.game_conflicts = game_conflicts;
        state.reload_generation += 1;
        Self::revalidate(editor);
    }

    /// The diagnostics of the assets as the open tabs show them; the console/logs, the trees' error colours and the
    /// Texture tabs' problems redraw when they changed.
    fn revalidate(editor: &mut Editor) {
        let Some(project) = editor.project.clone().filter(|p| p.domain_enabled(LIGHT)) else {
            let state = editor.domains.get_mut::<Self>().expect("Light domain not registered");
            if !state.diagnostics.is_empty() {
                state.diagnostics.clear();
                editor.ui.log += 1;
                editor.ui.browser += 1;
                editor.ui.props += 1;
            }
            return;
        };
        let shown_assets = {
            let state = editor.domains.get::<Self>().expect("Light domain not registered");
            shown(&state.assets, &editor.tabs)
        };
        let state = editor.domains.get_mut::<Self>().expect("Light domain not registered");
        let prev_len = state.texture_diags.len();
        state.texture_diags.retain(|k, _| shown_assets.textures.contains_key(k));
        let paths_changed = state.texture_diags.len() != prev_len
            || shown_assets
                .textures
                .iter()
                .any(|(k, t)| state.texture_diags.get(k).is_none_or(|(cached, _)| cached.path != t.path));
        if paths_changed {
            state.texture_diags.clear();
        }
        if shown_assets.textures.iter().any(|(k, t)| state.texture_diags.get(k).is_none_or(|(cached, _)| cached != t)) {
            let mut by_file: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
            for (path, t) in &shown_assets.textures {
                by_file.entry(t.path.as_str()).or_default().push(path.as_str());
            }
            for (path, t) in &shown_assets.textures {
                if state.texture_diags.get(path).is_none_or(|(cached, _)| cached != t) {
                    let others: Vec<String> = by_file[t.path.as_str()]
                        .iter()
                        .filter(|o| **o != path)
                        .map(|o| texture_ref(o).source_rel())
                        .collect();
                    let diags = validate_texture(&project, path, t, &others);
                    state.texture_diags.insert(path.clone(), (t.clone(), diags));
                }
            }
        }
        let tex_map: BTreeMap<String, Vec<Diagnostic>> =
            state.texture_diags.iter().map(|(k, (_, d))| (k.clone(), d.clone())).collect();
        let diagnostics = validate_in_memory(&shown_assets, &tex_map, &state.game_conflicts);
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

    /// The cube maps naming a deleted Light texture (`names`) were cleared in the environment files on disk
    /// (`rewritten`): the open environment tabs follow ([`EnvironmentDoc::clear_cubemap`]: their edits and undo
    /// history stay), then the assets reload and the module regenerates. Returns the environments that showed it.
    fn texture_deleted(editor: &mut Editor, names: &[String], rewritten: Vec<String>) -> Vec<String> {
        let mut shown = rewritten;
        for doc in editor.tabs.iter_mut().filter_map(|t| t.get_mut::<EnvironmentDoc>()) {
            // Edited, saved and undo / redo states alike: a clean tab stays clean, a dirty one keeps its edits.
            if doc.clear_cubemap(names) && !shown.contains(&doc.aref.path) {
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

/// The Light Texture asset path a cube map `value` names: a UUID the editor's `AssetIndex` knows reads as the
/// texture's current path (ADR 0047 §4); a path or a dangling UUID stays.
pub(super) fn cubemap_path<'a>(editor: &'a Editor, value: &'a str) -> &'a str {
    resolve_texture(value, &editor.asset_index).unwrap_or(value)
}

/// `assets` with the open Light tabs' environments, curves and Texture assets (unsaved edits included) in place of
/// their files, cube maps resolved to paths ([`LightAssets::resolve_locally`]).
pub(super) fn shown(assets: &LightAssets, tabs: &[Tab]) -> LightAssets {
    let mut out = assets.clone();
    for t in tabs.iter().filter(|t| t.domain == LIGHT) {
        if let Some(d) = t.get::<EnvironmentDoc>() {
            out.environments.insert(d.aref.path.clone(), d.env.clone());
        } else if let Some(d) = t.get::<CurveDoc<LightCurve>>() {
            out.curves.insert(d.aref.path.clone(), d.curve.clone());
        } else if let Some(d) = t.get::<TextureDoc<LightTexture>>() {
            out.textures.insert(d.aref.path.clone(), d.asset.clone());
        }
    }
    out.resolve_locally();
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

    /// A Light texture that environments show (cube maps are resolved to paths): they lose their cube map.
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

    /// Any rename regenerates (the module lists the environments; core rescanned first). Nothing else follows: cube
    /// maps store the texture's UUID and the form shows its new path (ADR 0047 §13).
    fn renamed(editor: &mut Editor, _from: &AssetRef, _to: &AssetRef) {
        Self::regenerate(editor);
    }

    /// The UUID quick fixes (ADR 0047 §16): through the canonical writers ([`set_asset_uuid`]), so a hand-written
    /// file without an `id` line gets one (core's default only rewrites an existing line).
    fn set_asset_uuid(editor: &mut Editor, r: &AssetRef, id: AssetUuid) -> Result<(), String> {
        let project = editor.project.clone().ok_or("no project is open")?;
        set_asset_uuid(&project, r, id, &editor.asset_index)
    }

    /// Keeps what a cube map naming a Light texture `r` stores for [`DomainState::deleted`].
    fn before_delete(editor: &mut Editor, r: &AssetRef) {
        let names =
            if r.suffix == TEXTURE_SUFFIX { texture_ref_names(&r.path, &editor.asset_index) } else { Vec::new() };
        editor.domains.get_mut::<Self>().expect("Light domain not registered").deleting = names;
    }

    /// A deleted Light texture: the environments naming it lose their cube map, with a warning.
    fn deleted(editor: &mut Editor, r: &AssetRef) {
        if r.suffix != TEXTURE_SUFFIX {
            return Self::regenerate(editor);
        }
        let Some(project) = editor.project.clone() else { return };
        let state = editor.domains.get_mut::<Self>().expect("Light domain not registered");
        let names = std::mem::take(&mut state.deleting);
        let rewritten = match clear_texture_refs(&project, &names) {
            Ok(r) => r,
            Err(e) => {
                editor.io_error("Cannot update the environments", e);
                Vec::new()
            }
        };
        let shown = Self::texture_deleted(editor, &names, rewritten);
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

    /// Opens the diagnostic's asset, or a read-only Rust text tab when the diagnostic points to a game `.rs` file
    /// (`LW102`).
    fn focus_diagnostic(editor: &mut Editor, diag: &Diagnostic) {
        let Some(rel) = diag.asset.as_deref() else { return };
        if let Some(r) = project_core::asset_ref::parse(rel, LIGHT, &[ENVIRONMENT_SUFFIX, CURVE_SUFFIX, TEXTURE_SUFFIX])
        {
            editor.open_ref(&r);
        } else if let Some(project) = editor.project.clone() {
            let mut lines = diag.location.as_deref().map(parse_highlights).unwrap_or_default();
            if lines.is_empty()
                && let Some(state) = editor.domains.get::<Self>()
            {
                for conflict in state.game_conflicts.iter().filter(|d| d.asset.as_deref() == Some(rel)) {
                    if let Some(loc) = conflict.location.as_deref() {
                        lines.extend(parse_highlights(loc));
                    }
                }
            }
            if let Some(i) = editor.tabs.iter().position(|t| t.get::<TextDoc>().is_some_and(|d| d.rel == rel)) {
                if let Some(doc) = editor.tabs[i].get_mut::<TextDoc>() {
                    doc.set_highlights(lines);
                }
                editor.activate(i);
            } else {
                let source = AssetRef { domain: LIGHT, path: rel.to_string(), suffix: "rs" };
                let doc = TextDoc::open(&project, source, "rust", project.root.join(rel)).with_highlights(lines);
                editor.push_tab(Tab::new(LIGHT, doc));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Environment, curve_ref, environment_ref};
    use curve_kit::Curve;

    /// The open tab's environment or curve replaces its file's in what is validated.
    #[test]
    fn open_tabs_override_the_files() {
        let mut assets = LightAssets::default();
        assets.environments.insert("day".into(), Environment::default());
        assets.curves.insert("ease".into(), Curve::ramp("ease"));
        let env = Environment { cubemap: Some("sky".into()), ..Environment::default() };
        let mut edited_curve = Curve::ramp("ease");
        edited_curve.max = 2.0;
        let tabs = [
            Tab::new(LIGHT, EnvironmentDoc::new(environment_ref("day"), env.clone())),
            Tab::new(LIGHT, CurveDoc::<LightCurve>::new(curve_ref("ease"), edited_curve.clone())),
        ];
        let overridden = shown(&assets, &tabs);
        assert_eq!(overridden.environments["day"], env);
        assert_eq!(overridden.curves["ease"], edited_curve);
        assert_eq!(shown(&assets, &[]).environments["day"], Environment::default());
        assert_eq!(shown(&assets, &[]).curves["ease"], Curve::ramp("ease"));
    }
}
