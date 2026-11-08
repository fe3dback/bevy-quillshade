//! `.qsd` binary container baker, `DATA_SCHEMA_HASH` (FNV-1a 64-bit), and non-panicking header/payload verification
//! (ADR 0049 §6, techdoc §7.7, §9.3).

use std::collections::{BTreeMap, BTreeSet};

use project_core::common::curve::{CurveData, Interp, normalize_points};
use project_core::common::gradient::{DEFAULT_MID, MID_MAX, MID_MIN, RawGradient, RawStop};
use project_core::{AssetIndex, AssetUuid, Project};
use serde::de::{DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq, SerializeTuple, SerializeTupleVariant};
use serde::{Deserialize, Serialize, Serializer};

use super::model::{CollectionAsset, DataValue, DtoAsset, EnumAsset, Field, FieldType, MAX_NESTING_DEPTH};

/// 8-byte magic prefix of every `.collection.qsd` binary file.
pub const QSD_MAGIC: [u8; 8] = *b"QSDATA\0\0";

/// Binary container format version written at bytes `8..12`.
pub const QSD_VERSION: u32 = 1;

/// Fixed little-endian header length in bytes: `8 (magic) + 4 (version) + 8 (schema_hash) + 16 (dto_uuid) = 36`.
pub const QSD_HEADER_LEN: usize = 36;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a_update(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// Computes the deterministic 64-bit FNV-1a structural schema hash of a single `dto` (and any transitively nested
/// `FieldType::Dto` schemas in `dtos`).
pub fn compute_schema_hash_with_dtos(dto: &DtoAsset, dtos: &BTreeMap<String, DtoAsset>) -> u64 {
    let empty_enums = BTreeMap::new();
    compute_dto_schema_hash_with_schemas(dto, dtos, &empty_enums)
}

/// Computes the deterministic 64-bit FNV-1a structural schema hash of a single `dto` (and any transitively nested
/// `FieldType::Dto` and `FieldType::Enum` schemas in `dtos` and `enums`).
pub fn compute_dto_schema_hash_with_schemas(
    dto: &DtoAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> u64 {
    let mut visited = BTreeSet::new();
    compute_dto_schema_hash_inner(dto, dtos, enums, &mut visited, 0)
}

fn hash_field_type_schema(
    mut h: u64,
    ty: &FieldType,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    visited: &mut BTreeSet<AssetUuid>,
    depth: usize,
) -> u64 {
    h = fnv1a_update(h, ty.canonical_tag().as_bytes());
    h = fnv1a_update(h, &[0]);
    match ty {
        FieldType::GameAsset(kind) => {
            h = fnv1a_update(h, &[ty.schema_tag(), kind.schema_tag()]);
        }
        FieldType::SoftRef(target_uuid) => {
            h = fnv1a_update(h, &[ty.schema_tag()]);
            h = fnv1a_update(h, &target_uuid.as_u128().to_le_bytes());
        }
        FieldType::Dto(target_uuid) => {
            h = fnv1a_update(h, &[11]);
            h = fnv1a_update(h, &target_uuid.as_u128().to_le_bytes());
            if depth < MAX_NESTING_DEPTH
                && !target_uuid.is_nil()
                && visited.insert(*target_uuid)
                && let Some((_, target_dto)) = dtos.iter().find(|(_, d)| d.id == *target_uuid)
            {
                let sub_hash = compute_dto_schema_hash_inner(target_dto, dtos, enums, visited, depth + 1);
                h = fnv1a_update(h, &sub_hash.to_le_bytes());
                visited.remove(target_uuid);
            }
        }
        FieldType::Enum(target_uuid) => {
            h = fnv1a_update(h, &[ty.schema_tag()]);
            h = fnv1a_update(h, &target_uuid.as_u128().to_le_bytes());
            if depth < MAX_NESTING_DEPTH
                && !target_uuid.is_nil()
                && visited.insert(*target_uuid)
                && let Some((_, target_enum)) = enums.iter().find(|(_, e)| e.id == *target_uuid)
            {
                let sub_hash = compute_enum_schema_hash_inner(target_enum, dtos, enums, visited, depth + 1);
                h = fnv1a_update(h, &sub_hash.to_le_bytes());
                visited.remove(target_uuid);
            }
        }
        FieldType::Option(inner) | FieldType::Array(inner) | FieldType::Set(inner) => {
            h = fnv1a_update(h, &[ty.schema_tag()]);
            if depth < MAX_NESTING_DEPTH {
                h = hash_field_type_schema(h, inner, dtos, enums, visited, depth + 1);
            }
        }
        FieldType::Map(k, v) => {
            h = fnv1a_update(h, &[ty.schema_tag()]);
            if depth < MAX_NESTING_DEPTH {
                h = hash_field_type_schema(h, k, dtos, enums, visited, depth + 1);
                h = hash_field_type_schema(h, v, dtos, enums, visited, depth + 1);
            }
        }
        _ => {}
    }
    h
}

fn hash_field_schema(
    mut h: u64,
    field: &Field,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    visited: &mut BTreeSet<AssetUuid>,
    depth: usize,
) -> u64 {
    h = fnv1a_update(h, &field.id.as_u128().to_le_bytes());
    h = fnv1a_update(h, &[0]);
    h = fnv1a_update(h, field.name.as_bytes());
    h = fnv1a_update(h, &[0]);
    hash_field_type_schema(h, &field.ty, dtos, enums, visited, depth)
}

fn compute_dto_schema_hash_inner(
    dto: &DtoAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    visited: &mut BTreeSet<AssetUuid>,
    depth: usize,
) -> u64 {
    let mut h = FNV_OFFSET;
    h = fnv1a_update(h, &dto.id.as_u128().to_le_bytes());
    h = fnv1a_update(h, &(dto.fields.len() as u64).to_le_bytes());
    for field in &dto.fields {
        h = hash_field_schema(h, field, dtos, enums, visited, depth);
    }
    h
}

fn compute_enum_schema_hash_inner(
    en: &EnumAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    visited: &mut BTreeSet<AssetUuid>,
    depth: usize,
) -> u64 {
    let mut h = FNV_OFFSET;
    h = fnv1a_update(h, &en.id.as_u128().to_le_bytes());
    h = fnv1a_update(h, &(en.variants.len() as u64).to_le_bytes());
    for variant in &en.variants {
        h = fnv1a_update(h, &variant.id.as_u128().to_le_bytes());
        h = fnv1a_update(h, &[0]);
        h = fnv1a_update(h, variant.name.as_bytes());
        h = fnv1a_update(h, &[0]);
        h = fnv1a_update(h, &(variant.fields.len() as u64).to_le_bytes());
        for field in &variant.fields {
            h = hash_field_schema(h, field, dtos, enums, visited, depth);
        }
    }
    h
}

/// Computes the deterministic 64-bit FNV-1a `DATA_SCHEMA_HASH` over all valid `.dto.ron` schemas in path order
/// (ADR 0049 §6).
///
/// Hashes only structural schema identity (`dto.id`, `dto_path`, field count, and each field's `id`, `name`, and
/// `ty`, including recursive `FieldType::Dto` target structure). Editing `.collection.ron` row values or soft
/// UI/metadata settings (`doc`, `category`, `summary`, `default`, `FieldSettings`) leaves `DATA_SCHEMA_HASH` unchanged
/// so value changes hot-reload without recompiling the game.
pub fn compute_schema_hash(dtos: &BTreeMap<String, DtoAsset>) -> u64 {
    let empty_enums = BTreeMap::new();
    compute_schema_hash_with_enums(dtos, &empty_enums)
}

/// Computes the deterministic 64-bit FNV-1a `DATA_SCHEMA_HASH` over all valid `.dto.ron` and `.enum.ron` schemas in
/// path order (ADR 0049 §6).
pub fn compute_schema_hash_with_enums(dtos: &BTreeMap<String, DtoAsset>, enums: &BTreeMap<String, EnumAsset>) -> u64 {
    let mut h = FNV_OFFSET;
    h = fnv1a_update(h, &(dtos.len() as u64).to_le_bytes());
    for (dto_path, dto) in dtos {
        h = fnv1a_update(h, &dto.id.as_u128().to_le_bytes());
        h = fnv1a_update(h, &[0]);
        h = fnv1a_update(h, dto_path.as_bytes());
        h = fnv1a_update(h, &[0]);
        h = fnv1a_update(h, &(dto.fields.len() as u64).to_le_bytes());
        for field in &dto.fields {
            let mut visited = BTreeSet::new();
            if !dto.id.is_nil() {
                visited.insert(dto.id);
            }
            h = hash_field_schema(h, field, dtos, enums, &mut visited, 0);
        }
    }
    for (enum_path, en) in enums {
        h = fnv1a_update(h, b"enum\0");
        h = fnv1a_update(h, &en.id.as_u128().to_le_bytes());
        h = fnv1a_update(h, &[0]);
        h = fnv1a_update(h, enum_path.as_bytes());
        h = fnv1a_update(h, &[0]);
        h = fnv1a_update(h, &(en.variants.len() as u64).to_le_bytes());
        for variant in &en.variants {
            h = fnv1a_update(h, &variant.id.as_u128().to_le_bytes());
            h = fnv1a_update(h, &[0]);
            h = fnv1a_update(h, variant.name.as_bytes());
            h = fnv1a_update(h, &[0]);
            h = fnv1a_update(h, &(variant.fields.len() as u64).to_le_bytes());
            for field in &variant.fields {
                let mut visited = BTreeSet::new();
                if !en.id.is_nil() {
                    visited.insert(en.id);
                }
                h = hash_field_schema(h, field, dtos, enums, &mut visited, 0);
            }
        }
    }
    h
}

/// Parsed 36-byte `.qsd` container header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QsdHeader {
    pub version: u32,
    pub schema_hash: u64,
    pub dto_uuid: u128,
}

/// Non-panicking error returned when verifying or decoding a `.qsd` binary buffer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QsdError {
    #[error("truncated .qsd file: expected at least {QSD_HEADER_LEN} bytes, got {0}")]
    TooShort(usize),
    #[error("invalid .qsd magic header (expected `QSDATA\\0\\0`)")]
    BadMagic,
    #[error("unsupported .qsd container version {found} (expected {QSD_VERSION})")]
    UnsupportedVersion { found: u32 },
    #[error("schema hash mismatch: expected {expected:#018x}, found {found:#018x}; rebuild game")]
    SchemaHashMismatch { expected: u64, found: u64 },
    #[error("DTO UUID mismatch: expected {expected:#034x}, found {found:#034x}")]
    DtoMismatch { expected: u128, found: u128 },
    #[error("postcard decode error: {0}")]
    Decode(String),
}

/// Parses the 36-byte header of a `.qsd` buffer, checking `QSD_MAGIC` and `QSD_VERSION`.
pub fn parse_qsd_header(bytes: &[u8]) -> Result<QsdHeader, QsdError> {
    if bytes.len() < QSD_HEADER_LEN {
        return Err(QsdError::TooShort(bytes.len()));
    }
    if bytes[0..8] != QSD_MAGIC {
        return Err(QsdError::BadMagic);
    }
    let version = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    if version != QSD_VERSION {
        return Err(QsdError::UnsupportedVersion { found: version });
    }
    let schema_hash = u64::from_le_bytes(bytes[12..20].try_into().expect("8 bytes"));
    let dto_uuid = u128::from_le_bytes(bytes[20..36].try_into().expect("16 bytes"));
    Ok(QsdHeader { version, schema_hash, dto_uuid })
}

