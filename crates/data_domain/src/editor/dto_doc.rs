//! `.dto.ron` schema document (`DtoDoc`): in-memory `DtoAsset`, undo/redo [`History`], selected field for the
//! settings inspector, add-field draft state, and [`TabDoc`] implementation.

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

use crate::backend::codegen::emit_dto_file_with_schemas;
use crate::backend::model::{
    DataValue, DtoAsset, EnumAsset, Field, FieldSettings, FieldType, GameAssetKind, dto_ref,
    ensure_nested_value_with_schemas_mut,
};
use crate::backend::ron_io::{load_dto, save_dto_with_schemas_and_index};

use super::DTO_ICON;
use super::state::{DataState, DataStep};
use super::widgets::{
    apply_container_add, apply_container_move, apply_container_remove, apply_inner_target_change,
    apply_inner_type_change, apply_key_target_change, apply_key_type_change, apply_set_component,
    apply_set_option_some, apply_set_value, coerce_default_for_type_change, data_value_to_prop, parse_type_tag_prop,
    resolve_primary_field_type, switch_enum_variant_with_drafts,
};

pub const DTO_VIEW: ViewKindId = ViewKindId("data.dto");

fn next_layout() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Property keys for widgets inside the `.dto.ron` editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DtoProp {
    DraftName,
    DraftType,
    DraftDtoTarget,
    DraftEnumTarget,
    DraftGameAssetKind,
    DraftInnerType,
    DraftInnerTarget,
    DraftKeyType,
    DraftKeyTarget,
    DraftCategory,
    DraftCategoryPick,
    DraftIsName,
    FieldName(AssetUuid),
    FieldType(AssetUuid),
    FieldDtoTarget(AssetUuid),
    FieldEnumTarget(AssetUuid),
    FieldGameAssetKind(AssetUuid),
    FieldInnerType(AssetUuid),
    FieldInnerTarget(AssetUuid),
    FieldKeyType(AssetUuid),
    FieldKeyTarget(AssetUuid),
    FieldCategory(AssetUuid),
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
    FieldUniqueInCollection(AssetUuid),
}

impl PropTarget for DtoProp {}

/// Individual field in the internal Constraints (`FieldSettings`) DTO.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintField {
    Doc,
    Min,
    Max,
    Step,
    Slider,
    Multiline,
    MaxLen,
    Required,
    UniqueInCollection,
}

/// Document-level actions for [`DtoDoc`] (`TabAction::Doc`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DtoAction {
    /// Adds a field using the draft bar (auto-generating a unique name if `draft_name` is empty).
    AddField,
    /// Selects a field row without rebuilding the table.
    SelectField(Option<AssetUuid>),
    /// Toggles the inline `Constraints` (`FieldSettings`) subpane right under field `id`.
    ToggleConstraints(AssetUuid),
    /// Toggles the inline `FieldType::Dto` default value sub-form at `path` (`vec![field_id, ...]`).
    ToggleDtoDefault(Vec<AssetUuid>),
    /// Sets an `Option<T>` constraint (`min`, `max`, `step`, `max_len`) to `None` (`some = false`) or `Some` (`some = true`).
    SetConstraintOption { id: AssetUuid, field: ConstraintField, some: bool },
    /// Resets a single constraint field in `FieldSettings` back to its default.
    ResetConstraint { id: AssetUuid, field: ConstraintField },
    /// Moves a field up (`delta = -1`) or down (`delta = 1`) in the ordered field list.
    MoveField { id: AssetUuid, delta: isize },
    /// Deletes a field from the schema.
    DeleteField(AssetUuid),
    /// Resets a field's default value to `field.ty.default_value()`.
    ResetFieldDefault(AssetUuid),
    /// Resets a top-level or nested `FieldType::Dto` default value at `path` (`vec![field_id, ...]`).
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

impl From<DtoAction> for Action {
    fn from(a: DtoAction) -> Self {
        Action::Tab(TabAction::doc(a))
    }
}

/// Open `.dto.ron` document tab.
#[derive(Debug, Clone)]
pub struct DtoDoc {
    pub path: String,
    pub aref: AssetRef,
    pub dto: DtoAsset,
    pub saved: DtoAsset,
    pub history: History<DtoAsset>,
    pub selected_field: Option<AssetUuid>,
    pub expanded_constraints: Option<AssetUuid>,
    pub expanded_dtos: BTreeSet<Vec<AssetUuid>>,
    pub cached_dtos: BTreeMap<String, DtoAsset>,
    pub cached_enums: BTreeMap<String, EnumAsset>,
    pub variant_drafts: BTreeMap<Vec<AssetUuid>, super::widgets::VariantPayloadDrafts>,
    pub draft_name: String,
    pub draft_ty: FieldType,
    pub draft_category: String,
    pub draft_is_name: bool,
    pub form_error: Option<String>,
    pub revision: u64,
    pub layout: u64,
    pub constraints_layout: u64,
}

impl DtoDoc {
    pub fn new(path: impl Into<String>, dto: DtoAsset) -> Self {
        Self::with_dtos(path, dto, BTreeMap::new())
    }

    pub fn with_dtos(path: impl Into<String>, dto: DtoAsset, cached_dtos: BTreeMap<String, DtoAsset>) -> Self {
        Self::with_schemas(path, dto, cached_dtos, BTreeMap::new())
    }

    pub fn with_schemas(
        path: impl Into<String>,
        mut dto: DtoAsset,
        mut cached_dtos: BTreeMap<String, DtoAsset>,
        cached_enums: BTreeMap<String, EnumAsset>,
    ) -> Self {
        dto.ensure_id();
        let path = path.into();
        if !dto.id.is_nil() {
            cached_dtos.insert(path.clone(), dto.clone());
        }
        let selected_field = dto.fields.first().map(|f| f.id);
        let saved = dto.clone();
        Self {
            aref: dto_ref(&path),
            path,
            dto,
            saved,
            history: History::default(),
            selected_field,
            expanded_constraints: None,
            expanded_dtos: BTreeSet::new(),
            cached_dtos,
            cached_enums,
            variant_drafts: BTreeMap::new(),
            draft_name: String::new(),
            draft_ty: FieldType::I32,
            draft_category: String::new(),
            draft_is_name: false,
            form_error: None,
            revision: 1,
            layout: next_layout(),
            constraints_layout: next_layout(),
        }
    }

