//! Shared value-editor widgets and value-mutation helpers for all [`FieldType`]s (Slice 1 primitives, Slice 2 nested
//! DTOs, and Slice 3 direct asset references).
//!
//! Used by both the `.dto.ron` schema editor (for field default values) and the `.collection.ron`
//! editor (for row cell values). All widgets emit standard `editor_core` property actions
//! ([`TabAction::SetValue`] / [`TabAction::SetComponent`]) keyed by [`PropKey`].

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};

use bevy::feathers::controls::{
    ButtonVariant, ColorInputValue, FeathersCheckbox, FeathersColorInput, FeathersToolButton, NumberInputValue,
    color_input_self_update,
};
use bevy::input_focus::InputFocus;
use bevy::math::{EulerRot, Quat};
use bevy::prelude::*;
use bevy::ui::Checked;

use project_core::common::curve::CurveData;
use project_core::common::gradient::RawGradient;
use project_core::{AssetIndex, AssetRef, AssetUuid, DomainId, Project};

use editor_core::action::Action;
use editor_core::dynval::{PropKey, PropValue};
use editor_core::state::editor::Editor;
use editor_core::theme::{ACCENT, BORDER, PANEL_BG, TEXT, TEXT_DIM, text};
use editor_core::tooltip::Tooltip;
use editor_core::ui::OnActivate;
use editor_core::ui::icons::{IconId, icon};
use editor_core::ui::props::{PropField, checkbox_w, linear_color, number_input, select_values, text_area, text_w};
use editor_core::ui::widgets::{Caption, GroupButton, button_group};

use crate::backend::DATA;
use crate::backend::bake::DataBakeResolver;
use crate::backend::model::{
    DataValue, DtoAsset, EnumAsset, Field, FieldSettings, FieldType, GameAssetKind, MAX_NESTING_DEPTH,
    ToShortViewString, collection_ref, path_step_item, path_step_map_key, path_step_map_val, path_step_option_some,
    to_pascal_case,
};
use crate::backend::validate::DataAssets;

use super::state::{DataState, DataStep};

/// Marker component on color swatch widgets spawned by [`data_value_editor`] for `FieldType::Color`.
#[derive(Component, Default, Clone, Copy, Debug)]
pub struct DataColorSwatch;

/// Marker component on the 64x20 px inline curve mini-preview spawned for `FieldType::Curve`.
#[derive(Component, Default, Clone, Copy, Debug)]
pub struct DataCurveMiniPreview;

/// Marker component on the 64x20 px inline gradient mini-preview spawned for `FieldType::Gradient`.
#[derive(Component, Default, Clone, Copy, Debug)]
pub struct DataGradientMiniPreview;

/// Marker component on the Jump (`OpenExternal`) button spawned for a populated asset reference in [`data_value_editor_with_catalog`].
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct DataJumpAssetButton(pub AssetRef);

/// Resolved slot entry `(slot_id, slot_name, resolved_gradient)` inside [`AssetPickerCatalog::gradients`].
pub type GradientSlotEntry = (u32, String, RawGradient);

/// Resolved row entry `(row_uuid, display_label, collection_asset_ref)` inside [`AssetPickerCatalog::soft_ref_rows`].
pub type SoftRefRowEntry = (AssetUuid, String, AssetRef);

/// Pre-resolved catalog of cross-domain assets, raw game files, and collection rows for Slice 3 & Slice 4 pickers and mini-previews.
#[derive(Debug, Clone, Default)]
pub struct AssetPickerCatalog {
    /// `(uuid, "<domain>:<path>", resolved_curve)` sorted by label.
    pub curves: Vec<(AssetUuid, String, CurveData)>,
    /// `(uuid, "<path>", Vec<GradientSlotEntry>)` sorted by path.
    pub gradients: Vec<(AssetUuid, String, Vec<GradientSlotEntry>)>,
    /// `(uuid, "<path>")` sorted by path.
    pub materials: Vec<(AssetUuid, String)>,
    /// `(uuid, "<domain>:<path>")` sorted by label.
    pub textures: Vec<(AssetUuid, String)>,
    /// `(uuid, "<path>")` sorted by path.
    pub environments: Vec<(AssetUuid, String)>,
    /// Relative file paths under `assets/` (excluding `quillshade/**`), sorted.
    pub game_files: Vec<String>,
    /// Lookup from UUID to `AssetRef` for Jump (`↗`) buttons.
    pub refs_by_id: BTreeMap<AssetUuid, AssetRef>,
    /// `(row_uuid, "<col_path> · #<idx> <row_short_view>", col_asset_ref)` per target DTO UUID.
    pub soft_ref_rows: BTreeMap<AssetUuid, Vec<SoftRefRowEntry>>,
    /// Lookup from Enum `AssetUuid` to `EnumAsset` for `FieldType::Enum` variant selectors.
    pub enums_by_id: BTreeMap<AssetUuid, EnumAsset>,
}

impl AssetPickerCatalog {
    /// Builds the asset picker catalog from `assets`, `index`, and `project`.
    pub fn from_state(assets: Option<&DataAssets>, idx: &AssetIndex, project: Option<&Project>) -> Self {
        let mut cat = Self::default();
        let resolver = DataBakeResolver { project, index: Some(idx) };
        let mat_dom = DomainId::new("material");
        let light_dom = DomainId::new("light");

        for dom in [DATA, mat_dom, light_dom] {
            for entry in idx.assets_of(dom, "curve") {
                cat.refs_by_id.insert(entry.id, entry.asset.clone());
                let label = format!("{}:{}", dom.as_str(), entry.asset.path);
                let curve = if dom == DATA
                    && let Some(c) = assets.and_then(|a| a.curves.get(&entry.asset.path))
                {
                    CurveData { interp: Default::default(), min: c.min, max: c.max, points: c.points.clone() }
                } else {
                    resolver.resolve_curve(entry.id)
                };
                cat.curves.push((entry.id, label, curve));
            }
        }
        for dom in [mat_dom, light_dom] {
            for entry in idx.assets_of(dom, "texture") {
                cat.refs_by_id.insert(entry.id, entry.asset.clone());
                let label = format!("{}:{}", dom.as_str(), entry.asset.path);
                cat.textures.push((entry.id, label));
            }
        }
        for entry in idx.assets_of(mat_dom, "gradients") {
            cat.refs_by_id.insert(entry.id, entry.asset.clone());
            let mut slots: Vec<(u32, String, RawGradient)> = entry
                .sub_items
                .iter()
                .filter_map(|sub| u32::try_from(sub.id).ok().filter(|&s| s > 0).map(|s| (s, sub.label.clone())))
                .map(|(slot_id, slot_label)| (slot_id, slot_label, resolver.resolve_gradient(entry.id, slot_id)))
                .collect();
            slots.sort_by_key(|(s, _, _)| *s);
            cat.gradients.push((entry.id, entry.asset.path.clone(), slots));
        }
        for entry in idx.assets_of(mat_dom, "material") {
            cat.refs_by_id.insert(entry.id, entry.asset.clone());
            cat.materials.push((entry.id, entry.asset.path.clone()));
        }
        for entry in idx.assets_of(light_dom, "environment") {
            cat.refs_by_id.insert(entry.id, entry.asset.clone());
            cat.environments.push((entry.id, entry.asset.path.clone()));
        }
        cat.curves.sort_by(|a, b| a.1.cmp(&b.1));
        cat.gradients.sort_by(|a, b| a.1.cmp(&b.1));
        cat.materials.sort_by(|a, b| a.1.cmp(&b.1));
        cat.textures.sort_by(|a, b| a.1.cmp(&b.1));
        cat.environments.sort_by(|a, b| a.1.cmp(&b.1));

        let mut files: BTreeSet<String> = BTreeSet::new();
        if let Some(project) = project {
            for p in project_core::game_files::scan(project, |_| true) {
                files.insert(p);
            }
        }
        cat.game_files = files.into_iter().collect();

        if let Some(assets) = assets {
            for en in assets.enums.values() {
                if !en.id.is_nil() {
                    cat.enums_by_id.insert(en.id, en.clone());
                }
            }
            for (col_path, col) in &assets.collections {
                if col.dto.is_nil() {
                    continue;
                }
                let dto_opt = assets.dtos.values().find(|d| !d.id.is_nil() && d.id == col.dto);
                let col_aref = collection_ref(col_path);
                let entries = cat.soft_ref_rows.entry(col.dto).or_default();
                for (row_idx, row) in col.rows.iter().enumerate() {
                    if row.id.is_nil() {
                        continue;
                    }
                    let label = match dto_opt {
                        Some(dto) => format!(
                            "{col_path} · #{row_idx} {}",
                            row.short_view_string_with_schemas(dto, &assets.dtos, &assets.enums)
                        ),
                        None => format!("{col_path} · #{row_idx}"),
                    };
                    entries.push((row.id, label, col_aref.clone()));
                }
            }
        }
        cat
    }

    /// Builds the asset picker catalog from `editor.asset_index`, `editor.project`, and `DataState`.
    pub fn from_editor(editor: &Editor) -> Self {
        let assets = editor.domains.get::<DataState>().map(|st| &st.assets);
        let mut cat = Self::from_state(assets, &editor.asset_index, editor.project.as_ref());
        if !editor.game_files.is_empty() {
            let mut files: BTreeSet<String> = cat.game_files.into_iter().collect();
            for p in editor.game_files.iter().filter(|p| !p.starts_with("quillshade/")) {
                files.insert(p.clone());
            }
            cat.game_files = files.into_iter().collect();
        }
        cat
    }

