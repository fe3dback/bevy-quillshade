use super::*;

/// Material's commands and hooks (`MaterialState`'s `apply`, `after_apply`, `before_close`, `rescan`).
pub trait MaterialApply {
    /// A payload for the Material domain ([`MaterialState`]'s `apply` hook): one of its commands or a step one of its
    /// documents deferred ([`DocCtx::defer`]: graph edits, constant table edits that need other tabs or a dialog).
    /// Texture steps go to `texture_kit`'s step handler first.
    fn material_payload(&mut self, payload: &Payload);

    fn material_action(&mut self, action: MaterialAction);

    /// After an action reached a Material tab or the domain ([`MaterialState`]'s `after_apply` hook): function
    /// signatures and validation follow closed tabs, and a changed preview regenerates (decision 0022).
    fn material_after_apply(&mut self);

    /// Documents a reload replaced while their generated files came from unsaved edits ([`Doc::restore_from_disk`])
    /// regenerate from disk (decision 0022).
    fn restore_reloaded(&mut self);

    /// A preview change (toggle, mode, undo, deleting the previewed node) regenerates the material from the in-memory
    /// document (decision 0022). Each graph document remembers the preview it last followed
    /// ([`Doc::preview_seen`]).
    fn follow_previews(&mut self);

    /// Tab `i` is about to lose its unsaved edits ([`MaterialState`]'s `before_close` hook): generated files written
    /// from them (a preview, decision 0022) are regenerated from disk.
    fn material_before_close(&mut self, i: usize);

    /// The project's files may have changed ([`MaterialState`]'s `rescan` hook): the asset list and the declared
    /// function signatures. After the project opened or closed, the previous project's state goes first: the
    /// registry with the new project's user nodes, diagnostics, generated line counts.
    fn material_rescan(&mut self);
}

impl MaterialApply for Editor {
    // ---- Material's commands (design §6.2) ----

    fn material_payload(&mut self, payload: &Payload) {
        if let Some(a) = payload.downcast::<MaterialAction>() {
            self.material_action(a.clone());
        } else if let Some(e) = payload.downcast::<GraphEdit>() {
            // The deferred step of a `TabAction::Doc` edit (`graph_ops`: the document checked the stale payload and
            // deferred the edit here); there is no other production sender, so this is not a second route.
            crate::graph_domain::graph_edit(self, e.clone());
        } else if let Some(a) = payload.downcast::<ConstsAction>() {
            self.consts_action(a.clone());
        } else {
            bevy::log::debug!("{payload:?} ignored by the Material domain");
        }
    }

    fn material_action(&mut self, action: MaterialAction) {
        match action {
            MaterialAction::AddNode { kind, pos, link, params } => {
                self.add_node(&kind, pos, link, &params);
            }
            MaterialAction::AddNodeAndRename { kind, pos, link, params } => {
                if let Some(target) = self.add_node(&kind, pos, link, &params).and_then(|id| self.rename_target(&id)) {
                    self.material_action(MaterialAction::TextDialog(target));
                }
            }
            MaterialAction::Drop { item, pos, node } => self.drop_item(item, pos, node),
            MaterialAction::SetPinType { node, ty } => self.set_pin_type(&node, ty),
            MaterialAction::ResetInput { node, pin } => {
                if let Some(d) = self.doc_mut() {
                    d.edit(|a| a.nodes.get_mut(&node).map(|n| n.inputs.remove(&pin)));
                }
                self.ui.props += 1;
            }
            MaterialAction::ConvertNodes { nodes, conv } => self.convert_nodes(&nodes, &conv),
            MaterialAction::TogglePreview(node) => self.toggle_preview(node),
            MaterialAction::TextDialog(target) => crate::graph_domain::text_dialog(self, target),
            MaterialAction::ShowFile(id, which) => self.show_file(&id, which),
            MaterialAction::ShowGenerated(id, lang) => self.show_generated(&id, lang),
            MaterialAction::FocusNode(node) => {
                let Some(d) = self.doc_mut() else { return };
                if !d.asset.nodes.contains_key(&node) {
                    return;
                }
                d.set_selection([node]);
                d.selected_frame = None;
                d.revision += 1;
                self.view_requests.retain(|v| *v != ViewRequest::FitAll);
                self.view_requests.push(ViewRequest::FitSelection);
                self.ui.props += 1;
            }
            MaterialAction::OpenGradient { id, slot_id } => {
                self.open_asset(&id);
                if let Some(d) = self.gradients_doc_mut()
                    && d.id == id
                    && let Some(pos) = d.collection.position(slot_id)
                {
                    d.select_slot(pos);
                    self.ui.props += 1;
                }
            }
        }
    }

    fn material_after_apply(&mut self) {
        if std::mem::take(&mut self.domains.material_mut().tabs_closed) {
            self.refresh_functions();
            self.domains.material_mut().validated = None;
        }
        self.restore_reloaded();
        self.follow_previews();
    }

    fn restore_reloaded(&mut self) {
        let mut ids = Vec::new();
        for d in self.tabs.iter_mut().filter_map(Tab::get_mut::<Doc>) {
            if std::mem::take(&mut d.restore_from_disk) {
                ids.push(d.id.clone());
            }
        }
        for id in &ids {
            self.regenerate_from_disk(id);
        }
    }

    fn follow_previews(&mut self) {
        for i in 0..self.tabs.len() {
            let Some(d) = self.tabs[i].get_mut::<Doc>() else { continue };
            if d.asset.preview == d.preview_seen {
                continue;
            }
            let before = std::mem::replace(&mut d.preview_seen, d.asset.preview.clone());
            if d.id.ty != AssetType::Material {
                continue;
            }
            self.ui.tabs += 1;
            // The mode alone, with no node previewed before or after, generates nothing different.
            if before.node.is_some() || d.asset.preview.node.is_some() {
                self.regenerate_from_memory(i);
            }
        }
    }

    fn material_before_close(&mut self, i: usize) {
        self.domains.material_mut().tabs_closed = true;
        for id in self.memory_regenerated(|j| j == i) {
            self.regenerate_from_disk(&id);
        }
    }

    fn material_rescan(&mut self) {
        if self.domains.material().loaded != self.project_generation {
            let (registry, project_diags) = match &self.project {
                Some(p) => project_registry(p),
                None => (shader_stdlib::registry(), Vec::new()),
            };
            let generation = self.project_generation;
            let m = self.domains.material_mut();
            m.registry = registry;
            m.project_diags = project_diags;
            m.disk = AssetSet::default();
            m.functions.clear();
            m.diags.clear();
            m.regen_only.clear();
            m.wesl_lines.clear();
            m.validated = None;
            m.loaded = generation;
        }
        let Some(project) = &self.project else { return };
        self.domains.material_mut().disk = AssetSet::load_all(project);
        self.refresh_functions();
        self.domains.material_mut().validated = None;
    }
}
