use super::*;
use shader_graph_core::registry::{curve_refs, gradient_refs, texture_refs};

/// Material's side of the core asset flows (rename, move, delete, focus a diagnostic).
pub trait MaterialAssets {
    /// The Delete dialog's question for `r`: what goes with the asset (generated files, constants nodes read).
    fn material_delete_message(&self, r: &AssetRef) -> Option<String>;

    /// Renames / moves `r`'s file: references to it in other assets are rewritten on disk ([`ops::rename`]).
    fn material_rename_file(&mut self, r: &AssetRef, new_path: &str) -> Result<AssetRef, String>;

    /// Asset `from` is now `to` (core renamed the file, rescanned and let the open documents and the navigation
    /// history follow): the statusbar's WESL line count follows, open graphs calling a renamed function or referring
    /// to a renamed curve / Texture asset / gradient collection follow the rewrite, autogen regenerates.
    fn material_renamed(&mut self, from: &AssetRef, to: &AssetRef);

    /// Deletes `r`'s file: references to it in other assets are cleared on disk ([`ops::delete`]).
    fn material_delete_file(&mut self, r: &AssetRef) -> Result<(), String>;

    /// Asset `r` was deleted (core rescanned): open graphs referring to a deleted Texture asset / gradient collection /
    /// constant table follow the rewrite, autogen regenerates.
    fn material_deleted(&mut self, r: &AssetRef);

    /// `ops::rename` / `ops::delete` rewrote the references to asset `target` (found by `refs`) on disk; open graph
    /// documents follow: clean ones reload the rewritten file, dirty ones get the same `rewrite` as an edit (unsaved
    /// edits stay unsaved). Call after [`Self::rescan`].
    fn follow_ref_rewrite(
        &mut self,
        refs: fn(&Asset, &Registry) -> Vec<(NodeId, String, String)>,
        target: &str,
        rewrite: impl Fn(&mut Asset, &Registry),
    );

    fn show_file(&mut self, id: &AssetId, lang: Option<&'static str>);

    /// Opens the asset of a diagnostic (a console/logs row, the editor statusbar's count) and selects its node
    /// (`MaterialState`'s `focus_diagnostic` hook).
    fn material_focus_diagnostic(&mut self, diag: &project_core::Diagnostic);
}

impl MaterialAssets for Editor {
    // ---- Material's side of the core asset flows (design §6.3, `MaterialState`'s hooks) ----

    fn material_delete_message(&self, r: &AssetRef) -> Option<String> {
        let id = AssetId::from_ref(r)?;
        let message = if id.ty.has_wesl() {
            format!("Delete {} and its generated files?", id.source_rel())
        } else if id.ty == AssetType::Consts
            && let Some(t) = self.const_table(&id.path)
        {
            let ids: Vec<u32> = t.consts.iter().map(|c| c.id).collect();
            let names: Vec<String> = t.consts.iter().map(|c| c.name.clone()).collect();
            match self.const_usages(&id.path, &ids) {
                u if u.is_empty() => format!("Delete {}?", id.source_rel()),
                u => usage_message(&format!("Deleting {}", id.source_rel()), &names, &u),
            }
        } else {
            format!("Delete {}?", id.source_rel())
        };
        Some(message)
    }

    fn material_rename_file(&mut self, r: &AssetRef, new_path: &str) -> Result<AssetRef, String> {
        let (Some(project), Some(id)) = (self.project.clone(), AssetId::from_ref(r)) else {
            return Err(format!("{} is not a Material asset", r.source_rel()));
        };
        ops::rename(&project, &id, new_path).map(|n| n.to_ref()).map_err(|e| e.to_string())
    }