    /// Returns all `(row_uuid, display_label, col_asset_ref)` entries across all collections bound to `dto_id`.
    pub fn soft_ref_rows_for(&self, dto_id: AssetUuid) -> &[SoftRefRowEntry] {
        self.soft_ref_rows.get(&dto_id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Finds the `(row_uuid, display_label, col_asset_ref)` entry for `row_id` in collections bound to `dto_id`.
    pub fn find_soft_ref_row(&self, dto_id: AssetUuid, row_id: AssetUuid) -> Option<&SoftRefRowEntry> {
        self.soft_ref_rows_for(dto_id).iter().find(|(id, _, _)| *id == row_id)
    }
}

/// Converts a unit quaternion `[x, y, z, w]` to Euler XYZ angles in degrees `[rx, ry, rz]`.
pub fn quat_to_euler_deg(q: [f32; 4]) -> [f32; 3] {
    let quat = Quat::from_array(q);
    let norm = if quat.length_squared() > 1e-8 { quat.normalize() } else { Quat::IDENTITY };
    let (x, y, z) = norm.to_euler(EulerRot::XYZ);
    let clean = |d: f32| if d.abs() < 1e-4 { 0.0 } else { d };
    [clean(x.to_degrees()), clean(y.to_degrees()), clean(z.to_degrees())]
}

/// Converts Euler XYZ angles in degrees `[rx, ry, rz]` to a normalized unit quaternion `[x, y, z, w]`.
pub fn euler_deg_to_quat(deg: [f32; 3]) -> [f32; 4] {
    let q = Quat::from_euler(EulerRot::XYZ, deg[0].to_radians(), deg[1].to_radians(), deg[2].to_radians()).normalize();
    let clean = |v: f32| if v.abs() < 1e-6 { 0.0 } else { v };
    let arr = q.to_array();
    [clean(arr[0]), clean(arr[1]), clean(arr[2]), clean(arr[3])]
}

/// Converts a [`DataValue`] of the given [`FieldType`] into a [`PropValue`] for `TabDoc::current_value`
/// and live widget synchronization.
pub fn data_value_to_prop(ty: impl Borrow<FieldType>, val: &DataValue) -> Option<PropValue> {
    let ty = ty.borrow();
    match (ty, val) {
        (FieldType::Bool, DataValue::Bool(b)) => Some(PropValue::Bool(*b)),
        (FieldType::I8, DataValue::I8(v)) => Some(PropValue::Float(*v as f32)),
        (FieldType::I16, DataValue::I16(v)) => Some(PropValue::Float(*v as f32)),
        (FieldType::I32, DataValue::I32(v)) => Some(PropValue::Float(*v as f32)),
        (FieldType::I64, DataValue::I64(v)) => Some(PropValue::Float(*v as f32)),
        (FieldType::U8, DataValue::U8(v)) => Some(PropValue::Float(*v as f32)),
        (FieldType::U16, DataValue::U16(v)) => Some(PropValue::Float(*v as f32)),
        (FieldType::U32, DataValue::U32(v)) => Some(PropValue::Float(*v as f32)),
        (FieldType::U64, DataValue::U64(v)) => Some(PropValue::Float(*v as f32)),
        (FieldType::F32, DataValue::F32(v)) => Some(PropValue::Float(*v)),
        (FieldType::F64, DataValue::F64(v)) => Some(PropValue::Float(*v as f32)),
        (FieldType::String, DataValue::String(s)) | (FieldType::LocalizedString, DataValue::LocalizedString(s)) => {
            Some(PropValue::Str(s.clone()))
        }
        (FieldType::Vec2, DataValue::Vec2(v)) => Some(PropValue::Float2(*v)),
        (FieldType::Vec3, DataValue::Vec3(v)) => Some(PropValue::Float3(*v)),
        (FieldType::Vec4, DataValue::Vec4(v)) => Some(PropValue::Float4(*v)),
        (FieldType::IVec2, DataValue::IVec2(v)) => Some(PropValue::Float2([v[0] as f32, v[1] as f32])),
        (FieldType::IVec3, DataValue::IVec3(v)) => Some(PropValue::Float3([v[0] as f32, v[1] as f32, v[2] as f32])),
        (FieldType::UVec2, DataValue::UVec2(v)) => Some(PropValue::Float2([v[0] as f32, v[1] as f32])),
        (FieldType::UVec3, DataValue::UVec3(v)) => Some(PropValue::Float3([v[0] as f32, v[1] as f32, v[2] as f32])),
        (FieldType::Quat, DataValue::Quat(q)) => Some(PropValue::Float3(quat_to_euler_deg(*q))),
        (FieldType::Color, DataValue::Color(rgba)) => Some(PropValue::Float4(*rgba)),
        (FieldType::Duration, DataValue::Duration(secs)) => Some(PropValue::Float(*secs)),
        (FieldType::RangeF32, DataValue::RangeF32(r)) => Some(PropValue::Float2(*r)),
        (FieldType::RangeI32, DataValue::RangeI32(r)) => Some(PropValue::Float2([r[0] as f32, r[1] as f32])),
        (FieldType::Curve, DataValue::Curve(u))
        | (FieldType::Material, DataValue::Material(u))
        | (FieldType::Texture, DataValue::Texture(u))
        | (FieldType::LightEnvironment, DataValue::LightEnvironment(u))
        | (FieldType::SoftRef(_), DataValue::SoftRef(u))
        | (FieldType::Enum(_), DataValue::Enum { variant: u, .. }) => {
            Some(PropValue::Str(if u.is_nil() { String::new() } else { u.to_string() }))
        }
        (FieldType::Gradient, DataValue::Gradient(file, slot)) => {
            Some(PropValue::Str(if file.is_nil() { String::new() } else { format!("{file}:{slot}") }))
        }
        (FieldType::GameAsset(_), DataValue::GameAsset(p)) => Some(PropValue::Str(p.clone())),
        _ => None,
    }
}

/// In-memory per-variant payload drafts (`variant_uuid` -> `payload_field_id` -> `DataValue`).
pub type VariantPayloadDrafts = BTreeMap<AssetUuid, BTreeMap<AssetUuid, DataValue>>;

/// Switches a `DataValue::Enum { variant, fields }` to `next_variant_id` while saving the outgoing variant's
/// payload fields into `drafts` and restoring any previously edited payload fields for `next_variant_id`
/// (falling back to schema defaults for missing or mismatched fields). Returns `true` when `slot` changed.
pub fn switch_enum_variant_with_drafts(
    slot: &mut DataValue,
    target_enum: Option<&EnumAsset>,
    next_variant_id: AssetUuid,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    drafts: &mut VariantPayloadDrafts,
) -> bool {
    if let DataValue::Enum { variant: cur_var, fields: cur_fields } = slot {
        if *cur_var == next_variant_id {
            return false;
        }
        if !cur_var.is_nil() {
            drafts.insert(*cur_var, cur_fields.clone());
        }
    }
    let mut next_fields = drafts.get(&next_variant_id).cloned().unwrap_or_default();
    if let Some(en) = target_enum
        && let Some(var_def) = en.variant_by_id(next_variant_id)
    {
        let valid_ids: BTreeSet<AssetUuid> = var_def.fields.iter().map(|vf| vf.id).collect();
        next_fields.retain(|k, _| valid_ids.contains(k));
        for vf in &var_def.fields {
            let def_val = vf.default_with_schemas(dtos, enums);
            let entry = next_fields.entry(vf.id).or_insert_with(|| def_val.clone());
            if !vf.ty.matches(entry) {
                *entry = def_val;
            }
        }
    }
    let next_val = DataValue::Enum { variant: next_variant_id, fields: next_fields };
    if *slot == next_val {
        return false;
    }
    *slot = next_val;
    true
}

fn clamp_f64(mut v: f64, settings: &FieldSettings) -> f64 {
    if !v.is_finite() {
        v = 0.0;
    }
    if let Some(min) = settings.min
        && v < min
    {
        v = min;
    }
    if let Some(max) = settings.max
        && v > max
    {
        v = max;
    }
    v
}

/// Applies a [`PropValue`] (`TabAction::SetValue`) to `target_val` for `ty`. Returns `true` when `target_val` changed.
pub fn apply_set_value(
    ty: impl Borrow<FieldType>,
    target_val: &mut DataValue,
    prop: &PropValue,
    settings: &FieldSettings,
) -> bool {
    let ty = ty.borrow();
    if !ty.matches(target_val) {
        *target_val = ty.default_value();
    }
    match (ty, target_val, prop) {
        (FieldType::Bool, DataValue::Bool(cur), PropValue::Bool(b)) => {
            if *cur == *b {
                return false;
            }
            *cur = *b;
            true
        }
        (FieldType::String, DataValue::String(cur), PropValue::Str(s))
        | (FieldType::LocalizedString, DataValue::LocalizedString(cur), PropValue::Str(s)) => {
            let next = if let Some(max_len) = settings.max_len
                && s.chars().count() > max_len
            {
                s.chars().take(max_len).collect::<String>()
            } else {
                s.clone()
            };
            if *cur == next {
                return false;
            }
            *cur = next;
            true
        }
        (FieldType::Curve, DataValue::Curve(cur), PropValue::Str(s))
        | (FieldType::Material, DataValue::Material(cur), PropValue::Str(s))
        | (FieldType::Texture, DataValue::Texture(cur), PropValue::Str(s))
        | (FieldType::LightEnvironment, DataValue::LightEnvironment(cur), PropValue::Str(s))
        | (FieldType::SoftRef(_), DataValue::SoftRef(cur), PropValue::Str(s)) => {
            let next = if s.is_empty() { AssetUuid::NIL } else { AssetUuid::parse(s).unwrap_or(AssetUuid::NIL) };
            if *cur == next {
                return false;
            }
            *cur = next;
            true
        }
        (FieldType::Enum(_), DataValue::Enum { variant, fields }, PropValue::Str(s)) => {
            let next = if s.is_empty() { AssetUuid::NIL } else { AssetUuid::parse(s).unwrap_or(AssetUuid::NIL) };
            if *variant == next {
                return false;
            }
            *variant = next;
            fields.clear();
            true
        }
        (FieldType::Gradient, DataValue::Gradient(file, slot), PropValue::Str(s)) => {
            let (next_file, next_slot) = if s.is_empty() {
                (AssetUuid::NIL, 0)
            } else if let Some((f_str, slot_str)) = s.split_once(':') {
                (AssetUuid::parse(f_str).unwrap_or(AssetUuid::NIL), slot_str.parse::<u32>().unwrap_or(0))
            } else {
                (AssetUuid::parse(s).unwrap_or(AssetUuid::NIL), 0)
            };
            if *file == next_file && *slot == next_slot {
                return false;
            }
            *file = next_file;
            *slot = next_slot;
            true
        }
        (FieldType::GameAsset(_), DataValue::GameAsset(cur), PropValue::Str(s)) => {
            let next = s.trim();
            if *cur == next {
                return false;
            }
            *cur = next.to_string();
            true
        }
        (ty, target_val, PropValue::Float(v)) => apply_set_component(ty, target_val, 0, *v, settings),
        _ => false,
    }
}

/// Applies a component float edit (`TabAction::SetComponent`) to `target_val` for `ty`. Returns `true` when `target_val` changed.
pub fn apply_set_component(
    ty: impl Borrow<FieldType>,
    target_val: &mut DataValue,
    comp: usize,
    value: f32,
    settings: &FieldSettings,
) -> bool {
    let ty = ty.borrow();
    if !ty.matches(target_val) {
        *target_val = ty.default_value();
    }
    let clamped = clamp_f64(value as f64, settings);
    match (ty, target_val) {
        (FieldType::I8, DataValue::I8(cur)) if comp == 0 => {
            let next = clamped.round().clamp(i8::MIN as f64, i8::MAX as f64) as i8;
            if *cur == next {
                return false;
            }
            *cur = next;
            true
        }
        (FieldType::I16, DataValue::I16(cur)) if comp == 0 => {
            let next = clamped.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16;
            if *cur == next {
                return false;
            }
            *cur = next;
            true
        }
        (FieldType::I32, DataValue::I32(cur)) if comp == 0 => {
            let next = clamped.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32;
            if *cur == next {
                return false;
            }
            *cur = next;
            true
        }
        (FieldType::I64, DataValue::I64(cur)) if comp == 0 => {
            let next = clamped.round().clamp(i64::MIN as f64, i64::MAX as f64) as i64;
            if *cur == next {
                return false;
            }
            *cur = next;
            true
        }
        (FieldType::U8, DataValue::U8(cur)) if comp == 0 => {
            let next = clamped.round().clamp(0.0, u8::MAX as f64) as u8;
            if *cur == next {
                return false;
            }
            *cur = next;
            true
        }
        (FieldType::U16, DataValue::U16(cur)) if comp == 0 => {
            let next = clamped.round().clamp(0.0, u16::MAX as f64) as u16;
            if *cur == next {
                return false;
            }
            *cur = next;
            true
        }
        (FieldType::U32, DataValue::U32(cur)) if comp == 0 => {
            let next = clamped.round().clamp(0.0, u32::MAX as f64) as u32;
            if *cur == next {
                return false;
            }
            *cur = next;
            true
        }
        (FieldType::U64, DataValue::U64(cur)) if comp == 0 => {
            let next = clamped.round().max(0.0) as u64;
            if *cur == next {
                return false;
            }
            *cur = next;
            true
        }
        (FieldType::F32, DataValue::F32(cur)) if comp == 0 => {
            let next = clamped as f32;
            if (*cur - next).abs() < 1e-7 {
                return false;
            }
            *cur = next;
            true
        }
        (FieldType::F64, DataValue::F64(cur)) if comp == 0 => {
            if (*cur - clamped).abs() < 1e-9 {
                return false;
            }
            *cur = clamped;
            true
        }
        (FieldType::Vec2, DataValue::Vec2(v)) if comp < 2 => {
            let next = clamped as f32;
            if (v[comp] - next).abs() < 1e-7 {
                return false;
            }
            v[comp] = next;
            true
        }
        (FieldType::Vec3, DataValue::Vec3(v)) if comp < 3 => {
            let next = clamped as f32;
            if (v[comp] - next).abs() < 1e-7 {
                return false;
            }
            v[comp] = next;
            true
        }
        (FieldType::Vec4, DataValue::Vec4(v)) if comp < 4 => {
            let next = clamped as f32;
            if (v[comp] - next).abs() < 1e-7 {
                return false;
            }
            v[comp] = next;
            true
        }
        (FieldType::IVec2, DataValue::IVec2(v)) if comp < 2 => {
            let next = clamped.round() as i32;
            if v[comp] == next {
                return false;
            }
            v[comp] = next;
            true
        }
        (FieldType::IVec3, DataValue::IVec3(v)) if comp < 3 => {
            let next = clamped.round() as i32;
            if v[comp] == next {
                return false;
            }
            v[comp] = next;
            true
        }
        (FieldType::UVec2, DataValue::UVec2(v)) if comp < 2 => {
            let next = clamped.round().max(0.0) as u32;
            if v[comp] == next {
                return false;
            }
            v[comp] = next;
            true
        }
        (FieldType::UVec3, DataValue::UVec3(v)) if comp < 3 => {
            let next = clamped.round().max(0.0) as u32;
            if v[comp] == next {
                return false;
            }
            v[comp] = next;
            true
        }
        (FieldType::Quat, DataValue::Quat(q)) if comp < 3 => {
            let mut deg = quat_to_euler_deg(*q);
            if (deg[comp] - value).abs() < 1e-5 {
                return false;
            }
            deg[comp] = value;
            *q = euler_deg_to_quat(deg);
            true
        }
        (FieldType::Color, DataValue::Color(rgba)) if comp < 4 => {
            let next = if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 };
            if (rgba[comp] - next).abs() < 1e-6 {
                return false;
            }
            rgba[comp] = next;
            true
        }
        (FieldType::Duration, DataValue::Duration(secs)) if comp == 0 => {
            let next = clamped.max(0.0) as f32;
            if (*secs - next).abs() < 1e-6 {
                return false;
            }
            *secs = next;
            true
        }
        (FieldType::RangeF32, DataValue::RangeF32(r)) if comp < 2 => {
            let next = clamped as f32;
            if (r[comp] - next).abs() < 1e-7 {
                return false;
            }
            r[comp] = next;
            true
        }
        (FieldType::RangeI32, DataValue::RangeI32(r)) if comp < 2 => {
            let next = clamped.round() as i32;
            if r[comp] == next {
                return false;
            }
            r[comp] = next;
            true
        }
        _ => false,
    }
}

pub type NumericLimits = (Option<(f32, f32)>, Option<(f32, f32)>);

pub fn numeric_limits(ty: impl Borrow<FieldType>, settings: &FieldSettings) -> NumericLimits {
    let ty = ty.borrow();
    let (type_min, type_max) = match ty {
        FieldType::I8 => (Some(i8::MIN as f32), Some(i8::MAX as f32)),
        FieldType::I16 => (Some(i16::MIN as f32), Some(i16::MAX as f32)),
        FieldType::U8 => (Some(0.0), Some(u8::MAX as f32)),
        FieldType::U16 => (Some(0.0), Some(u16::MAX as f32)),
        FieldType::U32 | FieldType::U64 | FieldType::UVec2 | FieldType::UVec3 | FieldType::Duration => {
            (Some(0.0), None)
        }
        _ => (None, None),
    };
    let min = match (type_min, settings.min.map(|v| v as f32)) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
    let max = match (type_max, settings.max.map(|v| v as f32)) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
    let hard =
        if min.is_some() || max.is_some() { Some((min.unwrap_or(f32::MIN), max.unwrap_or(f32::MAX))) } else { None };
    let soft = if settings.slider
        && let (Some(lo), Some(hi)) = (min, max)
        && lo < hi
    {
        Some((lo, hi))
    } else {
        None
    };
    (soft, hard)
}

fn labeled_components(
    target: &PropKey,
    labels: &[&'static str],
    vals: &[f32],
    soft: Option<(f32, f32)>,
    hard: Option<(f32, f32)>,
    precision: i32,
    suffix: Option<&'static str>,
) -> Box<dyn Scene> {
    let mut items: Vec<Box<dyn Scene>> = Vec::with_capacity(labels.len() * 2 + 1);
    for (comp, (&lbl, &v)) in labels.iter().zip(vals.iter()).enumerate() {
        items.push(Box::new(text(lbl, TEXT_DIM, 11.0)));
        let input = number_input(target.clone(), comp, v, soft, hard, precision);
        items.push(Box::new(bsn! {
            Node { flex_grow: 1.0, flex_basis: px(0), min_width: px(42) }
            Children [ @{input} ]
        }));
    }
    if let Some(sfx) = suffix {
        items.push(Box::new(text(sfx, TEXT_DIM, 11.0)));
    }
    Box::new(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(4),
            flex_grow: 1.0,
            min_width: px(0),
        }
        Children [ {items} ]
    })
}