/// Parses the 36-byte header of `bytes`, verifies `expected_schema_hash` and optional `expected_dto_uuid`, and returns
/// `(header, payload_slice)`.
pub fn verify_qsd_header(
    bytes: &[u8],
    expected_schema_hash: u64,
    expected_dto_uuid: Option<u128>,
) -> Result<(QsdHeader, &[u8]), QsdError> {
    let header = parse_qsd_header(bytes)?;
    if header.schema_hash != expected_schema_hash {
        return Err(QsdError::SchemaHashMismatch { expected: expected_schema_hash, found: header.schema_hash });
    }
    if let Some(expected_dto) = expected_dto_uuid
        && header.dto_uuid != expected_dto
    {
        return Err(QsdError::DtoMismatch { expected: expected_dto, found: header.dto_uuid });
    }
    Ok((header, &bytes[QSD_HEADER_LEN..]))
}

/// Resolves cross-domain asset references (`Curve`, `Gradient`, `Material`, `Texture`, `LightEnvironment`, `GameAsset`)
/// into their baked `.qsd` wire representations (ADR 0049 §6).
#[derive(Default, Clone, Copy)]
pub struct DataBakeResolver<'a> {
    pub project: Option<&'a Project>,
    pub index: Option<&'a AssetIndex>,
}

fn default_one() -> f32 {
    1.0
}

fn default_mid() -> f32 {
    0.5
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CurvePointRon {
    Tuple((f32, f32)),
    Seq(Vec<f32>),
}

impl CurvePointRon {
    fn into_pair(self) -> (f32, f32) {
        match self {
            CurvePointRon::Tuple((x, y)) => (x, y),
            CurvePointRon::Seq(v) => (v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(0.0)),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename = "Curve")]
struct CurveFileRon {
    #[serde(default)]
    interp: Interp,
    #[serde(default)]
    min: f32,
    #[serde(default = "default_one")]
    max: f32,
    #[serde(default)]
    points: Vec<CurvePointRon>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StopColorRon {
    Tuple((f32, f32, f32, f32)),
    Seq(Vec<f32>),
}

impl StopColorRon {
    fn into_array(self) -> [f32; 4] {
        match self {
            StopColorRon::Tuple((r, g, b, a)) => [r, g, b, a],
            StopColorRon::Seq(v) => [
                v.first().copied().unwrap_or(0.0),
                v.get(1).copied().unwrap_or(0.0),
                v.get(2).copied().unwrap_or(0.0),
                v.get(3).copied().unwrap_or(1.0),
            ],
        }
    }
}

#[derive(Deserialize)]
#[serde(rename = "Stop")]
struct StopFileRon {
    #[serde(alias = "pos")]
    t: f32,
    color: StopColorRon,
    #[serde(default = "default_mid")]
    mid: f32,
}

#[derive(Deserialize)]
#[serde(rename = "Slot")]
struct GradientSlotRon {
    id: u32,
    #[serde(default)]
    stops: Vec<StopFileRon>,
}

#[derive(Deserialize)]
#[serde(rename = "GradientCollection")]
struct GradientFileRon {
    #[serde(default, alias = "gradients")]
    slots: Vec<GradientSlotRon>,
}

#[derive(Deserialize)]
#[serde(rename = "TextureAsset")]
struct TextureFileRon {
    #[serde(default)]
    path: String,
}

impl DataBakeResolver<'_> {
    /// Resolves a `.curve.ron` reference into [`CurveData`], returning [`CurveData::default()`] when `uuid` is nil or
    /// the source file cannot be read.
    pub fn resolve_curve(&self, uuid: AssetUuid) -> CurveData {
        if uuid.is_nil() {
            return CurveData::default();
        }
        let Some(proj) = self.project else {
            return CurveData::default();
        };
        let Some(ix) = self.index else {
            return CurveData::default();
        };
        let Some(entry) = ix.get(uuid) else {
            return CurveData::default();
        };
        let Ok(text) = std::fs::read_to_string(proj.root.join(entry.asset.source_rel())) else {
            return CurveData::default();
        };
        let Ok(parsed) = ron::Options::default().from_str::<CurveFileRon>(&text) else {
            return CurveData::default();
        };
        let mut pts: Vec<(f32, f32)> = parsed.points.into_iter().map(CurvePointRon::into_pair).collect();
        if pts.is_empty() {
            return CurveData::default();
        }
        normalize_points(&mut pts);
        CurveData { interp: parsed.interp, min: parsed.min, max: parsed.max, points: pts }
    }

    /// Resolves a `.gradients.ron` slot reference into [`RawGradient`], returning [`RawGradient::default()`] when
    /// `uuid` is nil, `slot == 0`, or the target slot is not found.
    pub fn resolve_gradient(&self, uuid: AssetUuid, slot: u32) -> RawGradient {
        if uuid.is_nil() || slot == 0 {
            return RawGradient::default();
        }
        let Some(proj) = self.project else {
            return RawGradient::default();
        };
        let Some(ix) = self.index else {
            return RawGradient::default();
        };
        let Some(entry) = ix.get(uuid) else {
            return RawGradient::default();
        };
        let Ok(text) = std::fs::read_to_string(proj.root.join(entry.asset.source_rel())) else {
            return RawGradient::default();
        };
        let Ok(parsed) = ron::Options::default().from_str::<GradientFileRon>(&text) else {
            return RawGradient::default();
        };
        let Some(slot_def) = parsed.slots.into_iter().find(|s| s.id == slot) else {
            return RawGradient::default();
        };
        if slot_def.stops.is_empty() {
            return RawGradient::default();
        }
        let mut stops: Vec<RawStop> = slot_def
            .stops
            .into_iter()
            .map(|s| RawStop {
                t: if s.t.is_finite() { s.t.clamp(0.0, 1.0) } else { 0.0 },
                color: s.color.into_array().map(|c| if c.is_finite() { c.clamp(0.0, 1.0) } else { 0.0 }),
                mid: if s.mid.is_finite() { s.mid.clamp(MID_MIN, MID_MAX) } else { DEFAULT_MID },
            })
            .collect();
        stops.sort_by(|a, b| a.t.total_cmp(&b.t));
        RawGradient { stops }
    }

    /// Resolves a `.material.ron` UUID into its wire `u128` (`0` when nil).
    pub const fn resolve_material(&self, uuid: AssetUuid) -> u128 {
        if uuid.is_nil() { 0 } else { uuid.as_u128() }
    }

    /// Resolves a `.texture.ron` UUID into `(uuid.as_u128(), image_path)` (`(0, "")` when nil).
    pub fn resolve_texture(&self, uuid: AssetUuid) -> (u128, String) {
        if uuid.is_nil() {
            return (0, String::new());
        }
        let path = (|| {
            let proj = self.project?;
            let ix = self.index?;
            let entry = ix.get(uuid)?;
            let text = std::fs::read_to_string(proj.root.join(entry.asset.source_rel())).ok()?;
            let parsed: TextureFileRon = ron::Options::default().from_str(&text).ok()?;
            Some(parsed.path)
        })()
        .unwrap_or_default();
        (uuid.as_u128(), path)
    }

    /// Resolves a `.environment.ron` UUID into its domain-relative path string (`""` when nil).
    pub fn resolve_light_env(&self, uuid: AssetUuid) -> String {
        if uuid.is_nil() {
            return String::new();
        }
        self.index.and_then(|ix| ix.get(uuid)).map(|e| e.asset.path.clone()).unwrap_or_default()
    }
}

enum WireValue<'a> {
    Leaf(&'a DataValue),
    Dto(Vec<WireValue<'a>>),
    Enum(u32, Vec<WireValue<'a>>),
    Option(Option<Box<WireValue<'a>>>),
    Seq(Vec<WireValue<'a>>),
    Map(Vec<(WireValue<'a>, WireValue<'a>)>),
    Curve(CurveData),
    Gradient(RawGradient),
    Material(u128),
    Texture((u128, String)),
    LightEnvironment(String),
    GameAsset(String),
    SoftRef(u128),
}

impl Serialize for WireValue<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            WireValue::Leaf(val) => match val {
                DataValue::Bool(v) => v.serialize(serializer),
                DataValue::I8(v) => v.serialize(serializer),
                DataValue::I16(v) => v.serialize(serializer),
                DataValue::I32(v) => v.serialize(serializer),
                DataValue::I64(v) => v.serialize(serializer),
                DataValue::U8(v) => v.serialize(serializer),
                DataValue::U16(v) => v.serialize(serializer),
                DataValue::U32(v) => v.serialize(serializer),
                DataValue::U64(v) => v.serialize(serializer),
                DataValue::F32(v) => v.serialize(serializer),
                DataValue::F64(v) => v.serialize(serializer),
                DataValue::String(v) | DataValue::LocalizedString(v) => v.serialize(serializer),
                DataValue::Vec2(v) | DataValue::RangeF32(v) => v.serialize(serializer),
                DataValue::Vec3(v) => v.serialize(serializer),
                DataValue::Vec4(v) | DataValue::Quat(v) | DataValue::Color(v) => v.serialize(serializer),
                DataValue::IVec2(v) | DataValue::RangeI32(v) => v.serialize(serializer),
                DataValue::IVec3(v) => v.serialize(serializer),
                DataValue::UVec2(v) => v.serialize(serializer),
                DataValue::UVec3(v) => v.serialize(serializer),
                DataValue::Duration(v) => v.serialize(serializer),
                DataValue::Dto(_)
                | DataValue::Enum { .. }
                | DataValue::Option(_)
                | DataValue::Array(_)
                | DataValue::Set(_)
                | DataValue::Map(_)
                | DataValue::Curve(_)
                | DataValue::Gradient(_, _)
                | DataValue::Material(_)
                | DataValue::Texture(_)
                | DataValue::LightEnvironment(_)
                | DataValue::GameAsset(_)
                | DataValue::SoftRef(_) => Err(serde::ser::Error::custom(
                    "Composite or asset-reference DataValue must be lowered to its WireValue variant before encoding",
                )),
            },
            WireValue::Dto(fields) => {
                let mut tup = serializer.serialize_tuple(fields.len())?;
                for f in fields {
                    tup.serialize_element(f)?;
                }
                tup.end()
            }
            WireValue::Enum(variant_idx, fields) => match fields.as_slice() {
                [] => serializer.serialize_unit_variant("Enum", *variant_idx, ""),
                [single] => serializer.serialize_newtype_variant("Enum", *variant_idx, "", single),
                many => {
                    let mut tup = serializer.serialize_tuple_variant("Enum", *variant_idx, "", many.len())?;
                    for f in many {
                        tup.serialize_field(f)?;
                    }
                    tup.end()
                }
            },
            WireValue::Option(None) => serializer.serialize_none(),
            WireValue::Option(Some(inner)) => serializer.serialize_some(inner.as_ref()),
            WireValue::Seq(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            WireValue::Map(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (k, v) in entries {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
            WireValue::Curve(v) => v.serialize(serializer),
            WireValue::Gradient(v) => v.serialize(serializer),
            WireValue::Material(v) => v.serialize(serializer),
            WireValue::Texture(v) => v.serialize(serializer),
            WireValue::LightEnvironment(v) => v.serialize(serializer),
            WireValue::GameAsset(v) => v.serialize(serializer),
            WireValue::SoftRef(v) => v.serialize(serializer),
        }
    }
}

struct WireDtoRow<'a> {
    fields: Vec<WireValue<'a>>,
}

impl Serialize for WireDtoRow<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut tup = serializer.serialize_tuple(self.fields.len())?;
        for f in &self.fields {
            tup.serialize_element(f)?;
        }
        tup.end()
    }
}

#[allow(clippy::too_many_arguments)]
fn build_wire_value<'a>(
    row_idx: usize,
    field_path: &str,
    ty: &FieldType,
    val: &'a DataValue,
    dtos: &'a BTreeMap<String, DtoAsset>,
    enums: &'a BTreeMap<String, EnumAsset>,
    resolver: &DataBakeResolver<'_>,
    visited: &mut BTreeSet<AssetUuid>,
    depth: usize,
) -> Result<WireValue<'a>, String> {
    if !ty.matches(val) {
        return Err(format!(
            "row #{row_idx} field `{field_path}` has mismatched type `{}` (expected `{}`)",
            val.field_type().label(),
            ty.label()
        ));
    }
    if let FieldType::Dto(target_uuid) = ty {
        if depth >= MAX_NESTING_DEPTH {
            return Err(format!(
                "row #{row_idx} field `{field_path}` exceeds maximum nesting depth of {MAX_NESTING_DEPTH}"
            ));
        }
        let Some((_, target_dto)) = dtos.iter().find(|(_, d)| !d.id.is_nil() && d.id == *target_uuid) else {
            return Err(format!("row #{row_idx} field `{field_path}` references unknown DTO `{target_uuid}`"));
        };
        if !visited.insert(*target_uuid) {
            return Err(format!("row #{row_idx} field `{field_path}` has cyclic DTO reference `{target_uuid}`"));
        }
        let DataValue::Dto(map) = val else {
            visited.remove(target_uuid);
            return Err(format!("row #{row_idx} field `{field_path}` expected DataValue::Dto"));
        };
        let mut nested_fields = Vec::with_capacity(target_dto.fields.len());
        for t_field in &target_dto.fields {
            let sub_val = map.get(&t_field.id).unwrap_or(&t_field.default);
            let sub_path = format!("{field_path}.{}", t_field.name);
            match build_wire_value(row_idx, &sub_path, &t_field.ty, sub_val, dtos, enums, resolver, visited, depth + 1)
            {
                Ok(wv) => nested_fields.push(wv),
                Err(e) => {
                    visited.remove(target_uuid);
                    return Err(e);
                }
            }
        }
        visited.remove(target_uuid);
        Ok(WireValue::Dto(nested_fields))
    } else if let FieldType::Enum(target_uuid) = ty {
        if depth >= MAX_NESTING_DEPTH {
            return Err(format!(
                "row #{row_idx} field `{field_path}` exceeds maximum nesting depth of {MAX_NESTING_DEPTH}"
            ));
        }
        let Some((_, target_enum)) = enums.iter().find(|(_, e)| !e.id.is_nil() && e.id == *target_uuid) else {
            return Err(format!("row #{row_idx} field `{field_path}` references unknown enum `{target_uuid}`"));
        };
        if !visited.insert(*target_uuid) {
            return Err(format!("row #{row_idx} field `{field_path}` has cyclic enum reference `{target_uuid}`"));
        }
        let DataValue::Enum { variant, fields } = val else {
            visited.remove(target_uuid);
            return Err(format!("row #{row_idx} field `{field_path}` expected DataValue::Enum"));
        };
        if target_enum.variants.is_empty() {
            visited.remove(target_uuid);
            return Ok(WireValue::Enum(0, Vec::new()));
        }
        let variant_idx = target_enum.variants.iter().position(|v| v.id == *variant).unwrap_or(0);
        let target_variant = &target_enum.variants[variant_idx];
        let mut wire_fields = Vec::with_capacity(target_variant.fields.len());
        for vf in &target_variant.fields {
            let sub_val = fields.get(&vf.id).unwrap_or(&vf.default);
            let sub_path = format!("{field_path}.{}.{}", target_variant.name, vf.name);
            match build_wire_value(row_idx, &sub_path, &vf.ty, sub_val, dtos, enums, resolver, visited, depth + 1) {
                Ok(wv) => wire_fields.push(wv),
                Err(e) => {
                    visited.remove(target_uuid);
                    return Err(e);
                }
            }
        }
        visited.remove(target_uuid);
        Ok(WireValue::Enum(variant_idx as u32, wire_fields))
    } else if let FieldType::Option(inner_ty) = ty {
        if depth >= MAX_NESTING_DEPTH {
            return Err(format!(
                "row #{row_idx} field `{field_path}` exceeds maximum nesting depth of {MAX_NESTING_DEPTH}"
            ));
        }
        let DataValue::Option(opt) = val else {
            return Err(format!("row #{row_idx} field `{field_path}` expected DataValue::Option"));
        };
        match opt {
            None => Ok(WireValue::Option(None)),
            Some(inner_val) => {
                let mut inner_visited = BTreeSet::new();
                let wv = build_wire_value(
                    row_idx,
                    field_path,
                    inner_ty,
                    inner_val,
                    dtos,
                    enums,
                    resolver,
                    &mut inner_visited,
                    depth + 1,
                )?;
                Ok(WireValue::Option(Some(Box::new(wv))))
            }
        }
    } else if let FieldType::Array(inner_ty) = ty {
        if depth >= MAX_NESTING_DEPTH {
            return Err(format!(
                "row #{row_idx} field `{field_path}` exceeds maximum nesting depth of {MAX_NESTING_DEPTH}"
            ));
        }
        let DataValue::Array(items) = val else {
            return Err(format!("row #{row_idx} field `{field_path}` expected DataValue::Array"));
        };
        let mut out = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let sub_path = format!("{field_path}[{i}]");
            let mut inner_visited = BTreeSet::new();
            let wv = build_wire_value(
                row_idx,
                &sub_path,
                inner_ty,
                item,
                dtos,
                enums,
                resolver,
                &mut inner_visited,
                depth + 1,
            )?;
            out.push(wv);
        }
        Ok(WireValue::Seq(out))
    } else if let FieldType::Set(key_ty) = ty {
        if depth >= MAX_NESTING_DEPTH {
            return Err(format!(
                "row #{row_idx} field `{field_path}` exceeds maximum nesting depth of {MAX_NESTING_DEPTH}"
            ));
        }
        let DataValue::Set(items) = val else {
            return Err(format!("row #{row_idx} field `{field_path}` expected DataValue::Set"));
        };
        let enum_target = key_ty.enum_target();
        let mut sorted: Vec<&'a DataValue> = items.iter().collect();
        sorted.sort_by(|a, b| DataValue::cmp_key(a, b, enums, enum_target));
        sorted.dedup_by(|a, b| DataValue::cmp_key(a, b, enums, enum_target).is_eq());
        let mut out = Vec::with_capacity(sorted.len());
        for (i, item) in sorted.into_iter().enumerate() {
            let sub_path = format!("{field_path}[{i}]");
            let mut inner_visited = BTreeSet::new();
            let wv = build_wire_value(
                row_idx,
                &sub_path,
                key_ty,
                item,
                dtos,
                enums,
                resolver,
                &mut inner_visited,
                depth + 1,
            )?;
            out.push(wv);
        }
        Ok(WireValue::Seq(out))
    } else if let FieldType::Map(key_ty, val_ty) = ty {
        if depth >= MAX_NESTING_DEPTH {
            return Err(format!(
                "row #{row_idx} field `{field_path}` exceeds maximum nesting depth of {MAX_NESTING_DEPTH}"
            ));
        }
        let DataValue::Map(entries) = val else {
            return Err(format!("row #{row_idx} field `{field_path}` expected DataValue::Map"));
        };
        let enum_target = key_ty.enum_target();
        let mut sorted: Vec<(&'a DataValue, &'a DataValue)> = entries.iter().map(|(k, v)| (k, v)).collect();
        sorted.sort_by(|(ka, _), (kb, _)| DataValue::cmp_key(ka, kb, enums, enum_target));
        sorted.dedup_by(|(ka, _), (kb, _)| DataValue::cmp_key(ka, kb, enums, enum_target).is_eq());
        let mut out = Vec::with_capacity(sorted.len());
        for (i, (k, v)) in sorted.into_iter().enumerate() {
            let key_path = format!("{field_path}[{i}].key");
            let val_path = format!("{field_path}[{i}].val");
            let mut key_visited = BTreeSet::new();
            let kwv =
                build_wire_value(row_idx, &key_path, key_ty, k, dtos, enums, resolver, &mut key_visited, depth + 1)?;
            let mut val_visited = BTreeSet::new();
            let vwv =
                build_wire_value(row_idx, &val_path, val_ty, v, dtos, enums, resolver, &mut val_visited, depth + 1)?;
            out.push((kwv, vwv));
        }
        Ok(WireValue::Map(out))
    } else {
        Ok(match val {
            DataValue::Curve(uuid) => WireValue::Curve(resolver.resolve_curve(*uuid)),
            DataValue::Gradient(uuid, slot) => WireValue::Gradient(resolver.resolve_gradient(*uuid, *slot)),
            DataValue::Material(uuid) => WireValue::Material(resolver.resolve_material(*uuid)),
            DataValue::Texture(uuid) => WireValue::Texture(resolver.resolve_texture(*uuid)),
            DataValue::LightEnvironment(uuid) => WireValue::LightEnvironment(resolver.resolve_light_env(*uuid)),
            DataValue::GameAsset(path) => WireValue::GameAsset(path.clone()),
            DataValue::SoftRef(row_uuid) => WireValue::SoftRef(if row_uuid.is_nil() { 0 } else { row_uuid.as_u128() }),
            _ => WireValue::Leaf(val),
        })
    }
}

