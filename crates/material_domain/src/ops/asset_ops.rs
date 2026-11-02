use super::consts_ops::ref_values;
use super::*;
use project_core::AssetUuid;
use shader_graph_core::registry::{gradient_refs, texture_refs};

/// Material's side of the core asset flows (rename, move, delete, focus a diagnostic).
pub trait MaterialAssets {
    /// The Delete dialog's question for `r`: what goes with the asset (generated files, constants nodes read).
    fn material_delete_message(&self, r: &AssetRef) -> Option<String>;

    /// Renames / moves `r`'s file ([`ops::rename`]): no other file changes, references store its UUID (ADR 0047 §13).
    fn material_rename_file(&mut self, r: &AssetRef, new_path: &str) -> Result<AssetRef, String>;

    /// Asset `from` is now `to` (core renamed the file, rescanned and let the open documents and the navigation
    /// history follow): the statusbar's WESL line count and the generated files' tabs follow, autogen regenerates.
    /// Open graphs referring to it are left alone: they store its UUID and show its new path through the resolved
    /// view (ADR 0047 §13); their D27 path comments follow on their next save.
    fn material_renamed(&mut self, from: &AssetRef, to: &AssetRef);

    /// What a reference to `r` stores ([`ref_values`]: its UUID and path), kept for [`Self::material_deleted`]: the
    /// index forgets the UUID when core rescans after the delete.
    fn material_before_delete(&mut self, r: &AssetRef);

    /// Deletes `r`'s file: references to it in other assets are cleared on disk ([`ops::delete`]).
    fn material_delete_file(&mut self, r: &AssetRef) -> Result<(), String>;

    /// Asset `r` was deleted (core rescanned): open graphs referring to a deleted Texture asset / gradient collection /
    /// constant table follow the clearing (matched by the values [`Self::material_before_delete`] kept), autogen
    /// regenerates.
    fn material_deleted(&mut self, r: &AssetRef);

    /// `ops::delete` cleared the references naming the deleted asset (`names`, found by `refs`) on disk; open graph
    /// documents follow: clean ones reload the rewritten file, dirty ones get the same `clear` as an edit (unsaved
    /// edits stay unsaved). Call after [`Self::rescan`].
    fn follow_ref_clear(
        &mut self,
        refs: fn(&Asset, &Registry) -> Vec<(NodeId, String, String)>,
        names: &[String],
        clear: impl Fn(&mut Asset, &Registry),
    );

    fn show_file(&mut self, id: &AssetId, lang: Option<&'static str>);

    /// Gives the file of `r` (any Material kind, user nodes included) the asset id `id` through its canonical writer
    /// ([`ops::set_asset_uuid`], labelled by the editor's index; the UUID quick fixes, ADR 0047 §16). A rewritten user
    /// node re-registers, so its UUID resolves at once.
    fn material_set_asset_uuid(&mut self, r: &AssetRef, id: AssetUuid) -> Result<(), String>;

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
        self.regenerate();
    }

    fn material_before_delete(&mut self, r: &AssetRef) {
        let names = AssetId::from_ref(r).map(|id| ref_values(&self.asset_index, id.ty, &id.path));
        self.domains.material_mut().deleting = names.unwrap_or_default();
    }

    fn material_delete_file(&mut self, r: &AssetRef) -> Result<(), String> {
        let (Some(project), Some(id)) = (self.project.clone(), AssetId::from_ref(r)) else {
            return Err(format!("{} is not a Material asset", r.source_rel()));
        };
        ops::delete(&project, &id).map_err(|e| e.to_string())
    }

    fn material_deleted(&mut self, r: &AssetRef) {
        let Some(id) = AssetId::from_ref(r) else { return };
        let names = std::mem::take(&mut self.domains.material_mut().deleting);
        let names = &names;
        self.domains.material_mut().wesl_lines.remove(&id);
        match id.ty {
            AssetType::Texture => self.follow_ref_clear(texture_refs, names, |a, r| {
                ops::clear_texture_refs(a, r, names);
            }),
            // The slot parameters of the same nodes are zeroed too (gradient design §6).
            AssetType::Gradients => self.follow_ref_clear(gradient_refs, names, |a, r| {
                ops::clear_gradient_refs(a, r, names);
            }),
            AssetType::Consts => self.follow_ref_clear(const_table_refs, names, |a, r| {
                ops::clear_const_table_refs(a, r, names);
            }),
            _ => {}
        }
        self.regenerate();
    }

    fn follow_ref_clear(
        &mut self,
        refs: fn(&Asset, &Registry) -> Vec<(NodeId, String, String)>,
        names: &[String],
        clear: impl Fn(&mut Asset, &Registry),
    ) {
        let registry = &self.domains.material().registry;
        for d in self.tabs.iter_mut().filter_map(Tab::get_mut::<Doc>) {
            if !refs(&d.asset, registry).iter().any(|r| names.contains(&r.2)) {
                continue;
            }
            if d.is_dirty() {
                d.edit(|a| clear(a, registry));
            } else if let Some(Ok(a)) = self.domains.material().disk.assets.get(&d.id) {
                d.reload(a.clone());
            }
        }
    }

    fn material_set_asset_uuid(&mut self, r: &AssetRef, id: AssetUuid) -> Result<(), String> {
        let project = self.project.clone().ok_or("no project is open")?;
        ops::set_asset_uuid(&project, r, id, &self.asset_index).map_err(|e| e.to_string())?;
        if r.suffix.is_empty() {
            self.material_files_changed(&[project.root.join(r.source_rel())]);
        }
        Ok(())
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
