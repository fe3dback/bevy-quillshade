//! Data domain models (ADR 0049, techdoc §7.1–§7.5): `.dto.ron` schemas ([`DtoAsset`], [`Field`], [`FieldType`],
//! [`FieldSettings`]), `.enum.ron` schemas ([`EnumAsset`], [`EnumVariant`]), `.collection.ron` tables
//! ([`CollectionAsset`], [`Row`]), [`DataValue`], and the [`ToShortViewString`] summary formatting contract.

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};

use project_core::{AssetRef, AssetUuid};
use serde::{Deserialize, Serialize};

use super::DATA;

/// Current `.dto.ron` file format version.
pub const DTO_FORMAT_VERSION: u32 = 1;

/// Current `.enum.ron` file format version.
pub const ENUM_FORMAT_VERSION: u32 = 1;

/// Current `.collection.ron` file format version.
pub const COLLECTION_FORMAT_VERSION: u32 = 1;

/// Maximum allowed nesting depth for by-value nested DTO/Enum schemas and values (`DE009`, D33).
pub const MAX_NESTING_DEPTH: usize = 64;

/// File suffix of a DTO schema (`<path>.dto.ron`).
pub const DTO_SUFFIX: &str = "dto";

/// File suffix of an enum / tagged union schema (`<path>.enum.ron`).
pub const ENUM_SUFFIX: &str = "enum";

/// File suffix of a data collection (`<path>.collection.ron`).
pub const COLLECTION_SUFFIX: &str = "collection";

/// File suffix of a data curve (`<path>.curve.ron`).
pub const CURVE_SUFFIX: &str = "curve";

/// Builds the [`AssetRef`] of a `.dto.ron` asset at domain-relative `path` (without suffix).
pub fn dto_ref(path: &str) -> AssetRef {
    AssetRef { domain: DATA, path: path.to_string(), suffix: DTO_SUFFIX }
}

/// Builds the [`AssetRef`] of an `.enum.ron` asset at domain-relative `path` (without suffix).
pub fn enum_ref(path: &str) -> AssetRef {
    AssetRef { domain: DATA, path: path.to_string(), suffix: ENUM_SUFFIX }
}

/// Builds the [`AssetRef`] of a `.collection.ron` asset at domain-relative `path` (without suffix).
pub fn collection_ref(path: &str) -> AssetRef {
    AssetRef { domain: DATA, path: path.to_string(), suffix: COLLECTION_SUFFIX }
}

/// Builds the [`AssetRef`] of a `.curve.ron` asset at domain-relative `path` (without suffix).
pub fn curve_ref(path: &str) -> AssetRef {
    AssetRef { domain: DATA, path: path.to_string(), suffix: CURVE_SUFFIX }
}

/// Converts a domain-relative asset path or snake_case identifier (`"book"`, `"library/book_item"`) into `PascalCase`
/// (`"Book"`, `"LibraryBookItem"`).
pub fn to_pascal_case(path: &str) -> String {
    let mut out = String::new();
    for seg in path.split(['/', '_']).filter(|s| !s.is_empty()) {
        let mut chars = seg.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            out.extend(chars);
        }
    }
    out
}

/// Converts a domain-relative asset path (`"library/books_core"`) into `UPPER_SNAKE_CASE` (`"LIBRARY_BOOKS_CORE"`).
pub fn to_upper_snake_case(path: &str) -> String {
    path.split(['/', '_']).filter(|s| !s.is_empty()).map(str::to_ascii_uppercase).collect::<Vec<_>>().join("_")
}

/// Filter category for `FieldType::GameAsset` file paths under `<game_root>/assets/` (D22, ADR 0049 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
pub enum GameAssetKind {
    #[default]
    Any,
    Image,
    Audio,
    Scene,
    Gltf,
    Font,
}

impl GameAssetKind {
    pub const ALL: &'static [GameAssetKind] = &[
        GameAssetKind::Any,
        GameAssetKind::Image,
        GameAssetKind::Audio,
        GameAssetKind::Scene,
        GameAssetKind::Gltf,
        GameAssetKind::Font,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            GameAssetKind::Any => "Any",
            GameAssetKind::Image => "Image",
            GameAssetKind::Audio => "Audio",
            GameAssetKind::Scene => "Scene",
            GameAssetKind::Gltf => "Gltf",
            GameAssetKind::Font => "Font",
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.label() == s)
    }

    pub const fn schema_tag(self) -> u8 {
        match self {
            GameAssetKind::Any => 0,
            GameAssetKind::Image => 1,
            GameAssetKind::Audio => 2,
            GameAssetKind::Scene => 3,
            GameAssetKind::Gltf => 4,
            GameAssetKind::Font => 5,
        }
    }

    pub const fn tag(self) -> usize {
        self.schema_tag() as usize
    }

    pub fn from_tag(tag: usize) -> Option<Self> {
        Self::ALL.get(tag).copied()
    }

    /// Lowercase file extensions (without leading dot) accepted by this filter. Empty slice for `Any` (accepts any file).
    pub const fn extensions(self) -> &'static [&'static str] {
        match self {
            GameAssetKind::Any => &[],
            GameAssetKind::Image => &["png", "jpg", "jpeg", "ktx2", "webp", "bmp", "tga", "dds", "exr", "hdr"],
            GameAssetKind::Audio => &["ogg", "wav", "mp3", "flac"],
            GameAssetKind::Scene => &["scn.ron", "scn", "ron"],
            GameAssetKind::Gltf => &["gltf", "glb"],
            GameAssetKind::Font => &["ttf", "otf"],
        }
    }

    /// Returns `true` if `path` matches the allowed extensions for `self` (case-insensitive).
    pub fn matches_path(self, path: &str) -> bool {
        let exts = self.extensions();
        if exts.is_empty() {
            return true;
        }
        let lower = path.to_ascii_lowercase();
        exts.iter().any(|ext| lower.ends_with(&format!(".{ext}")))
    }
}

/// All field types supported in `.dto.ron` and `.enum.ron` schemas (ADR 0049 §2, Slice 1–5).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
pub enum FieldType {
    #[default]
    Bool,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    String,
    LocalizedString,
    Vec2,
    Vec3,
    Vec4,
    IVec2,
    IVec3,
    UVec2,
    UVec3,
    Quat,
    Color,
    Duration,
    RangeF32,
    RangeI32,
    /// Hard-embedded (by-value) nested DTO referencing a `.dto.ron` asset by its [`AssetUuid`].
    Dto(AssetUuid),
    /// Direct reference to a Material or Light `.curve.ron` asset by [`AssetUuid`].
    Curve,
    /// Direct reference to a Material `.gradients.ron` asset by [`AssetUuid`] and 1-based slot index.
    Gradient,
    /// Direct reference to a Material `.material.ron` asset by [`AssetUuid`].
    Material,
    /// Direct reference to a Material or Light `.texture.ron` asset by [`AssetUuid`].
    Texture,
    /// Direct reference to a Light `.environment.ron` asset by [`AssetUuid`].
    LightEnvironment,
    /// Direct reference to a raw game file under `<game_root>/assets/` filtered by [`GameAssetKind`].
    GameAsset(GameAssetKind),
    /// Typed soft reference to a row in any collection of the target DTO (`AssetUuid` is the target `.dto.ron` UUID).
    SoftRef(AssetUuid),
    /// Hard-embedded (by-value) plain enum or tagged union referencing an `.enum.ron` asset by its [`AssetUuid`].
    Enum(AssetUuid),
    /// Optional value of inner [`FieldType`] (`Option<T>` / `Option<Box<T>>` in generated Rust).
    Option(Box<FieldType>),
    /// Ordered list of inner [`FieldType`] (`Vec<T>` in generated Rust).
    Array(Box<FieldType>),
    /// Unique sorted set of valid key [`FieldType`] (`std::collections::BTreeSet<K>` in generated Rust).
    Set(Box<FieldType>),
    /// Key-value map (`std::collections::BTreeMap<K, V>` in generated Rust).
    Map(Box<FieldType>, Box<FieldType>),
}

