//! Rust code generation for `crates/quillshade/src/data/`, `crates/quillshade/src/data/dtos/**`, and
//! `crates/quillshade/src/data/enums/**` (ADR 0049 §7, techdoc §9.1–§9.4).

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};

use project_core::{AssetIndex, AssetUuid, DomainId, Project, marker};

use super::bake::DataBakeResolver;
use super::model::{
    CollectionAsset, DataValue, DtoAsset, EnumAsset, Field, FieldType, MAX_NESTING_DEPTH, collection_ref, dto_ref,
    enum_ref, to_pascal_case, to_upper_snake_case,
};
use super::validate::is_rust_keyword;

const DATA_MOD_TEMPLATE: &str = include_str!("templates/data_mod.rs");

/// Relative path under `crates/quillshade/src/data/` for a generated DTO module (`"dtos/<dto_path>.rs"`).
pub fn dto_rust_rel_path(dto_path: &str) -> String {
    format!("dtos/{dto_path}.rs")
}

/// Rust submodule identifier inside `crates/quillshade/src/data/dtos/mod.rs` (`"dto_<slug>"`).
pub fn dto_module_ident(dto_path: &str) -> String {
    format!("dto_{}", dto_path.replace('/', "_"))
}

/// Relative path under `crates/quillshade/src/data/` for a generated Enum module (`"enums/<enum_path>.rs"`).
pub fn enum_rust_rel_path(enum_path: &str) -> String {
    format!("enums/{enum_path}.rs")
}

/// Rust submodule identifier inside `crates/quillshade/src/data/enums/mod.rs` (`"enum_<slug>"`).
pub fn enum_module_ident(enum_path: &str) -> String {
    format!("enum_{}", enum_path.replace('/', "_"))
}

/// Relative path under `crates/quillshade/src/data/` for a generated Collection module (`"collections/<col_path>.rs"`).
pub fn collection_rust_rel_path(col_path: &str) -> String {
    format!("collections/{col_path}.rs")
}

/// Rust submodule identifier inside `crates/quillshade/src/data/collections/mod.rs` (`"col_<slug>"`).
pub fn collection_module_ident(col_path: &str) -> String {
    format!("col_{}", col_path.replace('/', "_"))
}

/// Returns the Rust field type emitted in `pub struct <Pascal>` for `ty` (ADR 0049 §2).
pub fn rust_field_type(ty: impl Borrow<FieldType>) -> &'static str {
    match ty.borrow() {
        FieldType::Bool => "bool",
        FieldType::I8 => "i8",
        FieldType::I16 => "i16",
        FieldType::I32 => "i32",
        FieldType::I64 => "i64",
        FieldType::U8 => "u8",
        FieldType::U16 => "u16",
        FieldType::U32 => "u32",
        FieldType::U64 => "u64",
        FieldType::F32 => "f32",
        FieldType::F64 => "f64",
        FieldType::String => "String",
        FieldType::LocalizedString => "super::LocalizedString",
        FieldType::Vec2 => "bevy::math::Vec2",
        FieldType::Vec3 => "bevy::math::Vec3",
        FieldType::Vec4 => "bevy::math::Vec4",
        FieldType::IVec2 => "bevy::math::IVec2",
        FieldType::IVec3 => "bevy::math::IVec3",
        FieldType::UVec2 => "bevy::math::UVec2",
        FieldType::UVec3 => "bevy::math::UVec3",
        FieldType::Quat => "bevy::math::Quat",
        FieldType::Color => "bevy::color::Color",
        FieldType::Duration => "std::time::Duration",
        FieldType::RangeF32 => "super::RangeF32",
        FieldType::RangeI32 => "super::RangeI32",
        FieldType::Dto(_) | FieldType::Enum(_) => "()",
        FieldType::Curve => "super::CurveData",
        FieldType::Gradient => "super::GradientData",
        FieldType::Material => "Option<bevy::asset::UntypedHandle>",
        FieldType::Texture => "Option<bevy::asset::Handle<bevy::image::Image>>",
        FieldType::LightEnvironment => "Option<crate::light::Preset>",
        FieldType::GameAsset(_) => "Option<bevy::asset::UntypedHandle>",
        FieldType::SoftRef(_) => "super::SoftRef<()>",
        FieldType::Option(_) => "Option<()>",
        FieldType::Array(_) => "Vec<()>",
        FieldType::Set(_) => "std::collections::BTreeSet<()>",
        FieldType::Map(_, _) => "std::collections::BTreeMap<(), ()>",
    }
}

