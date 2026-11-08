//! `.enum.ron` schema document (`EnumDoc`): in-memory [`EnumAsset`], undo/redo [`History`], selected variant and
//! selected payload field, add-variant / add-field draft state, and [`TabDoc`] implementation.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};

use project_core::asset_uuid::AssetUuid;
use project_core::ident::check_ident;
use project_core::{AssetRef, Project};

use editor_core::action::{Action, Handled, TabAction};
use editor_core::doc_kind::{DocCtx, StatusCtx, StatusInfo, TabDoc};
use editor_core::dynval::{PropKey, PropTarget, PropValue};
use editor_core::history::{History, MergeKey};
use editor_core::tab::ViewKindId;
use editor_core::ui::icons::IconRef;

use crate::backend::codegen::emit_enum_file_with_schemas;
use crate::backend::model::{
    DataValue, DtoAsset, EnumAsset, EnumVariant, Field, FieldSettings, FieldType, GameAssetKind,
    ensure_nested_value_with_schemas_mut, enum_ref,
};
use crate::backend::ron_io::{load_enum, save_enum_with_index};

use super::ENUM_ICON;
use super::dto_doc::ConstraintField;
use super::state::{DataState, DataStep};
use super::widgets::{
    apply_container_add, apply_container_move, apply_container_remove, apply_inner_target_change,
    apply_inner_type_change, apply_key_target_change, apply_key_type_change, apply_set_component,
    apply_set_option_some, apply_set_value, coerce_default_for_type_change, data_value_to_prop, parse_type_tag_prop,
    resolve_primary_field_type, switch_enum_variant_with_drafts,
};

pub const ENUM_VIEW: ViewKindId = ViewKindId("data.enum");

fn next_layout() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Property keys for widgets inside the `.enum.ron` editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnumProp {
    EnumDoc,
    DraftVariantName,
    VariantName(AssetUuid),
    VariantDoc(AssetUuid),
    DraftFieldName,
    DraftFieldType,
    DraftFieldDtoTarget,
    DraftFieldEnumTarget,
    DraftFieldGameAssetKind,
    DraftFieldInnerType,
    DraftFieldInnerTarget,
    DraftFieldKeyType,
    DraftFieldKeyTarget,
    DraftFieldIsName,
    FieldName(AssetUuid),
    FieldType(AssetUuid),
    FieldDtoTarget(AssetUuid),
    FieldEnumTarget(AssetUuid),
    FieldGameAssetKind(AssetUuid),
    FieldInnerType(AssetUuid),
    FieldInnerTarget(AssetUuid),
    FieldKeyType(AssetUuid),
    FieldKeyTarget(AssetUuid),
    FieldIsName(AssetUuid),
    FieldDefault(AssetUuid),
    NestedFieldDefault(Vec<AssetUuid>),
    FieldDoc(AssetUuid),
    FieldMinEnabled(AssetUuid),
    FieldMin(AssetUuid),
    FieldMaxEnabled(AssetUuid),
    FieldMax(AssetUuid),
    FieldStepEnabled(AssetUuid),
    FieldStep(AssetUuid),
    FieldSlider(AssetUuid),
    FieldMultiline(AssetUuid),
    FieldMaxLenEnabled(AssetUuid),
    FieldMaxLen(AssetUuid),
    FieldRequired(AssetUuid),
}

impl PropTarget for EnumProp {}

/// Document-level actions for [`EnumDoc`] (`TabAction::Doc`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnumAction {
    /// Adds a variant using the draft variant bar (auto-generating a unique name if `draft_variant_name` is empty).
    AddVariant,
    /// Selects a variant row.
    SelectVariant(Option<AssetUuid>),
    /// Moves a variant up (`delta = -1`) or down (`delta = 1`).
    MoveVariant { id: AssetUuid, delta: isize },
    /// Deletes a variant from the enum schema.
    DeleteVariant(AssetUuid),
    /// Adds a payload field to the selected variant using the draft field bar.
    AddField,
    /// Selects a payload field row inside the selected variant.
    SelectField(Option<AssetUuid>),
    /// Toggles the inline `Constraints` (`FieldSettings`) subpane right under payload field `id`.
    ToggleConstraints(AssetUuid),
    /// Toggles the inline `FieldType::Dto` or `FieldType::Enum` default value sub-form at `path` (`vec![field_id, ...]`).
    ToggleNestedDefault(Vec<AssetUuid>),
    /// Sets an `Option<T>` constraint (`min`, `max`, `step`, `max_len`) to `None` (`some = false`) or `Some` (`some = true`).
    SetConstraintOption { id: AssetUuid, field: ConstraintField, some: bool },
    /// Resets a single constraint field in `FieldSettings` back to its default.
    ResetConstraint { id: AssetUuid, field: ConstraintField },
    /// Moves a payload field up (`delta = -1`) or down (`delta = 1`) in the selected variant.
    MoveField { id: AssetUuid, delta: isize },
    /// Deletes a payload field from its variant.
    DeleteField(AssetUuid),
    /// Resets a payload field's default value to its type default.
    ResetFieldDefault(AssetUuid),
    /// Resets a nested default value at `path` (`vec![field_id, ...]`).
    ResetNestedDefault(Vec<AssetUuid>),
    /// Sets an `Option<T>` value at `path` to `None` (`some = false`) or `Some(default)` (`some = true`).
    SetOptionSome { path: Vec<AssetUuid>, some: bool },
    /// Appends a default element/entry to a container (`Array`, `Set`, `Map`) at `path`.
    ContainerAdd { path: Vec<AssetUuid> },
    /// Removes the element/entry at `index` from a container (`Array`, `Set`, `Map`) at `path`.
    ContainerRemove { path: Vec<AssetUuid>, index: usize },
    /// Reorders the element at `index` by `delta` (`-1` or `+1`) in an `Array` at `path`.
    ContainerMove { path: Vec<AssetUuid>, index: usize, delta: isize },
}

impl From<EnumAction> for Action {
    fn from(a: EnumAction) -> Self {
        Action::Tab(TabAction::doc(a))
    }
}

/// Open `.enum.ron` document tab.
#[derive(Debug, Clone)]
pub struct EnumDoc {
    pub path: String,
    pub aref: AssetRef,
    pub en: EnumAsset,
    pub saved: EnumAsset,
    pub history: History<EnumAsset>,
    pub selected_variant: Option<AssetUuid>,
    pub selected_field: Option<AssetUuid>,
    pub expanded_constraints: Option<AssetUuid>,
    pub expanded_nested: BTreeSet<Vec<AssetUuid>>,
    pub cached_dtos: BTreeMap<String, DtoAsset>,
    pub cached_enums: BTreeMap<String, EnumAsset>,
    pub variant_drafts: BTreeMap<Vec<AssetUuid>, BTreeMap<AssetUuid, BTreeMap<AssetUuid, DataValue>>>,
    pub draft_variant_name: String,
    pub draft_field_name: String,
    pub draft_field_ty: FieldType,
    pub draft_field_is_name: bool,
    pub variant_error: Option<String>,
    pub field_error: Option<String>,
    pub revision: u64,
    pub layout: u64,
    pub constraints_layout: u64,
}

impl EnumDoc {
    pub fn new(path: impl Into<String>, en: EnumAsset) -> Self {
        Self::with_schemas(path, en, BTreeMap::new(), BTreeMap::new())
    }

    pub fn with_schemas(
        path: impl Into<String>,
        mut en: EnumAsset,
        cached_dtos: BTreeMap<String, DtoAsset>,
        mut cached_enums: BTreeMap<String, EnumAsset>,
    ) -> Self {
        en.ensure_id();
        let path = path.into();
        if !en.id.is_nil() {
            cached_enums.insert(path.clone(), en.clone());
        }
        let selected_variant = en.variants.first().map(|v| v.id);
        let selected_field = en.variants.first().and_then(|v| v.fields.first()).map(|f| f.id);
        let saved = en.clone();
        Self {
            aref: enum_ref(&path),
            path,
            en,
            saved,
            history: History::default(),
            selected_variant,
            selected_field,
            expanded_constraints: None,
            expanded_nested: BTreeSet::new(),
            cached_dtos,
            cached_enums,
            variant_drafts: BTreeMap::new(),
            draft_variant_name: String::new(),
            draft_field_name: String::new(),
            draft_field_ty: FieldType::I32,
            draft_field_is_name: false,
            variant_error: None,
            field_error: None,
            revision: 1,
            layout: next_layout(),
            constraints_layout: next_layout(),
        }
    }

