use super::*;

/// Text of the confirmation before an edit breaking the readers of global constants `names`.
pub(super) fn usage_message(what: &str, names: &[String], usages: &[(AssetId, usize)]) -> String {
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

impl Editor {
    pub(super) fn consts_action(&mut self, action: ConstsAction) {
        let Some(c) = self.consts_doc() else { return };
        let id = c.id.clone();
        match action {
            ConstsAction::Add => {
                let (name, ty) = (c.draft_name.trim().to_string(), c.draft_ty);
                let problem = match check_ident(&name) {
                    Err(e) => Some(e),
                    Ok(()) if c.table.get(&name).is_some() => Some(format!("`{name}` is already in this table")),
                    Ok(()) => self
                        .global_consts()
                        .get(&name)
                        .filter(|(table, _)| *table != id)
                        .map(|(table, _)| format!("`{name}` is already defined in {table}")),
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
            ConstsAction::SetType { name, ty, confirmed } => {
                let Some(from) = c.table.get(&name).map(|k| k.ty) else { return };
                if from == ty {
                    return;
                }
                let what = format!("Changing `{name}` from {} to {}", from.label(), ty.label());
                let again = Action::Consts(ConstsAction::SetType { name: name.clone(), ty, confirmed: true });
                let names = self.owned_consts(&id, [name.clone()]);
                self.breaking_edit(names, confirmed, &what, "Change type", again, |c| c.set_type(&name, ty));
            }
            ConstsAction::Delete { name, confirmed } => {
                let what = format!("Deleting `{name}`");
                let again = Action::Consts(ConstsAction::Delete { name: name.clone(), confirmed: true });
                let names = self.owned_consts(&id, [name.clone()]);
                self.breaking_edit(names, confirmed, &what, "Delete", again, |c| c.remove(&name));
            }
            ConstsAction::History { undo, confirmed } => {
                let target = if undo { c.peek_undo() } else { c.peek_redo() };
                let Some(target) = target else { return };
                // Constants the step removes or retypes break their readers like a direct edit would.
                let changed: Vec<String> = c
                    .table
                    .consts
                    .iter()
                    .filter(|k| target.get(&k.name).is_none_or(|t| t.ty != k.ty))
                    .map(|k| k.name.clone())
                    .collect();
                let what = if undo { "Undo" } else { "Redo" };
                let again = Action::Consts(ConstsAction::History { undo, confirmed: true });
                let names = self.owned_consts(&id, changed);
                self.breaking_edit(names, confirmed, what, what, again, |c| {
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

    /// Those of `names` the global constant table `table` defines (a name repeated in a later table belongs to the
    /// first one, which nodes read).
    pub(super) fn owned_consts(&self, table: &AssetId, names: impl IntoIterator<Item = String>) -> Vec<String> {
        let all = self.global_consts();
        names.into_iter().filter(|n| all.get(n).is_some_and(|(t, _)| t == table)).collect()
    }

    /// Runs `apply` on the active table, an edit that breaks the readers of constants `names` (type change, delete,
    /// an undo/redo doing either). Without readers it just runs. With readers and not `confirmed` yet it opens a
    /// dialog listing them, whose OK runs `again`; once confirmed the edit runs, the table is saved and the readers'
    /// references are cleared (decision 0017).
    pub(super) fn breaking_edit(
        &mut self,
        names: Vec<String>,
        confirmed: bool,
        what: &str,
        ok: &str,
        again: Action,
        apply: impl FnOnce(&mut ConstsDoc),
    ) {
        self.dialog = None;
        let usages = self.const_usages(&names);
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
            self.clear_const_usages(&names);
        }
    }

    /// Graphs whose nodes read any of global constants `names` (open documents as edited, the rest as saved), with
    /// the number of such nodes.
    pub fn const_usages(&self, names: &[String]) -> Vec<(AssetId, usize)> {
        let mut out = Vec::new();
        if names.is_empty() {
            return out;
        }
        for (id, a) in &self.overridden_set().assets {
            let Ok(a) = a else { continue };
            let n = const_refs(a, &self.registry).iter().filter(|r| names.contains(&r.2)).count();
            if n > 0 {
                out.push((id.clone(), n));
            }
        }
        out
    }

    /// Clears every reference to constants `names` (decision 0017): open graphs are edited (and saved unless they
    /// had unsaved changes), closed ones are rewritten on disk; then the project regenerates.
    pub(super) fn clear_const_usages(&mut self, names: &[String]) {
        let Some(project) = self.project.clone() else { return };
        let registry = self.registry.clone();
        let reads = |a: &Asset| const_refs(a, &registry).iter().any(|r| names.contains(&r.2));
        let mut cleared = Vec::new();
        for i in 0..self.tabs.len() {
            let Some(d) = self.tabs[i].graph_mut() else { continue };
            if !reads(&d.asset) {
                continue;
            }
            let was_dirty = d.is_dirty();
            d.edit(|a| {
                for name in names {
                    ops::clear_const_refs(a, &registry, name);
                }
            });
            cleared.push(d.id.source_rel());
            if !was_dirty {
                self.save(i);
            }
        }
        let ids: Vec<AssetId> = self.disk.assets.keys().filter(|id| self.doc_index(id).is_none()).cloned().collect();
        for id in ids {
            let Ok(mut a) = assets::load(&project, &id) else { continue };
            if !reads(&a) {
                continue;
            }
            for name in names {
                ops::clear_const_refs(&mut a, &registry, name);
            }
            if let Err(e) = ops::save(&project, &id, &a) {
                return self.io_error(&format!("Cannot save {}", id.source_rel()), e);
            }
            cleared.push(id.source_rel());
        }
        let list = names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ");
        self.info(format!("Cleared references to {list} in {}", cleared.join(", ")));
        self.regenerate();
        self.touch_all();
    }
}