/// Returns `true` when `FieldType::Option(inner)` must wrap its payload in `Box<...>` (`Option<Box<T>>`) to break a
/// direct or mutual recursion cycle through inline `Dto`, `Enum`, and `Option` fields.
pub fn option_needs_box(
    inner: &FieldType,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> bool {
    let (FieldType::Dto(start_id) | FieldType::Enum(start_id)) = inner else {
        return false;
    };
    if start_id.is_nil() {
        return false;
    }
    schema_reaches_self_inline(*start_id, dtos, enums)
}

fn schema_reaches_self_inline(
    start_id: AssetUuid,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> bool {
    fn collect_inline_targets(ty: &FieldType, out: &mut Vec<AssetUuid>) {
        match ty {
            FieldType::Dto(id) | FieldType::Enum(id) if !id.is_nil() => out.push(*id),
            FieldType::Option(inner) => collect_inline_targets(inner, out),
            _ => {}
        }
    }

    let mut visited = BTreeSet::new();
    let mut stack = Vec::new();
    if let Some((_, dto)) = dtos.iter().find(|(_, d)| d.id == start_id) {
        for f in &dto.fields {
            collect_inline_targets(&f.ty, &mut stack);
        }
    } else if let Some((_, en)) = enums.iter().find(|(_, e)| e.id == start_id) {
        for v in &en.variants {
            for f in &v.fields {
                collect_inline_targets(&f.ty, &mut stack);
            }
        }
    }
    while let Some(curr) = stack.pop() {
        if curr == start_id {
            return true;
        }
        if !visited.insert(curr) {
            continue;
        }
        if let Some((_, dto)) = dtos.iter().find(|(_, d)| d.id == curr) {
            for f in &dto.fields {
                collect_inline_targets(&f.ty, &mut stack);
            }
        } else if let Some((_, en)) = enums.iter().find(|(_, e)| e.id == curr) {
            for v in &en.variants {
                for f in &v.fields {
                    collect_inline_targets(&f.ty, &mut stack);
                }
            }
        }
    }
    false
}

/// Returns the Rust field type emitted in `pub struct <Pascal>` for `ty`, resolving `FieldType::Dto` against `dtos`.
pub fn rust_field_type_with_dtos(ty: impl Borrow<FieldType>, dtos: &BTreeMap<String, DtoAsset>) -> String {
    rust_field_type_with_context(ty, dtos, true)
}

/// Returns the Rust field type emitted in `pub struct <Pascal>` for `ty`, resolving `FieldType::Dto` against `dtos` and
/// `FieldType::LightEnvironment` based on whether the `light` domain is enabled (`Option<crate::light::Preset>` vs `Option<String>`).
pub fn rust_field_type_with_context(
    ty: impl Borrow<FieldType>,
    dtos: &BTreeMap<String, DtoAsset>,
    light_enabled: bool,
) -> String {
    let empty_enums = BTreeMap::new();
    rust_field_type_with_schemas_and_context(ty, dtos, &empty_enums, light_enabled)
}

/// Returns the Rust field type emitted in `pub struct <Pascal>` or `pub enum <Pascal>` for `ty`, resolving
/// `FieldType::Dto` and `FieldType::Enum` against `dtos` and `enums`.
pub fn rust_field_type_with_schemas(
    ty: impl Borrow<FieldType>,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> String {
    rust_field_type_with_schemas_and_context(ty, dtos, enums, true)
}

/// Returns the Rust field type emitted in `pub struct <Pascal>` or `pub enum <Pascal>` for `ty`, resolving
/// `FieldType::Dto`, `FieldType::Enum`, `FieldType::SoftRef`, `FieldType::LightEnvironment`, and container types.
pub fn rust_field_type_with_schemas_and_context(
    ty: impl Borrow<FieldType>,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    light_enabled: bool,
) -> String {
    let ty = ty.borrow();
    match ty {
        FieldType::Option(inner) => {
            let inner_ty = rust_field_type_with_schemas_and_context(inner.as_ref(), dtos, enums, light_enabled);
            if option_needs_box(inner, dtos, enums) {
                return format!("Option<Box<{inner_ty}>>");
            }
            return format!("Option<{inner_ty}>");
        }
        FieldType::Array(inner) => {
            let inner_ty = rust_field_type_with_schemas_and_context(inner.as_ref(), dtos, enums, light_enabled);
            return format!("Vec<{inner_ty}>");
        }
        FieldType::Set(key) => {
            let key_ty = rust_field_type_with_schemas_and_context(key.as_ref(), dtos, enums, light_enabled);
            return format!("std::collections::BTreeSet<{key_ty}>");
        }
        FieldType::Map(key, val) => {
            let key_ty = rust_field_type_with_schemas_and_context(key.as_ref(), dtos, enums, light_enabled);
            let val_ty = rust_field_type_with_schemas_and_context(val.as_ref(), dtos, enums, light_enabled);
            return format!("std::collections::BTreeMap<{key_ty}, {val_ty}>");
        }
        _ => {}
    }
    if let FieldType::Dto(target_uuid) = ty
        && let Some((target_path, _)) = dtos.iter().find(|(_, d)| !d.id.is_nil() && d.id == *target_uuid)
    {
        return format!("super::{}", to_pascal_case(target_path));
    }
    if let FieldType::Enum(target_uuid) = ty
        && let Some((target_path, _)) = enums.iter().find(|(_, e)| !e.id.is_nil() && e.id == *target_uuid)
    {
        return format!("super::{}", to_pascal_case(target_path));
    }
    if let FieldType::SoftRef(target_uuid) = ty
        && let Some((target_path, _)) = dtos.iter().find(|(_, d)| !d.id.is_nil() && d.id == *target_uuid)
    {
        return format!("super::SoftRef<super::{}>", to_pascal_case(target_path));
    }
    if matches!(ty, FieldType::LightEnvironment) && !light_enabled {
        return "Option<String>".to_string();
    }
    rust_field_type(ty).to_string()
}

/// Returns the `postcard` wire type emitted in `pub struct <Pascal>Wire(...)` for `ty` (ADR 0049 §2).
pub fn wire_field_type(ty: impl Borrow<FieldType>) -> &'static str {
    match ty.borrow() {
        FieldType::Bool => "bool",
        FieldType::I8 => "i8",
        FieldType::I16 => "i16",
        FieldType::I32 => "i32",
        FieldType::I64 => "i64",
        FieldType::U8 => "u8",
        FieldType::U16 => "u16",
        FieldType::U32 => "u32",
        FieldType::U64 => "u64",
        FieldType::F32 | FieldType::Duration => "f32",
        FieldType::F64 => "f64",
        FieldType::String | FieldType::LocalizedString | FieldType::LightEnvironment | FieldType::GameAsset(_) => {
            "String"
        }
        FieldType::Vec2 | FieldType::RangeF32 => "[f32; 2]",
        FieldType::Vec3 => "[f32; 3]",
        FieldType::Vec4 | FieldType::Quat | FieldType::Color => "[f32; 4]",
        FieldType::IVec2 | FieldType::RangeI32 => "[i32; 2]",
        FieldType::IVec3 => "[i32; 3]",
        FieldType::UVec2 => "[u32; 2]",
        FieldType::UVec3 => "[u32; 3]",
        FieldType::Dto(_) | FieldType::Enum(_) => "()",
        FieldType::Curve => "super::CurveData",
        FieldType::Gradient => "super::GradientData",
        FieldType::Material | FieldType::SoftRef(_) => "u128",
        FieldType::Texture => "(u128, String)",
        FieldType::Option(_) => "Option<()>",
        FieldType::Array(_) => "Vec<()>",
        FieldType::Set(_) => "std::collections::BTreeSet<()>",
        FieldType::Map(_, _) => "std::collections::BTreeMap<(), ()>",
    }
}

/// Returns the `postcard` wire type emitted in `pub struct <Pascal>Wire(...)` for `ty`, resolving `FieldType::Dto`
/// against `dtos`.
pub fn wire_field_type_with_dtos(ty: impl Borrow<FieldType>, dtos: &BTreeMap<String, DtoAsset>) -> String {
    let empty_enums = BTreeMap::new();
    wire_field_type_with_schemas(ty, dtos, &empty_enums)
}

/// Returns the `postcard` wire type emitted in `pub struct <Pascal>Wire(...)` or `pub enum <Pascal>Wire` for `ty`,
/// resolving `FieldType::Dto`, `FieldType::Enum`, and container types against `dtos` and `enums`.
pub fn wire_field_type_with_schemas(
    ty: impl Borrow<FieldType>,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> String {
    let ty = ty.borrow();
    match ty {
        FieldType::Option(inner) => {
            let inner_ty = wire_field_type_with_schemas(inner.as_ref(), dtos, enums);
            if option_needs_box(inner, dtos, enums) {
                return format!("Option<Box<{inner_ty}>>");
            }
            return format!("Option<{inner_ty}>");
        }
        FieldType::Array(inner) => {
            let inner_ty = wire_field_type_with_schemas(inner.as_ref(), dtos, enums);
            return format!("Vec<{inner_ty}>");
        }
        FieldType::Set(key) => {
            let key_ty = wire_field_type_with_schemas(key.as_ref(), dtos, enums);
            return format!("std::collections::BTreeSet<{key_ty}>");
        }
        FieldType::Map(key, val) => {
            let key_ty = wire_field_type_with_schemas(key.as_ref(), dtos, enums);
            let val_ty = wire_field_type_with_schemas(val.as_ref(), dtos, enums);
            return format!("std::collections::BTreeMap<{key_ty}, {val_ty}>");
        }
        _ => {}
    }
    if let FieldType::Dto(target_uuid) = ty
        && let Some((target_path, _)) = dtos.iter().find(|(_, d)| !d.id.is_nil() && d.id == *target_uuid)
    {
        return format!("super::{}Wire", to_pascal_case(target_path));
    }
    if let FieldType::Enum(target_uuid) = ty
        && let Some((target_path, _)) = enums.iter().find(|(_, e)| !e.id.is_nil() && e.id == *target_uuid)
    {
        return format!("super::{}Wire", to_pascal_case(target_path));
    }
    wire_field_type(ty).to_string()
}

fn fmt_f32(v: f32) -> String {
    if v.is_finite() { format!("{v:?}") } else { "0.0".to_string() }
}

fn fmt_f64(v: f64) -> String {
    if v.is_finite() { format!("{v:?}") } else { "0.0".to_string() }
}

#[allow(clippy::too_many_arguments)]
fn emit_default_literal_with_schemas_and_context(
    ty: &FieldType,
    raw_default: &DataValue,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    resolver: &DataBakeResolver<'_>,
    light_enabled: bool,
    visited: &mut BTreeSet<AssetUuid>,
    depth: usize,
    indent: &str,
) -> String {
    if let FieldType::Dto(target_uuid) = ty {
        let Some((target_path, target_dto)) = dtos.iter().find(|(_, d)| !d.id.is_nil() && d.id == *target_uuid) else {
            return "()".to_string();
        };
        let target_pascal = to_pascal_case(target_path);
        if depth >= MAX_NESTING_DEPTH || !visited.insert(*target_uuid) {
            return format!("super::{target_pascal}::default()");
        }
        if target_dto.fields.is_empty() {
            visited.remove(target_uuid);
            return format!("super::{target_pascal} {{}}");
        }
        let map = raw_default.as_dto_map();
        let child_indent = format!("{indent}    ");
        let mut s = format!("super::{target_pascal} {{\n");
        for t_field in &target_dto.fields {
            let sub_val = map.and_then(|m| m.get(&t_field.id)).unwrap_or(&t_field.default);
            let sub_lit = emit_default_literal_with_schemas_and_context(
                &t_field.ty,
                sub_val,
                dtos,
                enums,
                resolver,
                light_enabled,
                visited,
                depth + 1,
                &child_indent,
            );
            s.push_str(&format!("{child_indent}{}: {sub_lit},\n", t_field.name));
        }
        s.push_str(&format!("{indent}}}"));
        visited.remove(target_uuid);
        return s;
    }

    if let FieldType::Enum(target_uuid) = ty {
        let Some((target_path, target_enum)) = enums.iter().find(|(_, e)| !e.id.is_nil() && e.id == *target_uuid)
        else {
            return "()".to_string();
        };
        let target_pascal = to_pascal_case(target_path);
        if depth >= MAX_NESTING_DEPTH || !visited.insert(*target_uuid) || target_enum.variants.is_empty() {
            return format!("super::{target_pascal}::default()");
        }
        let variant_id = raw_default.as_enum().map(|(v, _)| v).unwrap_or(AssetUuid::NIL);
        let fields_map = raw_default.as_enum().map(|(_, m)| m);
        let active_variant =
            target_enum.variant_by_id(variant_id).or_else(|| target_enum.variants.first()).expect("non-empty variants");
        let var_pascal = to_pascal_case(&active_variant.name);
        if active_variant.fields.is_empty() {
            visited.remove(target_uuid);
            return format!("super::{target_pascal}::{var_pascal}");
        }
        let child_indent = format!("{indent}    ");
        let mut s = format!("super::{target_pascal}::{var_pascal} {{\n");
        for vf in &active_variant.fields {
            let sub_val = fields_map.and_then(|m| m.get(&vf.id)).unwrap_or(&vf.default);
            let sub_lit = emit_default_literal_with_schemas_and_context(
                &vf.ty,
                sub_val,
                dtos,
                enums,
                resolver,
                light_enabled,
                visited,
                depth + 1,
                &child_indent,
            );
            s.push_str(&format!("{child_indent}{}: {sub_lit},\n", vf.name));
        }
        s.push_str(&format!("{indent}}}"));
        visited.remove(target_uuid);
        return s;
    }

    if let FieldType::Option(inner) = ty {
        if depth >= MAX_NESTING_DEPTH {
            return "None".to_string();
        }
        if let DataValue::Option(Some(inner_val)) = raw_default {
            let mut inner_visited = BTreeSet::new();
            let sub_lit = emit_default_literal_with_schemas_and_context(
                inner,
                inner_val,
                dtos,
                enums,
                resolver,
                light_enabled,
                &mut inner_visited,
                depth + 1,
                indent,
            );
            if option_needs_box(inner, dtos, enums) {
                return format!("Some(Box::new({sub_lit}))");
            }
            return format!("Some({sub_lit})");
        }
        return "None".to_string();
    }

    if let FieldType::Array(inner) = ty {
        if depth >= MAX_NESTING_DEPTH {
            return "Vec::new()".to_string();
        }
        if let DataValue::Array(items) = raw_default
            && !items.is_empty()
        {
            let elems: Vec<String> = items
                .iter()
                .map(|item| {
                    let mut item_visited = BTreeSet::new();
                    emit_default_literal_with_schemas_and_context(
                        inner,
                        item,
                        dtos,
                        enums,
                        resolver,
                        light_enabled,
                        &mut item_visited,
                        depth + 1,
                        indent,
                    )
                })
                .collect();
            return format!("vec![{}]", elems.join(", "));
        }
        return "Vec::new()".to_string();
    }

    if let FieldType::Set(key_ty) = ty {
        if depth >= MAX_NESTING_DEPTH {
            return "std::collections::BTreeSet::new()".to_string();
        }
        if let DataValue::Set(items) = raw_default
            && !items.is_empty()
        {
            let target_en = key_ty.enum_target();
            let mut sorted: Vec<&DataValue> = items.iter().collect();
            sorted.sort_by(|a, b| a.cmp_key(b, enums, target_en));
            sorted.dedup_by(|a, b| a.cmp_key(b, enums, target_en).is_eq());
            let elems: Vec<String> = sorted
                .into_iter()
                .map(|item| {
                    let mut item_visited = BTreeSet::new();
                    emit_default_literal_with_schemas_and_context(
                        key_ty,
                        item,
                        dtos,
                        enums,
                        resolver,
                        light_enabled,
                        &mut item_visited,
                        depth + 1,
                        indent,
                    )
                })
                .collect();
            return format!("std::collections::BTreeSet::from([{}])", elems.join(", "));
        }
        return "std::collections::BTreeSet::new()".to_string();
    }

    if let FieldType::Map(key_ty, val_ty) = ty {
        if depth >= MAX_NESTING_DEPTH {
            return "std::collections::BTreeMap::new()".to_string();
        }
        if let DataValue::Map(entries) = raw_default
            && !entries.is_empty()
        {
            let target_en = key_ty.enum_target();
            let mut sorted: Vec<&(DataValue, DataValue)> = entries.iter().collect();
            sorted.sort_by(|(ka, _), (kb, _)| ka.cmp_key(kb, enums, target_en));
            sorted.dedup_by(|(ka, _), (kb, _)| ka.cmp_key(kb, enums, target_en).is_eq());
            let pairs: Vec<String> = sorted
                .into_iter()
                .map(|(k, v)| {
                    let mut k_visited = BTreeSet::new();
                    let k_lit = emit_default_literal_with_schemas_and_context(
                        key_ty,
                        k,
                        dtos,
                        enums,
                        resolver,
                        light_enabled,
                        &mut k_visited,
                        depth + 1,
                        indent,
                    );
                    let mut v_visited = BTreeSet::new();
                    let v_lit = emit_default_literal_with_schemas_and_context(
                        val_ty,
                        v,
                        dtos,
                        enums,
                        resolver,
                        light_enabled,
                        &mut v_visited,
                        depth + 1,
                        indent,
                    );
                    format!("({k_lit}, {v_lit})")
                })
                .collect();
            return format!("std::collections::BTreeMap::from([{}])", pairs.join(", "));
        }
        return "std::collections::BTreeMap::new()".to_string();
    }

    let fallback;
    let val = if ty.matches(raw_default) {
        raw_default
    } else {
        fallback = ty.default_value();
        &fallback
    };
    match val {
        DataValue::Bool(v) => format!("{v}"),
        DataValue::I8(v) => format!("{v}i8"),
        DataValue::I16(v) => format!("{v}i16"),
        DataValue::I32(v) => format!("{v}i32"),
        DataValue::I64(v) => format!("{v}i64"),
        DataValue::U8(v) => format!("{v}u8"),
        DataValue::U16(v) => format!("{v}u16"),
        DataValue::U32(v) => format!("{v}u32"),
        DataValue::U64(v) => format!("{v}u64"),
        DataValue::F32(v) => format!("{}_f32", fmt_f32(*v)),
        DataValue::F64(v) => format!("{}_f64", fmt_f64(*v)),
        DataValue::String(s) => {
            if s.is_empty() {
                "String::new()".to_string()
            } else {
                format!("String::from({s:?})")
            }
        }
        DataValue::LocalizedString(s) => {
            if s.is_empty() {
                "super::LocalizedString(String::new())".to_string()
            } else {
                format!("super::LocalizedString(String::from({s:?}))")
            }
        }
        DataValue::Vec2([x, y]) => format!("bevy::math::Vec2::new({}, {})", fmt_f32(*x), fmt_f32(*y)),
        DataValue::Vec3([x, y, z]) => {
            format!("bevy::math::Vec3::new({}, {}, {})", fmt_f32(*x), fmt_f32(*y), fmt_f32(*z))
        }
        DataValue::Vec4([x, y, z, w]) => {
            format!("bevy::math::Vec4::new({}, {}, {}, {})", fmt_f32(*x), fmt_f32(*y), fmt_f32(*z), fmt_f32(*w))
        }
        DataValue::IVec2([x, y]) => format!("bevy::math::IVec2::new({x}, {y})"),
        DataValue::IVec3([x, y, z]) => format!("bevy::math::IVec3::new({x}, {y}, {z})"),
        DataValue::UVec2([x, y]) => format!("bevy::math::UVec2::new({x}, {y})"),
        DataValue::UVec3([x, y, z]) => format!("bevy::math::UVec3::new({x}, {y}, {z})"),
        DataValue::Quat([x, y, z, w]) => {
            format!("bevy::math::Quat::from_xyzw({}, {}, {}, {})", fmt_f32(*x), fmt_f32(*y), fmt_f32(*z), fmt_f32(*w))
        }
        DataValue::Color([r, g, b, a]) => format!(
            "bevy::color::Color::linear_rgba({}, {}, {}, {})",
            fmt_f32(*r),
            fmt_f32(*g),
            fmt_f32(*b),
            fmt_f32(*a)
        ),
        DataValue::Duration(secs) => {
            let s = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
            format!("std::time::Duration::from_secs_f32({})", fmt_f32(s))
        }
        DataValue::RangeF32([min, max]) => format!("super::RangeF32::new({}, {})", fmt_f32(*min), fmt_f32(*max)),
        DataValue::RangeI32([min, max]) => format!("super::RangeI32::new({min}, {max})"),
        DataValue::Dto(_) | DataValue::Enum { .. } => "()".to_string(),
        DataValue::Curve(uuid) => {
            let c = resolver.resolve_curve(*uuid);
            let interp = match c.interp {
                project_core::common::curve::Interp::Simple => "super::Interp::Simple",
                project_core::common::curve::Interp::CatmullRom => "super::Interp::CatmullRom",
            };
            let pts: Vec<String> =
                c.points.iter().map(|&(x, y)| format!("({}_f32, {}_f32)", fmt_f32(x), fmt_f32(y))).collect();
            format!(
                "super::CurveData {{ interp: {interp}, min: {}_f32, max: {}_f32, points: vec![{}] }}",
                fmt_f32(c.min),
                fmt_f32(c.max),
                pts.join(", ")
            )
        }
        DataValue::Gradient(uuid, slot) => {
            let g = resolver.resolve_gradient(*uuid, *slot);
            let stops: Vec<String> = g
                .stops
                .iter()
                .map(|s| {
                    format!(
                        "super::GradientStop {{ t: {}_f32, color: bevy::color::Color::linear_rgba({}_f32, {}_f32, {}_f32, {}_f32), mid: Some({}_f32) }}",
                        fmt_f32(s.t),
                        fmt_f32(s.color[0]),
                        fmt_f32(s.color[1]),
                        fmt_f32(s.color[2]),
                        fmt_f32(s.color[3]),
                        fmt_f32(s.mid)
                    )
                })
                .collect();
            format!("super::GradientData {{ stops: vec![{}] }}", stops.join(", "))
        }
        DataValue::Material(uuid) => {
            if uuid.is_nil() {
                "None".to_string()
            } else {
                format!("super::material_uuid_handle({:#034x}_u128)", uuid.as_u128())
            }
        }
        DataValue::Texture(uuid) => {
            if uuid.is_nil() {
                "None".to_string()
            } else {
                format!("super::texture_uuid_handle({:#034x}_u128)", uuid.as_u128())
            }
        }
        DataValue::LightEnvironment(uuid) => {
            let path = resolver.resolve_light_env(*uuid);
            if path.is_empty() {
                "None".to_string()
            } else if light_enabled {
                format!("crate::light::Preset::from_path({path:?})")
            } else {
                format!("Some(String::from({path:?}))")
            }
        }
        DataValue::GameAsset(path) => {
            if path.is_empty() {
                "None".to_string()
            } else {
                format!("super::game_asset_path_handle({path:?})")
            }
        }
        DataValue::SoftRef(row_uuid) => {
            let raw = if row_uuid.is_nil() { 0 } else { row_uuid.as_u128() };
            if raw == 0 {
                "super::SoftRef::from_raw(0u128)".to_string()
            } else {
                format!("super::SoftRef::from_raw({raw:#034x}u128)")
            }
        }
        DataValue::Option(_) => "None".to_string(),
        DataValue::Array(_) => "Vec::new()".to_string(),
        DataValue::Set(_) => "std::collections::BTreeSet::new()".to_string(),
        DataValue::Map(_) => "std::collections::BTreeMap::new()".to_string(),
    }
}

fn type_needs_wire_conversion(ty: &FieldType) -> bool {
    match ty {
        FieldType::Bool
        | FieldType::I8
        | FieldType::I16
        | FieldType::I32
        | FieldType::I64
        | FieldType::U8
        | FieldType::U16
        | FieldType::U32
        | FieldType::U64
        | FieldType::F32
        | FieldType::F64
        | FieldType::String
        | FieldType::Curve
        | FieldType::Gradient => false,
        FieldType::Option(inner) | FieldType::Array(inner) | FieldType::Set(inner) => type_needs_wire_conversion(inner),
        FieldType::Map(k, v) => type_needs_wire_conversion(k) || type_needs_wire_conversion(v),
        _ => true,
    }
}

fn emit_from_wire_access_expr(
    access: &str,
    ty: &FieldType,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    light_enabled: bool,
) -> String {
    emit_from_wire_access_expr_depth(access, ty, dtos, enums, light_enabled, 0)
}

fn map_closure_or_fn(var: &str, expr: &str) -> String {
    let suffix = format!("({var})");
    if let Some(func) = expr.strip_suffix(&suffix)
        && !func.is_empty()
        && func.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
    {
        func.to_string()
    } else {
        format!("|{var}| {expr}")
    }
}

fn emit_from_wire_access_expr_depth(
    access: &str,
    ty: &FieldType,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    light_enabled: bool,
    depth: usize,
) -> String {
    match ty {
        FieldType::Bool
        | FieldType::I8
        | FieldType::I16
        | FieldType::I32
        | FieldType::I64
        | FieldType::U8
        | FieldType::U16
        | FieldType::U32
        | FieldType::U64
        | FieldType::F32
        | FieldType::F64
        | FieldType::String
        | FieldType::Curve
        | FieldType::Gradient => access.to_string(),
        FieldType::LocalizedString => format!("super::LocalizedString({access})"),
        FieldType::Vec2 => format!("bevy::math::Vec2::from_array({access})"),
        FieldType::Vec3 => format!("bevy::math::Vec3::from_array({access})"),
        FieldType::Vec4 => format!("bevy::math::Vec4::from_array({access})"),
        FieldType::IVec2 => format!("bevy::math::IVec2::from_array({access})"),
        FieldType::IVec3 => format!("bevy::math::IVec3::from_array({access})"),
        FieldType::UVec2 => format!("bevy::math::UVec2::from_array({access})"),
        FieldType::UVec3 => format!("bevy::math::UVec3::from_array({access})"),
        FieldType::Quat => format!("bevy::math::Quat::from_array({access})"),
        FieldType::Color => {
            format!("bevy::color::Color::linear_rgba({access}[0], {access}[1], {access}[2], {access}[3])")
        }
        FieldType::Duration => {
            format!(
                "std::time::Duration::from_secs_f32(if {access}.is_finite() {{ {access}.max(0.0) }} else {{ 0.0 }})"
            )
        }
        FieldType::RangeF32 => format!("super::RangeF32::new({access}[0], {access}[1])"),
        FieldType::RangeI32 => format!("super::RangeI32::new({access}[0], {access}[1])"),
        FieldType::Dto(target_uuid) => {
            if let Some((target_path, _)) = dtos.iter().find(|(_, d)| !d.id.is_nil() && d.id == *target_uuid) {
                format!("super::{}::from_wire({access}, ctx)", to_pascal_case(target_path))
            } else {
                access.to_string()
            }
        }
        FieldType::Enum(target_uuid) => {
            if let Some((target_path, _)) = enums.iter().find(|(_, e)| !e.id.is_nil() && e.id == *target_uuid) {
                format!("super::{}::from_wire({access}, ctx)", to_pascal_case(target_path))
            } else {
                access.to_string()
            }
        }
        FieldType::Material => format!("ctx.resolve_material({access})"),
        FieldType::Texture => format!("ctx.resolve_texture({access}.0, &{access}.1)"),
        FieldType::LightEnvironment => {
            if light_enabled {
                format!("if {access}.is_empty() {{ None }} else {{ crate::light::Preset::from_path(&{access}) }}")
            } else {
                format!("if {access}.is_empty() {{ None }} else {{ Some({access}) }}")
            }
        }
        FieldType::GameAsset(_) => format!("ctx.resolve_game_asset(&{access})"),
        FieldType::SoftRef(_) => format!("super::SoftRef::from_raw({access})"),
        FieldType::Option(inner) => {
            let boxed = option_needs_box(inner, dtos, enums);
            if !boxed && !type_needs_wire_conversion(inner) {
                access.to_string()
            } else {
                let var = if depth == 0 { "v".to_string() } else { format!("v{depth}") };
                if boxed {
                    let inner_expr = emit_from_wire_access_expr_depth(
                        &format!("*{var}"),
                        inner,
                        dtos,
                        enums,
                        light_enabled,
                        depth + 1,
                    );
                    format!("{access}.map(|{var}| Box::new({inner_expr}))")
                } else {
                    let inner_expr =
                        emit_from_wire_access_expr_depth(&var, inner, dtos, enums, light_enabled, depth + 1);
                    let mapper = map_closure_or_fn(&var, &inner_expr);
                    format!("{access}.map({mapper})")
                }
            }
        }
        FieldType::Array(inner) => {
            if !type_needs_wire_conversion(inner) {
                access.to_string()
            } else {
                let var = if depth == 0 { "v".to_string() } else { format!("v{depth}") };
                let inner_expr = emit_from_wire_access_expr_depth(&var, inner, dtos, enums, light_enabled, depth + 1);
                let mapper = map_closure_or_fn(&var, &inner_expr);
                format!("{access}.into_iter().map({mapper}).collect()")
            }
        }
        FieldType::Set(key) => {
            if !type_needs_wire_conversion(key) {
                access.to_string()
            } else {
                let var = if depth == 0 { "k".to_string() } else { format!("k{depth}") };
                let key_expr = emit_from_wire_access_expr_depth(&var, key, dtos, enums, light_enabled, depth + 1);
                let mapper = map_closure_or_fn(&var, &key_expr);
                format!("{access}.into_iter().map({mapper}).collect()")
            }
        }
        FieldType::Map(key, val) => {
            if !type_needs_wire_conversion(key) && !type_needs_wire_conversion(val) {
                access.to_string()
            } else {
                let k_var = if depth == 0 { "k".to_string() } else { format!("k{depth}") };
                let v_var = if depth == 0 { "v".to_string() } else { format!("v{depth}") };
                let key_expr = emit_from_wire_access_expr_depth(&k_var, key, dtos, enums, light_enabled, depth + 1);
                let val_expr = emit_from_wire_access_expr_depth(&v_var, val, dtos, enums, light_enabled, depth + 1);
                format!("{access}.into_iter().map(|({k_var}, {v_var})| ({key_expr}, {val_expr})).collect()")
            }
        }
    }
}

fn emit_from_wire_expr_with_schemas_and_context(
    idx: usize,
    ty: &FieldType,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    light_enabled: bool,
) -> String {
    let access = format!("wire.{idx}");
    emit_from_wire_access_expr(&access, ty, dtos, enums, light_enabled)
}

fn push_doc_comment(out: &mut String, indent: &str, doc: &str) {
    let trimmed = doc.trim();
    if trimmed.is_empty() {
        return;
    }
    for line in trimmed.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            out.push_str(&format!("{indent}///\n"));
        } else {
            out.push_str(&format!("{indent}/// {line}\n"));
        }
    }
}

/// Emits the generated Rust module for a single `.dto.ron` schema (`crates/quillshade/src/data/dtos/<dto_path>.rs`).
pub fn emit_dto_file(dto_path: &str, dto: &DtoAsset) -> String {
    let empty = BTreeMap::new();
    emit_dto_file_with_dtos(dto_path, dto, &empty)
}

/// Emits the generated Rust module for a single `.dto.ron` schema (`crates/quillshade/src/data/dtos/<dto_path>.rs`),
/// resolving by-value `FieldType::Dto` fields against `dtos`.
pub fn emit_dto_file_with_dtos(dto_path: &str, dto: &DtoAsset, dtos: &BTreeMap<String, DtoAsset>) -> String {
    emit_dto_file_with_context(dto_path, dto, dtos, None, None)
}

/// Emits the generated Rust module for a single `.dto.ron` schema (`crates/quillshade/src/data/dtos/<dto_path>.rs`),
/// resolving by-value `FieldType::Dto` and `FieldType::Enum` fields against `dtos` and `enums`.
pub fn emit_dto_file_with_schemas(
    dto_path: &str,
    dto: &DtoAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> String {
    emit_dto_file_with_schemas_and_context(dto_path, dto, dtos, enums, None, None)
}

/// Emits the generated Rust module for a single `.dto.ron` schema (`crates/quillshade/src/data/dtos/<dto_path>.rs`),
/// resolving `FieldType::Dto` fields against `dtos` and cross-domain asset defaults against `(project, index)`.
pub fn emit_dto_file_with_context(
    dto_path: &str,
    dto: &DtoAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    project: Option<&Project>,
    index: Option<&AssetIndex>,
) -> String {
    let empty_enums = BTreeMap::new();
    emit_dto_file_with_schemas_and_context(dto_path, dto, dtos, &empty_enums, project, index)
}

/// Emits the generated Rust module for a single `.dto.ron` schema (`crates/quillshade/src/data/dtos/<dto_path>.rs`),
/// resolving `FieldType::Dto` and `FieldType::Enum` fields against `dtos` and `enums` and cross-domain asset defaults
/// against `(project, index)`.
pub fn emit_dto_file_with_schemas_and_context(
    dto_path: &str,
    dto: &DtoAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    project: Option<&Project>,
    index: Option<&AssetIndex>,
) -> String {
    let source_rel = dto_ref(dto_path).source_rel();
    let pascal = to_pascal_case(dto_path);
    let wire_name = format!("{pascal}Wire");
    let dto_id = dto.id.as_u128();
    let light_enabled = project.is_none_or(|p| p.domain_enabled(DomainId::new("light")));
    let resolver = DataBakeResolver { project, index };

    let mut o = format!(
        "{}\n//! Generated DTO schema for `{source_rel}`.\n#![allow(clippy::approx_constant, clippy::excessive_precision)]\n\n",
        marker::header("//", Some(&source_rel))
    );

    if dto.doc.trim().is_empty() {
        o.push_str(&format!("/// Generated DTO struct for `{dto_path}` (`{source_rel}`).\n"));
    } else {
        push_doc_comment(&mut o, "", &dto.doc);
    }
    o.push_str("#[derive(Clone, Debug, PartialEq)]\n");
    if dto.fields.is_empty() {
        o.push_str(&format!("pub struct {pascal} {{}}\n\n"));
    } else {
        o.push_str(&format!("pub struct {pascal} {{\n"));
        for field in &dto.fields {
            push_doc_comment(&mut o, "    ", &field.settings.doc);
            o.push_str(&format!(
                "    pub {}: {},\n",
                field.name,
                rust_field_type_with_schemas_and_context(&field.ty, dtos, enums, light_enabled)
            ));
        }
        o.push_str("}\n\n");
    }

    o.push_str(&format!(
        "impl {pascal} {{\n    /// Stable 128-bit UUIDv7 of `{source_rel}`.\n    pub const DTO_ID: u128 = {dto_id:#034x};\n}}\n\n"
    ));

    o.push_str("#[allow(clippy::derivable_impls)]\n");
    o.push_str(&format!("impl Default for {pascal} {{\n    fn default() -> Self {{\n"));
    if dto.fields.is_empty() {
        o.push_str("        Self {}\n");
    } else {
        o.push_str("        Self {\n");
        let mut visited = BTreeSet::new();
        if !dto.id.is_nil() {
            visited.insert(dto.id);
        }
        for field in &dto.fields {
            let lit = emit_default_literal_with_schemas_and_context(
                &field.ty,
                &field.default,
                dtos,
                enums,
                &resolver,
                light_enabled,
                &mut visited,
                0,
                "            ",
            );
            o.push_str(&format!("            {}: {lit},\n", field.name));
        }
        o.push_str("        }\n");
    }
    o.push_str("    }\n}\n\n");

    let name_fields: Vec<&Field> = dto.fields.iter().filter(|f| f.is_name).collect();
    let fallback_string_fields: Vec<&Field> = dto.fields.iter().filter(|f| f.ty.is_string() && !f.is_name).collect();

    o.push_str(&format!(
        "impl super::ToShortViewString for {pascal} {{\n    fn to_short_view_string(&self) -> String {{\n"
    ));
    if !name_fields.is_empty() {
        o.push_str("        let mut parts: Vec<String> = Vec::new();\n");
        for field in &name_fields {
            if matches!(field.ty, FieldType::LightEnvironment) {
                o.push_str(&format!(
                    "        let s = match &self.{} {{\n            Some(p) => p.as_str().to_string(),\n            None => String::from(\"None\"),\n        }};\n",
                    field.name
                ));
            } else {
                o.push_str(&format!(
                    "        let s = super::ToShortViewString::to_short_view_string(&self.{});\n",
                    field.name
                ));
            }
            o.push_str("        if !s.trim().is_empty() {\n            parts.push(s);\n        }\n");
        }
        o.push_str("        if !parts.is_empty() {\n            return parts.join(\" | \");\n        }\n");
    }
    for field in &fallback_string_fields {
        o.push_str(&format!("        let s = super::ToShortViewString::to_short_view_string(&self.{});\n", field.name));
        o.push_str("        if !s.trim().is_empty() {\n            return s;\n        }\n");
    }
    o.push_str(&format!("        String::from({pascal:?})\n    }}\n}}\n\n"));

    o.push_str(&format!(
        "impl std::fmt::Display for {pascal} {{\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n        f.write_str(&super::ToShortViewString::to_short_view_string(self))\n    }}\n}}\n\n"
    ));

    o.push_str("#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]\n");
    if dto.fields.is_empty() {
        o.push_str(&format!("pub struct {wire_name}(pub ());\n\n"));
        o.push_str(&format!(
            "impl {pascal} {{\n    /// Converts a decoded [`{wire_name}`] tuple into [`{pascal}`].\n    #[allow(unused_variables)]\n    pub fn from_wire(_wire: {wire_name}, _ctx: &super::DataResolveCtx<'_>) -> Self {{\n        Self {{}}\n    }}\n}}\n\n"
        ));
    } else {
        let wire_tys: Vec<String> =
            dto.fields.iter().map(|f| format!("pub {}", wire_field_type_with_schemas(&f.ty, dtos, enums))).collect();
        o.push_str(&format!("pub struct {wire_name}({});\n\n", wire_tys.join(", ")));
        o.push_str(&format!(
            "impl {pascal} {{\n    /// Converts a decoded [`{wire_name}`] tuple into [`{pascal}`], resolving asset handles via `ctx`.\n    #[allow(unused_variables)]\n    pub fn from_wire(wire: {wire_name}, ctx: &super::DataResolveCtx<'_>) -> Self {{\n        Self {{\n"
        ));
        for (idx, field) in dto.fields.iter().enumerate() {
            o.push_str(&format!(
                "            {}: {},\n",
                field.name,
                emit_from_wire_expr_with_schemas_and_context(idx, &field.ty, dtos, enums, light_enabled)
            ));
        }
        o.push_str("        }\n    }\n}\n\n");
    }

    o.push_str(&format!(
        "impl From<{wire_name}> for {pascal} {{\n    fn from(wire: {wire_name}) -> Self {{\n        Self::from_wire(wire, &super::DataResolveCtx::default())\n    }}\n}}\n\n"
    ));

    o.push_str(&format!(
        "impl super::DataDto for {pascal} {{\n    const DTO_ID: u128 = Self::DTO_ID;\n\n    fn decode_qsd_payload(\n        payload: &[u8],\n        ctx: &super::DataResolveCtx<'_>,\n    ) -> Result<super::Collection<Self>, String> {{\n        let wire_rows: Vec<(u128, {wire_name})> =\n            postcard::from_bytes(payload).map_err(|e| format!(\"postcard decode error: {{e}}\"))?;\n        let rows = wire_rows\n            .into_iter()\n            .map(|(row_id, wire)| (super::RowUuid::from_u128(row_id), Self::from_wire(wire, ctx)))\n            .collect();\n        Ok(super::Collection::from_rows(rows))\n    }}\n}}\n"
    ));

    o
}

/// Emits `crates/quillshade/src/data/dtos/mod.rs`.
pub fn emit_dtos_mod(dtos: &BTreeMap<String, DtoAsset>) -> String {
    let mut o = format!(
        "{}\n//! Generated DTO modules (`quillshade/data/**/*.dto.ron`).\n\n#[allow(unused_imports)]\npub(super) use super::{{Collection, CurveData, DataDto, DataResolveCtx, GradientData, GradientStop, Interp, LocalizedString, RangeF32, RangeI32, RowUuid, SoftRef, ToShortViewString, game_asset_path_handle, material_uuid_handle, texture_uuid_handle}};\n#[allow(unused_imports)]\nuse super::enums::*;\n",
        marker::header("//", None)
    );

    if dtos.is_empty() {
        return o;
    }

    let full_pascals: BTreeSet<String> = dtos.keys().map(|p| to_pascal_case(p)).collect();
    let mut leaf_counts: BTreeMap<String, usize> = BTreeMap::new();
    for dto_path in dtos.keys() {
        let leaf = dto_path.rsplit('/').next().unwrap_or(dto_path.as_str());
        *leaf_counts.entry(to_pascal_case(leaf)).or_default() += 1;
    }

    o.push('\n');
    for dto_path in dtos.keys() {
        let mod_ident = dto_module_ident(dto_path);
        let pascal = to_pascal_case(dto_path);
        let wire_name = format!("{pascal}Wire");
        o.push_str(&format!(
            "#[path = \"{dto_path}.rs\"]\npub mod {mod_ident};\npub use {mod_ident}::{{{pascal}, {wire_name}}};\n"
        ));

        if dto_path.contains('/') {
            let leaf = dto_path.rsplit('/').next().unwrap_or(dto_path.as_str());
            let leaf_pascal = to_pascal_case(leaf);
            if leaf_pascal != pascal
                && leaf_counts.get(&leaf_pascal) == Some(&1)
                && !full_pascals.contains(&leaf_pascal)
            {
                o.push_str(&format!(
                    "/// Unambiguous short alias for [`{pascal}`] (`{dto_path}.dto.ron`).\npub type {leaf_pascal} = {pascal};\npub type {leaf_pascal}Wire = {wire_name};\n"
                ));
            }
        }
    }

    o
}

/// Emits the generated Rust module for a single `.enum.ron` schema (`crates/quillshade/src/data/enums/<enum_path>.rs`).
pub fn emit_enum_file(enum_path: &str, en: &EnumAsset) -> String {
    let empty_dtos = BTreeMap::new();
    let empty_enums = BTreeMap::new();
    emit_enum_file_with_schemas(enum_path, en, &empty_dtos, &empty_enums)
}

/// Emits the generated Rust module for a single `.enum.ron` schema (`crates/quillshade/src/data/enums/<enum_path>.rs`),
/// resolving nested `FieldType::Dto` and `FieldType::Enum` payload fields against `dtos` and `enums`.
pub fn emit_enum_file_with_schemas(
    enum_path: &str,
    en: &EnumAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> String {
    emit_enum_file_with_schemas_and_context(enum_path, en, dtos, enums, None, None)
}

/// Emits the generated Rust module for a single `.enum.ron` schema (`crates/quillshade/src/data/enums/<enum_path>.rs`),
/// resolving nested `FieldType::Dto` and `FieldType::Enum` payload fields against `dtos` and `enums` and cross-domain
/// asset defaults against `(project, index)`.
pub fn emit_enum_file_with_schemas_and_context(
    enum_path: &str,
    en: &EnumAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    project: Option<&Project>,
    index: Option<&AssetIndex>,
) -> String {
    let source_rel = enum_ref(enum_path).source_rel();
    let pascal = to_pascal_case(enum_path);
    let wire_name = format!("{pascal}Wire");
    let enum_id = en.id.as_u128();
    let light_enabled = project.is_none_or(|p| p.domain_enabled(DomainId::new("light")));
    let resolver = DataBakeResolver { project, index };
    let unit_only = en.is_unit_only();

    let mut o = format!(
        "{}\n//! Generated Enum schema for `{source_rel}`.\n#![allow(clippy::approx_constant, clippy::excessive_precision)]\n\n",
        marker::header("//", Some(&source_rel))
    );

    if en.doc.trim().is_empty() {
        o.push_str(&format!("/// Generated enum for `{enum_path}` (`{source_rel}`).\n"));
    } else {
        push_doc_comment(&mut o, "", &en.doc);
    }
    if unit_only {
        o.push_str("#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]\n");
    } else {
        o.push_str("#[derive(Clone, Debug, PartialEq)]\n");
    }
    o.push_str(&format!("pub enum {pascal} {{\n"));
    if en.variants.is_empty() {
        o.push_str("    _Empty,\n");
    } else {
        for variant in &en.variants {
            push_doc_comment(&mut o, "    ", &variant.doc);
            let var_pascal = to_pascal_case(&variant.name);
            if variant.fields.is_empty() {
                o.push_str(&format!("    {var_pascal},\n"));
            } else {
                o.push_str(&format!("    {var_pascal} {{\n"));
                for field in &variant.fields {
                    push_doc_comment(&mut o, "        ", &field.settings.doc);
                    o.push_str(&format!(
                        "        {}: {},\n",
                        field.name,
                        rust_field_type_with_schemas_and_context(&field.ty, dtos, enums, light_enabled)
                    ));
                }
                o.push_str("    },\n");
            }
        }
    }
    o.push_str("}\n\n");

    o.push_str(&format!(
        "impl {pascal} {{\n    /// Stable 128-bit UUIDv7 of `{source_rel}`.\n    pub const ENUM_ID: u128 = {enum_id:#034x};\n}}\n\n"
    ));

    o.push_str("#[allow(clippy::derivable_impls)]\n");
    o.push_str(&format!("impl Default for {pascal} {{\n    fn default() -> Self {{\n"));
    if let Some(first) = en.variants.first() {
        let var_pascal = to_pascal_case(&first.name);
        if first.fields.is_empty() {
            o.push_str(&format!("        Self::{var_pascal}\n"));
        } else {
            o.push_str(&format!("        Self::{var_pascal} {{\n"));
            let mut visited = BTreeSet::new();
            if !en.id.is_nil() {
                visited.insert(en.id);
            }
            for field in &first.fields {
                let lit = emit_default_literal_with_schemas_and_context(
                    &field.ty,
                    &field.default,
                    dtos,
                    enums,
                    &resolver,
                    light_enabled,
                    &mut visited,
                    0,
                    "            ",
                );
                o.push_str(&format!("            {}: {lit},\n", field.name));
            }
            o.push_str("        }\n");
        }
    } else {
        o.push_str("        Self::_Empty\n");
    }
    o.push_str("    }\n}\n\n");

    o.push_str(&format!(
        "impl super::ToShortViewString for {pascal} {{\n    fn to_short_view_string(&self) -> String {{\n        match self {{\n"
    ));
    if en.variants.is_empty() {
        o.push_str(&format!("            Self::_Empty => String::from({pascal:?}),\n"));
    }
    for variant in &en.variants {
        let var_pascal = to_pascal_case(&variant.name);
        if variant.fields.is_empty() {
            o.push_str(&format!("            Self::{var_pascal} => String::from({var_pascal:?}),\n"));
        } else {
            let name_fields: Vec<&Field> = variant.fields.iter().filter(|f| f.is_name).collect();
            let fallback_string_fields: Vec<&Field> =
                variant.fields.iter().filter(|f| f.ty.is_string() && !f.is_name).collect();
            if name_fields.is_empty() && fallback_string_fields.is_empty() {
                o.push_str(&format!("            Self::{var_pascal} {{ .. }} => String::from({var_pascal:?}),\n"));
            } else {
                let bound_names: Vec<&str> =
                    name_fields.iter().chain(fallback_string_fields.iter()).map(|f| f.name.as_str()).collect();
                o.push_str(&format!("            Self::{var_pascal} {{ {}, .. }} => {{\n", bound_names.join(", ")));
                if !name_fields.is_empty() {
                    o.push_str("                let mut parts: Vec<String> = Vec::new();\n");
                    for field in &name_fields {
                        if matches!(field.ty, FieldType::LightEnvironment) {
                            o.push_str(&format!(
                                "                let s = match {} {{\n                    Some(p) => p.as_str().to_string(),\n                    None => String::from(\"None\"),\n                }};\n",
                                field.name
                            ));
                        } else {
                            o.push_str(&format!(
                                "                let s = super::ToShortViewString::to_short_view_string({});\n",
                                field.name
                            ));
                        }
                        o.push_str(
                            "                if !s.trim().is_empty() {\n                    parts.push(s);\n                }\n",
                        );
                    }
                    o.push_str(&format!(
                        "                if !parts.is_empty() {{\n                    return format!(\"{var_pascal}({{}})\", parts.join(\" | \"));\n                }}\n"
                    ));
                }
                for field in &fallback_string_fields {
                    o.push_str(&format!(
                        "                let s = super::ToShortViewString::to_short_view_string({});\n",
                        field.name
                    ));
                    o.push_str(&format!(
                        "                if !s.trim().is_empty() {{\n                    return format!(\"{var_pascal}({{s}})\");\n                }}\n"
                    ));
                }
                o.push_str(&format!("                String::from({var_pascal:?})\n            }}\n"));
            }
        }
    }
    o.push_str("        }\n    }\n}\n\n");

    o.push_str(&format!(
        "impl std::fmt::Display for {pascal} {{\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n        f.write_str(&super::ToShortViewString::to_short_view_string(self))\n    }}\n}}\n\n"
    ));

    if unit_only {
        o.push_str(
            "#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]\n",
        );
    } else {
        o.push_str("#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]\n");
    }
    o.push_str(&format!("pub enum {wire_name} {{\n"));
    if en.variants.is_empty() {
        o.push_str("    _Empty,\n");
    } else {
        for variant in &en.variants {
            let var_pascal = to_pascal_case(&variant.name);
            if variant.fields.is_empty() {
                o.push_str(&format!("    {var_pascal},\n"));
            } else {
                let wire_tys: Vec<String> =
                    variant.fields.iter().map(|f| wire_field_type_with_schemas(&f.ty, dtos, enums)).collect();
                o.push_str(&format!("    {var_pascal}({}),\n", wire_tys.join(", ")));
            }
        }
    }
    o.push_str("}\n\n");

    o.push_str(&format!(
        "impl {pascal} {{\n    /// Converts a decoded [`{wire_name}`] value into [`{pascal}`], resolving asset handles via `ctx`.\n    #[allow(unused_variables)]\n    pub fn from_wire(wire: {wire_name}, ctx: &super::DataResolveCtx<'_>) -> Self {{\n        match wire {{\n"
    ));
    if en.variants.is_empty() {
        o.push_str(&format!("            {wire_name}::_Empty => Self::_Empty,\n"));
    } else {
        for variant in &en.variants {
            let var_pascal = to_pascal_case(&variant.name);
            if variant.fields.is_empty() {
                o.push_str(&format!("            {wire_name}::{var_pascal} => Self::{var_pascal},\n"));
            } else {
                let pat_vars: Vec<String> = (0..variant.fields.len()).map(|i| format!("f{i}")).collect();
                o.push_str(&format!(
                    "            {wire_name}::{var_pascal}({}) => Self::{var_pascal} {{\n",
                    pat_vars.join(", ")
                ));
                for (idx, field) in variant.fields.iter().enumerate() {
                    let access = format!("f{idx}");
                    o.push_str(&format!(
                        "                {}: {},\n",
                        field.name,
                        emit_from_wire_access_expr(&access, &field.ty, dtos, enums, light_enabled)
                    ));
                }
                o.push_str("            },\n");
            }
        }
    }
    o.push_str("        }\n    }\n}\n\n");

    o.push_str(&format!(
        "impl From<{wire_name}> for {pascal} {{\n    fn from(wire: {wire_name}) -> Self {{\n        Self::from_wire(wire, &super::DataResolveCtx::default())\n    }}\n}}\n"
    ));

    o
}

/// Emits `crates/quillshade/src/data/enums/mod.rs`.
pub fn emit_enums_mod(enums: &BTreeMap<String, EnumAsset>) -> String {
    let mut o = format!(
        "{}\n//! Generated Enum modules (`quillshade/data/**/*.enum.ron`).\n\n#[allow(unused_imports)]\npub(super) use super::{{CurveData, DataResolveCtx, GradientData, GradientStop, Interp, LocalizedString, RangeF32, RangeI32, SoftRef, ToShortViewString, game_asset_path_handle, material_uuid_handle, texture_uuid_handle}};\n#[allow(unused_imports)]\nuse super::dtos::*;\n",
        marker::header("//", None)
    );

    if enums.is_empty() {
        return o;
    }

    let full_pascals: BTreeSet<String> = enums.keys().map(|p| to_pascal_case(p)).collect();
    let mut leaf_counts: BTreeMap<String, usize> = BTreeMap::new();
    for enum_path in enums.keys() {
        let leaf = enum_path.rsplit('/').next().unwrap_or(enum_path.as_str());
        *leaf_counts.entry(to_pascal_case(leaf)).or_default() += 1;
    }

    o.push('\n');
    for enum_path in enums.keys() {
        let mod_ident = enum_module_ident(enum_path);
        let pascal = to_pascal_case(enum_path);
        let wire_name = format!("{pascal}Wire");
        o.push_str(&format!(
            "#[path = \"{enum_path}.rs\"]\npub mod {mod_ident};\npub use {mod_ident}::{{{pascal}, {wire_name}}};\n"
        ));

        if enum_path.contains('/') {
            let leaf = enum_path.rsplit('/').next().unwrap_or(enum_path.as_str());
            let leaf_pascal = to_pascal_case(leaf);
            if leaf_pascal != pascal
                && leaf_counts.get(&leaf_pascal) == Some(&1)
                && !full_pascals.contains(&leaf_pascal)
            {
                o.push_str(&format!(
                    "/// Unambiguous short alias for [`{pascal}`] (`{enum_path}.enum.ron`).\npub type {leaf_pascal} = {pascal};\npub type {leaf_pascal}Wire = {wire_name};\n"
                ));
            }
        }
    }

    o
}

#[derive(Clone)]
struct ResolvedCollection<'a> {
    col_path: &'a str,
    col: &'a CollectionAsset,
    dto_pascal: String,
    full_const: String,
    leaf_const: String,
    full_snake: String,
    leaf_snake: String,
    compile_len: usize,
}

fn compile_row_count(col: &CollectionAsset, skipped_rows: &BTreeSet<usize>) -> usize {
    col.rows.iter().enumerate().filter(|(idx, _)| !skipped_rows.contains(idx)).count()
}

fn resolve_collections<'a>(
    dtos: &BTreeMap<String, DtoAsset>,
    collections: &'a BTreeMap<String, CollectionAsset>,
    skipped_rows: &BTreeSet<(String, usize)>,
) -> Vec<ResolvedCollection<'a>> {
    let mut resolved_cols: Vec<ResolvedCollection<'a>> = Vec::new();
    for (col_path, col) in collections {
        let Some((dto_path, _)) = dtos.iter().find(|(_, d)| d.id == col.dto) else {
            continue;
        };
        let leaf = col_path.rsplit('/').next().unwrap_or(col_path.as_str());
        let skipped: BTreeSet<usize> =
            skipped_rows.iter().filter_map(|(p, idx)| (p == col_path).then_some(*idx)).collect();
        let compile_len = compile_row_count(col, &skipped);
        resolved_cols.push(ResolvedCollection {
            col_path: col_path.as_str(),
            col,
            dto_pascal: to_pascal_case(dto_path),
            full_const: to_upper_snake_case(col_path),
            leaf_const: to_upper_snake_case(leaf),
            full_snake: col_path.replace('/', "_"),
            leaf_snake: leaf.to_string(),
            compile_len,
        });
    }
    resolved_cols
}