/// Builds a $64 \times 20\text{ px}$ inline curve mini-preview (`DataCurveMiniPreview`) sampling 16 vertical bars across $x \in [0, 1]$.
pub fn curve_mini_preview(curve: Option<&CurveData>) -> Box<dyn Scene> {
    if let Some(c) = curve {
        let bar_color = ACCENT.with_alpha(0.85);
        let mut bars: Vec<Box<dyn Scene>> = Vec::with_capacity(16);
        for i in 0..16 {
            let t = (i as f32 + 0.5) / 16.0;
            let y = c.sample(t).clamp(0.0, 1.0);
            let h = (y * 16.0).clamp(2.0, 16.0);
            bars.push(Box::new(bsn! {
                Node {
                    flex_grow: 1.0,
                    height: px(h),
                }
                BackgroundColor(bar_color)
            }));
        }
        Box::new(bsn! {
            Node {
                width: px(64),
                height: px(20),
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::FlexEnd,
                column_gap: px(1),
                padding: UiRect::axes(px(2), px(2)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(3)),
                overflow: Overflow::clip(),
            }
            BackgroundColor(PANEL_BG)
            BorderColor::all(BORDER)
            DataCurveMiniPreview
            Children [ {bars} ]
        })
    } else {
        let dash_color = TEXT_DIM.with_alpha(0.5);
        Box::new(bsn! {
            Node {
                width: px(64),
                height: px(20),
                flex_shrink: 0.0,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(3)),
                overflow: Overflow::clip(),
            }
            BackgroundColor(PANEL_BG)
            BorderColor::all(BORDER)
            DataCurveMiniPreview
            Children [ @text("-", dash_color, 10.0) ]
        })
    }
}

/// Builds a $64 \times 20\text{ px}$ inline OKLab gradient color bar (`DataGradientMiniPreview`) sampling 16 strips across $t \in [0, 1]$.
pub fn gradient_mini_preview(gradient: Option<&RawGradient>) -> Box<dyn Scene> {
    if let Some(g) = gradient {
        let mut strips: Vec<Box<dyn Scene>> = Vec::with_capacity(16);
        for i in 0..16 {
            let t = (i as f32 + 0.5) / 16.0;
            let [r, gr, b, a] = g.sample(t).unwrap_or([0.0, 0.0, 0.0, 1.0]);
            let strip_color = Color::srgba(r, gr, b, a);
            strips.push(Box::new(bsn! {
                Node {
                    flex_grow: 1.0,
                    height: Val::Percent(100.0),
                }
                BackgroundColor(strip_color)
            }));
        }
        Box::new(bsn! {
            Node {
                width: px(64),
                height: px(20),
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Row,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(3)),
                overflow: Overflow::clip(),
            }
            BackgroundColor(PANEL_BG)
            BorderColor::all(BORDER)
            DataGradientMiniPreview
            Children [ {strips} ]
        })
    } else {
        let dash_color = TEXT_DIM.with_alpha(0.5);
        Box::new(bsn! {
            Node {
                width: px(64),
                height: px(20),
                flex_shrink: 0.0,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(3)),
                overflow: Overflow::clip(),
            }
            BackgroundColor(PANEL_BG)
            BorderColor::all(BORDER)
            DataGradientMiniPreview
            Children [ @text("-", dash_color, 10.0) ]
        })
    }
}

/// Builds a Jump icon button (`IconId::OpenExternal`) that dispatches `Action::OpenAsset(AssetRef)` when enabled.
pub fn jump_asset_button(asset_ref: Option<AssetRef>) -> Box<dyn Scene> {
    if let Some(r) = asset_ref {
        let tip = Tooltip(format!("Open {}", r.source_rel()));
        let act = OnActivate(Action::OpenAsset(r.clone()));
        let marker = DataJumpAssetButton(r);
        Box::new(bsn! {
            @FeathersToolButton {
                @variant: ButtonVariant::Plain,
                @caption: bsn! { @icon(IconId::OpenExternal, 13.0, ACCENT) },
            }
            Node { flex_shrink: 0.0 }
            act
            tip
            marker
        })
    } else {
        let dim_icon = TEXT_DIM.with_alpha(0.35);
        let tip = Tooltip("No target asset selected".to_string());
        Box::new(bsn! {
            @FeathersToolButton {
                @variant: ButtonVariant::Plain,
                @caption: bsn! { @icon(IconId::OpenExternal, 13.0, dim_icon) },
            }
            Node { flex_shrink: 0.0 }
            bevy::ui::InteractionDisabled
            tip
        })
    }
}

/// Builds a Jump icon button (`IconId::OpenExternal`) that dispatches `DataStep::OpenCollectionRow` when enabled.
pub fn jump_row_button(target: Option<(AssetRef, AssetUuid)>) -> Box<dyn Scene> {
    if let Some((col_ref, row_id)) = target {
        let tip = Tooltip(format!("Open {} and select row {row_id}", col_ref.source_rel()));
        let act = OnActivate(Action::from(DataStep::OpenCollectionRow { collection: col_ref.clone(), row: row_id }));
        let marker = DataJumpAssetButton(col_ref);
        Box::new(bsn! {
            @FeathersToolButton {
                @variant: ButtonVariant::Plain,
                @caption: bsn! { @icon(IconId::OpenExternal, 13.0, ACCENT) },
            }
            Node { flex_shrink: 0.0 }
            act
            tip
            marker
        })
    } else {
        let dim_icon = TEXT_DIM.with_alpha(0.35);
        let tip = Tooltip("No target row selected".to_string());
        Box::new(bsn! {
            @FeathersToolButton {
                @variant: ButtonVariant::Plain,
                @caption: bsn! { @icon(IconId::OpenExternal, 13.0, dim_icon) },
            }
            Node { flex_shrink: 0.0 }
            bevy::ui::InteractionDisabled
            tip
        })
    }
}

fn uuid_asset_picker_row(
    target: PropKey,
    u: AssetUuid,
    entries: &[(AssetUuid, String)],
    refs_by_id: &BTreeMap<AssetUuid, AssetRef>,
    preview: Option<Box<dyn Scene>>,
) -> Box<dyn Scene> {
    let mut labels: Vec<String> = Vec::with_capacity(entries.len() + 2);
    let mut values: Vec<String> = Vec::with_capacity(entries.len() + 2);
    labels.push("(None)".to_string());
    values.push(String::new());
    let mut cur_idx = 0usize;
    for (i, (id, lbl)) in entries.iter().enumerate() {
        labels.push(lbl.clone());
        values.push(id.to_string());
        if *id == u {
            cur_idx = i + 1;
        }
    }
    if !u.is_nil() && cur_idx == 0 {
        labels.push(format!("{u} (missing)"));
        values.push(u.to_string());
        cur_idx = labels.len() - 1;
    }
    let select = select_values(target, labels, values, cur_idx);
    let jump_ref = (!u.is_nil()).then(|| refs_by_id.get(&u).cloned()).flatten();
    let jump_btn = jump_asset_button(jump_ref);
    let mut children: Vec<Box<dyn Scene>> = vec![Box::new(bsn! {
        Node { flex_grow: 1.0, min_width: px(90) }
        Children [ @{select} ]
    })];
    if let Some(prev) = preview {
        children.push(prev);
    }
    children.push(jump_btn);
    Box::new(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(4),
            flex_grow: 1.0,
            min_width: px(0),
        }
        Children [ {children} ]
    })
}

