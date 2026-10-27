use super::*;
use shader_stdlib::internal::{INTERNAL_TEXTURES, internal_texture};

impl Editor {
    pub(super) fn texture_action(&mut self, action: TextureAction) {
        let Some(t) = self.texture_doc().filter(|t| !t.is_internal()) else { return };
        match action {
            TextureAction::Preset(p) => {
                let settings = p.apply(&t.asset.settings);
                if let Some(t) = self.texture_doc_mut() {
                    t.set_settings(settings, None);
                }
            }
            TextureAction::SetEasy(easy) => {
                // Easy shows only the presets: refused while the sampler is none of them (the button is disabled,
                // design §6.3). Switching never rewrites the settings.
                if easy && !fits_easy(&t.asset.settings) {
                    return;
                }
                if let Some(t) = self.texture_doc_mut() {
                    t.easy = easy;
                    t.relayout();
                }
            }
            TextureAction::SetPath(path) => {
                let (id, path) = (t.id.clone(), path.trim().to_string());
                if path == t.asset.path {
                    // The field may show surrounding spaces the model never had.
                    if let Some(t) = self.texture_doc_mut() {
                        t.relayout();
                    }
                    return;
                }
                match self.texture_path_problem(&id, &path) {
                    Some(why) => self.refuse_texture_edit("Cannot change the texture path", &why),
                    None => {
                        if let Some(t) = self.texture_doc_mut() {
                            t.set_path(&path);
                        }
                    }
                }
            }
            TextureAction::SetSettings(settings) => {
                if let Some(t) = self.texture_doc_mut() {
                    t.set_settings(settings, None);
                }
            }
        }
        self.ui.props += 1;
    }

    /// Undo / redo in the active Texture tab; refused (like a path edit) when the step would point the asset at a
    /// file another Texture asset now points at (design §1.1).
    pub(super) fn texture_history(&mut self, undo: bool) {
        let Some(t) = self.texture_doc().filter(|t| !t.is_internal()) else { return };
        let target = if undo { t.peek_undo() } else { t.peek_redo() };
        let Some(target) = target else { return };
        if target.path != t.asset.path
            && let Some(why) = self.texture_path_problem(&t.id, &target.path)
        {
            return self.refuse_texture_edit(if undo { "Cannot undo" } else { "Cannot redo" }, &why);
        }
        if let Some(t) = self.texture_doc_mut() {
            if undo {
                t.undo();
            } else {
                t.redo();
            }
        }
        self.ui.props += 1;
    }

    /// Why Texture asset `id` may not point at the game file `path`: the E012 rules, or another Texture asset already
    /// claims it on disk or in an open tab ([`Self::texture_owner`]; E011 would follow, design §1.1).
    fn texture_path_problem(&self, id: &AssetId, path: &str) -> Option<String> {
        if let Err(e) = TextureAsset::check_path(path) {
            return Some(e);
        }
        let other = self.texture_owner(path, Some(id))?;
        Some(format!("`{path}` already has a Texture asset: {}", other.source_rel()))
    }

    /// Reports a refused Texture edit (log + dialog); the form shows the unchanged model again.
    fn refuse_texture_edit(&mut self, what: &str, why: &str) {
        self.io_error(what, why);
        if let Some(t) = self.texture_doc_mut() {
            t.relayout();
        }
        self.ui.props += 1;
    }

    /// The Texture asset other than `except` that claims the game file `path`: it points there **on disk or in an
    /// open tab** (unsaved). Both count until the tab is saved: saving it, or regenerating from disk, must never
    /// produce two assets on one file (E011), so a file an unsaved tab moved away from stays taken until then.
    pub fn texture_owner(&self, path: &str, except: Option<&AssetId>) -> Option<AssetId> {
        let on_disk = self.disk.textures.iter().filter(|(_, t)| t.as_ref().is_ok_and(|t| t.path == path));
        let on_disk = on_disk.map(|(id, _)| id);
        let open = self.tabs.iter().filter_map(Tab::texture).filter(|t| !t.is_internal() && t.asset.path == path);
        let open = open.map(|t| &t.id);
        on_disk.chain(open).find(|id| Some(*id) != except).cloned()
    }

    /// The Texture asset claiming the game file `path` (on disk or in an open tab): "Create Texture Asset" is
    /// offered only when there is none.
    pub fn texture_for_file(&self, path: &str) -> Option<AssetId> {
        self.texture_owner(path, None)
    }

    /// What a texture reference (a node's `texture` value: a Texture asset path or an `@internal/…` id) shows: the
    /// key [`crate::game_image::GameImageCache::get`] decodes and the settings. Built-in textures first, then open
    /// Texture tabs (unsaved edits), then the asset as saved; `None` for an unknown or broken reference.
    pub fn texture_image(&self, key: &str) -> Option<(String, TextureSettings)> {
        if let Some(t) = internal_texture(key) {
            return Some((t.id.to_string(), (t.settings)()));
        }
        let id = AssetId::new(AssetType::Texture, key);
        if let Some(t) = self.tabs.iter().filter_map(Tab::texture).find(|t| t.id == id) {
            return Some((t.asset.path.clone(), t.asset.settings.clone()));
        }
        let t = self.disk.textures.get(&id)?.as_ref().ok()?;
        Some((t.path.clone(), t.settings.clone()))
    }

    /// Choices of a `ParamKind::Texture` picker as `(stored value, label)`: the project's Texture assets, then the
    /// built-in textures (`Internal/default`).
    pub fn texture_choices(&self) -> Vec<(String, String)> {
        let assets = self.disk.textures.keys().map(|id| (id.path.clone(), id.path.clone()));
        let internal = INTERNAL_TEXTURES.iter().map(|t| {
            let name = t.id.strip_prefix(shader_graph_core::texture::INTERNAL_PREFIX).unwrap_or(t.id);
            (t.id.to_string(), format!("Internal/{name}"))
        });
        assets.chain(internal).collect()
    }
}