impl FieldType {
    /// All 25 non-DTO primitive/scalar/engine field types in canonical declaration order.
    pub const ALL_PRIMITIVES: &'static [FieldType] = &[
        FieldType::Bool,
        FieldType::I8,
        FieldType::I16,
        FieldType::I32,
        FieldType::I64,
        FieldType::U8,
        FieldType::U16,
        FieldType::U32,
        FieldType::U64,
        FieldType::F32,
        FieldType::F64,
        FieldType::String,
        FieldType::LocalizedString,
        FieldType::Vec2,
        FieldType::Vec3,
        FieldType::Vec4,
        FieldType::IVec2,
        FieldType::IVec3,
        FieldType::UVec2,
        FieldType::UVec3,
        FieldType::Quat,
        FieldType::Color,
        FieldType::Duration,
        FieldType::RangeF32,
        FieldType::RangeI32,
    ];

    /// Candidate key types selectable for `Set<K>` and `Map<K, V>` in the schema editor UI.
    pub const ALL_KEY_TYPES: &'static [FieldType] = &[
        FieldType::Bool,
        FieldType::I8,
        FieldType::I16,
        FieldType::I32,
        FieldType::I64,
        FieldType::U8,
        FieldType::U16,
        FieldType::U32,
        FieldType::U64,
        FieldType::String,
        FieldType::LocalizedString,
        FieldType::SoftRef(AssetUuid::NIL),
        FieldType::Enum(AssetUuid::NIL),
    ];

    /// All non-container field types selectable as inner/value types in the schema editor UI.
    pub const ALL_INNER_TYPES: [FieldType; 34] = [
        FieldType::Bool,
        FieldType::I8,
        FieldType::I16,
        FieldType::I32,
        FieldType::I64,
        FieldType::U8,
        FieldType::U16,
        FieldType::U32,
        FieldType::U64,
        FieldType::F32,
        FieldType::F64,
        FieldType::String,
        FieldType::LocalizedString,
        FieldType::Vec2,
        FieldType::Vec3,
        FieldType::Vec4,
        FieldType::IVec2,
        FieldType::IVec3,
        FieldType::UVec2,
        FieldType::UVec3,
        FieldType::Quat,
        FieldType::Color,
        FieldType::Duration,
        FieldType::RangeF32,
        FieldType::RangeI32,
        FieldType::Dto(AssetUuid::NIL),
        FieldType::Curve,
        FieldType::Gradient,
        FieldType::Material,
        FieldType::Texture,
        FieldType::LightEnvironment,
        FieldType::GameAsset(GameAssetKind::Any),
        FieldType::SoftRef(AssetUuid::NIL),
        FieldType::Enum(AssetUuid::NIL),
    ];

    /// All field types selectable in the schema editor UI (including placeholders for parameterized variants).
    pub fn all() -> &'static [FieldType; 38] {
        static ALL: std::sync::LazyLock<[FieldType; 38]> = std::sync::LazyLock::new(|| {
            [
                FieldType::Bool,
                FieldType::I8,
                FieldType::I16,
                FieldType::I32,
                FieldType::I64,
                FieldType::U8,
                FieldType::U16,
                FieldType::U32,
                FieldType::U64,
                FieldType::F32,
                FieldType::F64,
                FieldType::String,
                FieldType::LocalizedString,
                FieldType::Vec2,
                FieldType::Vec3,
                FieldType::Vec4,
                FieldType::IVec2,
                FieldType::IVec3,
                FieldType::UVec2,
                FieldType::UVec3,
                FieldType::Quat,
                FieldType::Color,
                FieldType::Duration,
                FieldType::RangeF32,
                FieldType::RangeI32,
                FieldType::Dto(AssetUuid::NIL),
                FieldType::Curve,
                FieldType::Gradient,
                FieldType::Material,
                FieldType::Texture,
                FieldType::LightEnvironment,
                FieldType::GameAsset(GameAssetKind::Any),
                FieldType::SoftRef(AssetUuid::NIL),
                FieldType::Enum(AssetUuid::NIL),
                FieldType::Option(Box::new(FieldType::I32)),
                FieldType::Array(Box::new(FieldType::I32)),
                FieldType::Set(Box::new(FieldType::I32)),
                FieldType::Map(Box::new(FieldType::String), Box::new(FieldType::I32)),
            ]
        });
        &ALL
    }

    /// Human-readable label shown in the editor UI.
    pub const fn label(&self) -> &'static str {
        match self {
            FieldType::Bool => "Bool",
            FieldType::I8 => "I8",
            FieldType::I16 => "I16",
            FieldType::I32 => "I32",
            FieldType::I64 => "I64",
            FieldType::U8 => "U8",
            FieldType::U16 => "U16",
            FieldType::U32 => "U32",
            FieldType::U64 => "U64",
            FieldType::F32 => "F32",
            FieldType::F64 => "F64",
            FieldType::String => "String",
            FieldType::LocalizedString => "LocalizedString",
            FieldType::Vec2 => "Vec2",
            FieldType::Vec3 => "Vec3",
            FieldType::Vec4 => "Vec4",
            FieldType::IVec2 => "IVec2",
            FieldType::IVec3 => "IVec3",
            FieldType::UVec2 => "UVec2",
            FieldType::UVec3 => "UVec3",
            FieldType::Quat => "Quat",
            FieldType::Color => "Color",
            FieldType::Duration => "Duration",
            FieldType::RangeF32 => "RangeF32",
            FieldType::RangeI32 => "RangeI32",
            FieldType::Dto(_) => "Dto",
            FieldType::Curve => "Curve",
            FieldType::Gradient => "Gradient",
            FieldType::Material => "Material",
            FieldType::Texture => "Texture",
            FieldType::LightEnvironment => "LightEnvironment",
            FieldType::GameAsset(_) => "GameAsset",
            FieldType::SoftRef(_) => "SoftRef",
            FieldType::Enum(_) => "Enum",
            FieldType::Option(_) => "Option",
            FieldType::Array(_) => "Array",
            FieldType::Set(_) => "Set",
            FieldType::Map(_, _) => "Map",
        }
    }

    /// Canonical tag hashed into `DATA_SCHEMA_HASH` (ADR 0049 §6).
    pub const fn canonical_tag(&self) -> &'static str {
        self.label()
    }

    /// Stable numeric schema tag for this `FieldType` variant (ADR 0049 §6).
    pub const fn schema_tag(&self) -> u8 {
        match self {
            FieldType::Bool => 0,
            FieldType::I8 => 1,
            FieldType::I16 => 2,
            FieldType::I32 => 3,
            FieldType::I64 => 4,
            FieldType::U8 => 5,
            FieldType::U16 => 6,
            FieldType::U32 => 7,
            FieldType::U64 => 8,
            FieldType::F32 => 9,
            FieldType::F64 => 10,
            FieldType::String => 11,
            FieldType::LocalizedString => 12,
            FieldType::Vec2 => 13,
            FieldType::Vec3 => 14,
            FieldType::Vec4 => 15,
            FieldType::IVec2 => 16,
            FieldType::IVec3 => 17,
            FieldType::UVec2 => 18,
            FieldType::UVec3 => 19,
            FieldType::Quat => 20,
            FieldType::Color => 21,
            FieldType::Duration => 22,
            FieldType::RangeF32 => 23,
            FieldType::RangeI32 => 24,
            FieldType::Dto(_) => 25,
            FieldType::Curve => 26,
            FieldType::Gradient => 27,
            FieldType::Material => 28,
            FieldType::Texture => 29,
            FieldType::LightEnvironment => 30,
            FieldType::GameAsset(_) => 31,
            FieldType::SoftRef(_) => 32,
            FieldType::Enum(_) => 33,
            FieldType::Option(_) => 34,
            FieldType::Array(_) => 35,
            FieldType::Set(_) => 36,
            FieldType::Map(_, _) => 37,
        }
    }

    /// Stable `usize` tag for UI selectors (`0..38`).
    pub const fn tag(&self) -> usize {
        self.schema_tag() as usize
    }

    /// Constructs the default `FieldType` variant corresponding to `tag` (`0..38`).
    pub fn from_tag(tag: usize) -> Option<FieldType> {
        Self::all().get(tag).cloned()
    }

    /// Whether `self` and `other` have the same outer `FieldType` variant (ignoring inner payload of `Dto` / `Enum` / `GameAsset` / `SoftRef` / containers).
    pub fn same_kind(&self, other: impl Borrow<FieldType>) -> bool {
        let other = other.borrow();
        self.schema_tag() == other.schema_tag()
    }

    /// Whether this field type is `FieldType::Option(_)`.
    pub const fn is_option(&self) -> bool {
        matches!(self, FieldType::Option(_))
    }

    /// Whether this field type is `FieldType::Array(_)`.
    pub const fn is_array(&self) -> bool {
        matches!(self, FieldType::Array(_))
    }

    /// Whether this field type is `FieldType::Set(_)`.
    pub const fn is_set(&self) -> bool {
        matches!(self, FieldType::Set(_))
    }

    /// Whether this field type is `FieldType::Map(_, _)`.
    pub const fn is_map(&self) -> bool {
        matches!(self, FieldType::Map(_, _))
    }

    /// Whether this field type is `Option`, `Array`, `Set`, or `Map`.
    pub const fn is_container(&self) -> bool {
        matches!(self, FieldType::Option(_) | FieldType::Array(_) | FieldType::Set(_) | FieldType::Map(_, _))
    }

    /// Borrows the inner element type for `Option(inner)`, `Array(inner)`, `Set(inner)`, or the value type for `Map(_, val)`.
    pub fn inner_type(&self) -> Option<&FieldType> {
        match self {
            FieldType::Option(inner) | FieldType::Array(inner) | FieldType::Set(inner) | FieldType::Map(_, inner) => {
                Some(inner)
            }
            _ => None,
        }
    }

    /// Borrows the key type for `Set(key)` or `Map(key, _)`.
    pub fn key_type(&self) -> Option<&FieldType> {
        match self {
            FieldType::Set(key) | FieldType::Map(key, _) => Some(key),
            _ => None,
        }
    }

    /// Whether `self` is a valid key type for `Set<K>` or `Map<K, V>` (`Eq + Ord + Hash` in generated Rust:
    /// `Bool`, `I8`..`I64`, `U8`..`U64`, `String`, `LocalizedString`, `SoftRef`, or unit-only `Enum`).
    pub fn is_valid_key_type(&self, enums: &BTreeMap<String, EnumAsset>) -> bool {
        match self {
            FieldType::Bool
            | FieldType::I8
            | FieldType::I16
            | FieldType::I32
            | FieldType::I64
            | FieldType::U8
            | FieldType::U16
            | FieldType::U32
            | FieldType::U64
            | FieldType::String
            | FieldType::LocalizedString
            | FieldType::SoftRef(_) => true,
            FieldType::Enum(target_id) => {
                !target_id.is_nil()
                    && enums
                        .values()
                        .find(|e| !e.id.is_nil() && e.id == *target_id)
                        .is_some_and(|e| !e.variants.is_empty() && e.is_unit_only())
            }
            _ => false,
        }
    }

    /// Human-readable type description resolving `Dto`, `Enum`, `SoftRef`, `Option`, `Array`, `Set`, and `Map` recursively.
    pub fn describe(&self, dtos: &BTreeMap<String, DtoAsset>, enums: &BTreeMap<String, EnumAsset>) -> String {
        match self {
            FieldType::Dto(id) => match dtos.iter().find(|(_, d)| !d.id.is_nil() && d.id == *id) {
                Some((p, _)) => format!("Dto({p})"),
                None => format!("Dto({id})"),
            },
            FieldType::Enum(id) => match enums.iter().find(|(_, e)| !e.id.is_nil() && e.id == *id) {
                Some((p, _)) => format!("Enum({p})"),
                None => format!("Enum({id})"),
            },
            FieldType::SoftRef(id) => match dtos.iter().find(|(_, d)| !d.id.is_nil() && d.id == *id) {
                Some((p, _)) => format!("SoftRef({p})"),
                None => format!("SoftRef({id})"),
            },
            FieldType::Option(inner) => format!("Option<{}>", inner.describe(dtos, enums)),
            FieldType::Array(inner) => format!("Array<{}>", inner.describe(dtos, enums)),
            FieldType::Set(k) => format!("Set<{}>", k.describe(dtos, enums)),
            FieldType::Map(k, v) => format!("Map<{}, {}>", k.describe(dtos, enums), v.describe(dtos, enums)),
            other => other.label().to_string(),
        }
    }

    /// Whether this field type is a nested by-value DTO (`FieldType::Dto(_)`).
    pub const fn is_dto(&self) -> bool {
        matches!(self, FieldType::Dto(_))
    }

    /// Returns the target DTO [`AssetUuid`] if this is `FieldType::Dto(id)`.
    pub const fn dto_target(&self) -> Option<AssetUuid> {
        match *self {
            FieldType::Dto(id) => Some(id),
            _ => None,
        }
    }

    /// Whether this field type is a plain enum or tagged union (`FieldType::Enum(_)`).
    pub const fn is_enum(&self) -> bool {
        matches!(self, FieldType::Enum(_))
    }

    /// Returns the target Enum [`AssetUuid`] if this is `FieldType::Enum(id)`.
    pub const fn enum_target(&self) -> Option<AssetUuid> {
        match *self {
            FieldType::Enum(id) => Some(id),
            _ => None,
        }
    }

    /// Whether this field type is a cross-collection row reference (`FieldType::SoftRef(_)`).
    pub const fn is_soft_ref(&self) -> bool {
        matches!(self, FieldType::SoftRef(_))
    }

    /// Returns the target DTO [`AssetUuid`] if this is `FieldType::SoftRef(id)`.
    pub const fn soft_ref_target(&self) -> Option<AssetUuid> {
        match *self {
            FieldType::SoftRef(id) => Some(id),
            _ => None,
        }
    }

    /// Whether this field type is `FieldType::GameAsset(_)`.
    pub const fn is_game_asset(&self) -> bool {
        matches!(self, FieldType::GameAsset(_))
    }

    /// Returns the [`GameAssetKind`] filter if this is `FieldType::GameAsset(kind)`.
    pub const fn game_asset_kind(&self) -> Option<GameAssetKind> {
        match *self {
            FieldType::GameAsset(kind) => Some(kind),
            _ => None,
        }
    }

    /// Whether this field type is one of the 6 direct asset reference types (`Curve`, `Gradient`, `Material`,
    /// `Texture`, `LightEnvironment`, or `GameAsset`).
    pub const fn is_asset_ref(&self) -> bool {
        matches!(
            self,
            FieldType::Curve
                | FieldType::Gradient
                | FieldType::Material
                | FieldType::Texture
                | FieldType::LightEnvironment
                | FieldType::GameAsset(_)
        )
    }

    /// Returns the default [`DataValue`] for this field type without DTO/Enum lookup context
    /// (`DataValue::Dto(BTreeMap::new())` for `FieldType::Dto`, `DataValue::Enum { variant: AssetUuid::NIL, fields: BTreeMap::new() }` for `FieldType::Enum`).
    pub fn default_value(&self) -> DataValue {
        match self {
            FieldType::Bool => DataValue::Bool(false),
            FieldType::I8 => DataValue::I8(0),
            FieldType::I16 => DataValue::I16(0),
            FieldType::I32 => DataValue::I32(0),
            FieldType::I64 => DataValue::I64(0),
            FieldType::U8 => DataValue::U8(0),
            FieldType::U16 => DataValue::U16(0),
            FieldType::U32 => DataValue::U32(0),
            FieldType::U64 => DataValue::U64(0),
            FieldType::F32 => DataValue::F32(0.0),
            FieldType::F64 => DataValue::F64(0.0),
            FieldType::String => DataValue::String(String::new()),
            FieldType::LocalizedString => DataValue::LocalizedString(String::new()),
            FieldType::Vec2 => DataValue::Vec2([0.0, 0.0]),
            FieldType::Vec3 => DataValue::Vec3([0.0, 0.0, 0.0]),
            FieldType::Vec4 => DataValue::Vec4([0.0, 0.0, 0.0, 0.0]),
            FieldType::IVec2 => DataValue::IVec2([0, 0]),
            FieldType::IVec3 => DataValue::IVec3([0, 0, 0]),
            FieldType::UVec2 => DataValue::UVec2([0, 0]),
            FieldType::UVec3 => DataValue::UVec3([0, 0, 0]),
            FieldType::Quat => DataValue::Quat([0.0, 0.0, 0.0, 1.0]),
            FieldType::Color => DataValue::Color([1.0, 1.0, 1.0, 1.0]),
            FieldType::Duration => DataValue::Duration(0.0),
            FieldType::RangeF32 => DataValue::RangeF32([0.0, 1.0]),
            FieldType::RangeI32 => DataValue::RangeI32([0, 1]),
            FieldType::Dto(_) => DataValue::Dto(BTreeMap::new()),
            FieldType::Curve => DataValue::Curve(AssetUuid::NIL),
            FieldType::Gradient => DataValue::Gradient(AssetUuid::NIL, 0),
            FieldType::Material => DataValue::Material(AssetUuid::NIL),
            FieldType::Texture => DataValue::Texture(AssetUuid::NIL),
            FieldType::LightEnvironment => DataValue::LightEnvironment(AssetUuid::NIL),
            FieldType::GameAsset(_) => DataValue::GameAsset(String::new()),
            FieldType::SoftRef(_) => DataValue::SoftRef(AssetUuid::NIL),
            FieldType::Enum(_) => DataValue::Enum { variant: AssetUuid::NIL, fields: BTreeMap::new() },
            FieldType::Option(_) => DataValue::Option(None),
            FieldType::Array(_) => DataValue::Array(Vec::new()),
            FieldType::Set(_) => DataValue::Set(Vec::new()),
            FieldType::Map(_, _) => DataValue::Map(Vec::new()),
        }
    }

    /// Returns the default [`DataValue`] for this field type, recursively populating `DataValue::Dto` field defaults
    /// from `dtos` up to [`MAX_NESTING_DEPTH`] (stopping on cycles).
    pub fn default_value_with_dtos(&self, dtos: &BTreeMap<String, DtoAsset>) -> DataValue {
        self.default_value_with_schemas(dtos, &BTreeMap::new())
    }

    /// Returns the default [`DataValue`] for this field type, recursively populating `DataValue::Dto` and
    /// `DataValue::Enum` defaults from `dtos` and `enums` up to [`MAX_NESTING_DEPTH`] (stopping on cycles).
    pub fn default_value_with_schemas(
        &self,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) -> DataValue {
        let mut visited = BTreeSet::new();
        self.default_value_with_schemas_inner(dtos, enums, &mut visited, 0)
    }

    fn default_value_with_schemas_inner(
        &self,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
        visited: &mut BTreeSet<AssetUuid>,
        depth: usize,
    ) -> DataValue {
        if depth >= MAX_NESTING_DEPTH {
            return self.default_value();
        }
        let mut val = self.default_value();
        match (self, &mut val) {
            (FieldType::Dto(target_uuid), DataValue::Dto(map)) => {
                fill_target_dto_map_defaults(map, *target_uuid, dtos, enums, visited, depth);
            }
            (FieldType::Enum(target_uuid), DataValue::Enum { variant, fields }) => {
                fill_target_enum_defaults(variant, fields, *target_uuid, dtos, enums, visited, depth);
            }
            _ => {}
        }
        val
    }

    /// Ensures `val` conforms to `self`, coercing or populating defaults recursively using `dtos` and `enums`.
    pub fn ensure_value_with_schemas(
        &self,
        val: &mut DataValue,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) {
        let mut visited = BTreeSet::new();
        fill_value_for_type(val, self, dtos, enums, &mut visited, 0);
    }

    /// Whether `val` has the variant matching this `FieldType` (returning `true` for any `(FieldType::Dto(_), DataValue::Dto(_))`,
    /// `(FieldType::Enum(_), DataValue::Enum { .. })`, `(FieldType::Option(inner), DataValue::Option(opt))` when `opt` is `None`
    /// or `inner.matches(v)`, and `(FieldType::Array(_), DataValue::Array(_))`, `(FieldType::Set(_), DataValue::Set(_))`,
    /// `(FieldType::Map(_, _), DataValue::Map(_))`).
    pub fn matches(&self, val: &DataValue) -> bool {
        match (self, val) {
            (FieldType::Option(inner), DataValue::Option(opt)) => opt.as_deref().is_none_or(|v| inner.matches(v)),
            (FieldType::Array(_), DataValue::Array(_))
            | (FieldType::Set(_), DataValue::Set(_))
            | (FieldType::Map(_, _), DataValue::Map(_)) => true,
            _ => self.same_kind(val.field_type()),
        }
    }

    /// Whether this field type is a scalar numeric value (integers, floats, or `Duration`).
    pub const fn is_numeric(&self) -> bool {
        matches!(
            self,
            FieldType::I8
                | FieldType::I16
                | FieldType::I32
                | FieldType::I64
                | FieldType::U8
                | FieldType::U16
                | FieldType::U32
                | FieldType::U64
                | FieldType::F32
                | FieldType::F64
                | FieldType::Duration
        )
    }

    /// Whether this field type is a string (`String` or `LocalizedString`).
    pub const fn is_string(&self) -> bool {
        matches!(self, FieldType::String | FieldType::LocalizedString)
    }
}

/// Synthetic step magic prefix for encoding container steps (`OptionSome`, `Item(idx)`, `MapKey(idx)`, `MapVal(idx)`)
/// inside `Vec<AssetUuid>` editor paths without colliding with UUIDv7/v8 field identifiers.
const PATH_STEP_MAGIC: [u8; 8] = *b"QSCSTEP\0";

/// Step within a `Vec<AssetUuid>` path addressing into nested `Dto`, `Enum`, `Option`, `Array`, `Set`, or `Map` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStep {
    /// Steps into a `Field::id` inside a `DataValue::Dto` or `DataValue::Enum` payload map.
    Field(AssetUuid),
    /// Steps into `DataValue::Option(Some(inner))`.
    OptionSome,
    /// Steps into element `index` of `DataValue::Array` or `DataValue::Set`.
    Item(usize),
    /// Steps into key `index` of `DataValue::Map`.
    MapKey(usize),
    /// Steps into value `index` of `DataValue::Map`.
    MapVal(usize),
}

fn encode_path_step(tag: u8, idx: usize) -> AssetUuid {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&PATH_STEP_MAGIC);
    bytes[8] = tag;
    bytes[12..16].copy_from_slice(&(idx as u32).to_be_bytes());
    AssetUuid::from_u128(u128::from_be_bytes(bytes))
}

/// Returns the synthetic [`AssetUuid`] path step addressing `DataValue::Option(Some(inner))`.
pub fn path_step_option_some() -> AssetUuid {
    encode_path_step(1, 0)
}

/// Returns the synthetic [`AssetUuid`] path step addressing element `index` of `DataValue::Array` or `DataValue::Set`.
pub fn path_step_item(index: usize) -> AssetUuid {
    encode_path_step(2, index)
}

/// Returns the synthetic [`AssetUuid`] path step addressing key `index` of `DataValue::Map`.
pub fn path_step_map_key(index: usize) -> AssetUuid {
    encode_path_step(3, index)
}

/// Returns the synthetic [`AssetUuid`] path step addressing value `index` of `DataValue::Map`.
pub fn path_step_map_val(index: usize) -> AssetUuid {
    encode_path_step(4, index)
}

/// Decodes a path step [`AssetUuid`] into a [`PathStep`].
pub fn decode_path_step(id: AssetUuid) -> PathStep {
    let bytes = id.as_u128().to_be_bytes();
    if bytes[..8] == PATH_STEP_MAGIC {
        let idx = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        match bytes[8] {
            1 => PathStep::OptionSome,
            2 => PathStep::Item(idx),
            3 => PathStep::MapKey(idx),
            4 => PathStep::MapVal(idx),
            _ => PathStep::Field(id),
        }
    } else {
        PathStep::Field(id)
    }
}

