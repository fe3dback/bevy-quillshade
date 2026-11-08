//! Deterministic RON loading and saving for `.dto.ron`, `.enum.ron`, and `.collection.ron` assets, including D27
//! trailing reference comments on collection `dto`, enum references, variant UUIDs, and field UUID keys
//! (ADR 0047 §5, ADR 0049 §4–§5).

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use project_core::asset_index::RefNames;
use project_core::is_uuid_shaped;
use project_core::{AssetIndex, AssetRef, AssetUuid, DomainId, LoadError, Project, check_version, probe_version};

use super::DATA;
use super::model::{
    COLLECTION_FORMAT_VERSION, COLLECTION_SUFFIX, CURVE_SUFFIX, CollectionAsset, DTO_FORMAT_VERSION, DTO_SUFFIX,
    DtoAsset, ENUM_FORMAT_VERSION, ENUM_SUFFIX, EnumAsset,
};

/// Reads and parses a `.curve.ron` file from disk.
pub fn load_curve(path: &Path) -> Result<curve_kit::Curve, LoadError> {
    let text = std::fs::read_to_string(path).map_err(|e| LoadError::Parse(e.to_string()))?;
    curve_kit::Curve::from_ron(&text)
}

/// Ensures `curve.id` is minted and writes `curve` atomically to `path`.
pub fn save_curve(path: &Path, curve: &mut curve_kit::Curve) -> io::Result<()> {
    curve.ensure_id();
    project_core::writer::write_atomic(path, curve.to_ron())
}

/// Parses a `.dto.ron` string of the current format ([`DTO_FORMAT_VERSION`]).
pub fn parse_dto(text: &str) -> Result<DtoAsset, LoadError> {
    check_version(probe_version(text)?, DTO_FORMAT_VERSION)?;
    ron::Options::default().from_str(text).map_err(|e| LoadError::Parse(e.to_string()))
}

/// Reads and parses a `.dto.ron` file from disk.
pub fn load_dto(path: &Path) -> Result<DtoAsset, LoadError> {
    let text = std::fs::read_to_string(path).map_err(|e| LoadError::Parse(e.to_string()))?;
    parse_dto(&text)
}

/// Parses an `.enum.ron` string of the current format ([`ENUM_FORMAT_VERSION`]).
pub fn parse_enum(text: &str) -> Result<EnumAsset, LoadError> {
    check_version(probe_version(text)?, ENUM_FORMAT_VERSION)?;
    ron::Options::default().from_str(text).map_err(|e| LoadError::Parse(e.to_string()))
}

/// Reads and parses an `.enum.ron` file from disk.
pub fn load_enum(path: &Path) -> Result<EnumAsset, LoadError> {
    let text = std::fs::read_to_string(path).map_err(|e| LoadError::Parse(e.to_string()))?;
    parse_enum(&text)
}

