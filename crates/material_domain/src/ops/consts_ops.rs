use super::*;

/// Text of the confirmation before an edit breaking the readers of global constants `names`.
pub(crate) fn usage_message(what: &str, names: &[String], usages: &[(AssetId, usize)]) -> String {
    let nodes: usize = usages.iter().map(|u| u.1).sum();
    let list: Vec<String> =
        usages.iter().map(|(id, n)| format!("{} ({})", id.source_rel(), plural(*n, "node", "nodes"))).collect();
    let consts = names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ");
    format!(
        "{what} breaks {} reading {consts} in:\n{}\nTheir references are cleared (they will output 1.0) and the \
         changes are saved.",
        plural(nodes, "node", "nodes"),
        list.join("\n")
    )
}

/// The constant table tab's edits (decision 0017, design §6.2): the body of [`TabDoc::apply`] /
/// [`TabDoc::current_value`] of [`ConstsDoc`]. Every [`ConstsAction`] checks the project's other tables or the graphs
/// reading the table (and may ask first), so it goes to the editor ([`Editor::consts_action`]).
impl ConstsDoc {
    pub(crate) fn apply_edit(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        match action {
            TabAction::Undo => ctx.defer(ConstsAction::History { undo: true, confirmed: false }),
            TabAction::Redo => ctx.defer(ConstsAction::History { undo: false, confirmed: false }),
            TabAction::CommitInline => {
                self.end_merge();
                ctx.redraw_props = true;
            }
            TabAction::SetValue { key, value, merge } => {
                let Some(target) = key.downcast::<ConstsProp>() else { return Handled::No };
                self.set_prop(ctx, target, value, *merge);
            }
            TabAction::SetComponent { key, comp, value, is_final } => {
                let Some(target) = key.downcast::<ConstsProp>() else { return Handled::No };
                // `None`: a field sent its shown (rounded) text back on focus loss, which is not an edit.
                let Some(new) = self.prop_of(target).and_then(|cur| cur.with_component(*comp, *value)) else {
                    return Handled::Yes;
                };
                self.set_prop(ctx, target, &new, true);
                if *is_final {
                    self.end_merge();
                }
            }
            TabAction::Doc(p) => {
                let Some(a) = p.downcast::<ConstsAction>() else { return Handled::No };
                ctx.defer(a.clone());
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    /// Value a property widget shows.
    pub(crate) fn prop_of(&self, target: &ConstsProp) -> Option<PropValue> {
        match target {
            ConstsProp::Name(id) => Some(PropValue::Str(self.table.get_by_id(*id)?.name.clone())),
            ConstsProp::Value(id) => Some(prop_value(&self.table.get_by_id(*id)?.value)),
            ConstsProp::Type(id) => Some(PropValue::Str(self.table.get_by_id(*id)?.ty.label().into())),
            ConstsProp::DraftName => Some(PropValue::Str(self.draft_name.clone())),
            ConstsProp::DraftType => Some(PropValue::Str(self.draft_ty.label().into())),
        }
    }

    /// Sets a property; `merge`d name or value edits of one constant are one undo step. None redraws the right
    /// sidebar: the table view keys on [`ConstsDoc::layout`].
    fn set_prop(&mut self, ctx: &mut DocCtx, target: &ConstsProp, value: &PropValue, merge: bool) {
        if self.prop_of(target).as_ref() == Some(value) {
            return;
        }
        match target {
            ConstsProp::Name(id) => self.set_name(*id, value.as_str(), merge),
            ConstsProp::Value(id) => self.set_value(*id, model_value(value), merge),
            ConstsProp::Type(id) => {
                if let Some(ty) = ConstType::from_label(value.as_str()) {
                    ctx.defer(ConstsAction::SetType { id: *id, ty, confirmed: false });
                }
            }
            ConstsProp::DraftName => self.draft_name = value.as_str().to_string(),
            ConstsProp::DraftType => {
                if let Some(ty) = ConstType::from_label(value.as_str()) {
                    self.draft_ty = ty;
                }
            }
        }
    }
}

/// Global constant tables (decision 0017): edits that check the graphs reading them.
pub trait MaterialConsts {
    fn consts_action(&mut self, action: ConstsAction);

    /// Runs `apply` on the active table, an edit that breaks the readers of `consts` (`(id, name)` pairs: type change,
    /// delete, an undo/redo doing either). Without readers it just runs. With readers and not `confirmed` yet it opens
    /// a dialog listing them, whose OK runs `again`; once confirmed the edit runs, the table is saved and the readers'
    /// references are cleared (decision 0017).
    fn breaking_edit(
        &mut self,
        consts: Vec<(u32, String)>,
        confirmed: bool,
        what: &str,
        ok: &str,
        again: Action,
        apply: impl FnOnce(&mut ConstsDoc),
    );

    /// Graphs whose nodes read any of constants `ids` of `table` (open documents as edited, the rest as saved), with
    /// the number of such nodes.
    fn const_usages(&self, table: &str, ids: &[u32]) -> Vec<(AssetId, usize)>;

    /// Clears every reference to constants `consts` (`(id, name)`) of `table` (decision 0017): open graphs are edited
    /// (and saved unless they had unsaved changes), closed ones are rewritten on disk; then the project regenerates.
    fn clear_const_usages(&mut self, table: &str, consts: &[(u32, String)]);
}

impl MaterialConsts for Editor {
    fn consts_action(&mut self, action: ConstsAction) {
        let Some(c) = self.consts_doc() else { return };
        match action {
            ConstsAction::Add => {
                let (name, ty) = (c.draft_name.trim().to_string(), c.draft_ty);
                let problem = if name.is_empty() {
                    Some("constant name must not be empty".into())
                } else if c.table.consts.iter().any(|k| k.name.trim() == name) {
                    Some(format!("`{name}` is already in this table"))
                } else {
                    None
                };
                let Some(c) = self.consts_doc_mut() else { return };
                match problem {
                    Some(p) => {
                        c.form_error = Some(p);
                        c.relayout();
                    }
                    None => {
                        c.add(&name, ty);
                        c.reset_form();
                    }
                }
            }
            ConstsAction::SetType { id: cid, ty, confirmed } => {
                let Some(k) = c.table.get_by_id(cid) else { return };
                let (from, name) = (k.ty, k.name.clone());
                if from == ty {
                    return;
                }
                let what = format!("Changing `{name}` from {} to {}", from.label(), ty.label());
                let again = ConstsAction::SetType { id: cid, ty, confirmed: true }.into();
                self.breaking_edit(vec![(cid, name)], confirmed, &what, "Change type", again, move |c| {
                    c.set_type(cid, ty);
                });
            }
            ConstsAction::Delete { id: cid, confirmed } => {
                let Some(k) = c.table.get_by_id(cid) else { return };
                let name = k.name.clone();
                let what = format!("Deleting `{name}`");
                let again = ConstsAction::Delete { id: cid, confirmed: true }.into();
                self.breaking_edit(vec![(cid, name)], confirmed, &what, "Delete", again, move |c| {
                    c.remove(cid);
                });
            }
            ConstsAction::History { undo, confirmed } => {
                let target = if undo { c.peek_undo() } else { c.peek_redo() };
                let Some(target) = target else { return };
                // Constants the step removes or retypes break their readers like a direct edit would.
                let changed: Vec<(u32, String)> = c
                    .table
                    .consts
                    .iter()
                    .filter(|k| target.get_by_id(k.id).is_none_or(|t| t.ty != k.ty))
                    .map(|k| (k.id, k.name.clone()))
                    .collect();
                let what = if undo { "Undo" } else { "Redo" };
                let again = ConstsAction::History { undo, confirmed: true }.into();
                self.breaking_edit(changed, confirmed, what, what, again, |c| {
                    if undo {
                        c.undo();
                    } else {
                        c.redo();
                    }
                });
            }
        }
        self.ui.props += 1;
    }

    fn breaking_edit(
        &mut self,
        consts: Vec<(u32, String)>,
        confirmed: bool,
        what: &str,
        ok: &str,
        again: Action,
        apply: impl FnOnce(&mut ConstsDoc),
    ) {
        self.dialog = None;
        let Some(table) = self.consts_doc().map(|c| c.id.path.clone()) else { return };
        let ids: Vec<u32> = consts.iter().map(|(id, _)| *id).collect();
        let names: Vec<String> = consts.iter().map(|(_, n)| n.clone()).collect();
        let usages = self.const_usages(&table, &ids);
        if !usages.is_empty() && !confirmed {
            self.dialog = Some(Dialog::Confirm {
                title: "Constant in use".into(),
                message: usage_message(what, &names, &usages),
                buttons: vec![("Cancel".into(), Action::CloseDialog), (ok.into(), again)],
            });
            // The view shows the old state again under the dialog (a select already shows the new type).
            if let Some(c) = self.consts_doc_mut() {
                c.relayout();
            }
            return;
        }
        if let Some(c) = self.consts_doc_mut() {
            apply(c);
        }
        if !usages.is_empty() {
            // Saved with the cleared references: the table and its readers never disagree on disk.
            if let Some(i) = self.active
                && !self.save(i)
            {
                return;
            }
            self.clear_const_usages(&table, &consts);
        }
    }

    fn const_usages(&self, table: &str, ids: &[u32]) -> Vec<(AssetId, usize)> {
        let mut out = Vec::new();
        if ids.is_empty() {
            return out;
        }
        for (id, a) in &self.overridden_set().assets {
            let Ok(a) = a else { continue };
            let n = const_refs(a, &self.domains.material().registry)
                .iter()
                .filter(|r| r.2 == table && ids.contains(&r.3))
                .count();
            if n > 0 {
                out.push((id.clone(), n));
            }
        }
        out
    }

    fn clear_const_usages(&mut self, table: &str, consts: &[(u32, String)]) {
        let Some(project) = self.project.clone() else { return };
        let registry = self.domains.material().registry.clone();
        let ids: Vec<u32> = consts.iter().map(|(id, _)| *id).collect();
        let reads = |a: &Asset| const_refs(a, &registry).iter().any(|r| r.2 == table && ids.contains(&r.3));
        let mut cleared = Vec::new();
        for i in 0..self.tabs.len() {
            let Some(d) = self.tabs[i].get_mut::<Doc>() else { continue };
            if !reads(&d.asset) {
                continue;
            }
            let was_dirty = d.is_dirty();
            d.edit(|a| {
                for &(cid, _) in consts {
                    ops::clear_const_refs(a, &registry, table, cid);
                }
            });
            cleared.push(d.id.source_rel());
            if !was_dirty {
                self.save(i);
            }
        }
        let asset_ids: Vec<AssetId> =
            self.domains.material().disk.assets.keys().filter(|id| self.doc_index(id).is_none()).cloned().collect();
        for id in asset_ids {
            let Ok(mut a) = assets::load(&project, &id) else { continue };
            if !reads(&a) {
                continue;
            }
            for &(cid, _) in consts {
                ops::clear_const_refs(&mut a, &registry, table, cid);
            }
            if let Err(e) = ops::save(&project, &id, &a) {
                return self.io_error(&format!("Cannot save {}", id.source_rel()), e);
            }
            cleared.push(id.source_rel());
        }
        let list = consts.iter().map(|(_, n)| format!("`{n}`")).collect::<Vec<_>>().join(", ");
        self.info(format!("Cleared references to {list} in {}", cleared.join(", ")));
        self.regenerate();
        self.touch_all();
    }
}