/// A typed value stored in `.dto.ron` / `.enum.ron` field defaults and `.collection.ron` row cells (ADR 0049 §2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DataValue {
    Bool(bool),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    F32(f32),
    F64(f64),
    String(String),
    LocalizedString(String),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    IVec2([i32; 2]),
    IVec3([i32; 3]),
    UVec2([u32; 2]),
    UVec3([u32; 3]),
    Quat([f32; 4]),
    Color([f32; 4]),
    Duration(f32),
    RangeF32([f32; 2]),
    RangeI32([i32; 2]),
    /// Nested by-value DTO field map keyed by target DTO [`Field::id`]s.
    Dto(BTreeMap<AssetUuid, DataValue>),
    /// Direct reference to a Material or Light `.curve.ron` asset by [`AssetUuid`] (`AssetUuid::NIL` when unset).
    Curve(AssetUuid),
    /// Direct reference to a Material `.gradients.ron` asset by [`AssetUuid`] and 1-based slot index (`(AssetUuid::NIL, 0)` when unset).
    Gradient(AssetUuid, u32),
    /// Direct reference to a Material `.material.ron` asset by [`AssetUuid`] (`AssetUuid::NIL` when unset).
    Material(AssetUuid),
    /// Direct reference to a Material or Light `.texture.ron` asset by [`AssetUuid`] (`AssetUuid::NIL` when unset).
    Texture(AssetUuid),
    /// Direct reference to a Light `.environment.ron` asset by [`AssetUuid`] (`AssetUuid::NIL` when unset).
    LightEnvironment(AssetUuid),
    /// Direct reference to a raw game file path relative to `<game_root>/assets/` (empty string when unset).
    GameAsset(String),
    /// Soft reference to a collection row by [`Row::id`] (`AssetUuid::NIL` when unset).
    #[serde(alias = "Ref")]
    SoftRef(AssetUuid),
    /// Plain enum or tagged union value storing the selected [`EnumVariant::id`] and its payload field values keyed by [`Field::id`].
    Enum {
        variant: AssetUuid,
        #[serde(default)]
        fields: BTreeMap<AssetUuid, DataValue>,
    },
    /// Optional value (`Option(None)` or `Option(Some(inner))`).
    Option(Option<Box<DataValue>>),
    /// Ordered list of values (`Array([...])`).
    Array(Vec<DataValue>),
    /// Unique sorted set of valid key values (`Set([...])`).
    Set(Vec<DataValue>),
    /// Key-value map (`Map([(k, v), ...])`).
    Map(Vec<(DataValue, DataValue)>),
}

impl DataValue {
    /// Returns the [`FieldType`] corresponding to this value's variant (`FieldType::Dto(AssetUuid::NIL)` for `DataValue::Dto(_)`,
    /// `FieldType::Enum(AssetUuid::NIL)` for `DataValue::Enum { .. }`, `FieldType::GameAsset(GameAssetKind::Any)` for `DataValue::GameAsset(_)`,
    /// `FieldType::SoftRef(AssetUuid::NIL)` for `DataValue::SoftRef(_)`).
    pub fn field_type(&self) -> FieldType {
        match self {
            DataValue::Bool(_) => FieldType::Bool,
            DataValue::I8(_) => FieldType::I8,
            DataValue::I16(_) => FieldType::I16,
            DataValue::I32(_) => FieldType::I32,
            DataValue::I64(_) => FieldType::I64,
            DataValue::U8(_) => FieldType::U8,
            DataValue::U16(_) => FieldType::U16,
            DataValue::U32(_) => FieldType::U32,
            DataValue::U64(_) => FieldType::U64,
            DataValue::F32(_) => FieldType::F32,
            DataValue::F64(_) => FieldType::F64,
            DataValue::String(_) => FieldType::String,
            DataValue::LocalizedString(_) => FieldType::LocalizedString,
            DataValue::Vec2(_) => FieldType::Vec2,
            DataValue::Vec3(_) => FieldType::Vec3,
            DataValue::Vec4(_) => FieldType::Vec4,
            DataValue::IVec2(_) => FieldType::IVec2,
            DataValue::IVec3(_) => FieldType::IVec3,
            DataValue::UVec2(_) => FieldType::UVec2,
            DataValue::UVec3(_) => FieldType::UVec3,
            DataValue::Quat(_) => FieldType::Quat,
            DataValue::Color(_) => FieldType::Color,
            DataValue::Duration(_) => FieldType::Duration,
            DataValue::RangeF32(_) => FieldType::RangeF32,
            DataValue::RangeI32(_) => FieldType::RangeI32,
            DataValue::Dto(_) => FieldType::Dto(AssetUuid::NIL),
            DataValue::Curve(_) => FieldType::Curve,
            DataValue::Gradient(_, _) => FieldType::Gradient,
            DataValue::Material(_) => FieldType::Material,
            DataValue::Texture(_) => FieldType::Texture,
            DataValue::LightEnvironment(_) => FieldType::LightEnvironment,
            DataValue::GameAsset(_) => FieldType::GameAsset(GameAssetKind::Any),
            DataValue::SoftRef(_) => FieldType::SoftRef(AssetUuid::NIL),
            DataValue::Enum { .. } => FieldType::Enum(AssetUuid::NIL),
            DataValue::Option(Some(inner)) => FieldType::Option(Box::new(inner.field_type())),
            DataValue::Option(None) => FieldType::Option(Box::new(FieldType::I32)),
            DataValue::Array(items) => {
                FieldType::Array(Box::new(items.first().map_or(FieldType::I32, DataValue::field_type)))
            }
            DataValue::Set(items) => {
                FieldType::Set(Box::new(items.first().map_or(FieldType::I32, DataValue::field_type)))
            }
            DataValue::Map(entries) => {
                let (k, v) = entries
                    .first()
                    .map_or((FieldType::String, FieldType::I32), |(k, v)| (k.field_type(), v.field_type()));
                FieldType::Map(Box::new(k), Box::new(v))
            }
        }
    }

    /// Deterministic ordering for valid `Set` and `Map` keys matching generated Rust `Ord` (`BTreeSet<K>`, `BTreeMap<K, V>`).
    pub fn cmp_key(
        &self,
        other: &Self,
        enums: &BTreeMap<String, EnumAsset>,
        target_enum: Option<AssetUuid>,
    ) -> std::cmp::Ordering {
        match (self, other) {
            (DataValue::Bool(a), DataValue::Bool(b)) => a.cmp(b),
            (DataValue::I8(a), DataValue::I8(b)) => a.cmp(b),
            (DataValue::I16(a), DataValue::I16(b)) => a.cmp(b),
            (DataValue::I32(a), DataValue::I32(b)) => a.cmp(b),
            (DataValue::I64(a), DataValue::I64(b)) => a.cmp(b),
            (DataValue::U8(a), DataValue::U8(b)) => a.cmp(b),
            (DataValue::U16(a), DataValue::U16(b)) => a.cmp(b),
            (DataValue::U32(a), DataValue::U32(b)) => a.cmp(b),
            (DataValue::U64(a), DataValue::U64(b)) => a.cmp(b),
            (DataValue::String(a), DataValue::String(b))
            | (DataValue::LocalizedString(a), DataValue::LocalizedString(b)) => a.cmp(b),
            (DataValue::SoftRef(a), DataValue::SoftRef(b)) => a.cmp(b),
            (DataValue::Enum { variant: va, .. }, DataValue::Enum { variant: vb, .. }) => {
                if let Some(en_id) = target_enum
                    && let Some(en) = enums.values().find(|e| !e.id.is_nil() && e.id == en_id)
                {
                    let idx_a = en.variants.iter().position(|v| v.id == *va).unwrap_or(usize::MAX);
                    let idx_b = en.variants.iter().position(|v| v.id == *vb).unwrap_or(usize::MAX);
                    return idx_a.cmp(&idx_b).then_with(|| va.cmp(vb));
                }
                for en in enums.values() {
                    if let (Some(idx_a), Some(idx_b)) =
                        (en.variants.iter().position(|v| v.id == *va), en.variants.iter().position(|v| v.id == *vb))
                    {
                        return idx_a.cmp(&idx_b).then_with(|| va.cmp(vb));
                    }
                }
                va.cmp(vb)
            }
            _ => self.to_short_view_string().cmp(&other.to_short_view_string()),
        }
    }

    /// Recursively sorts `DataValue::Set` and `DataValue::Map` entries in `self` using [`DataValue::cmp_key`].
    pub fn sort_container_keys(
        &mut self,
        ty: Option<&FieldType>,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) {
        self.sort_container_keys_inner(ty, dtos, enums, 0);
    }

    fn sort_container_keys_inner(
        &mut self,
        ty: Option<&FieldType>,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
        depth: usize,
    ) {
        if depth >= MAX_NESTING_DEPTH {
            return;
        }
        match self {
            DataValue::Option(Some(inner)) => {
                let inner_ty = match ty {
                    Some(FieldType::Option(t)) => Some(t.as_ref()),
                    _ => None,
                };
                inner.sort_container_keys_inner(inner_ty, dtos, enums, depth + 1);
            }
            DataValue::Array(items) => {
                let inner_ty = match ty {
                    Some(FieldType::Array(t)) => Some(t.as_ref()),
                    _ => None,
                };
                for item in items {
                    item.sort_container_keys_inner(inner_ty, dtos, enums, depth + 1);
                }
            }
            DataValue::Set(items) => {
                let key_ty = match ty {
                    Some(FieldType::Set(k)) => Some(k.as_ref()),
                    _ => None,
                };
                for item in items.iter_mut() {
                    item.sort_container_keys_inner(key_ty, dtos, enums, depth + 1);
                }
                let target_enum = key_ty.and_then(FieldType::enum_target);
                items.sort_by(|a, b| a.cmp_key(b, enums, target_enum));
            }
            DataValue::Map(entries) => {
                let (key_ty, val_ty) = match ty {
                    Some(FieldType::Map(k, v)) => (Some(k.as_ref()), Some(v.as_ref())),
                    _ => (None, None),
                };
                for (k, v) in entries.iter_mut() {
                    k.sort_container_keys_inner(key_ty, dtos, enums, depth + 1);
                    v.sort_container_keys_inner(val_ty, dtos, enums, depth + 1);
                }
                let target_enum = key_ty.and_then(FieldType::enum_target);
                entries.sort_by(|(ka, _), (kb, _)| ka.cmp_key(kb, enums, target_enum));
            }
            DataValue::Dto(map) => {
                let target_dto = match ty {
                    Some(FieldType::Dto(id)) if !id.is_nil() => dtos.values().find(|d| !d.id.is_nil() && d.id == *id),
                    _ => None,
                };
                for (fid, v) in map.iter_mut() {
                    let f_ty = target_dto.and_then(|d| d.field_by_id(*fid)).map(|f| &f.ty);
                    v.sort_container_keys_inner(f_ty, dtos, enums, depth + 1);
                }
            }
            DataValue::Enum { variant, fields } => {
                let target_var = match ty {
                    Some(FieldType::Enum(id)) if !id.is_nil() => {
                        enums.values().find(|e| !e.id.is_nil() && e.id == *id).and_then(|e| e.variant_by_id(*variant))
                    }
                    _ => None,
                };
                for (fid, v) in fields.iter_mut() {
                    let f_ty = target_var.and_then(|var| var.field_by_id(*fid)).map(|f| &f.ty);
                    v.sort_container_keys_inner(f_ty, dtos, enums, depth + 1);
                }
            }
            _ => {}
        }
    }

    /// Whether this value's variant matches `ty`.
    pub fn matches_type(&self, ty: &FieldType) -> bool {
        ty.matches(self)
    }

    /// Borrows the inner field map when this is `DataValue::Dto`.
    pub const fn as_dto_map(&self) -> Option<&BTreeMap<AssetUuid, DataValue>> {
        match self {
            DataValue::Dto(map) => Some(map),
            _ => None,
        }
    }

    /// Mutably borrows the inner field map when this is `DataValue::Dto`.
    pub fn as_dto_map_mut(&mut self) -> Option<&mut BTreeMap<AssetUuid, DataValue>> {
        match self {
            DataValue::Dto(map) => Some(map),
            _ => None,
        }
    }

    /// Borrows `(variant, &fields)` when this is `DataValue::Enum`.
    pub const fn as_enum(&self) -> Option<(AssetUuid, &BTreeMap<AssetUuid, DataValue>)> {
        match self {
            DataValue::Enum { variant, fields } => Some((*variant, fields)),
            _ => None,
        }
    }

    /// Mutably borrows `(&mut variant, &mut fields)` when this is `DataValue::Enum`.
    pub fn as_enum_mut(&mut self) -> Option<(&mut AssetUuid, &mut BTreeMap<AssetUuid, DataValue>)> {
        match self {
            DataValue::Enum { variant, fields } => Some((variant, fields)),
            _ => None,
        }
    }

    /// Borrows `Option<&DataValue>` when this is `DataValue::Option`.
    pub fn as_option(&self) -> Option<Option<&DataValue>> {
        match self {
            DataValue::Option(opt) => Some(opt.as_deref()),
            _ => None,
        }
    }

    /// Borrows the element slice when this is `DataValue::Array`.
    pub fn as_array(&self) -> Option<&[DataValue]> {
        match self {
            DataValue::Array(items) => Some(items.as_slice()),
            _ => None,
        }
    }

    /// Borrows the element slice when this is `DataValue::Set`.
    pub fn as_set(&self) -> Option<&[DataValue]> {
        match self {
            DataValue::Set(items) => Some(items.as_slice()),
            _ => None,
        }
    }

    /// Borrows the `(key, val)` slice when this is `DataValue::Map`.
    pub fn as_map(&self) -> Option<&[(DataValue, DataValue)]> {
        match self {
            DataValue::Map(entries) => Some(entries.as_slice()),
            _ => None,
        }
    }

    /// Resolves a nested [`DataValue`] along `sub_path` of [`PathStep`]s inside `Dto`, `Enum`, `Option`, `Array`, `Set`, or `Map`.
    pub fn get_at_path(&self, sub_path: &[AssetUuid]) -> Option<&DataValue> {
        let mut cur = self;
        for &step_id in sub_path {
            cur = match (decode_path_step(step_id), cur) {
                (PathStep::Field(fid), DataValue::Dto(map)) => map.get(&fid)?,
                (PathStep::Field(fid), DataValue::Enum { fields, .. }) => fields.get(&fid)?,
                (PathStep::OptionSome, DataValue::Option(Some(inner))) => inner.as_ref(),
                (PathStep::Item(idx), DataValue::Array(items) | DataValue::Set(items)) => items.get(idx)?,
                (PathStep::MapKey(idx), DataValue::Map(entries)) => &entries.get(idx)?.0,
                (PathStep::MapVal(idx), DataValue::Map(entries)) => &entries.get(idx)?.1,
                _ => return None,
            };
        }
        Some(cur)
    }

    /// Returns the scalar numeric value as `f64` for numeric types (`I8`..`U64`, `F32`, `F64`, `Duration`).
    pub const fn as_f64(&self) -> Option<f64> {
        match *self {
            DataValue::I8(v) => Some(v as f64),
            DataValue::I16(v) => Some(v as f64),
            DataValue::I32(v) => Some(v as f64),
            DataValue::I64(v) => Some(v as f64),
            DataValue::U8(v) => Some(v as f64),
            DataValue::U16(v) => Some(v as f64),
            DataValue::U32(v) => Some(v as f64),
            DataValue::U64(v) => Some(v as f64),
            DataValue::F32(v) => Some(v as f64),
            DataValue::F64(v) => Some(v),
            DataValue::Duration(v) => Some(v as f64),
            _ => None,
        }
    }