/// Builds a variant selector dropdown for a `FieldType::Enum(target_id)` value.
pub fn enum_variant_select(
    target: PropKey,
    target_enum: Option<&EnumAsset>,
    current_variant: AssetUuid,
) -> Box<dyn Scene> {
    let mut labels = Vec::new();
    let mut values = Vec::new();
    let mut cur_idx = 0usize;
    if let Some(en) = target_enum {
        if en.variants.is_empty() {
            labels.push("(No Variants)".to_string());
            values.push(String::new());
        } else {
            for (i, v) in en.variants.iter().enumerate() {
                if v.id.is_nil() {
                    continue;
                }
                labels.push(to_pascal_case(&v.name));
                values.push(v.id.to_string());
                if v.id == current_variant {
                    cur_idx = i;
                }
            }
        }
    } else {
        labels.push("(Select Enum)".to_string());
        values.push(String::new());
    }
    if !current_variant.is_nil() && !values.iter().any(|v| v == &current_variant.to_string()) {
        labels.push(format!("{current_variant} (missing)"));
        values.push(current_variant.to_string());
        cur_idx = labels.len() - 1;
    }
    if labels.is_empty() {
        labels.push("(No Variants)".to_string());
        values.push(String::new());
    }
    select_values(target, labels, values, cur_idx)
}

/// Builds the editor widget scene for a [`DataValue`] of the given [`FieldType`] and [`FieldSettings`].
pub fn data_value_editor(
    target: PropKey,
    ty: impl Borrow<FieldType>,
    val: &DataValue,
    settings: &FieldSettings,
) -> Box<dyn Scene> {
    data_value_editor_with_catalog(target, ty, val, settings, &AssetPickerCatalog::default())
}

/// Builds the editor widget scene for a [`DataValue`] of the given [`FieldType`], [`FieldSettings`],
/// and [`AssetPickerCatalog`] (providing `AssetIndex`-backed options and inline curve/gradient previews).
pub fn data_value_editor_with_catalog(
    target: PropKey,
    ty: impl Borrow<FieldType>,
    val: &DataValue,
    settings: &FieldSettings,
    catalog: &AssetPickerCatalog,
) -> Box<dyn Scene> {
    let ty = ty.borrow();
    let fallback;
    let val = if ty.matches(val) {
        val
    } else {
        fallback = ty.default_value();
        &fallback
    };
    let (soft, hard) = numeric_limits(ty, settings);
    match (ty, val) {
        (FieldType::Bool, DataValue::Bool(b)) => checkbox_w(target, *b),
        (FieldType::I8, DataValue::I8(v)) => number_input(target, 0, *v as f32, soft, hard, 0),
        (FieldType::I16, DataValue::I16(v)) => number_input(target, 0, *v as f32, soft, hard, 0),
        (FieldType::I32, DataValue::I32(v)) => number_input(target, 0, *v as f32, soft, hard, 0),
        (FieldType::I64, DataValue::I64(v)) => number_input(target, 0, *v as f32, soft, hard, 0),
        (FieldType::U8, DataValue::U8(v)) => number_input(target, 0, *v as f32, soft, hard, 0),
        (FieldType::U16, DataValue::U16(v)) => number_input(target, 0, *v as f32, soft, hard, 0),
        (FieldType::U32, DataValue::U32(v)) => number_input(target, 0, *v as f32, soft, hard, 0),
        (FieldType::U64, DataValue::U64(v)) => number_input(target, 0, *v as f32, soft, hard, 0),
        (FieldType::F32, DataValue::F32(v)) => number_input(target, 0, *v, soft, hard, 2),
        (FieldType::F64, DataValue::F64(v)) => number_input(target, 0, *v as f32, soft, hard, 2),
        (FieldType::String, DataValue::String(s)) | (FieldType::LocalizedString, DataValue::LocalizedString(s)) => {
            if settings.multiline
                && let Some(max_len) = settings.max_len
            {
                text_area(target, s.clone(), max_len)
            } else {
                text_w(target, s.clone(), settings.multiline)
            }
        }
        (FieldType::Vec2, DataValue::Vec2(v)) => labeled_components(&target, &["X", "Y"], v, soft, hard, 2, None),
        (FieldType::Vec3, DataValue::Vec3(v)) => labeled_components(&target, &["X", "Y", "Z"], v, soft, hard, 2, None),
        (FieldType::Vec4, DataValue::Vec4(v)) => {
            labeled_components(&target, &["X", "Y", "Z", "W"], v, soft, hard, 2, None)
        }
        (FieldType::IVec2, DataValue::IVec2(v)) => {
            labeled_components(&target, &["X", "Y"], &[v[0] as f32, v[1] as f32], soft, hard, 0, None)
        }
        (FieldType::IVec3, DataValue::IVec3(v)) => {
            labeled_components(&target, &["X", "Y", "Z"], &[v[0] as f32, v[1] as f32, v[2] as f32], soft, hard, 0, None)
        }
        (FieldType::UVec2, DataValue::UVec2(v)) => {
            labeled_components(&target, &["X", "Y"], &[v[0] as f32, v[1] as f32], soft, hard, 0, None)
        }
        (FieldType::UVec3, DataValue::UVec3(v)) => {
            labeled_components(&target, &["X", "Y", "Z"], &[v[0] as f32, v[1] as f32, v[2] as f32], soft, hard, 0, None)
        }
        (FieldType::Quat, DataValue::Quat(q)) => {
            let deg = quat_to_euler_deg(*q);
            labeled_components(&target, &["X", "Y", "Z"], &deg, None, None, 1, Some("deg"))
        }
        (FieldType::Color, DataValue::Color(rgba)) => {
            let color = linear_color(rgba);
            let field = PropField { target, ..default() };
            Box::new(bsn! {
                @FeathersColorInput
                ColorInputValue(color)
                on(color_input_self_update)
                field
                DataColorSwatch
            })
        }
        (FieldType::Duration, DataValue::Duration(secs)) => {
            let input = number_input(target, 0, *secs, soft, hard, 2);
            Box::new(bsn! {
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(4),
                    flex_grow: 1.0,
                    min_width: px(0),
                }
                Children [
                    Node { flex_grow: 1.0, min_width: px(60) }
                    Children [ @{input} ]
                    --
                    @text("s", TEXT_DIM, 11.0)
                ]
            })
        }
        (FieldType::RangeF32, DataValue::RangeF32(r)) => {
            labeled_components(&target, &["min", "max"], r, soft, hard, 2, None)
        }
        (FieldType::RangeI32, DataValue::RangeI32(r)) => {
            labeled_components(&target, &["min", "max"], &[r[0] as f32, r[1] as f32], soft, hard, 0, None)
        }
        (FieldType::Curve, DataValue::Curve(u)) => {
            let curve_data = catalog.curves.iter().find(|(id, _, _)| id == u).map(|(_, _, c)| c);
            let prev = curve_mini_preview(curve_data);
            let entries: Vec<(AssetUuid, String)> =
                catalog.curves.iter().map(|(id, lbl, _)| (*id, lbl.clone())).collect();
            uuid_asset_picker_row(target, *u, &entries, &catalog.refs_by_id, Some(prev))
        }
        (FieldType::Gradient, DataValue::Gradient(file, slot)) => {
            let mut labels: Vec<String> = vec!["(None)".to_string()];
            let mut values: Vec<String> = vec![String::new()];
            let mut cur_idx = 0usize;
            let mut selected_grad: Option<&RawGradient> = None;
            for (gid, gpath, slots) in &catalog.gradients {
                for (slot_id, slot_label, grad) in slots {
                    let lbl = if slot_label.is_empty() {
                        format!("{gpath} #{slot_id}")
                    } else {
                        format!("{gpath} #{slot_id} ({slot_label})")
                    };
                    let val_str = format!("{gid}:{slot_id}");
                    labels.push(lbl);
                    values.push(val_str);
                    if gid == file && slot_id == slot {
                        cur_idx = labels.len() - 1;
                        selected_grad = Some(grad);
                    }
                }
            }
            if !file.is_nil() && cur_idx == 0 {
                labels.push(format!("{file}:{slot} (missing)"));
                values.push(format!("{file}:{slot}"));
                cur_idx = labels.len() - 1;
            }
            let select = select_values(target, labels, values, cur_idx);
            let prev = gradient_mini_preview(selected_grad);
            let jump_ref = (!file.is_nil()).then(|| catalog.refs_by_id.get(file).cloned()).flatten();
            let jump_btn = jump_asset_button(jump_ref);
            Box::new(bsn! {
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(4),
                    flex_grow: 1.0,
                    min_width: px(0),
                }
                Children [
                    Node { flex_grow: 1.0, min_width: px(90) }
                    Children [ @{select} ]
                    --
                    @{prev}
                    --
                    @{jump_btn}
                ]
            })
        }
        (FieldType::Material, DataValue::Material(u)) => {
            uuid_asset_picker_row(target, *u, &catalog.materials, &catalog.refs_by_id, None)
        }
        (FieldType::Texture, DataValue::Texture(u)) => {
            uuid_asset_picker_row(target, *u, &catalog.textures, &catalog.refs_by_id, None)
        }
        (FieldType::LightEnvironment, DataValue::LightEnvironment(u)) => {
            uuid_asset_picker_row(target, *u, &catalog.environments, &catalog.refs_by_id, None)
        }
        (FieldType::GameAsset(kind), DataValue::GameAsset(p)) => {
            let matching: Vec<&str> =
                catalog.game_files.iter().map(String::as_str).filter(|rel| kind.matches_path(rel)).collect();
            let mut labels: Vec<String> = Vec::with_capacity(matching.len() + 2);
            let mut values: Vec<String> = Vec::with_capacity(matching.len() + 2);
            labels.push("(None)".to_string());
            values.push(String::new());
            let mut cur_idx = 0usize;
            for (i, item) in matching.iter().enumerate() {
                labels.push((*item).to_string());
                values.push((*item).to_string());
                if *item == p.as_str() {
                    cur_idx = i + 1;
                }
            }
            if !p.is_empty() && cur_idx == 0 {
                labels.push(format!("{p} (missing)"));
                values.push(p.clone());
                cur_idx = labels.len() - 1;
            }
            let select = select_values(target, labels, values, cur_idx);
            Box::new(bsn! {
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(4),
                    flex_grow: 1.0,
                    min_width: px(0),
                }
                Children [
                    Node { flex_grow: 1.0, min_width: px(90) }
                    Children [ @{select} ]
                ]
            })
        }
        (FieldType::SoftRef(target_dto_uuid), DataValue::SoftRef(current_row_uuid)) => {
            let rows = catalog.soft_ref_rows_for(*target_dto_uuid);
            let mut labels: Vec<String> = Vec::with_capacity(rows.len() + 2);
            let mut values: Vec<String> = Vec::with_capacity(rows.len() + 2);
            labels.push("(None)".to_string());
            values.push(String::new());
            let mut cur_idx = 0usize;
            for (i, (row_id, lbl, _)) in rows.iter().enumerate() {
                labels.push(lbl.clone());
                values.push(row_id.to_string());
                if *row_id == *current_row_uuid {
                    cur_idx = i + 1;
                }
            }
            if !current_row_uuid.is_nil() && cur_idx == 0 {
                labels.push(format!("(missing: {current_row_uuid})"));
                values.push(current_row_uuid.to_string());
                cur_idx = labels.len() - 1;
            }
            let select = select_values(target, labels, values, cur_idx);
            let jump_target = (!current_row_uuid.is_nil())
                .then(|| catalog.find_soft_ref_row(*target_dto_uuid, *current_row_uuid))
                .flatten()
                .map(|(row_id, _, col_ref)| (col_ref.clone(), *row_id));
            let jump_btn = jump_row_button(jump_target);
            Box::new(bsn! {
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(4),
                    flex_grow: 1.0,
                    min_width: px(0),
                }
                Children [
                    Node { flex_grow: 1.0, min_width: px(90) }
                    Children [ @{select} ]
                    --
                    @{jump_btn}
                ]
            })
        }
        (FieldType::Enum(target_enum_uuid), DataValue::Enum { variant, .. }) => {
            enum_variant_select(target, catalog.enums_by_id.get(target_enum_uuid), *variant)
        }
        _ => Box::new(text(val.to_short_view_string(), TEXT_DIM, 12.0)),
    }
}