    pub fn sync_cached_dtos(&mut self, ctx: &mut DocCtx) {
        if let Some(st) = ctx.domain::<DataState>() {
            if !st.assets.dtos.is_empty() {
                self.cached_dtos = st.assets.dtos.clone();
            }
            if !st.assets.enums.is_empty() {
                self.cached_enums = st.assets.enums.clone();
            }
        }
        if !self.dto.id.is_nil() {
            self.cached_dtos.insert(self.path.clone(), self.dto.clone());
        }
    }

    pub fn first_non_self_dto_id(&self) -> AssetUuid {
        self.cached_dtos
            .values()
            .find(|d| !d.id.is_nil() && d.id != self.dto.id)
            .map(|d| d.id)
            .unwrap_or(AssetUuid::NIL)
    }

    pub fn first_enum_id(&self) -> AssetUuid {
        self.cached_enums.values().find(|e| !e.id.is_nil()).map(|e| e.id).unwrap_or(AssetUuid::NIL)
    }

    pub fn field_default_meta_at_path(
        &self,
        path: &[AssetUuid],
    ) -> Option<(FieldType, FieldSettings, DataValue, bool)> {
        self.dto.field_meta_at_path_with_schemas(path, &self.cached_dtos, &self.cached_enums, true)
    }

    pub fn get_default_at_path<'a>(&'a self, path: &[AssetUuid], fallback: &'a DataValue) -> &'a DataValue {
        let Some(&first) = path.first() else {
            return fallback;
        };
        let Some(f0) = self.dto.field_by_id(first) else {
            return fallback;
        };
        if path.len() == 1 {
            return &f0.default;
        }
        f0.default.get_at_path(&path[1..]).unwrap_or(fallback)
    }

    pub fn get_default_mut_at_path(&mut self, path: &[AssetUuid]) -> Option<&mut DataValue> {
        self.field_default_meta_at_path(path)?;
        let first = *path.first()?;
        let idx = self.dto.fields.iter().position(|f| f.id == first)?;
        if path.len() == 1 {
            return Some(&mut self.dto.fields[idx].default);
        }
        let ty = self.dto.fields[idx].ty.clone();
        ensure_nested_value_with_schemas_mut(
            &mut self.dto.fields[idx].default,
            ty,
            &self.cached_dtos,
            &self.cached_enums,
            &path[1..],
        )
    }

    fn prune_stale_expanded_dtos(&mut self) {
        self.expanded_dtos.retain(|path| {
            path.first()
                .and_then(|fid| self.dto.field_by_id(*fid))
                .is_some_and(|f| f.ty.is_dto() || f.ty.is_enum() || f.ty.is_option() || f.ty.is_container())
        });
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

    /// Distinct non-empty categories currently used by fields in this schema, in sorted order.
    pub fn category_names(&self) -> Vec<String> {
        let mut set = BTreeSet::new();
        for f in &self.dto.fields {
            let trimmed = f.category.trim();
            if !trimmed.is_empty() {
                set.insert(trimmed.to_string());
            }
        }
        set.into_iter().collect()
    }

    /// Generates a unique `snake_case` field name (`new_field`, `new_field_2`, ...).
    pub fn next_unique_field_name(&self, base: &str) -> String {
        let base = if base.trim().is_empty() { "new_field" } else { base.trim() };
        if !self.dto.fields.iter().any(|f| f.name == base) {
            return base.to_string();
        }
        let mut i = 2usize;
        loop {
            let candidate = format!("{base}_{i}");
            if !self.dto.fields.iter().any(|f| f.name == candidate) {
                return candidate;
            }
            i += 1;
        }
    }

    /// Programmatically adds a new field with the given `name` and `ty`, recording an undo step.
    pub fn add_field_named(
        &mut self,
        name: &str,
        ty: FieldType,
        category: &str,
        is_name: bool,
    ) -> Result<AssetUuid, String> {
        let trimmed = name.trim();
        let final_name = if trimmed.is_empty() {
            self.next_unique_field_name("new_field")
        } else {
            if let Err(e) = check_ident(trimmed) {
                return Err(format!("Invalid field name '{trimmed}': {e}"));
            }
            if self.dto.fields.iter().any(|f| f.name == trimmed) {
                return Err(format!("Field '{trimmed}' already exists in this DTO"));
            }
            trimmed.to_string()
        };
        let before = self.dto.clone();
        let id = AssetUuid::new_v7();
        let default = ty.default_value_with_schemas(&self.cached_dtos, &self.cached_enums);
        let field = Field {
            id,
            name: final_name,
            ty,
            category: category.trim().to_string(),
            is_name,
            default,
            settings: FieldSettings::default(),
        };
        self.dto.fields.push(field);
        self.history.record(Some(before), None);
        self.selected_field = Some(id);
        self.form_error = None;
        self.bump(true);
        Ok(id)
    }

    /// Moves field `id` by `delta` (`-1` for up, `+1` for down). Returns `true` if moved.
    pub fn move_field(&mut self, id: AssetUuid, delta: isize) -> bool {
        let Some(idx) = self.dto.fields.iter().position(|f| f.id == id) else {
            return false;
        };
        let next = idx as isize + delta;
        if next < 0 || next as usize >= self.dto.fields.len() {
            return false;
        }
        let before = self.dto.clone();
        self.dto.fields.swap(idx, next as usize);
        self.history.record(Some(before), None);
        self.bump(true);
        true
    }

    /// Deletes field `id`. Returns `true` if removed.
    pub fn delete_field(&mut self, id: AssetUuid) -> bool {
        let Some(idx) = self.dto.fields.iter().position(|f| f.id == id) else {
            return false;
        };
        let before = self.dto.clone();
        self.dto.fields.remove(idx);
        self.history.record(Some(before), None);
        if self.selected_field == Some(id) {
            self.selected_field = self.dto.fields.get(idx).or_else(|| self.dto.fields.last()).map(|f| f.id);
        }
        if self.expanded_constraints == Some(id) {
            self.expanded_constraints = None;
        }
        self.expanded_dtos.retain(|p| p.first() != Some(&id));
        self.variant_drafts.retain(|p, _| p.first() != Some(&id));
        self.bump(true);
        true
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
            let before = self.dto.clone();
            if let Some(slot) = self.get_default_mut_at_path(path) {
                *slot = schema_default;
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
            }
        }
        Handled::Yes
    }