/// Emits the generated Rust module for a single `.collection.ron` (`crates/quillshade/src/data/collections/<col_path>.rs`).
pub fn emit_collection_file(
    col_path: &str,
    col: &CollectionAsset,
    dto_path: &str,
    skipped_rows: &BTreeSet<usize>,
) -> String {
    let source_rel = collection_ref(col_path).source_rel();
    let dto_pascal = to_pascal_case(dto_path);
    let full_const = to_upper_snake_case(col_path);
    let compile_len = compile_row_count(col, skipped_rows);
    let col_id = col.id.as_u128();
    let qsd_path = format!("quillshade/data/autogen/{col_path}.collection.qsd");

    let mut o = format!(
        "{}\n//! Generated Collection module for `{source_rel}`.\n\n#[allow(unused_imports)]\nuse super::{{CollectionUuid, dtos}};\n\n",
        marker::header("//", Some(&source_rel))
    );

    o.push_str(&format!("/// Row DTO type for `{col_path}` (`{source_rel}`).\npub type Row = dtos::{dto_pascal};\n\n"));
    o.push_str(&format!(
        "/// Collection `{col_path}` (`{source_rel}`).\npub const {full_const}: CollectionUuid<Row> = CollectionUuid::from_u128({col_id:#034x});\n\n"
    ));
    if full_const != "ID" {
        o.push_str(&format!(
            "/// Collection UUID handle for `{col_path}`.\npub const ID: CollectionUuid<Row> = {full_const};\n\n"
        ));
    }
    if full_const != "LEN" {
        o.push_str(&format!("/// Compile-time row count for `{col_path}`.\npub const LEN: usize = {compile_len};\n\n"));
    }
    if full_const != "BAKED_PATH" {
        o.push_str(&format!(
            "/// Baked `.collection.qsd` asset path for `{col_path}`.\npub const BAKED_PATH: &str = {qsd_path:?};\n"
        ));
    }

    o
}