    /// Returns the string slice for `String` or `LocalizedString`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            DataValue::String(s) | DataValue::LocalizedString(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Coerces this value to `target` when changing a field's type in the editor, preserving compatible string or
    /// numeric values where possible and falling back to `target.default_value()` otherwise.
    pub fn coerce_to(&self, target: impl Borrow<FieldType>) -> DataValue {
        let target = target.borrow();
        if target.matches(self) {
            return self.clone();
        }
        if let Some(s) = self.as_str() {
            return match target {
                FieldType::String => DataValue::String(s.to_string()),
                FieldType::LocalizedString => DataValue::LocalizedString(s.to_string()),
                _ => target.default_value(),
            };
        }
        if let Some(n) = self.as_f64()
            && n.is_finite()
        {
            return match target {
                FieldType::I8 => DataValue::I8(n.round().clamp(i8::MIN as f64, i8::MAX as f64) as i8),
                FieldType::I16 => DataValue::I16(n.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16),
                FieldType::I32 => DataValue::I32(n.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32),
                FieldType::I64 => DataValue::I64(n.round().clamp(i64::MIN as f64, i64::MAX as f64) as i64),
                FieldType::U8 => DataValue::U8(n.round().clamp(0.0, u8::MAX as f64) as u8),
                FieldType::U16 => DataValue::U16(n.round().clamp(0.0, u16::MAX as f64) as u16),
                FieldType::U32 => DataValue::U32(n.round().clamp(0.0, u32::MAX as f64) as u32),
                FieldType::U64 => DataValue::U64(n.round().clamp(0.0, u64::MAX as f64) as u64),
                FieldType::F32 => DataValue::F32(n as f32),
                FieldType::F64 => DataValue::F64(n),
                FieldType::Duration => DataValue::Duration(n.max(0.0) as f32),
                _ => target.default_value(),
            };
        }
        target.default_value()
    }

    /// Formats this value as a short display string, resolving `FieldType::Dto(target_uuid)` against `dtos` so nested
    /// `is_name` fields are formatted in schema order up to [`MAX_NESTING_DEPTH`].
    pub fn to_short_view_string_with_dtos(
        &self,
        ty: impl Borrow<FieldType>,
        dtos: &BTreeMap<String, DtoAsset>,
    ) -> String {
        self.to_short_view_string_with_schemas(ty, dtos, &BTreeMap::new())
    }

    /// Formats this value as a short display string, resolving `FieldType::Dto(target_uuid)` against `dtos` and
    /// `FieldType::Enum(target_uuid)` against `enums` up to [`MAX_NESTING_DEPTH`].
    pub fn to_short_view_string_with_schemas(
        &self,
        ty: impl Borrow<FieldType>,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) -> String {
        let mut visited = BTreeSet::new();
        self.to_short_view_string_inner(ty.borrow(), dtos, enums, &mut visited, 0)
    }

    fn to_short_view_string_inner(
        &self,
        ty: &FieldType,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
        visited: &mut BTreeSet<AssetUuid>,
        depth: usize,
    ) -> String {
        match (ty, self) {
            (FieldType::Dto(target_uuid), DataValue::Dto(map)) => {
                if depth >= MAX_NESTING_DEPTH || target_uuid.is_nil() {
                    return self.to_short_view_string();
                }
                if let Some((target_path, target_dto)) =
                    dtos.iter().find(|(_, d)| !d.id.is_nil() && d.id == *target_uuid)
                {
                    if !visited.insert(*target_uuid) {
                        return to_pascal_case(target_path);
                    }
                    let s = target_dto.short_view_string_inner(
                        map,
                        None,
                        Some(target_path.as_str()),
                        dtos,
                        enums,
                        visited,
                        depth + 1,
                    );
                    visited.remove(target_uuid);
                    return s;
                }
            }
            (FieldType::Enum(target_uuid), DataValue::Enum { variant, fields }) => {
                if depth >= MAX_NESTING_DEPTH || target_uuid.is_nil() {
                    return self.to_short_view_string();
                }
                if let Some((target_path, target_enum)) =
                    enums.iter().find(|(_, e)| !e.id.is_nil() && e.id == *target_uuid)
                {
                    if !visited.insert(*target_uuid) {
                        return to_pascal_case(target_path);
                    }
                    let s = target_enum.short_view_string_inner(*variant, fields, dtos, enums, visited, depth + 1);
                    visited.remove(target_uuid);
                    return s;
                }
            }
            (FieldType::Option(inner_ty), DataValue::Option(opt)) => {
                if depth >= MAX_NESTING_DEPTH {
                    return self.to_short_view_string();
                }
                return match opt {
                    Some(v) => {
                        let mut sub_visited = BTreeSet::new();
                        v.to_short_view_string_inner(inner_ty, dtos, enums, &mut sub_visited, depth + 1)
                    }
                    None => "None".to_string(),
                };
            }
            (FieldType::Array(inner_ty), DataValue::Array(items)) => {
                if depth >= MAX_NESTING_DEPTH {
                    return self.to_short_view_string();
                }
                let mut parts: Vec<String> = items
                    .iter()
                    .take(3)
                    .map(|v| {
                        let mut sub_visited = BTreeSet::new();
                        v.to_short_view_string_inner(inner_ty, dtos, enums, &mut sub_visited, depth + 1)
                    })
                    .collect();
                if items.len() > 3 {
                    parts.push("...".to_string());
                }
                return format!("[{}]", parts.join(", "));
            }
            (FieldType::Set(inner_ty), DataValue::Set(items)) => {
                if depth >= MAX_NESTING_DEPTH {
                    return self.to_short_view_string();
                }
                let mut parts: Vec<String> = items
                    .iter()
                    .take(3)
                    .map(|v| {
                        let mut sub_visited = BTreeSet::new();
                        v.to_short_view_string_inner(inner_ty, dtos, enums, &mut sub_visited, depth + 1)
                    })
                    .collect();
                if items.len() > 3 {
                    parts.push("...".to_string());
                }
                return format!("{{{}}}", parts.join(", "));
            }
            (FieldType::Map(k_ty, v_ty), DataValue::Map(entries)) => {
                if depth >= MAX_NESTING_DEPTH {
                    return self.to_short_view_string();
                }
                let mut parts: Vec<String> = entries
                    .iter()
                    .take(3)
                    .map(|(k, v)| {
                        let mut k_visited = BTreeSet::new();
                        let mut v_visited = BTreeSet::new();
                        format!(
                            "{}: {}",
                            k.to_short_view_string_inner(k_ty, dtos, enums, &mut k_visited, depth + 1),
                            v.to_short_view_string_inner(v_ty, dtos, enums, &mut v_visited, depth + 1)
                        )
                    })
                    .collect();
                if entries.len() > 3 {
                    parts.push("...".to_string());
                }
                return format!("{{{}}}", parts.join(", "));
            }
            _ => {}
        }
        self.to_short_view_string()
    }
}

/// Short human-readable summary string for editor row labels and runtime inspection (ADR 0049 §3).
pub trait ToShortViewString {
    fn to_short_view_string(&self) -> String;
}

/// Formats a floating-point number with at most 2 decimal places, stripping trailing zeros and decimal point.
pub fn format_compact_f64(v: f64) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    let normalized = if v.abs() < 0.000_000_1 { 0.0 } else { v };
    let s = format!("{normalized:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" { "0".to_string() } else { s.to_string() }
}

/// Converts a single linear RGB channel `[0.0, 1.0]` to an 8-bit sRGB byte.
pub fn linear_channel_to_srgb_u8(c: f32) -> u8 {
    if !c.is_finite() {
        return 0;
    }
    let c = c.clamp(0.0, 1.0);
    let srgb = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (srgb * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8
}

/// Formats a linear RGBA color `[r, g, b, a]` as uppercase sRGB hex `#RRGGBB` (or `#RRGGBBAA` when `a < 0.999`).
pub fn format_color_hex(rgba: [f32; 4]) -> String {
    let r = linear_channel_to_srgb_u8(rgba[0]);
    let g = linear_channel_to_srgb_u8(rgba[1]);
    let b = linear_channel_to_srgb_u8(rgba[2]);
    let a = rgba[3];
    if a.is_finite() && a < 0.999 {
        let a_u8 = (a.clamp(0.0, 1.0) * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8;
        format!("#{r:02X}{g:02X}{b:02X}{a_u8:02X}")
    } else {
        format!("#{r:02X}{g:02X}{b:02X}")
    }
}

impl ToShortViewString for DataValue {
    fn to_short_view_string(&self) -> String {
        match self {
            DataValue::Bool(v) => v.to_string(),
            DataValue::I8(v) => v.to_string(),
            DataValue::I16(v) => v.to_string(),
            DataValue::I32(v) => v.to_string(),
            DataValue::I64(v) => v.to_string(),
            DataValue::U8(v) => v.to_string(),
            DataValue::U16(v) => v.to_string(),
            DataValue::U32(v) => v.to_string(),
            DataValue::U64(v) => v.to_string(),
            DataValue::F32(v) => format_compact_f64(f64::from(*v)),
            DataValue::F64(v) => format_compact_f64(*v),
            DataValue::String(s) | DataValue::LocalizedString(s) => s.clone(),
            DataValue::Vec2([x, y]) => {
                format!("({}, {})", format_compact_f64(f64::from(*x)), format_compact_f64(f64::from(*y)))
            }
            DataValue::Vec3([x, y, z]) => format!(
                "({}, {}, {})",
                format_compact_f64(f64::from(*x)),
                format_compact_f64(f64::from(*y)),
                format_compact_f64(f64::from(*z))
            ),
            DataValue::Vec4([x, y, z, w]) | DataValue::Quat([x, y, z, w]) => format!(
                "({}, {}, {}, {})",
                format_compact_f64(f64::from(*x)),
                format_compact_f64(f64::from(*y)),
                format_compact_f64(f64::from(*z)),
                format_compact_f64(f64::from(*w))
            ),
            DataValue::IVec2([x, y]) => format!("({x}, {y})"),
            DataValue::IVec3([x, y, z]) => format!("({x}, {y}, {z})"),
            DataValue::UVec2([x, y]) => format!("({x}, {y})"),
            DataValue::UVec3([x, y, z]) => format!("({x}, {y}, {z})"),
            DataValue::Color(rgba) => format_color_hex(*rgba),
            DataValue::Duration(secs) => format!("{}s", format_compact_f64(f64::from(*secs))),
            DataValue::RangeF32([min, max]) => {
                format!("{}..{}", format_compact_f64(f64::from(*min)), format_compact_f64(f64::from(*max)))
            }
            DataValue::RangeI32([min, max]) => format!("{min}..{max}"),
            DataValue::Dto(map) => {
                let parts: Vec<String> = map
                    .values()
                    .map(ToShortViewString::to_short_view_string)
                    .filter(|s| !s.trim().is_empty())
                    .collect();
                if parts.is_empty() { "{}".to_string() } else { parts.join(" | ") }
            }
            DataValue::Curve(id)
            | DataValue::Material(id)
            | DataValue::Texture(id)
            | DataValue::LightEnvironment(id) => {
                if id.is_nil() {
                    "None".to_string()
                } else {
                    id.to_string()
                }
            }
            DataValue::Gradient(id, slot) => {
                if id.is_nil() || *slot == 0 {
                    "None".to_string()
                } else {
                    format!("{id}:{slot}")
                }
            }
            DataValue::GameAsset(path) => {
                if path.is_empty() {
                    "None".to_string()
                } else {
                    path.clone()
                }
            }
            DataValue::SoftRef(id) => {
                if id.is_nil() {
                    "<none>".to_string()
                } else {
                    id.to_string()
                }
            }
            DataValue::Enum { variant, fields } => {
                if variant.is_nil() {
                    "None".to_string()
                } else if fields.is_empty() {
                    variant.to_string()
                } else {
                    let parts: Vec<String> = fields
                        .values()
                        .map(ToShortViewString::to_short_view_string)
                        .filter(|s| !s.trim().is_empty())
                        .collect();
                    if parts.is_empty() { variant.to_string() } else { format!("{variant}({})", parts.join(" | ")) }
                }
            }
            DataValue::Option(opt) => match opt {
                Some(v) => v.to_short_view_string(),
                None => "None".to_string(),
            },
            DataValue::Array(items) => {
                let mut parts: Vec<String> =
                    items.iter().take(3).map(ToShortViewString::to_short_view_string).collect();
                if items.len() > 3 {
                    parts.push("...".to_string());
                }
                format!("[{}]", parts.join(", "))
            }
            DataValue::Set(items) => {
                let mut parts: Vec<String> =
                    items.iter().take(3).map(ToShortViewString::to_short_view_string).collect();
                if items.len() > 3 {
                    parts.push("...".to_string());
                }
                format!("{{{}}}", parts.join(", "))
            }
            DataValue::Map(entries) => {
                let mut parts: Vec<String> = entries
                    .iter()
                    .take(3)
                    .map(|(k, v)| format!("{}: {}", k.to_short_view_string(), v.to_short_view_string()))
                    .collect();
                if entries.len() > 3 {
                    parts.push("...".to_string());
                }
                format!("{{{}}}", parts.join(", "))
            }
        }
    }
}

/// Trait over row-like field maps ([`Row`] and `BTreeMap<AssetUuid, DataValue>`) for [`DtoAsset::short_view_string`].
pub trait HasFieldValues {
    fn field_values(&self) -> &BTreeMap<AssetUuid, DataValue>;
}

impl HasFieldValues for Row {
    fn field_values(&self) -> &BTreeMap<AssetUuid, DataValue> {
        &self.values
    }
}

impl HasFieldValues for BTreeMap<AssetUuid, DataValue> {
    fn field_values(&self) -> &BTreeMap<AssetUuid, DataValue> {
        self
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Optional validation constraints and UI widget hints for a DTO [`Field`] (D13, techdoc §7.3).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FieldSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub slider: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub multiline: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_len: Option<usize>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub unique_in_collection: bool,
}

impl FieldSettings {
    /// Whether all settings are at their default values.
    pub fn is_default(&self) -> bool {
        self == &FieldSettings::default()
    }
}

/// One schema field inside a [`DtoAsset`] or [`EnumVariant`] (ADR 0049 §3, §5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    /// Stable UUIDv7 of this field. Row values in `.collection.ron` and enum payload maps are keyed by this UUID so
    /// renaming a field in the schema never loses data.
    #[serde(default, skip_serializing_if = "AssetUuid::is_nil")]
    pub id: AssetUuid,
    /// Lower `snake_case` Rust identifier of the field.
    pub name: String,
    /// Declared type of the field.
    pub ty: FieldType,
    /// Free-form category header in the editor (empty string groups under `"General"`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
    /// Whether this field participates in the [`ToShortViewString`] name/summary label.
    #[serde(default, alias = "summary", skip_serializing_if = "is_false")]
    pub is_name: bool,
    /// Mandatory default value (D6).
    pub default: DataValue,
    /// Optional constraints and UI widget hints.
    #[serde(default, skip_serializing_if = "FieldSettings::is_default")]
    pub settings: FieldSettings,
}

impl Field {
    /// Creates a new field with a freshly minted UUIDv7 and default value for `ty`.
    pub fn new(name: impl Into<String>, ty: FieldType) -> Self {
        let default = ty.default_value();
        Field {
            id: AssetUuid::new_v7(),
            name: name.into(),
            ty,
            category: String::new(),
            is_name: false,
            default,
            settings: FieldSettings::default(),
        }
    }

    /// Creates a new field with a freshly minted UUIDv7 and recursively populated default value for `ty`.
    pub fn new_with_dtos(name: impl Into<String>, ty: FieldType, dtos: &BTreeMap<String, DtoAsset>) -> Self {
        Self::new_with_schemas(name, ty, dtos, &BTreeMap::new())
    }

    /// Creates a new field with a freshly minted UUIDv7 and recursively populated default value for `ty` using `dtos` and `enums`.
    pub fn new_with_schemas(
        name: impl Into<String>,
        ty: FieldType,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) -> Self {
        let default = ty.default_value_with_schemas(dtos, enums);
        Field {
            id: AssetUuid::new_v7(),
            name: name.into(),
            ty,
            category: String::new(),
            is_name: false,
            default,
            settings: FieldSettings::default(),
        }
    }

    /// Effective category label (`"General"` when `category` is empty).
    pub fn effective_category(&self) -> &str {
        let trimmed = self.category.trim();
        if trimmed.is_empty() { "General" } else { trimmed }
    }

    /// Returns this field's default [`DataValue`], recursively populating any missing keys in `FieldType::Dto` maps
    /// from `dtos` up to [`MAX_NESTING_DEPTH`].
    pub fn default_with_dtos(&self, dtos: &BTreeMap<String, DtoAsset>) -> DataValue {
        self.default_with_schemas(dtos, &BTreeMap::new())
    }

    /// Returns this field's default [`DataValue`], recursively populating any missing keys in `FieldType::Dto` and
    /// `FieldType::Enum` maps from `dtos` and `enums` up to [`MAX_NESTING_DEPTH`].
    pub fn default_with_schemas(
        &self,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) -> DataValue {
        let mut visited = BTreeSet::new();
        self.default_with_schemas_inner(dtos, enums, &mut visited, 0)
    }

    fn default_with_schemas_inner(
        &self,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
        visited: &mut BTreeSet<AssetUuid>,
        depth: usize,
    ) -> DataValue {
        let mut val = self.default.clone();
        fill_value_for_type(&mut val, &self.ty, dtos, enums, visited, depth);
        val
    }
}

/// One variant inside an [`EnumAsset`] (ADR 0049 §5, techdoc §7.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnumVariant {
    /// Stable UUIDv7 of this variant.
    #[serde(default, skip_serializing_if = "AssetUuid::is_nil")]
    pub id: AssetUuid,
    /// Lower `snake_case` identifier of the variant (emitted as `PascalCase` in Rust).
    pub name: String,
    /// Optional documentation comment emitted on the generated Rust variant.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc: String,
    /// Ordered payload fields of this variant (empty for a unit variant).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Field>,
}

impl EnumVariant {
    /// Creates a new unit variant with a freshly minted UUIDv7.
    pub fn new(name: impl Into<String>) -> Self {
        EnumVariant { id: AssetUuid::new_v7(), name: name.into(), doc: String::new(), fields: Vec::new() }
    }

    /// Looks up a payload field by its [`AssetUuid`].
    pub fn field_by_id(&self, id: AssetUuid) -> Option<&Field> {
        self.fields.iter().find(|f| f.id == id)
    }

    /// Looks up a payload field by its `name`.
    pub fn field_by_name(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }
}

/// An `.enum.ron` plain enum or tagged union schema asset (ADR 0049 §5, techdoc §7.5, D8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnumAsset {
    pub format_version: u32,
    /// The asset's UUIDv7 (ADR 0047 §2); nil until first saved.
    #[serde(default, skip_serializing_if = "AssetUuid::is_nil")]
    pub id: AssetUuid,
    /// Optional documentation comment emitted on the generated Rust enum.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc: String,
    /// Ordered variants of this enum.
    #[serde(default)]
    pub variants: Vec<EnumVariant>,
}

