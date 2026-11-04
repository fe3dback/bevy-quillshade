use super::consts_ops::ref_values;
use super::*;
use crate::features_doc::FeaturesDoc;
use project_core::AssetUuid;
use shader_graph_core::features::variant_pin_id;
use shader_graph_core::registry::feature_enum_refs;

/// `(enum_id, enum_name, variant_id, variant_name)` of a deleted feature variant.
pub type RemovedVariant = (AssetUuid, String, AssetUuid, String);

impl FeaturesDoc {
    pub(crate) fn apply_edit(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        match action {
            TabAction::Undo => ctx.defer(FeaturesAction::History { undo: true, confirmed: false }),
            TabAction::Redo => ctx.defer(FeaturesAction::History { undo: false, confirmed: false }),
            TabAction::CommitInline => {
                self.end_merge();
                ctx.redraw_props = true;
            }
            TabAction::SetValue { key, value, merge } => {
                let Some(target) = key.downcast::<FeaturesProp>() else { return Handled::No };
                self.set_prop(ctx, target, value, *merge);
            }
            TabAction::Doc(p) => {
                let Some(a) = p.downcast::<FeaturesAction>() else { return Handled::No };
                ctx.defer(a.clone());
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    pub(crate) fn prop_of(&self, target: &FeaturesProp) -> Option<PropValue> {
        match target {
            FeaturesProp::EnumName(enum_id) => Some(PropValue::Str(self.table.get_by_id(*enum_id)?.name.clone())),
            FeaturesProp::VariantName(enum_id, variant_id) => {
                let e = self.table.get_by_id(*enum_id)?;
                Some(PropValue::Str(e.get_variant_by_id(*variant_id)?.name.clone()))
            }
            FeaturesProp::DraftEnumName => Some(PropValue::Str(self.draft_enum_name.clone())),
            FeaturesProp::DraftVariantName(enum_id) => {
                Some(PropValue::Str(self.draft_variant_names.get(enum_id).cloned().unwrap_or_default()))
            }
        }
    }

    fn set_prop(&mut self, ctx: &mut DocCtx, target: &FeaturesProp, value: &PropValue, merge: bool) {
        if self.prop_of(target).as_ref() == Some(value) {
            return;
        }
        match target {
            FeaturesProp::EnumName(enum_id) => {
                self.set_enum_name(*enum_id, value.as_str(), merge);
            }
            FeaturesProp::VariantName(enum_id, variant_id) => {
                self.set_variant_name(*enum_id, *variant_id, value.as_str(), merge);
            }
            FeaturesProp::DraftEnumName => {
                self.draft_enum_name = value.as_str().to_string();
            }
            FeaturesProp::DraftVariantName(enum_id) => {
                self.draft_variant_names.insert(*enum_id, value.as_str().to_string());
            }
        }
        let _ = ctx;
    }
}

/// Game Features tables: edits that check the graphs reading them.
pub trait MaterialFeatures {
    fn features_action(&mut self, action: FeaturesAction);

    fn breaking_feature_edit(
        &mut self,
        removed: (Vec<(AssetUuid, String)>, Vec<RemovedVariant>),
        confirmed: bool,
        what: &str,
        ok: &str,
        again: Action,
        apply: impl FnOnce(&mut FeaturesDoc),
    );

    fn feature_enum_usages(&self, table: &str, enum_ids: &[AssetUuid]) -> Vec<(AssetId, usize)>;

    fn feature_variant_usages(&self, table: &str, variants: &[RemovedVariant]) -> Vec<(AssetId, usize)>;

    fn clear_feature_enum_usages(&mut self, table: &str, enums: &[(AssetUuid, String)]);

    fn clear_feature_variant_usages(&mut self, table: &str, variants: &[RemovedVariant]);
}

impl MaterialFeatures for Editor {
    fn features_action(&mut self, action: FeaturesAction) {
        let Some(f) = self.features_doc() else { return };
        match action {
            FeaturesAction::AddEnum => {
                let name = f.draft_enum_name.trim().to_string();
                let problem = if name.is_empty() {
                    Some("enum name must not be empty".into())
                } else if let Err(e) = project_core::ident::check_ident(&name) {
                    Some(e)
                } else if f.table.enums.iter().any(|e| e.name.trim() == name) {
                    Some(format!("`{name}` is already in this table"))
                } else {
                    None
                };
                let Some(f) = self.features_doc_mut() else { return };
                match problem {
                    Some(p) => {
                        f.form_error = Some(p);
                        f.relayout();
                    }
                    None => {
                        f.add_enum(&name);
                        f.reset_enum_form();
                    }
                }
            }
            FeaturesAction::DeleteEnum { enum_id, confirmed } => {
                let Some(e) = f.table.get_by_id(enum_id) else { return };
                let name = e.name.clone();
                let what = format!("Deleting enum `{name}`");
                let again = FeaturesAction::DeleteEnum { enum_id, confirmed: true }.into();
                self.breaking_feature_edit(
                    (vec![(enum_id, name)], Vec::new()),
                    confirmed,
                    &what,
                    "Delete",
                    again,
                    move |f| {
                        f.remove_enum(enum_id);
                    },
                );
            }
            FeaturesAction::AddVariant { enum_id } => {
                let name = f.draft_variant_names.get(&enum_id).map(|s| s.trim().to_string()).unwrap_or_default();
                let Some(e) = f.table.get_by_id(enum_id) else { return };
                let problem = if name.is_empty() {
                    Some("variant name must not be empty".into())
                } else if let Err(err) = project_core::ident::check_ident(&name) {
                    Some(err)
                } else if e.variants.iter().any(|v| v.name.trim() == name) {
                    Some(format!("variant `{name}` is already in `{}`", e.name))
                } else {
                    None
                };
                let Some(f) = self.features_doc_mut() else { return };
                match problem {
                    Some(p) => {
                        f.form_error = Some(p);
                        f.relayout();
                    }
                    None => {
                        f.add_variant(enum_id, &name);
                    }
                }
            }
            FeaturesAction::DeleteVariant { enum_id, variant_id, confirmed } => {
                let Some(e) = f.table.get_by_id(enum_id) else { return };
                if e.variants.len() <= 2 {
                    return;
                }
                let Some(v) = e.get_variant_by_id(variant_id) else { return };
                let (enum_name, var_name) = (e.name.clone(), v.name.clone());
                let what = format!("Deleting variant `{enum_name}::{var_name}`");
                let again = FeaturesAction::DeleteVariant { enum_id, variant_id, confirmed: true }.into();
                self.breaking_feature_edit(
                    (Vec::new(), vec![(enum_id, enum_name, variant_id, var_name)]),
                    confirmed,
                    &what,
                    "Delete",
                    again,
                    move |f| {
                        f.remove_variant(enum_id, variant_id);
                    },
                );
            }
            FeaturesAction::MoveVariant { enum_id, variant_id, dir } => {
                if let Some(f) = self.features_doc_mut() {
                    f.move_variant(enum_id, variant_id, dir);
                }
            }
            FeaturesAction::History { undo, confirmed } => {
                let target = if undo { f.peek_undo() } else { f.peek_redo() };
                let Some(target) = target else { return };
                let removed: Vec<(AssetUuid, String)> = f
                    .table
                    .enums
                    .iter()
                    .filter(|e| target.get_by_id(e.id).is_none())
                    .map(|e| (e.id, e.name.clone()))
                    .collect();
                let mut removed_variants: Vec<RemovedVariant> = Vec::new();
                for e in &f.table.enums {
                    if let Some(te) = target.get_by_id(e.id) {
                        for v in &e.variants {
                            if te.get_variant_by_id(v.id).is_none() {
                                removed_variants.push((e.id, e.name.clone(), v.id, v.name.clone()));
                            }
                        }
                    }
                }
                let what = if undo { "Undo" } else { "Redo" };
                let again = FeaturesAction::History { undo, confirmed: true }.into();
                self.breaking_feature_edit((removed, removed_variants), confirmed, what, what, again, |f| {
                    if undo {
                        f.undo();
                    } else {
                        f.redo();
                    }
                });
            }
        }
        if let Some((path, table)) = self.features_doc().map(|f| (f.id.path.clone(), f.table.clone())) {
            self.domains.material_mut().features.insert(path, table);
        }
        self.touch_all();
        self.ui.props += 1;
    }

    fn breaking_feature_edit(
        &mut self,
        (enums, variants): (Vec<(AssetUuid, String)>, Vec<RemovedVariant>),
        confirmed: bool,
        what: &str,
        ok: &str,
        again: Action,
        apply: impl FnOnce(&mut FeaturesDoc),
    ) {
        self.dialog = None;
        let Some(table) = self.features_doc().map(|f| f.id.path.clone()) else { return };
        let ids: Vec<AssetUuid> = enums.iter().map(|(id, _)| *id).collect();
        let names: Vec<String> = enums
            .iter()
            .map(|(_, n)| n.clone())
            .chain(variants.iter().map(|(_, en, _, vn)| format!("{en}::{vn}")))
            .collect();
        let mut usage_map: BTreeMap<AssetId, usize> = BTreeMap::new();
        for (id, n) in self.feature_enum_usages(&table, &ids) {
            *usage_map.entry(id).or_default() += n;
        }
        for (id, n) in self.feature_variant_usages(&table, &variants) {
            *usage_map.entry(id).or_default() += n;
        }
        let usages: Vec<(AssetId, usize)> = usage_map.into_iter().collect();
        if !usages.is_empty() && !confirmed {
            let title = if !enums.is_empty() { "Feature enum in use" } else { "Feature variant in use" };
            self.dialog = Some(Dialog::Confirm {
                title: title.into(),
                message: usage_message(what, &names, &usages),
                buttons: vec![("Cancel".into(), Action::CloseDialog), (ok.into(), again)],
            });
            if let Some(f) = self.features_doc_mut() {
                f.relayout();
            }
            return;
        }
        if let Some(f) = self.features_doc_mut() {
            apply(f);
        }
        if !usages.is_empty() {
            if let Some(i) = self.active
                && !self.save(i)
            {
                return;
            }
            if !enums.is_empty() {
                self.clear_feature_enum_usages(&table, &enums);
            }
            if !variants.is_empty() {
                self.clear_feature_variant_usages(&table, &variants);
            }
        }
    }

    fn feature_enum_usages(&self, table: &str, enum_ids: &[AssetUuid]) -> Vec<(AssetId, usize)> {
        let mut out = Vec::new();
        if enum_ids.is_empty() {
            return out;
        }
        let table_names = ref_values(&self.asset_index, AssetType::Features, table);
        let enum_strs: Vec<String> = enum_ids.iter().map(|id| id.to_string()).collect();
        for (id, a) in &self.overridden_set().assets {
            let Ok(a) = a else { continue };
            let n = feature_enum_refs(a, &self.domains.material().registry)
                .iter()
                .filter(|r| table_names.contains(&r.2) && enum_strs.contains(&r.3))
                .count();
            if n > 0 {
                out.push((id.clone(), n));
            }
        }
        out
    }

    fn feature_variant_usages(&self, table: &str, variants: &[RemovedVariant]) -> Vec<(AssetId, usize)> {
        let mut out = Vec::new();
        if variants.is_empty() {
            return out;
        }
        let table_names = ref_values(&self.asset_index, AssetType::Features, table);
        for (id, a) in &self.overridden_set().assets {
            let Ok(a) = a else { continue };
            let refs = feature_enum_refs(a, &self.domains.material().registry);
            let mut n = 0usize;
            for (node, _, t, eid) in &refs {
                if !table_names.contains(t) {
                    continue;
                }
                let connected = variants.iter().any(|(enum_id, enum_name, var_id, _)| {
                    (eid == &enum_id.to_string() || eid == enum_name)
                        && a.edges.iter().any(|e| &e.to.0 == node && e.to.1 == variant_pin_id(*var_id))
                });
                if connected {
                    n += 1;
                }
            }
            if n > 0 {
                out.push((id.clone(), n));
            }
        }
        out
    }

    fn clear_feature_enum_usages(&mut self, table: &str, enums: &[(AssetUuid, String)]) {
        let Some(project) = self.project.clone() else { return };
        let registry = self.domains.material().registry.clone();
        let table_names = ref_values(&self.asset_index, AssetType::Features, table);
        let enum_strs: Vec<String> = enums.iter().flat_map(|(id, name)| [id.to_string(), name.clone()]).collect();
        let reads = |a: &Asset| {
            feature_enum_refs(a, &registry).iter().any(|r| table_names.contains(&r.2) && enum_strs.contains(&r.3))
        };
        let clear = |a: &mut Asset| {
            for tname in &table_names {
                ops::clear_feature_enum_refs(a, &registry, tname, &enum_strs);
            }
        };
        let mut cleared = Vec::new();
        for i in 0..self.tabs.len() {
            let Some(d) = self.tabs[i].get_mut::<Doc>() else { continue };
            if !reads(&d.asset) {
                continue;
            }
            let was_dirty = d.is_dirty();
            d.edit(clear);
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
            clear(&mut a);
            if let Err(e) = ops::save_annotated(&project, &id, &mut a, &self.asset_index) {
                return self.io_error(&format!("Cannot save {}", id.source_rel()), e);
            }
            cleared.push(id.source_rel());
        }
        let list = enums.iter().map(|(_, n)| format!("`{n}`")).collect::<Vec<_>>().join(", ");
        self.info(format!("Cleared references to {list} in {}", cleared.join(", ")));
        self.regenerate();
        self.touch_all();
    }

    fn clear_feature_variant_usages(&mut self, table: &str, variants: &[RemovedVariant]) {
        let Some(project) = self.project.clone() else { return };
        let registry = self.domains.material().registry.clone();
        let table_names = ref_values(&self.asset_index, AssetType::Features, table);
        let clear = |a: &mut Asset| -> bool {
            let mut changed = false;
            for (enum_id, enum_name, var_id, _) in variants {
                let enum_keys = [enum_id.to_string(), enum_name.clone()];
                let var_pins = [variant_pin_id(*var_id)];
                for tname in &table_names {
                    changed |= ops::clear_feature_variant_refs(a, &registry, tname, &enum_keys, &var_pins);
                }
            }
            changed
        };
        let mut cleared = Vec::new();
        for i in 0..self.tabs.len() {
            let Some(d) = self.tabs[i].get_mut::<Doc>() else { continue };
            let mut probe = d.asset.clone();
            if !clear(&mut probe) {
                continue;
            }
            let was_dirty = d.is_dirty();
            d.edit(|a| {
                clear(a);
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
            if !clear(&mut a) {
                continue;
            }
            if let Err(e) = ops::save_annotated(&project, &id, &mut a, &self.asset_index) {
                return self.io_error(&format!("Cannot save {}", id.source_rel()), e);
            }
            cleared.push(id.source_rel());
        }
        if !cleared.is_empty() {
            let list = variants.iter().map(|(_, en, _, vn)| format!("`{en}::{vn}`")).collect::<Vec<_>>().join(", ");
            self.info(format!("Cleared references to {list} in {}", cleared.join(", ")));
            self.regenerate();
            self.touch_all();
        }
    }
}