/// Emits `crates/quillshade/src/data/collections/mod.rs` with no skipped rows.
pub fn emit_collections_mod(
    dtos: &BTreeMap<String, DtoAsset>,
    collections: &BTreeMap<String, CollectionAsset>,
) -> String {
    emit_collections_mod_with_skipped(dtos, collections, &BTreeSet::new())
}

/// Emits `crates/quillshade/src/data/collections/mod.rs`.
pub fn emit_collections_mod_with_skipped(
    dtos: &BTreeMap<String, DtoAsset>,
    collections: &BTreeMap<String, CollectionAsset>,
    skipped_rows: &BTreeSet<(String, usize)>,
) -> String {
    let resolved_cols = resolve_collections(dtos, collections, skipped_rows);
    let mut o = format!(
        "{}\n//! Generated Collection modules (`quillshade/data/**/*.collection.ron`).\n\n#[allow(unused_imports)]\npub(super) use super::{{CollectionUuid, dtos}};\n",
        marker::header("//", None)
    );

    if resolved_cols.is_empty() {
        return o;
    }

    let full_consts: BTreeSet<&str> = resolved_cols.iter().map(|c| c.full_const.as_str()).collect();
    let mut leaf_const_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for rc in &resolved_cols {
        *leaf_const_counts.entry(rc.leaf_const.as_str()).or_default() += 1;
    }

    let mut used_mod_names: BTreeSet<String> = BTreeSet::new();

    for rc in &resolved_cols {
        let mod_ident = collection_module_ident(rc.col_path);
        let col_path = rc.col_path;
        let dto_pascal = &rc.dto_pascal;
        let full_const = &rc.full_const;
        used_mod_names.insert(mod_ident.clone());

        o.push_str(&format!(
            "\n#[path = \"{col_path}.rs\"]\npub mod {mod_ident};\npub use {mod_ident}::{full_const};\n"
        ));

        if rc.col_path.contains('/')
            && rc.leaf_const != rc.full_const
            && leaf_const_counts.get(rc.leaf_const.as_str()) == Some(&1)
            && !full_consts.contains(rc.leaf_const.as_str())
        {
            let leaf_const = &rc.leaf_const;
            o.push_str(&format!(
                "/// Short alias for [`{full_const}`].\npub const {leaf_const}: CollectionUuid<dtos::{dto_pascal}> = {mod_ident}::{full_const};\n"
            ));
        }
    }

    // Build nested directory tree for collections inside subdirectories (e.g. `library/books_core`).
    let mut top_dirs: BTreeSet<&str> = BTreeSet::new();
    for rc in &resolved_cols {
        if let Some((first_seg, _)) = rc.col_path.split_once('/') {
            top_dirs.insert(first_seg);
        }
    }
    for dir_seg in top_dirs {
        if !is_rust_keyword(dir_seg) && used_mod_names.insert(dir_seg.to_string()) {
            emit_nested_collection_dir(&mut o, "", dir_seg, dir_seg, &resolved_cols);
        }
    }

    o
}