const FORM_ROW_LINE: Color = Color::srgb_u8(40, 42, 48);
const REQUIRED_STAR_COLOR: Color = Color::srgb(0.95, 0.42, 0.42);

pub fn reset_icon_color(is_default: bool) -> Color {
    if is_default { TEXT_DIM.with_alpha(0.35) } else { ACCENT }
}

/// Builds a reset-to-default icon button (`IconId::Undo`) tinted [`ACCENT`] when modified.
pub fn dto_reset_button(action: Action, is_default: bool, default_str: &str) -> Box<dyn Scene> {
    let color = reset_icon_color(is_default);
    let act = OnActivate(action);
    let tip = if is_default {
        Tooltip(format!("At default ({default_str})"))
    } else {
        Tooltip(format!("Reset to default ({default_str})"))
    };
    Box::new(bsn! {
        @FeathersToolButton {
            @variant: ButtonVariant::Plain,
            @caption: bsn! { @icon(IconId::Undo, 14.0, color) },
        }
        Node { flex_shrink: 0.0 }
        act
        tip
    })
}

/// Builds a standard vertical DTO form row (`{label} | {value} | {reset}`) shared by Collection Form View
/// and the internal Constraints DTO form in the DTO Editor.
pub fn dto_form_row(
    label: impl Into<String>,
    label_color: Color,
    required: bool,
    tip: Tooltip,
    editor_widget: Box<dyn Scene>,
    reset_button: Box<dyn Scene>,
) -> Box<dyn Scene> {
    let mut label_children: Vec<Box<dyn Scene>> = vec![Box::new(text(label.into(), label_color, 12.0))];
    if required {
        label_children.push(Box::new(text("*", REQUIRED_STAR_COLOR, 12.0)));
    }

    Box::new(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(8),
            min_height: px(30),
            padding: UiRect::axes(px(8), px(2)),
            border: UiRect::bottom(px(1)),
        }
        BorderColor::all(FORM_ROW_LINE)
        Children [
            Node {
                width: px(148),
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(3),
            }
            tip
            bevy::picking::cursor::EntityCursor::System(bevy::window::SystemCursorIcon::Help)
            Children [ {label_children} ]
            --
            Node { flex_grow: 1.0, min_width: px(120) }
            Children [ @{editor_widget} ]
            --
            @{reset_button}
        ]
    })
}

/// Builds a vertical DTO form row for an `Option<T>` field (`None | Some` toggle + indented `value` sub-row when `Some`).
#[allow(clippy::too_many_arguments)]
pub fn dto_option_row(
    label: impl Into<String>,
    tip: Tooltip,
    is_some: bool,
    none_action: Action,
    some_action: Action,
    inner_widget: Option<Box<dyn Scene>>,
    reset_action: Action,
) -> Box<dyn Scene> {
    let toggle = button_group(vec![
        GroupButton {
            caption: Caption::Text("None".to_string()),
            action: none_action,
            selected: !is_some,
            disabled: false,
            tip: "Unset (None)".to_string(),
        },
        GroupButton {
            caption: Caption::Text("Some".to_string()),
            action: some_action,
            selected: is_some,
            disabled: false,
            tip: "Set value (Some)".to_string(),
        },
    ]);
    let label_color = if is_some { ACCENT } else { TEXT };
    let reset_btn = dto_reset_button(reset_action, !is_some, "None");
    let top_row = dto_form_row(label, label_color, false, tip, Box::new(toggle), reset_btn);
    if is_some && let Some(inner) = inner_widget {
        Box::new(bsn! {
            Node { flex_direction: FlexDirection::Column }
            Children [
                @{top_row}
                --
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(8),
                    min_height: px(30),
                    padding: UiRect { left: px(24), right: px(30), top: px(2), bottom: px(4) },
                    border: UiRect::bottom(px(1)),
                }
                BorderColor::all(FORM_ROW_LINE)
                Children [
                    Node {
                        width: px(132),
                        flex_shrink: 0.0,
                        padding: UiRect::left(px(8)),
                        border: UiRect::left(px(2)),
                    }
                    BorderColor::all(ACCENT)
                    Children [ @text("value", TEXT_DIM, 11.0) ]
                    --
                    Node { flex_grow: 1.0, min_width: px(120) }
                    Children [ @{inner} ]
                ]
            ]
        })
    } else {
        top_row
    }
}

/// Synchronizes `NumberInputValue`, `ColorInputValue`, and `FeathersCheckbox` widgets matching `matches_target`
/// with the active tab's `current_value`, skipping the widget currently focused for keyboard typing.
#[allow(clippy::too_many_arguments)]
pub fn sync_data_value_widgets(
    editor: &editor_core::state::editor::Editor,
    focus: Option<&InputFocus>,
    numbers: &Query<(Entity, &PropField, &NumberInputValue)>,
    pickers: &Query<(Entity, &PropField, &ColorInputValue)>,
    checkboxes: &Query<(Entity, &PropField, Has<Checked>), With<FeathersCheckbox>>,
    parents: &Query<&ChildOf>,
    commands: &mut Commands,
    matches_target: impl Fn(&PropKey) -> bool,
) {
    let typing = focus.and_then(|f| f.get()).and_then(|f| parents.get(f).ok()).map(ChildOf::parent);
    for (e, field, shown) in numbers {
        if !matches_target(&field.target) {
            continue;
        }
        let Some(v) = editor.current_value(&field.target) else { continue };
        let Some(&want_f32) = v.floats().get(field.comp) else { continue };
        let want = NumberInputValue::F32(if want_f32.abs() < 1e-6 { 0.0 } else { want_f32 });
        if Some(e) != typing && *shown != want {
            commands.entity(e).try_insert(want);
        }
    }
    for (e, field, shown) in pickers {
        if !matches_target(&field.target) {
            continue;
        }
        let Some(v) = editor.current_value(&field.target) else { continue };
        let mut rgba = v.floats();
        if rgba.len() < 4 {
            continue;
        }
        rgba.truncate(4);
        let want = linear_color(&rgba);
        let (a, b) = (shown.0.to_linear().to_f32_array(), want.to_linear().to_f32_array());
        if a.iter().zip(b).any(|(x, y)| (x - y).abs() > 1e-5) {
            commands.entity(e).try_insert(ColorInputValue(want));
        }
    }
    for (e, field, is_checked) in checkboxes {
        if !matches_target(&field.target) {
            continue;
        }
        let Some(PropValue::Bool(want)) = editor.current_value(&field.target) else { continue };
        if is_checked != want {
            if want {
                commands.entity(e).try_insert(Checked);
            } else {
                commands.entity(e).remove::<Checked>();
            }
        }
    }
}

/// Parses a [`FieldType`] tag from a [`PropValue`] (supporting `Int(tag)`, `Str("<tag>")`, or `Str("<label>")`).
pub fn parse_type_tag_prop(prop: &PropValue, allowed: &[FieldType]) -> Option<usize> {
    match prop {
        PropValue::Int(i) => {
            let idx = usize::try_from(*i).ok()?;
            if allowed.iter().any(|t| t.tag() == idx) { Some(idx) } else { allowed.get(idx).map(FieldType::tag) }
        }
        PropValue::Str(s) => {
            let trimmed = s.trim();
            if let Ok(tag) = trimmed.parse::<usize>()
                && allowed.iter().any(|t| t.tag() == tag)
            {
                return Some(tag);
            }
            allowed.iter().find(|t| t.label().eq_ignore_ascii_case(trimmed)).map(FieldType::tag)
        }
        _ => None,
    }
}

/// Resolves a leaf [`FieldType`] (tag `< 34`) with default target UUIDs when switching types.
pub fn resolve_leaf_field_type(
    tag: usize,
    prev: Option<&FieldType>,
    first_dto_id: Option<AssetUuid>,
    first_enum_id: Option<AssetUuid>,
) -> Option<FieldType> {
    if tag >= 34 {
        return None;
    }
    let base = FieldType::from_tag(tag)?;
    Some(match base {
        FieldType::Dto(_) => {
            let existing = prev.and_then(|t| t.dto_target().or_else(|| t.soft_ref_target()));
            FieldType::Dto(existing.filter(|u| !u.is_nil()).or(first_dto_id).unwrap_or(AssetUuid::NIL))
        }
        FieldType::SoftRef(_) => {
            let existing = prev.and_then(|t| t.soft_ref_target().or_else(|| t.dto_target()));
            FieldType::SoftRef(existing.filter(|u| !u.is_nil()).or(first_dto_id).unwrap_or(AssetUuid::NIL))
        }
        FieldType::Enum(_) => {
            let existing = prev.and_then(FieldType::enum_target);
            FieldType::Enum(existing.filter(|u| !u.is_nil()).or(first_enum_id).unwrap_or(AssetUuid::NIL))
        }
        FieldType::GameAsset(_) => {
            if let Some(FieldType::GameAsset(k)) = prev {
                FieldType::GameAsset(*k)
            } else {
                FieldType::GameAsset(GameAssetKind::Any)
            }
        }
        other => other,
    })
}

/// Resolves a primary [`FieldType`] change (`tag` in `0..38`), preserving compatible inner/key types across transitions.
pub fn resolve_primary_field_type(
    current: &FieldType,
    new_tag: usize,
    first_dto_id: Option<AssetUuid>,
    first_enum_id: Option<AssetUuid>,
) -> Option<FieldType> {
    if new_tag < 34 {
        let prev = current.inner_type().or(Some(current));
        return resolve_leaf_field_type(new_tag, prev, first_dto_id, first_enum_id);
    }
    let preserved_inner = current
        .inner_type()
        .cloned()
        .or_else(|| (current.tag() < 34).then(|| current.clone()))
        .unwrap_or(FieldType::I32);
    let is_key_candidate = |t: &FieldType| FieldType::ALL_KEY_TYPES.iter().any(|k| k.tag() == t.tag());
    match new_tag {
        34 => Some(FieldType::Option(Box::new(preserved_inner))),
        35 => Some(FieldType::Array(Box::new(preserved_inner))),
        36 => {
            let k = if is_key_candidate(&preserved_inner) { preserved_inner } else { FieldType::I32 };
            Some(FieldType::Set(Box::new(k)))
        }
        37 => {
            let k = current.key_type().cloned().filter(is_key_candidate).unwrap_or(FieldType::String);
            let v = if matches!(current, FieldType::Set(_)) { FieldType::I32 } else { preserved_inner };
            Some(FieldType::Map(Box::new(k), Box::new(v)))
        }
        _ => None,
    }
}

/// Applies an inner-type (`T` for `Option`/`Array`, `K` for `Set`, or `V` for `Map`) change to `ty`.
pub fn apply_inner_type_change(
    ty: &mut FieldType,
    prop: &PropValue,
    first_dto_id: Option<AssetUuid>,
    first_enum_id: Option<AssetUuid>,
) -> bool {
    let is_set = matches!(ty, FieldType::Set(_));
    let allowed: &[FieldType] = if is_set { FieldType::ALL_KEY_TYPES } else { &FieldType::ALL_INNER_TYPES };
    let Some(tag) = parse_type_tag_prop(prop, allowed) else {
        return false;
    };
    let prev = ty.inner_type().cloned();
    let Some(next_inner) = resolve_leaf_field_type(tag, prev.as_ref(), first_dto_id, first_enum_id) else {
        return false;
    };
    match ty {
        FieldType::Option(inner) | FieldType::Array(inner) | FieldType::Set(inner) | FieldType::Map(_, inner) => {
            if **inner == next_inner {
                return false;
            }
            **inner = next_inner;
            true
        }
        _ => false,
    }
}

