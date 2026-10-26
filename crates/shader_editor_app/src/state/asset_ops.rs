use super::*;
use shader_graph_core::registry::{curve_refs, texture_refs};

impl Editor {
    pub(super) fn submit(&mut self, text: &str) {
        let Some(Dialog::Input { submit, .. }) = self.dialog.take() else { return };
        self.pick_folder = false;
        match submit {
            Submit::OpenProject => return self.apply(Action::OpenProject(PathBuf::from(text))),
            Submit::NewProject => return self.apply(Action::NewProject(PathBuf::from(text))),
            _ => {}
        }
        let Some(project) = self.project.clone() else { return };
        let text = text.trim_matches('/');
        match submit {
            Submit::Create(kind) => {
                let r = match (kind, kind.material()) {
                    (_, Some(m)) => ops::create_material(&project, text, m),
                    (NewKind::Curve, None) => ops::create_curve(&project, text),
                    (NewKind::Consts, None) => ops::create_consts(&project, text),
                    (_, None) => ops::create_function(&project, text),
                };
                match r {
                    Ok(id) => {
                        self.info(format!("Created {}", id.source_rel()));
                        self.rescan();
                        self.open_asset(&id);
                        self.regenerate();
                    }
                    Err(e) => self.io_error("Cannot create asset", e),
                }
            }
            Submit::Folder => match ops::create_folder(&project, text) {
                Ok(()) => {
                    self.current_folder = text.to_string();
                    self.rescan();
                }
                Err(e) => self.io_error("Cannot create folder", e),
            },
            Submit::Rename(id) => self.rename(&id, text),
            Submit::MoveTo(id) => {
                self.apply(Action::MoveTo(id, text.to_string()));
            }
            Submit::OpenProject | Submit::NewProject => {}
        }
    }

    /// Creates a Texture asset with default settings for the game image `path` (relative to `assets/`) at
    /// `shaders/textures/<folders>/<stem>.texture.ron` ([`game_assets::new_texture_id_for_file`], design §4.3) and
    /// regenerates. Refused (an error message) when a Texture asset already points at the file, on disk or in an open
    /// tab. Not undoable: asset creation is not part of any document's history.
    pub fn create_texture_for_file(&mut self, path: &str) -> Option<AssetId> {
        self.try_create_texture_for_file(path).map_err(|e| self.io_error("Cannot create Texture asset", e)).ok()
    }

    /// [`Self::create_texture_for_file`] without the error dialog: the error text is returned instead (a drop onto
    /// a graph reports it itself).
    pub fn try_create_texture_for_file(&mut self, path: &str) -> Result<AssetId, String> {
        let project = self.project.clone().ok_or("no project is open")?;
        // One game file, one Texture asset: claimed on disk or by an open tab (unsaved) is refused.
        if let Some(other) = self.texture_for_file(path) {
            return Err(format!("`{path}` already has a Texture asset: {}", other.source_rel()));
        }
        let id =
            game_assets::create_texture_for_file(&project, &self.overridden_set(), path).map_err(|e| e.to_string())?;
        self.info(format!("Created {} for assets/{path}", id.source_rel()));
        self.rescan();
        // An image tab of the file offers the creation in Properties.
        self.ui.props += 1;
        self.regenerate();
        Ok(id)
    }

    /// Rename/move; open documents follow, function references are updated, autogen regenerates.
    pub(super) fn rename(&mut self, id: &AssetId, new_path: &str) {
        self.dialog = None;
        let Some(project) = self.project.clone() else { return };
        if let Some(i) = self.asset_index(id)
            && self.tabs[i].doc().is_dirty()
            && !self.save(i)
        {
            return;
        }
        let new_id = match ops::rename(&project, id, new_path) {
            Ok(n) => n,
            Err(e) => return self.io_error("Cannot rename", e),
        };
        if new_id == *id {
            return;
        }
        self.rescan();
        // The asset's document and its generated files follow (regeneration below fills the files).
        for t in &mut self.tabs {
            t.doc_mut().rename(&project, id, &new_id);
        }
        if id.ty == AssetType::Function {
            let (old_kind, new_kind) =
                (format!("{FUNCTION_PREFIX}{}", id.path), format!("{FUNCTION_PREFIX}{}", new_id.path));
            for d in self.tabs.iter_mut().filter_map(Tab::graph_mut) {
                if !d.asset.nodes.values().any(|n| n.kind == old_kind) {
                    continue;
                }
                if d.is_dirty() {
                    d.edit(|a| {
                        for n in a.nodes.values_mut().filter(|n| n.kind == old_kind) {
                            n.kind = new_kind.clone();
                        }
                    });
                } else if let Some(Ok(a)) = self.disk.assets.get(&d.id) {
                    d.reload(a.clone());
                }
            }
        }
        match id.ty {
            AssetType::Curve => self.follow_ref_rewrite(curve_refs, &id.path, |a, r| {
                ops::rename_curve_refs(a, r, &id.path, &new_id.path);
            }),
            AssetType::Texture => self.follow_ref_rewrite(texture_refs, &id.path, |a, r| {
                ops::rename_texture_refs(a, r, &id.path, &new_id.path);
            }),
            _ => {}
        }
        self.info(format!("Renamed {} -> {}", id.source_rel(), new_id.source_rel()));
        self.regenerate();
        self.touch_all();
        self.save_config();
    }