/// Bakes `col` against `dto` into a `.collection.qsd` binary buffer (36-byte header + `postcard` payload), skipping
/// any row indices present in `skipped_rows` (ADR 0049 §6).
pub fn bake_collection(
    col: &CollectionAsset,
    dto: &DtoAsset,
    schema_hash: u64,
    skipped_rows: &BTreeSet<usize>,
) -> Result<Vec<u8>, String> {
    let empty = BTreeMap::new();
    bake_collection_with_dtos(col, dto, &empty, schema_hash, skipped_rows)
}

/// Bakes `col` against `dto` and all project `dtos` (for by-value `FieldType::Dto` fields) into a `.collection.qsd`
/// binary buffer (36-byte header + `postcard` payload), skipping any row indices present in `skipped_rows`.
pub fn bake_collection_with_dtos(
    col: &CollectionAsset,
    dto: &DtoAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    schema_hash: u64,
    skipped_rows: &BTreeSet<usize>,
) -> Result<Vec<u8>, String> {
    bake_collection_with_context(col, dto, dtos, None, None, schema_hash, skipped_rows)
}

/// Bakes `col` against `dto`, all project `dtos`, and all project `enums` into a `.collection.qsd` binary buffer.
pub fn bake_collection_with_schemas(
    col: &CollectionAsset,
    dto: &DtoAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    schema_hash: u64,
    skipped_rows: &BTreeSet<usize>,
) -> Result<Vec<u8>, String> {
    bake_collection_with_schemas_and_context(col, dto, dtos, enums, None, None, schema_hash, skipped_rows)
}

/// Bakes `col` against `dto`, all project `dtos`, and optional `(project, index)` (for resolving cross-domain `Curve`,
/// `Gradient`, `Material`, `Texture`, and `LightEnvironment` references) into a `.collection.qsd` binary buffer.
pub fn bake_collection_with_context(
    col: &CollectionAsset,
    dto: &DtoAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    project: Option<&Project>,
    index: Option<&AssetIndex>,
    schema_hash: u64,
    skipped_rows: &BTreeSet<usize>,
) -> Result<Vec<u8>, String> {
    let empty_enums = BTreeMap::new();
    bake_collection_with_schemas_and_context(col, dto, dtos, &empty_enums, project, index, schema_hash, skipped_rows)
}