    pub fn sync_cached_schemas(&mut self, ctx: &mut DocCtx) {
        if let Some(st) = ctx.domain::<DataState>() {
            if !st.assets.dtos.is_empty() {
                self.cached_dtos = st.assets.dtos.clone();
            }
            if !st.assets.enums.is_empty() {
                self.cached_enums = st.assets.enums.clone();
            }
        }
        if !self.en.id.is_nil() {
            self.cached_enums.insert(self.path.clone(), self.en.clone());
        }
    }

    pub fn first_dto_id(&self) -> AssetUuid {
        self.cached_dtos.values().find(|d| !d.id.is_nil()).map(|d| d.id).unwrap_or(AssetUuid::NIL)
    }

    pub fn first_non_self_enum_id(&self) -> AssetUuid {
        self.cached_enums
            .values()
            .find(|e| !e.id.is_nil() && e.id != self.en.id)
            .map(|e| e.id)
            .unwrap_or(AssetUuid::NIL)
    }

    pub fn relayout(&mut self) {
        self.layout = next_layout();
        self.constraints_layout = next_layout();
    }

    pub fn relayout_constraints(&mut self) {
        self.constraints_layout = next_layout();
    }

    pub fn bump(&mut self, relayout: bool) {
        self.revision = self.revision.wrapping_add(1);
        if relayout {
            self.relayout();
        }
    }

    pub fn active_variant(&self) -> Option<&EnumVariant> {
        let vid = self.selected_variant.or_else(|| self.en.variants.first().map(|v| v.id))?;
        self.en.variant_by_id(vid)
    }

    pub fn next_unique_variant_name(&self, base: &str) -> String {
        let base = if base.trim().is_empty() { "new_variant" } else { base.trim() };
        if !self.en.variants.iter().any(|v| v.name == base) {
            return base.to_string();
        }
        let mut i = 2usize;
        loop {
            let candidate = format!("{base}_{i}");
            if !self.en.variants.iter().any(|v| v.name == candidate) {
                return candidate;
            }
            i += 1;
        }
    }

    pub fn next_unique_field_name(&self, variant_id: AssetUuid, base: &str) -> String {
        let base = if base.trim().is_empty() { "new_field" } else { base.trim() };
        let Some(var) = self.en.variant_by_id(variant_id) else {
            return base.to_string();
        };
        if !var.fields.iter().any(|f| f.name == base) {
            return base.to_string();
        }
        let mut i = 2usize;
        loop {
            let candidate = format!("{base}_{i}");
            if !var.fields.iter().any(|f| f.name == candidate) {
                return candidate;
            }
            i += 1;
        }
    }

    /// Programmatically adds a new variant with the given `name`, recording an undo step.
    pub fn add_variant_named(&mut self, name: &str) -> Result<AssetUuid, String> {
        let trimmed = name.trim();
        let final_name = if trimmed.is_empty() {
            self.next_unique_variant_name("new_variant")
        } else {
            if let Err(e) = check_ident(trimmed) {
                return Err(format!("Invalid variant name '{trimmed}': {e}"));
            }
            if self.en.variants.iter().any(|v| v.name == trimmed) {
                return Err(format!("Variant '{trimmed}' already exists in this Enum"));
            }
            trimmed.to_string()
        };
        let before = self.en.clone();
        let variant = EnumVariant::new(final_name);
        let vid = variant.id;
        self.en.variants.push(variant);
        self.history.record(Some(before), None);
        self.selected_variant = Some(vid);
        self.selected_field = None;
        self.variant_error = None;
        self.bump(true);
        Ok(vid)
    }

    /// Moves variant `id` by `delta` (`-1` up, `+1` down).
    pub fn move_variant(&mut self, id: AssetUuid, delta: isize) -> bool {
        let Some(idx) = self.en.variants.iter().position(|v| v.id == id) else {
            return false;
        };
        let next = idx as isize + delta;
        if next < 0 || next as usize >= self.en.variants.len() {
            return false;
        }
        let before = self.en.clone();
        self.en.variants.swap(idx, next as usize);
        self.history.record(Some(before), None);
        self.bump(true);
        true
    }

    /// Deletes variant `id`.
    pub fn delete_variant(&mut self, id: AssetUuid) -> bool {
        let Some(idx) = self.en.variants.iter().position(|v| v.id == id) else {
            return false;
        };
        let removed_field_ids: BTreeSet<AssetUuid> = self.en.variants[idx].fields.iter().map(|f| f.id).collect();
        let before = self.en.clone();
        self.en.variants.remove(idx);
        self.history.record(Some(before), None);
        if self.selected_variant == Some(id) {
            self.selected_variant = self.en.variants.get(idx).or_else(|| self.en.variants.last()).map(|v| v.id);
            self.selected_field = self
                .selected_variant
                .and_then(|vid| self.en.variant_by_id(vid))
                .and_then(|v| v.fields.first())
                .map(|f| f.id);
        }
        if self.expanded_constraints.is_some_and(|fid| removed_field_ids.contains(&fid)) {
            self.expanded_constraints = None;
        }
        self.expanded_nested.retain(|p| p.first().is_none_or(|fid| !removed_field_ids.contains(fid)));
        self.variant_drafts.retain(|p, _| p.first().is_none_or(|fid| !removed_field_ids.contains(fid)));
        self.bump(true);
        true
    }

    /// Programmatically adds a payload field to `variant_id` (or `selected_variant`), recording an undo step.
    pub fn add_field_named(
        &mut self,
        variant_id: Option<AssetUuid>,
        name: &str,
        ty: FieldType,
        is_name: bool,
    ) -> Result<AssetUuid, String> {
        let vid = variant_id
            .or(self.selected_variant)
            .or_else(|| self.en.variants.first().map(|v| v.id))
            .ok_or_else(|| "Select or add a variant first".to_string())?;
        let trimmed = name.trim();
        let final_name = if trimmed.is_empty() {
            self.next_unique_field_name(vid, "new_field")
        } else {
            if let Err(e) = check_ident(trimmed) {
                return Err(format!("Invalid field name '{trimmed}': {e}"));
            }
            let var = self.en.variant_by_id(vid).ok_or_else(|| "Selected variant not found".to_string())?;
            if var.fields.iter().any(|f| f.name == trimmed) {
                return Err(format!("Field '{trimmed}' already exists in variant '{}'", var.name));
            }
            trimmed.to_string()
        };
        let default = ty.default_value_with_schemas(&self.cached_dtos, &self.cached_enums);
        let Some(var_idx) = self.en.variants.iter().position(|v| v.id == vid) else {
            return Err("Selected variant not found".to_string());
        };
        let before = self.en.clone();
        let id = AssetUuid::new_v7();
        let field = Field {
            id,
            name: final_name,
            ty,
            category: String::new(),
            is_name,
            default,
            settings: FieldSettings::default(),
        };
        self.en.variants[var_idx].fields.push(field);
        self.history.record(Some(before), None);
        self.selected_variant = Some(vid);
        self.selected_field = Some(id);
        self.field_error = None;
        self.bump(true);
        Ok(id)
    }

    fn find_field_indices(&self, field_id: AssetUuid) -> Option<(usize, usize)> {
        for (vi, v) in self.en.variants.iter().enumerate() {
            if let Some(fi) = v.fields.iter().position(|f| f.id == field_id) {
                return Some((vi, fi));
            }
        }
        None
    }

    pub fn move_field(&mut self, id: AssetUuid, delta: isize) -> bool {
        let Some((vi, fi)) = self.find_field_indices(id) else {
            return false;
        };
        let next = fi as isize + delta;
        if next < 0 || next as usize >= self.en.variants[vi].fields.len() {
            return false;
        }
        let before = self.en.clone();
        self.en.variants[vi].fields.swap(fi, next as usize);
        self.history.record(Some(before), None);
        self.bump(true);
        true
    }

    pub fn delete_field(&mut self, id: AssetUuid) -> bool {
        let Some((vi, fi)) = self.find_field_indices(id) else {
            return false;
        };
        let before = self.en.clone();
        self.en.variants[vi].fields.remove(fi);
        self.history.record(Some(before), None);
        if self.selected_field == Some(id) {
            self.selected_field =
                self.en.variants[vi].fields.get(fi).or_else(|| self.en.variants[vi].fields.last()).map(|f| f.id);
        }
        if self.expanded_constraints == Some(id) {
            self.expanded_constraints = None;
        }
        self.expanded_nested.retain(|p| p.first() != Some(&id));
        self.variant_drafts.retain(|p, _| p.first() != Some(&id));
        self.bump(true);
        true
    }