/// Applies a target change (`Dto` / `SoftRef` / `Enum` UUID or `GameAssetKind` tag) to the inner type of `ty`.
pub fn apply_inner_target_change(ty: &mut FieldType, prop: &PropValue) -> bool {
    let Some(inner) = (match ty {
        FieldType::Option(i) | FieldType::Array(i) | FieldType::Set(i) | FieldType::Map(_, i) => Some(i.as_mut()),
        _ => None,
    }) else {
        return false;
    };
    apply_leaf_target_change(inner, prop)
}

/// Applies a key-type (`K` for `Map<K, V>`) change to `ty`.
pub fn apply_key_type_change(ty: &mut FieldType, prop: &PropValue, first_enum_id: Option<AssetUuid>) -> bool {
    let FieldType::Map(key, _) = ty else {
        return false;
    };
    let Some(tag) = parse_type_tag_prop(prop, FieldType::ALL_KEY_TYPES) else {
        return false;
    };
    let prev = (**key).clone();
    let Some(next_key) = resolve_leaf_field_type(tag, Some(&prev), None, first_enum_id) else {
        return false;
    };
    if **key == next_key {
        return false;
    }
    **key = next_key;
    true
}

/// Applies an enum target UUID change to the key type `K` of `Map<K, V>`.
pub fn apply_key_target_change(ty: &mut FieldType, prop: &PropValue) -> bool {
    let FieldType::Map(key, _) = ty else {
        return false;
    };
    apply_leaf_target_change(key.as_mut(), prop)
}

fn apply_leaf_target_change(leaf: &mut FieldType, prop: &PropValue) -> bool {
    match (leaf, prop) {
        (FieldType::Dto(u) | FieldType::SoftRef(u) | FieldType::Enum(u), PropValue::Str(s)) => {
            let next = if s.is_empty() {
                AssetUuid::NIL
            } else if let Some(parsed) = AssetUuid::parse(s) {
                parsed
            } else {
                return false;
            };
            if *u == next {
                return false;
            }
            *u = next;
            true
        }
        (FieldType::GameAsset(kind), PropValue::Int(i)) => {
            let Some(next) = usize::try_from(*i).ok().and_then(GameAssetKind::from_tag) else {
                return false;
            };
            if *kind == next {
                return false;
            }
            *kind = next;
            true
        }
        (FieldType::GameAsset(kind), PropValue::Str(s)) => {
            let next = s
                .trim()
                .parse::<usize>()
                .ok()
                .and_then(GameAssetKind::from_tag)
                .or_else(|| GameAssetKind::ALL.iter().copied().find(|k| k.label().eq_ignore_ascii_case(s.trim())));
            let Some(next) = next else {
                return false;
            };
            if *kind == next {
                return false;
            }
            *kind = next;
            true
        }
        _ => false,
    }
}

/// Coerces a field's default `val` when its type changes from `old_ty` to `new_ty`, preserving inner values across
/// `T <-> Option<T>` and `Array<T> <-> Set<T>` transitions when compatible.
pub fn coerce_default_for_type_change(
    old_ty: &FieldType,
    new_ty: &FieldType,
    val: &mut DataValue,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) {
    if new_ty.matches(val) {
        new_ty.ensure_value_with_schemas(val, dtos, enums);
        return;
    }
    if let FieldType::Option(new_inner) = new_ty {
        if let DataValue::Option(Some(prev_inner)) = val {
            let mut next_inner = (**prev_inner).clone();
            coerce_default_for_type_change(
                old_ty.inner_type().unwrap_or(old_ty),
                new_inner,
                &mut next_inner,
                dtos,
                enums,
            );
            *val = DataValue::Option(Some(Box::new(next_inner)));
            return;
        }
        if new_inner.matches(val) {
            let old_default = old_ty.default_value_with_schemas(dtos, enums);
            if *val != old_default {
                let mut inner_val = val.clone();
                new_inner.ensure_value_with_schemas(&mut inner_val, dtos, enums);
                *val = DataValue::Option(Some(Box::new(inner_val)));
            } else {
                *val = DataValue::Option(None);
            }
            return;
        }
    }
    if let DataValue::Option(Some(inner_val)) = val
        && new_ty.matches(inner_val)
    {
        let mut unwrapped = (**inner_val).clone();
        new_ty.ensure_value_with_schemas(&mut unwrapped, dtos, enums);
        *val = unwrapped;
        return;
    }
    match (&new_ty, val.clone()) {
        (FieldType::Array(inner), DataValue::Array(mut items) | DataValue::Set(mut items)) => {
            for item in &mut items {
                inner.ensure_value_with_schemas(item, dtos, enums);
            }
            *val = DataValue::Array(items);
            return;
        }
        (FieldType::Set(inner), DataValue::Set(mut items) | DataValue::Array(mut items)) => {
            for item in &mut items {
                inner.ensure_value_with_schemas(item, dtos, enums);
            }
            *val = DataValue::Set(items);
            return;
        }
        (FieldType::Map(k_ty, v_ty), DataValue::Map(mut entries)) => {
            for (k, v) in &mut entries {
                k_ty.ensure_value_with_schemas(k, dtos, enums);
                v_ty.ensure_value_with_schemas(v, dtos, enums);
            }
            *val = DataValue::Map(entries);
            return;
        }
        _ => {}
    }
    *val = new_ty.default_value_with_schemas(dtos, enums);
}