/// Bakes `col` against `dto`, all project `dtos` and `enums`, and optional `(project, index)` into a `.collection.qsd`
/// binary buffer.
#[allow(clippy::too_many_arguments)]
pub fn bake_collection_with_schemas_and_context(
    col: &CollectionAsset,
    dto: &DtoAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    project: Option<&Project>,
    index: Option<&AssetIndex>,
    schema_hash: u64,
    skipped_rows: &BTreeSet<usize>,
) -> Result<Vec<u8>, String> {
    let resolver = DataBakeResolver { project, index };
    let mut wire_rows: Vec<(u128, WireDtoRow<'_>)> = Vec::with_capacity(col.rows.len());
    for (row_idx, row) in col.rows.iter().enumerate() {
        if skipped_rows.contains(&row_idx) {
            continue;
        }
        let mut fields = Vec::with_capacity(dto.fields.len());
        let mut visited = BTreeSet::new();
        if !dto.id.is_nil() {
            visited.insert(dto.id);
        }
        for field in &dto.fields {
            let val = row.values.get(&field.id).unwrap_or(&field.default);
            let wv = build_wire_value(row_idx, &field.name, &field.ty, val, dtos, enums, &resolver, &mut visited, 0)?;
            fields.push(wv);
        }
        wire_rows.push((row.id.as_u128(), WireDtoRow { fields }));
    }

    let payload = postcard::to_allocvec(&wire_rows).map_err(|e| format!("postcard encode failed: {e}"))?;
    let mut out = Vec::with_capacity(QSD_HEADER_LEN + payload.len());
    out.extend_from_slice(&QSD_MAGIC);
    out.extend_from_slice(&QSD_VERSION.to_le_bytes());
    out.extend_from_slice(&schema_hash.to_le_bytes());
    out.extend_from_slice(&dto.id.as_u128().to_le_bytes());
    out.extend_from_slice(&payload);
    debug_assert_eq!(out.len() - payload.len(), QSD_HEADER_LEN);
    Ok(out)
}

/// Dynamic deserializer seed for one [`FieldType`].
struct FieldTypeSeed<'a> {
    ty: FieldType,
    dtos: &'a BTreeMap<String, DtoAsset>,
    enums: &'a BTreeMap<String, EnumAsset>,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for FieldTypeSeed<'_> {
    type Value = DataValue;

    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        Ok(match self.ty {
            FieldType::Bool => DataValue::Bool(bool::deserialize(deserializer)?),
            FieldType::I8 => DataValue::I8(i8::deserialize(deserializer)?),
            FieldType::I16 => DataValue::I16(i16::deserialize(deserializer)?),
            FieldType::I32 => DataValue::I32(i32::deserialize(deserializer)?),
            FieldType::I64 => DataValue::I64(i64::deserialize(deserializer)?),
            FieldType::U8 => DataValue::U8(u8::deserialize(deserializer)?),
            FieldType::U16 => DataValue::U16(u16::deserialize(deserializer)?),
            FieldType::U32 => DataValue::U32(u32::deserialize(deserializer)?),
            FieldType::U64 => DataValue::U64(u64::deserialize(deserializer)?),
            FieldType::F32 => DataValue::F32(f32::deserialize(deserializer)?),
            FieldType::F64 => DataValue::F64(f64::deserialize(deserializer)?),
            FieldType::String => DataValue::String(String::deserialize(deserializer)?),
            FieldType::LocalizedString => DataValue::LocalizedString(String::deserialize(deserializer)?),
            FieldType::Vec2 => DataValue::Vec2(<[f32; 2]>::deserialize(deserializer)?),
            FieldType::Vec3 => DataValue::Vec3(<[f32; 3]>::deserialize(deserializer)?),
            FieldType::Vec4 => DataValue::Vec4(<[f32; 4]>::deserialize(deserializer)?),
            FieldType::IVec2 => DataValue::IVec2(<[i32; 2]>::deserialize(deserializer)?),
            FieldType::IVec3 => DataValue::IVec3(<[i32; 3]>::deserialize(deserializer)?),
            FieldType::UVec2 => DataValue::UVec2(<[u32; 2]>::deserialize(deserializer)?),
            FieldType::UVec3 => DataValue::UVec3(<[u32; 3]>::deserialize(deserializer)?),
            FieldType::Quat => DataValue::Quat(<[f32; 4]>::deserialize(deserializer)?),
            FieldType::Color => DataValue::Color(<[f32; 4]>::deserialize(deserializer)?),
            FieldType::Duration => DataValue::Duration(f32::deserialize(deserializer)?),
            FieldType::RangeF32 => DataValue::RangeF32(<[f32; 2]>::deserialize(deserializer)?),
            FieldType::RangeI32 => DataValue::RangeI32(<[i32; 2]>::deserialize(deserializer)?),
            FieldType::Dto(target_uuid) => {
                if self.depth >= MAX_NESTING_DEPTH {
                    return Err(serde::de::Error::custom("nested DTO exceeds MAX_NESTING_DEPTH"));
                }
                let Some((_, target_dto)) = self.dtos.iter().find(|(_, d)| !d.id.is_nil() && d.id == target_uuid)
                else {
                    return Err(serde::de::Error::custom(format!("unknown nested DTO `{target_uuid}`")));
                };
                let map = DtoFieldsSeed {
                    fields: &target_dto.fields,
                    dtos: self.dtos,
                    enums: self.enums,
                    depth: self.depth + 1,
                }
                .deserialize(deserializer)?;
                DataValue::Dto(map)
            }
            FieldType::Enum(target_uuid) => {
                if self.depth >= MAX_NESTING_DEPTH {
                    return Err(serde::de::Error::custom("nested enum exceeds MAX_NESTING_DEPTH"));
                }
                let Some((_, target_enum)) = self.enums.iter().find(|(_, e)| !e.id.is_nil() && e.id == target_uuid)
                else {
                    return Err(serde::de::Error::custom(format!("unknown enum `{target_uuid}`")));
                };
                deserializer.deserialize_enum(
                    "Enum",
                    &[],
                    EnumVariantVisitor { target_enum, dtos: self.dtos, enums: self.enums, depth: self.depth + 1 },
                )?
            }
            FieldType::Option(inner_ty) => {
                if self.depth >= MAX_NESTING_DEPTH {
                    return Err(serde::de::Error::custom("nested Option exceeds MAX_NESTING_DEPTH"));
                }
                deserializer.deserialize_option(OptionVisitor {
                    inner_ty: *inner_ty,
                    dtos: self.dtos,
                    enums: self.enums,
                    depth: self.depth + 1,
                })?
            }
            FieldType::Array(inner_ty) => {
                if self.depth >= MAX_NESTING_DEPTH {
                    return Err(serde::de::Error::custom("nested Array exceeds MAX_NESTING_DEPTH"));
                }
                let items = deserializer.deserialize_seq(SeqValueVisitor {
                    inner_ty: *inner_ty,
                    dtos: self.dtos,
                    enums: self.enums,
                    depth: self.depth + 1,
                })?;
                DataValue::Array(items)
            }
            FieldType::Set(key_ty) => {
                if self.depth >= MAX_NESTING_DEPTH {
                    return Err(serde::de::Error::custom("nested Set exceeds MAX_NESTING_DEPTH"));
                }
                let items = deserializer.deserialize_seq(SeqValueVisitor {
                    inner_ty: *key_ty,
                    dtos: self.dtos,
                    enums: self.enums,
                    depth: self.depth + 1,
                })?;
                DataValue::Set(items)
            }
            FieldType::Map(key_ty, val_ty) => {
                if self.depth >= MAX_NESTING_DEPTH {
                    return Err(serde::de::Error::custom("nested Map exceeds MAX_NESTING_DEPTH"));
                }
                let entries = deserializer.deserialize_map(MapValueVisitor {
                    key_ty: *key_ty,
                    val_ty: *val_ty,
                    dtos: self.dtos,
                    enums: self.enums,
                    depth: self.depth + 1,
                })?;
                DataValue::Map(entries)
            }
            FieldType::Curve => {
                let _ = CurveData::deserialize(deserializer)?;
                DataValue::Curve(AssetUuid::NIL)
            }
            FieldType::Gradient => {
                let _ = RawGradient::deserialize(deserializer)?;
                DataValue::Gradient(AssetUuid::NIL, 0)
            }
            FieldType::Material => {
                let raw = u128::deserialize(deserializer)?;
                DataValue::Material(AssetUuid::from_u128(raw))
            }
            FieldType::Texture => {
                let (raw, _path) = <(u128, String)>::deserialize(deserializer)?;
                DataValue::Texture(AssetUuid::from_u128(raw))
            }
            FieldType::LightEnvironment => {
                let _ = String::deserialize(deserializer)?;
                DataValue::LightEnvironment(AssetUuid::NIL)
            }
            FieldType::GameAsset(_) => DataValue::GameAsset(String::deserialize(deserializer)?),
            FieldType::SoftRef(_) => {
                let raw = u128::deserialize(deserializer)?;
                DataValue::SoftRef(AssetUuid::from_u128(raw))
            }
        })
    }
}

struct OptionVisitor<'a> {
    inner_ty: FieldType,
    dtos: &'a BTreeMap<String, DtoAsset>,
    enums: &'a BTreeMap<String, EnumAsset>,
    depth: usize,
}

impl<'de> Visitor<'de> for OptionVisitor<'_> {
    type Value = DataValue;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a wire Option")
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(DataValue::Option(None))
    }

    fn visit_some<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        let val = FieldTypeSeed { ty: self.inner_ty, dtos: self.dtos, enums: self.enums, depth: self.depth }
            .deserialize(deserializer)?;
        Ok(DataValue::Option(Some(Box::new(val))))
    }
}

struct SeqValueVisitor<'a> {
    inner_ty: FieldType,
    dtos: &'a BTreeMap<String, DtoAsset>,
    enums: &'a BTreeMap<String, EnumAsset>,
    depth: usize,
}

impl<'de> Visitor<'de> for SeqValueVisitor<'_> {
    type Value = Vec<DataValue>;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a wire sequence")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(val) = seq.next_element_seed(FieldTypeSeed {
            ty: self.inner_ty.clone(),
            dtos: self.dtos,
            enums: self.enums,
            depth: self.depth,
        })? {
            out.push(val);
        }
        Ok(out)
    }
}

struct MapValueVisitor<'a> {
    key_ty: FieldType,
    val_ty: FieldType,
    dtos: &'a BTreeMap<String, DtoAsset>,
    enums: &'a BTreeMap<String, EnumAsset>,
    depth: usize,
}

impl<'de> Visitor<'de> for MapValueVisitor<'_> {
    type Value = Vec<(DataValue, DataValue)>;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a wire map")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut out = Vec::with_capacity(map.size_hint().unwrap_or(0));
        while let Some((k, v)) = map.next_entry_seed(
            FieldTypeSeed { ty: self.key_ty.clone(), dtos: self.dtos, enums: self.enums, depth: self.depth },
            FieldTypeSeed { ty: self.val_ty.clone(), dtos: self.dtos, enums: self.enums, depth: self.depth },
        )? {
            out.push((k, v));
        }
        Ok(out)
    }
}

struct EnumVariantVisitor<'a> {
    target_enum: &'a EnumAsset,
    dtos: &'a BTreeMap<String, DtoAsset>,
    enums: &'a BTreeMap<String, EnumAsset>,
    depth: usize,
}