impl Default for EnumAsset {
    fn default() -> Self {
        Self::new()
    }
}

impl EnumAsset {
    /// Creates an empty `.enum.ron` model at [`ENUM_FORMAT_VERSION`].
    pub fn new() -> Self {
        EnumAsset { format_version: ENUM_FORMAT_VERSION, id: AssetUuid::NIL, doc: String::new(), variants: Vec::new() }
    }

    /// Mints UUIDv7s for the root `id`, every variant whose `id` is nil or non-v7, and every variant field whose `id`
    /// is nil or non-v7; returns the root `id`.
    pub fn ensure_ids(&mut self) -> AssetUuid {
        for variant in &mut self.variants {
            variant.id.ensure();
            for field in &mut variant.fields {
                field.id.ensure();
            }
        }
        self.id.ensure()
    }

    /// Alias of [`EnumAsset::ensure_ids`] matching other domain models.
    pub fn ensure_id(&mut self) -> AssetUuid {
        self.ensure_ids()
    }

    /// Mints fresh UUIDv7s for every variant and variant field (used when regenerating the UUID of a copied `.enum.ron`
    /// file so its sub-items do not collide with the original enum's).
    pub fn regenerate_sub_ids(&mut self) {
        for variant in &mut self.variants {
            variant.id = AssetUuid::new_v7();
            for field in &mut variant.fields {
                field.id = AssetUuid::new_v7();
            }
        }
    }

    /// Whether every variant in this enum has zero payload fields (allowing generated Rust to derive `Copy, Eq, Ord, Hash`).
    pub fn is_unit_only(&self) -> bool {
        self.variants.iter().all(|v| v.fields.is_empty())
    }

    /// Looks up a variant by its [`AssetUuid`].
    pub fn variant_by_id(&self, id: AssetUuid) -> Option<&EnumVariant> {
        self.variants.iter().find(|v| v.id == id)
    }

    /// Mutably looks up a variant by its [`AssetUuid`].
    pub fn variant_by_id_mut(&mut self, id: AssetUuid) -> Option<&mut EnumVariant> {
        self.variants.iter_mut().find(|v| v.id == id)
    }

    /// Looks up a variant by its `name`.
    pub fn variant_by_name(&self, name: &str) -> Option<&EnumVariant> {
        self.variants.iter().find(|v| v.name == name)
    }

    /// Looks up a payload field by its [`AssetUuid`] across all variants, returning `(&EnumVariant, &Field)`.
    pub fn find_variant_field(&self, field_id: AssetUuid) -> Option<(&EnumVariant, &Field)> {
        for variant in &self.variants {
            if let Some(field) = variant.field_by_id(field_id) {
                return Some((variant, field));
            }
        }
        None
    }

    /// Formats an enum value `(variant_id, fields)` according to this enum's schema: unit variants format as
    /// `to_pascal_case(&variant.name)`, and payload variants format as `format!("{}({})", to_pascal_case(&variant.name), payload_summary)`.
    pub fn short_view_string(
        &self,
        variant_id: AssetUuid,
        fields: &BTreeMap<AssetUuid, DataValue>,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) -> String {
        let mut visited = BTreeSet::new();
        if !self.id.is_nil() {
            visited.insert(self.id);
        }
        self.short_view_string_inner(variant_id, fields, dtos, enums, &mut visited, 0)
    }

    fn short_view_string_inner(
        &self,
        variant_id: AssetUuid,
        fields: &BTreeMap<AssetUuid, DataValue>,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
        visited: &mut BTreeSet<AssetUuid>,
        depth: usize,
    ) -> String {
        let Some(variant) = self.variant_by_id(variant_id).or_else(|| self.variants.first()) else {
            return "None".to_string();
        };
        let v_pascal = to_pascal_case(&variant.name);
        if variant.fields.is_empty() {
            return v_pascal;
        }
        let has_name_fields = variant.fields.iter().any(|f| f.is_name);
        let mut parts = Vec::new();
        if has_name_fields {
            for field in variant.fields.iter().filter(|f| f.is_name) {
                let val = fields.get(&field.id).unwrap_or(&field.default);
                let s = val.to_short_view_string_inner(&field.ty, dtos, enums, visited, depth);
                if !s.trim().is_empty() {
                    parts.push(s);
                }
            }
        }
        if parts.is_empty() {
            for field in &variant.fields {
                let val = fields.get(&field.id).unwrap_or(&field.default);
                let s = val.to_short_view_string_inner(&field.ty, dtos, enums, visited, depth);
                if !s.trim().is_empty() {
                    parts.push(s);
                }
            }
        }
        format!("{v_pascal}({})", parts.join(" | "))
    }

    /// Populates any missing nested field keys in `FieldType::Dto` and `FieldType::Enum` default values across all variant fields.
    pub fn fill_defaults_with_schemas(
        &mut self,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) {
        let mut visited = BTreeSet::new();
        if !self.id.is_nil() {
            visited.insert(self.id);
        }
        for variant in &mut self.variants {
            for field in &mut variant.fields {
                fill_field_default(field, dtos, enums, &mut visited, 0);
            }
        }
    }

    /// Removes unknown nested field UUID keys from `FieldType::Dto` and `FieldType::Enum` default values across all variant fields.
    pub fn prune_unknown_fields_with_schemas(
        &mut self,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) -> usize {
        let mut visited = BTreeSet::new();
        if !self.id.is_nil() {
            visited.insert(self.id);
        }
        let mut removed = 0;
        for variant in &mut self.variants {
            for field in &mut variant.fields {
                removed += prune_field_default(field, dtos, enums, &mut visited, 0);
            }
        }
        removed
    }

    /// Sorts `Set` and `Map` default values across all variant fields using [`DataValue::cmp_key`].
    pub fn sort_container_keys(&mut self, dtos: &BTreeMap<String, DtoAsset>, enums: &BTreeMap<String, EnumAsset>) {
        for variant in &mut self.variants {
            for field in &mut variant.fields {
                field.default.sort_container_keys(Some(&field.ty), dtos, enums);
            }
        }
    }

    /// Resolves `(FieldType, FieldSettings, effective_default_value, is_name)` at a field UUID path starting from a
    /// variant's `fields` (or any variant in `self` if `variant_id` is nil).
    pub fn variant_field_meta_at_path(
        &self,
        variant_id: AssetUuid,
        path: &[AssetUuid],
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
        use_type_default_at_root: bool,
    ) -> Option<(FieldType, FieldSettings, DataValue, bool)> {
        if path.is_empty() || path.len() > MAX_NESTING_DEPTH {
            return None;
        }
        let f0 = self
            .variant_by_id(variant_id)
            .and_then(|v| v.field_by_id(path[0]))
            .or_else(|| self.find_variant_field(path[0]).map(|(_, f)| f))?;
        let root_def = if use_type_default_at_root {
            f0.ty.default_value_with_schemas(dtos, enums)
        } else {
            f0.default_with_schemas(dtos, enums)
        };
        if path.len() == 1 {
            return Some((f0.ty.clone(), f0.settings.clone(), root_def, f0.is_name));
        }
        let mut visited = BTreeSet::new();
        if !self.id.is_nil() {
            visited.insert(self.id);
        }
        resolve_sub_path_meta(f0.ty.clone(), f0.settings.clone(), root_def, &path[1..], dtos, enums, &mut visited)
    }
}

/// A `.dto.ron` schema asset (ADR 0049 §3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DtoAsset {
    pub format_version: u32,
    /// The asset's UUIDv7 (ADR 0047 §2); nil until first saved.
    #[serde(default, skip_serializing_if = "AssetUuid::is_nil")]
    pub id: AssetUuid,
    /// Optional documentation comment emitted on the generated Rust struct.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc: String,
    /// Ordered schema fields.
    #[serde(default)]
    pub fields: Vec<Field>,
}

impl Default for DtoAsset {
    fn default() -> Self {
        Self::new()
    }
}

impl DtoAsset {
    /// Creates an empty `.dto.ron` model at [`DTO_FORMAT_VERSION`].
    pub fn new() -> Self {
        DtoAsset { format_version: DTO_FORMAT_VERSION, id: AssetUuid::NIL, doc: String::new(), fields: Vec::new() }
    }

    /// Mints UUIDv7s for the root `id` and any field whose `id` is nil or non-v7; returns the root `id`.
    pub fn ensure_ids(&mut self) -> AssetUuid {
        for field in &mut self.fields {
            field.id.ensure();
        }
        self.id.ensure()
    }

    /// Alias of [`DtoAsset::ensure_ids`] matching other domain models.
    pub fn ensure_id(&mut self) -> AssetUuid {
        self.ensure_ids()
    }

    /// Mints fresh UUIDv7s for every field (used when regenerating the UUID of a copied `.dto.ron` file so its field
    /// sub-items do not collide with the original DTO's).
    pub fn regenerate_sub_ids(&mut self) {
        for field in &mut self.fields {
            field.id = AssetUuid::new_v7();
        }
    }

    /// Looks up a field by its [`AssetUuid`].
    pub fn field_by_id(&self, id: AssetUuid) -> Option<&Field> {
        self.fields.iter().find(|f| f.id == id)
    }