/// Emits `crates/quillshade/src/data/mod.rs` with no skipped rows.
pub fn emit_data_mod(
    dtos: &BTreeMap<String, DtoAsset>,
    collections: &BTreeMap<String, CollectionAsset>,
    schema_hash: u64,
) -> String {
    emit_data_mod_with_skipped(dtos, collections, schema_hash, &BTreeSet::new())
}

/// Emits `crates/quillshade/src/data/mod.rs`, excluding any `(collection_path, row_index)` pairs in `skipped_rows` from
/// compile-time row counts.
pub fn emit_data_mod_with_skipped(
    dtos: &BTreeMap<String, DtoAsset>,
    collections: &BTreeMap<String, CollectionAsset>,
    schema_hash: u64,
    skipped_rows: &BTreeSet<(String, usize)>,
) -> String {
    emit_data_mod_with_context(dtos, collections, schema_hash, skipped_rows, None)
}

/// Emits `crates/quillshade/src/data/mod.rs`, excluding any `(collection_path, row_index)` pairs in `skipped_rows` from
/// compile-time row counts and tailoring cross-domain runtime glue (such as `BakedMaterials` syncing) to `project`'s
/// enabled domains.
pub fn emit_data_mod_with_context(
    dtos: &BTreeMap<String, DtoAsset>,
    collections: &BTreeMap<String, CollectionAsset>,
    schema_hash: u64,
    skipped_rows: &BTreeSet<(String, usize)>,
    project: Option<&Project>,
) -> String {
    let empty_enums = BTreeMap::new();
    emit_data_mod_with_schemas_and_context(dtos, &empty_enums, collections, schema_hash, skipped_rows, project)
}