    pub fn field_default_meta_at_path(
        &self,
        path: &[AssetUuid],
    ) -> Option<(FieldType, FieldSettings, DataValue, bool)> {
        let first = *path.first()?;
        let (vi, _) = self.find_field_indices(first)?;
        let vid = self.en.variants[vi].id;
        self.en.variant_field_meta_at_path(vid, path, &self.cached_dtos, &self.cached_enums, true)
    }

    pub fn get_default_at_path<'a>(&'a self, path: &[AssetUuid], fallback: &'a DataValue) -> &'a DataValue {
        let Some(&first) = path.first() else {
            return fallback;
        };
        let Some((vi, fi)) = self.find_field_indices(first) else {
            return fallback;
        };
        let f0 = &self.en.variants[vi].fields[fi];
        if path.len() == 1 {
            return &f0.default;
        }
        f0.default.get_at_path(&path[1..]).unwrap_or(fallback)
    }

    pub fn get_default_mut_at_path(&mut self, path: &[AssetUuid]) -> Option<&mut DataValue> {
        self.field_default_meta_at_path(path)?;
        let first = *path.first()?;
        let (vi, fi) = self.find_field_indices(first)?;
        if path.len() == 1 {
            return Some(&mut self.en.variants[vi].fields[fi].default);
        }
        let ty = self.en.variants[vi].fields[fi].ty.clone();
        ensure_nested_value_with_schemas_mut(
            &mut self.en.variants[vi].fields[fi].default,
            ty,
            &self.cached_dtos,
            &self.cached_enums,
            &path[1..],
        )
    }

    fn clamp_field_default(field: &mut Field) {
        let Some(pv) = data_value_to_prop(&field.ty, &field.default) else {
            return;
        };
        let settings = field.settings.clone();
        if field.ty.is_numeric() {
            for (comp, &v) in pv.floats().iter().enumerate() {
                let _ = apply_set_component(&field.ty, &mut field.default, comp, v, &settings);
            }
        } else if field.ty.is_string() {
            let _ = apply_set_value(&field.ty, &mut field.default, &pv, &settings);
        }
    }

    fn reset_default_at_path(&mut self, ctx: &mut DocCtx, path: &[AssetUuid]) -> Handled {
        let Some((_, _, schema_default, _)) = self.field_default_meta_at_path(path) else {
            return Handled::No;
        };
        if self.get_default_at_path(path, &schema_default) != &schema_default {
            let before = self.en.clone();
            if let Some(slot) = self.get_default_mut_at_path(path) {
                *slot = schema_default;
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
            }
        }
        Handled::Yes
    }

    fn set_default_value_at_path(
        &mut self,
        ctx: &mut DocCtx,
        prop: &EnumProp,
        path: &[AssetUuid],
        value: &PropValue,
        merge: bool,
    ) -> Handled {
        let Some((ty, settings, _schema_default, _)) = self.field_default_meta_at_path(path) else {
            return Handled::No;
        };
        if let FieldType::Enum(enum_id) = &ty
            && let PropValue::Str(s) = value
        {
            let next_var = if s.trim().is_empty() {
                AssetUuid::NIL
            } else if let Some(id) = AssetUuid::parse(s.trim()) {
                id
            } else {
                return Handled::No;
            };
            let target_enum = self.cached_enums.values().find(|e| e.id == *enum_id).cloned();
            let dtos = self.cached_dtos.clone();
            let enums = self.cached_enums.clone();
            let mut drafts = self.variant_drafts.get(path).cloned().unwrap_or_default();
            let before = self.en.clone();
            let Some(slot) = self.get_default_mut_at_path(path) else {
                return Handled::No;
            };
            if switch_enum_variant_with_drafts(slot, target_enum.as_ref(), next_var, &dtos, &enums, &mut drafts) {
                self.variant_drafts.insert(path.to_vec(), drafts);
                let mkey = merge.then(|| MergeKey::from(PropKey::new(prop.clone())));
                self.history.record(Some(before), mkey);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
            }
            return Handled::Yes;
        }
        let before = self.en.clone();
        let Some(slot) = self.get_default_mut_at_path(path) else {
            return Handled::No;
        };
        if apply_set_value(&ty, slot, value, &settings) {
            let mkey = merge.then(|| MergeKey::from(PropKey::new(prop.clone())));
            self.history.record(Some(before), mkey);
            self.bump(ty.is_asset_ref() || ty.is_soft_ref() || ty.is_enum());
            ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
        }
        Handled::Yes
    }

    fn set_default_component_at_path(
        &mut self,
        ctx: &mut DocCtx,
        prop: &EnumProp,
        path: &[AssetUuid],
        comp: usize,
        value: f32,
        is_final: bool,
    ) -> Handled {
        let Some((ty, settings, _schema_default, _)) = self.field_default_meta_at_path(path) else {
            return Handled::No;
        };
        let before = self.en.clone();
        let Some(slot) = self.get_default_mut_at_path(path) else {
            return Handled::No;
        };
        if apply_set_component(&ty, slot, comp, value, &settings) {
            let mkey = MergeKey::from(PropKey::new(prop.clone()));
            self.history.record(Some(before), Some(mkey));
            self.bump(false);
            ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
        }
        if is_final {
            self.history.end_merge();
        }
        Handled::Yes
    }