    pub(super) fn delete(&mut self, id: &AssetId) {
        self.dialog = None;
        let Some(project) = self.project.clone() else { return };
        // A deleted table's constants: their readers lose them (unless another table defines the same name).
        let owned = self.owned_consts(id, self.global_consts().into_keys());
        while let Some(i) = self.tabs.iter().position(|t| t.owner() == Some(id)) {
            self.close_tab(i, true);
        }
        if let Err(e) = ops::delete(&project, id) {
            return self.io_error("Cannot delete", e);
        }
        self.info(format!("Deleted {}", id.source_rel()));
        self.rescan();
        if id.ty == AssetType::Texture {
            self.follow_ref_rewrite(texture_refs, &id.path, |a, r| {
                ops::clear_texture_refs(a, r, &id.path);
            });
        }
        let left = self.global_consts();
        let lost: Vec<String> = owned.into_iter().filter(|n| !left.contains_key(n)).collect();
        if self.const_usages(&lost).is_empty() {
            self.regenerate();
        } else {
            self.clear_const_usages(&lost);
        }
        self.touch_all();
    }

    /// `ops::rename` / `ops::delete` rewrote the references to asset `target` (found by `refs`) on disk; open graph
    /// documents follow: clean ones reload the rewritten file, dirty ones get the same `rewrite` as an edit (unsaved
    /// edits stay unsaved). Call after [`Self::rescan`].
    fn follow_ref_rewrite(
        &mut self,
        refs: fn(&Asset, &Registry) -> Vec<(NodeId, String, String)>,
        target: &str,
        rewrite: impl Fn(&mut Asset, &Registry),
    ) {
        let registry = &self.registry;
        for d in self.tabs.iter_mut().filter_map(Tab::graph_mut) {
            if !refs(&d.asset, registry).iter().any(|r| r.2 == target) {
                continue;
            }
            if d.is_dirty() {
                d.edit(|a| rewrite(a, registry));
            } else if let Some(Ok(a)) = self.disk.assets.get(&d.id) {
                d.reload(a.clone());
            }
        }
    }

    pub(super) fn show_file(&mut self, id: &AssetId, which: GenFile) {
        let Some(project) = &self.project else { return };
        let path = match which {
            GenFile::Wesl => wesl_file(project, id),
            GenFile::Rust => rust_file(project, id),
            GenFile::Source => Some(id.file(project)),
        };
        match path {
            Some(p) if p.exists() => self.external.push(p),
            Some(p) => {
                let msg = format!("{} does not exist yet (save the asset to generate it)", project.rel(&p));
                self.push_msg(Level::Warning, msg);
            }
            None if !id.ty.has_wesl() => self.push_msg(Level::Warning, "constant tables generate no files".into()),
            None => self.push_msg(Level::Warning, "functions have no Rust module".into()),
        }
    }

    pub(super) fn focus_diagnostic(&mut self, diag: &Diagnostic) {
        let Some(id) = diag.asset.as_deref().and_then(assets::id_from_source_rel) else { return };
        if self.disk.contains(&id) {
            self.open_asset(&id);
        }
        if let (Some(node), Some(d)) = (&diag.node, self.doc_mut())
            && d.id == id
            && d.asset.nodes.contains_key(node)
        {
            d.selection = [node.clone()].into();
            d.selected_frame = None;
            d.revision += 1;
            self.view_requests.retain(|v| *v != ViewRequest::FitAll);
            self.view_requests.push(ViewRequest::FitSelection);
            self.ui.props += 1;
        }
    }
}