/// Toggles an `Option<T>` value between `None` and `Some(inner_default)`. Returns `true` when `slot` changed.
pub fn apply_set_option_some(
    ty: &FieldType,
    slot: &mut DataValue,
    some: bool,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> bool {
    let FieldType::Option(inner) = ty else {
        return false;
    };
    if some {
        if let DataValue::Option(Some(existing)) = slot
            && inner.matches(existing)
        {
            return false;
        }
        *slot = DataValue::Option(Some(Box::new(inner.default_value_with_schemas(dtos, enums))));
        true
    } else {
        if matches!(slot, DataValue::Option(None)) {
            return false;
        }
        *slot = DataValue::Option(None);
        true
    }
}

/// Appends a default element/entry to an `Array<T>`, `Set<K>`, or `Map<K, V>`. Returns `true` when `slot` changed.
pub fn apply_container_add(
    ty: &FieldType,
    slot: &mut DataValue,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> bool {
    ty.ensure_value_with_schemas(slot, dtos, enums);
    match (ty, slot) {
        (FieldType::Array(inner), DataValue::Array(items)) | (FieldType::Set(inner), DataValue::Set(items)) => {
            items.push(inner.default_value_with_schemas(dtos, enums));
            true
        }
        (FieldType::Map(k_ty, v_ty), DataValue::Map(entries)) => {
            entries.push((k_ty.default_value_with_schemas(dtos, enums), v_ty.default_value_with_schemas(dtos, enums)));
            true
        }
        _ => false,
    }
}

/// Removes the element/entry at `index` from an `Array<T>`, `Set<K>`, or `Map<K, V>`. Returns `true` when `slot` changed.
pub fn apply_container_remove(ty: &FieldType, slot: &mut DataValue, index: usize) -> bool {
    match (ty, slot) {
        (FieldType::Array(_), DataValue::Array(items)) | (FieldType::Set(_), DataValue::Set(items))
            if index < items.len() =>
        {
            items.remove(index);
            true
        }
        (FieldType::Map(_, _), DataValue::Map(entries)) if index < entries.len() => {
            entries.remove(index);
            true
        }
        _ => false,
    }
}

/// Reorders the element at `index` by `delta` (`-1` or `+1`) in an `Array<T>`. Returns `true` when `slot` changed.
pub fn apply_container_move(ty: &FieldType, slot: &mut DataValue, index: usize, delta: isize) -> bool {
    let (FieldType::Array(_), DataValue::Array(items)) = (ty, slot) else {
        return false;
    };
    let Some(target) = index.checked_add_signed(delta) else {
        return false;
    };
    if index >= items.len() || target >= items.len() || index == target {
        return false;
    }
    items.swap(index, target);
    true
}

/// Returns the indices of elements in `items` that share a duplicate key (`DE008`) under `key_ty`.
pub fn duplicate_set_indices(
    items: &[DataValue],
    key_ty: &FieldType,
    enums: &BTreeMap<String, EnumAsset>,
) -> BTreeSet<usize> {
    let target_en = key_ty.enum_target();
    let mut dups = BTreeSet::new();
    for i in 0..items.len() {
        for j in (i + 1)..items.len() {
            if items[i].cmp_key(&items[j], enums, target_en).is_eq() {
                dups.insert(i);
                dups.insert(j);
            }
        }
    }
    dups
}

/// Returns the indices of entries in `entries` that share a duplicate key (`DE008`) under `key_ty`.
pub fn duplicate_map_indices(
    entries: &[(DataValue, DataValue)],
    key_ty: &FieldType,
    enums: &BTreeMap<String, EnumAsset>,
) -> BTreeSet<usize> {
    let target_en = key_ty.enum_target();
    let mut dups = BTreeSet::new();
    for i in 0..entries.len() {
        for j in (i + 1)..entries.len() {
            if entries[i].0.cmp_key(&entries[j].0, enums, target_en).is_eq() {
                dups.insert(i);
                dups.insert(j);
            }
        }
    }
    dups
}

/// Builds a secondary inner-type selector (`T` from `ALL_INNER_TYPES` when `!for_set`, or `K` from `ALL_KEY_TYPES` when `for_set`).
pub fn inner_type_select(target: impl Into<PropKey>, inner_ty: &FieldType, for_set: bool) -> Box<dyn Scene> {
    let list: &[FieldType] = if for_set { FieldType::ALL_KEY_TYPES } else { &FieldType::ALL_INNER_TYPES };
    let labels: Vec<String> = list.iter().map(|t| t.label().to_string()).collect();
    let values: Vec<String> = list.iter().map(|t| t.tag().to_string()).collect();
    let cur_idx = list.iter().position(|t| t.tag() == inner_ty.tag()).unwrap_or(0);
    select_values(target, labels, values, cur_idx)
}

/// Builds a key-type selector (`K` from `FieldType::ALL_KEY_TYPES`) for `FieldType::Map(K, V)`.
pub fn key_type_select(target: impl Into<PropKey>, key_ty: &FieldType) -> Box<dyn Scene> {
    inner_type_select(target, key_ty, true)
}

/// Builds a target selector (`Dto`, `SoftRef`, `Enum`, or `GameAssetKind`) for a leaf [`FieldType`] when applicable.
pub fn leaf_target_select(
    target: impl Into<PropKey>,
    leaf_ty: &FieldType,
    self_dto_id: Option<AssetUuid>,
    allow_self_dto: bool,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> Option<Box<dyn Scene>> {
    match *leaf_ty {
        FieldType::Dto(target_id) | FieldType::SoftRef(target_id) => {
            let allow_self = allow_self_dto || leaf_ty.is_soft_ref();
            let mut labels = vec!["(Select DTO)".to_string()];
            let mut values = vec![String::new()];
            let mut cur_idx = 0usize;
            for (path, d) in dtos {
                if d.id.is_nil() || (!allow_self && self_dto_id == Some(d.id)) {
                    continue;
                }
                labels.push(format!("{path}.dto"));
                values.push(d.id.to_string());
                if d.id == target_id {
                    cur_idx = labels.len() - 1;
                }
            }
            if !target_id.is_nil() && cur_idx == 0 {
                labels.push(format!("(Missing: {target_id})"));
                values.push(target_id.to_string());
                cur_idx = labels.len() - 1;
            }
            Some(select_values(target, labels, values, cur_idx))
        }
        FieldType::Enum(target_id) => {
            let mut labels = vec!["(Select Enum)".to_string()];
            let mut values = vec![String::new()];
            let mut cur_idx = 0usize;
            for (path, e) in enums {
                if e.id.is_nil() {
                    continue;
                }
                labels.push(format!("{path}.enum"));
                values.push(e.id.to_string());
                if e.id == target_id {
                    cur_idx = labels.len() - 1;
                }
            }
            if !target_id.is_nil() && cur_idx == 0 {
                labels.push(format!("(Missing: {target_id})"));
                values.push(target_id.to_string());
                cur_idx = labels.len() - 1;
            }
            Some(select_values(target, labels, values, cur_idx))
        }
        FieldType::GameAsset(kind) => {
            let labels: Vec<String> = GameAssetKind::ALL.iter().map(|k| k.label().to_string()).collect();
            let values: Vec<String> = GameAssetKind::ALL.iter().map(|k| k.tag().to_string()).collect();
            Some(select_values(target, labels, values, kind.tag()))
        }
        _ => None,
    }
}

/// Builds the `[None | Some]` toggle button group for a `FieldType::Option(T)` value.
pub fn option_toggle_buttons(is_some: bool, none_action: Action, some_action: Action) -> Box<dyn Scene> {
    Box::new(button_group(vec![
        GroupButton {
            caption: Caption::Text("None".to_string()),
            action: none_action,
            selected: !is_some,
            disabled: false,
            tip: "Unset (None)".to_string(),
        },
        GroupButton {
            caption: Caption::Text("Some".to_string()),
            action: some_action,
            selected: is_some,
            disabled: false,
            tip: "Set value (Some)".to_string(),
        },
    ]))
}

const CONTAINER_ERR_COLOR: Color = Color::srgb(0.95, 0.42, 0.42);
const CONTAINER_ERR_BG: Color = Color::srgba(0.94, 0.27, 0.27, 0.14);
const CONTAINER_ERR_BORDER: Color = Color::srgba(0.94, 0.27, 0.27, 0.45);

/// Returns the compact summary string for a container header button (e.g. `"[2 items] [10, 20]"`).
pub fn container_summary_label(
    ty: &FieldType,
    val: &DataValue,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> String {
    let count_badge = match (ty, val) {
        (FieldType::Array(_), DataValue::Array(items)) => {
            format!("[{} {}]", items.len(), if items.len() == 1 { "item" } else { "items" })
        }
        (FieldType::Set(_), DataValue::Set(items)) => {
            format!("{{{} {}}}", items.len(), if items.len() == 1 { "item" } else { "items" })
        }
        (FieldType::Map(_, _), DataValue::Map(entries)) => {
            format!("{{{} {}}}", entries.len(), if entries.len() == 1 { "entry" } else { "entries" })
        }
        (FieldType::Array(_), _) => "[0 items]".to_string(),
        (FieldType::Set(_), _) => "{0 items}".to_string(),
        (FieldType::Map(_, _), _) => "{0 entries}".to_string(),
        _ => val.to_short_view_string_with_schemas(ty, dtos, enums),
    };
    let preview = val.to_short_view_string_with_schemas(ty, dtos, enums);
    if preview == "[]" || preview == "{}" || preview.is_empty() {
        count_badge
    } else {
        format!("{count_badge} {preview}")
    }
}

/// Builds the inline header bar (`{expand_toggle} | {dup_badge?} | {+}`) for an `Array<T>`, `Set<K>`, or `Map<K, V>`.
pub fn container_header_bar(
    ty: &FieldType,
    val: &DataValue,
    toggle_button: Box<dyn Scene>,
    add_action: Action,
    enums: &BTreeMap<String, EnumAsset>,
) -> Box<dyn Scene> {
    let has_dups = match (ty, val) {
        (FieldType::Set(inner), DataValue::Set(items)) => !duplicate_set_indices(items, inner, enums).is_empty(),
        (FieldType::Map(k_ty, _), DataValue::Map(entries)) => !duplicate_map_indices(entries, k_ty, enums).is_empty(),
        _ => false,
    };
    let mut children: Vec<Box<dyn Scene>> = vec![Box::new(bsn! {
        Node { flex_grow: 1.0, min_width: px(0) }
        Children [ @{toggle_button} ]
    })];
    if has_dups {
        let tip = Tooltip("Duplicate container keys detected (DE008)".to_string());
        children.push(Box::new(bsn! {
            Node {
                flex_shrink: 0.0,
                padding: UiRect::axes(px(5), px(1)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(3)),
            }
            BackgroundColor(CONTAINER_ERR_BG)
            BorderColor::all(CONTAINER_ERR_BORDER)
            tip
            Children [ @text("DE008 dup", CONTAINER_ERR_COLOR, 10.5) ]
        }));
    }
    let add_tip = Tooltip("Add element / entry".to_string());
    let add_act = OnActivate(add_action);
    children.push(Box::new(bsn! {
        @FeathersToolButton {
            @variant: ButtonVariant::Plain,
            @caption: bsn! { @icon(IconId::Plus, 13.0, ACCENT) },
        }
        Node { flex_shrink: 0.0 }
        add_act
        add_tip
    }));
    Box::new(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(4),
            flex_grow: 1.0,
            min_width: px(0),
        }
        Children [ {children} ]
    })
}

/// Callbacks used by [`build_container_items_rows`] and [`build_inline_value_and_subrows`] to render recursive
/// `Option<T>`, `Array<T>`, `Set<K>`, and `Map<K, V>` controls across `DtoEditor`, `EnumEditor`, and `CollectionEditor`.
#[allow(clippy::type_complexity)]
pub struct ContainerEditorCallbacks<'a> {
    pub value_prop: &'a dyn Fn(Vec<AssetUuid>) -> PropKey,
    pub toggle_expand: &'a dyn Fn(Vec<AssetUuid>, String, bool, &str) -> Box<dyn Scene>,
    pub set_option_some: &'a dyn Fn(Vec<AssetUuid>, bool) -> Action,
    pub container_add: &'a dyn Fn(Vec<AssetUuid>) -> Action,
    pub container_remove: &'a dyn Fn(Vec<AssetUuid>, usize) -> Action,
    pub container_move: &'a dyn Fn(Vec<AssetUuid>, usize, isize) -> Action,
    pub render_nested_fields:
        &'a dyn Fn(&[Field], Vec<AssetUuid>, usize, &mut BTreeSet<AssetUuid>) -> Vec<Box<dyn Scene>>,
}

/// Builds `(inline_control, expanded_subrows)` for any [`FieldType`] at `path`.
#[allow(clippy::too_many_arguments)]
pub fn build_inline_value_and_subrows(
    ty: &FieldType,
    val: &DataValue,
    settings: &FieldSettings,
    path: &[AssetUuid],
    depth: usize,
    visited: &mut BTreeSet<AssetUuid>,
    expanded: &BTreeSet<Vec<AssetUuid>>,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    catalog: &AssetPickerCatalog,
    cb: &ContainerEditorCallbacks<'_>,
) -> (Box<dyn Scene>, Vec<Box<dyn Scene>>) {
    let path_vec = path.to_vec();
    match ty {
        FieldType::Dto(target_id) => {
            let is_exp = expanded.contains(&path_vec);
            let summary = val.to_short_view_string_with_schemas(ty, dtos, enums);
            let sub_entry = dtos.iter().find(|(_, d)| d.id == *target_id);
            let toggle_tip = match sub_entry {
                Some((sub_path, _)) => format!("{sub_path}.dto — click to expand / collapse nested DTO fields"),
                None => "Expand / collapse nested DTO fields".to_string(),
            };
            let toggle_btn = (cb.toggle_expand)(path_vec.clone(), summary, is_exp, &toggle_tip);
            let mut sub_rows: Vec<Box<dyn Scene>> = Vec::new();
            if is_exp {
                if depth + 1 >= MAX_NESTING_DEPTH {
                    sub_rows.push(Box::new(bsn! {
                        Node { padding: UiRect::axes(px(16), px(4)) }
                        Children [ @text("Maximum nesting depth reached.", CONTAINER_ERR_COLOR, 11.0) ]
                    }));
                } else if visited.contains(target_id) {
                    sub_rows.push(Box::new(bsn! {
                        Node { padding: UiRect::axes(px(16), px(4)) }
                        Children [ @text("Recursive DTO cycle detected (DE003).", CONTAINER_ERR_COLOR, 11.0) ]
                    }));
                } else if let Some((_, sub_dto)) = sub_entry {
                    visited.insert(*target_id);
                    let child_rows = (cb.render_nested_fields)(&sub_dto.fields, path_vec, depth + 1, visited);
                    visited.remove(target_id);
                    sub_rows.push(Box::new(bsn! {
                        Node {
                            margin: UiRect { left: px(18), right: px(0), top: px(2), bottom: px(4) },
                            border: UiRect::left(px(2)),
                            flex_direction: FlexDirection::Column,
                        }
                        BorderColor::all(BORDER)
                        Children [ {child_rows} ]
                    }));
                } else {
                    sub_rows.push(Box::new(bsn! {
                        Node { padding: UiRect::axes(px(16), px(4)) }
                        Children [ @text(format!("Target DTO {} not found.", target_id), CONTAINER_ERR_COLOR, 11.0) ]
                    }));
                }
            }
            (toggle_btn, sub_rows)
        }
        FieldType::Enum(target_id) => {
            let sub_entry = enums.iter().find(|(_, e)| e.id == *target_id);
            let sub_enum = sub_entry.map(|(_, e)| e);
            let active_vid = val
                .as_enum()
                .map(|(vid, _)| vid)
                .or_else(|| sub_enum.and_then(|e| e.variants.first()).map(|v| v.id))
                .unwrap_or(AssetUuid::NIL);
            let variant_select = enum_variant_select((cb.value_prop)(path_vec.clone()), sub_enum, active_vid);
            let active_var_def = sub_enum.and_then(|e| e.variant_by_id(active_vid));
            let has_payload = active_var_def.is_some_and(|v| !v.fields.is_empty());
            let is_exp = expanded.contains(&path_vec);
            let inline_ctrl: Box<dyn Scene> = if has_payload {
                let summary = val.to_short_view_string_with_schemas(ty, dtos, enums);
                let toggle_tip = match sub_entry {
                    Some((sub_path, _)) => {
                        format!("{sub_path}.enum — click to expand / collapse variant payload fields")
                    }
                    None => "Expand / collapse enum variant payload fields".to_string(),
                };
                let toggle_btn = (cb.toggle_expand)(path_vec.clone(), summary, is_exp, &toggle_tip);
                Box::new(bsn! {
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: px(4),
                        flex_grow: 1.0,
                        min_width: px(0),
                    }
                    Children [
                        Node { width: px(120), flex_shrink: 0.0 }
                        Children [ @{variant_select} ]
                        --
                        @{toggle_btn}
                    ]
                })
            } else {
                variant_select
            };
            let mut sub_rows: Vec<Box<dyn Scene>> = Vec::new();
            if has_payload && is_exp {
                if depth + 1 >= MAX_NESTING_DEPTH {
                    sub_rows.push(Box::new(bsn! {
                        Node { padding: UiRect::axes(px(16), px(4)) }
                        Children [ @text("Maximum nesting depth reached.", CONTAINER_ERR_COLOR, 11.0) ]
                    }));
                } else if visited.contains(target_id) {
                    sub_rows.push(Box::new(bsn! {
                        Node { padding: UiRect::axes(px(16), px(4)) }
                        Children [ @text("Recursive Enum cycle detected (DE003).", CONTAINER_ERR_COLOR, 11.0) ]
                    }));
                } else if let Some(var_def) = active_var_def {
                    visited.insert(*target_id);
                    let child_rows = (cb.render_nested_fields)(&var_def.fields, path_vec, depth + 1, visited);
                    visited.remove(target_id);
                    sub_rows.push(Box::new(bsn! {
                        Node {
                            margin: UiRect { left: px(18), right: px(0), top: px(2), bottom: px(4) },
                            border: UiRect::left(px(2)),
                            flex_direction: FlexDirection::Column,
                        }
                        BorderColor::all(BORDER)
                        Children [ {child_rows} ]
                    }));
                }
            }
            (inline_ctrl, sub_rows)
        }
        FieldType::Option(inner) => {
            let opt_inner = val.as_option().flatten();
            let is_some = opt_inner.is_some();
            let toggle = option_toggle_buttons(
                is_some,
                (cb.set_option_some)(path_vec.clone(), false),
                (cb.set_option_some)(path_vec.clone(), true),
            );
            if let Some(inner_val) = opt_inner {
                let mut some_path = path_vec;
                some_path.push(path_step_option_some());
                let (inner_ctrl, sub_rows) = build_inline_value_and_subrows(
                    inner, inner_val, settings, &some_path, depth, visited, expanded, dtos, enums, catalog, cb,
                );
                let combined: Box<dyn Scene> = Box::new(bsn! {
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: px(6),
                        flex_grow: 1.0,
                        min_width: px(0),
                    }
                    Children [
                        Node { flex_shrink: 0.0 }
                        Children [ @{toggle} ]
                        --
                        Node { flex_grow: 1.0, min_width: px(0) }
                        Children [ @{inner_ctrl} ]
                    ]
                });
                (combined, sub_rows)
            } else {
                (toggle, Vec::new())
            }
        }
        FieldType::Array(_) | FieldType::Set(_) | FieldType::Map(_, _) => {
            let is_exp = expanded.contains(&path_vec);
            let summary = container_summary_label(ty, val, dtos, enums);
            let toggle_btn =
                (cb.toggle_expand)(path_vec.clone(), summary, is_exp, "Click to expand / collapse container items");
            let header = container_header_bar(ty, val, toggle_btn, (cb.container_add)(path_vec.clone()), enums);
            let mut sub_rows: Vec<Box<dyn Scene>> = Vec::new();
            if is_exp {
                let item_rows =
                    build_container_items_rows(ty, val, &path_vec, depth, visited, expanded, dtos, enums, catalog, cb);
                sub_rows.push(Box::new(bsn! {
                    Node {
                        margin: UiRect { left: px(18), right: px(0), top: px(2), bottom: px(4) },
                        border: UiRect::left(px(2)),
                        flex_direction: FlexDirection::Column,
                    }
                    BorderColor::all(BORDER)
                    Children [ {item_rows} ]
                }));
            }
            (header, sub_rows)
        }
        _ => {
            let ctrl = data_value_editor_with_catalog((cb.value_prop)(path_vec), ty, val, settings, catalog);
            (ctrl, Vec::new())
        }
    }
}