/// Emits `crates/quillshade/src/data/mod.rs` with `.dto.ron` and `.enum.ron` schemas, excluding any
/// `(collection_path, row_index)` pairs in `skipped_rows` from compile-time row counts.
pub fn emit_data_mod_with_schemas_and_context(
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    collections: &BTreeMap<String, CollectionAsset>,
    schema_hash: u64,
    skipped_rows: &BTreeSet<(String, usize)>,
    project: Option<&Project>,
) -> String {
    let resolved_cols = resolve_collections(dtos, collections, skipped_rows);
    let material_enabled = project.is_some_and(|p| p.domain_enabled(DomainId::new("material")));

    let mut o = format!(
        "{}\n//! Generated Data domain runtime module (`quillshade::data`).\n\n{DATA_MOD_TEMPLATE}\n",
        marker::header("//", None)
    );

    o.push_str(&format!(
        "/// Structural FNV-1a 64-bit hash over all valid `.dto.ron` and `.enum.ron` schemas.\npub const DATA_SCHEMA_HASH: u64 = {schema_hash:#018x};\n\n"
    ));

    o.push_str("/// Baked `.collection.qsd` assets `(collection_uuid, asset_path)` loaded at startup.\n");
    if resolved_cols.is_empty() {
        o.push_str("pub const BAKED_COLLECTIONS: &[(u128, &str)] = &[];\n\n");
    } else {
        o.push_str("pub const BAKED_COLLECTIONS: &[(u128, &str)] = &[\n");
        for rc in &resolved_cols {
            let col_id = rc.col.id.as_u128();
            let qsd_path = format!("quillshade/data/autogen/{}.collection.qsd", rc.col_path);
            o.push_str(&format!("    ({col_id:#034x}, {qsd_path:?}),\n"));
        }
        o.push_str("];\n\n");
    }

    o.push_str("pub mod dtos;\n");
    if !dtos.is_empty() {
        o.push_str("#[allow(unused_imports)]\npub use dtos::*;\n");
    }
    o.push('\n');

    o.push_str("pub mod enums;\n");
    if !enums.is_empty() {
        o.push_str("#[allow(unused_imports)]\npub use enums::*;\n");
    }
    o.push('\n');

    o.push_str(
        "/// Typed [`CollectionUuid`] handles and metadata for `.collection.ron` assets.\npub mod collections;\n\n",
    );
    emit_data_system_param(&mut o, &resolved_cols);
    emit_init_registered_collections(&mut o, &resolved_cols);
    emit_sync_registered_materials(&mut o, material_enabled);
    emit_decode_registered_collection(&mut o, &resolved_cols);

    o
}