    fn apply_doc_action(&mut self, ctx: &mut DocCtx, action: &EnumAction) -> Handled {
        self.sync_cached_schemas(ctx);
        match action {
            EnumAction::AddVariant => {
                let name = self.draft_variant_name.clone();
                match self.add_variant_named(&name) {
                    Ok(_) => {
                        self.draft_variant_name.clear();
                        ctx.redraw_props = true;
                        ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                    }
                    Err(err) => {
                        self.variant_error = Some(err);
                        self.relayout();
                    }
                }
                Handled::Yes
            }
            EnumAction::SelectVariant(id) => {
                if self.selected_variant != *id {
                    self.selected_variant = *id;
                    self.selected_field =
                        id.and_then(|vid| self.en.variant_by_id(vid)).and_then(|v| v.fields.first()).map(|f| f.id);
                    self.field_error = None;
                    self.relayout();
                    ctx.redraw_props = true;
                }
                Handled::Yes
            }
            EnumAction::MoveVariant { id, delta } => {
                if self.move_variant(*id, *delta) {
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                }
                Handled::Yes
            }
            EnumAction::DeleteVariant(id) => {
                if self.delete_variant(*id) {
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                }
                Handled::Yes
            }
            EnumAction::AddField => {
                let name = self.draft_field_name.clone();
                let ty = self.draft_field_ty.clone();
                let is_name = self.draft_field_is_name;
                match self.add_field_named(None, &name, ty, is_name) {
                    Ok(_) => {
                        self.draft_field_name.clear();
                        self.draft_field_is_name = false;
                        ctx.redraw_props = true;
                        ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                    }
                    Err(err) => {
                        self.field_error = Some(err);
                        self.relayout();
                    }
                }
                Handled::Yes
            }
            EnumAction::SelectField(id) => {
                if self.selected_field != *id {
                    self.selected_field = *id;
                    if let Some(fid) = *id
                        && let Some((vi, _)) = self.find_field_indices(fid)
                    {
                        self.selected_variant = Some(self.en.variants[vi].id);
                    }
                    self.bump(false);
                    ctx.redraw_props = true;
                }
                Handled::Yes
            }
            EnumAction::ToggleConstraints(id) => {
                self.selected_field = Some(*id);
                if self.expanded_constraints == Some(*id) {
                    self.expanded_constraints = None;
                } else {
                    self.expanded_constraints = Some(*id);
                }
                self.relayout_constraints();
                self.bump(false);
                ctx.redraw_props = true;
                Handled::Yes
            }
            EnumAction::ToggleNestedDefault(path) => {
                if let Some(&first) = path.first() {
                    self.selected_field = Some(first);
                }
                if !self.expanded_nested.remove(path) {
                    self.expanded_nested.insert(path.clone());
                }
                self.relayout();
                self.bump(false);
                ctx.redraw_props = true;
                Handled::Yes
            }
            EnumAction::SetConstraintOption { id, field, some } => {
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                let before = self.en.clone();
                let f = &mut self.en.variants[vi].fields[fi];
                let changed = match field {
                    ConstraintField::Min => {
                        if f.settings.min.is_some() == *some {
                            false
                        } else {
                            f.settings.min = some.then_some(0.0);
                            if !*some {
                                f.settings.slider = false;
                            }
                            Self::clamp_field_default(f);
                            true
                        }
                    }
                    ConstraintField::Max => {
                        if f.settings.max.is_some() == *some {
                            false
                        } else {
                            let default_max = f.settings.min.unwrap_or(0.0) + 100.0;
                            f.settings.max = some.then_some(default_max);
                            if !*some {
                                f.settings.slider = false;
                            }
                            Self::clamp_field_default(f);
                            true
                        }
                    }
                    ConstraintField::Step => {
                        if f.settings.step.is_some() == *some {
                            false
                        } else {
                            f.settings.step = some.then_some(1.0);
                            true
                        }
                    }
                    ConstraintField::MaxLen => {
                        if f.settings.max_len.is_some() == *some {
                            false
                        } else {
                            f.settings.max_len = some.then_some(64);
                            Self::clamp_field_default(f);
                            true
                        }
                    }
                    _ => false,
                };
                if changed {
                    self.history.record(Some(before), None);
                    self.relayout_constraints();
                    self.bump(false);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                }
                Handled::Yes
            }
            EnumAction::ResetConstraint { id, field } => {
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                let before = self.en.clone();
                let f = &mut self.en.variants[vi].fields[fi];
                let mut rebuild_table = false;
                let changed = match field {
                    ConstraintField::Doc => {
                        if f.settings.doc.is_empty() {
                            false
                        } else {
                            f.settings.doc.clear();
                            true
                        }
                    }
                    ConstraintField::Min => {
                        if f.settings.min.is_none() {
                            false
                        } else {
                            f.settings.min = None;
                            f.settings.slider = false;
                            true
                        }
                    }
                    ConstraintField::Max => {
                        if f.settings.max.is_none() {
                            false
                        } else {
                            f.settings.max = None;
                            f.settings.slider = false;
                            true
                        }
                    }
                    ConstraintField::Step => {
                        if f.settings.step.is_none() {
                            false
                        } else {
                            f.settings.step = None;
                            true
                        }
                    }
                    ConstraintField::Slider => {
                        if !f.settings.slider {
                            false
                        } else {
                            f.settings.slider = false;
                            true
                        }
                    }
                    ConstraintField::Multiline => {
                        if !f.settings.multiline {
                            false
                        } else {
                            f.settings.multiline = false;
                            rebuild_table = true;
                            true
                        }
                    }
                    ConstraintField::MaxLen => {
                        if f.settings.max_len.is_none() {
                            false
                        } else {
                            f.settings.max_len = None;
                            rebuild_table = f.settings.multiline;
                            true
                        }
                    }
                    ConstraintField::Required => {
                        if !f.settings.required {
                            false
                        } else {
                            f.settings.required = false;
                            true
                        }
                    }
                    ConstraintField::UniqueInCollection => false,
                };
                if changed {
                    self.history.record(Some(before), None);
                    self.relayout_constraints();
                    self.bump(rebuild_table);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                }
                Handled::Yes
            }
            EnumAction::MoveField { id, delta } => {
                if self.move_field(*id, *delta) {
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                }
                Handled::Yes
            }
            EnumAction::DeleteField(id) => {
                if self.delete_field(*id) {
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                }
                Handled::Yes
            }
            EnumAction::ResetFieldDefault(id) => self.reset_default_at_path(ctx, &[*id]),
            EnumAction::ResetNestedDefault(path) => self.reset_default_at_path(ctx, path),
            EnumAction::SetOptionSome { path, some } => {
                let Some((ty, _, _, _)) = self.field_default_meta_at_path(path) else {
                    return Handled::No;
                };
                let before = self.en.clone();
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                let Some(slot) = self.get_default_mut_at_path(path) else {
                    return Handled::No;
                };
                if apply_set_option_some(&ty, slot, *some, &dtos, &enums) {
                    self.history.record(Some(before), None);
                    self.bump(true);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                }
                Handled::Yes
            }
            EnumAction::ContainerAdd { path } => {
                let Some((ty, _, _, _)) = self.field_default_meta_at_path(path) else {
                    return Handled::No;
                };
                let before = self.en.clone();
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                let Some(slot) = self.get_default_mut_at_path(path) else {
                    return Handled::No;
                };
                if apply_container_add(&ty, slot, &dtos, &enums) {
                    self.expanded_nested.insert(path.clone());
                    self.history.record(Some(before), None);
                    self.bump(true);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                }
                Handled::Yes
            }
            EnumAction::ContainerRemove { path, index } => {
                let Some((ty, _, _, _)) = self.field_default_meta_at_path(path) else {
                    return Handled::No;
                };
                let before = self.en.clone();
                let Some(slot) = self.get_default_mut_at_path(path) else {
                    return Handled::No;
                };
                if apply_container_remove(&ty, slot, *index) {
                    self.history.record(Some(before), None);
                    self.bump(true);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                }
                Handled::Yes
            }
            EnumAction::ContainerMove { path, index, delta } => {
                let Some((ty, _, _, _)) = self.field_default_meta_at_path(path) else {
                    return Handled::No;
                };
                let before = self.en.clone();
                let Some(slot) = self.get_default_mut_at_path(path) else {
                    return Handled::No;
                };
                if apply_container_move(&ty, slot, *index, *delta) {
                    self.history.record(Some(before), None);
                    self.bump(true);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                }
                Handled::Yes
            }
        }
    }