    fn set_default_value_at_path(
        &mut self,
        ctx: &mut DocCtx,
        prop: &DtoProp,
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
            let before = self.dto.clone();
            let Some(slot) = self.get_default_mut_at_path(path) else {
                return Handled::No;
            };
            if switch_enum_variant_with_drafts(slot, target_enum.as_ref(), next_var, &dtos, &enums, &mut drafts) {
                self.variant_drafts.insert(path.to_vec(), drafts);
                let mkey = merge.then(|| MergeKey::from(PropKey::new(prop.clone())));
                self.history.record(Some(before), mkey);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
            }
            return Handled::Yes;
        }
        let before = self.dto.clone();
        let Some(slot) = self.get_default_mut_at_path(path) else {
            return Handled::No;
        };
        if apply_set_value(&ty, slot, value, &settings) {
            let mkey = merge.then(|| MergeKey::from(PropKey::new(prop.clone())));
            self.history.record(Some(before), mkey);
            self.bump(ty.is_asset_ref() || ty.is_soft_ref() || ty.is_enum());
            ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
        }
        Handled::Yes
    }

    fn set_default_component_at_path(
        &mut self,
        ctx: &mut DocCtx,
        prop: &DtoProp,
        path: &[AssetUuid],
        comp: usize,
        value: f32,
        is_final: bool,
    ) -> Handled {
        let Some((ty, settings, _schema_default, _)) = self.field_default_meta_at_path(path) else {
            return Handled::No;
        };
        let before = self.dto.clone();
        let Some(slot) = self.get_default_mut_at_path(path) else {
            return Handled::No;
        };
        if apply_set_component(ty, slot, comp, value, &settings) {
            let mkey = MergeKey::from(PropKey::new(prop.clone()));
            self.history.record(Some(before), Some(mkey));
            self.bump(false);
            ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
        }
        if is_final {
            self.history.end_merge();
        }
        Handled::Yes
    }

    fn apply_doc_action(&mut self, ctx: &mut DocCtx, action: &DtoAction) -> Handled {
        self.sync_cached_dtos(ctx);
        match action {
            DtoAction::AddField => {
                let name = self.draft_name.clone();
                let ty = self.draft_ty.clone();
                let cat = self.draft_category.clone();
                let is_name = self.draft_is_name;
                match self.add_field_named(&name, ty, &cat, is_name) {
                    Ok(_) => {
                        self.draft_name.clear();
                        self.draft_is_name = false;
                        ctx.redraw_props = true;
                        ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                    }
                    Err(err) => {
                        self.form_error = Some(err);
                        self.relayout();
                    }
                }
                Handled::Yes
            }
            DtoAction::SelectField(id) => {
                if self.selected_field != *id {
                    self.selected_field = *id;
                    self.bump(false);
                    ctx.redraw_props = true;
                }
                Handled::Yes
            }
            DtoAction::ToggleConstraints(id) => {
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
            DtoAction::ToggleDtoDefault(path) => {
                if let Some(&first) = path.first() {
                    self.selected_field = Some(first);
                }
                if !self.expanded_dtos.remove(path) {
                    self.expanded_dtos.insert(path.clone());
                }
                self.relayout();
                self.bump(false);
                ctx.redraw_props = true;
                Handled::Yes
            }
            DtoAction::SetConstraintOption { id, field, some } => {
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                let before = self.dto.clone();
                let f = &mut self.dto.fields[idx];
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
                    ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                }
                Handled::Yes
            }
            DtoAction::ResetConstraint { id, field } => {
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                let before = self.dto.clone();
                let f = &mut self.dto.fields[idx];
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
                    ConstraintField::UniqueInCollection => {
                        if !f.settings.unique_in_collection {
                            false
                        } else {
                            f.settings.unique_in_collection = false;
                            true
                        }
                    }
                };
                if changed {
                    self.history.record(Some(before), None);
                    self.relayout_constraints();
                    self.bump(rebuild_table);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                }
                Handled::Yes
            }
            DtoAction::MoveField { id, delta } => {
                if self.move_field(*id, *delta) {
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                }
                Handled::Yes
            }
            DtoAction::DeleteField(id) => {
                if self.delete_field(*id) {
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                }
                Handled::Yes
            }
            DtoAction::ResetFieldDefault(id) => self.reset_default_at_path(ctx, &[*id]),
            DtoAction::ResetNestedDefault(path) => self.reset_default_at_path(ctx, path),
            DtoAction::SetOptionSome { path, some } => {
                let Some((ty, _, _, _)) = self.field_default_meta_at_path(path) else {
                    return Handled::No;
                };
                let before = self.dto.clone();
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                let Some(slot) = self.get_default_mut_at_path(path) else {
                    return Handled::No;
                };
                if apply_set_option_some(&ty, slot, *some, &dtos, &enums) {
                    self.history.record(Some(before), None);
                    self.bump(true);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                }
                Handled::Yes
            }
            DtoAction::ContainerAdd { path } => {
                let Some((ty, _, _, _)) = self.field_default_meta_at_path(path) else {
                    return Handled::No;
                };
                let before = self.dto.clone();
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                let Some(slot) = self.get_default_mut_at_path(path) else {
                    return Handled::No;
                };
                if apply_container_add(&ty, slot, &dtos, &enums) {
                    self.expanded_dtos.insert(path.clone());
                    self.history.record(Some(before), None);
                    self.bump(true);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                }
                Handled::Yes
            }
            DtoAction::ContainerRemove { path, index } => {
                let Some((ty, _, _, _)) = self.field_default_meta_at_path(path) else {
                    return Handled::No;
                };
                let before = self.dto.clone();
                let Some(slot) = self.get_default_mut_at_path(path) else {
                    return Handled::No;
                };
                if apply_container_remove(&ty, slot, *index) {
                    self.history.record(Some(before), None);
                    self.bump(true);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                }
                Handled::Yes
            }
            DtoAction::ContainerMove { path, index, delta } => {
                let Some((ty, _, _, _)) = self.field_default_meta_at_path(path) else {
                    return Handled::No;
                };
                let before = self.dto.clone();
                let Some(slot) = self.get_default_mut_at_path(path) else {
                    return Handled::No;
                };
                if apply_container_move(&ty, slot, *index, *delta) {
                    self.history.record(Some(before), None);
                    self.bump(true);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                }
                Handled::Yes
            }
        }
    }

    fn apply_set_prop_value(&mut self, ctx: &mut DocCtx, prop: &DtoProp, value: &PropValue, merge: bool) -> Handled {
        self.sync_cached_dtos(ctx);
        let mkey = |p: &DtoProp| merge.then(|| MergeKey::from(PropKey::new(p.clone())));
        match prop {
            DtoProp::DraftName => {
                if let PropValue::Str(s) = value {
                    self.draft_name = s.clone();
                    if self.form_error.is_some() {
                        self.form_error = None;
                        self.relayout();
                    }
                    return Handled::Yes;
                }
                Handled::No
            }
            DtoProp::DraftType => {
                let Some(tag) = parse_type_tag_prop(value, FieldType::all()) else {
                    return Handled::No;
                };
                let first_dto = {
                    let non_self = self.first_non_self_dto_id();
                    if tag == FieldType::SoftRef(AssetUuid::NIL).tag() && non_self.is_nil() {
                        self.cached_dtos.values().find(|d| !d.id.is_nil()).map(|d| d.id).unwrap_or(AssetUuid::NIL)
                    } else {
                        non_self
                    }
                };
                let Some(next_ty) =
                    resolve_primary_field_type(&self.draft_ty, tag, Some(first_dto), Some(self.first_enum_id()))
                else {
                    return Handled::No;
                };
                if self.draft_ty != next_ty {
                    self.draft_ty = next_ty;
                    self.relayout();
                }
                Handled::Yes
            }
            DtoProp::DraftDtoTarget => {
                let PropValue::Str(s) = value else { return Handled::No };
                let target = if s.trim().is_empty() {
                    AssetUuid::NIL
                } else if let Some(id) = AssetUuid::parse(s.trim()) {
                    id
                } else {
                    return Handled::No;
                };
                if self.draft_ty.is_soft_ref() {
                    self.draft_ty = FieldType::SoftRef(target);
                } else {
                    self.draft_ty = FieldType::Dto(target);
                }
                Handled::Yes
            }
            DtoProp::DraftEnumTarget => {
                let PropValue::Str(s) = value else { return Handled::No };
                let target = if s.trim().is_empty() {
                    AssetUuid::NIL
                } else if let Some(id) = AssetUuid::parse(s.trim()) {
                    id
                } else {
                    return Handled::No;
                };
                self.draft_ty = FieldType::Enum(target);
                Handled::Yes
            }
            DtoProp::DraftGameAssetKind => {
                let PropValue::Str(s) = value else { return Handled::No };
                let Some(kind) = GameAssetKind::ALL.iter().copied().find(|k| k.label().eq_ignore_ascii_case(s.trim()))
                else {
                    return Handled::No;
                };
                self.draft_ty = FieldType::GameAsset(kind);
                Handled::Yes
            }
            DtoProp::DraftInnerType => {
                let first_dto = self.first_non_self_dto_id();
                let first_enum = self.first_enum_id();
                if apply_inner_type_change(&mut self.draft_ty, value, Some(first_dto), Some(first_enum)) {
                    self.relayout();
                }
                Handled::Yes
            }
            DtoProp::DraftInnerTarget => {
                if apply_inner_target_change(&mut self.draft_ty, value) {
                    self.relayout();
                }
                Handled::Yes
            }
            DtoProp::DraftKeyType => {
                let first_enum = self.first_enum_id();
                if apply_key_type_change(&mut self.draft_ty, value, Some(first_enum)) {
                    self.relayout();
                }
                Handled::Yes
            }
            DtoProp::DraftKeyTarget => {
                if apply_key_target_change(&mut self.draft_ty, value) {
                    self.relayout();
                }
                Handled::Yes
            }
            DtoProp::DraftCategory => {
                if let PropValue::Str(s) = value {
                    self.draft_category = s.clone();
                    return Handled::Yes;
                }
                Handled::No
            }
            DtoProp::DraftCategoryPick => {
                if let PropValue::Str(s) = value {
                    self.draft_category = s.clone();
                    self.relayout();
                    return Handled::Yes;
                }
                Handled::No
            }
            DtoProp::DraftIsName => {
                if let PropValue::Bool(b) = value {
                    self.draft_is_name = *b;
                    return Handled::Yes;
                }
                Handled::No
            }
            DtoProp::FieldName(id) => {
                let PropValue::Str(s) = value else { return Handled::No };
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                if self.dto.fields[idx].name == *s {
                    return Handled::Yes;
                }
                let before = self.dto.clone();
                self.dto.fields[idx].name = s.clone();
                self.history.record(Some(before), mkey(prop));
                self.bump(false);
                if self.expanded_constraints == Some(*id) {
                    self.relayout_constraints();
                }
                ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                Handled::Yes
            }
            DtoProp::FieldType(id) => {
                let Some(tag) = parse_type_tag_prop(value, FieldType::all()) else {
                    return Handled::No;
                };
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                let prev_ty = self.dto.fields[idx].ty.clone();
                let first_dto = {
                    let non_self = self.first_non_self_dto_id();
                    if tag == FieldType::SoftRef(AssetUuid::NIL).tag() && non_self.is_nil() {
                        self.cached_dtos.values().find(|d| !d.id.is_nil()).map(|d| d.id).unwrap_or(AssetUuid::NIL)
                    } else {
                        non_self
                    }
                };
                let Some(next_ty) =
                    resolve_primary_field_type(&prev_ty, tag, Some(first_dto), Some(self.first_enum_id()))
                else {
                    return Handled::No;
                };
                if self.dto.fields[idx].ty == next_ty {
                    return Handled::Yes;
                }
                let before = self.dto.clone();
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                coerce_default_for_type_change(&prev_ty, &next_ty, &mut self.dto.fields[idx].default, &dtos, &enums);
                if !(next_ty.is_dto() || next_ty.is_enum() || next_ty.is_option() || next_ty.is_container()) {
                    self.expanded_dtos.retain(|p| p.first() != Some(id));
                }
                self.variant_drafts.retain(|p, _| p.first() != Some(id));
                self.dto.fields[idx].ty = next_ty.clone();
                if !next_ty.is_numeric() {
                    self.dto.fields[idx].settings.min = None;
                    self.dto.fields[idx].settings.max = None;
                    self.dto.fields[idx].settings.step = None;
                    self.dto.fields[idx].settings.slider = false;
                }
                if !next_ty.is_string() {
                    self.dto.fields[idx].settings.multiline = false;
                    self.dto.fields[idx].settings.max_len = None;
                    self.dto.fields[idx].settings.unique_in_collection = false;
                }
                if !(next_ty.is_string() || next_ty.is_soft_ref() || next_ty.is_asset_ref() || next_ty.is_container()) {
                    self.dto.fields[idx].settings.required = false;
                }
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                Handled::Yes
            }
            DtoProp::FieldInnerType(id) => {
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                let prev_ty = self.dto.fields[idx].ty.clone();
                let mut next_ty = prev_ty.clone();
                let first_dto = self.first_non_self_dto_id();
                let first_enum = self.first_enum_id();
                if !apply_inner_type_change(&mut next_ty, value, Some(first_dto), Some(first_enum)) {
                    return Handled::Yes;
                }
                let before = self.dto.clone();
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                coerce_default_for_type_change(&prev_ty, &next_ty, &mut self.dto.fields[idx].default, &dtos, &enums);
                self.dto.fields[idx].ty = next_ty;
                self.expanded_dtos.retain(|p| p.first() != Some(id) || p.len() == 1);
                self.variant_drafts.retain(|p, _| p.first() != Some(id));
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                Handled::Yes
            }
            DtoProp::FieldInnerTarget(id) => {
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                let prev_ty = self.dto.fields[idx].ty.clone();
                let mut next_ty = prev_ty.clone();
                if !apply_inner_target_change(&mut next_ty, value) {
                    return Handled::Yes;
                }
                let before = self.dto.clone();
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                coerce_default_for_type_change(&prev_ty, &next_ty, &mut self.dto.fields[idx].default, &dtos, &enums);
                self.dto.fields[idx].ty = next_ty;
                self.expanded_dtos.retain(|p| p.first() != Some(id) || p.len() == 1);
                self.variant_drafts.retain(|p, _| p.first() != Some(id));
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                Handled::Yes
            }
            DtoProp::FieldKeyType(id) => {
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                let prev_ty = self.dto.fields[idx].ty.clone();
                let mut next_ty = prev_ty.clone();
                let first_enum = self.first_enum_id();
                if !apply_key_type_change(&mut next_ty, value, Some(first_enum)) {
                    return Handled::Yes;
                }
                let before = self.dto.clone();
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                coerce_default_for_type_change(&prev_ty, &next_ty, &mut self.dto.fields[idx].default, &dtos, &enums);
                self.dto.fields[idx].ty = next_ty;
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                Handled::Yes
            }
            DtoProp::FieldKeyTarget(id) => {
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                let prev_ty = self.dto.fields[idx].ty.clone();
                let mut next_ty = prev_ty.clone();
                if !apply_key_target_change(&mut next_ty, value) {
                    return Handled::Yes;
                }
                let before = self.dto.clone();
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                coerce_default_for_type_change(&prev_ty, &next_ty, &mut self.dto.fields[idx].default, &dtos, &enums);
                self.dto.fields[idx].ty = next_ty;
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                Handled::Yes
            }
            DtoProp::FieldDtoTarget(id) => {
                let PropValue::Str(s) = value else { return Handled::No };
                let target = if s.trim().is_empty() {
                    AssetUuid::NIL
                } else if let Some(tid) = AssetUuid::parse(s.trim()) {
                    tid
                } else {
                    return Handled::No;
                };
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                let is_soft_ref = self.dto.fields[idx].ty.is_soft_ref();
                let next_ty = if is_soft_ref { FieldType::SoftRef(target) } else { FieldType::Dto(target) };
                if self.dto.fields[idx].ty == next_ty {
                    return Handled::Yes;
                }
                let before = self.dto.clone();
                self.dto.fields[idx].ty = next_ty.clone();
                self.dto.fields[idx].default = if is_soft_ref {
                    DataValue::SoftRef(AssetUuid::NIL)
                } else {
                    next_ty.default_value_with_schemas(&self.cached_dtos, &self.cached_enums)
                };
                self.expanded_dtos.retain(|p| p.first() != Some(id) || p.len() == 1);
                self.variant_drafts.retain(|p, _| p.first() != Some(id));
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                Handled::Yes
            }
            DtoProp::FieldEnumTarget(id) => {
                let PropValue::Str(s) = value else { return Handled::No };
                let target = if s.trim().is_empty() {
                    AssetUuid::NIL
                } else if let Some(tid) = AssetUuid::parse(s.trim()) {
                    tid
                } else {
                    return Handled::No;
                };
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                let next_ty = FieldType::Enum(target);
                if self.dto.fields[idx].ty == next_ty {
                    return Handled::Yes;
                }
                let before = self.dto.clone();
                self.dto.fields[idx].ty = next_ty.clone();
                self.dto.fields[idx].default =
                    next_ty.default_value_with_schemas(&self.cached_dtos, &self.cached_enums);
                self.expanded_dtos.retain(|p| p.first() != Some(id) || p.len() == 1);
                self.variant_drafts.retain(|p, _| p.first() != Some(id));
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                Handled::Yes
            }
            DtoProp::FieldGameAssetKind(id) => {
                let PropValue::Str(s) = value else { return Handled::No };
                let Some(kind) = GameAssetKind::ALL.iter().copied().find(|k| k.label().eq_ignore_ascii_case(s.trim()))
                else {
                    return Handled::No;
                };
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                let next_ty = FieldType::GameAsset(kind);
                if self.dto.fields[idx].ty == next_ty {
                    return Handled::Yes;
                }
                let before = self.dto.clone();
                self.dto.fields[idx].ty = next_ty;
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                Handled::Yes
            }
            DtoProp::FieldCategory(id) => {
                let PropValue::Str(s) = value else { return Handled::No };
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                if self.dto.fields[idx].category == *s {
                    return Handled::Yes;
                }
                let before = self.dto.clone();
                self.dto.fields[idx].category = s.clone();
                self.history.record(Some(before), mkey(prop));
                self.bump(!merge);
                ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                Handled::Yes
            }
            DtoProp::FieldIsName(id) => {
                let PropValue::Bool(b) = value else { return Handled::No };
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                if self.dto.fields[idx].is_name == *b {
                    return Handled::Yes;
                }
                let before = self.dto.clone();
                self.dto.fields[idx].is_name = *b;
                self.history.record(Some(before), None);
                self.bump(false);
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                Handled::Yes
            }
            DtoProp::FieldDefault(id) => self.set_default_value_at_path(ctx, prop, &[*id], value, merge),
            DtoProp::NestedFieldDefault(path) => self.set_default_value_at_path(ctx, prop, path, value, merge),
            DtoProp::FieldDoc(id) => {
                let PropValue::Str(s) = value else { return Handled::No };
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                if self.dto.fields[idx].settings.doc == *s {
                    return Handled::Yes;
                }
                let was_empty = self.dto.fields[idx].settings.doc.is_empty();
                let before = self.dto.clone();
                self.dto.fields[idx].settings.doc = s.clone();
                let is_empty = self.dto.fields[idx].settings.doc.is_empty();
                self.history.record(Some(before), mkey(prop));
                if was_empty != is_empty && !merge {
                    self.relayout_constraints();
                }
                self.bump(false);
                ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                Handled::Yes
            }
            DtoProp::FieldMinEnabled(id) => {
                let PropValue::Bool(b) = value else { return Handled::No };
                self.apply_doc_action(
                    ctx,
                    &DtoAction::SetConstraintOption { id: *id, field: ConstraintField::Min, some: *b },
                )
            }
            DtoProp::FieldMaxEnabled(id) => {
                let PropValue::Bool(b) = value else { return Handled::No };
                self.apply_doc_action(
                    ctx,
                    &DtoAction::SetConstraintOption { id: *id, field: ConstraintField::Max, some: *b },
                )
            }
            DtoProp::FieldStepEnabled(id) => {
                let PropValue::Bool(b) = value else { return Handled::No };
                self.apply_doc_action(
                    ctx,
                    &DtoAction::SetConstraintOption { id: *id, field: ConstraintField::Step, some: *b },
                )
            }
            DtoProp::FieldSlider(id) => {
                let PropValue::Bool(b) = value else { return Handled::No };
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                if self.dto.fields[idx].settings.slider == *b {
                    return Handled::Yes;
                }
                let before = self.dto.clone();
                self.dto.fields[idx].settings.slider = *b;
                if *b {
                    if self.dto.fields[idx].settings.min.is_none() {
                        self.dto.fields[idx].settings.min = Some(0.0);
                    }
                    if self.dto.fields[idx].settings.max.is_none() {
                        let lo = self.dto.fields[idx].settings.min.unwrap_or(0.0);
                        self.dto.fields[idx].settings.max = Some(lo + 100.0);
                    }
                    Self::clamp_field_default(&mut self.dto.fields[idx]);
                }
                self.history.record(Some(before), None);
                self.relayout_constraints();
                self.bump(false);
                ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                Handled::Yes
            }
            DtoProp::FieldMultiline(id) => {
                let PropValue::Bool(b) = value else { return Handled::No };
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                if self.dto.fields[idx].settings.multiline == *b {
                    return Handled::Yes;
                }
                let before = self.dto.clone();
                self.dto.fields[idx].settings.multiline = *b;
                self.history.record(Some(before), None);
                self.bump(true);
                ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                Handled::Yes
            }
            DtoProp::FieldMaxLenEnabled(id) => {
                let PropValue::Bool(b) = value else { return Handled::No };
                self.apply_doc_action(
                    ctx,
                    &DtoAction::SetConstraintOption { id: *id, field: ConstraintField::MaxLen, some: *b },
                )
            }
            DtoProp::FieldRequired(id) => {
                let PropValue::Bool(b) = value else { return Handled::No };
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                if self.dto.fields[idx].settings.required == *b {
                    return Handled::Yes;
                }
                let before = self.dto.clone();
                self.dto.fields[idx].settings.required = *b;
                self.history.record(Some(before), None);
                self.relayout_constraints();
                self.bump(false);
                ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                Handled::Yes
            }
            DtoProp::FieldUniqueInCollection(id) => {
                let PropValue::Bool(b) = value else { return Handled::No };
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                if self.dto.fields[idx].settings.unique_in_collection == *b {
                    return Handled::Yes;
                }
                let before = self.dto.clone();
                self.dto.fields[idx].settings.unique_in_collection = *b;
                self.history.record(Some(before), None);
                self.relayout_constraints();
                self.bump(false);
                ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                Handled::Yes
            }
            DtoProp::FieldMin(_) | DtoProp::FieldMax(_) | DtoProp::FieldStep(_) | DtoProp::FieldMaxLen(_) => {
                Handled::No
            }
        }
    }

    fn apply_set_prop_component(
        &mut self,
        ctx: &mut DocCtx,
        prop: &DtoProp,
        comp: usize,
        value: f32,
        is_final: bool,
    ) -> Handled {
        self.sync_cached_dtos(ctx);
        let mkey = MergeKey::from(PropKey::new(prop.clone()));
        match prop {
            DtoProp::FieldDefault(id) => self.set_default_component_at_path(ctx, prop, &[*id], comp, value, is_final),
            DtoProp::NestedFieldDefault(path) => {
                self.set_default_component_at_path(ctx, prop, path, comp, value, is_final)
            }
            DtoProp::FieldMin(id) if comp == 0 => {
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                let next = if value.is_finite() { value as f64 } else { 0.0 };
                if self.dto.fields[idx].settings.min != Some(next) {
                    let before = self.dto.clone();
                    self.dto.fields[idx].settings.min = Some(next);
                    Self::clamp_field_default(&mut self.dto.fields[idx]);
                    self.history.record(Some(before), Some(mkey));
                    self.bump(false);
                    ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                }
                if is_final {
                    self.history.end_merge();
                }
                Handled::Yes
            }
            DtoProp::FieldMax(id) if comp == 0 => {
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                let next = if value.is_finite() { value as f64 } else { 100.0 };
                if self.dto.fields[idx].settings.max != Some(next) {
                    let before = self.dto.clone();
                    self.dto.fields[idx].settings.max = Some(next);
                    Self::clamp_field_default(&mut self.dto.fields[idx]);
                    self.history.record(Some(before), Some(mkey));
                    self.bump(false);
                    ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                }
                if is_final {
                    self.history.end_merge();
                }
                Handled::Yes
            }
            DtoProp::FieldStep(id) if comp == 0 => {
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                let next = if value.is_finite() && value > 0.0 { value as f64 } else { 1.0 };
                if self.dto.fields[idx].settings.step != Some(next) {
                    let before = self.dto.clone();
                    self.dto.fields[idx].settings.step = Some(next);
                    self.history.record(Some(before), Some(mkey));
                    self.bump(false);
                    ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                }
                if is_final {
                    self.history.end_merge();
                }
                Handled::Yes
            }
            DtoProp::FieldMaxLen(id) if comp == 0 => {
                let Some(idx) = self.dto.fields.iter().position(|f| f.id == *id) else {
                    return Handled::No;
                };
                let next = if value.is_finite() { value.round().max(1.0) as usize } else { 64 };
                if self.dto.fields[idx].settings.max_len != Some(next) {
                    let before = self.dto.clone();
                    self.dto.fields[idx].settings.max_len = Some(next);
                    Self::clamp_field_default(&mut self.dto.fields[idx]);
                    self.history.record(Some(before), Some(mkey));
                    self.bump(false);
                    ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
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

impl TabDoc for DtoDoc {
    fn asset(&self) -> Option<&AssetRef> {
        Some(&self.aref)
    }

    fn view(&self) -> ViewKindId {
        DTO_VIEW
    }

    fn base_title(&self) -> String {
        format!("quillshade/data/{}.dto", self.path)
    }

    fn icon(&self) -> IconRef {
        DTO_ICON
    }

    fn watermark(&self) -> Option<&'static str> {
        Some("DTO")
    }

    fn is_dirty(&self) -> bool {
        self.dto != self.saved
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
        self.sync_cached_dtos(ctx);
        let file = project.root.join(self.aref.source_rel());
        save_dto_with_schemas_and_index(&file, &mut self.dto, &self.cached_dtos, &self.cached_enums, Some(ctx.index))?;
        self.cached_dtos.insert(self.path.clone(), self.dto.clone());
        self.saved = self.dto.clone();
        self.bump(false);
        ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
        Ok(())
    }

    fn changed_on_disk(&self, project: &Project) -> bool {
        let file = project.root.join(self.aref.source_rel());
        load_dto(&file).is_ok_and(|disk| disk != self.saved)
    }

    fn reload_from_disk(&mut self, ctx: &mut DocCtx) -> Result<(), String> {
        let Some(project) = ctx.project else {
            return Err("No open project".into());
        };
        let file = project.root.join(self.aref.source_rel());
        let mut loaded = load_dto(&file).map_err(|e| e.to_string())?;
        loaded.ensure_id();
        self.dto = loaded.clone();
        self.saved = loaded;
        self.history = History::default();
        self.sync_cached_dtos(ctx);
        if self.selected_field.is_none_or(|id| !self.dto.fields.iter().any(|f| f.id == id)) {
            self.selected_field = self.dto.fields.first().map(|f| f.id);
        }
        if self.expanded_constraints.is_some_and(|id| !self.dto.fields.iter().any(|f| f.id == id)) {
            self.expanded_constraints = None;
        }
        self.prune_stale_expanded_dtos();
        self.bump(true);
        ctx.redraw_props = true;
        ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
        Ok(())
    }

    fn keep_mine(&mut self, project: &Project) {
        let file = project.root.join(self.aref.source_rel());
        if let Ok(disk) = load_dto(&file) {
            self.saved = disk;
            self.bump(false);
        }
    }

    fn rename(&mut self, _project: &Project, from: &AssetRef, to: &AssetRef) {
        if &self.aref == from {
            self.cached_dtos.remove(&self.path);
            self.path = to.path.clone();
            self.aref = to.clone();
            if !self.dto.id.is_nil() {
                self.cached_dtos.insert(self.path.clone(), self.dto.clone());
            }
            self.bump(true);
        }
    }

    fn config_key(&self) -> Option<String> {
        Some(self.aref.source_rel())
    }

    fn status(&self, ctx: &StatusCtx) -> Option<StatusInfo> {
        let n_fields = self.dto.fields.len();
        let fields_label = if n_fields == 1 { "1 field".to_string() } else { format!("{n_fields} fields") };
        let n_cats = self.dto.categories().len();
        let cats_label = if n_cats == 1 { "1 category".to_string() } else { format!("{n_cats} categories") };
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
        let rust_lines = emit_dto_file_with_schemas(&self.path, &self.dto, dtos, enums).lines().count();
        Some(StatusInfo::new(self.is_dirty(), ctx, vec![fields_label, cats_label, format!("{rust_lines} Rust lines")]))
    }

    fn apply(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        self.sync_cached_dtos(ctx);
        match action {
            TabAction::Undo => {
                if self.history.undo_keeping(&mut self.dto, |m| &mut m.id) {
                    if self.selected_field.is_none_or(|id| !self.dto.fields.iter().any(|f| f.id == id)) {
                        self.selected_field = self.dto.fields.first().map(|f| f.id);
                    }
                    if self.expanded_constraints.is_some_and(|id| !self.dto.fields.iter().any(|f| f.id == id)) {
                        self.expanded_constraints = None;
                    }
                    self.prune_stale_expanded_dtos();
                    self.bump(true);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                }
                Handled::Yes
            }
            TabAction::Redo => {
                if self.history.redo_keeping(&mut self.dto, |m| &mut m.id) {
                    if self.selected_field.is_none_or(|id| !self.dto.fields.iter().any(|f| f.id == id)) {
                        self.selected_field = self.dto.fields.first().map(|f| f.id);
                    }
                    if self.expanded_constraints.is_some_and(|id| !self.dto.fields.iter().any(|f| f.id == id)) {
                        self.expanded_constraints = None;
                    }
                    self.prune_stale_expanded_dtos();
                    self.bump(true);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncDtoInMemory { path: self.path.clone(), dto: self.dto.clone() });
                }
                Handled::Yes
            }
            TabAction::CommitInline => {
                self.history.end_merge();
                Handled::Yes
            }
            TabAction::DeleteSelection => {
                if let Some(id) = self.selected_field {
                    return self.apply_doc_action(ctx, &DtoAction::DeleteField(id));
                }
                Handled::No
            }
            TabAction::SetValue { key, value, merge } => {
                let Some(prop) = key.downcast::<DtoProp>() else {
                    return Handled::No;
                };
                self.apply_set_prop_value(ctx, prop, value, *merge)
            }
            TabAction::SetComponent { key, comp, value, is_final } => {
                let Some(prop) = key.downcast::<DtoProp>() else {
                    return Handled::No;
                };
                self.apply_set_prop_component(ctx, prop, *comp, *value, *is_final)
            }
            TabAction::Doc(payload) => {
                let Some(act) = payload.downcast::<DtoAction>() else {
                    return Handled::No;
                };
                self.apply_doc_action(ctx, act)
            }
            _ => Handled::No,
        }
    }

    fn current_value(&self, key: &PropKey, ctx: &StatusCtx) -> Option<PropValue> {
        let prop = key.downcast::<DtoProp>()?;
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
            DtoProp::DraftName => Some(PropValue::Str(self.draft_name.clone())),
            DtoProp::DraftType => Some(PropValue::Str(self.draft_ty.label().to_string())),
            DtoProp::DraftDtoTarget => {
                let target =
                    self.draft_ty.dto_target().or_else(|| self.draft_ty.soft_ref_target()).unwrap_or(AssetUuid::NIL);
                Some(PropValue::Str(if target.is_nil() { String::new() } else { target.to_string() }))
            }
            DtoProp::DraftEnumTarget => {
                let target = self.draft_ty.enum_target().unwrap_or(AssetUuid::NIL);
                Some(PropValue::Str(if target.is_nil() { String::new() } else { target.to_string() }))
            }
            DtoProp::DraftGameAssetKind => {
                let kind = match self.draft_ty {
                    FieldType::GameAsset(k) => k,
                    _ => GameAssetKind::Any,
                };
                Some(PropValue::Str(kind.label().to_string()))
            }
            DtoProp::DraftInnerType => self.draft_ty.inner_type().map(|t| PropValue::Str(t.tag().to_string())),
            DtoProp::DraftInnerTarget => {
                let inner = self.draft_ty.inner_type()?;
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
            DtoProp::DraftKeyType => self.draft_ty.key_type().map(|t| PropValue::Str(t.tag().to_string())),
            DtoProp::DraftKeyTarget => {
                let key = self.draft_ty.key_type()?;
                let u = key.enum_target()?;
                Some(PropValue::Str(if u.is_nil() { String::new() } else { u.to_string() }))
            }
            DtoProp::DraftCategory | DtoProp::DraftCategoryPick => Some(PropValue::Str(self.draft_category.clone())),
            DtoProp::DraftIsName => Some(PropValue::Bool(self.draft_is_name)),
            DtoProp::FieldName(id) => {
                self.dto.fields.iter().find(|f| f.id == *id).map(|f| PropValue::Str(f.name.clone()))
            }
            DtoProp::FieldType(id) => {
                self.dto.fields.iter().find(|f| f.id == *id).map(|f| PropValue::Str(f.ty.label().to_string()))
            }
            DtoProp::FieldDtoTarget(id) => {
                let f = self.dto.fields.iter().find(|f| f.id == *id)?;
                let target = f.ty.dto_target().or_else(|| f.ty.soft_ref_target()).unwrap_or(AssetUuid::NIL);
                Some(PropValue::Str(if target.is_nil() { String::new() } else { target.to_string() }))
            }
            DtoProp::FieldEnumTarget(id) => {
                let f = self.dto.fields.iter().find(|f| f.id == *id)?;
                let target = f.ty.enum_target().unwrap_or(AssetUuid::NIL);
                Some(PropValue::Str(if target.is_nil() { String::new() } else { target.to_string() }))
            }
            DtoProp::FieldGameAssetKind(id) => {
                let f = self.dto.fields.iter().find(|f| f.id == *id)?;
                let kind = match f.ty {
                    FieldType::GameAsset(k) => k,
                    _ => GameAssetKind::Any,
                };
                Some(PropValue::Str(kind.label().to_string()))
            }
            DtoProp::FieldInnerType(id) => {
                let f = self.dto.fields.iter().find(|f| f.id == *id)?;
                f.ty.inner_type().map(|t| PropValue::Str(t.tag().to_string()))
            }
            DtoProp::FieldInnerTarget(id) => {
                let f = self.dto.fields.iter().find(|f| f.id == *id)?;
                let inner = f.ty.inner_type()?;
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
            DtoProp::FieldKeyType(id) => {
                let f = self.dto.fields.iter().find(|f| f.id == *id)?;
                f.ty.key_type().map(|t| PropValue::Str(t.tag().to_string()))
            }
            DtoProp::FieldKeyTarget(id) => {
                let f = self.dto.fields.iter().find(|f| f.id == *id)?;
                let key = f.ty.key_type()?;
                let u = key.enum_target()?;
                Some(PropValue::Str(if u.is_nil() { String::new() } else { u.to_string() }))
            }
            DtoProp::FieldCategory(id) => {
                self.dto.fields.iter().find(|f| f.id == *id).map(|f| PropValue::Str(f.category.clone()))
            }
            DtoProp::FieldIsName(id) => {
                self.dto.fields.iter().find(|f| f.id == *id).map(|f| PropValue::Bool(f.is_name))
            }
            DtoProp::FieldDefault(id) => {
                let (ty, _, schema_default, _) = self.dto.field_meta_at_path_with_schemas(&[*id], dtos, enums, true)?;
                let val = self.get_default_at_path(&[*id], &schema_default);
                data_value_to_prop(&ty, val)
            }
            DtoProp::NestedFieldDefault(path) => {
                let (ty, _, schema_default, _) = self.dto.field_meta_at_path_with_schemas(path, dtos, enums, true)?;
                let val = self.get_default_at_path(path, &schema_default);
                data_value_to_prop(&ty, val)
            }
            DtoProp::FieldDoc(id) => {
                self.dto.fields.iter().find(|f| f.id == *id).map(|f| PropValue::Str(f.settings.doc.clone()))
            }
            DtoProp::FieldMinEnabled(id) => {
                self.dto.fields.iter().find(|f| f.id == *id).map(|f| PropValue::Bool(f.settings.min.is_some()))
            }
            DtoProp::FieldMin(id) => self
                .dto
                .fields
                .iter()
                .find(|f| f.id == *id)
                .and_then(|f| f.settings.min)
                .map(|v| PropValue::Float(v as f32)),
            DtoProp::FieldMaxEnabled(id) => {
                self.dto.fields.iter().find(|f| f.id == *id).map(|f| PropValue::Bool(f.settings.max.is_some()))
            }
            DtoProp::FieldMax(id) => self
                .dto
                .fields
                .iter()
                .find(|f| f.id == *id)
                .and_then(|f| f.settings.max)
                .map(|v| PropValue::Float(v as f32)),
            DtoProp::FieldStepEnabled(id) => {
                self.dto.fields.iter().find(|f| f.id == *id).map(|f| PropValue::Bool(f.settings.step.is_some()))
            }
            DtoProp::FieldStep(id) => self
                .dto
                .fields
                .iter()
                .find(|f| f.id == *id)
                .and_then(|f| f.settings.step)
                .map(|v| PropValue::Float(v as f32)),
            DtoProp::FieldSlider(id) => {
                self.dto.fields.iter().find(|f| f.id == *id).map(|f| PropValue::Bool(f.settings.slider))
            }
            DtoProp::FieldMultiline(id) => {
                self.dto.fields.iter().find(|f| f.id == *id).map(|f| PropValue::Bool(f.settings.multiline))
            }
            DtoProp::FieldMaxLenEnabled(id) => {
                self.dto.fields.iter().find(|f| f.id == *id).map(|f| PropValue::Bool(f.settings.max_len.is_some()))
            }
            DtoProp::FieldMaxLen(id) => self
                .dto
                .fields
                .iter()
                .find(|f| f.id == *id)
                .and_then(|f| f.settings.max_len)
                .map(|v| PropValue::Float(v as f32)),
            DtoProp::FieldRequired(id) => {
                self.dto.fields.iter().find(|f| f.id == *id).map(|f| PropValue::Bool(f.settings.required))
            }
            DtoProp::FieldUniqueInCollection(id) => {
                self.dto.fields.iter().find(|f| f.id == *id).map(|f| PropValue::Bool(f.settings.unique_in_collection))
            }
        }
    }
}