fn emit_sync_registered_materials(o: &mut String, material_enabled: bool) {
    if material_enabled {
        o.push_str(
            "fn sync_registered_materials(\n    mut store: ResMut<DataStore>,\n    baked: Option<Res<crate::material::BakedMaterials>>,\n    textures: Option<Res<crate::material::BakedTextures>>,\n    blobs: Option<Res<Assets<QsdBlobAsset>>>,\n) {\n    let mut changed = false;\n    if let Some(baked) = baked\n        && store.material_by_id != baked.by_id\n    {\n        store.material_by_id = baked.by_id.clone();\n        changed = true;\n    }\n    if let Some(textures) = textures\n        && (bevy::ecs::change_detection::DetectChanges::is_changed(&textures)\n            || (store.texture_by_path.is_empty() && !textures.by_path.is_empty()))\n    {\n        let mut next = HashMap::with_capacity(textures.by_path.len() * 2);\n        for (k, h) in &textures.by_path {\n            next.insert(k.clone(), h.clone());\n            if let Some(ap) = h.path() {\n                next.insert(ap.path().to_string_lossy().into_owned(), h.clone());\n            }\n        }\n        if store.texture_by_path != next {\n            store.texture_by_path = next;\n            changed = true;\n        }\n    }\n    if changed\n        && !store.loaded_collections.is_empty()\n        && let Some(blobs) = blobs\n    {\n        let loaded: Vec<(u128, Vec<u8>)> = store\n            .loaded_collections\n            .iter()\n            .filter_map(|col_id| {\n                let (_, handle) = store.handles.get(col_id)?;\n                let blob = blobs.get(handle.id())?;\n                Some((*col_id, blob.bytes.clone()))\n            })\n            .collect();\n        for (col_id, bytes) in loaded {\n            let _ = store.apply_qsd_bytes_by_collection_id(col_id, &bytes);\n        }\n    }\n}\n\n",
        );
    } else {
        o.push_str("fn sync_registered_materials(_store: ResMut<DataStore>) {}\n\n");
    }
}

fn emit_data_system_param(o: &mut String, resolved_cols: &[ResolvedCollection<'_>]) {
    let full_snakes: BTreeSet<&str> = resolved_cols.iter().map(|c| c.full_snake.as_str()).collect();
    let mut leaf_snake_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for rc in resolved_cols {
        *leaf_snake_counts.entry(rc.leaf_snake.as_str()).or_default() += 1;
    }

    struct HandleField<'a> {
        field_ident: String,
        source_rel: String,
        dto_pascal: &'a str,
        full_const: &'a str,
        compile_len: usize,
        alias_of: Option<String>,
    }

    let mut fields: Vec<HandleField<'_>> = Vec::new();
    let mut used_fields: BTreeSet<String> = BTreeSet::new();

    for rc in resolved_cols {
        let source_rel = collection_ref(rc.col_path).source_rel();
        let primary_ident = format!("collection_{}", rc.full_snake);
        if used_fields.insert(primary_ident.clone()) {
            fields.push(HandleField {
                field_ident: primary_ident.clone(),
                source_rel: source_rel.clone(),
                dto_pascal: &rc.dto_pascal,
                full_const: &rc.full_const,
                compile_len: rc.compile_len,
                alias_of: None,
            });
        }

        if rc.col_path.contains('/')
            && rc.leaf_snake != rc.full_snake
            && leaf_snake_counts.get(rc.leaf_snake.as_str()) == Some(&1)
            && !full_snakes.contains(rc.leaf_snake.as_str())
        {
            let alias_ident = format!("collection_{}", rc.leaf_snake);
            if used_fields.insert(alias_ident.clone()) {
                fields.push(HandleField {
                    field_ident: alias_ident,
                    source_rel,
                    dto_pascal: &rc.dto_pascal,
                    full_const: &rc.full_const,
                    compile_len: rc.compile_len,
                    alias_of: Some(primary_ident),
                });
            }
        }
    }

    o.push_str(
        "/// Primary game-facing [`SystemParam`] for querying baked Data domain collections and rows (ADR 0049 §7).\n",
    );
    o.push_str("#[derive(Clone, Copy, Debug)]\n");
    o.push_str("pub struct Data<'w> {\n");
    o.push_str("    store: &'w DataStore,\n");
    for hf in &fields {
        let field_ident = &hf.field_ident;
        let dto_pascal = hf.dto_pascal;
        let source_rel = &hf.source_rel;
        if let Some(primary) = &hf.alias_of {
            o.push_str(&format!(
                "    /// Short alias for [`Self::{primary}`] (`{source_rel}`).\n    pub {field_ident}: CollectionHandle<'w, dtos::{dto_pascal}>,\n"
            ));
        } else {
            o.push_str(&format!(
                "    /// Statically typed handle for `{source_rel}`.\n    pub {field_ident}: CollectionHandle<'w, dtos::{dto_pascal}>,\n"
            ));
        }
    }
    o.push_str("}\n\n");

    o.push_str("impl<'w> Data<'w> {\n");
    o.push_str("    #[inline]\n");
    o.push_str("    pub(crate) fn from_store(store: &'w DataStore) -> Self {\n");
    for hf in fields.iter().filter(|f| f.alias_of.is_none()) {
        let field_ident = &hf.field_ident;
        let full_const = hf.full_const;
        let compile_len = hf.compile_len;
        o.push_str(&format!(
            "        let {field_ident} = CollectionHandle::new(store.collection(collections::{full_const}), {compile_len});\n"
        ));
    }
    if fields.is_empty() {
        o.push_str("        Self { store }\n");
    } else {
        o.push_str("        Self {\n            store,\n");
        for hf in &fields {
            let field_ident = &hf.field_ident;
            if let Some(primary) = &hf.alias_of {
                o.push_str(&format!("            {field_ident}: {primary},\n"));
            } else {
                o.push_str(&format!("            {field_ident},\n"));
            }
        }
        o.push_str("        }\n");
    }
    o.push_str("    }\n");

    for hf in &fields {
        let field_ident = &hf.field_ident;
        let dto_pascal = hf.dto_pascal;
        let source_rel = &hf.source_rel;
        o.push_str(&format!(
            "\n    /// Returns the [`CollectionHandle`] for `{source_rel}`.\n    #[inline]\n    pub const fn {field_ident}(&self) -> CollectionHandle<'w, dtos::{dto_pascal}> {{\n        self.{field_ident}\n    }}\n"
        ));
    }

    o.push_str("}\n\n");
}