    fn apply_set_prop_value(&mut self, ctx: &mut DocCtx, prop: &EnumProp, value: &PropValue, merge: bool) -> Handled {
        self.sync_cached_schemas(ctx);
        let mkey = |p: &EnumProp| merge.then(|| MergeKey::from(PropKey::new(p.clone())));
        match prop {
            EnumProp::EnumDoc => {
                let PropValue::Str(s) = value else { return Handled::No };
                if self.en.doc == *s {
                    return Handled::Yes;
                }
                let before = self.en.clone();
                self.en.doc = s.clone();
                self.history.record(Some(before), mkey(prop));
                self.bump(false);
                ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                Handled::Yes
            }
            EnumProp::DraftVariantName => {
                if let PropValue::Str(s) = value {
                    self.draft_variant_name = s.clone();
                    if self.variant_error.is_some() {
                        self.variant_error = None;
                        self.relayout();
                    }
                    return Handled::Yes;
                }
                Handled::No
            }
            EnumProp::VariantName(vid) => {
                let PropValue::Str(s) = value else { return Handled::No };
                let Some(vi) = self.en.variants.iter().position(|v| v.id == *vid) else {
                    return Handled::No;
                };
                if self.en.variants[vi].name == *s {
                    return Handled::Yes;
                }
                let before = self.en.clone();
                self.en.variants[vi].name = s.clone();
                self.history.record(Some(before), mkey(prop));
                self.bump(false);
                ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                Handled::Yes
            }
            EnumProp::VariantDoc(vid) => {
                let PropValue::Str(s) = value else { return Handled::No };
                let Some(vi) = self.en.variants.iter().position(|v| v.id == *vid) else {
                    return Handled::No;
                };
                if self.en.variants[vi].doc == *s {
                    return Handled::Yes;
                }
                let before = self.en.clone();
                self.en.variants[vi].doc = s.clone();
                self.history.record(Some(before), mkey(prop));
                self.bump(false);
                ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                Handled::Yes
            }
            EnumProp::DraftFieldName => {
                if let PropValue::Str(s) = value {
                    self.draft_field_name = s.clone();
                    if self.field_error.is_some() {
                        self.field_error = None;
                        self.relayout();
                    }
                    return Handled::Yes;
                }
                Handled::No
            }
            EnumProp::DraftFieldType => {
                let Some(tag) = parse_type_tag_prop(value, FieldType::all()) else {
                    return Handled::No;
                };
                let Some(next_ty) = resolve_primary_field_type(
                    &self.draft_field_ty,
                    tag,
                    Some(self.first_dto_id()),
                    Some(self.first_non_self_enum_id()),
                ) else {
                    return Handled::No;
                };
                if self.draft_field_ty != next_ty {
                    self.draft_field_ty = next_ty;
                    self.relayout();
                }
                Handled::Yes
            }
            EnumProp::DraftFieldDtoTarget => {
                let PropValue::Str(s) = value else { return Handled::No };
                let target = if s.trim().is_empty() {
                    AssetUuid::NIL
                } else if let Some(id) = AssetUuid::parse(s.trim()) {
                    id
                } else {
                    return Handled::No;
                };
                if self.draft_field_ty.is_soft_ref() {
                    self.draft_field_ty = FieldType::SoftRef(target);
                } else {
                    self.draft_field_ty = FieldType::Dto(target);
                }
                Handled::Yes
            }
            EnumProp::DraftFieldEnumTarget => {
                let PropValue::Str(s) = value else { return Handled::No };
                let target = if s.trim().is_empty() {
                    AssetUuid::NIL
                } else if let Some(id) = AssetUuid::parse(s.trim()) {
                    id
                } else {
                    return Handled::No;
                };
                self.draft_field_ty = FieldType::Enum(target);
                Handled::Yes
            }
            EnumProp::DraftFieldGameAssetKind => {
                let PropValue::Str(s) = value else { return Handled::No };
                let Some(kind) = GameAssetKind::ALL.iter().copied().find(|k| k.label().eq_ignore_ascii_case(s.trim()))
                else {
                    return Handled::No;
                };
                self.draft_field_ty = FieldType::GameAsset(kind);
                Handled::Yes
            }
            EnumProp::DraftFieldInnerType => {
                let first_dto = self.first_dto_id();
                let first_enum = self.first_non_self_enum_id();
                if apply_inner_type_change(&mut self.draft_field_ty, value, Some(first_dto), Some(first_enum)) {
                    self.relayout();
                }
                Handled::Yes
            }
            EnumProp::DraftFieldInnerTarget => {
                if apply_inner_target_change(&mut self.draft_field_ty, value) {
                    self.relayout();
                }
                Handled::Yes
            }
            EnumProp::DraftFieldKeyType => {
                let first_enum = self.first_non_self_enum_id();
                if apply_key_type_change(&mut self.draft_field_ty, value, Some(first_enum)) {
                    self.relayout();
                }
                Handled::Yes
            }
            EnumProp::DraftFieldKeyTarget => {
                if apply_key_target_change(&mut self.draft_field_ty, value) {
                    self.relayout();
                }
                Handled::Yes
            }
            EnumProp::DraftFieldIsName => {
                if let PropValue::Bool(b) = value {
                    self.draft_field_is_name = *b;
                    return Handled::Yes;
                }
                Handled::No
            }
            EnumProp::FieldName(id) => {
                let PropValue::Str(s) = value else { return Handled::No };
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                if self.en.variants[vi].fields[fi].name == *s {
                    return Handled::Yes;
                }
                let before = self.en.clone();
                self.en.variants[vi].fields[fi].name = s.clone();
                self.history.record(Some(before), mkey(prop));
                self.bump(false);
                if self.expanded_constraints == Some(*id) {
                    self.relayout_constraints();
                }
                ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                Handled::Yes
            }
            EnumProp::FieldType(id) => {
                let Some(tag) = parse_type_tag_prop(value, FieldType::all()) else {
                    return Handled::No;
                };
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                let prev_ty = self.en.variants[vi].fields[fi].ty.clone();
                let Some(next_ty) = resolve_primary_field_type(
                    &prev_ty,
                    tag,
                    Some(self.first_dto_id()),
                    Some(self.first_non_self_enum_id()),
                ) else {
                    return Handled::No;
                };
                if self.en.variants[vi].fields[fi].ty == next_ty {
                    return Handled::Yes;
                }
                let before = self.en.clone();
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                coerce_default_for_type_change(
                    &prev_ty,
                    &next_ty,
                    &mut self.en.variants[vi].fields[fi].default,
                    &dtos,
                    &enums,
                );
                if !(next_ty.is_dto() || next_ty.is_enum() || next_ty.is_option() || next_ty.is_container()) {
                    self.expanded_nested.retain(|p| p.first() != Some(id));
                }
                self.variant_drafts.retain(|p, _| p.first() != Some(id));
                let f = &mut self.en.variants[vi].fields[fi];
                f.ty = next_ty.clone();
                if !next_ty.is_numeric() {
                    f.settings.min = None;
                    f.settings.max = None;
                    f.settings.step = None;
                    f.settings.slider = false;
                }
                if !next_ty.is_string() {
                    f.settings.multiline = false;
                    f.settings.max_len = None;
                    f.settings.unique_in_collection = false;
                }
                if !(next_ty.is_string() || next_ty.is_soft_ref() || next_ty.is_asset_ref() || next_ty.is_container()) {
                    f.settings.required = false;
                }
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                Handled::Yes
            }
            EnumProp::FieldInnerType(id) => {
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                let prev_ty = self.en.variants[vi].fields[fi].ty.clone();
                let mut next_ty = prev_ty.clone();
                let first_dto = self.first_dto_id();
                let first_enum = self.first_non_self_enum_id();
                if !apply_inner_type_change(&mut next_ty, value, Some(first_dto), Some(first_enum)) {
                    return Handled::Yes;
                }
                let before = self.en.clone();
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                coerce_default_for_type_change(
                    &prev_ty,
                    &next_ty,
                    &mut self.en.variants[vi].fields[fi].default,
                    &dtos,
                    &enums,
                );
                self.en.variants[vi].fields[fi].ty = next_ty;
                self.expanded_nested.retain(|p| p.first() != Some(id) || p.len() == 1);
                self.variant_drafts.retain(|p, _| p.first() != Some(id));
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                Handled::Yes
            }
            EnumProp::FieldInnerTarget(id) => {
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                let prev_ty = self.en.variants[vi].fields[fi].ty.clone();
                let mut next_ty = prev_ty.clone();
                if !apply_inner_target_change(&mut next_ty, value) {
                    return Handled::Yes;
                }
                let before = self.en.clone();
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                coerce_default_for_type_change(
                    &prev_ty,
                    &next_ty,
                    &mut self.en.variants[vi].fields[fi].default,
                    &dtos,
                    &enums,
                );
                self.en.variants[vi].fields[fi].ty = next_ty;
                self.expanded_nested.retain(|p| p.first() != Some(id) || p.len() == 1);
                self.variant_drafts.retain(|p, _| p.first() != Some(id));
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                Handled::Yes
            }
            EnumProp::FieldKeyType(id) => {
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                let prev_ty = self.en.variants[vi].fields[fi].ty.clone();
                let mut next_ty = prev_ty.clone();
                let first_enum = self.first_non_self_enum_id();
                if !apply_key_type_change(&mut next_ty, value, Some(first_enum)) {
                    return Handled::Yes;
                }
                let before = self.en.clone();
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                coerce_default_for_type_change(
                    &prev_ty,
                    &next_ty,
                    &mut self.en.variants[vi].fields[fi].default,
                    &dtos,
                    &enums,
                );
                self.en.variants[vi].fields[fi].ty = next_ty;
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                Handled::Yes
            }
            EnumProp::FieldKeyTarget(id) => {
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                let prev_ty = self.en.variants[vi].fields[fi].ty.clone();
                let mut next_ty = prev_ty.clone();
                if !apply_key_target_change(&mut next_ty, value) {
                    return Handled::Yes;
                }
                let before = self.en.clone();
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                coerce_default_for_type_change(
                    &prev_ty,
                    &next_ty,
                    &mut self.en.variants[vi].fields[fi].default,
                    &dtos,
                    &enums,
                );
                self.en.variants[vi].fields[fi].ty = next_ty;
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                Handled::Yes
            }
            EnumProp::FieldDtoTarget(id) => {
                let PropValue::Str(s) = value else { return Handled::No };
                let target = if s.trim().is_empty() {
                    AssetUuid::NIL
                } else if let Some(tid) = AssetUuid::parse(s.trim()) {
                    tid
                } else {
                    return Handled::No;
                };
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                let is_soft_ref = self.en.variants[vi].fields[fi].ty.is_soft_ref();
                let next_ty = if is_soft_ref { FieldType::SoftRef(target) } else { FieldType::Dto(target) };
                if self.en.variants[vi].fields[fi].ty == next_ty {
                    return Handled::Yes;
                }
                let before = self.en.clone();
                self.en.variants[vi].fields[fi].ty = next_ty.clone();
                self.en.variants[vi].fields[fi].default = if is_soft_ref {
                    DataValue::SoftRef(AssetUuid::NIL)
                } else {
                    next_ty.default_value_with_schemas(&self.cached_dtos, &self.cached_enums)
                };
                self.expanded_nested.retain(|p| p.first() != Some(id) || p.len() == 1);
                self.variant_drafts.retain(|p, _| p.first() != Some(id));
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                Handled::Yes
            }
            EnumProp::FieldEnumTarget(id) => {
                let PropValue::Str(s) = value else { return Handled::No };
                let target = if s.trim().is_empty() {
                    AssetUuid::NIL
                } else if let Some(tid) = AssetUuid::parse(s.trim()) {
                    tid
                } else {
                    return Handled::No;
                };
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                let next_ty = FieldType::Enum(target);
                if self.en.variants[vi].fields[fi].ty == next_ty {
                    return Handled::Yes;
                }
                let before = self.en.clone();
                self.en.variants[vi].fields[fi].ty = next_ty.clone();
                self.en.variants[vi].fields[fi].default =
                    next_ty.default_value_with_schemas(&self.cached_dtos, &self.cached_enums);
                self.expanded_nested.retain(|p| p.first() != Some(id) || p.len() == 1);
                self.variant_drafts.retain(|p, _| p.first() != Some(id));
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                Handled::Yes
            }
            EnumProp::FieldGameAssetKind(id) => {
                let PropValue::Str(s) = value else { return Handled::No };
                let Some(kind) = GameAssetKind::ALL.iter().copied().find(|k| k.label().eq_ignore_ascii_case(s.trim()))
                else {
                    return Handled::No;
                };
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                let next_ty = FieldType::GameAsset(kind);
                if self.en.variants[vi].fields[fi].ty == next_ty {
                    return Handled::Yes;
                }
                let before = self.en.clone();
                self.en.variants[vi].fields[fi].ty = next_ty;
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                Handled::Yes
            }
            EnumProp::FieldIsName(id) => {
                let PropValue::Bool(b) = value else { return Handled::No };
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                if self.en.variants[vi].fields[fi].is_name == *b {
                    return Handled::Yes;
                }
                let before = self.en.clone();
                self.en.variants[vi].fields[fi].is_name = *b;
                self.history.record(Some(before), None);
                self.bump(false);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                Handled::Yes
            }
            EnumProp::FieldDefault(id) => self.set_default_value_at_path(ctx, prop, &[*id], value, merge),
            EnumProp::NestedFieldDefault(path) => self.set_default_value_at_path(ctx, prop, path, value, merge),
            EnumProp::FieldDoc(id) => {
                let PropValue::Str(s) = value else { return Handled::No };
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                if self.en.variants[vi].fields[fi].settings.doc == *s {
                    return Handled::Yes;
                }
                let was_empty = self.en.variants[vi].fields[fi].settings.doc.is_empty();
                let before = self.en.clone();
                self.en.variants[vi].fields[fi].settings.doc = s.clone();
                let is_empty = self.en.variants[vi].fields[fi].settings.doc.is_empty();
                self.history.record(Some(before), mkey(prop));
                if was_empty != is_empty && !merge {
                    self.relayout_constraints();
                }
                self.bump(false);
                ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                Handled::Yes
            }
            EnumProp::FieldMinEnabled(id) => {
                let PropValue::Bool(b) = value else { return Handled::No };
                self.apply_doc_action(
                    ctx,
                    &EnumAction::SetConstraintOption { id: *id, field: ConstraintField::Min, some: *b },
                )
            }
            EnumProp::FieldMaxEnabled(id) => {
                let PropValue::Bool(b) = value else { return Handled::No };
                self.apply_doc_action(
                    ctx,
                    &EnumAction::SetConstraintOption { id: *id, field: ConstraintField::Max, some: *b },
                )
            }
            EnumProp::FieldStepEnabled(id) => {
                let PropValue::Bool(b) = value else { return Handled::No };
                self.apply_doc_action(
                    ctx,
                    &EnumAction::SetConstraintOption { id: *id, field: ConstraintField::Step, some: *b },
                )
            }
            EnumProp::FieldSlider(id) => {
                let PropValue::Bool(b) = value else { return Handled::No };
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                if self.en.variants[vi].fields[fi].settings.slider == *b {
                    return Handled::Yes;
                }
                let before = self.en.clone();
                let f = &mut self.en.variants[vi].fields[fi];
                f.settings.slider = *b;
                if *b {
                    if f.settings.min.is_none() {
                        f.settings.min = Some(0.0);
                    }
                    if f.settings.max.is_none() {
                        let lo = f.settings.min.unwrap_or(0.0);
                        f.settings.max = Some(lo + 100.0);
                    }
                    Self::clamp_field_default(f);
                }
                self.history.record(Some(before), None);
                self.relayout_constraints();
                self.bump(false);
                ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                Handled::Yes
            }
            EnumProp::FieldMultiline(id) => {
                let PropValue::Bool(b) = value else { return Handled::No };
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                if self.en.variants[vi].fields[fi].settings.multiline == *b {
                    return Handled::Yes;
                }
                let before = self.en.clone();
                self.en.variants[vi].fields[fi].settings.multiline = *b;
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                Handled::Yes
            }
            EnumProp::FieldMaxLenEnabled(id) => {
                let PropValue::Bool(b) = value else { return Handled::No };
                self.apply_doc_action(
                    ctx,
                    &EnumAction::SetConstraintOption { id: *id, field: ConstraintField::MaxLen, some: *b },
                )
            }
            EnumProp::FieldRequired(id) => {
                let PropValue::Bool(b) = value else { return Handled::No };
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                if self.en.variants[vi].fields[fi].settings.required == *b {
                    return Handled::Yes;
                }
                let before = self.en.clone();
                self.en.variants[vi].fields[fi].settings.required = *b;
                self.history.record(Some(before), None);
                self.relayout_constraints();
                self.bump(false);
                ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                Handled::Yes
            }
            EnumProp::FieldMin(_) | EnumProp::FieldMax(_) | EnumProp::FieldStep(_) | EnumProp::FieldMaxLen(_) => {
                Handled::No
            }
        }
    }