impl<'de> Visitor<'de> for EnumVariantVisitor<'_> {
    type Value = DataValue;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a wire enum variant")
    }

    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<Self::Value, A::Error> {
        let (variant_idx, variant_access): (u32, _) = data.variant()?;
        if self.target_enum.variants.is_empty() {
            variant_access.unit_variant()?;
            return Ok(DataValue::Enum { variant: AssetUuid::NIL, fields: BTreeMap::new() });
        }
        let Some(target_variant) = self.target_enum.variants.get(variant_idx as usize) else {
            return Err(serde::de::Error::custom(format!(
                "unknown variant index {variant_idx} for enum `{}`",
                self.target_enum.id
            )));
        };
        let fields = match target_variant.fields.as_slice() {
            [] => {
                variant_access.unit_variant()?;
                BTreeMap::new()
            }
            [single] => {
                let val = variant_access.newtype_variant_seed(FieldTypeSeed {
                    ty: single.ty.clone(),
                    dtos: self.dtos,
                    enums: self.enums,
                    depth: self.depth,
                })?;
                BTreeMap::from([(single.id, val)])
            }
            many => variant_access.tuple_variant(
                many.len(),
                FieldsVisitor { fields: many, dtos: self.dtos, enums: self.enums, depth: self.depth },
            )?,
        };
        Ok(DataValue::Enum { variant: target_variant.id, fields })
    }
}

/// A decoded row `(row_id, field_values)` returned by [`decode_collection_rows`].
pub type DecodedRow = (AssetUuid, BTreeMap<AssetUuid, DataValue>);

struct RowSeed<'a> {
    dto: &'a DtoAsset,
    dtos: &'a BTreeMap<String, DtoAsset>,
    enums: &'a BTreeMap<String, EnumAsset>,
}

impl<'de> DeserializeSeed<'de> for RowSeed<'_> {
    type Value = DecodedRow;

    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        struct RowVisitor<'a> {
            dto: &'a DtoAsset,
            dtos: &'a BTreeMap<String, DtoAsset>,
            enums: &'a BTreeMap<String, EnumAsset>,
        }
        impl<'de> Visitor<'de> for RowVisitor<'_> {
            type Value = DecodedRow;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a (row_id, wire_dto) tuple")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let raw_id: u128 = seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
                let values = seq
                    .next_element_seed(DtoFieldsSeed {
                        fields: &self.dto.fields,
                        dtos: self.dtos,
                        enums: self.enums,
                        depth: 0,
                    })?
                    .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
                Ok((AssetUuid::from_u128(raw_id), values))
            }
        }
        deserializer.deserialize_tuple(2, RowVisitor { dto: self.dto, dtos: self.dtos, enums: self.enums })
    }
}

struct DtoFieldsSeed<'a> {
    fields: &'a [Field],
    dtos: &'a BTreeMap<String, DtoAsset>,
    enums: &'a BTreeMap<String, EnumAsset>,
    depth: usize,
}

struct FieldsVisitor<'a> {
    fields: &'a [Field],
    dtos: &'a BTreeMap<String, DtoAsset>,
    enums: &'a BTreeMap<String, EnumAsset>,
    depth: usize,
}

impl<'de> Visitor<'de> for FieldsVisitor<'_> {
    type Value = BTreeMap<AssetUuid, DataValue>;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("wire fields tuple")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut map = BTreeMap::new();
        for (i, field) in self.fields.iter().enumerate() {
            let val = seq
                .next_element_seed(FieldTypeSeed {
                    ty: field.ty.clone(),
                    dtos: self.dtos,
                    enums: self.enums,
                    depth: self.depth,
                })?
                .ok_or_else(|| serde::de::Error::invalid_length(i, &self))?;
            map.insert(field.id, val);
        }
        Ok(map)
    }
}

impl<'de> DeserializeSeed<'de> for DtoFieldsSeed<'_> {
    type Value = BTreeMap<AssetUuid, DataValue>;

    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_tuple(
            self.fields.len(),
            FieldsVisitor { fields: self.fields, dtos: self.dtos, enums: self.enums, depth: self.depth },
        )
    }
}

struct RowsVecSeed<'a> {
    dto: &'a DtoAsset,
    dtos: &'a BTreeMap<String, DtoAsset>,
    enums: &'a BTreeMap<String, EnumAsset>,
}

impl<'de> DeserializeSeed<'de> for RowsVecSeed<'_> {
    type Value = Vec<DecodedRow>;

    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        struct VecVisitor<'a> {
            dto: &'a DtoAsset,
            dtos: &'a BTreeMap<String, DtoAsset>,
            enums: &'a BTreeMap<String, EnumAsset>,
        }
        impl<'de> Visitor<'de> for VecVisitor<'_> {
            type Value = Vec<DecodedRow>;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a sequence of wire rows")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some(row) =
                    seq.next_element_seed(RowSeed { dto: self.dto, dtos: self.dtos, enums: self.enums })?
                {
                    out.push(row);
                }
                Ok(out)
            }
        }
        deserializer.deserialize_seq(VecVisitor { dto: self.dto, dtos: self.dtos, enums: self.enums })
    }
}

/// Verifies the 36-byte `.qsd` header against `expected_schema_hash` and `dto.id`, then decodes all rows using `dto`'s
/// field schema without panicking on corrupted inputs.
pub fn decode_collection_rows(
    bytes: &[u8],
    dto: &DtoAsset,
    expected_schema_hash: u64,
) -> Result<Vec<DecodedRow>, QsdError> {
    let empty = BTreeMap::new();
    decode_collection_rows_with_dtos(bytes, dto, &empty, expected_schema_hash)
}

/// Like [`decode_collection_rows`], resolving nested `FieldType::Dto` fields via `dtos`.
pub fn decode_collection_rows_with_dtos(
    bytes: &[u8],
    dto: &DtoAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    expected_schema_hash: u64,
) -> Result<Vec<DecodedRow>, QsdError> {
    let empty_enums = BTreeMap::new();
    decode_collection_rows_with_schemas(bytes, dto, dtos, &empty_enums, expected_schema_hash)
}