/// Builds the expanded item / entry rows for an `Array<T>`, `Set<K>`, or `Map<K, V>` at `path`.
#[allow(clippy::too_many_arguments)]
pub fn build_container_items_rows(
    ty: &FieldType,
    val: &DataValue,
    path: &[AssetUuid],
    depth: usize,
    visited: &mut BTreeSet<AssetUuid>,
    expanded: &BTreeSet<Vec<AssetUuid>>,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    catalog: &AssetPickerCatalog,
    cb: &ContainerEditorCallbacks<'_>,
) -> Vec<Box<dyn Scene>> {
    if depth >= MAX_NESTING_DEPTH {
        return vec![Box::new(bsn! {
            Node { padding: UiRect::all(px(6)) }
            Children [ @text("Maximum nesting depth reached.", CONTAINER_ERR_COLOR, 11.0) ]
        })];
    }
    let path_vec = path.to_vec();
    let mut rows: Vec<Box<dyn Scene>> = Vec::new();

    match ty {
        FieldType::Array(inner) => {
            let items = val.as_array().unwrap_or(&[]);
            if items.is_empty() {
                rows.push(Box::new(bsn! {
                    Node { padding: UiRect::axes(px(10), px(4)) }
                    Children [ @text("Empty array — click + to add an item.", TEXT_DIM, 11.0) ]
                }));
                return rows;
            }
            let len = items.len();
            for (idx, item_val) in items.iter().enumerate() {
                let mut item_path = path_vec.clone();
                item_path.push(path_step_item(idx));
                let (item_ctrl, sub_rows) = build_inline_value_and_subrows(
                    inner,
                    item_val,
                    &FieldSettings::default(),
                    &item_path,
                    depth + 1,
                    visited,
                    expanded,
                    dtos,
                    enums,
                    catalog,
                    cb,
                );
                let up_btn = container_icon_btn(
                    IconId::ArrowUp,
                    (cb.container_move)(path_vec.clone(), idx, -1),
                    idx == 0,
                    "Move item up",
                );
                let down_btn = container_icon_btn(
                    IconId::ArrowDown,
                    (cb.container_move)(path_vec.clone(), idx, 1),
                    idx + 1 >= len,
                    "Move item down",
                );
                let del_btn = container_icon_btn(
                    IconId::Delete,
                    (cb.container_remove)(path_vec.clone(), idx),
                    false,
                    "Remove item",
                );
                rows.push(Box::new(bsn! {
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: px(6),
                        min_height: px(28),
                        padding: UiRect::axes(px(8), px(2)),
                        border: UiRect::bottom(px(1)),
                    }
                    BorderColor::all(FORM_ROW_LINE)
                    Children [
                        Node { width: px(38), flex_shrink: 0.0 }
                        Children [ @text(format!("[{idx}]"), TEXT_DIM, 11.0) ]
                        --
                        Node { flex_grow: 1.0, min_width: px(100) }
                        Children [ @{item_ctrl} ]
                        --
                        @{up_btn}
                        --
                        @{down_btn}
                        --
                        @{del_btn}
                    ]
                }));
                rows.extend(sub_rows);
            }
        }
        FieldType::Set(inner) => {
            let items = val.as_set().unwrap_or(&[]);
            if items.is_empty() {
                rows.push(Box::new(bsn! {
                    Node { padding: UiRect::axes(px(10), px(4)) }
                    Children [ @text("Empty set — click + to add an item.", TEXT_DIM, 11.0) ]
                }));
                return rows;
            }
            let dups = duplicate_set_indices(items, inner, enums);
            for (idx, item_val) in items.iter().enumerate() {
                let mut item_path = path_vec.clone();
                item_path.push(path_step_item(idx));
                let is_dup = dups.contains(&idx);
                let item_ctrl = data_value_editor_with_catalog(
                    (cb.value_prop)(item_path),
                    inner.as_ref(),
                    item_val,
                    &FieldSettings::default(),
                    catalog,
                );
                let del_btn = container_icon_btn(
                    IconId::Delete,
                    (cb.container_remove)(path_vec.clone(), idx),
                    false,
                    "Remove item",
                );
                let idx_color = if is_dup { CONTAINER_ERR_COLOR } else { TEXT_DIM };
                let row_bg = if is_dup { CONTAINER_ERR_BG } else { Color::NONE };
                let mut row_children: Vec<Box<dyn Scene>> = vec![
                    Box::new(bsn! {
                        Node { width: px(38), flex_shrink: 0.0 }
                        Children [ @text(format!("[{idx}]"), idx_color, 11.0) ]
                    }),
                    Box::new(bsn! {
                        Node { flex_grow: 1.0, min_width: px(100) }
                        Children [ @{item_ctrl} ]
                    }),
                ];
                if is_dup {
                    row_children.push(Box::new(bsn! {
                        Node {
                            flex_shrink: 0.0,
                            padding: UiRect::axes(px(4), px(1)),
                            border: UiRect::all(px(1)),
                            border_radius: BorderRadius::all(px(3)),
                        }
                        BackgroundColor(CONTAINER_ERR_BG)
                        BorderColor::all(CONTAINER_ERR_BORDER)
                        Children [ @text("dup", CONTAINER_ERR_COLOR, 10.0) ]
                    }));
                }
                row_children.push(del_btn);
                rows.push(Box::new(bsn! {
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: px(6),
                        min_height: px(28),
                        padding: UiRect::axes(px(8), px(2)),
                        border: UiRect::bottom(px(1)),
                    }
                    BackgroundColor(row_bg)
                    BorderColor::all(FORM_ROW_LINE)
                    Children [ {row_children} ]
                }));
            }
        }
        FieldType::Map(key_ty, val_ty) => {
            let entries = val.as_map().unwrap_or(&[]);
            if entries.is_empty() {
                rows.push(Box::new(bsn! {
                    Node { padding: UiRect::axes(px(10), px(4)) }
                    Children [ @text("Empty map — click + to add an entry.", TEXT_DIM, 11.0) ]
                }));
                return rows;
            }
            let dups = duplicate_map_indices(entries, key_ty, enums);
            for (idx, (k_val, v_val)) in entries.iter().enumerate() {
                let mut key_path = path_vec.clone();
                key_path.push(path_step_map_key(idx));
                let mut val_path = path_vec.clone();
                val_path.push(path_step_map_val(idx));
                let is_dup = dups.contains(&idx);
                let key_ctrl = data_value_editor_with_catalog(
                    (cb.value_prop)(key_path),
                    key_ty.as_ref(),
                    k_val,
                    &FieldSettings::default(),
                    catalog,
                );
                let (val_ctrl, sub_rows) = build_inline_value_and_subrows(
                    val_ty,
                    v_val,
                    &FieldSettings::default(),
                    &val_path,
                    depth + 1,
                    visited,
                    expanded,
                    dtos,
                    enums,
                    catalog,
                    cb,
                );
                let del_btn = container_icon_btn(
                    IconId::Delete,
                    (cb.container_remove)(path_vec.clone(), idx),
                    false,
                    "Remove entry",
                );
                let idx_color = if is_dup { CONTAINER_ERR_COLOR } else { TEXT_DIM };
                let row_bg = if is_dup { CONTAINER_ERR_BG } else { Color::NONE };
                let mut row_children: Vec<Box<dyn Scene>> = vec![
                    Box::new(bsn! {
                        Node { width: px(38), flex_shrink: 0.0 }
                        Children [ @text(format!("[{idx}]"), idx_color, 11.0) ]
                    }),
                    Box::new(bsn! {
                        Node { width: px(136), flex_shrink: 0.0 }
                        Children [ @{key_ctrl} ]
                    }),
                    Box::new(text("→", TEXT_DIM, 11.0)),
                    Box::new(bsn! {
                        Node { flex_grow: 1.0, min_width: px(100) }
                        Children [ @{val_ctrl} ]
                    }),
                ];
                if is_dup {
                    row_children.push(Box::new(bsn! {
                        Node {
                            flex_shrink: 0.0,
                            padding: UiRect::axes(px(4), px(1)),
                            border: UiRect::all(px(1)),
                            border_radius: BorderRadius::all(px(3)),
                        }
                        BackgroundColor(CONTAINER_ERR_BG)
                        BorderColor::all(CONTAINER_ERR_BORDER)
                        Children [ @text("dup", CONTAINER_ERR_COLOR, 10.0) ]
                    }));
                }
                row_children.push(del_btn);
                rows.push(Box::new(bsn! {
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: px(6),
                        min_height: px(28),
                        padding: UiRect::axes(px(8), px(2)),
                        border: UiRect::bottom(px(1)),
                    }
                    BackgroundColor(row_bg)
                    BorderColor::all(FORM_ROW_LINE)
                    Children [ {row_children} ]
                }));
                rows.extend(sub_rows);
            }
        }
        _ => {}
    }

    rows
}

fn container_icon_btn(icon_id: IconId, action: Action, disabled: bool, tooltip: &str) -> Box<dyn Scene> {
    let tip = Tooltip(tooltip.to_string());
    if disabled {
        let dim = TEXT_DIM.with_alpha(0.3);
        Box::new(bsn! {
            @FeathersToolButton {
                @variant: ButtonVariant::Plain,
                @caption: bsn! { @icon(icon_id, 12.0, dim) },
            }
            Node { flex_shrink: 0.0 }
            bevy::ui::InteractionDisabled
            tip
        })
    } else {
        let act = OnActivate(action);
        Box::new(bsn! {
            @FeathersToolButton {
                @variant: ButtonVariant::Plain,
                @caption: bsn! { @icon(icon_id, 12.0, TEXT) },
            }
            Node { flex_shrink: 0.0 }
            act
            tip
        })
    }
}