    /// Looks up a field by its `name`.
    pub fn field_by_name(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Groups fields by [`Field::effective_category`], preserving the order in which each category first appears.
    pub fn categories(&self) -> Vec<(&str, Vec<(usize, &Field)>)> {
        let mut out: Vec<(&str, Vec<(usize, &Field)>)> = Vec::new();
        for (idx, field) in self.fields.iter().enumerate() {
            let cat = field.effective_category();
            if let Some((_, group)) = out.iter_mut().find(|(c, _)| *c == cat) {
                group.push((idx, field));
            } else {
                out.push((cat, vec![(idx, field)]));
            }
        }
        out
    }

    /// Computes the short display name string of `row` according to this DTO's `is_name` flags (ADR 0049 §3):
    /// joins non-empty `to_short_view_string()` values of all `is_name: true` fields in schema order with `" | "`,
    /// falling back to the first non-empty string field or `"row"` (`"row #<idx>"` in [`Self::short_view_string_indexed`]).
    pub fn short_view_string<V: HasFieldValues + ?Sized>(&self, row: &V) -> String {
        self.short_view_string_indexed(row, None)
    }

    /// Like [`Self::short_view_string`], resolving nested `FieldType::Dto` fields via `dtos`.
    pub fn short_view_string_with_dtos<V: HasFieldValues + ?Sized>(
        &self,
        row: &V,
        dtos: &BTreeMap<String, DtoAsset>,
    ) -> String {
        self.short_view_string_indexed_with_dtos(row, None, dtos)
    }

    /// Like [`Self::short_view_string`], resolving nested `FieldType::Dto` and `FieldType::Enum` fields via `dtos` and `enums`.
    pub fn short_view_string_with_schemas<V: HasFieldValues + ?Sized>(
        &self,
        row: &V,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) -> String {
        self.short_view_string_indexed_with_schemas(row, None, dtos, enums)
    }

    /// Like [`Self::short_view_string`], using `"row #<idx>"` as the final fallback when `row_index` is provided.
    pub fn short_view_string_indexed<V: HasFieldValues + ?Sized>(&self, row: &V, row_index: Option<usize>) -> String {
        let empty = BTreeMap::new();
        self.short_view_string_indexed_with_dtos(row, row_index, &empty)
    }

    /// Like [`Self::short_view_string_indexed`], resolving nested `FieldType::Dto` fields via `dtos`.
    pub fn short_view_string_indexed_with_dtos<V: HasFieldValues + ?Sized>(
        &self,
        row: &V,
        row_index: Option<usize>,
        dtos: &BTreeMap<String, DtoAsset>,
    ) -> String {
        let empty = BTreeMap::new();
        self.short_view_string_indexed_with_schemas(row, row_index, dtos, &empty)
    }

    /// Like [`Self::short_view_string_indexed`], resolving nested `FieldType::Dto` and `FieldType::Enum` fields via `dtos` and `enums`.
    pub fn short_view_string_indexed_with_schemas<V: HasFieldValues + ?Sized>(
        &self,
        row: &V,
        row_index: Option<usize>,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) -> String {
        let mut visited = BTreeSet::new();
        if !self.id.is_nil() {
            visited.insert(self.id);
        }
        self.short_view_string_inner(row.field_values(), row_index, None, dtos, enums, &mut visited, 0)
    }

    #[allow(clippy::too_many_arguments)]
    fn short_view_string_inner(
        &self,
        values: &BTreeMap<AssetUuid, DataValue>,
        row_index: Option<usize>,
        dto_path_hint: Option<&str>,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
        visited: &mut BTreeSet<AssetUuid>,
        depth: usize,
    ) -> String {
        let mut parts = Vec::new();
        for field in self.fields.iter().filter(|f| f.is_name) {
            let val = values.get(&field.id).unwrap_or(&field.default);
            let s = val.to_short_view_string_inner(&field.ty, dtos, enums, visited, depth);
            if !s.trim().is_empty() {
                parts.push(s);
            }
        }
        if !parts.is_empty() {
            return parts.join(" | ");
        }
        for field in self.fields.iter().filter(|f| f.ty.is_string()) {
            let val = values.get(&field.id).unwrap_or(&field.default);
            let s = val.to_short_view_string_inner(&field.ty, dtos, enums, visited, depth);
            if !s.trim().is_empty() {
                return s;
            }
        }
        if let Some(path) = dto_path_hint {
            return to_pascal_case(path);
        }
        match row_index {
            Some(idx) => format!("row #{idx}"),
            None => "row".to_string(),
        }
    }

    /// Populates any missing nested field keys in `FieldType::Dto` default values across `self.fields`.
    pub fn fill_defaults_with_dtos(&mut self, dtos: &BTreeMap<String, DtoAsset>) {
        self.fill_defaults_with_schemas(dtos, &BTreeMap::new());
    }

    /// Populates any missing nested field keys in `FieldType::Dto` and `FieldType::Enum` default values across `self.fields`.
    pub fn fill_defaults_with_schemas(
        &mut self,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) {
        let mut visited = BTreeSet::new();
        if !self.id.is_nil() {
            visited.insert(self.id);
        }
        for field in &mut self.fields {
            fill_field_default(field, dtos, enums, &mut visited, 0);
        }
    }

    /// Removes unknown nested field UUID keys from `FieldType::Dto` default values across `self.fields`.
    pub fn prune_unknown_fields_with_dtos(&mut self, dtos: &BTreeMap<String, DtoAsset>) -> usize {
        self.prune_unknown_fields_with_schemas(dtos, &BTreeMap::new())
    }

    /// Removes unknown nested field UUID keys from `FieldType::Dto` and `FieldType::Enum` default values across `self.fields`.
    pub fn prune_unknown_fields_with_schemas(
        &mut self,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) -> usize {
        let mut visited = BTreeSet::new();
        if !self.id.is_nil() {
            visited.insert(self.id);
        }
        let mut removed = 0;
        for field in &mut self.fields {
            removed += prune_field_default(field, dtos, enums, &mut visited, 0);
        }
        removed
    }

    /// Sorts `Set` and `Map` default values across `self.fields` using [`DataValue::cmp_key`].
    pub fn sort_container_keys(&mut self, dtos: &BTreeMap<String, DtoAsset>, enums: &BTreeMap<String, EnumAsset>) {
        for field in &mut self.fields {
            field.default.sort_container_keys(Some(&field.ty), dtos, enums);
        }
    }

    /// Resolves `(FieldType, FieldSettings, effective_default_value, is_name)` at a field UUID path starting from
    /// `self.fields`. When `use_type_default_at_root` is `true` (used by `.dto.ron` default editing), the root field's
    /// baseline default is `f0.ty.default_value_with_dtos(dtos)`; when `false` (used by `.collection.ron` cell editing),
    /// the root field's baseline default is `f0.default_with_dtos(dtos)`.
    pub fn field_meta_at_path(
        &self,
        path: &[AssetUuid],
        dtos: &BTreeMap<String, DtoAsset>,
        use_type_default_at_root: bool,
    ) -> Option<(FieldType, FieldSettings, DataValue, bool)> {
        self.field_meta_at_path_with_schemas(path, dtos, &BTreeMap::new(), use_type_default_at_root)
    }

    /// Like [`Self::field_meta_at_path`], also resolving nested `FieldType::Enum` payload fields via `enums`.
    pub fn field_meta_at_path_with_schemas(
        &self,
        path: &[AssetUuid],
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
        use_type_default_at_root: bool,
    ) -> Option<(FieldType, FieldSettings, DataValue, bool)> {
        if path.is_empty() || path.len() > MAX_NESTING_DEPTH {
            return None;
        }
        let f0 = self.field_by_id(path[0])?;
        let root_def = if use_type_default_at_root {
            f0.ty.default_value_with_schemas(dtos, enums)
        } else {
            f0.default_with_schemas(dtos, enums)
        };
        if path.len() == 1 {
            return Some((f0.ty.clone(), f0.settings.clone(), root_def, f0.is_name));
        }

        let mut visited = BTreeSet::new();
        if !self.id.is_nil() {
            visited.insert(self.id);
        }
        resolve_sub_path_meta(f0.ty.clone(), f0.settings.clone(), root_def, &path[1..], dtos, enums, &mut visited)
    }
}

fn resolve_sub_path_meta(
    mut cur_ty: FieldType,
    mut cur_settings: FieldSettings,
    mut cur_def: DataValue,
    sub_path: &[AssetUuid],
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    visited: &mut BTreeSet<AssetUuid>,
) -> Option<(FieldType, FieldSettings, DataValue, bool)> {
    for (depth, &step_id) in sub_path.iter().enumerate() {
        if depth + 1 >= MAX_NESTING_DEPTH {
            return None;
        }
        let (next_ty, next_settings, next_def, next_is_name) = match (decode_path_step(step_id), cur_ty) {
            (PathStep::Field(fid), FieldType::Dto(target_uuid)) => {
                if target_uuid.is_nil() || !visited.insert(target_uuid) {
                    return None;
                }
                let (_, target_dto) = dtos.iter().find(|(_, d)| !d.id.is_nil() && d.id == target_uuid)?;
                let child_f = target_dto.field_by_id(fid)?;
                let next_def = cur_def
                    .as_dto_map()
                    .and_then(|m| m.get(&fid))
                    .cloned()
                    .unwrap_or_else(|| child_f.default_with_schemas(dtos, enums));
                (child_f.ty.clone(), child_f.settings.clone(), next_def, child_f.is_name)
            }
            (PathStep::Field(fid), FieldType::Enum(target_uuid)) => {
                if target_uuid.is_nil() || !visited.insert(target_uuid) {
                    return None;
                }
                let (_, target_enum) = enums.iter().find(|(_, e)| !e.id.is_nil() && e.id == target_uuid)?;
                let (variant, child_f) = target_enum.find_variant_field(fid)?;
                let next_def = cur_def
                    .as_enum()
                    .filter(|(vid, _)| *vid == variant.id)
                    .and_then(|(_, m)| m.get(&fid))
                    .cloned()
                    .unwrap_or_else(|| child_f.default_with_schemas(dtos, enums));
                (child_f.ty.clone(), child_f.settings.clone(), next_def, child_f.is_name)
            }
            (PathStep::OptionSome, FieldType::Option(inner_ty)) => {
                visited.clear();
                let next_def = match &cur_def {
                    DataValue::Option(Some(v)) => (**v).clone(),
                    _ => inner_ty.default_value_with_schemas(dtos, enums),
                };
                ((*inner_ty).clone(), cur_settings.clone(), next_def, false)
            }
            (PathStep::Item(idx), FieldType::Array(inner_ty) | FieldType::Set(inner_ty)) => {
                visited.clear();
                let next_def = match &cur_def {
                    DataValue::Array(items) | DataValue::Set(items) => {
                        items.get(idx).cloned().unwrap_or_else(|| inner_ty.default_value_with_schemas(dtos, enums))
                    }
                    _ => inner_ty.default_value_with_schemas(dtos, enums),
                };
                ((*inner_ty).clone(), FieldSettings::default(), next_def, false)
            }
            (PathStep::MapKey(idx), FieldType::Map(key_ty, _)) => {
                visited.clear();
                let next_def = match &cur_def {
                    DataValue::Map(entries) => entries
                        .get(idx)
                        .map(|(k, _)| k.clone())
                        .unwrap_or_else(|| key_ty.default_value_with_schemas(dtos, enums)),
                    _ => key_ty.default_value_with_schemas(dtos, enums),
                };
                ((*key_ty).clone(), FieldSettings::default(), next_def, false)
            }
            (PathStep::MapVal(idx), FieldType::Map(_, val_ty)) => {
                visited.clear();
                let next_def = match &cur_def {
                    DataValue::Map(entries) => entries
                        .get(idx)
                        .map(|(_, v)| v.clone())
                        .unwrap_or_else(|| val_ty.default_value_with_schemas(dtos, enums)),
                    _ => val_ty.default_value_with_schemas(dtos, enums),
                };
                ((*val_ty).clone(), FieldSettings::default(), next_def, false)
            }
            _ => return None,
        };
        if depth + 1 == sub_path.len() {
            return Some((next_ty, next_settings, next_def, next_is_name));
        }
        cur_ty = next_ty;
        cur_settings = next_settings;
        cur_def = next_def;
    }
    None
}

/// Navigates into nested [`DataValue::Dto`] maps along `sub_path`, initializing any missing intermediate maps with
/// dense defaults from `dtos` and returning `&mut DataValue` for the target entry.
pub fn ensure_nested_value_mut<'a>(
    cur_val: &'a mut DataValue,
    cur_ty: FieldType,
    dtos: &BTreeMap<String, DtoAsset>,
    sub_path: &[AssetUuid],
) -> Option<&'a mut DataValue> {
    ensure_nested_value_with_schemas_mut(cur_val, cur_ty, dtos, &BTreeMap::new(), sub_path)
}

/// Navigates into nested [`DataValue::Dto`], [`DataValue::Enum`], [`DataValue::Option`], [`DataValue::Array`],
/// [`DataValue::Set`], and [`DataValue::Map`] values along `sub_path`, initializing any missing intermediate DTO/Enum
/// maps or `Option(Some)` with defaults from `dtos` and `enums` and returning `&mut DataValue` for the target entry.
pub fn ensure_nested_value_with_schemas_mut<'a>(
    mut cur_val: &'a mut DataValue,
    mut cur_ty: FieldType,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    sub_path: &[AssetUuid],
) -> Option<&'a mut DataValue> {
    if sub_path.len() > MAX_NESTING_DEPTH {
        return None;
    }
    for &step_id in sub_path {
        match (decode_path_step(step_id), cur_ty) {
            (PathStep::Field(fid), FieldType::Dto(target_uuid)) => {
                let (_, target_dto) = dtos.iter().find(|(_, d)| !d.id.is_nil() && d.id == target_uuid)?;
                let child_f = target_dto.field_by_id(fid)?;
                if !matches!(cur_val, DataValue::Dto(_)) {
                    *cur_val = FieldType::Dto(target_uuid).default_value_with_schemas(dtos, enums);
                }
                let DataValue::Dto(map) = cur_val else {
                    return None;
                };
                for tf in &target_dto.fields {
                    if !tf.id.is_nil() {
                        map.entry(tf.id).or_insert_with(|| tf.default_with_schemas(dtos, enums));
                    }
                }
                cur_val = map.get_mut(&fid)?;
                cur_ty = child_f.ty.clone();
            }
            (PathStep::Field(fid), FieldType::Enum(target_uuid)) => {
                let (_, target_enum) = enums.iter().find(|(_, e)| !e.id.is_nil() && e.id == target_uuid)?;
                let (target_variant, child_f) = target_enum.find_variant_field(fid)?;
                if !matches!(cur_val, DataValue::Enum { .. }) {
                    *cur_val = DataValue::Enum { variant: target_variant.id, fields: BTreeMap::new() };
                }
                let DataValue::Enum { variant, fields } = cur_val else {
                    return None;
                };
                if *variant != target_variant.id {
                    *variant = target_variant.id;
                    fields.clear();
                }
                for tf in &target_variant.fields {
                    if !tf.id.is_nil() {
                        fields.entry(tf.id).or_insert_with(|| tf.default_with_schemas(dtos, enums));
                    }
                }
                cur_val = fields.get_mut(&fid)?;
                cur_ty = child_f.ty.clone();
            }
            (PathStep::OptionSome, FieldType::Option(inner_ty)) => {
                if !matches!(cur_val, DataValue::Option(Some(_))) {
                    *cur_val = DataValue::Option(Some(Box::new(inner_ty.default_value_with_schemas(dtos, enums))));
                }
                let DataValue::Option(Some(inner)) = cur_val else {
                    return None;
                };
                cur_val = inner.as_mut();
                cur_ty = *inner_ty;
            }
            (PathStep::Item(idx), FieldType::Array(inner_ty)) => {
                let DataValue::Array(items) = cur_val else {
                    return None;
                };
                cur_val = items.get_mut(idx)?;
                cur_ty = *inner_ty;
            }
            (PathStep::Item(idx), FieldType::Set(inner_ty)) => {
                let DataValue::Set(items) = cur_val else {
                    return None;
                };
                cur_val = items.get_mut(idx)?;
                cur_ty = *inner_ty;
            }
            (PathStep::MapKey(idx), FieldType::Map(key_ty, _)) => {
                let DataValue::Map(entries) = cur_val else {
                    return None;
                };
                cur_val = &mut entries.get_mut(idx)?.0;
                cur_ty = *key_ty;
            }
            (PathStep::MapVal(idx), FieldType::Map(_, val_ty)) => {
                let DataValue::Map(entries) = cur_val else {
                    return None;
                };
                cur_val = &mut entries.get_mut(idx)?.1;
                cur_ty = *val_ty;
            }
            _ => return None,
        }
    }
    Some(cur_val)
}

fn fill_value_for_type(
    val: &mut DataValue,
    ty: &FieldType,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    visited: &mut BTreeSet<AssetUuid>,
    depth: usize,
) {
    if depth >= MAX_NESTING_DEPTH {
        return;
    }
    match ty {
        FieldType::Dto(target_uuid) => {
            if !matches!(val, DataValue::Dto(_)) {
                *val = DataValue::Dto(BTreeMap::new());
            }
            if let DataValue::Dto(nested_map) = val {
                fill_target_dto_map_defaults(nested_map, *target_uuid, dtos, enums, visited, depth);
            }
        }
        FieldType::Enum(target_uuid) => {
            if !matches!(val, DataValue::Enum { .. }) {
                *val = DataValue::Enum { variant: AssetUuid::NIL, fields: BTreeMap::new() };
            }
            if let DataValue::Enum { variant, fields } = val {
                fill_target_enum_defaults(variant, fields, *target_uuid, dtos, enums, visited, depth);
            }
        }
        FieldType::Option(inner_ty) => {
            if !matches!(val, DataValue::Option(_)) {
                *val = DataValue::Option(None);
            }
            if let DataValue::Option(Some(inner_val)) = val {
                let mut sub_visited = BTreeSet::new();
                if !inner_ty.matches(inner_val) {
                    **inner_val = inner_ty.default_value_with_schemas(dtos, enums);
                } else {
                    fill_value_for_type(inner_val, inner_ty, dtos, enums, &mut sub_visited, depth + 1);
                }
            }
        }
        FieldType::Array(inner_ty) => {
            if !matches!(val, DataValue::Array(_)) {
                *val = DataValue::Array(Vec::new());
            }
            if let DataValue::Array(items) = val {
                for item in items {
                    let mut sub_visited = BTreeSet::new();
                    fill_value_for_type(item, inner_ty, dtos, enums, &mut sub_visited, depth + 1);
                }
            }
        }
        FieldType::Set(key_ty) => {
            if !matches!(val, DataValue::Set(_)) {
                *val = DataValue::Set(Vec::new());
            }
            if let DataValue::Set(items) = val {
                for item in items {
                    let mut sub_visited = BTreeSet::new();
                    fill_value_for_type(item, key_ty, dtos, enums, &mut sub_visited, depth + 1);
                }
            }
        }
        FieldType::Map(key_ty, val_ty) => {
            if !matches!(val, DataValue::Map(_)) {
                *val = DataValue::Map(Vec::new());
            }
            if let DataValue::Map(entries) = val {
                for (k, v) in entries {
                    let mut k_visited = BTreeSet::new();
                    let mut v_visited = BTreeSet::new();
                    fill_value_for_type(k, key_ty, dtos, enums, &mut k_visited, depth + 1);
                    fill_value_for_type(v, val_ty, dtos, enums, &mut v_visited, depth + 1);
                }
            }
        }
        _ => {}
    }
}

fn fill_field_default(
    field: &mut Field,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    visited: &mut BTreeSet<AssetUuid>,
    depth: usize,
) {
    fill_value_for_type(&mut field.default, &field.ty, dtos, enums, visited, depth);
}

fn prune_value_for_type(
    val: &mut DataValue,
    ty: &FieldType,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    visited: &mut BTreeSet<AssetUuid>,
    depth: usize,
) -> usize {
    if depth >= MAX_NESTING_DEPTH {
        return 0;
    }
    match (ty, val) {
        (FieldType::Dto(target_uuid), DataValue::Dto(nested_map)) => {
            prune_target_dto_map_unknown_fields(nested_map, *target_uuid, dtos, enums, visited, depth)
        }
        (FieldType::Enum(target_uuid), DataValue::Enum { variant, fields }) => {
            prune_target_enum_unknown_fields(*variant, fields, *target_uuid, dtos, enums, visited, depth)
        }
        (FieldType::Option(inner_ty), DataValue::Option(Some(inner_val))) => {
            let mut sub_visited = BTreeSet::new();
            prune_value_for_type(inner_val, inner_ty, dtos, enums, &mut sub_visited, depth + 1)
        }
        (FieldType::Array(inner_ty) | FieldType::Set(inner_ty), DataValue::Array(items) | DataValue::Set(items)) => {
            let mut removed = 0;
            for item in items {
                let mut sub_visited = BTreeSet::new();
                removed += prune_value_for_type(item, inner_ty, dtos, enums, &mut sub_visited, depth + 1);
            }
            removed
        }
        (FieldType::Map(key_ty, val_ty), DataValue::Map(entries)) => {
            let mut removed = 0;
            for (k, v) in entries {
                let mut k_visited = BTreeSet::new();
                let mut v_visited = BTreeSet::new();
                removed += prune_value_for_type(k, key_ty, dtos, enums, &mut k_visited, depth + 1);
                removed += prune_value_for_type(v, val_ty, dtos, enums, &mut v_visited, depth + 1);
            }
            removed
        }
        _ => 0,
    }
}