    fn material_renamed(&mut self, from: &AssetRef, to: &AssetRef) {
        let (Some(id), Some(new_id)) = (AssetId::from_ref(from), AssetId::from_ref(to)) else { return };
        let id = &id;
        // The statusbar's WESL line count follows until regeneration below recounts.
        if let Some(n) = self.domains.material_mut().wesl_lines.remove(id) {
            self.domains.material_mut().wesl_lines.insert(new_id.clone(), n);
        }
        // The generated files' tabs follow (regeneration fills the new files).
        if let Some(project) = self.project.clone() {
            for t in self.tabs.iter_mut().filter_map(Tab::get_mut::<TextDoc>).filter(|t| t.source == *from) {
                if let Some(path) = crate::tab::generated_path(&project, &new_id, t.lang) {
                    t.retarget(&project, to.clone(), path);
                }
            }
        }
        if id.ty == AssetType::Function {
            let (old_kind, new_kind) =
                (format!("{FUNCTION_PREFIX}{}", id.path), format!("{FUNCTION_PREFIX}{}", new_id.path));
            for d in self.tabs.iter_mut().filter_map(Tab::get_mut::<Doc>) {
                if !d.asset.nodes.values().any(|n| n.kind == old_kind) {
                    continue;
                }
                if d.is_dirty() {
                    d.edit(|a| {
                        for n in a.nodes.values_mut().filter(|n| n.kind == old_kind) {
                            n.kind = new_kind.clone();
                        }
                    });
                } else if let Some(Ok(a)) = self.domains.material().disk.assets.get(&d.id) {
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
            AssetType::Gradients => self.follow_ref_rewrite(gradient_refs, &id.path, |a, r| {
                ops::rename_gradient_refs(a, r, &id.path, &new_id.path);
            }),
            AssetType::Consts => self.follow_ref_rewrite(const_table_refs, &id.path, |a, r| {
                ops::rename_const_table_refs(a, r, &id.path, &new_id.path);
            }),
            _ => {}
        }
        self.regenerate();
    }

    fn material_delete_file(&mut self, r: &AssetRef) -> Result<(), String> {
        let (Some(project), Some(id)) = (self.project.clone(), AssetId::from_ref(r)) else {
            return Err(format!("{} is not a Material asset", r.source_rel()));
        };
        ops::delete(&project, &id).map_err(|e| e.to_string())
    }

    fn material_deleted(&mut self, r: &AssetRef) {
        let Some(id) = AssetId::from_ref(r) else { return };
        let id = &id;
        self.domains.material_mut().wesl_lines.remove(id);
        match id.ty {
            AssetType::Texture => self.follow_ref_rewrite(texture_refs, &id.path, |a, r| {
                ops::clear_texture_refs(a, r, &id.path);
            }),
            // The slot parameters of the same nodes are zeroed too (gradient design §6).
            AssetType::Gradients => self.follow_ref_rewrite(gradient_refs, &id.path, |a, r| {
                ops::clear_gradient_refs(a, r, &id.path);
            }),
            AssetType::Consts => self.follow_ref_rewrite(const_table_refs, &id.path, |a, r| {
                ops::clear_const_table_refs(a, r, &id.path);
            }),
            _ => {}
        }
        self.regenerate();
    }

    fn follow_ref_rewrite(
        &mut self,
        refs: fn(&Asset, &Registry) -> Vec<(NodeId, String, String)>,
        target: &str,
        rewrite: impl Fn(&mut Asset, &Registry),
    ) {
        let registry = &self.domains.material().registry;
        for d in self.tabs.iter_mut().filter_map(Tab::get_mut::<Doc>) {
            if !refs(&d.asset, registry).iter().any(|r| r.2 == target) {
                continue;
            }
            if d.is_dirty() {
                d.edit(|a| rewrite(a, registry));
            } else if let Some(Ok(a)) = self.domains.material().disk.assets.get(&d.id) {
                d.reload(a.clone());
            }
        }
    }

    fn show_file(&mut self, id: &AssetId, lang: Option<&'static str>) {
        let Some(project) = &self.project else { return };
        let path = match lang {
            Some(crate::tab::WESL) => wesl_file(project, id),
            Some(crate::tab::RUST) => rust_file(project, id),
            _ => Some(id.file(project)),
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

    fn material_focus_diagnostic(&mut self, diag: &project_core::Diagnostic) {
        let Some(id) = diag.asset.as_deref().and_then(assets::id_from_source_rel) else { return };
        if self.domains.material().disk.contains(&id) {
            self.open_asset(&id);
        }
        // The location is `node[.pin]`.
        let node = diag.location.as_deref().map(|l| l.split_once('.').map_or(l, |(n, _)| n).to_string());
        if let (Some(node), Some(d)) = (&node, self.doc_mut())
            && d.id == id
            && d.asset.nodes.contains_key(node)
        {
            d.set_selection([node.clone()]);
            d.selected_frame = None;
            d.revision += 1;
            self.view_requests.retain(|v| *v != ViewRequest::FitAll);
            self.view_requests.push(ViewRequest::FitSelection);
            self.ui.props += 1;
        }
    }
}