/// Like [`decode_collection_rows`], resolving nested `FieldType::Dto` and `FieldType::Enum` fields via `dtos` and
/// `enums`.
pub fn decode_collection_rows_with_schemas(
    bytes: &[u8],
    dto: &DtoAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    expected_schema_hash: u64,
) -> Result<Vec<DecodedRow>, QsdError> {
    let (_header, payload) = verify_qsd_header(bytes, expected_schema_hash, Some(dto.id.as_u128()))?;
    let mut de = postcard::Deserializer::from_bytes(payload);
    RowsVecSeed { dto, dtos, enums }.deserialize(&mut de).map_err(|e| QsdError::Decode(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::model::{COLLECTION_FORMAT_VERSION, DTO_FORMAT_VERSION, Field, FieldSettings, Row};

    fn test_uuid(n: u128) -> AssetUuid {
        AssetUuid::from_u128(0x0196_3a10_0000_7000_8000_0000_0000_0000 | n)
    }

    #[test]
    fn schema_hash_is_invariant_to_soft_metadata_and_sensitive_to_structure() {
        let dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: test_uuid(1),
            doc: "Initial doc".into(),
            fields: vec![
                Field {
                    id: test_uuid(101),
                    name: "title".into(),
                    ty: FieldType::LocalizedString,
                    category: "Identity".into(),
                    is_name: true,
                    default: DataValue::LocalizedString("Default A".into()),
                    settings: FieldSettings { max_len: Some(80), required: true, ..FieldSettings::default() },
                },
                Field {
                    id: test_uuid(102),
                    name: "pages".into(),
                    ty: FieldType::U32,
                    category: "Physical".into(),
                    is_name: false,
                    default: DataValue::U32(100),
                    settings: FieldSettings {
                        min: Some(1.0),
                        max: Some(999.0),
                        slider: true,
                        ..FieldSettings::default()
                    },
                },
            ],
        };
        let mut map = BTreeMap::from([("library/book".to_string(), dto.clone())]);
        let base_hash = compute_schema_hash(&map);

        // Changing doc, category, is_name, default, and settings does NOT change `DATA_SCHEMA_HASH`.
        let mut soft_modified = dto.clone();
        soft_modified.doc = "Updated doc".into();
        soft_modified.fields[0].category = "Renamed Category".into();
        soft_modified.fields[0].is_name = false;
        soft_modified.fields[0].default = DataValue::LocalizedString("Default B".into());
        soft_modified.fields[0].settings.max_len = Some(200);
        soft_modified.fields[1].settings.slider = false;
        map.insert("library/book".to_string(), soft_modified);
        assert_eq!(compute_schema_hash(&map), base_hash);

        // Renaming a field changes `DATA_SCHEMA_HASH`.
        let mut renamed = dto.clone();
        renamed.fields[0].name = "book_title".into();
        map.insert("library/book".to_string(), renamed);
        assert_ne!(compute_schema_hash(&map), base_hash);

        // Changing a field's type changes `DATA_SCHEMA_HASH`.
        let mut retyped = dto.clone();
        retyped.fields[1].ty = FieldType::I32;
        map.insert("library/book".to_string(), retyped);
        assert_ne!(compute_schema_hash(&map), base_hash);

        // Reordering fields changes `DATA_SCHEMA_HASH`.
        let mut reordered = dto.clone();
        reordered.fields.reverse();
        map.insert("library/book".to_string(), reordered);
        assert_ne!(compute_schema_hash(&map), base_hash);

        // Adding a field changes `DATA_SCHEMA_HASH`.
        let mut added = dto.clone();
        added.fields.push(Field { id: test_uuid(103), ..Field::new("extra", FieldType::Bool) });
        map.insert("library/book".to_string(), added);
        assert_ne!(compute_schema_hash(&map), base_hash);
    }

    #[test]
    fn qsd_bake_and_postcard_round_trip_all_24_field_types() {
        #[derive(Debug, PartialEq, serde::Deserialize)]
        struct All24Wire {
            f_bool: bool,
            f_i8: i8,
            f_i16: i16,
            f_i32: i32,
            f_i64: i64,
            f_u8: u8,
            f_u16: u16,
            f_u32: u32,
            f_u64: u64,
            f_f32: f32,
            f_f64: f64,
            f_string: String,
            f_loc: String,
            f_vec2: [f32; 2],
            f_vec3: [f32; 3],
            f_vec4: [f32; 4],
            f_ivec2: [i32; 2],
            f_ivec3: [i32; 3],
            f_uvec2: [u32; 2],
            f_uvec3: [u32; 3],
            f_quat: [f32; 4],
            f_color: [f32; 4],
            f_duration: f32,
            f_range_f32: [f32; 2],
            f_range_i32: [i32; 2],
        }

        let sample_values: [(FieldType, DataValue); 25] = [
            (FieldType::Bool, DataValue::Bool(true)),
            (FieldType::I8, DataValue::I8(-12)),
            (FieldType::I16, DataValue::I16(-1234)),
            (FieldType::I32, DataValue::I32(-123_456)),
            (FieldType::I64, DataValue::I64(-9_876_543_210)),
            (FieldType::U8, DataValue::U8(200)),
            (FieldType::U16, DataValue::U16(54321)),
            (FieldType::U32, DataValue::U32(1_234_567)),
            (FieldType::U64, DataValue::U64(9_876_543_210)),
            (FieldType::F32, DataValue::F32(3.25)),
            (FieldType::F64, DataValue::F64(6.5)),
            (FieldType::String, DataValue::String("A. Vance".into())),
            (FieldType::LocalizedString, DataValue::LocalizedString("The Shader Alchemist".into())),
            (FieldType::Vec2, DataValue::Vec2([1.0, 2.0])),
            (FieldType::Vec3, DataValue::Vec3([14.8, 21.0, 2.8])),
            (FieldType::Vec4, DataValue::Vec4([1.0, 2.0, 3.0, 4.0])),
            (FieldType::IVec2, DataValue::IVec2([-5, 10])),
            (FieldType::IVec3, DataValue::IVec3([-1, 0, 1])),
            (FieldType::UVec2, DataValue::UVec2([8, 16])),
            (FieldType::UVec3, DataValue::UVec3([32, 64, 128])),
            (FieldType::Quat, DataValue::Quat([0.0, 0.5, 0.0, 0.866])),
            (FieldType::Color, DataValue::Color([0.55, 0.14, 0.12, 1.0])),
            (FieldType::Duration, DataValue::Duration(120.5)),
            (FieldType::RangeF32, DataValue::RangeF32([0.5, 2.5])),
            (FieldType::RangeI32, DataValue::RangeI32([1, 5])),
        ];

        let mut dto =
            DtoAsset { format_version: DTO_FORMAT_VERSION, id: test_uuid(1), doc: String::new(), fields: Vec::new() };
        let mut row_values = BTreeMap::new();
        for (i, (ty, val)) in sample_values.iter().enumerate() {
            let fid = test_uuid(100 + i as u128);
            dto.fields.push(Field {
                id: fid,
                name: format!("f_{i}"),
                ty: ty.clone(),
                category: String::new(),
                is_name: i == 12,
                default: ty.default_value(),
                settings: FieldSettings::default(),
            });
            row_values.insert(fid, val.clone());
        }

        let row0_id = test_uuid(501);
        let row1_skipped_id = test_uuid(502);
        let col = CollectionAsset {
            format_version: COLLECTION_FORMAT_VERSION,
            id: test_uuid(10),
            dto: dto.id,
            rows: vec![
                Row { id: row0_id, values: row_values.clone() },
                Row { id: row1_skipped_id, values: row_values.clone() },
            ],
        };

        let dtos = BTreeMap::from([("library/all_types".to_string(), dto.clone())]);
        let schema_hash = compute_schema_hash(&dtos);
        let skipped = BTreeSet::from([1usize]);
        let bytes = bake_collection(&col, &dto, schema_hash, &skipped).unwrap();

        // Check 36-byte header.
        assert!(bytes.len() > QSD_HEADER_LEN);
        let (hdr, payload) = verify_qsd_header(&bytes, schema_hash, Some(dto.id.as_u128())).unwrap();
        assert_eq!(hdr.version, QSD_VERSION);
        assert_eq!(hdr.schema_hash, schema_hash);
        assert_eq!(hdr.dto_uuid, dto.id.as_u128());

        // 1. Dynamic decode round-trips row 0 and skips row 1.
        let decoded = decode_collection_rows(&bytes, &dto, schema_hash).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].0, row0_id);
        assert_eq!(decoded[0].1, row_values);

        // 2. Generated struct `All24Wire` decodes the exact same postcard payload bytes!
        let typed_rows: Vec<(u128, All24Wire)> = postcard::from_bytes(payload).unwrap();
        assert_eq!(typed_rows.len(), 1);
        assert_eq!(typed_rows[0].0, row0_id.as_u128());
        assert_eq!(
            typed_rows[0].1,
            All24Wire {
                f_bool: true,
                f_i8: -12,
                f_i16: -1234,
                f_i32: -123_456,
                f_i64: -9_876_543_210,
                f_u8: 200,
                f_u16: 54321,
                f_u32: 1_234_567,
                f_u64: 9_876_543_210,
                f_f32: 3.25,
                f_f64: 6.5,
                f_string: "A. Vance".into(),
                f_loc: "The Shader Alchemist".into(),
                f_vec2: [1.0, 2.0],
                f_vec3: [14.8, 21.0, 2.8],
                f_vec4: [1.0, 2.0, 3.0, 4.0],
                f_ivec2: [-5, 10],
                f_ivec3: [-1, 0, 1],
                f_uvec2: [8, 16],
                f_uvec3: [32, 64, 128],
                f_quat: [0.0, 0.5, 0.0, 0.866],
                f_color: [0.55, 0.14, 0.12, 1.0],
                f_duration: 120.5,
                f_range_f32: [0.5, 2.5],
                f_range_i32: [1, 5],
            }
        );

        // 3. Corrupting header fields returns structured errors without panicking.
        assert_eq!(parse_qsd_header(&bytes[..10]), Err(QsdError::TooShort(10)));
        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'X';
        assert_eq!(parse_qsd_header(&bad_magic), Err(QsdError::BadMagic));

        let mut bad_ver = bytes.clone();
        bad_ver[8..12].copy_from_slice(&99u32.to_le_bytes());
        assert_eq!(parse_qsd_header(&bad_ver), Err(QsdError::UnsupportedVersion { found: 99 }));

        let mut bad_hash = bytes.clone();
        bad_hash[12..20].copy_from_slice(&0xdead_beef_u64.to_le_bytes());
        assert_eq!(
            decode_collection_rows(&bad_hash, &dto, schema_hash),
            Err(QsdError::SchemaHashMismatch { expected: schema_hash, found: 0xdead_beef })
        );
    }

    #[test]
    fn qsd_bake_and_decode_nested_dto_hierarchy() {
        #[derive(Debug, PartialEq, serde::Deserialize)]
        struct SectorCoordsWire([i32; 3]);
        #[derive(Debug, PartialEq, serde::Deserialize)]
        struct PlanetWire(String, SectorCoordsWire);
        #[derive(Debug, PartialEq, serde::Deserialize)]
        struct AuthorWire(String, PlanetWire);
        #[derive(Debug, PartialEq, serde::Deserialize)]
        struct BookWire(String, AuthorWire);

        let coords_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: test_uuid(10),
            doc: String::new(),
            fields: vec![Field {
                id: test_uuid(11),
                name: "sector".into(),
                ty: FieldType::IVec3,
                category: String::new(),
                is_name: true,
                default: DataValue::IVec3([7, 8, 9]),
                settings: FieldSettings::default(),
            }],
        };
        let planet_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: test_uuid(20),
            doc: String::new(),
            fields: vec![
                Field {
                    id: test_uuid(21),
                    name: "name".into(),
                    ty: FieldType::String,
                    category: String::new(),
                    is_name: true,
                    default: DataValue::String("Arrakis".into()),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: test_uuid(22),
                    name: "coords".into(),
                    ty: FieldType::Dto(coords_dto.id),
                    category: String::new(),
                    is_name: true,
                    default: DataValue::Dto(BTreeMap::new()),
                    settings: FieldSettings::default(),
                },
            ],
        };
        let author_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: test_uuid(30),
            doc: String::new(),
            fields: vec![
                Field {
                    id: test_uuid(31),
                    name: "name".into(),
                    ty: FieldType::String,
                    category: String::new(),
                    is_name: true,
                    default: DataValue::String("A. Vance".into()),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: test_uuid(32),
                    name: "homeworld".into(),
                    ty: FieldType::Dto(planet_dto.id),
                    category: String::new(),
                    is_name: true,
                    default: DataValue::Dto(BTreeMap::new()),
                    settings: FieldSettings::default(),
                },
            ],
        };
        let book_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: test_uuid(40),
            doc: String::new(),
            fields: vec![
                Field {
                    id: test_uuid(41),
                    name: "title".into(),
                    ty: FieldType::LocalizedString,
                    category: String::new(),
                    is_name: true,
                    default: DataValue::LocalizedString("The Shader Alchemist".into()),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: test_uuid(42),
                    name: "author".into(),
                    ty: FieldType::Dto(author_dto.id),
                    category: String::new(),
                    is_name: true,
                    default: DataValue::Dto(BTreeMap::new()),
                    settings: FieldSettings::default(),
                },
            ],
        };

        let mut dtos = BTreeMap::new();
        dtos.insert("sector_coords".into(), coords_dto);
        dtos.insert("planet".into(), planet_dto);
        dtos.insert("author".into(), author_dto);
        dtos.insert("book".into(), book_dto.clone());

        let base_book_hash = compute_schema_hash_with_dtos(&book_dto, &dtos);
        let base_global_hash = compute_schema_hash(&dtos);

        // Mutating leaf `sector_coords` field type changes `book`'s recursive schema hash!
        let mut dtos_retyped = dtos.clone();
        dtos_retyped.get_mut("sector_coords").unwrap().fields[0].ty = FieldType::Vec3;
        assert_ne!(compute_schema_hash_with_dtos(&book_dto, &dtos_retyped), base_book_hash);
        assert_ne!(compute_schema_hash(&dtos_retyped), base_global_hash);

        // Bake a row where `author.homeworld.coords` is omitted in the row map (should fall back to `[7, 8, 9]`).
        let mut planet_map = BTreeMap::new();
        planet_map.insert(test_uuid(21), DataValue::String("Caladan".into()));
        let mut author_map = BTreeMap::new();
        author_map.insert(test_uuid(31), DataValue::String("F. Herbert".into()));
        author_map.insert(test_uuid(32), DataValue::Dto(planet_map));
        let mut row_vals = BTreeMap::new();
        row_vals.insert(test_uuid(41), DataValue::LocalizedString("Dune".into()));
        row_vals.insert(test_uuid(42), DataValue::Dto(author_map));

        let row_id = test_uuid(500);
        let col = CollectionAsset {
            format_version: COLLECTION_FORMAT_VERSION,
            id: test_uuid(90),
            dto: book_dto.id,
            rows: vec![Row { id: row_id, values: row_vals }],
        };

        let bytes = bake_collection_with_dtos(&col, &book_dto, &dtos, base_global_hash, &BTreeSet::new()).unwrap();
        let (_, payload) = verify_qsd_header(&bytes, base_global_hash, Some(book_dto.id.as_u128())).unwrap();

        // Typed postcard decode matches nested wire structs!
        let typed_rows: Vec<(u128, BookWire)> = postcard::from_bytes(payload).unwrap();
        assert_eq!(
            typed_rows,
            vec![(
                row_id.as_u128(),
                BookWire(
                    "Dune".into(),
                    AuthorWire("F. Herbert".into(), PlanetWire("Caladan".into(), SectorCoordsWire([7, 8, 9]))),
                ),
            )]
        );

        // Dynamic decode also reconstructs the full nested `DataValue::Dto` tree!
        let decoded = decode_collection_rows_with_dtos(&bytes, &book_dto, &dtos, base_global_hash).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].0, row_id);
        assert_eq!(
            book_dto.short_view_string_with_dtos(&decoded[0].1, &dtos),
            "Dune | F. Herbert | Caladan | (7, 8, 9)"
        );
    }

    #[test]
    fn direct_asset_ref_baking_and_schema_hash() {
        use crate::backend::model::GameAssetKind;
        use project_core::asset_index::{DomainIndex, IndexedAsset, SubItem};
        use project_core::layout::{ProjectSettings, save_settings};
        use project_core::{AssetRef, DomainId};

        #[derive(Debug, PartialEq, serde::Deserialize)]
        struct VfxPresetWire(CurveData, RawGradient, u128, (u128, String), String, String);

        let curve_id = test_uuid(10);
        let grad_id = test_uuid(11);
        let mat_id = test_uuid(12);
        let tex_id = test_uuid(13);
        let env_id = test_uuid(14);

        let mat_domain = DomainId::new("material");
        let light_domain = DomainId::new("light");

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
        std::fs::create_dir_all(dir.path().join("assets")).unwrap();
        let project = Project::create(dir.path()).unwrap();
        let mut settings = ProjectSettings::default();
        settings.domains.insert(mat_domain.to_string(), project_core::layout::DomainEntry { enabled: true });
        settings.domains.insert(light_domain.to_string(), project_core::layout::DomainEntry { enabled: true });
        save_settings(&project, &settings).unwrap();

        let mat_dir = dir.path().join("quillshade/material");
        std::fs::create_dir_all(&mat_dir).unwrap();
        std::fs::write(
            mat_dir.join("ease.curve.ron"),
            format!("(format_version: 2, id: \"{curve_id}\", min: -1.0, max: 2.0, points: [(0.0, 0.0), (0.5, 0.5), (1.0, 1.0)])\n"),
        )
        .unwrap();
        std::fs::write(
            mat_dir.join("fire.gradients.ron"),
            format!(
                "(format_version: 2, id: \"{grad_id}\", slots: [(id: 1, name: \"ember\", stops: [(t: 0.0, color: [1.0, 0.2, 0.0, 1.0], mid: 0.4), (t: 1.0, color: [1.0, 0.9, 0.1, 1.0], mid: 0.6)])])\n"
            ),
        )
        .unwrap();
        std::fs::write(
            mat_dir.join("rock.texture.ron"),
            format!("(format_version: 2, id: \"{tex_id}\", path: \"textures/rock_albedo.png\")\n"),
        )
        .unwrap();

        let project = Project::open(dir.path()).unwrap();
        let index = AssetIndex::build(vec![
            DomainIndex {
                domain: mat_domain,
                assets: vec![
                    IndexedAsset {
                        asset: AssetRef { domain: mat_domain, path: "ease".into(), suffix: "curve" },
                        id: Ok(curve_id),
                        sub_items: Vec::new(),
                        refs: Vec::new(),
                    },
                    IndexedAsset {
                        asset: AssetRef { domain: mat_domain, path: "fire".into(), suffix: "gradients" },
                        id: Ok(grad_id),
                        sub_items: vec![SubItem::new(1, "ember")],
                        refs: Vec::new(),
                    },
                    IndexedAsset {
                        asset: AssetRef { domain: mat_domain, path: "rock_mat".into(), suffix: "material" },
                        id: Ok(mat_id),
                        sub_items: Vec::new(),
                        refs: Vec::new(),
                    },
                    IndexedAsset {
                        asset: AssetRef { domain: mat_domain, path: "rock".into(), suffix: "texture" },
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

        let dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: test_uuid(1),
            doc: String::new(),
            fields: vec![
                Field {
                    id: test_uuid(101),
                    name: "falloff".into(),
                    ty: FieldType::Curve,
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Curve(AssetUuid::NIL),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: test_uuid(102),
                    name: "ramp".into(),
                    ty: FieldType::Gradient,
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Gradient(AssetUuid::NIL, 0),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: test_uuid(103),
                    name: "surface".into(),
                    ty: FieldType::Material,
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Material(AssetUuid::NIL),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: test_uuid(104),
                    name: "icon".into(),
                    ty: FieldType::Texture,
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Texture(AssetUuid::NIL),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: test_uuid(105),
                    name: "lighting".into(),
                    ty: FieldType::LightEnvironment,
                    category: String::new(),
                    is_name: false,
                    default: DataValue::LightEnvironment(AssetUuid::NIL),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: test_uuid(106),
                    name: "sfx".into(),
                    ty: FieldType::GameAsset(GameAssetKind::Audio),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::GameAsset(String::new()),
                    settings: FieldSettings::default(),
                },
            ],
        };

        let mut dtos = BTreeMap::new();
        dtos.insert("vfx_preset".into(), dto.clone());
        let schema_hash = compute_schema_hash(&dtos);

        // Changing GameAssetKind filter changes DATA_SCHEMA_HASH!
        let mut dtos_any = dtos.clone();
        dtos_any.get_mut("vfx_preset").unwrap().fields[5].ty = FieldType::GameAsset(GameAssetKind::Any);
        assert_ne!(compute_schema_hash(&dtos_any), schema_hash);

        let mut row0_vals = BTreeMap::new();
        row0_vals.insert(test_uuid(101), DataValue::Curve(curve_id));
        row0_vals.insert(test_uuid(102), DataValue::Gradient(grad_id, 1));
        row0_vals.insert(test_uuid(103), DataValue::Material(mat_id));
        row0_vals.insert(test_uuid(104), DataValue::Texture(tex_id));
        row0_vals.insert(test_uuid(105), DataValue::LightEnvironment(env_id));
        row0_vals.insert(test_uuid(106), DataValue::GameAsset("audio/boom.ogg".into()));

        let col = CollectionAsset {
            format_version: COLLECTION_FORMAT_VERSION,
            id: test_uuid(50),
            dto: dto.id,
            rows: vec![
                Row { id: test_uuid(500), values: row0_vals },
                Row { id: test_uuid(501), values: BTreeMap::new() }, // uses nil/empty defaults
            ],
        };

        let bytes = bake_collection_with_context(
            &col,
            &dto,
            &dtos,
            Some(&project),
            Some(&index),
            schema_hash,
            &BTreeSet::new(),
        )
        .unwrap();
        let (_, payload) = verify_qsd_header(&bytes, schema_hash, Some(dto.id.as_u128())).unwrap();
        let typed_rows: Vec<(u128, VfxPresetWire)> = postcard::from_bytes(payload).unwrap();
        assert_eq!(typed_rows.len(), 2);

        let r0 = &typed_rows[0].1;
        assert_eq!(r0.0.min, -1.0);
        assert_eq!(r0.0.max, 2.0);
        assert_eq!(r0.0.points, vec![(0.0, 0.0), (0.5, 0.5), (1.0, 1.0)]);
        assert!((r0.0.sample(0.5) - 0.5).abs() < 1e-4);
        assert_eq!(r0.1.stops.len(), 2);
        assert_eq!(r0.1.stops[0].color, [1.0, 0.2, 0.0, 1.0]);
        assert_eq!(r0.2, mat_id.as_u128());
        assert_eq!(r0.3, (tex_id.as_u128(), "textures/rock_albedo.png".into()));
        assert_eq!(r0.4, "caves/crystal");
        assert_eq!(r0.5, "audio/boom.ogg");

        // Row 1 uses nil/empty defaults.
        let r1 = &typed_rows[1].1;
        assert_eq!(r1.0, CurveData::default());
        assert_eq!(r1.1, RawGradient::default());
        assert_eq!(r1.2, 0);
        assert_eq!(r1.3, (0, String::new()));
        assert_eq!(r1.4, "");
        assert_eq!(r1.5, "");

        let decoded = decode_collection_rows_with_dtos(&bytes, &dto, &dtos, schema_hash).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].1.get(&test_uuid(103)), Some(&DataValue::Material(mat_id)));
        assert_eq!(decoded[0].1.get(&test_uuid(104)), Some(&DataValue::Texture(tex_id)));
        assert_eq!(decoded[0].1.get(&test_uuid(106)), Some(&DataValue::GameAsset("audio/boom.ogg".into())));
    }

    #[test]
    fn resolve_texture_with_settings_and_sampler_returns_non_empty_path() {
        use project_core::asset_index::{DomainIndex, IndexedAsset};
        use project_core::layout::{ProjectSettings, save_settings};
        use project_core::{AssetRef, DomainId};

        let tex_id = test_uuid(42);
        let mat_domain = DomainId::new("material");

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
        std::fs::create_dir_all(dir.path().join("assets")).unwrap();
        let project = Project::create(dir.path()).unwrap();
        let mut settings = ProjectSettings::default();
        settings.domains.insert(mat_domain.to_string(), project_core::layout::DomainEntry { enabled: true });
        save_settings(&project, &settings).unwrap();

        let tex_dir = dir.path().join("quillshade/material/textures");
        std::fs::create_dir_all(&tex_dir).unwrap();
        std::fs::write(
            tex_dir.join("atlas.texture.ron"),
            format!(
                "TextureAsset(\n  format_version: 2,\n  id: \"{tex_id}\",\n  path: \"textures/atlas.png\",\n  settings: TextureSettings(\n    is_srgb: false,\n    sampler: Some(SamplerSettings()),\n  ),\n)\n"
            ),
        )
        .unwrap();

        let project = Project::open(dir.path()).unwrap();
        let index = AssetIndex::build(vec![DomainIndex {
            domain: mat_domain,
            assets: vec![IndexedAsset {
                asset: AssetRef { domain: mat_domain, path: "textures/atlas".into(), suffix: "texture" },
                id: Ok(tex_id),
                sub_items: Vec::new(),
                refs: Vec::new(),
            }],
        }]);

        let resolver = DataBakeResolver { project: Some(&project), index: Some(&index) };
        assert_eq!(resolver.resolve_texture(tex_id), (tex_id.as_u128(), "textures/atlas.png".to_string()));
    }

    #[test]
    fn bake_and_decode_soft_ref_and_schema_hash() {
        let book_dto_id = test_uuid(201);
        let author_dto_id = test_uuid(202);
        let f_title = test_uuid(203);
        let f_sequel = test_uuid(204);
        let row_0 = test_uuid(210);
        let row_1 = test_uuid(211);

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
                    default: DataValue::SoftRef(AssetUuid::NIL),
                    settings: FieldSettings::default(),
                },
            ],
        };

        let mut dtos = BTreeMap::from([("book".to_string(), book_dto.clone())]);
        let schema_hash = compute_schema_hash(&dtos);

        // Changing target DTO UUID changes `DATA_SCHEMA_HASH`.
        let mut book_dto_other_target = book_dto.clone();
        book_dto_other_target.fields[1].ty = FieldType::SoftRef(author_dto_id);
        let other_hash = compute_schema_hash(&BTreeMap::from([("book".to_string(), book_dto_other_target)]));
        assert_ne!(schema_hash, other_hash);

        // Changing a field on the target DTO does NOT recurse through SoftRef: if `author` is added and its fields
        // change, a `SoftRef(author_dto_id)` field's contribution only depends on `author_dto_id`.
        dtos.insert(
            "author".into(),
            DtoAsset { format_version: DTO_FORMAT_VERSION, id: author_dto_id, doc: String::new(), fields: Vec::new() },
        );

        let col = CollectionAsset {
            format_version: COLLECTION_FORMAT_VERSION,
            id: test_uuid(220),
            dto: book_dto_id,
            rows: vec![
                Row {
                    id: row_0,
                    values: BTreeMap::from([
                        (f_title, DataValue::String("Volume I".into())),
                        (f_sequel, DataValue::SoftRef(row_1)),
                    ]),
                },
                Row {
                    id: row_1,
                    values: BTreeMap::from([
                        (f_title, DataValue::String("Volume II".into())),
                        (f_sequel, DataValue::SoftRef(AssetUuid::NIL)),
                    ]),
                },
            ],
        };

        let bytes = bake_collection_with_dtos(&col, &book_dto, &dtos, schema_hash, &BTreeSet::new()).unwrap();
        let (_, payload) = verify_qsd_header(&bytes, schema_hash, Some(book_dto_id.as_u128())).unwrap();
        let raw_rows: Vec<(u128, (String, u128))> = postcard::from_bytes(payload).unwrap();
        assert_eq!(raw_rows.len(), 2);
        assert_eq!(raw_rows[0], (row_0.as_u128(), ("Volume I".to_string(), row_1.as_u128())));
        assert_eq!(raw_rows[1], (row_1.as_u128(), ("Volume II".to_string(), 0u128)));

        let decoded = decode_collection_rows_with_dtos(&bytes, &book_dto, &dtos, schema_hash).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].0, row_0);
        assert_eq!(decoded[0].1.get(&f_sequel), Some(&DataValue::SoftRef(row_1)));
        assert_eq!(decoded[1].0, row_1);
        assert_eq!(decoded[1].1.get(&f_sequel), Some(&DataValue::SoftRef(AssetUuid::NIL)));
    }

    #[test]
    fn bake_and_decode_enum_variants_and_schema_hash() {
        use crate::backend::model::{ENUM_FORMAT_VERSION, EnumAsset, EnumVariant};

        #[derive(Debug, PartialEq, Deserialize)]
        enum RarityWire {
            Common,
            Rare,
        }

        #[derive(Debug, PartialEq, Deserialize)]
        enum EffectWire {
            None,
            Shield(u32),
            Burn(f32, f32),
        }

        let rarity_id = test_uuid(300);
        let var_common = test_uuid(301);
        let var_rare = test_uuid(302);

        let effect_id = test_uuid(310);
        let var_none = test_uuid(311);
        let var_shield = test_uuid(312);
        let f_shield_hp = test_uuid(313);
        let var_burn = test_uuid(314);
        let f_burn_dps = test_uuid(315);
        let f_burn_dur = test_uuid(316);

        let item_dto_id = test_uuid(320);
        let f_rarity = test_uuid(321);
        let f_effect = test_uuid(322);

        let rarity_enum = EnumAsset {
            format_version: ENUM_FORMAT_VERSION,
            id: rarity_id,
            doc: "Rarity".into(),
            variants: vec![
                EnumVariant { id: var_common, name: "common".into(), doc: String::new(), fields: Vec::new() },
                EnumVariant { id: var_rare, name: "rare".into(), doc: String::new(), fields: Vec::new() },
            ],
        };

        let effect_enum = EnumAsset {
            format_version: ENUM_FORMAT_VERSION,
            id: effect_id,
            doc: "Effect".into(),
            variants: vec![
                EnumVariant { id: var_none, name: "none".into(), doc: String::new(), fields: Vec::new() },
                EnumVariant {
                    id: var_shield,
                    name: "shield".into(),
                    doc: String::new(),
                    fields: vec![Field {
                        id: f_shield_hp,
                        name: "hp".into(),
                        ty: FieldType::U32,
                        category: String::new(),
                        is_name: true,
                        default: DataValue::U32(50),
                        settings: FieldSettings::default(),
                    }],
                },
                EnumVariant {
                    id: var_burn,
                    name: "burn".into(),
                    doc: String::new(),
                    fields: vec![
                        Field {
                            id: f_burn_dps,
                            name: "dps".into(),
                            ty: FieldType::F32,
                            category: String::new(),
                            is_name: true,
                            default: DataValue::F32(10.0),
                            settings: FieldSettings::default(),
                        },
                        Field {
                            id: f_burn_dur,
                            name: "dur".into(),
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

        let item_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: item_dto_id,
            doc: String::new(),
            fields: vec![
                Field {
                    id: f_rarity,
                    name: "rarity".into(),
                    ty: FieldType::Enum(rarity_id),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Enum { variant: var_common, fields: BTreeMap::new() },
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_effect,
                    name: "effect".into(),
                    ty: FieldType::Enum(effect_id),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Enum { variant: var_none, fields: BTreeMap::new() },
                    settings: FieldSettings::default(),
                },
            ],
        };

        let dtos = BTreeMap::from([("item".to_string(), item_dto.clone())]);
        let enums =
            BTreeMap::from([("rarity".to_string(), rarity_enum.clone()), ("effect".to_string(), effect_enum.clone())]);
        let schema_hash = compute_schema_hash_with_enums(&dtos, &enums);

        // Changing soft metadata on an enum (doc, variant doc, payload field default) leaves `DATA_SCHEMA_HASH` unchanged.
        let mut enums_soft = enums.clone();
        enums_soft.get_mut("effect").unwrap().doc = "Edited doc".into();
        enums_soft.get_mut("effect").unwrap().variants[2].doc = "Edited variant doc".into();
        enums_soft.get_mut("effect").unwrap().variants[2].fields[0].default = DataValue::F32(999.0);
        assert_eq!(schema_hash, compute_schema_hash_with_enums(&dtos, &enums_soft));

        // Renaming a variant or changing a variant field type changes `DATA_SCHEMA_HASH`.
        let mut enums_renamed = enums.clone();
        enums_renamed.get_mut("rarity").unwrap().variants[1].name = "epic".into();
        assert_ne!(schema_hash, compute_schema_hash_with_enums(&dtos, &enums_renamed));

        let row0 = test_uuid(350);
        let row1 = test_uuid(351);
        let row2 = test_uuid(352);
        let col = CollectionAsset {
            format_version: COLLECTION_FORMAT_VERSION,
            id: test_uuid(340),
            dto: item_dto_id,
            rows: vec![
                Row {
                    id: row0,
                    values: BTreeMap::from([
                        (f_rarity, DataValue::Enum { variant: var_common, fields: BTreeMap::new() }),
                        (f_effect, DataValue::Enum { variant: var_none, fields: BTreeMap::new() }),
                    ]),
                },
                Row {
                    id: row1,
                    values: BTreeMap::from([
                        (f_rarity, DataValue::Enum { variant: var_rare, fields: BTreeMap::new() }),
                        (
                            f_effect,
                            DataValue::Enum {
                                variant: var_shield,
                                fields: BTreeMap::from([(f_shield_hp, DataValue::U32(120))]),
                            },
                        ),
                    ]),
                },
                Row {
                    id: row2,
                    values: BTreeMap::from([
                        (f_rarity, DataValue::Enum { variant: var_rare, fields: BTreeMap::new() }),
                        (
                            f_effect,
                            DataValue::Enum {
                                variant: var_burn,
                                fields: BTreeMap::from([
                                    (f_burn_dps, DataValue::F32(25.0)),
                                    (f_burn_dur, DataValue::Duration(4.5)),
                                ]),
                            },
                        ),
                    ]),
                },
            ],
        };

        let bytes =
            bake_collection_with_schemas(&col, &item_dto, &dtos, &enums, schema_hash, &BTreeSet::new()).unwrap();
        let (_, payload) = verify_qsd_header(&bytes, schema_hash, Some(item_dto_id.as_u128())).unwrap();
        let wire_rows: Vec<(u128, (RarityWire, EffectWire))> = postcard::from_bytes(payload).unwrap();
        assert_eq!(wire_rows.len(), 3);
        assert_eq!(wire_rows[0], (row0.as_u128(), (RarityWire::Common, EffectWire::None)));
        assert_eq!(wire_rows[1], (row1.as_u128(), (RarityWire::Rare, EffectWire::Shield(120))));
        assert_eq!(wire_rows[2], (row2.as_u128(), (RarityWire::Rare, EffectWire::Burn(25.0, 4.5))));

        let decoded = decode_collection_rows_with_schemas(&bytes, &item_dto, &dtos, &enums, schema_hash).unwrap();
        assert_eq!(decoded.len(), 3);
        assert_eq!(
            decoded[0].1.get(&f_rarity),
            Some(&DataValue::Enum { variant: var_common, fields: BTreeMap::new() })
        );
        assert_eq!(
            decoded[1].1.get(&f_effect),
            Some(&DataValue::Enum {
                variant: var_shield,
                fields: BTreeMap::from([(f_shield_hp, DataValue::U32(120))]),
            })
        );
        assert_eq!(
            decoded[2].1.get(&f_effect),
            Some(&DataValue::Enum {
                variant: var_burn,
                fields: BTreeMap::from([(f_burn_dps, DataValue::F32(25.0)), (f_burn_dur, DataValue::Duration(4.5)),]),
            })
        );
    }

    #[test]
    fn bake_and_decode_option_and_containers_and_schema_hash() {
        use std::collections::{BTreeMap as StdMap, BTreeSet as StdSet};

        let dto_id = test_uuid(500);
        let f_opt = test_uuid(501);
        let f_arr = test_uuid(502);
        let f_set = test_uuid(503);
        let f_map = test_uuid(504);

        let dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: dto_id,
            doc: String::new(),
            fields: vec![
                Field {
                    id: f_opt,
                    name: "opt_score".into(),
                    ty: FieldType::Option(Box::new(FieldType::I32)),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Option(None),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_arr,
                    name: "levels".into(),
                    ty: FieldType::Array(Box::new(FieldType::U32)),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Array(Vec::new()),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_set,
                    name: "tags".into(),
                    ty: FieldType::Set(Box::new(FieldType::String)),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Set(Vec::new()),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_map,
                    name: "stats".into(),
                    ty: FieldType::Map(Box::new(FieldType::String), Box::new(FieldType::I32)),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Map(Vec::new()),
                    settings: FieldSettings::default(),
                },
            ],
        };

        let dtos = BTreeMap::from([("player".to_string(), dto.clone())]);
        let enums = BTreeMap::new();
        let hash0 = compute_schema_hash_with_enums(&dtos, &enums);

        // Changing inner type of Option or value type of Map must change DATA_SCHEMA_HASH.
        let mut dtos_opt_changed = dtos.clone();
        dtos_opt_changed.get_mut("player").unwrap().fields[0].ty = FieldType::Option(Box::new(FieldType::I64));
        assert_ne!(hash0, compute_schema_hash_with_enums(&dtos_opt_changed, &enums));

        let mut dtos_map_changed = dtos.clone();
        dtos_map_changed.get_mut("player").unwrap().fields[3].ty =
            FieldType::Map(Box::new(FieldType::String), Box::new(FieldType::F32));
        assert_ne!(hash0, compute_schema_hash_with_enums(&dtos_map_changed, &enums));

        let row0_id = test_uuid(510);
        let row1_id = test_uuid(511);
        let col = CollectionAsset {
            format_version: COLLECTION_FORMAT_VERSION,
            id: test_uuid(509),
            dto: dto_id,
            rows: vec![
                Row {
                    id: row0_id,
                    values: BTreeMap::from([
                        (f_opt, DataValue::Option(Some(Box::new(DataValue::I32(42))))),
                        (f_arr, DataValue::Array(vec![DataValue::U32(10), DataValue::U32(20)])),
                        (
                            f_set,
                            DataValue::Set(vec![DataValue::String("zeta".into()), DataValue::String("alpha".into())]),
                        ),
                        (
                            f_map,
                            DataValue::Map(vec![
                                (DataValue::String("str".into()), DataValue::I32(18)),
                                (DataValue::String("agi".into()), DataValue::I32(14)),
                            ]),
                        ),
                    ]),
                },
                Row {
                    id: row1_id,
                    values: BTreeMap::from([
                        (f_opt, DataValue::Option(None)),
                        (f_arr, DataValue::Array(Vec::new())),
                        (f_set, DataValue::Set(Vec::new())),
                        (f_map, DataValue::Map(Vec::new())),
                    ]),
                },
            ],
        };

        let bytes = bake_collection_with_schemas(&col, &dto, &dtos, &enums, hash0, &BTreeSet::new()).unwrap();
        let (_, payload) = verify_qsd_header(&bytes, hash0, Some(dto_id.as_u128())).unwrap();
        type PlayerWire = (Option<i32>, Vec<u32>, StdSet<String>, StdMap<String, i32>);
        let wire_rows: Vec<(u128, PlayerWire)> = postcard::from_bytes(payload).unwrap();
        assert_eq!(wire_rows.len(), 2);
        assert_eq!(wire_rows[0].0, row0_id.as_u128());
        assert_eq!(wire_rows[0].1.0, Some(42));
        assert_eq!(wire_rows[0].1.1, vec![10, 20]);
        assert_eq!(wire_rows[0].1.2, StdSet::from(["alpha".to_string(), "zeta".to_string()]));
        assert_eq!(wire_rows[0].1.3, StdMap::from([("agi".to_string(), 14), ("str".to_string(), 18)]));
        assert_eq!(wire_rows[1].1, (None, Vec::new(), StdSet::new(), StdMap::new()));

        let decoded = decode_collection_rows_with_schemas(&bytes, &dto, &dtos, &enums, hash0).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].1.get(&f_opt), Some(&DataValue::Option(Some(Box::new(DataValue::I32(42))))));
        assert_eq!(
            decoded[0].1.get(&f_set),
            Some(&DataValue::Set(vec![DataValue::String("alpha".into()), DataValue::String("zeta".into())]))
        );
        assert_eq!(
            decoded[0].1.get(&f_map),
            Some(&DataValue::Map(vec![
                (DataValue::String("agi".into()), DataValue::I32(14)),
                (DataValue::String("str".into()), DataValue::I32(18)),
            ]))
        );
        assert_eq!(decoded[1].1.get(&f_opt), Some(&DataValue::Option(None)));
    }
}