fn prune_field_default(
    field: &mut Field,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    visited: &mut BTreeSet<AssetUuid>,
    depth: usize,
) -> usize {
    prune_value_for_type(&mut field.default, &field.ty, dtos, enums, visited, depth)
}

fn fill_target_dto_map_defaults(
    map: &mut BTreeMap<AssetUuid, DataValue>,
    target_uuid: AssetUuid,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    visited: &mut BTreeSet<AssetUuid>,
    depth: usize,
) {
    if depth >= MAX_NESTING_DEPTH || target_uuid.is_nil() {
        return;
    }
    let Some((_, target_dto)) = dtos.iter().find(|(_, d)| !d.id.is_nil() && d.id == target_uuid) else {
        return;
    };
    if !visited.insert(target_uuid) {
        return;
    }
    fill_dto_map_defaults(map, target_dto, dtos, enums, visited, depth + 1);
    visited.remove(&target_uuid);
}

fn fill_dto_map_defaults(
    map: &mut BTreeMap<AssetUuid, DataValue>,
    dto: &DtoAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    visited: &mut BTreeSet<AssetUuid>,
    depth: usize,
) {
    fill_fields_slice_defaults(map, &dto.fields, dtos, enums, visited, depth);
}

fn fill_target_enum_defaults(
    variant_id: &mut AssetUuid,
    fields: &mut BTreeMap<AssetUuid, DataValue>,
    target_uuid: AssetUuid,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    visited: &mut BTreeSet<AssetUuid>,
    depth: usize,
) {
    if depth >= MAX_NESTING_DEPTH || target_uuid.is_nil() {
        return;
    }
    let Some((_, target_enum)) = enums.iter().find(|(_, e)| !e.id.is_nil() && e.id == target_uuid) else {
        return;
    };
    if !visited.insert(target_uuid) {
        return;
    }
    let active_variant = target_enum.variant_by_id(*variant_id).or_else(|| target_enum.variants.first());
    if let Some(v) = active_variant {
        if *variant_id != v.id {
            *variant_id = v.id;
            fields.clear();
        }
        fill_fields_slice_defaults(fields, &v.fields, dtos, enums, visited, depth + 1);
    }
    visited.remove(&target_uuid);
}

fn fill_fields_slice_defaults(
    map: &mut BTreeMap<AssetUuid, DataValue>,
    schema_fields: &[Field],
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    visited: &mut BTreeSet<AssetUuid>,
    depth: usize,
) {
    for field in schema_fields {
        if field.id.is_nil() {
            continue;
        }
        let entry = map.entry(field.id).or_insert_with(|| field.default.clone());
        match &field.ty {
            FieldType::Dto(target_uuid) => {
                if !matches!(entry, DataValue::Dto(_)) {
                    *entry = field.default.clone();
                }
                if let DataValue::Dto(nested_map) = entry {
                    if let DataValue::Dto(def_map) = &field.default {
                        for (k, v) in def_map {
                            nested_map.entry(*k).or_insert_with(|| v.clone());
                        }
                    }
                    fill_target_dto_map_defaults(nested_map, *target_uuid, dtos, enums, visited, depth);
                }
            }
            FieldType::Enum(target_uuid) => {
                if !matches!(entry, DataValue::Enum { .. }) {
                    *entry = field.default.clone();
                }
                if let DataValue::Enum { variant, fields } = entry {
                    if variant.is_nil()
                        && let DataValue::Enum { variant: def_v, fields: def_f } = &field.default
                        && !def_v.is_nil()
                    {
                        *variant = *def_v;
                        for (k, v) in def_f {
                            fields.entry(*k).or_insert_with(|| v.clone());
                        }
                    }
                    fill_target_enum_defaults(variant, fields, *target_uuid, dtos, enums, visited, depth);
                }
            }
            other if other.is_container() => {
                fill_value_for_type(entry, other, dtos, enums, visited, depth);
            }
            _ => {}
        }
    }
}

fn prune_target_dto_map_unknown_fields(
    map: &mut BTreeMap<AssetUuid, DataValue>,
    target_uuid: AssetUuid,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    visited: &mut BTreeSet<AssetUuid>,
    depth: usize,
) -> usize {
    if depth >= MAX_NESTING_DEPTH || target_uuid.is_nil() {
        return 0;
    }
    let Some((_, target_dto)) = dtos.iter().find(|(_, d)| !d.id.is_nil() && d.id == target_uuid) else {
        return 0;
    };
    if !visited.insert(target_uuid) {
        return 0;
    }
    let removed = prune_dto_map_unknown_fields(map, target_dto, dtos, enums, visited, depth + 1);
    visited.remove(&target_uuid);
    removed
}

fn prune_dto_map_unknown_fields(
    map: &mut BTreeMap<AssetUuid, DataValue>,
    dto: &DtoAsset,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    visited: &mut BTreeSet<AssetUuid>,
    depth: usize,
) -> usize {
    prune_fields_slice_unknown_fields(map, &dto.fields, dtos, enums, visited, depth)
}

fn prune_target_enum_unknown_fields(
    variant_id: AssetUuid,
    fields: &mut BTreeMap<AssetUuid, DataValue>,
    target_uuid: AssetUuid,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    visited: &mut BTreeSet<AssetUuid>,
    depth: usize,
) -> usize {
    if depth >= MAX_NESTING_DEPTH || target_uuid.is_nil() {
        return 0;
    }
    let Some((_, target_enum)) = enums.iter().find(|(_, e)| !e.id.is_nil() && e.id == target_uuid) else {
        return 0;
    };
    let Some(variant) = target_enum.variant_by_id(variant_id) else {
        return 0;
    };
    if !visited.insert(target_uuid) {
        return 0;
    }
    let removed = prune_fields_slice_unknown_fields(fields, &variant.fields, dtos, enums, visited, depth + 1);
    visited.remove(&target_uuid);
    removed
}

fn prune_fields_slice_unknown_fields(
    map: &mut BTreeMap<AssetUuid, DataValue>,
    schema_fields: &[Field],
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    visited: &mut BTreeSet<AssetUuid>,
    depth: usize,
) -> usize {
    let known: BTreeSet<AssetUuid> = schema_fields.iter().map(|f| f.id).collect();
    let before = map.len();
    map.retain(|k, _| known.contains(k));
    let mut removed = before - map.len();
    for field in schema_fields {
        if let Some(val) = map.get_mut(&field.id) {
            removed += prune_value_for_type(val, &field.ty, dtos, enums, visited, depth);
        }
    }
    removed
}

/// One UUIDv7-identified row in a [`CollectionAsset`] (ADR 0049 §4).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Row {
    /// Project-unique UUIDv7 of this row.
    #[serde(default, skip_serializing_if = "AssetUuid::is_nil")]
    pub id: AssetUuid,
    /// Dense field values keyed by [`Field::id`].
    #[serde(default)]
    pub values: BTreeMap<AssetUuid, DataValue>,
}

impl Row {
    /// Creates a new row with a freshly minted UUIDv7 and all field defaults copied from `dto`.
    pub fn from_dto_defaults(dto: &DtoAsset) -> Self {
        let mut values = BTreeMap::new();
        for field in &dto.fields {
            if !field.id.is_nil() {
                values.insert(field.id, field.default.clone());
            }
        }
        Row { id: AssetUuid::new_v7(), values }
    }

    /// Creates a new row with a freshly minted UUIDv7 and all field defaults (including nested `FieldType::Dto`
    /// sub-trees) recursively populated from `dto` and `dtos`.
    pub fn from_dto_defaults_with_dtos(dto: &DtoAsset, dtos: &BTreeMap<String, DtoAsset>) -> Self {
        Self::from_dto_defaults_with_schemas(dto, dtos, &BTreeMap::new())
    }

    /// Creates a new row with a freshly minted UUIDv7 and all field defaults (including nested `FieldType::Dto`
    /// and `FieldType::Enum` sub-trees) recursively populated from `dto`, `dtos`, and `enums`.
    pub fn from_dto_defaults_with_schemas(
        dto: &DtoAsset,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) -> Self {
        let mut row = Self::from_dto_defaults(dto);
        let mut visited = BTreeSet::new();
        if !dto.id.is_nil() {
            visited.insert(dto.id);
        }
        fill_dto_map_defaults(&mut row.values, dto, dtos, enums, &mut visited, 0);
        row
    }

    /// Computes the short display name string of `self` according to `dto` and `dtos`.
    pub fn short_view_string(&self, dto: &DtoAsset, dtos: &BTreeMap<String, DtoAsset>) -> String {
        dto.short_view_string_with_dtos(self, dtos)
    }

    /// Computes the short display name string of `self` according to `dto`, `dtos`, and `enums`.
    pub fn short_view_string_with_schemas(
        &self,
        dto: &DtoAsset,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) -> String {
        dto.short_view_string_with_schemas(self, dtos, enums)
    }
}

/// A `.collection.ron` table asset (ADR 0049 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectionAsset {
    pub format_version: u32,
    /// The asset's UUIDv7 (ADR 0047 §2); nil until first saved.
    #[serde(default, skip_serializing_if = "AssetUuid::is_nil")]
    pub id: AssetUuid,
    /// Target `.dto.ron` asset UUID.
    #[serde(default)]
    pub dto: AssetUuid,
    /// Ordered rows of this collection.
    #[serde(default)]
    pub rows: Vec<Row>,
}

impl Default for CollectionAsset {
    fn default() -> Self {
        Self::new(AssetUuid::NIL)
    }
}

impl CollectionAsset {
    /// Creates an empty `.collection.ron` model targeting `dto` at [`COLLECTION_FORMAT_VERSION`].
    pub fn new(dto: AssetUuid) -> Self {
        CollectionAsset { format_version: COLLECTION_FORMAT_VERSION, id: AssetUuid::NIL, dto, rows: Vec::new() }
    }

    /// Mints UUIDv7s for the root `id` and any row whose `id` is nil or non-v7; returns the root `id`.
    pub fn ensure_ids(&mut self) -> AssetUuid {
        for row in &mut self.rows {
            row.id.ensure();
        }
        self.id.ensure()
    }

    /// Alias of [`CollectionAsset::ensure_ids`] matching other domain models.
    pub fn ensure_id(&mut self) -> AssetUuid {
        self.ensure_ids()
    }

    /// Mints fresh UUIDv7s for every row (used when regenerating the UUID of a copied `.collection.ron` file so its
    /// row UUIDs do not collide project-wide with the original collection's rows).
    pub fn regenerate_sub_ids(&mut self) {
        for row in &mut self.rows {
            row.id = AssetUuid::new_v7();
        }
    }

    /// Populates any missing field keys in `rows` with the current defaults from `dto` (dense storage, D31).
    pub fn fill_defaults(&mut self, dto: &DtoAsset) {
        let empty = BTreeMap::new();
        self.fill_defaults_with_dtos(dto, &empty);
    }

    /// Populates any missing field keys in `rows` (and nested `DataValue::Dto` sub-trees) with the current defaults
    /// from `dto` and `dtos` (dense storage, D31).
    pub fn fill_defaults_with_dtos(&mut self, dto: &DtoAsset, dtos: &BTreeMap<String, DtoAsset>) {
        self.fill_defaults_with_schemas(dto, dtos, &BTreeMap::new());
    }

    /// Populates any missing field keys in `rows` (and nested `DataValue::Dto` and `DataValue::Enum` sub-trees) with
    /// the current defaults from `dto`, `dtos`, and `enums` (dense storage, D31).
    pub fn fill_defaults_with_schemas(
        &mut self,
        dto: &DtoAsset,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) {
        let mut visited = BTreeSet::new();
        if !dto.id.is_nil() {
            visited.insert(dto.id);
        }
        for row in &mut self.rows {
            fill_dto_map_defaults(&mut row.values, dto, dtos, enums, &mut visited, 0);
        }
    }

    /// Removes any field UUID keys from `rows` that are not present in `dto.fields` (pruning `DW101` stale fields on
    /// save), returning the total number of removed entries.
    pub fn prune_unknown_fields(&mut self, dto: &DtoAsset) -> usize {
        let empty = BTreeMap::new();
        self.prune_unknown_fields_with_dtos(dto, &empty)
    }

    /// Removes any field UUID keys from `rows` (and nested `DataValue::Dto` sub-trees) that are not present in `dto`
    /// or `dtos`, returning the total number of removed entries.
    pub fn prune_unknown_fields_with_dtos(&mut self, dto: &DtoAsset, dtos: &BTreeMap<String, DtoAsset>) -> usize {
        self.prune_unknown_fields_with_schemas(dto, dtos, &BTreeMap::new())
    }

    /// Removes any field UUID keys from `rows` (and nested `DataValue::Dto` and `DataValue::Enum` sub-trees) that are
    /// not present in `dto`, `dtos`, or `enums`, returning the total number of removed entries.
    pub fn prune_unknown_fields_with_schemas(
        &mut self,
        dto: &DtoAsset,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) -> usize {
        let mut visited = BTreeSet::new();
        if !dto.id.is_nil() {
            visited.insert(dto.id);
        }
        let mut removed = 0;
        for row in &mut self.rows {
            removed += prune_dto_map_unknown_fields(&mut row.values, dto, dtos, enums, &mut visited, 0);
        }
        removed
    }