    fn apply_set_prop_component(
        &mut self,
        ctx: &mut DocCtx,
        prop: &EnumProp,
        comp: usize,
        value: f32,
        is_final: bool,
    ) -> Handled {
        self.sync_cached_schemas(ctx);
        let mkey = MergeKey::from(PropKey::new(prop.clone()));
        match prop {
            EnumProp::FieldDefault(id) => self.set_default_component_at_path(ctx, prop, &[*id], comp, value, is_final),
            EnumProp::NestedFieldDefault(path) => {
                self.set_default_component_at_path(ctx, prop, path, comp, value, is_final)
            }
            EnumProp::FieldMin(id) if comp == 0 => {
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                let next = if value.is_finite() { value as f64 } else { 0.0 };
                if self.en.variants[vi].fields[fi].settings.min != Some(next) {
                    let before = self.en.clone();
                    let f = &mut self.en.variants[vi].fields[fi];
                    f.settings.min = Some(next);
                    Self::clamp_field_default(f);
                    self.history.record(Some(before), Some(mkey));
                    self.bump(false);
                    ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                }
                if is_final {
                    self.history.end_merge();
                }
                Handled::Yes
            }
            EnumProp::FieldMax(id) if comp == 0 => {
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                let next = if value.is_finite() { value as f64 } else { 100.0 };
                if self.en.variants[vi].fields[fi].settings.max != Some(next) {
                    let before = self.en.clone();
                    let f = &mut self.en.variants[vi].fields[fi];
                    f.settings.max = Some(next);
                    Self::clamp_field_default(f);
                    self.history.record(Some(before), Some(mkey));
                    self.bump(false);
                    ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                }
                if is_final {
                    self.history.end_merge();
                }
                Handled::Yes
            }
            EnumProp::FieldStep(id) if comp == 0 => {
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                let next = if value.is_finite() && value > 0.0 { value as f64 } else { 1.0 };
                if self.en.variants[vi].fields[fi].settings.step != Some(next) {
                    let before = self.en.clone();
                    self.en.variants[vi].fields[fi].settings.step = Some(next);
                    self.history.record(Some(before), Some(mkey));
                    self.bump(false);
                    ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                }
                if is_final {
                    self.history.end_merge();
                }
                Handled::Yes
            }
            EnumProp::FieldMaxLen(id) if comp == 0 => {
                let Some((vi, fi)) = self.find_field_indices(*id) else {
                    return Handled::No;
                };
                let next = if value.is_finite() { value.round().max(1.0) as usize } else { 64 };
                if self.en.variants[vi].fields[fi].settings.max_len != Some(next) {
                    let before = self.en.clone();
                    let f = &mut self.en.variants[vi].fields[fi];
                    f.settings.max_len = Some(next);
                    Self::clamp_field_default(f);
                    self.history.record(Some(before), Some(mkey));
                    self.bump(false);
                    ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                }
                if is_final {
                    self.history.end_merge();
                }
                Handled::Yes
            }
            _ => Handled::No,
        }
    }
}