/// Builds the reference name table for all DTOs, Enums, variants, and fields in `dtos` and `enums`.
fn build_schema_ref_names(
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> BTreeMap<AssetUuid, (DomainId, String)> {
    let mut names: BTreeMap<AssetUuid, (DomainId, String)> = BTreeMap::new();
    for (dto_path, d) in dtos {
        if !d.id.is_nil() {
            names.insert(d.id, (DATA, dto_path.clone()));
        }
        for field in &d.fields {
            if !field.id.is_nil() {
                names.insert(field.id, (DATA, field.name.clone()));
            }
        }
    }
    for (enum_path, e) in enums {
        if !e.id.is_nil() {
            names.insert(e.id, (DATA, enum_path.clone()));
        }
        for variant in &e.variants {
            if !variant.id.is_nil() {
                names.insert(variant.id, (DATA, format!("{enum_path}::{}", variant.name)));
            }
            for field in &variant.fields {
                if !field.id.is_nil() {
                    names.insert(field.id, (DATA, field.name.clone()));
                }
            }
        }
    }
    names
}

fn comment_start(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    let (mut in_str, mut i) = (false, 0);
    while i < b.len() {
        match b[i] {
            b'\\' if in_str => i += 1,
            b'"' => in_str = !in_str,
            b'/' if !in_str && b.get(i + 1) == Some(&b'/') => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

fn is_id_line(line: &str) -> bool {
    line.trim_start()
        .strip_prefix("id:")
        .map(str::trim_start)
        .and_then(|v| v.strip_prefix('"'))
        .is_some_and(|v| v.get(..36).is_some_and(is_uuid_shaped) && v[36..].starts_with('"'))
}

fn extract_gradient_slot(code: &str, uuid_str: &str) -> Option<u32> {
    let needle = format!("Gradient(\"{uuid_str}\",");
    let rest = code.split(&needle).nth(1)?.trim_start();
    let digits_end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    rest.get(..digits_end)?.parse::<u32>().ok()
}

fn non_nil_uuid_tokens(code: &str) -> Vec<&str> {
    let b = code.as_bytes();
    let (mut out, mut in_str, mut str_start, mut i) = (Vec::new(), false, 0, 0);
    while i < b.len() {
        match b[i] {
            b'\\' if in_str => i += 1,
            b'"' if !in_str => {
                in_str = true;
                str_start = i + 1;
            }
            b'"' => {
                in_str = false;
                let s = &code[str_start..i];
                if s.len() == 36 && is_uuid_shaped(s) && s != "00000000-0000-0000-0000-000000000000" {
                    out.push(s);
                } else if let Some(u) =
                    s.strip_suffix(|c: char| c != ':').and_then(|_| s.rsplit_once("::")).map(|(_, u)| u)
                    && u.len() == 36
                    && is_uuid_shaped(u)
                    && u != "00000000-0000-0000-0000-000000000000"
                {
                    out.push(u);
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

fn resolve_ref_label(
    token: &str,
    gradient_slot: Option<u32>,
    names: &BTreeMap<AssetUuid, (DomainId, String)>,
    index: Option<&AssetIndex>,
) -> String {
    let Some(id) = AssetUuid::parse(token) else {
        return "(missing)".to_string();
    };
    if let Some((d, path)) = names.get(&id) {
        return if *d == DATA { path.clone() } else { format!("{d}:{path}") };
    }
    if let Some(ix) = index {
        if let Some(entry) = ix.get(id) {
            let base = if entry.asset.domain == DATA {
                entry.asset.path.clone()
            } else {
                format!("{}:{}", entry.asset.domain, entry.asset.path)
            };
            if let Some(slot) = gradient_slot
                && slot != 0
                && let Some(sub) = entry.sub_items.iter().find(|s| s.id == u64::from(slot))
                && !sub.label.is_empty()
            {
                return format!("{base}.{}", sub.label);
            }
            return base;
        }
        for entry in ix.assets_of(DATA, ENUM_SUFFIX) {
            for sub in &entry.sub_items {
                if sub.uuid == Some(id) {
                    if let Some((_, field_name)) = sub.label.split_once('.') {
                        return field_name.to_string();
                    }
                    return format!("{}::{}", entry.asset.path, sub.label);
                }
            }
        }
        for entry in ix.assets_of(DATA, DTO_SUFFIX) {
            for sub in &entry.sub_items {
                if sub.uuid == Some(id) {
                    return sub.label.clone();
                }
            }
        }
        if let Some((d, path)) = ix.name_of(id) {
            if d == DATA {
                if let Some((col_path, row_label)) = path.split_once("::") {
                    return format!("{col_path}#{row_label}");
                }
                return path.to_string();
            }
            return format!("{d}:{path}");
        }
    }
    "(missing)".to_string()
}

fn annotate_data_ron(
    text: &str,
    names: &BTreeMap<AssetUuid, (DomainId, String)>,
    index: Option<&AssetIndex>,
) -> String {
    let mut out = String::with_capacity(text.len() + 64);
    for line in text.split_inclusive('\n') {
        let (body, eol) = match line.strip_suffix('\n') {
            Some(b) => b.strip_suffix('\r').map_or((b, "\n"), |b| (b, "\r\n")),
            None => (line, ""),
        };
        let code = &body[..comment_start(body).unwrap_or(body.len())];
        let tokens = non_nil_uuid_tokens(code);
        if is_id_line(code) || tokens.is_empty() {
            out.push_str(line);
            continue;
        }
        let labels: Vec<String> = tokens
            .iter()
            .map(|t| {
                let slot = extract_gradient_slot(code, t);
                resolve_ref_label(t, slot, names, index)
            })
            .collect();
        out.push_str(code.trim_end());
        out.push_str(" // ");
        out.push_str(&labels.join(", "));
        out.push_str(eol);
    }
    out
}

/// Serializes `dto` to canonical RON text (struct names, 2-space indent, `\n` line endings, trailing newline).
pub fn dto_text(dto: &DtoAsset) -> String {
    let empty = BTreeMap::new();
    dto_text_with_dtos(dto, &empty)
}

/// Serializes `dto` to canonical RON text and annotates `FieldType::Dto("<uuid>")` with `// <dto_path>` and any nested
/// `DataValue::Dto` field keys `"<field_uuid>": <val>` with `// <field_name>`, while keeping `dto.id` and `Field.id`
/// lines uncommented (D27, ADR 0049 §3).
pub fn dto_text_with_dtos(dto: &DtoAsset, dtos: &BTreeMap<String, DtoAsset>) -> String {
    dto_text_with_index(dto, dtos, None)
}

/// Like [`dto_text_with_dtos`], also resolving `FieldType::Enum` and `DataValue::Enum` against `enums`.
pub fn dto_text_with_schemas(
    dto: &DtoAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> String {
    dto_text_with_schemas_and_index(dto, dtos, enums, None)
}

/// Like [`dto_text_with_dtos`], additionally resolving cross-domain `Curve`, `Gradient` (with slot label), `Material`,
/// `Texture`, `LightEnvironment`, and `Enum` default values via `index` when provided.
pub fn dto_text_with_index(dto: &DtoAsset, dtos: &BTreeMap<String, DtoAsset>, index: Option<&AssetIndex>) -> String {
    dto_text_with_schemas_and_index(dto, dtos, &BTreeMap::new(), index)
}

/// Like [`dto_text_with_index`], also accepting an explicit `enums` map.
pub fn dto_text_with_schemas_and_index(
    dto: &DtoAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    index: Option<&AssetIndex>,
) -> String {
    let mut sorted = dto.clone();
    sorted.sort_container_keys(dtos, enums);
    let cfg = ron::ser::PrettyConfig::new().struct_names(true).indentor("  ".to_string()).new_line("\n".to_string());
    let mut raw = ron::ser::to_string_pretty(&sorted, cfg).expect("dto serialization");
    raw.push('\n');

    let mut names = build_schema_ref_names(dtos, enums);
    for field in &dto.fields {
        if !field.id.is_nil() {
            names.insert(field.id, (DATA, field.name.clone()));
        }
    }
    annotate_data_ron(&raw, &names, index)
}

/// Ensures all UUIDs (`dto.id` and every `field.id`) are minted and writes `dto` atomically to `path`.
pub fn save_dto(path: &Path, dto: &mut DtoAsset) -> io::Result<()> {
    let empty = BTreeMap::new();
    save_dto_with_dtos(path, dto, &empty)
}

/// Ensures all UUIDs (`dto.id` and every `field.id`) are minted and writes `dto` atomically to `path` with D27 trailing
/// comments for nested DTO references.
pub fn save_dto_with_dtos(path: &Path, dto: &mut DtoAsset, dtos: &BTreeMap<String, DtoAsset>) -> io::Result<()> {
    save_dto_with_index(path, dto, dtos, None)
}

/// Ensures all UUIDs (`dto.id` and every `field.id`) are minted and writes `dto` atomically to `path` with D27 trailing
/// comments for nested DTO and Enum references.
pub fn save_dto_with_schemas(
    path: &Path,
    dto: &mut DtoAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> io::Result<()> {
    save_dto_with_schemas_and_index(path, dto, dtos, enums, None)
}

/// Ensures all UUIDs (`dto.id` and every `field.id`) are minted and writes `dto` atomically to `path` with D27 trailing
/// comments for both nested DTOs and cross-domain asset references.
pub fn save_dto_with_index(
    path: &Path,
    dto: &mut DtoAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    index: Option<&AssetIndex>,
) -> io::Result<()> {
    let empty_enums = BTreeMap::new();
    save_dto_with_schemas_and_index(path, dto, dtos, &empty_enums, index)
}

/// Ensures all UUIDs (`dto.id` and every `field.id`) are minted and writes `dto` atomically to `path` with D27 trailing
/// comments for nested DTOs, Enums, and cross-domain asset references.
pub fn save_dto_with_schemas_and_index(
    path: &Path,
    dto: &mut DtoAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    index: Option<&AssetIndex>,
) -> io::Result<()> {
    dto.ensure_ids();
    dto.sort_container_keys(dtos, enums);
    project_core::writer::write_atomic(path, dto_text_with_schemas_and_index(dto, dtos, enums, index))
}

/// Serializes `en` to canonical RON text (struct names, 2-space indent, `\n` line endings, trailing newline).
pub fn enum_text(en: &EnumAsset) -> String {
    let empty_dtos = BTreeMap::new();
    let empty_enums = BTreeMap::new();
    enum_text_with_schemas(en, &empty_dtos, &empty_enums)
}

/// Serializes `en` to canonical RON text and annotates references inside variant payload fields (`FieldType::Dto`,
/// `FieldType::Enum`, `FieldType::SoftRef`, nested `DataValue::Dto` / `DataValue::Enum` keys) while keeping `en.id`,
/// `EnumVariant.id`, and `Field.id` definition lines uncommented (D27, ADR 0049 §5).
pub fn enum_text_with_schemas(
    en: &EnumAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> String {
    enum_text_with_index(en, dtos, enums, None)
}

/// Like [`enum_text_with_schemas`], additionally resolving cross-domain asset references via `index` when provided.
pub fn enum_text_with_index(
    en: &EnumAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    index: Option<&AssetIndex>,
) -> String {
    let mut sorted = en.clone();
    sorted.sort_container_keys(dtos, enums);
    let cfg = ron::ser::PrettyConfig::new().struct_names(true).indentor("  ".to_string()).new_line("\n".to_string());
    let mut raw = ron::ser::to_string_pretty(&sorted, cfg).expect("enum serialization");
    raw.push('\n');

    let mut names = build_schema_ref_names(dtos, enums);
    for variant in &en.variants {
        for field in &variant.fields {
            if !field.id.is_nil() {
                names.insert(field.id, (DATA, field.name.clone()));
            }
        }
    }
    annotate_data_ron(&raw, &names, index)
}

/// Ensures all UUIDs (`en.id`, every `variant.id`, and every variant `field.id`) are minted and writes `en` atomically to `path`.
pub fn save_enum(path: &Path, en: &mut EnumAsset) -> io::Result<()> {
    let empty_dtos = BTreeMap::new();
    let empty_enums = BTreeMap::new();
    save_enum_with_schemas(path, en, &empty_dtos, &empty_enums)
}

/// Ensures all UUIDs (`en.id`, every `variant.id`, and every variant `field.id`) are minted and writes `en` atomically
/// to `path` with D27 trailing comments resolved across `dtos` and `enums`.
pub fn save_enum_with_schemas(
    path: &Path,
    en: &mut EnumAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> io::Result<()> {
    save_enum_with_index(path, en, dtos, enums, None)
}

/// Ensures all UUIDs (`en.id`, every `variant.id`, and every variant `field.id`) are minted and writes `en` atomically
/// to `path` with D27 trailing comments resolved across `dtos`, `enums`, and `index`.
pub fn save_enum_with_index(
    path: &Path,
    en: &mut EnumAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    index: Option<&AssetIndex>,
) -> io::Result<()> {
    en.ensure_ids();
    en.sort_container_keys(dtos, enums);
    project_core::writer::write_atomic(path, enum_text_with_index(en, dtos, enums, index))
}

/// Parses a `.collection.ron` string of the current format ([`COLLECTION_FORMAT_VERSION`]).
pub fn parse_collection(text: &str) -> Result<CollectionAsset, LoadError> {
    check_version(probe_version(text)?, COLLECTION_FORMAT_VERSION)?;
    ron::Options::default().from_str(text).map_err(|e| LoadError::Parse(e.to_string()))
}

/// Reads and parses a `.collection.ron` file from disk.
pub fn load_collection(path: &Path) -> Result<CollectionAsset, LoadError> {
    let text = std::fs::read_to_string(path).map_err(|e| LoadError::Parse(e.to_string()))?;
    parse_collection(&text)
}

/// Serializes `col` to canonical RON text and annotates `dto: "<uuid>"` with `// <dto_path>` and every row field key
/// `"<field_uuid>": <val>` with `// <field_name>`, while keeping `col.id` and `Row.id` lines uncommented (ADR 0049 §4).
pub fn collection_text(col: &CollectionAsset, dto_path: Option<&str>, dto: Option<&DtoAsset>) -> String {
    let empty_dtos = BTreeMap::new();
    let empty_enums = BTreeMap::new();
    collection_text_inner(col, None, dto_path, dto, &empty_dtos, &empty_enums, None)
}

/// Like [`collection_text`], resolving both top-level and nested `DataValue::Dto` field UUID keys across all project
/// `dtos` for D27 trailing comments.
pub fn collection_text_with_dtos(
    col: &CollectionAsset,
    dto: Option<&DtoAsset>,
    dtos: &BTreeMap<String, DtoAsset>,
) -> String {
    let empty_enums = BTreeMap::new();
    collection_text_inner(col, None, None, dto, dtos, &empty_enums, None)
}

/// Like [`collection_text_with_dtos`], also resolving `DataValue::Enum` variant and field UUIDs across `enums`.
pub fn collection_text_with_schemas(
    col: &CollectionAsset,
    dto: Option<&DtoAsset>,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> String {
    collection_text_inner(col, None, None, dto, dtos, enums, None)
}

/// Like [`collection_text_with_dtos`], additionally resolving cross-domain `Curve`, `Gradient` (with slot label),
/// `Material`, `Texture`, `LightEnvironment`, `Enum`, and `SoftRef` cell values via `index` when provided.
pub fn collection_text_with_index(
    col: &CollectionAsset,
    dto: Option<&DtoAsset>,
    dtos: &BTreeMap<String, DtoAsset>,
    index: Option<&AssetIndex>,
) -> String {
    let empty_enums = BTreeMap::new();
    collection_text_inner(col, None, None, dto, dtos, &empty_enums, index)
}

/// Like [`collection_text_with_index`], also accepting an explicit `enums` map.
pub fn collection_text_with_schemas_and_index(
    col: &CollectionAsset,
    dto: Option<&DtoAsset>,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    index: Option<&AssetIndex>,
) -> String {
    collection_text_inner(col, None, None, dto, dtos, enums, index)
}

fn infer_collection_rel_path(path: &Path) -> Option<String> {
    let s = path.to_string_lossy().replace('\\', "/");
    let without_suffix = s.strip_suffix(".collection.ron")?;
    if let Some((_, rel)) = without_suffix.rsplit_once("/quillshade/data/") {
        return Some(rel.to_string());
    }
    if let Some(rel) = without_suffix.strip_prefix("quillshade/data/") {
        return Some(rel.to_string());
    }
    Path::new(without_suffix).file_name().and_then(|n| n.to_str()).map(str::to_string)
}

#[allow(clippy::too_many_arguments)]
fn collection_text_inner(
    col: &CollectionAsset,
    col_path_hint: Option<&str>,
    dto_path: Option<&str>,
    dto: Option<&DtoAsset>,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    index: Option<&AssetIndex>,
) -> String {
    let mut sorted = col.clone();
    sorted.sort_container_keys(dto, dtos, enums);
    let cfg = ron::ser::PrettyConfig::new().struct_names(true).indentor("  ".to_string()).new_line("\n".to_string());
    let mut raw = ron::ser::to_string_pretty(&sorted, cfg).expect("collection serialization");
    raw.push('\n');

    let mut names = build_schema_ref_names(dtos, enums);
    if !col.dto.is_nil()
        && let Some(path) = dto_path
    {
        names.insert(col.dto, (DATA, path.to_string()));
    }
    if let Some(dto_asset) = dto {
        for field in &dto_asset.fields {
            if !field.id.is_nil() {
                names.insert(field.id, (DATA, field.name.clone()));
            }
        }
    }
    let resolved_col_path =
        index.and_then(|ix| ix.get(col.id)).map(|e| e.asset.path.clone()).or_else(|| col_path_hint.map(str::to_string));
    let resolved_dto = dto.or_else(|| dtos.values().find(|d| !d.id.is_nil() && d.id == col.dto));
    if let Some(cpath) = resolved_col_path {
        for (i, r) in col.rows.iter().enumerate() {
            if !r.id.is_nil() {
                let row_label = resolved_dto
                    .map(|d| r.short_view_string_with_schemas(d, dtos, enums))
                    .filter(|s| !s.is_empty() && s != "—" && s != "row")
                    .unwrap_or_else(|| format!("row#{i}"));
                names.insert(r.id, (DATA, format!("{cpath}#{row_label}")));
            }
        }
    }
    annotate_data_ron(&raw, &names, index)
}

/// Ensures all UUIDs (`col.id` and every `row.id`) are minted and writes `col` atomically to `path` with D27 trailing
/// comments.
pub fn save_collection(
    path: &Path,
    col: &mut CollectionAsset,
    dto_path: Option<&str>,
    dto: Option<&DtoAsset>,
) -> io::Result<()> {
    col.ensure_ids();
    let col_path_hint = infer_collection_rel_path(path);
    let empty_dtos = BTreeMap::new();
    let empty_enums = BTreeMap::new();
    col.sort_container_keys(dto, &empty_dtos, &empty_enums);
    project_core::writer::write_atomic(
        path,
        collection_text_inner(col, col_path_hint.as_deref(), dto_path, dto, &empty_dtos, &empty_enums, None),
    )
}

/// Ensures all UUIDs (`col.id` and every `row.id`) are minted and writes `col` atomically to `path` with D27 trailing
/// comments resolved across all project `dtos`.
pub fn save_collection_with_dtos(
    path: &Path,
    col: &mut CollectionAsset,
    dto: Option<&DtoAsset>,
    dtos: &BTreeMap<String, DtoAsset>,
) -> io::Result<()> {
    save_collection_with_index(path, col, dto, dtos, None)
}

/// Ensures all UUIDs (`col.id` and every `row.id`) are minted and writes `col` atomically to `path` with D27 trailing
/// comments resolved across all project `dtos` and `enums`.
pub fn save_collection_with_schemas(
    path: &Path,
    col: &mut CollectionAsset,
    dto: Option<&DtoAsset>,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> io::Result<()> {
    save_collection_with_schemas_and_index(path, col, dto, dtos, enums, None)
}

/// Ensures all UUIDs (`col.id` and every `row.id`) are minted and writes `col` atomically to `path` with D27 trailing
/// comments resolved across all project `dtos` and `index`.
pub fn save_collection_with_index(
    path: &Path,
    col: &mut CollectionAsset,
    dto: Option<&DtoAsset>,
    dtos: &BTreeMap<String, DtoAsset>,
    index: Option<&AssetIndex>,
) -> io::Result<()> {
    let empty_enums = BTreeMap::new();
    save_collection_with_schemas_and_index(path, col, dto, dtos, &empty_enums, index)
}

/// Ensures all UUIDs (`col.id` and every `row.id`) are minted and writes `col` atomically to `path` with D27 trailing
/// comments resolved across all project `dtos`, `enums`, and `index`.
pub fn save_collection_with_schemas_and_index(
    path: &Path,
    col: &mut CollectionAsset,
    dto: Option<&DtoAsset>,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    index: Option<&AssetIndex>,
) -> io::Result<()> {
    col.ensure_ids();
    col.sort_container_keys(dto, dtos, enums);
    let col_path_hint = infer_collection_rel_path(path);
    project_core::writer::write_atomic(
        path,
        collection_text_inner(col, col_path_hint.as_deref(), None, dto, dtos, enums, index),
    )
}

/// Assigns `id` as the root UUID of the Data asset `r` (used by `AE001` Regenerate UUID and `AE004` Assign UUID quick
/// fixes, ADR 0047 §11, §16), regenerating sub-item UUIDs when a duplicate file's root UUID is replaced so field/row
/// UUIDs do not collide with the original file's.
pub fn set_asset_uuid(
    project: &Project,
    r: &AssetRef,
    id: AssetUuid,
    index: &AssetIndex,
    dtos: &BTreeMap<String, DtoAsset>,
) -> Result<(), String> {
    set_asset_uuid_with_schemas(project, r, id, index, dtos, &BTreeMap::new())
}

/// Assigns `id` as the root UUID of the Data asset `r` with full `dtos` and `enums` context for D27 reference
/// annotation.
pub fn set_asset_uuid_with_schemas(
    project: &Project,
    r: &AssetRef,
    id: AssetUuid,
    index: &AssetIndex,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> Result<(), String> {
    let file = project.root.join(r.source_rel());
    let invalid = |e: String| format!("`{r}`: {e}");
    let written = match r.suffix {
        DTO_SUFFIX => {
            let mut dto = load_dto(&file).map_err(|e| invalid(e.to_string()))?;
            let was_duplicate = !dto.id.is_nil() && dto.id != id;
            dto.id = id;
            if was_duplicate {
                dto.regenerate_sub_ids();
            }
            save_dto_with_schemas_and_index(&file, &mut dto, dtos, enums, Some(index))
        }
        ENUM_SUFFIX => {
            let mut en = load_enum(&file).map_err(|e| invalid(e.to_string()))?;
            let was_duplicate = !en.id.is_nil() && en.id != id;
            en.id = id;
            if was_duplicate {
                en.regenerate_sub_ids();
            }
            save_enum_with_index(&file, &mut en, dtos, enums, Some(index))
        }
        COLLECTION_SUFFIX => {
            let mut col = load_collection(&file).map_err(|e| invalid(e.to_string()))?;
            let was_duplicate = !col.id.is_nil() && col.id != id;
            col.id = id;
            if was_duplicate {
                col.regenerate_sub_ids();
            }
            let dto_path = index
                .name_of(col.dto)
                .and_then(|(d, p)| (d == DATA).then_some(p))
                .or_else(|| dtos.iter().find_map(|(p, d)| (d.id == col.dto).then_some(p.as_str())));
            let dto_asset = dto_path.and_then(|p| dtos.get(p)).or_else(|| dtos.values().find(|d| d.id == col.dto));
            col.ensure_ids();
            project_core::writer::write_atomic(
                &file,
                collection_text_inner(&col, Some(r.path.as_str()), dto_path, dto_asset, dtos, enums, Some(index)),
            )
        }
        CURVE_SUFFIX => {
            let mut curve = load_curve(&file).map_err(|e| invalid(e.to_string()))?;
            curve.id = id;
            save_curve(&file, &mut curve)
        }
        _ => return Err(format!("`{r}` is not a Data asset")),
    };
    written.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::model::{DataValue, Field, FieldSettings, FieldType, Row};

    fn test_uuid(n: u128) -> AssetUuid {
        AssetUuid::from_u128(0x0196_3a10_0000_7000_8000_0000_0000_0000 | n)
    }

    #[test]
    fn dto_round_trip_and_version_checks() {
        let mut dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: test_uuid(1),
            doc: "A single book edition on the library shelf.".into(),
            fields: vec![
                Field {
                    id: test_uuid(101),
                    name: "title".into(),
                    ty: FieldType::LocalizedString,
                    category: "Identity & Catalog".into(),
                    is_name: true,
                    default: DataValue::LocalizedString("Untitled Book".into()),
                    settings: FieldSettings {
                        required: true,
                        max_len: Some(80),
                        doc: "Display title on the book spine.".into(),
                        ..FieldSettings::default()
                    },
                },
                Field {
                    id: test_uuid(102),
                    name: "pages".into(),
                    ty: FieldType::U32,
                    category: String::new(),
                    is_name: true,
                    default: DataValue::U32(250),
                    settings: FieldSettings::default(),
                },
            ],
        };

        let text = dto_text(&dto);
        let parsed = parse_dto(&text).unwrap();
        assert_eq!(parsed, dto);

        // Also parses tuple-style RON without struct names.
        let anon = "(format_version: 1, id: \"01963a10-0000-7000-8000-000000000001\", fields: [])";
        assert!(parse_dto(anon).is_ok());

        // Version checks.
        assert!(matches!(parse_dto("(format_version: 2, fields: [])"), Err(LoadError::TooNew { found: 2, .. })));
        assert!(matches!(parse_dto("(format_version: 0, fields: [])"), Err(LoadError::Unsupported(0))));

        // Disk save mints missing UUIDs and round-trips.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("book.dto.ron");
        dto.id = AssetUuid::NIL;
        dto.fields[0].id = AssetUuid::NIL;
        save_dto(&file, &mut dto).unwrap();
        assert!(dto.id.is_v7() && dto.fields[0].id.is_v7());
        assert_eq!(load_dto(&file).unwrap(), dto);
    }

    #[test]
    fn collection_d27_comments_annotate_dto_and_fields_but_not_row_id() {
        let dto_id = test_uuid(1);
        let f_title_id = test_uuid(101);
        let f_author_id = test_uuid(102);
        let stale_id = test_uuid(999);

        let dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: dto_id,
            doc: String::new(),
            fields: vec![
                Field {
                    id: f_title_id,
                    name: "title".into(),
                    ty: FieldType::LocalizedString,
                    category: "Identity".into(),
                    is_name: true,
                    default: DataValue::LocalizedString(String::new()),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_author_id,
                    name: "author".into(),
                    ty: FieldType::String,
                    category: "Identity".into(),
                    is_name: false,
                    default: DataValue::String(String::new()),
                    settings: FieldSettings::default(),
                },
            ],
        };

        let col_id = test_uuid(3);
        let row1_id = test_uuid(301);
        let row2_id = test_uuid(302);
        let mut row1_vals = BTreeMap::new();
        row1_vals.insert(f_title_id, DataValue::LocalizedString("The Shader Alchemist".into()));
        row1_vals.insert(f_author_id, DataValue::String("A. Vance".into()));
        row1_vals.insert(stale_id, DataValue::I32(42));

        let mut row2_vals = BTreeMap::new();
        row2_vals.insert(f_title_id, DataValue::LocalizedString("Atlas of Whispering Fogs".into()));
        row2_vals.insert(f_author_id, DataValue::String("M. Sterling".into()));

        let col = CollectionAsset {
            format_version: COLLECTION_FORMAT_VERSION,
            id: col_id,
            dto: dto_id,
            rows: vec![Row { id: row1_id, values: row1_vals }, Row { id: row2_id, values: row2_vals }],
        };

        let text = collection_text(&col, Some("library/book"), Some(&dto));
        // Root `id:` line has NO trailing comment.
        assert!(text.contains(&format!("  id: \"{col_id}\",\n")), "{text}");
        // `dto:` line has `// library/book`.
        assert!(text.contains(&format!("  dto: \"{dto_id}\", // library/book\n")), "{text}");
        // `Row.id` lines have NO trailing comment (`//`).
        assert!(text.contains(&format!("      id: \"{row1_id}\",\n")), "{text}");
        assert!(text.contains(&format!("      id: \"{row2_id}\",\n")), "{text}");
        // Field keys have `// title` and `// author`; unknown field has `// (missing)`.
        assert!(
            text.contains(&format!("\"{f_title_id}\": LocalizedString(\"The Shader Alchemist\"), // title\n")),
            "{text}"
        );
        assert!(text.contains(&format!("\"{f_author_id}\": String(\"A. Vance\"), // author\n")), "{text}");
        assert!(text.contains(&format!("\"{stale_id}\": I32(42), // (missing)\n")), "{text}");

        // Parsing the D27-commented text round-trips back to the exact `CollectionAsset`.
        let parsed = parse_collection(&text).unwrap();
        assert_eq!(parsed, col);
    }

    #[test]
    fn nested_dto_d27_comments_in_dto_and_collection() {
        let author_dto_id = test_uuid(2);
        let f_name_id = test_uuid(201);
        let f_born_id = test_uuid(202);
        let author_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: author_dto_id,
            doc: String::new(),
            fields: vec![
                Field {
                    id: f_name_id,
                    name: "name".into(),
                    ty: FieldType::String,
                    category: String::new(),
                    is_name: true,
                    default: DataValue::String("Unknown".into()),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_born_id,
                    name: "born_year".into(),
                    ty: FieldType::I32,
                    category: String::new(),
                    is_name: false,
                    default: DataValue::I32(1900),
                    settings: FieldSettings::default(),
                },
            ],
        };

        let book_dto_id = test_uuid(1);
        let f_title_id = test_uuid(101);
        let f_author_id = test_uuid(102);
        let mut def_author = BTreeMap::new();
        def_author.insert(f_name_id, DataValue::String("A. Vance".into()));
        def_author.insert(f_born_id, DataValue::I32(1892));
        let book_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: book_dto_id,
            doc: String::new(),
            fields: vec![
                Field {
                    id: f_title_id,
                    name: "title".into(),
                    ty: FieldType::LocalizedString,
                    category: String::new(),
                    is_name: true,
                    default: DataValue::LocalizedString("Untitled".into()),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_author_id,
                    name: "author".into(),
                    ty: FieldType::Dto(author_dto_id),
                    category: String::new(),
                    is_name: true,
                    default: DataValue::Dto(def_author.clone()),
                    settings: FieldSettings::default(),
                },
            ],
        };

        let mut dtos = BTreeMap::new();
        dtos.insert("library/author".into(), author_dto);
        dtos.insert("library/book".into(), book_dto.clone());

        let dto_ron = dto_text_with_dtos(&book_dto, &dtos);
        // Root `id:` and `Field.id` lines remain uncommented.
        assert!(dto_ron.contains(&format!("  id: \"{book_dto_id}\",\n")), "{dto_ron}");
        assert!(dto_ron.contains(&format!("      id: \"{f_author_id}\",\n")), "{dto_ron}");
        // `FieldType::Dto("<uuid>")` gets `// library/author`.
        assert!(dto_ron.contains(&format!("      ty: Dto(\"{author_dto_id}\"), // library/author\n")), "{dto_ron}");
        // Nested default map keys get `// name` and `// born_year`.
        assert!(dto_ron.contains(&format!("\"{f_name_id}\": String(\"A. Vance\"), // name\n")), "{dto_ron}");
        assert!(dto_ron.contains(&format!("\"{f_born_id}\": I32(1892), // born_year\n")), "{dto_ron}");
        assert_eq!(parse_dto(&dto_ron).unwrap(), book_dto);

        let mut row_vals = BTreeMap::new();
        row_vals.insert(f_title_id, DataValue::LocalizedString("The Shader Alchemist".into()));
        row_vals.insert(f_author_id, DataValue::Dto(def_author));
        let col = CollectionAsset {
            format_version: COLLECTION_FORMAT_VERSION,
            id: test_uuid(10),
            dto: book_dto_id,
            rows: vec![Row { id: test_uuid(100), values: row_vals }],
        };

        let col_ron = collection_text_with_dtos(&col, Some(&book_dto), &dtos);
        assert!(col_ron.contains(&format!("  dto: \"{book_dto_id}\", // library/book\n")), "{col_ron}");
        assert!(col_ron.contains(&format!("\"{f_author_id}\": Dto({{ // author\n")), "{col_ron}");
        assert!(col_ron.contains(&format!("\"{f_name_id}\": String(\"A. Vance\"), // name\n")), "{col_ron}");
        assert!(col_ron.contains(&format!("\"{f_born_id}\": I32(1892), // born_year\n")), "{col_ron}");
        assert_eq!(parse_collection(&col_ron).unwrap(), col);
    }

    #[test]
    fn direct_asset_ref_d27_comments_and_round_trip() {
        use crate::backend::model::GameAssetKind;
        use project_core::asset_index::{DomainIndex, IndexedAsset, SubItem};

        let curve_id = test_uuid(10);
        let grad_id = test_uuid(11);
        let mat_id = test_uuid(12);
        let tex_id = test_uuid(13);
        let env_id = test_uuid(14);
        let missing_id = test_uuid(99);

        let mat_domain = DomainId::new("material");
        let light_domain = DomainId::new("light");
        let index = AssetIndex::build(vec![
            DomainIndex {
                domain: mat_domain,
                assets: vec![
                    IndexedAsset {
                        asset: AssetRef { domain: mat_domain, path: "curves/ease_in".into(), suffix: "curve" },
                        id: Ok(curve_id),
                        sub_items: Vec::new(),
                        refs: Vec::new(),
                    },
                    IndexedAsset {
                        asset: AssetRef { domain: mat_domain, path: "palettes/fire".into(), suffix: "gradients" },
                        id: Ok(grad_id),
                        sub_items: vec![SubItem::new(1, "ember"), SubItem::new(2, "")],
                        refs: Vec::new(),
                    },
                    IndexedAsset {
                        asset: AssetRef { domain: mat_domain, path: "env/rock_wet".into(), suffix: "material" },
                        id: Ok(mat_id),
                        sub_items: Vec::new(),
                        refs: Vec::new(),
                    },
                    IndexedAsset {
                        asset: AssetRef { domain: mat_domain, path: "albedo/rock".into(), suffix: "texture" },
                        id: Ok(tex_id),
                        sub_items: Vec::new(),
                        refs: Vec::new(),
                    },
                ],
            },
            DomainIndex {
                domain: light_domain,
                assets: vec![IndexedAsset {
                    asset: AssetRef { domain: light_domain, path: "caves/crystal".into(), suffix: "environment" },
                    id: Ok(env_id),
                    sub_items: Vec::new(),
                    refs: Vec::new(),
                }],
            },
        ]);

        let dto_id = test_uuid(1);
        let f_curve = test_uuid(101);
        let f_grad = test_uuid(102);
        let f_mat = test_uuid(103);
        let f_tex = test_uuid(104);
        let f_env = test_uuid(105);
        let f_game = test_uuid(106);

        let dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: dto_id,
            doc: String::new(),
            fields: vec![
                Field {
                    id: f_curve,
                    name: "falloff".into(),
                    ty: FieldType::Curve,
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Curve(curve_id),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_grad,
                    name: "ramp".into(),
                    ty: FieldType::Gradient,
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Gradient(grad_id, 1),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_mat,
                    name: "surface".into(),
                    ty: FieldType::Material,
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Material(AssetUuid::NIL),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_tex,
                    name: "icon".into(),
                    ty: FieldType::Texture,
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Texture(tex_id),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_env,
                    name: "lighting".into(),
                    ty: FieldType::LightEnvironment,
                    category: String::new(),
                    is_name: false,
                    default: DataValue::LightEnvironment(env_id),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_game,
                    name: "sfx".into(),
                    ty: FieldType::GameAsset(GameAssetKind::Audio),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::GameAsset("audio/open.ogg".into()),
                    settings: FieldSettings::default(),
                },
            ],
        };

        let mut dtos = BTreeMap::new();
        dtos.insert("vfx_preset".into(), dto.clone());

        let dto_ron = dto_text_with_index(&dto, &dtos, Some(&index));
        assert!(
            dto_ron.contains(&format!("default: Curve(\"{curve_id}\"), // material:curves/ease_in\n")),
            "{dto_ron}"
        );
        assert!(
            dto_ron.contains(&format!("default: Gradient(\"{grad_id}\", 1), // material:palettes/fire.ember\n")),
            "{dto_ron}"
        );
        // Nil UUID default has no trailing comment.
        assert!(dto_ron.contains("default: Material(\"00000000-0000-0000-0000-000000000000\"),\n"), "{dto_ron}");
        assert!(dto_ron.contains(&format!("default: Texture(\"{tex_id}\"), // material:albedo/rock\n")), "{dto_ron}");
        assert!(
            dto_ron.contains(&format!("default: LightEnvironment(\"{env_id}\"), // light:caves/crystal\n")),
            "{dto_ron}"
        );
        assert_eq!(parse_dto(&dto_ron).unwrap(), dto);

        let mut row_vals = BTreeMap::new();
        row_vals.insert(f_curve, DataValue::Curve(AssetUuid::NIL));
        row_vals.insert(f_grad, DataValue::Gradient(grad_id, 2)); // unnamed slot 2 -> material:palettes/fire
        row_vals.insert(f_mat, DataValue::Material(mat_id));
        row_vals.insert(f_tex, DataValue::Texture(missing_id));
        row_vals.insert(f_env, DataValue::LightEnvironment(env_id));
        row_vals.insert(f_game, DataValue::GameAsset("audio/close.ogg".into()));

        let col = CollectionAsset {
            format_version: COLLECTION_FORMAT_VERSION,
            id: test_uuid(20),
            dto: dto_id,
            rows: vec![Row { id: test_uuid(200), values: row_vals }],
        };

        let col_ron = collection_text_with_index(&col, Some(&dto), &dtos, Some(&index));
        assert!(
            col_ron.contains(&format!("\"{f_curve}\": Curve(\"00000000-0000-0000-0000-000000000000\"), // falloff\n")),
            "{col_ron}"
        );
        assert!(
            col_ron.contains(&format!("\"{f_grad}\": Gradient(\"{grad_id}\", 2), // ramp, material:palettes/fire\n")),
            "{col_ron}"
        );
        assert!(
            col_ron.contains(&format!("\"{f_mat}\": Material(\"{mat_id}\"), // surface, material:env/rock_wet\n")),
            "{col_ron}"
        );
        assert!(
            col_ron.contains(&format!("\"{f_tex}\": Texture(\"{missing_id}\"), // icon, (missing)\n")),
            "{col_ron}"
        );
        assert!(
            col_ron
                .contains(&format!("\"{f_env}\": LightEnvironment(\"{env_id}\"), // lighting, light:caves/crystal\n")),
            "{col_ron}"
        );
        assert_eq!(parse_collection(&col_ron).unwrap(), col);
    }

    #[test]
    fn dto_and_collection_soft_ref_d27_comments() {
        use project_core::asset_index::{DomainIndex, IndexedAsset, SubItem};

        let book_dto_id = test_uuid(50);
        let f_title = test_uuid(51);
        let f_sequel = test_uuid(52);
        let book_row_0 = test_uuid(53);
        let book_row_1 = test_uuid(54);
        let missing_row = test_uuid(59);

        let book_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: book_dto_id,
            doc: String::new(),
            fields: vec![
                Field {
                    id: f_title,
                    name: "title".into(),
                    ty: FieldType::String,
                    category: String::new(),
                    is_name: true,
                    default: DataValue::String("Untitled".into()),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_sequel,
                    name: "sequel".into(),
                    ty: FieldType::SoftRef(book_dto_id),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::SoftRef(book_row_0),
                    settings: FieldSettings::default(),
                },
            ],
        };

        let mut dtos = BTreeMap::new();
        dtos.insert("book".into(), book_dto.clone());

        let index = AssetIndex::build(vec![DomainIndex {
            domain: DATA,
            assets: vec![
                IndexedAsset {
                    asset: AssetRef { domain: DATA, path: "book".into(), suffix: "dto" },
                    id: Ok(book_dto_id),
                    sub_items: vec![SubItem::with_uuid(f_title, "title"), SubItem::with_uuid(f_sequel, "sequel")],
                    refs: Vec::new(),
                },
                IndexedAsset {
                    asset: AssetRef { domain: DATA, path: "books".into(), suffix: "collection" },
                    id: Ok(test_uuid(55)),
                    sub_items: vec![
                        SubItem::with_uuid(book_row_0, "The Shader Alchemist"),
                        SubItem::with_uuid(book_row_1, "Volume II"),
                    ],
                    refs: Vec::new(),
                },
            ],
        }]);

        let dto_ron = dto_text_with_index(&book_dto, &dtos, Some(&index));
        assert!(dto_ron.contains(&format!("ty: SoftRef(\"{book_dto_id}\"), // book\n")), "{dto_ron}");
        assert!(
            dto_ron.contains(&format!("default: SoftRef(\"{book_row_0}\"), // books#The Shader Alchemist\n")),
            "{dto_ron}"
        );
        assert_eq!(parse_dto(&dto_ron).unwrap(), book_dto);

        let col = CollectionAsset {
            format_version: COLLECTION_FORMAT_VERSION,
            id: test_uuid(55),
            dto: book_dto_id,
            rows: vec![
                Row {
                    id: book_row_0,
                    values: BTreeMap::from([
                        (f_title, DataValue::String("The Shader Alchemist".into())),
                        (f_sequel, DataValue::SoftRef(book_row_1)),
                    ]),
                },
                Row {
                    id: book_row_1,
                    values: BTreeMap::from([
                        (f_title, DataValue::String("Volume II".into())),
                        (f_sequel, DataValue::SoftRef(AssetUuid::NIL)),
                    ]),
                },
                Row {
                    id: test_uuid(56),
                    values: BTreeMap::from([
                        (f_title, DataValue::String("Dangling".into())),
                        (f_sequel, DataValue::SoftRef(missing_row)),
                    ]),
                },
            ],
        };

        let col_ron = collection_text_with_index(&col, Some(&book_dto), &dtos, Some(&index));
        assert!(
            col_ron.contains(&format!("\"{f_sequel}\": SoftRef(\"{book_row_1}\"), // sequel, books#Volume II\n")),
            "{col_ron}"
        );
        assert!(
            col_ron
                .contains(&format!("\"{f_sequel}\": SoftRef(\"00000000-0000-0000-0000-000000000000\"), // sequel\n")),
            "{col_ron}"
        );
        assert!(
            col_ron.contains(&format!("\"{f_sequel}\": SoftRef(\"{missing_row}\"), // sequel, (missing)\n")),
            "{col_ron}"
        );
        assert_eq!(parse_collection(&col_ron).unwrap(), col);
    }

    #[test]
    fn enum_ron_round_trip_and_d27_comments_for_variants_and_fields() {
        use crate::backend::model::EnumVariant;
        use project_core::asset_index::{DomainIndex, IndexedAsset, SubItem};

        let enum_id = test_uuid(70);
        let v_none = test_uuid(71);
        let v_ward = test_uuid(72);
        let f_strength = test_uuid(73);
        let f_origin = test_uuid(74);
        let planet_dto_id = test_uuid(80);
        let f_planet_name = test_uuid(81);

        let planet_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: planet_dto_id,
            doc: String::new(),
            fields: vec![Field {
                id: f_planet_name,
                name: "name".into(),
                ty: FieldType::String,
                category: String::new(),
                is_name: true,
                default: DataValue::String("Arrakis".into()),
                settings: FieldSettings::default(),
            }],
        };
        let enchantment = EnumAsset {
            format_version: ENUM_FORMAT_VERSION,
            id: enum_id,
            doc: "Enchantment union".into(),
            variants: vec![
                EnumVariant { id: v_none, name: "none".into(), doc: String::new(), fields: Vec::new() },
                EnumVariant {
                    id: v_ward,
                    name: "ward".into(),
                    doc: "Protective ward".into(),
                    fields: vec![
                        Field {
                            id: f_strength,
                            name: "strength".into(),
                            ty: FieldType::F32,
                            category: String::new(),
                            is_name: true,
                            default: DataValue::F32(10.0),
                            settings: FieldSettings::default(),
                        },
                        Field {
                            id: f_origin,
                            name: "origin".into(),
                            ty: FieldType::Dto(planet_dto_id),
                            category: String::new(),
                            is_name: false,
                            default: DataValue::Dto(BTreeMap::from([(
                                f_planet_name,
                                DataValue::String("Caladan".into()),
                            )])),
                            settings: FieldSettings::default(),
                        },
                    ],
                },
            ],
        };

        let dtos = BTreeMap::from([("library/planet".to_string(), planet_dto.clone())]);
        let enums = BTreeMap::from([("library/enchantment".to_string(), enchantment.clone())]);

        // 1. `.enum.ron` text keeps `EnumAsset.id`, `EnumVariant.id`, and `Field.id` uncommented, and comments `ty: Dto(...)` and nested keys.
        let enum_ron = enum_text_with_schemas(&enchantment, &dtos, &enums);
        assert!(enum_ron.contains(&format!("  id: \"{enum_id}\",\n")), "{enum_ron}");
        assert!(enum_ron.contains(&format!("      id: \"{v_none}\",\n")), "{enum_ron}");
        assert!(enum_ron.contains(&format!("      id: \"{v_ward}\",\n")), "{enum_ron}");
        assert!(enum_ron.contains(&format!("          id: \"{f_strength}\",\n")), "{enum_ron}");
        assert!(enum_ron.contains(&format!("          id: \"{f_origin}\",\n")), "{enum_ron}");
        assert!(
            enum_ron.contains(&format!("          ty: Dto(\"{planet_dto_id}\"), // library/planet\n")),
            "{enum_ron}"
        );
        assert!(
            enum_ron.contains(&format!("            \"{f_planet_name}\": String(\"Caladan\"), // name\n")),
            "{enum_ron}"
        );
        assert_eq!(parse_enum(&enum_ron).unwrap(), enchantment);

        // 2. `.dto.ron` and `.collection.ron` text annotate `ty: Enum(...)`, `variant: "..."`, and payload `fields` keys (both via `enums` map and via `AssetIndex`).
        let f_ench = test_uuid(91);
        let book_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: test_uuid(90),
            doc: String::new(),
            fields: vec![Field {
                id: f_ench,
                name: "enchantment".into(),
                ty: FieldType::Enum(enum_id),
                category: String::new(),
                is_name: true,
                default: DataValue::Enum {
                    variant: v_ward,
                    fields: BTreeMap::from([(f_strength, DataValue::F32(15.0))]),
                },
                settings: FieldSettings::default(),
            }],
        };
        let dtos_with_book = BTreeMap::from([
            ("library/planet".to_string(), planet_dto),
            ("library/book".to_string(), book_dto.clone()),
        ]);

        let index = AssetIndex::build(vec![DomainIndex {
            domain: DATA,
            assets: vec![IndexedAsset {
                asset: AssetRef { domain: DATA, path: "library/enchantment".into(), suffix: ENUM_SUFFIX },
                id: Ok(enum_id),
                sub_items: vec![
                    SubItem::with_uuid(v_none, "none"),
                    SubItem::with_uuid(v_ward, "ward"),
                    SubItem::with_uuid(f_strength, "ward.strength"),
                    SubItem::with_uuid(f_origin, "ward.origin"),
                ],
                refs: Vec::new(),
            }],
        }]);

        let dto_ron = dto_text_with_index(&book_dto, &dtos_with_book, Some(&index));
        assert!(dto_ron.contains(&format!("      ty: Enum(\"{enum_id}\"), // library/enchantment\n")), "{dto_ron}");
        assert!(
            dto_ron.contains(&format!("        variant: \"{v_ward}\", // library/enchantment::ward\n")),
            "{dto_ron}"
        );
        assert!(dto_ron.contains(&format!("          \"{f_strength}\": F32(15.0), // strength\n")), "{dto_ron}");
        assert_eq!(parse_dto(&dto_ron).unwrap(), book_dto);
    }

    #[test]
    fn container_types_and_values_ron_round_trip_d27_comments_and_sorted_keys() {
        let author_dto_id = test_uuid(110);
        let f_author_name = test_uuid(111);
        let author_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: author_dto_id,
            doc: String::new(),
            fields: vec![Field {
                id: f_author_name,
                name: "name".into(),
                ty: FieldType::String,
                category: String::new(),
                is_name: true,
                default: DataValue::String("Unknown".into()),
                settings: FieldSettings::default(),
            }],
        };

        let book_dto_id = test_uuid(120);
        let f_opt_author = test_uuid(121);
        let f_arr_tags = test_uuid(122);
        let f_set_codes = test_uuid(123);
        let f_map_authors = test_uuid(124);

        let book_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: book_dto_id,
            doc: String::new(),
            fields: vec![
                Field {
                    id: f_opt_author,
                    name: "co_author".into(),
                    ty: FieldType::Option(Box::new(FieldType::Dto(author_dto_id))),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Option(Some(Box::new(DataValue::Dto(BTreeMap::from([(
                        f_author_name,
                        DataValue::String("B. Sterling".into()),
                    )]))))),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_arr_tags,
                    name: "tags".into(),
                    ty: FieldType::Array(Box::new(FieldType::String)),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Array(vec![
                        DataValue::String("magic".into()),
                        DataValue::String("rare".into()),
                    ]),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_set_codes,
                    name: "codes".into(),
                    ty: FieldType::Set(Box::new(FieldType::I32)),
                    category: String::new(),
                    is_name: false,
                    // Intentionally unsorted to verify deterministic sorting on serialization
                    default: DataValue::Set(vec![DataValue::I32(30), DataValue::I32(10), DataValue::I32(20)]),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_map_authors,
                    name: "contributors".into(),
                    ty: FieldType::Map(
                        Box::new(FieldType::SoftRef(author_dto_id)),
                        Box::new(FieldType::Dto(author_dto_id)),
                    ),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Map(vec![
                        (
                            DataValue::SoftRef(test_uuid(902)),
                            DataValue::Dto(BTreeMap::from([(f_author_name, DataValue::String("Zed".into()))])),
                        ),
                        (
                            DataValue::SoftRef(test_uuid(901)),
                            DataValue::Dto(BTreeMap::from([(f_author_name, DataValue::String("Ada".into()))])),
                        ),
                    ]),
                    settings: FieldSettings::default(),
                },
            ],
        };

        let dtos = BTreeMap::from([
            ("library/author".to_string(), author_dto),
            ("library/book".to_string(), book_dto.clone()),
        ]);

        let dto_ron = dto_text_with_dtos(&book_dto, &dtos);
        assert!(dto_ron.contains(&format!("ty: Option(Dto(\"{author_dto_id}\")), // library/author\n")), "{dto_ron}");
        assert!(
            dto_ron.contains(&format!(
                "ty: Map(SoftRef(\"{author_dto_id}\"), Dto(\"{author_dto_id}\")), // library/author, library/author\n"
            )),
            "{dto_ron}"
        );
        let parsed = parse_dto(&dto_ron).unwrap();
        // Set keys and Map entries were sorted deterministically:
        assert_eq!(
            parsed.fields[2].default,
            DataValue::Set(vec![DataValue::I32(10), DataValue::I32(20), DataValue::I32(30)])
        );
        if let DataValue::Map(entries) = &parsed.fields[3].default {
            assert_eq!(entries[0].0, DataValue::SoftRef(test_uuid(901)));
            assert_eq!(entries[1].0, DataValue::SoftRef(test_uuid(902)));
        } else {
            panic!("expected Map");
        }
    }
}