    /// Sorts `Set` and `Map` values across all `rows` using [`DataValue::cmp_key`].
    pub fn sort_container_keys(
        &mut self,
        dto: Option<&DtoAsset>,
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) {
        let bound_dto = dto.or_else(|| dtos.values().find(|d| !d.id.is_nil() && d.id == self.dto));
        for row in &mut self.rows {
            for (fid, val) in &mut row.values {
                let f_ty = bound_dto.and_then(|d| d.field_by_id(*fid)).map(|f| &f.ty);
                val.sort_container_keys(f_ty, dtos, enums);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_uuid(n: u128) -> AssetUuid {
        AssetUuid::from_u128(0x0196_3a10_0000_7000_8000_0000_0000_0000 | n)
    }

    #[test]
    fn all_field_types_match_defaults_and_coerce() {
        assert_eq!(FieldType::ALL_PRIMITIVES.len(), 25);
        assert_eq!(FieldType::all().len(), 38);
        for (idx, ty) in FieldType::all().iter().enumerate() {
            let def = ty.default_value();
            assert!(ty.matches(&def), "{ty:?} default {def:?} does not match");
            assert!(ty.same_kind(def.field_type()));
            assert!(!ty.label().is_empty());
            assert_eq!(ty.canonical_tag(), ty.label());
            assert_eq!(usize::from(ty.schema_tag()), idx);
            assert_eq!(def.coerce_to(ty), def);
        }

        // Enum helpers and same_kind across different target Enum UUIDs.
        let en1 = FieldType::Enum(test_uuid(1));
        let en2 = FieldType::Enum(test_uuid(2));
        assert!(en1.is_enum());
        assert_eq!(en1.enum_target(), Some(test_uuid(1)));
        assert_eq!(FieldType::Bool.enum_target(), None);
        assert!(en1.same_kind(&en2));
        let en_val = DataValue::Enum { variant: test_uuid(10), fields: BTreeMap::new() };
        assert!(en1.matches(&en_val));
        assert_eq!(en_val.coerce_to(&en2), en_val);

        // SoftRef helpers and same_kind across different target DTO UUIDs.
        let sr1 = FieldType::SoftRef(test_uuid(1));
        let sr2 = FieldType::SoftRef(test_uuid(2));
        assert!(sr1.is_soft_ref());
        assert_eq!(sr1.soft_ref_target(), Some(test_uuid(1)));
        assert_eq!(FieldType::Bool.soft_ref_target(), None);
        assert!(sr1.same_kind(&sr2));
        assert_eq!(sr1.default_value_with_dtos(&BTreeMap::new()), DataValue::SoftRef(AssetUuid::NIL));
        let sr_val = DataValue::SoftRef(test_uuid(500));
        assert!(sr1.matches(&sr_val));
        assert_eq!(sr_val.coerce_to(&sr2), sr_val);

        // String <-> LocalizedString coercion preserves text.
        let s = DataValue::String("The Shader Alchemist".into());
        assert_eq!(s.coerce_to(FieldType::LocalizedString), DataValue::LocalizedString("The Shader Alchemist".into()));
        // Numeric coercions preserve value and clamp.
        assert_eq!(DataValue::I32(42).coerce_to(FieldType::F32), DataValue::F32(42.0));
        assert_eq!(DataValue::F32(-5.0).coerce_to(FieldType::U32), DataValue::U32(0));
        assert_eq!(DataValue::F32(-2.0).coerce_to(FieldType::Duration), DataValue::Duration(0.0));
        // GameAsset sub-filter change preserves DataValue::GameAsset(path).
        let ga = DataValue::GameAsset("audio/theme.ogg".into());
        assert_eq!(ga.coerce_to(FieldType::GameAsset(GameAssetKind::Audio)), ga);
        // Incompatible types fall back to target default.
        assert_eq!(DataValue::Vec3([1.0, 2.0, 3.0]).coerce_to(FieldType::Bool), DataValue::Bool(false));
        assert_eq!(ga.coerce_to(FieldType::Curve), DataValue::Curve(AssetUuid::NIL));
    }

    #[test]
    fn game_asset_kind_labels_tags_and_extension_filters() {
        assert_eq!(GameAssetKind::ALL.len(), 6);
        for (idx, &kind) in GameAssetKind::ALL.iter().enumerate() {
            assert_eq!(usize::from(kind.schema_tag()), idx);
            assert_eq!(GameAssetKind::from_label(kind.label()), Some(kind));
            assert!(FieldType::GameAsset(kind).is_game_asset());
            assert!(FieldType::GameAsset(kind).is_asset_ref());
            assert_eq!(FieldType::GameAsset(kind).game_asset_kind(), Some(kind));
        }
        assert_eq!(GameAssetKind::from_label("Unknown"), None);
        assert!(GameAssetKind::Any.matches_path("anything/file.custom"));
        assert!(GameAssetKind::Image.matches_path("textures/hero.PNG"));
        assert!(GameAssetKind::Image.matches_path("textures/sky.ktx2"));
        assert!(!GameAssetKind::Image.matches_path("audio/click.ogg"));
        assert!(GameAssetKind::Audio.matches_path("audio/click.ogg"));
        assert!(GameAssetKind::Scene.matches_path("scenes/level1.scn.ron"));
        assert!(GameAssetKind::Gltf.matches_path("models/ship.glb"));
        assert!(GameAssetKind::Font.matches_path("fonts/mono.ttf"));
    }

    #[test]
    fn to_short_view_string_formats_all_types() {
        assert_eq!(DataValue::Bool(true).to_short_view_string(), "true");
        assert_eq!(DataValue::I32(-1928).to_short_view_string(), "-1928");
        assert_eq!(DataValue::U32(312).to_short_view_string(), "312");
        assert_eq!(DataValue::F32(15.0).to_short_view_string(), "15");
        assert_eq!(DataValue::F32(1.45).to_short_view_string(), "1.45");
        assert_eq!(DataValue::F64(2.50).to_short_view_string(), "2.5");
        assert_eq!(DataValue::String("A. Vance".into()).to_short_view_string(), "A. Vance");
        assert_eq!(DataValue::LocalizedString("Dune".into()).to_short_view_string(), "Dune");
        assert_eq!(DataValue::Vec2([1.5, 2.0]).to_short_view_string(), "(1.5, 2)");
        assert_eq!(DataValue::Vec3([14.8, 21.0, 2.8]).to_short_view_string(), "(14.8, 21, 2.8)");
        assert_eq!(DataValue::IVec2([-1, 4]).to_short_view_string(), "(-1, 4)");
        assert_eq!(DataValue::UVec3([1, 2, 3]).to_short_view_string(), "(1, 2, 3)");
        assert_eq!(DataValue::Quat([0.0, 0.0, 0.0, 1.0]).to_short_view_string(), "(0, 0, 0, 1)");
        assert_eq!(DataValue::Color([1.0, 1.0, 1.0, 1.0]).to_short_view_string(), "#FFFFFF");
        assert_eq!(DataValue::Color([0.0, 0.0, 0.0, 0.5]).to_short_view_string(), "#00000080");
        assert_eq!(DataValue::Duration(12.5).to_short_view_string(), "12.5s");
        assert_eq!(DataValue::RangeF32([0.25, 1.5]).to_short_view_string(), "0.25..1.5");
        assert_eq!(DataValue::RangeI32([1, 5]).to_short_view_string(), "1..5");
        assert_eq!(DataValue::Dto(BTreeMap::new()).to_short_view_string(), "{}");
        let u = test_uuid(7);
        assert_eq!(DataValue::Curve(AssetUuid::NIL).to_short_view_string(), "None");
        assert_eq!(DataValue::Curve(u).to_short_view_string(), u.to_string());
        assert_eq!(DataValue::Gradient(AssetUuid::NIL, 0).to_short_view_string(), "None");
        assert_eq!(DataValue::Gradient(u, 0).to_short_view_string(), "None");
        assert_eq!(DataValue::Gradient(u, 2).to_short_view_string(), format!("{u}:2"));
        assert_eq!(DataValue::Material(AssetUuid::NIL).to_short_view_string(), "None");
        assert_eq!(DataValue::Material(u).to_short_view_string(), u.to_string());
        assert_eq!(DataValue::Texture(AssetUuid::NIL).to_short_view_string(), "None");
        assert_eq!(DataValue::Texture(u).to_short_view_string(), u.to_string());
        assert_eq!(DataValue::LightEnvironment(AssetUuid::NIL).to_short_view_string(), "None");
        assert_eq!(DataValue::LightEnvironment(u).to_short_view_string(), u.to_string());
        assert_eq!(DataValue::GameAsset(String::new()).to_short_view_string(), "None");
        assert_eq!(DataValue::GameAsset("textures/cover.png".into()).to_short_view_string(), "textures/cover.png");
        assert_eq!(DataValue::SoftRef(AssetUuid::NIL).to_short_view_string(), "<none>");
        assert_eq!(DataValue::SoftRef(u).to_short_view_string(), u.to_string());
        assert_eq!(DataValue::Enum { variant: AssetUuid::NIL, fields: BTreeMap::new() }.to_short_view_string(), "None");
        assert_eq!(DataValue::Enum { variant: u, fields: BTreeMap::new() }.to_short_view_string(), u.to_string());
        assert_eq!(
            DataValue::Enum { variant: u, fields: BTreeMap::from([(test_uuid(8), DataValue::F32(12.5))]) }
                .to_short_view_string(),
            format!("{u}(12.5)")
        );
    }

    #[test]
    fn dto_short_view_string_joins_summary_fields_and_falls_back() {
        let f_title = Field {
            id: test_uuid(1),
            name: "title".into(),
            ty: FieldType::LocalizedString,
            category: "Identity".into(),
            is_name: true,
            default: DataValue::LocalizedString("Untitled".into()),
            settings: FieldSettings::default(),
        };
        let f_year = Field {
            id: test_uuid(2),
            name: "publish_year".into(),
            ty: FieldType::I32,
            category: "Identity".into(),
            is_name: true,
            default: DataValue::I32(1928),
            settings: FieldSettings::default(),
        };
        let f_pages = Field {
            id: test_uuid(3),
            name: "pages".into(),
            ty: FieldType::U32,
            category: "Physical".into(),
            is_name: true,
            default: DataValue::U32(312),
            settings: FieldSettings::default(),
        };
        let f_author = Field {
            id: test_uuid(4),
            name: "author".into(),
            ty: FieldType::String,
            category: String::new(),
            is_name: false,
            default: DataValue::String("A. Vance".into()),
            settings: FieldSettings::default(),
        };
        let mut dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: test_uuid(100),
            doc: String::new(),
            fields: vec![f_title, f_year, f_pages, f_author],
        };

        let mut row = Row::from_dto_defaults(&dto);
        row.values.insert(test_uuid(1), DataValue::LocalizedString("The Shader Alchemist".into()));
        assert_eq!(dto.short_view_string(&row), "The Shader Alchemist | 1928 | 312");
        assert_eq!(dto.short_view_string(&row.values), "The Shader Alchemist | 1928 | 312");

        // Empty summary string is skipped.
        row.values.insert(test_uuid(1), DataValue::LocalizedString(String::new()));
        assert_eq!(dto.short_view_string(&row), "1928 | 312");

        // When no field has `is_name: true`, falls back to first non-empty string field (`author`).
        for f in &mut dto.fields {
            f.is_name = false;
        }
        assert_eq!(dto.short_view_string(&row), "A. Vance");

        // When all string fields are empty too, falls back to `"row"` or `"row #<idx>"`.
        row.values.insert(test_uuid(4), DataValue::String(String::new()));
        assert_eq!(dto.short_view_string(&row), "row");
        assert_eq!(dto.short_view_string_indexed(&row, Some(3)), "row #3");

        // Categories preserve first-seen order and map `""` to `"General"`.
        let cats: Vec<_> = dto.categories().into_iter().map(|(c, fs)| (c, fs.len())).collect();
        assert_eq!(cats, vec![("Identity", 2), ("Physical", 1), ("General", 1)]);
    }

    #[test]
    fn nested_dto_defaults_short_view_and_recursive_fill_prune() {
        // 3-level nested DTO hierarchy: SectorCoords -> Planet -> Author -> Book
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
                default: DataValue::IVec3([1, 2, 3]),
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

        let mut col = CollectionAsset::new(book_dto.id);
        let mut row = Row { id: test_uuid(100), values: BTreeMap::new() };
        // Insert a stale top-level field and a stale nested field inside `author.homeworld`
        row.values.insert(test_uuid(999), DataValue::Bool(true));
        let mut homeworld_map = BTreeMap::new();
        homeworld_map.insert(test_uuid(888), DataValue::I32(7));
        let mut author_map = BTreeMap::new();
        author_map.insert(test_uuid(32), DataValue::Dto(homeworld_map));
        row.values.insert(test_uuid(42), DataValue::Dto(author_map));
        col.rows.push(row);

        col.fill_defaults_with_dtos(&book_dto, &dtos);
        assert_eq!(
            book_dto.short_view_string_with_dtos(&col.rows[0], &dtos),
            "The Shader Alchemist | A. Vance | Arrakis | (1, 2, 3)"
        );

        let pruned = col.prune_unknown_fields_with_dtos(&book_dto, &dtos);
        assert_eq!(pruned, 2);
    }

    #[test]
    fn enum_asset_helpers_short_view_defaults_and_prune() {
        let mut rarity = EnumAsset {
            format_version: ENUM_FORMAT_VERSION,
            id: test_uuid(50),
            doc: "Book rarity".into(),
            variants: vec![
                EnumVariant { id: test_uuid(51), name: "common".into(), doc: String::new(), fields: Vec::new() },
                EnumVariant { id: test_uuid(52), name: "rare".into(), doc: String::new(), fields: Vec::new() },
            ],
        };
        assert!(rarity.is_unit_only());
        assert_eq!(rarity.variant_by_name("rare").unwrap().id, test_uuid(52));

        let mut enchantment = EnumAsset {
            format_version: ENUM_FORMAT_VERSION,
            id: test_uuid(60),
            doc: "Enchantment union".into(),
            variants: vec![
                EnumVariant { id: test_uuid(61), name: "none".into(), doc: String::new(), fields: Vec::new() },
                EnumVariant {
                    id: test_uuid(62),
                    name: "ward".into(),
                    doc: String::new(),
                    fields: vec![
                        Field {
                            id: test_uuid(63),
                            name: "strength".into(),
                            ty: FieldType::F32,
                            category: String::new(),
                            is_name: true,
                            default: DataValue::F32(10.0),
                            settings: FieldSettings::default(),
                        },
                        Field {
                            id: test_uuid(64),
                            name: "school".into(),
                            ty: FieldType::LocalizedString,
                            category: String::new(),
                            is_name: true,
                            default: DataValue::LocalizedString("Arcane".into()),
                            settings: FieldSettings::default(),
                        },
                    ],
                },
            ],
        };
        assert!(!enchantment.is_unit_only());
        rarity.ensure_ids();
        enchantment.ensure_ids();

        let dtos = BTreeMap::new();
        let enums =
            BTreeMap::from([("rarity".to_string(), rarity.clone()), ("enchantment".to_string(), enchantment.clone())]);

        // Default for unit enum selects first variant (`common`).
        assert_eq!(
            FieldType::Enum(rarity.id).default_value_with_schemas(&dtos, &enums),
            DataValue::Enum { variant: test_uuid(51), fields: BTreeMap::new() }
        );

        // ShortViewString formatting for unit and payload variants.
        assert_eq!(rarity.short_view_string(test_uuid(52), &BTreeMap::new(), &dtos, &enums), "Rare");
        assert_eq!(enchantment.short_view_string(test_uuid(61), &BTreeMap::new(), &dtos, &enums), "None");
        let ward_fields = BTreeMap::from([
            (test_uuid(63), DataValue::F32(25.5)),
            (test_uuid(64), DataValue::LocalizedString("Void".into())),
        ]);
        assert_eq!(enchantment.short_view_string(test_uuid(62), &ward_fields, &dtos, &enums), "Ward(25.5 | Void)");

        // Fill and prune unknown fields inside an enum value in a collection row.
        let book_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: test_uuid(70),
            doc: String::new(),
            fields: vec![Field {
                id: test_uuid(71),
                name: "enchantment".into(),
                ty: FieldType::Enum(enchantment.id),
                category: String::new(),
                is_name: true,
                default: DataValue::Enum { variant: test_uuid(61), fields: BTreeMap::new() },
                settings: FieldSettings::default(),
            }],
        };
        let mut col = CollectionAsset::new(book_dto.id);
        let mut partial_ward = BTreeMap::new();
        partial_ward.insert(test_uuid(63), DataValue::F32(50.0));
        partial_ward.insert(test_uuid(999), DataValue::Bool(true)); // stale field key
        col.rows.push(Row {
            id: test_uuid(100),
            values: BTreeMap::from([(test_uuid(71), DataValue::Enum { variant: test_uuid(62), fields: partial_ward })]),
        });

        col.fill_defaults_with_schemas(&book_dto, &dtos, &enums);
        assert_eq!(book_dto.short_view_string_with_schemas(&col.rows[0], &dtos, &enums), "Ward(50 | Arcane)");
        let pruned = col.prune_unknown_fields_with_schemas(&book_dto, &dtos, &enums);
        assert_eq!(pruned, 1);
    }

    #[test]
    fn collection_fill_defaults_and_prune_unknown_fields() {
        let mut dto = DtoAsset::new();
        let f1 = Field { id: test_uuid(1), ..Field::new("title", FieldType::String) };
        let f2 = Field { id: test_uuid(2), default: DataValue::I32(10), ..Field::new("count", FieldType::I32) };
        dto.fields = vec![f1, f2];

        let mut col = CollectionAsset::new(test_uuid(10));
        let mut row = Row { id: test_uuid(101), values: BTreeMap::new() };
        row.values.insert(test_uuid(999), DataValue::Bool(true)); // stale field
        col.rows.push(row);

        col.fill_defaults(&dto);
        assert_eq!(col.rows[0].values.get(&test_uuid(1)), Some(&DataValue::String(String::new())));
        assert_eq!(col.rows[0].values.get(&test_uuid(2)), Some(&DataValue::I32(10)));
        assert_eq!(col.rows[0].values.len(), 3);

        let pruned = col.prune_unknown_fields(&dto);
        assert_eq!(pruned, 1);
        assert!(!col.rows[0].values.contains_key(&test_uuid(999)));
        assert_eq!(col.rows[0].values.len(), 2);
    }
}