impl TabDoc for EnumDoc {
    fn asset(&self) -> Option<&AssetRef> {
        Some(&self.aref)
    }

    fn view(&self) -> ViewKindId {
        ENUM_VIEW
    }

    fn base_title(&self) -> String {
        format!("quillshade/data/{}.enum", self.path)
    }

    fn icon(&self) -> IconRef {
        ENUM_ICON
    }

    fn watermark(&self) -> Option<&'static str> {
        Some("ENUM")
    }

    fn is_dirty(&self) -> bool {
        self.en != self.saved
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn save(&mut self, ctx: &mut DocCtx) -> io::Result<()> {
        let Some(project) = ctx.project else {
            return Err(io::Error::other("No open project"));
        };
        self.sync_cached_schemas(ctx);
        let file = project.root.join(self.aref.source_rel());
        save_enum_with_index(&file, &mut self.en, &self.cached_dtos, &self.cached_enums, Some(ctx.index))?;
        self.cached_enums.insert(self.path.clone(), self.en.clone());
        self.saved = self.en.clone();
        self.bump(false);
        ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
        Ok(())
    }

    fn changed_on_disk(&self, project: &Project) -> bool {
        let file = project.root.join(self.aref.source_rel());
        load_enum(&file).is_ok_and(|disk| disk != self.saved)
    }

    fn reload_from_disk(&mut self, ctx: &mut DocCtx) -> Result<(), String> {
        let Some(project) = ctx.project else {
            return Err("No open project".into());
        };
        let file = project.root.join(self.aref.source_rel());
        let mut loaded = load_enum(&file).map_err(|e| e.to_string())?;
        loaded.ensure_id();
        self.en = loaded.clone();
        self.saved = loaded;
        self.history = History::default();
        self.sync_cached_schemas(ctx);
        if self.selected_variant.is_none_or(|id| !self.en.variants.iter().any(|v| v.id == id)) {
            self.selected_variant = self.en.variants.first().map(|v| v.id);
        }
        if self.selected_field.is_some_and(|fid| self.find_field_indices(fid).is_none()) {
            self.selected_field = self.active_variant().and_then(|v| v.fields.first()).map(|f| f.id);
        }
        if self.expanded_constraints.is_some_and(|fid| self.find_field_indices(fid).is_none()) {
            self.expanded_constraints = None;
        }
        self.bump(true);
        ctx.redraw_props = true;
        ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
        Ok(())
    }

    fn keep_mine(&mut self, project: &Project) {
        let file = project.root.join(self.aref.source_rel());
        if let Ok(disk) = load_enum(&file) {
            self.saved = disk;
            self.bump(false);
        }
    }

    fn rename(&mut self, _project: &Project, from: &AssetRef, to: &AssetRef) {
        if &self.aref == from {
            self.cached_enums.remove(&self.path);
            self.path = to.path.clone();
            self.aref = to.clone();
            if !self.en.id.is_nil() {
                self.cached_enums.insert(self.path.clone(), self.en.clone());
            }
            self.bump(true);
        }
    }

    fn config_key(&self) -> Option<String> {
        Some(self.aref.source_rel())
    }

    fn status(&self, ctx: &StatusCtx) -> Option<StatusInfo> {
        let n_vars = self.en.variants.len();
        let vars_label = if n_vars == 1 { "1 variant".to_string() } else { format!("{n_vars} variants") };
        let kind_label = if self.en.is_unit_only() { "unit enum".to_string() } else { "tagged union".to_string() };
        let dtos = ctx
            .domain::<DataState>()
            .filter(|st| !st.assets.dtos.is_empty())
            .map(|st| &st.assets.dtos)
            .unwrap_or(&self.cached_dtos);
        let enums = ctx
            .domain::<DataState>()
            .filter(|st| !st.assets.enums.is_empty())
            .map(|st| &st.assets.enums)
            .unwrap_or(&self.cached_enums);
        let rust_lines = emit_enum_file_with_schemas(&self.path, &self.en, dtos, enums).lines().count();
        Some(StatusInfo::new(self.is_dirty(), ctx, vec![vars_label, kind_label, format!("{rust_lines} Rust lines")]))
    }

    fn apply(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        self.sync_cached_schemas(ctx);
        match action {
            TabAction::Undo => {
                if self.history.undo_keeping(&mut self.en, |m| &mut m.id) {
                    if self.selected_variant.is_none_or(|id| !self.en.variants.iter().any(|v| v.id == id)) {
                        self.selected_variant = self.en.variants.first().map(|v| v.id);
                    }
                    if self.selected_field.is_some_and(|fid| self.find_field_indices(fid).is_none()) {
                        self.selected_field = self.active_variant().and_then(|v| v.fields.first()).map(|f| f.id);
                    }
                    if self.expanded_constraints.is_some_and(|fid| self.find_field_indices(fid).is_none()) {
                        self.expanded_constraints = None;
                    }
                    self.bump(true);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                }
                Handled::Yes
            }
            TabAction::Redo => {
                if self.history.redo_keeping(&mut self.en, |m| &mut m.id) {
                    if self.selected_variant.is_none_or(|id| !self.en.variants.iter().any(|v| v.id == id)) {
                        self.selected_variant = self.en.variants.first().map(|v| v.id);
                    }
                    if self.selected_field.is_some_and(|fid| self.find_field_indices(fid).is_none()) {
                        self.selected_field = self.active_variant().and_then(|v| v.fields.first()).map(|f| f.id);
                    }
                    if self.expanded_constraints.is_some_and(|fid| self.find_field_indices(fid).is_none()) {
                        self.expanded_constraints = None;
                    }
                    self.bump(true);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncEnumInMemory { path: self.path.clone(), en: self.en.clone() });
                }
                Handled::Yes
            }
            TabAction::CommitInline => {
                self.history.end_merge();
                Handled::Yes
            }
            TabAction::DeleteSelection => {
                if let Some(fid) = self.selected_field {
                    return self.apply_doc_action(ctx, &EnumAction::DeleteField(fid));
                }
                if let Some(vid) = self.selected_variant {
                    return self.apply_doc_action(ctx, &EnumAction::DeleteVariant(vid));
                }
                Handled::No
            }
            TabAction::SetValue { key, value, merge } => {
                let Some(prop) = key.downcast::<EnumProp>() else {
                    return Handled::No;
                };
                self.apply_set_prop_value(ctx, prop, value, *merge)
            }
            TabAction::SetComponent { key, comp, value, is_final } => {
                let Some(prop) = key.downcast::<EnumProp>() else {
                    return Handled::No;
                };
                self.apply_set_prop_component(ctx, prop, *comp, *value, *is_final)
            }
            TabAction::Doc(payload) => {
                let Some(act) = payload.downcast::<EnumAction>() else {
                    return Handled::No;
                };
                self.apply_doc_action(ctx, act)
            }
            _ => Handled::No,
        }
    }

    fn current_value(&self, key: &PropKey, ctx: &StatusCtx) -> Option<PropValue> {
        let prop = key.downcast::<EnumProp>()?;
        let dtos = ctx
            .domain::<DataState>()
            .filter(|st| !st.assets.dtos.is_empty())
            .map(|st| &st.assets.dtos)
            .unwrap_or(&self.cached_dtos);
        let enums = ctx
            .domain::<DataState>()
            .filter(|st| !st.assets.enums.is_empty())
            .map(|st| &st.assets.enums)
            .unwrap_or(&self.cached_enums);
        match prop {
            EnumProp::EnumDoc => Some(PropValue::Str(self.en.doc.clone())),
            EnumProp::DraftVariantName => Some(PropValue::Str(self.draft_variant_name.clone())),
            EnumProp::VariantName(vid) => self.en.variant_by_id(*vid).map(|v| PropValue::Str(v.name.clone())),
            EnumProp::VariantDoc(vid) => self.en.variant_by_id(*vid).map(|v| PropValue::Str(v.doc.clone())),
            EnumProp::DraftFieldName => Some(PropValue::Str(self.draft_field_name.clone())),
            EnumProp::DraftFieldType => Some(PropValue::Str(self.draft_field_ty.label().to_string())),
            EnumProp::DraftFieldDtoTarget => {
                let target = self
                    .draft_field_ty
                    .dto_target()
                    .or_else(|| self.draft_field_ty.soft_ref_target())
                    .unwrap_or(AssetUuid::NIL);
                Some(PropValue::Str(if target.is_nil() { String::new() } else { target.to_string() }))
            }
            EnumProp::DraftFieldEnumTarget => {
                let target = self.draft_field_ty.enum_target().unwrap_or(AssetUuid::NIL);
                Some(PropValue::Str(if target.is_nil() { String::new() } else { target.to_string() }))
            }
            EnumProp::DraftFieldGameAssetKind => {
                let kind = match self.draft_field_ty {
                    FieldType::GameAsset(k) => k,
                    _ => GameAssetKind::Any,
                };
                Some(PropValue::Str(kind.label().to_string()))
            }
            EnumProp::DraftFieldInnerType => {
                self.draft_field_ty.inner_type().map(|t| PropValue::Str(t.tag().to_string()))
            }
            EnumProp::DraftFieldInnerTarget => {
                let inner = self.draft_field_ty.inner_type()?;
                let s = match inner {
                    FieldType::Dto(u) | FieldType::SoftRef(u) | FieldType::Enum(u) => {
                        if u.is_nil() {
                            String::new()
                        } else {
                            u.to_string()
                        }
                    }
                    FieldType::GameAsset(k) => k.tag().to_string(),
                    _ => return None,
                };
                Some(PropValue::Str(s))
            }
            EnumProp::DraftFieldKeyType => self.draft_field_ty.key_type().map(|t| PropValue::Str(t.tag().to_string())),
            EnumProp::DraftFieldKeyTarget => {
                let key = self.draft_field_ty.key_type()?;
                let u = key.enum_target()?;
                Some(PropValue::Str(if u.is_nil() { String::new() } else { u.to_string() }))
            }
            EnumProp::DraftFieldIsName => Some(PropValue::Bool(self.draft_field_is_name)),
            EnumProp::FieldName(id) => self
                .find_field_indices(*id)
                .map(|(vi, fi)| PropValue::Str(self.en.variants[vi].fields[fi].name.clone())),
            EnumProp::FieldType(id) => self
                .find_field_indices(*id)
                .map(|(vi, fi)| PropValue::Str(self.en.variants[vi].fields[fi].ty.label().to_string())),
            EnumProp::FieldDtoTarget(id) => {
                let (vi, fi) = self.find_field_indices(*id)?;
                let f = &self.en.variants[vi].fields[fi];
                let target = f.ty.dto_target().or_else(|| f.ty.soft_ref_target()).unwrap_or(AssetUuid::NIL);
                Some(PropValue::Str(if target.is_nil() { String::new() } else { target.to_string() }))
            }
            EnumProp::FieldEnumTarget(id) => {
                let (vi, fi) = self.find_field_indices(*id)?;
                let f = &self.en.variants[vi].fields[fi];
                let target = f.ty.enum_target().unwrap_or(AssetUuid::NIL);
                Some(PropValue::Str(if target.is_nil() { String::new() } else { target.to_string() }))
            }
            EnumProp::FieldGameAssetKind(id) => {
                let (vi, fi) = self.find_field_indices(*id)?;
                let kind = match self.en.variants[vi].fields[fi].ty {
                    FieldType::GameAsset(k) => k,
                    _ => GameAssetKind::Any,
                };
                Some(PropValue::Str(kind.label().to_string()))
            }
            EnumProp::FieldInnerType(id) => {
                let (vi, fi) = self.find_field_indices(*id)?;
                self.en.variants[vi].fields[fi].ty.inner_type().map(|t| PropValue::Str(t.tag().to_string()))
            }
            EnumProp::FieldInnerTarget(id) => {
                let (vi, fi) = self.find_field_indices(*id)?;
                let inner = self.en.variants[vi].fields[fi].ty.inner_type()?;
                let s = match inner {
                    FieldType::Dto(u) | FieldType::SoftRef(u) | FieldType::Enum(u) => {
                        if u.is_nil() {
                            String::new()
                        } else {
                            u.to_string()
                        }
                    }
                    FieldType::GameAsset(k) => k.tag().to_string(),
                    _ => return None,
                };
                Some(PropValue::Str(s))
            }
            EnumProp::FieldKeyType(id) => {
                let (vi, fi) = self.find_field_indices(*id)?;
                self.en.variants[vi].fields[fi].ty.key_type().map(|t| PropValue::Str(t.tag().to_string()))
            }
            EnumProp::FieldKeyTarget(id) => {
                let (vi, fi) = self.find_field_indices(*id)?;
                let key = self.en.variants[vi].fields[fi].ty.key_type()?;
                let u = key.enum_target()?;
                Some(PropValue::Str(if u.is_nil() { String::new() } else { u.to_string() }))
            }
            EnumProp::FieldIsName(id) => {
                self.find_field_indices(*id).map(|(vi, fi)| PropValue::Bool(self.en.variants[vi].fields[fi].is_name))
            }
            EnumProp::FieldDefault(id) => {
                let (vi, _) = self.find_field_indices(*id)?;
                let vid = self.en.variants[vi].id;
                let (ty, _, schema_default, _) = self.en.variant_field_meta_at_path(vid, &[*id], dtos, enums, true)?;
                let val = self.get_default_at_path(&[*id], &schema_default);
                data_value_to_prop(&ty, val)
            }
            EnumProp::NestedFieldDefault(path) => {
                let first = *path.first()?;
                let (vi, _) = self.find_field_indices(first)?;
                let vid = self.en.variants[vi].id;
                let (ty, _, schema_default, _) = self.en.variant_field_meta_at_path(vid, path, dtos, enums, true)?;
                let val = self.get_default_at_path(path, &schema_default);
                data_value_to_prop(&ty, val)
            }
            EnumProp::FieldDoc(id) => self
                .find_field_indices(*id)
                .map(|(vi, fi)| PropValue::Str(self.en.variants[vi].fields[fi].settings.doc.clone())),
            EnumProp::FieldMinEnabled(id) => self
                .find_field_indices(*id)
                .map(|(vi, fi)| PropValue::Bool(self.en.variants[vi].fields[fi].settings.min.is_some())),
            EnumProp::FieldMin(id) => self
                .find_field_indices(*id)
                .and_then(|(vi, fi)| self.en.variants[vi].fields[fi].settings.min)
                .map(|v| PropValue::Float(v as f32)),
            EnumProp::FieldMaxEnabled(id) => self
                .find_field_indices(*id)
                .map(|(vi, fi)| PropValue::Bool(self.en.variants[vi].fields[fi].settings.max.is_some())),
            EnumProp::FieldMax(id) => self
                .find_field_indices(*id)
                .and_then(|(vi, fi)| self.en.variants[vi].fields[fi].settings.max)
                .map(|v| PropValue::Float(v as f32)),
            EnumProp::FieldStepEnabled(id) => self
                .find_field_indices(*id)
                .map(|(vi, fi)| PropValue::Bool(self.en.variants[vi].fields[fi].settings.step.is_some())),
            EnumProp::FieldStep(id) => self
                .find_field_indices(*id)
                .and_then(|(vi, fi)| self.en.variants[vi].fields[fi].settings.step)
                .map(|v| PropValue::Float(v as f32)),
            EnumProp::FieldSlider(id) => self
                .find_field_indices(*id)
                .map(|(vi, fi)| PropValue::Bool(self.en.variants[vi].fields[fi].settings.slider)),
            EnumProp::FieldMultiline(id) => self
                .find_field_indices(*id)
                .map(|(vi, fi)| PropValue::Bool(self.en.variants[vi].fields[fi].settings.multiline)),
            EnumProp::FieldMaxLenEnabled(id) => self
                .find_field_indices(*id)
                .map(|(vi, fi)| PropValue::Bool(self.en.variants[vi].fields[fi].settings.max_len.is_some())),
            EnumProp::FieldMaxLen(id) => self
                .find_field_indices(*id)
                .and_then(|(vi, fi)| self.en.variants[vi].fields[fi].settings.max_len)
                .map(|v| PropValue::Float(v as f32)),
            EnumProp::FieldRequired(id) => self
                .find_field_indices(*id)
                .map(|(vi, fi)| PropValue::Bool(self.en.variants[vi].fields[fi].settings.required)),
        }
    }
}