fn emit_init_registered_collections(o: &mut String, resolved_cols: &[ResolvedCollection<'_>]) {
    if resolved_cols.is_empty() {
        o.push_str("fn init_registered_collections(_store: &mut DataStore) {}\n\n");
        return;
    }
    o.push_str("fn init_registered_collections(store: &mut DataStore) {\n");
    for rc in resolved_cols {
        let dto_pascal = &rc.dto_pascal;
        let full_const = &rc.full_const;
        let compile_len = rc.compile_len;
        o.push_str(&format!(
            "    store.init_collection::<dtos::{dto_pascal}>(collections::{full_const}, {compile_len});\n"
        ));
    }
    o.push_str("}\n\n");
}

fn emit_nested_collection_dir(
    o: &mut String,
    indent: &str,
    dir_prefix: &str,
    mod_name: &str,
    resolved_cols: &[ResolvedCollection<'_>],
) {
    o.push_str(&format!("\n{indent}pub mod {mod_name} {{\n"));
    o.push_str(&format!("{indent}    #[allow(unused_imports)]\n{indent}    use super::{{CollectionUuid, dtos}};\n"));

    let child_indent = format!("{indent}    ");
    let mut used_names: BTreeSet<String> = BTreeSet::new();
    let mut child_dirs: BTreeSet<&str> = BTreeSet::new();

    for rc in resolved_cols {
        let Some(rest) = rc.col_path.strip_prefix(dir_prefix).and_then(|s| s.strip_prefix('/')) else {
            continue;
        };
        if let Some((next_dir, _)) = rest.split_once('/') {
            child_dirs.insert(next_dir);
        } else {
            let source_rel = collection_ref(rc.col_path).source_rel();
            let col_id = rc.col.id.as_u128();
            let dto_pascal = &rc.dto_pascal;
            let leaf_const = &rc.leaf_const;
            o.push_str(&format!(
                "\n{child_indent}/// Collection `{}` (`{source_rel}`).\n{child_indent}pub const {leaf_const}: CollectionUuid<dtos::{dto_pascal}> = CollectionUuid::from_u128({col_id:#034x});\n",
                rc.col_path
            ));
        }
    }

    for next_dir in child_dirs {
        if !is_rust_keyword(next_dir) && used_names.insert(next_dir.to_string()) {
            let next_prefix = format!("{dir_prefix}/{next_dir}");
            emit_nested_collection_dir(o, &child_indent, &next_prefix, next_dir, resolved_cols);
        }
    }

    o.push_str(&format!("{indent}}}\n"));
}

fn emit_decode_registered_collection(o: &mut String, resolved_cols: &[ResolvedCollection<'_>]) {
    if resolved_cols.is_empty() {
        o.push_str(
            "fn decode_registered_collection(\n    _store: &mut DataStore,\n    _collection_id: u128,\n    _bytes: &[u8],\n) -> Result<bool, QsdRuntimeError> {\n    Ok(false)\n}\n",
        );
        return;
    }

    o.push_str("#[allow(clippy::match_single_binding)]\n");
    o.push_str(
        "fn decode_registered_collection(\n    store: &mut DataStore,\n    collection_id: u128,\n    bytes: &[u8],\n) -> Result<bool, QsdRuntimeError> {\n    match collection_id {\n",
    );
    for rc in resolved_cols {
        let col_id = rc.col.id.as_u128();
        let dto_pascal = &rc.dto_pascal;
        let full_const = &rc.full_const;
        let qsd_path = format!("quillshade/data/autogen/{}.collection.qsd", rc.col_path);
        o.push_str(&format!(
            "        {col_id:#034x} => {{\n            store.apply_collection_bytes::<dtos::{dto_pascal}>(\n                collections::{full_const},\n                {qsd_path:?},\n                bytes,\n            )?;\n            Ok(true)\n        }}\n"
        ));
    }
    o.push_str("        _ => Ok(false),\n    }\n}\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::model::{DTO_FORMAT_VERSION, FieldSettings};

    fn test_uuid(seed: u128) -> AssetUuid {
        let raw = (0x0195_0000_0000_7000_8000_0000_0000_0000_u128) | (seed & 0xffff_ffff_ffff);
        AssetUuid::from_u128(raw)
    }

    #[test]
    fn codegen_soft_ref_emits_typed_field_wire_u128_and_defaults() {
        let book_id = test_uuid(1);
        let row_id = test_uuid(10);
        let book_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: book_id,
            doc: String::new(),
            fields: vec![
                Field {
                    id: test_uuid(2),
                    name: "title".into(),
                    ty: FieldType::String,
                    category: String::new(),
                    is_name: true,
                    default: DataValue::String("The Shader Alchemist".into()),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: test_uuid(3),
                    name: "sequel".into(),
                    ty: FieldType::SoftRef(book_id),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::SoftRef(AssetUuid::NIL),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: test_uuid(4),
                    name: "prequel".into(),
                    ty: FieldType::SoftRef(book_id),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::SoftRef(row_id),
                    settings: FieldSettings::default(),
                },
            ],
        };

        let dtos = BTreeMap::from([("library/book".to_string(), book_dto.clone())]);
        let dto_code = emit_dto_file_with_dtos("library/book", &book_dto, &dtos);

        assert!(dto_code.contains("pub sequel: super::SoftRef<super::LibraryBook>,"), "{dto_code}");
        assert!(dto_code.contains("pub prequel: super::SoftRef<super::LibraryBook>,"), "{dto_code}");
        assert!(dto_code.contains("pub struct LibraryBookWire(pub String, pub u128, pub u128);"), "{dto_code}");
        assert!(dto_code.contains("sequel: super::SoftRef::from_raw(0u128),"), "{dto_code}");
        assert!(
            dto_code.contains(&format!("prequel: super::SoftRef::from_raw({:#034x}u128),", row_id.as_u128())),
            "{dto_code}"
        );
        assert!(dto_code.contains("sequel: super::SoftRef::from_raw(wire.1),"), "{dto_code}");
        assert!(dto_code.contains("prequel: super::SoftRef::from_raw(wire.2),"), "{dto_code}");

        let dtos_mod = emit_dtos_mod(&dtos);
        assert!(dtos_mod.contains("SoftRef"), "{dtos_mod}");
    }

    #[test]
    fn codegen_unit_enum_and_tagged_union_emits_derives_wire_and_defaults() {
        use crate::backend::model::{ENUM_FORMAT_VERSION, EnumVariant};

        let rarity_id = test_uuid(100);
        let var_common = test_uuid(101);
        let var_rare = test_uuid(102);

        let effect_id = test_uuid(110);
        let var_none = test_uuid(111);
        let var_burn = test_uuid(112);
        let f_dps = test_uuid(113);
        let f_dur = test_uuid(114);

        let rarity_enum = EnumAsset {
            format_version: ENUM_FORMAT_VERSION,
            id: rarity_id,
            doc: "Item rarity".into(),
            variants: vec![
                EnumVariant { id: var_common, name: "common".into(), doc: String::new(), fields: Vec::new() },
                EnumVariant { id: var_rare, name: "rare".into(), doc: String::new(), fields: Vec::new() },
            ],
        };

        let effect_enum = EnumAsset {
            format_version: ENUM_FORMAT_VERSION,
            id: effect_id,
            doc: "Status effect".into(),
            variants: vec![
                EnumVariant { id: var_none, name: "none".into(), doc: String::new(), fields: Vec::new() },
                EnumVariant {
                    id: var_burn,
                    name: "burn".into(),
                    doc: String::new(),
                    fields: vec![
                        Field {
                            id: f_dps,
                            name: "dps".into(),
                            ty: FieldType::F32,
                            category: String::new(),
                            is_name: true,
                            default: DataValue::F32(15.0),
                            settings: FieldSettings::default(),
                        },
                        Field {
                            id: f_dur,
                            name: "duration".into(),
                            ty: FieldType::Duration,
                            category: String::new(),
                            is_name: false,
                            default: DataValue::Duration(3.0),
                            settings: FieldSettings::default(),
                        },
                    ],
                },
            ],
        };

        let enums = BTreeMap::from([
            ("combat/rarity".to_string(), rarity_enum.clone()),
            ("combat/effect".to_string(), effect_enum.clone()),
        ]);
        let dtos = BTreeMap::new();

        let rarity_code = emit_enum_file_with_schemas("combat/rarity", &rarity_enum, &dtos, &enums);
        assert!(
            rarity_code.contains("#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]"),
            "{rarity_code}"
        );
        assert!(rarity_code.contains("pub enum CombatRarity {\n    Common,\n    Rare,\n}"), "{rarity_code}");
        assert!(rarity_code.contains("pub enum CombatRarityWire {\n    Common,\n    Rare,\n}"), "{rarity_code}");
        assert!(rarity_code.contains("Self::Common"), "{rarity_code}");

        let effect_code = emit_enum_file_with_schemas("combat/effect", &effect_enum, &dtos, &enums);
        assert!(effect_code.contains("#[derive(Clone, Debug, PartialEq)]\npub enum CombatEffect {"), "{effect_code}");
        assert!(
            effect_code.contains("    Burn {\n        dps: f32,\n        duration: std::time::Duration,\n    },"),
            "{effect_code}"
        );
        assert!(
            effect_code.contains("pub enum CombatEffectWire {\n    None,\n    Burn(f32, f32),\n}"),
            "{effect_code}"
        );
        assert!(effect_code.contains("CombatEffectWire::Burn(f0, f1) => Self::Burn {"), "{effect_code}");

        let enums_mod = emit_enums_mod(&enums);
        assert!(enums_mod.contains("pub type Rarity = CombatRarity;"), "{enums_mod}");
        assert!(enums_mod.contains("pub type Effect = CombatEffect;"), "{enums_mod}");
    }

    #[test]
    fn codegen_option_recursive_box_and_containers_emits_types_wire_and_defaults() {
        use crate::backend::model::{ENUM_FORMAT_VERSION, EnumVariant};

        let rarity_id = test_uuid(200);
        let var_common = test_uuid(201);
        let var_rare = test_uuid(202);
        let rarity_enum = EnumAsset {
            format_version: ENUM_FORMAT_VERSION,
            id: rarity_id,
            doc: String::new(),
            variants: vec![
                EnumVariant { id: var_common, name: "common".into(), doc: String::new(), fields: Vec::new() },
                EnumVariant { id: var_rare, name: "rare".into(), doc: String::new(), fields: Vec::new() },
            ],
        };

        let node_id = test_uuid(210);
        let f_label = test_uuid(211);
        let f_parent = test_uuid(212);
        let f_opt_vec = test_uuid(213);
        let f_weights = test_uuid(214);
        let f_rarities = test_uuid(215);
        let f_drop_table = test_uuid(216);

        let node_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: node_id,
            doc: "Recursive tree node with container fields.".into(),
            fields: vec![
                Field {
                    id: f_label,
                    name: "label".into(),
                    ty: FieldType::String,
                    category: String::new(),
                    is_name: true,
                    default: DataValue::String("root".into()),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_parent,
                    name: "next_node".into(),
                    ty: FieldType::Option(Box::new(FieldType::Dto(node_id))),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Option(None),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_opt_vec,
                    name: "spawn_offset".into(),
                    ty: FieldType::Option(Box::new(FieldType::Vec3)),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Option(Some(Box::new(DataValue::Vec3([1.0, 2.0, 3.0])))),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_weights,
                    name: "weights".into(),
                    ty: FieldType::Array(Box::new(FieldType::F32)),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Array(vec![DataValue::F32(0.5), DataValue::F32(1.5)]),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_rarities,
                    name: "allowed_rarities".into(),
                    ty: FieldType::Set(Box::new(FieldType::Enum(rarity_id))),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Set(vec![DataValue::Enum { variant: var_rare, fields: BTreeMap::new() }]),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_drop_table,
                    name: "drop_table".into(),
                    ty: FieldType::Map(Box::new(FieldType::Enum(rarity_id)), Box::new(FieldType::Vec2)),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Map(vec![(
                        DataValue::Enum { variant: var_common, fields: BTreeMap::new() },
                        DataValue::Vec2([1.0, 5.0]),
                    )]),
                    settings: FieldSettings::default(),
                },
            ],
        };

        let dtos = BTreeMap::from([("loot/node".to_string(), node_dto.clone())]);
        let enums = BTreeMap::from([("loot/rarity".to_string(), rarity_enum)]);

        assert!(option_needs_box(&FieldType::Dto(node_id), &dtos, &enums));
        assert!(!option_needs_box(&FieldType::Vec3, &dtos, &enums));

        let code = emit_dto_file_with_schemas("loot/node", &node_dto, &dtos, &enums);
        assert!(code.contains("pub next_node: Option<Box<super::LootNode>>,"), "{code}");
        assert!(code.contains("pub spawn_offset: Option<bevy::math::Vec3>,"), "{code}");
        assert!(code.contains("pub weights: Vec<f32>,"), "{code}");
        assert!(code.contains("pub allowed_rarities: std::collections::BTreeSet<super::LootRarity>,"), "{code}");
        assert!(
            code.contains("pub drop_table: std::collections::BTreeMap<super::LootRarity, bevy::math::Vec2>,"),
            "{code}"
        );
        assert!(
            code.contains(
                "pub struct LootNodeWire(pub String, pub Option<Box<super::LootNodeWire>>, pub Option<[f32; 3]>, pub Vec<f32>, pub std::collections::BTreeSet<super::LootRarityWire>, pub std::collections::BTreeMap<super::LootRarityWire, [f32; 2]>);"
            ),
            "{code}"
        );
        assert!(code.contains("next_node: wire.1.map(|v| Box::new(super::LootNode::from_wire(*v, ctx))),"), "{code}");
        assert!(code.contains("spawn_offset: wire.2.map(bevy::math::Vec3::from_array),"), "{code}");
        assert!(code.contains("weights: wire.3,"), "{code}");
        assert!(
            code.contains(
                "allowed_rarities: wire.4.into_iter().map(|k| super::LootRarity::from_wire(k, ctx)).collect(),"
            ),
            "{code}"
        );
        assert!(
            code.contains(
                "drop_table: wire.5.into_iter().map(|(k, v)| (super::LootRarity::from_wire(k, ctx), bevy::math::Vec2::from_array(v))).collect(),"
            ),
            "{code}"
        );
    }
}
