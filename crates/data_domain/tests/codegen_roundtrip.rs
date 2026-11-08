//! Integration and runtime round-trip tests for `data_domain::backend::codegen`, `.qsd` baking, `ToShortViewString`
//! parity across all 25 Slice 1 `FieldType`s, `Data<'w>` `SystemParam`, and `DataDomainPlugin` hot-reloading.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use bevy::asset::{AssetEvent, Assets};
use bevy::color::{Color, LinearRgba};
use bevy::ecs::message::Messages;
use bevy::ecs::system::RunSystemOnce;
use bevy::math::{IVec2, IVec3, Quat, UVec2, UVec3, Vec2, Vec3, Vec4};
use bevy::prelude::*;
use data_domain::backend::{
    COLLECTION_FORMAT_VERSION, CollectionAsset, DATA, DTO_FORMAT_VERSION, DataBackend, DataValue, DtoAsset, Field,
    FieldSettings, FieldType, Row, ToShortViewString as ModelToShortViewString, bake_collection,
    collection_rust_rel_path, compute_schema_hash, dto_rust_rel_path, emit_data_mod, emit_dto_file, emit_dtos_mod,
    save_collection, save_dto,
};
use project_core::{AssetUuid, Project};

/// Shared `common` runtime module matching `crates/quillshade/src/common/` (ADR 0048).
pub mod common {
    pub mod curve {
        include!("../../project_core/templates/common/curve.rs");
    }
    pub mod gradient {
        include!("../../project_core/templates/common/gradient.rs");
        include!("../../project_core/templates/common/gradient_bevy.rs");
    }
    pub use curve::{CurveData, Interp};
    pub use gradient::{GradientData, GradientStop};
}

/// Compile-time verification that a generated `data/mod.rs` with 0 DTOs and 0 collections compiles cleanly under
/// `clippy -D warnings` with no unused warnings.
pub mod generated_empty_data {
    include!("../src/backend/templates/data_mod.rs");

    /// Structural FNV-1a 64-bit hash over all valid `.dto.ron` schemas.
    pub const DATA_SCHEMA_HASH: u64 = 0xcbf29ce484222325;

    /// Baked `.collection.qsd` assets `(collection_uuid, asset_path)` loaded at startup.
    pub const BAKED_COLLECTIONS: &[(u128, &str)] = &[];

    pub mod dtos {
        #[allow(unused_imports)]
        pub(super) use super::{
            Collection, CurveData, DataDto, DataResolveCtx, GradientData, GradientStop, Interp, LocalizedString,
            RangeF32, RangeI32, RowUuid, ToShortViewString, game_asset_path_handle, material_uuid_handle,
            texture_uuid_handle,
        };
    }

    /// Typed [`CollectionUuid`] handles and metadata for `.collection.ron` assets.
    pub mod collections {}

    /// Primary game-facing [`SystemParam`] for querying baked Data domain collections and rows (ADR 0049 §7).
    #[derive(Clone, Copy, Debug)]
    pub struct Data<'w> {
        store: &'w DataStore,
    }

    impl<'w> Data<'w> {
        #[inline]
        pub(crate) fn from_store(store: &'w DataStore) -> Self {
            Self { store }
        }
    }

    fn init_registered_collections(_store: &mut DataStore) {}

    fn sync_registered_materials(_store: ResMut<DataStore>) {}

    fn decode_registered_collection(
        _store: &mut DataStore,
        _collection_id: u128,
        _bytes: &[u8],
    ) -> Result<bool, QsdRuntimeError> {
        Ok(false)
    }
}

/// Compile-time and runtime verification of generated `data/mod.rs` + `dtos/*` with:
/// - `library/all_types.dto.ron` (all 25 Slice 1 `FieldType`s),
/// - `library/rules.dto.ron` (1-row config DTO),
/// - `library/empty.dto.ron` (0-field DTO),
/// - `library/items.collection.ron`, and
/// - `library/rules.collection.ron`.
pub mod generated_data {
    include!("../src/backend/templates/data_mod.rs");

    /// Structural FNV-1a 64-bit hash over all valid `.dto.ron` schemas.
    pub const DATA_SCHEMA_HASH: u64 = 0x75fb1da08820eea3;

    /// Baked `.collection.qsd` assets `(collection_uuid, asset_path)` loaded at startup.
    pub const BAKED_COLLECTIONS: &[(u128, &str)] = &[
        (0x01963a10000070008000000000000010, "quillshade/data/autogen/library/items.collection.qsd"),
        (0x01963a10000070008000000000000011, "quillshade/data/autogen/library/rules.collection.qsd"),
    ];

    pub mod dtos {
        #[allow(unused_imports)]
        pub(super) use super::{
            Collection, CurveData, DataDto, DataResolveCtx, GradientData, GradientStop, Interp, LocalizedString,
            RangeF32, RangeI32, RowUuid, ToShortViewString, game_asset_path_handle, material_uuid_handle,
            texture_uuid_handle,
        };

        pub mod dto_library_all_types {
            #![allow(clippy::approx_constant, clippy::excessive_precision)]

            /// Showcase DTO covering all 25 Slice 1 field types.
            #[derive(Clone, Debug, PartialEq)]
            pub struct LibraryAllTypes {
                pub f_bool: bool,
                pub f_i8: i8,
                pub f_i16: i16,
                pub f_i32: i32,
                pub f_i64: i64,
                pub f_u8: u8,
                pub f_u16: u16,
                pub f_u32: u32,
                pub f_u64: u64,
                pub f_f32: f32,
                pub f_f64: f64,
                /// Author or code tag.
                pub f_string: String,
                /// Localized display title.
                pub f_loc: super::LocalizedString,
                pub f_vec2: bevy::math::Vec2,
                pub f_vec3: bevy::math::Vec3,
                pub f_vec4: bevy::math::Vec4,
                pub f_ivec2: bevy::math::IVec2,
                pub f_ivec3: bevy::math::IVec3,
                pub f_uvec2: bevy::math::UVec2,
                pub f_uvec3: bevy::math::UVec3,
                pub f_quat: bevy::math::Quat,
                pub f_color: bevy::color::Color,
                pub f_duration: std::time::Duration,
                pub f_range_f32: super::RangeF32,
                pub f_range_i32: super::RangeI32,
            }

            impl LibraryAllTypes {
                /// Stable 128-bit UUIDv7 of `quillshade/data/library/all_types.dto.ron`.
                pub const DTO_ID: u128 = 0x01963a10000070008000000000000001;
            }

            #[allow(clippy::derivable_impls)]
            impl Default for LibraryAllTypes {
                fn default() -> Self {
                    Self {
                        f_bool: false,
                        f_i8: 0i8,
                        f_i16: 0i16,
                        f_i32: 1928i32,
                        f_i64: 0i64,
                        f_u8: 0u8,
                        f_u16: 0u16,
                        f_u32: 312u32,
                        f_u64: 0u64,
                        f_f32: 0.0_f32,
                        f_f64: 0.0_f64,
                        f_string: String::from("A. Vance"),
                        f_loc: super::LocalizedString(String::from("Untitled")),
                        f_vec2: bevy::math::Vec2::new(0.0, 0.0),
                        f_vec3: bevy::math::Vec3::new(0.0, 0.0, 0.0),
                        f_vec4: bevy::math::Vec4::new(0.0, 0.0, 0.0, 0.0),
                        f_ivec2: bevy::math::IVec2::new(0, 0),
                        f_ivec3: bevy::math::IVec3::new(0, 0, 0),
                        f_uvec2: bevy::math::UVec2::new(0, 0),
                        f_uvec3: bevy::math::UVec3::new(0, 0, 0),
                        f_quat: bevy::math::Quat::from_xyzw(0.0, 0.0, 0.0, 1.0),
                        f_color: bevy::color::Color::linear_rgba(1.0, 1.0, 1.0, 1.0),
                        f_duration: std::time::Duration::from_secs_f32(0.0),
                        f_range_f32: super::RangeF32::new(0.0, 1.0),
                        f_range_i32: super::RangeI32::new(0, 1),
                    }
                }
            }

            impl super::ToShortViewString for LibraryAllTypes {
                fn to_short_view_string(&self) -> String {
                    let mut parts: Vec<String> = Vec::new();
                    let s = super::ToShortViewString::to_short_view_string(&self.f_i32);
                    if !s.trim().is_empty() {
                        parts.push(s);
                    }
                    let s = super::ToShortViewString::to_short_view_string(&self.f_u32);
                    if !s.trim().is_empty() {
                        parts.push(s);
                    }
                    let s = super::ToShortViewString::to_short_view_string(&self.f_loc);
                    if !s.trim().is_empty() {
                        parts.push(s);
                    }
                    if !parts.is_empty() {
                        return parts.join(" | ");
                    }
                    let s = super::ToShortViewString::to_short_view_string(&self.f_string);
                    if !s.trim().is_empty() {
                        return s;
                    }
                    String::from("LibraryAllTypes")
                }
            }

            impl std::fmt::Display for LibraryAllTypes {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str(&super::ToShortViewString::to_short_view_string(self))
                }
            }

            #[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
            pub struct LibraryAllTypesWire(
                pub bool,
                pub i8,
                pub i16,
                pub i32,
                pub i64,
                pub u8,
                pub u16,
                pub u32,
                pub u64,
                pub f32,
                pub f64,
                pub String,
                pub String,
                pub [f32; 2],
                pub [f32; 3],
                pub [f32; 4],
                pub [i32; 2],
                pub [i32; 3],
                pub [u32; 2],
                pub [u32; 3],
                pub [f32; 4],
                pub [f32; 4],
                pub f32,
                pub [f32; 2],
                pub [i32; 2],
            );

            impl From<LibraryAllTypesWire> for LibraryAllTypes {
                fn from(wire: LibraryAllTypesWire) -> Self {
                    Self {
                        f_bool: wire.0,
                        f_i8: wire.1,
                        f_i16: wire.2,
                        f_i32: wire.3,
                        f_i64: wire.4,
                        f_u8: wire.5,
                        f_u16: wire.6,
                        f_u32: wire.7,
                        f_u64: wire.8,
                        f_f32: wire.9,
                        f_f64: wire.10,
                        f_string: wire.11,
                        f_loc: super::LocalizedString(wire.12),
                        f_vec2: bevy::math::Vec2::from_array(wire.13),
                        f_vec3: bevy::math::Vec3::from_array(wire.14),
                        f_vec4: bevy::math::Vec4::from_array(wire.15),
                        f_ivec2: bevy::math::IVec2::from_array(wire.16),
                        f_ivec3: bevy::math::IVec3::from_array(wire.17),
                        f_uvec2: bevy::math::UVec2::from_array(wire.18),
                        f_uvec3: bevy::math::UVec3::from_array(wire.19),
                        f_quat: bevy::math::Quat::from_array(wire.20),
                        f_color: bevy::color::Color::linear_rgba(wire.21[0], wire.21[1], wire.21[2], wire.21[3]),
                        f_duration: std::time::Duration::from_secs_f32(if wire.22.is_finite() {
                            wire.22.max(0.0)
                        } else {
                            0.0
                        }),
                        f_range_f32: super::RangeF32::new(wire.23[0], wire.23[1]),
                        f_range_i32: super::RangeI32::new(wire.24[0], wire.24[1]),
                    }
                }
            }

            impl super::DataDto for LibraryAllTypes {
                const DTO_ID: u128 = Self::DTO_ID;

                fn decode_qsd_payload(
                    payload: &[u8],
                    _ctx: &super::DataResolveCtx<'_>,
                ) -> Result<super::Collection<Self>, String> {
                    let wire_rows: Vec<(u128, LibraryAllTypesWire)> =
                        postcard::from_bytes(payload).map_err(|e| format!("postcard decode error: {e}"))?;
                    let rows = wire_rows
                        .into_iter()
                        .map(|(row_id, wire)| (super::RowUuid::from_u128(row_id), Self::from(wire)))
                        .collect();
                    Ok(super::Collection::from_rows(rows))
                }
            }
        }
        pub use dto_library_all_types::{LibraryAllTypes, LibraryAllTypesWire};
        /// Unambiguous short alias for [`LibraryAllTypes`] (`library/all_types.dto.ron`).
        pub type AllTypes = LibraryAllTypes;
        pub type AllTypesWire = LibraryAllTypesWire;

        pub mod dto_library_empty {
            #![allow(clippy::approx_constant, clippy::excessive_precision)]

            /// Generated DTO struct for `library/empty` (`quillshade/data/library/empty.dto.ron`).
            #[derive(Clone, Debug, PartialEq)]
            pub struct LibraryEmpty {}

            impl LibraryEmpty {
                /// Stable 128-bit UUIDv7 of `quillshade/data/library/empty.dto.ron`.
                pub const DTO_ID: u128 = 0x01963a10000070008000000000000003;
            }

            #[allow(clippy::derivable_impls)]
            impl Default for LibraryEmpty {
                fn default() -> Self {
                    Self {}
                }
            }

            impl super::ToShortViewString for LibraryEmpty {
                fn to_short_view_string(&self) -> String {
                    String::from("LibraryEmpty")
                }
            }

            impl std::fmt::Display for LibraryEmpty {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str(&super::ToShortViewString::to_short_view_string(self))
                }
            }

            #[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
            pub struct LibraryEmptyWire(pub ());

            impl From<LibraryEmptyWire> for LibraryEmpty {
                fn from(_wire: LibraryEmptyWire) -> Self {
                    Self {}
                }
            }

            impl super::DataDto for LibraryEmpty {
                const DTO_ID: u128 = Self::DTO_ID;

                fn decode_qsd_payload(
                    payload: &[u8],
                    _ctx: &super::DataResolveCtx<'_>,
                ) -> Result<super::Collection<Self>, String> {
                    let wire_rows: Vec<(u128, LibraryEmptyWire)> =
                        postcard::from_bytes(payload).map_err(|e| format!("postcard decode error: {e}"))?;
                    let rows = wire_rows
                        .into_iter()
                        .map(|(row_id, wire)| (super::RowUuid::from_u128(row_id), Self::from(wire)))
                        .collect();
                    Ok(super::Collection::from_rows(rows))
                }
            }
        }
        pub use dto_library_empty::{LibraryEmpty, LibraryEmptyWire};
        /// Unambiguous short alias for [`LibraryEmpty`] (`library/empty.dto.ron`).
        pub type Empty = LibraryEmpty;
        pub type EmptyWire = LibraryEmptyWire;

        pub mod dto_library_rules {
            #![allow(clippy::approx_constant, clippy::excessive_precision)]

            /// Single-row library configuration rules.
            #[derive(Clone, Debug, PartialEq)]
            pub struct LibraryRules {
                pub library_name: super::LocalizedString,
                pub shelf_width_m: f32,
            }

            impl LibraryRules {
                /// Stable 128-bit UUIDv7 of `quillshade/data/library/rules.dto.ron`.
                pub const DTO_ID: u128 = 0x01963a10000070008000000000000002;
            }

            #[allow(clippy::derivable_impls)]
            impl Default for LibraryRules {
                fn default() -> Self {
                    Self {
                        library_name: super::LocalizedString(String::from("Grand Athenaeum")),
                        shelf_width_m: 2.2_f32,
                    }
                }
            }

            impl super::ToShortViewString for LibraryRules {
                fn to_short_view_string(&self) -> String {
                    let mut parts: Vec<String> = Vec::new();
                    let s = super::ToShortViewString::to_short_view_string(&self.library_name);
                    if !s.trim().is_empty() {
                        parts.push(s);
                    }
                    if !parts.is_empty() {
                        return parts.join(" | ");
                    }
                    String::from("LibraryRules")
                }
            }

            impl std::fmt::Display for LibraryRules {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str(&super::ToShortViewString::to_short_view_string(self))
                }
            }

            #[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
            pub struct LibraryRulesWire(pub String, pub f32);

            impl From<LibraryRulesWire> for LibraryRules {
                fn from(wire: LibraryRulesWire) -> Self {
                    Self { library_name: super::LocalizedString(wire.0), shelf_width_m: wire.1 }
                }
            }

            impl super::DataDto for LibraryRules {
                const DTO_ID: u128 = Self::DTO_ID;

                fn decode_qsd_payload(
                    payload: &[u8],
                    _ctx: &super::DataResolveCtx<'_>,
                ) -> Result<super::Collection<Self>, String> {
                    let wire_rows: Vec<(u128, LibraryRulesWire)> =
                        postcard::from_bytes(payload).map_err(|e| format!("postcard decode error: {e}"))?;
                    let rows = wire_rows
                        .into_iter()
                        .map(|(row_id, wire)| (super::RowUuid::from_u128(row_id), Self::from(wire)))
                        .collect();
                    Ok(super::Collection::from_rows(rows))
                }
            }
        }
        pub use dto_library_rules::{LibraryRules, LibraryRulesWire};
        /// Unambiguous short alias for [`LibraryRules`] (`library/rules.dto.ron`).
        pub type Rules = LibraryRules;
        pub type RulesWire = LibraryRulesWire;
    }
    #[allow(unused_imports)]
    pub use dtos::*;

    /// Typed [`CollectionUuid`] handles and metadata for `.collection.ron` assets.
    pub mod collections {
        #[allow(unused_imports)]
        use super::{CollectionUuid, dtos};

        /// Collection `library/items` (`quillshade/data/library/items.collection.ron`).
        pub const LIBRARY_ITEMS: CollectionUuid<dtos::LibraryAllTypes> =
            CollectionUuid::from_u128(0x01963a10000070008000000000000010);
        /// Short alias for [`LIBRARY_ITEMS`].
        pub const ITEMS: CollectionUuid<dtos::LibraryAllTypes> = LIBRARY_ITEMS;

        /// Collection `library/rules` (`quillshade/data/library/rules.collection.ron`).
        pub const LIBRARY_RULES: CollectionUuid<dtos::LibraryRules> =
            CollectionUuid::from_u128(0x01963a10000070008000000000000011);
        /// Short alias for [`LIBRARY_RULES`].
        pub const RULES: CollectionUuid<dtos::LibraryRules> = LIBRARY_RULES;

        pub mod library {
            #[allow(unused_imports)]
            use super::{CollectionUuid, dtos};

            /// Collection `library/items` (`quillshade/data/library/items.collection.ron`).
            pub const ITEMS: CollectionUuid<dtos::LibraryAllTypes> =
                CollectionUuid::from_u128(0x01963a10000070008000000000000010);

            /// Collection `library/rules` (`quillshade/data/library/rules.collection.ron`).
            pub const RULES: CollectionUuid<dtos::LibraryRules> =
                CollectionUuid::from_u128(0x01963a10000070008000000000000011);
        }
    }

    /// Primary game-facing [`SystemParam`] for querying baked Data domain collections and rows (ADR 0049 §7).
    #[derive(Clone, Copy, Debug)]
    pub struct Data<'w> {
        store: &'w DataStore,
        /// Statically typed handle for `quillshade/data/library/items.collection.ron`.
        pub collection_library_items: CollectionHandle<'w, dtos::LibraryAllTypes>,
        /// Short alias for [`Self::collection_library_items`] (`quillshade/data/library/items.collection.ron`).
        pub collection_items: CollectionHandle<'w, dtos::LibraryAllTypes>,
        /// Statically typed handle for `quillshade/data/library/rules.collection.ron`.
        pub collection_library_rules: CollectionHandle<'w, dtos::LibraryRules>,
        /// Short alias for [`Self::collection_library_rules`] (`quillshade/data/library/rules.collection.ron`).
        pub collection_rules: CollectionHandle<'w, dtos::LibraryRules>,
    }

    impl<'w> Data<'w> {
        #[inline]
        pub(crate) fn from_store(store: &'w DataStore) -> Self {
            let collection_library_items = CollectionHandle::new(store.collection(collections::LIBRARY_ITEMS), 1);
            let collection_library_rules = CollectionHandle::new(store.collection(collections::LIBRARY_RULES), 1);
            Self {
                store,
                collection_library_items,
                collection_items: collection_library_items,
                collection_library_rules,
                collection_rules: collection_library_rules,
            }
        }

        /// Returns the [`CollectionHandle`] for `quillshade/data/library/items.collection.ron`.
        #[inline]
        pub const fn collection_library_items(&self) -> CollectionHandle<'w, dtos::LibraryAllTypes> {
            self.collection_library_items
        }

        /// Returns the [`CollectionHandle`] for `quillshade/data/library/items.collection.ron`.
        #[inline]
        pub const fn collection_items(&self) -> CollectionHandle<'w, dtos::LibraryAllTypes> {
            self.collection_items
        }

        /// Returns the [`CollectionHandle`] for `quillshade/data/library/rules.collection.ron`.
        #[inline]
        pub const fn collection_library_rules(&self) -> CollectionHandle<'w, dtos::LibraryRules> {
            self.collection_library_rules
        }

        /// Returns the [`CollectionHandle`] for `quillshade/data/library/rules.collection.ron`.
        #[inline]
        pub const fn collection_rules(&self) -> CollectionHandle<'w, dtos::LibraryRules> {
            self.collection_rules
        }
    }

    fn init_registered_collections(store: &mut DataStore) {
        store.init_collection::<dtos::LibraryAllTypes>(collections::LIBRARY_ITEMS, 1);
        store.init_collection::<dtos::LibraryRules>(collections::LIBRARY_RULES, 1);
    }

    fn sync_registered_materials(_store: ResMut<DataStore>) {}

    #[allow(clippy::match_single_binding)]
    fn decode_registered_collection(
        store: &mut DataStore,
        collection_id: u128,
        bytes: &[u8],
    ) -> Result<bool, QsdRuntimeError> {
        match collection_id {
            0x01963a10000070008000000000000010 => {
                store.apply_collection_bytes::<dtos::LibraryAllTypes>(
                    collections::LIBRARY_ITEMS,
                    "quillshade/data/autogen/library/items.collection.qsd",
                    bytes,
                )?;
                Ok(true)
            }
            0x01963a10000070008000000000000011 => {
                store.apply_collection_bytes::<dtos::LibraryRules>(
                    collections::LIBRARY_RULES,
                    "quillshade/data/autogen/library/rules.collection.qsd",
                    bytes,
                )?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

fn test_uuid(n: u128) -> AssetUuid {
    AssetUuid::from_u128(0x0196_3a10_0000_7000_8000_0000_0000_0000 | n)
}

fn sample_25_field_values() -> Vec<(&'static str, FieldType, DataValue, DataValue, bool, &'static str)> {
    vec![
        ("f_bool", FieldType::Bool, DataValue::Bool(false), DataValue::Bool(true), false, ""),
        ("f_i8", FieldType::I8, DataValue::I8(0), DataValue::I8(-12), false, ""),
        ("f_i16", FieldType::I16, DataValue::I16(0), DataValue::I16(-1234), false, ""),
        ("f_i32", FieldType::I32, DataValue::I32(1928), DataValue::I32(1928), true, ""),
        ("f_i64", FieldType::I64, DataValue::I64(0), DataValue::I64(-9_876_543_210), false, ""),
        ("f_u8", FieldType::U8, DataValue::U8(0), DataValue::U8(200), false, ""),
        ("f_u16", FieldType::U16, DataValue::U16(0), DataValue::U16(54321), false, ""),
        ("f_u32", FieldType::U32, DataValue::U32(312), DataValue::U32(312), true, ""),
        ("f_u64", FieldType::U64, DataValue::U64(0), DataValue::U64(9_876_543_210), false, ""),
        ("f_f32", FieldType::F32, DataValue::F32(0.0), DataValue::F32(3.25), false, ""),
        ("f_f64", FieldType::F64, DataValue::F64(0.0), DataValue::F64(6.5), false, ""),
        (
            "f_string",
            FieldType::String,
            DataValue::String("A. Vance".into()),
            DataValue::String("A. Vance".into()),
            false,
            "Author or code tag.",
        ),
        (
            "f_loc",
            FieldType::LocalizedString,
            DataValue::LocalizedString("Untitled".into()),
            DataValue::LocalizedString("The Shader Alchemist".into()),
            true,
            "Localized display title.",
        ),
        ("f_vec2", FieldType::Vec2, DataValue::Vec2([0.0, 0.0]), DataValue::Vec2([1.5, 2.0]), false, ""),
        ("f_vec3", FieldType::Vec3, DataValue::Vec3([0.0, 0.0, 0.0]), DataValue::Vec3([14.8, 21.0, 2.8]), false, ""),
        (
            "f_vec4",
            FieldType::Vec4,
            DataValue::Vec4([0.0, 0.0, 0.0, 0.0]),
            DataValue::Vec4([1.0, 2.0, 3.0, 4.0]),
            false,
            "",
        ),
        ("f_ivec2", FieldType::IVec2, DataValue::IVec2([0, 0]), DataValue::IVec2([-5, 10]), false, ""),
        ("f_ivec3", FieldType::IVec3, DataValue::IVec3([0, 0, 0]), DataValue::IVec3([-1, 0, 1]), false, ""),
        ("f_uvec2", FieldType::UVec2, DataValue::UVec2([0, 0]), DataValue::UVec2([8, 16]), false, ""),
        ("f_uvec3", FieldType::UVec3, DataValue::UVec3([0, 0, 0]), DataValue::UVec3([32, 64, 128]), false, ""),
        (
            "f_quat",
            FieldType::Quat,
            DataValue::Quat([0.0, 0.0, 0.0, 1.0]),
            DataValue::Quat([0.0, 0.5, 0.0, 0.866]),
            false,
            "",
        ),
        (
            "f_color",
            FieldType::Color,
            DataValue::Color([1.0, 1.0, 1.0, 1.0]),
            DataValue::Color([0.55, 0.14, 0.12, 1.0]),
            false,
            "",
        ),
        ("f_duration", FieldType::Duration, DataValue::Duration(0.0), DataValue::Duration(120.5), false, ""),
        (
            "f_range_f32",
            FieldType::RangeF32,
            DataValue::RangeF32([0.0, 1.0]),
            DataValue::RangeF32([0.5, 2.5]),
            false,
            "",
        ),
        ("f_range_i32", FieldType::RangeI32, DataValue::RangeI32([0, 1]), DataValue::RangeI32([1, 5]), false, ""),
    ]
}

fn build_all_types_dto_and_row() -> (DtoAsset, Row) {
    let entries = sample_25_field_values();
    assert_eq!(entries.len(), 25);
    let mut fields = Vec::with_capacity(entries.len());
    let mut row_values = BTreeMap::new();
    for (idx, (name, ty, default, sample, is_name, doc)) in entries.into_iter().enumerate() {
        let fid = test_uuid(100 + idx as u128);
        fields.push(Field {
            id: fid,
            name: name.to_string(),
            ty,
            category: if idx < 13 { "Primitives & Strings".into() } else { "Spatial & Engine".into() },
            is_name,
            default,
            settings: FieldSettings { doc: doc.to_string(), ..FieldSettings::default() },
        });
        row_values.insert(fid, sample);
    }
    let dto = DtoAsset {
        format_version: DTO_FORMAT_VERSION,
        id: test_uuid(1),
        doc: "Showcase DTO covering all 25 Slice 1 field types.".into(),
        fields,
    };
    let row = Row { id: test_uuid(0x501), values: row_values };
    (dto, row)
}

fn build_rules_dto_and_row() -> (DtoAsset, Row) {
    let f_name_id = test_uuid(201);
    let f_width_id = test_uuid(202);
    let dto = DtoAsset {
        format_version: DTO_FORMAT_VERSION,
        id: test_uuid(2),
        doc: "Single-row library configuration rules.".into(),
        fields: vec![
            Field {
                id: f_name_id,
                name: "library_name".into(),
                ty: FieldType::LocalizedString,
                category: "General".into(),
                is_name: true,
                default: DataValue::LocalizedString("Grand Athenaeum".into()),
                settings: FieldSettings::default(),
            },
            Field {
                id: f_width_id,
                name: "shelf_width_m".into(),
                ty: FieldType::F32,
                category: "General".into(),
                is_name: false,
                default: DataValue::F32(2.2),
                settings: FieldSettings::default(),
            },
        ],
    };
    let row = Row {
        id: test_uuid(0x601),
        values: BTreeMap::from([
            (f_name_id, DataValue::LocalizedString("Grand Athenaeum".into())),
            (f_width_id, DataValue::F32(2.2)),
        ]),
    };
    (dto, row)
}

fn build_empty_dto() -> DtoAsset {
    DtoAsset { format_version: DTO_FORMAT_VERSION, id: test_uuid(3), doc: String::new(), fields: Vec::new() }
}

/// Test 1: `regen::run` emits `crates/quillshade/src/data/mod.rs`, `dtos/mod.rs`, `dtos/<path>.rs`, and
/// `assets/quillshade/data/autogen/<path>.collection.qsd`, and cleans up orphaned `.rs` and `.qsd` files when DTOs and
/// collections are removed.
#[test]
fn regen_emits_data_mod_dtos_and_qsd_and_cleans_orphans() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    let project = Project::create(dir.path()).unwrap();

    let (mut all_types_dto, item_row) = build_all_types_dto_and_row();
    let (mut rules_dto, rules_row) = build_rules_dto_and_row();
    let mut empty_dto = build_empty_dto();

    let all_types_path = project.domain_dir(DATA).join("library/all_types.dto.ron");
    let rules_dto_path = project.domain_dir(DATA).join("library/rules.dto.ron");
    let empty_dto_path = project.domain_dir(DATA).join("library/empty.dto.ron");
    save_dto(&all_types_path, &mut all_types_dto).unwrap();
    save_dto(&rules_dto_path, &mut rules_dto).unwrap();
    save_dto(&empty_dto_path, &mut empty_dto).unwrap();

    let mut items_col = CollectionAsset {
        format_version: COLLECTION_FORMAT_VERSION,
        id: test_uuid(0x10),
        dto: all_types_dto.id,
        rows: vec![item_row],
    };
    let mut rules_col = CollectionAsset {
        format_version: COLLECTION_FORMAT_VERSION,
        id: test_uuid(0x11),
        dto: rules_dto.id,
        rows: vec![rules_row],
    };

    let items_col_path = project.domain_dir(DATA).join("library/items.collection.ron");
    let rules_col_path = project.domain_dir(DATA).join("library/rules.collection.ron");
    save_collection(&items_col_path, &mut items_col, Some("library/all_types"), Some(&all_types_dto)).unwrap();
    save_collection(&rules_col_path, &mut rules_col, Some("library/rules"), Some(&rules_dto)).unwrap();

    let report = project_core::regen::run(&project, &[&DataBackend], &[], false, false).unwrap();
    assert!(!report.has_errors(), "{:?}", report.diagnostics);

    let data_mod_rs = project.module_dir(DATA).join("mod.rs");
    let dtos_mod_rs = project.module_dir(DATA).join("dtos/mod.rs");
    let collections_mod_rs = project.module_dir(DATA).join("collections/mod.rs");
    let all_types_rs = project.module_dir(DATA).join(dto_rust_rel_path("library/all_types"));
    let rules_rs = project.module_dir(DATA).join(dto_rust_rel_path("library/rules"));
    let empty_rs = project.module_dir(DATA).join(dto_rust_rel_path("library/empty"));
    let items_col_rs = project.module_dir(DATA).join(collection_rust_rel_path("library/items"));
    let rules_col_rs = project.module_dir(DATA).join(collection_rust_rel_path("library/rules"));
    let items_qsd = project.output_dir(DATA).join("autogen/library/items.collection.qsd");
    let rules_qsd = project.output_dir(DATA).join("autogen/library/rules.collection.qsd");

    for path in [
        &data_mod_rs,
        &dtos_mod_rs,
        &collections_mod_rs,
        &all_types_rs,
        &rules_rs,
        &empty_rs,
        &items_col_rs,
        &rules_col_rs,
        &items_qsd,
        &rules_qsd,
    ] {
        assert!(path.is_file(), "expected generated file `{}` to exist", path.display());
    }

    let mod_text = std::fs::read_to_string(&data_mod_rs).unwrap();
    assert!(
        mod_text.contains(&format!("pub const DATA_SCHEMA_HASH: u64 = {:#018x};", generated_data::DATA_SCHEMA_HASH))
    );
    assert!(mod_text.contains("pub mod collections;"));
    assert!(mod_text.contains("pub collection_library_items: CollectionHandle<'w, dtos::LibraryAllTypes>,"));
    assert!(mod_text.contains("pub collection_library_rules: CollectionHandle<'w, dtos::LibraryRules>,"));
    assert!(!mod_text.contains("ROW_0"));
    assert!(!mod_text.contains("items_rows"));

    let cols_mod_text = std::fs::read_to_string(&collections_mod_rs).unwrap();
    assert!(cols_mod_text.contains("#[path = \"library/items.rs\"]"));
    assert!(
        cols_mod_text
            .contains("pub const ITEMS: CollectionUuid<dtos::LibraryAllTypes> = col_library_items::LIBRARY_ITEMS;")
    );
    assert!(cols_mod_text.contains("#[path = \"library/rules.rs\"]"));
    assert!(
        cols_mod_text
            .contains("pub const RULES: CollectionUuid<dtos::LibraryRules> = col_library_rules::LIBRARY_RULES;")
    );
    assert!(cols_mod_text.contains("pub mod library {"));
    assert!(!cols_mod_text.contains("ROW_0"));
    assert!(!cols_mod_text.contains("_rows"));

    let items_col_text = std::fs::read_to_string(&items_col_rs).unwrap();
    assert!(items_col_text.contains("pub type Row = dtos::LibraryAllTypes;"));
    assert!(items_col_text.contains("pub const LIBRARY_ITEMS: CollectionUuid<Row>"));
    assert!(items_col_text.contains("pub const ID: CollectionUuid<Row> = LIBRARY_ITEMS;"));
    assert!(items_col_text.contains("pub const LEN: usize = 1;"));
    assert!(!items_col_text.contains("ROW_0"));
    assert!(!items_col_text.contains("_rows"));

    let dtos_mod_text = std::fs::read_to_string(&dtos_mod_rs).unwrap();
    assert!(dtos_mod_text.contains("#[path = \"library/all_types.rs\"]"));
    assert!(dtos_mod_text.contains("pub type AllTypes = LibraryAllTypes;"));
    assert!(dtos_mod_text.contains("#[path = \"library/rules.rs\"]"));
    assert!(dtos_mod_text.contains("pub type Rules = LibraryRules;"));
    assert!(dtos_mod_text.contains("#[path = \"library/empty.rs\"]"));
    assert!(dtos_mod_text.contains("pub type Empty = LibraryEmpty;"));

    // Remove `library/rules.dto.ron`, `library/empty.dto.ron`, and `library/rules.collection.ron` and re-run regen.
    std::fs::remove_file(&rules_dto_path).unwrap();
    std::fs::remove_file(&empty_dto_path).unwrap();
    std::fs::remove_file(&rules_col_path).unwrap();

    let report2 = project_core::regen::run(&project, &[&DataBackend], &[], false, false).unwrap();
    assert!(!report2.has_errors(), "{:?}", report2.diagnostics);

    assert!(!rules_rs.exists(), "orphan DTO .rs should be deleted");
    assert!(!empty_rs.exists(), "orphan 0-field DTO .rs should be deleted");
    assert!(!rules_col_rs.exists(), "orphan collection .rs should be deleted");
    assert!(!rules_qsd.exists(), "orphan collection .qsd should be deleted");
    assert!(all_types_rs.is_file(), "remaining DTO .rs should be kept");
    assert!(items_col_rs.is_file(), "remaining collection .rs should be kept");
    assert!(items_qsd.is_file(), "remaining collection .qsd should be kept");

    // Remove the last DTO and collection as well: `dtos/library/`, `collections/library/`, and `autogen/` should be cleaned up completely.
    std::fs::remove_file(&all_types_path).unwrap();
    std::fs::remove_file(&items_col_path).unwrap();

    let report3 = project_core::regen::run(&project, &[&DataBackend], &[], false, false).unwrap();
    assert!(!report3.has_errors(), "{:?}", report3.diagnostics);
    assert!(!all_types_rs.exists());
    assert!(!items_col_rs.exists());
    assert!(!items_qsd.exists());
    assert!(!project.module_dir(DATA).join("dtos/library").exists(), "empty dto subdirectory should be removed");
    assert!(
        !project.module_dir(DATA).join("collections/library").exists(),
        "empty collection subdirectory should be removed"
    );
}

/// Test 2: Verifies that `bake_collection` bytes decode via `postcard` into the exact tuple wire layout emitted by
/// `codegen.rs` for all 25 `FieldType`s (and 0-field DTOs), and that `ToShortViewString` formatting matches bit-for-bit
/// between `DataValue` and runtime types.
#[test]
fn generated_wire_round_trip_and_to_short_view_string_parity() {
    use generated_data::ToShortViewString as RuntimeToShortViewString;
    use generated_data::{AllTypes, DataDto, Empty, LocalizedString, RangeF32, RangeI32, RowUuid, Rules};

    let (all_types_dto, item_row) = build_all_types_dto_and_row();
    let (rules_dto, rules_row) = build_rules_dto_and_row();
    let empty_dto = build_empty_dto();

    let dtos = BTreeMap::from([
        ("library/all_types".to_string(), all_types_dto.clone()),
        ("library/empty".to_string(), empty_dto.clone()),
        ("library/rules".to_string(), rules_dto.clone()),
    ]);
    let schema_hash = compute_schema_hash(&dtos);
    assert_eq!(schema_hash, generated_data::DATA_SCHEMA_HASH);

    // Verify codegen outputs contain all expected structural constructs.
    let emitted_all_types = emit_dto_file("library/all_types", &all_types_dto);
    assert!(emitted_all_types.contains("pub struct LibraryAllTypes {"));
    assert!(emitted_all_types.contains("pub struct LibraryAllTypesWire("));
    let emitted_empty = emit_dto_file("library/empty", &empty_dto);
    assert!(emitted_empty.contains("pub struct LibraryEmpty {}"));
    assert!(emitted_empty.contains("pub struct LibraryEmptyWire(pub ());"));
    let emitted_dtos_mod = emit_dtos_mod(&dtos);
    assert!(emitted_dtos_mod.contains("pub use dto_library_all_types::{LibraryAllTypes, LibraryAllTypesWire};"));

    let items_col = CollectionAsset {
        format_version: COLLECTION_FORMAT_VERSION,
        id: test_uuid(0x10),
        dto: all_types_dto.id,
        rows: vec![item_row.clone()],
    };
    let rules_col = CollectionAsset {
        format_version: COLLECTION_FORMAT_VERSION,
        id: test_uuid(0x11),
        dto: rules_dto.id,
        rows: vec![rules_row.clone()],
    };
    let collections_map =
        BTreeMap::from([("library/items".to_string(), items_col.clone()), ("library/rules".to_string(), rules_col)]);
    let emitted_mod = emit_data_mod(&dtos, &collections_map, schema_hash);
    assert!(emitted_mod.contains("pub struct Data<'w>"));
    assert!(emitted_mod.contains("pub struct DataDomainPlugin;"));

    // Bake `items_col` (all 25 types) and decode via `<AllTypes as DataDto>::decode_qsd_payload`.
    let qsd_bytes = bake_collection(&items_col, &all_types_dto, schema_hash, &BTreeSet::new()).unwrap();
    let (_, payload) =
        generated_data::verify_qsd_header(&qsd_bytes, schema_hash, AllTypes::DTO_ID).expect("valid header");
    let decoded_col = AllTypes::decode_qsd_payload(payload, &generated_data::DataResolveCtx::default())
        .expect("postcard wire decode succeeds");
    assert_eq!(decoded_col.len(), 1);

    let expected_row_id = RowUuid::<AllTypes>::from_u128(item_row.id.as_u128());
    assert_eq!(decoded_col.index_of(expected_row_id), Some(0));
    let row_val = decoded_col.get(expected_row_id).expect("row found by RowUuid");
    assert_eq!(decoded_col.first(), Some(row_val));
    assert_eq!(decoded_col.first_with_id(), Some((expected_row_id, row_val)));

    // Verify all 25 decoded runtime values and their `ToShortViewString` parity with `DataValue`.
    assert!(row_val.f_bool);
    assert_eq!(row_val.f_i8, -12);
    assert_eq!(row_val.f_i16, -1234);
    assert_eq!(row_val.f_i32, 1928);
    assert_eq!(row_val.f_i64, -9_876_543_210);
    assert_eq!(row_val.f_u8, 200);
    assert_eq!(row_val.f_u16, 54321);
    assert_eq!(row_val.f_u32, 312);
    assert_eq!(row_val.f_u64, 9_876_543_210);
    assert_eq!(row_val.f_f32, 3.25);
    assert_eq!(row_val.f_f64, 6.5);
    assert_eq!(row_val.f_string, "A. Vance");
    assert_eq!(row_val.f_loc, LocalizedString::from("The Shader Alchemist"));
    assert_eq!(&*row_val.f_loc, "The Shader Alchemist");
    assert_eq!(row_val.f_vec2, Vec2::new(1.5, 2.0));
    assert_eq!(row_val.f_vec3, Vec3::new(14.8, 21.0, 2.8));
    assert_eq!(row_val.f_vec4, Vec4::new(1.0, 2.0, 3.0, 4.0));
    assert_eq!(row_val.f_ivec2, IVec2::new(-5, 10));
    assert_eq!(row_val.f_ivec3, IVec3::new(-1, 0, 1));
    assert_eq!(row_val.f_uvec2, UVec2::new(8, 16));
    assert_eq!(row_val.f_uvec3, UVec3::new(32, 64, 128));
    assert_eq!(row_val.f_quat, Quat::from_xyzw(0.0, 0.5, 0.0, 0.866));
    assert_eq!(row_val.f_color, Color::LinearRgba(LinearRgba::from_f32_array([0.55, 0.14, 0.12, 1.0])));
    assert_eq!(row_val.f_duration, Duration::from_secs_f32(120.5));
    assert_eq!(row_val.f_range_f32, RangeF32::new(0.5, 2.5));
    assert!(row_val.f_range_f32.contains(1.5) && !row_val.f_range_f32.contains(3.0));
    assert_eq!(row_val.f_range_i32, RangeI32::new(1, 5));
    assert!(row_val.f_range_i32.contains(3) && !row_val.f_range_i32.contains(9));

    let runtime_short_strings: [String; 25] = [
        row_val.f_bool.to_short_view_string(),
        row_val.f_i8.to_short_view_string(),
        row_val.f_i16.to_short_view_string(),
        row_val.f_i32.to_short_view_string(),
        row_val.f_i64.to_short_view_string(),
        row_val.f_u8.to_short_view_string(),
        row_val.f_u16.to_short_view_string(),
        row_val.f_u32.to_short_view_string(),
        row_val.f_u64.to_short_view_string(),
        row_val.f_f32.to_short_view_string(),
        row_val.f_f64.to_short_view_string(),
        row_val.f_string.to_short_view_string(),
        row_val.f_loc.to_short_view_string(),
        row_val.f_vec2.to_short_view_string(),
        row_val.f_vec3.to_short_view_string(),
        row_val.f_vec4.to_short_view_string(),
        row_val.f_ivec2.to_short_view_string(),
        row_val.f_ivec3.to_short_view_string(),
        row_val.f_uvec2.to_short_view_string(),
        row_val.f_uvec3.to_short_view_string(),
        row_val.f_quat.to_short_view_string(),
        row_val.f_color.to_short_view_string(),
        row_val.f_duration.to_short_view_string(),
        row_val.f_range_f32.to_short_view_string(),
        row_val.f_range_i32.to_short_view_string(),
    ];

    for (idx, (_, _, _, sample_val, _, _)) in sample_25_field_values().into_iter().enumerate() {
        assert_eq!(
            runtime_short_strings[idx],
            ModelToShortViewString::to_short_view_string(&sample_val),
            "field #{idx} ToShortViewString mismatch"
        );
    }

    // Verify DTO-level `ToShortViewString` and `Display` parity with `DtoAsset::short_view_string`.
    assert_eq!(row_val.to_short_view_string(), all_types_dto.short_view_string(&item_row));
    assert_eq!(row_val.to_string(), "1928 | 312 | The Shader Alchemist");

    // Verify default literal construction on `AllTypes::default()` and `Rules::default()`.
    let default_all = AllTypes::default();
    let default_row = Row::from_dto_defaults(&all_types_dto);
    assert_eq!(default_all.to_short_view_string(), all_types_dto.short_view_string(&default_row));
    assert_eq!(Rules::default().to_short_view_string(), "Grand Athenaeum");

    // Verify 0-field DTO `Empty` round-trips through `bake_collection` and `Empty::decode_qsd_payload`.
    let empty_col = CollectionAsset {
        format_version: COLLECTION_FORMAT_VERSION,
        id: test_uuid(0x12),
        dto: empty_dto.id,
        rows: vec![Row { id: test_uuid(0x701), values: BTreeMap::new() }],
    };
    let empty_bytes = bake_collection(&empty_col, &empty_dto, schema_hash, &BTreeSet::new()).unwrap();
    let (_, empty_payload) =
        generated_data::verify_qsd_header(&empty_bytes, schema_hash, Empty::DTO_ID).expect("valid empty header");
    let decoded_empty = Empty::decode_qsd_payload(empty_payload, &generated_data::DataResolveCtx::default())
        .expect("0-field wire decode succeeds");
    assert_eq!(decoded_empty.len(), 1);
    assert_eq!(decoded_empty.first().unwrap().to_short_view_string(), "LibraryEmpty");
}

/// Test 3: Tests `DataDomainPlugin`, `Data<'w>` `SystemParam`, `Assets<QsdBlobAsset>` hot-reload, and non-panicking
/// rejection of a `.qsd` buffer with a mismatched `DATA_SCHEMA_HASH` while preserving the previous in-memory
/// collection.
#[test]
fn runtime_loader_hot_reload_and_schema_hash_guard() {
    use generated_data::{
        AllTypes, Data, DataDomainPlugin, DataStore, LocalizedString, QsdBlobAsset, QsdRuntimeError, RowUuid,
        ToShortViewString, collections,
    };

    let (all_types_dto, mut item_row_0) = build_all_types_dto_and_row();
    let (rules_dto, rules_row_0) = build_rules_dto_and_row();
    let empty_dto = build_empty_dto();
    let dtos = BTreeMap::from([
        ("library/all_types".to_string(), all_types_dto.clone()),
        ("library/empty".to_string(), empty_dto),
        ("library/rules".to_string(), rules_dto.clone()),
    ]);
    let schema_hash = compute_schema_hash(&dtos);

    let items_col_v1 = CollectionAsset {
        format_version: COLLECTION_FORMAT_VERSION,
        id: test_uuid(0x10),
        dto: all_types_dto.id,
        rows: vec![item_row_0.clone()],
    };
    let rules_col_v1 = CollectionAsset {
        format_version: COLLECTION_FORMAT_VERSION,
        id: test_uuid(0x11),
        dto: rules_dto.id,
        rows: vec![rules_row_0],
    };

    let items_bytes_v1 = bake_collection(&items_col_v1, &all_types_dto, schema_hash, &BTreeSet::new()).unwrap();
    let rules_bytes_v1 = bake_collection(&rules_col_v1, &rules_dto, schema_hash, &BTreeSet::new()).unwrap();

    let mut app = App::new();
    app.add_plugins(DataDomainPlugin);
    app.update();

    // Before the two `.qsd` blobs finish loading, `is_ready()` is false, compile-time `len` is already known,
    // and iterating yields 0 rows without Option unwrapping.
    let (ready_before, len_before, iter_count_before) = app
        .world_mut()
        .run_system_once(|data: Data| {
            (data.is_ready(), data.collection_library_items.len, data.collection_library_items.into_iter().count())
        })
        .unwrap();
    assert!(!ready_before);
    assert_eq!(len_before, 1);
    assert_eq!(iter_count_before, 0);

    // Add both `.qsd` blobs to `Assets<QsdBlobAsset>`, register their handles, and fire `AssetEvent::Added`.
    let (items_handle, rules_handle) = {
        let mut blobs = app.world_mut().resource_mut::<Assets<QsdBlobAsset>>();
        let h_items = blobs.add(QsdBlobAsset { bytes: items_bytes_v1 });
        let h_rules = blobs.add(QsdBlobAsset { bytes: rules_bytes_v1 });
        (h_items, h_rules)
    };
    {
        let mut store = app.world_mut().resource_mut::<DataStore>();
        store.register_handle(
            collections::LIBRARY_ITEMS.as_u128(),
            "quillshade/data/autogen/library/items.collection.qsd",
            items_handle.clone(),
        );
        store.register_handle(
            collections::LIBRARY_RULES.as_u128(),
            "quillshade/data/autogen/library/rules.collection.qsd",
            rules_handle.clone(),
        );
    }
    {
        let mut events = app.world_mut().resource_mut::<Messages<AssetEvent<QsdBlobAsset>>>();
        events.write(AssetEvent::Added { id: items_handle.id() });
        events.write(AssetEvent::Added { id: rules_handle.id() });
    }
    app.update();

    let row_0_uuid = RowUuid::<AllTypes>::from_u128(test_uuid(0x501).as_u128());
    let row_0_str = row_0_uuid.to_string();

    // Verify `Data<'w>` queries after initial load.
    app.world_mut()
        .run_system_once(move |data: Data| {
            assert!(data.is_ready());
            assert_eq!(data.collection_library_items.len, 1);
            assert_eq!(data.collection_library_items.len(), 1);
            assert!(!data.collection_library_items.is_empty());
            assert!(data.collection_library_items.is_loaded());

            // Direct iteration over `data.collection_library_items` (`IntoIterator` on `CollectionHandle` which is `Copy`)
            let mut iterated = Vec::new();
            for (id, row) in data.collection_library_items {
                iterated.push((id, row.f_loc.clone()));
            }
            assert_eq!(iterated, vec![(row_0_uuid, LocalizedString::from("The Shader Alchemist"))]);

            // Also iterate via `.iter()` and lookup via `.get(uuid)` in O(1):
            #[allow(clippy::needless_borrows_for_generic_args)]
            for (id, row) in data.collection_library_items.iter() {
                assert_eq!(data.collection_library_items.get(id), Some(row));
                assert_eq!(data.collection_library_items.get(&id), Some(row));
                assert_eq!(data.collection_library_items.get(id.as_u128()), Some(row));
                assert_eq!(data.collection_library_items.get(row_0_str.as_str()), Some(row));
                assert_eq!(data.collection_library_items.get(&row_0_str), Some(row));
                assert_eq!(data.collection_library_items.get(row_0_str.clone()), Some(row));
            }

            let items = data.collection(collections::ITEMS);
            assert_eq!(items.len(), 1);
            assert!(!items.is_empty());

            let by_full = data.collection(collections::LIBRARY_ITEMS);
            let by_nested = data.collection(collections::library::ITEMS);
            assert_eq!(items, by_full);
            assert_eq!(items, by_nested);

            let r0 = data.get(collections::ITEMS, row_0_uuid).expect("row 0 found by collection + row UUID");
            assert_eq!(r0.f_loc, LocalizedString::from("The Shader Alchemist"));
            assert_eq!(r0.to_short_view_string(), "1928 | 312 | The Shader Alchemist");

            // Global row lookup across all collections of `AllTypes`:
            let found = data.find_row::<AllTypes>(row_0_uuid).expect("row 0 found via find_row");
            assert_eq!(found, r0);
            let found_by_str = data.find_row::<AllTypes>(row_0_str.as_str()).expect("row 0 found via &str");
            assert_eq!(found_by_str, r0);

            // 1-row config collection `collection_library_rules`:
            let rules = data.collection_library_rules.first().expect("rules row 0");
            assert_eq!(rules.library_name.as_str(), "Grand Athenaeum");
            assert_eq!(rules.shelf_width_m, 2.2);
            assert_eq!(data.first(collections::RULES), Some(rules));
        })
        .unwrap();

    // Hot-reload `items.collection.qsd` (`AssetEvent::Modified`) with updated title and a second row!
    let f_loc_id = all_types_dto.field_by_name("f_loc").unwrap().id;
    item_row_0.values.insert(f_loc_id, DataValue::LocalizedString("The Shader Alchemist (2nd Ed.)".into()));
    let mut item_row_1 = item_row_0.clone();
    item_row_1.id = test_uuid(0x502);
    item_row_1.values.insert(f_loc_id, DataValue::LocalizedString("Atlas of Whispering Fogs".into()));

    let items_col_v2 = CollectionAsset {
        format_version: COLLECTION_FORMAT_VERSION,
        id: test_uuid(0x10),
        dto: all_types_dto.id,
        rows: vec![item_row_0, item_row_1],
    };
    let items_bytes_v2 = bake_collection(&items_col_v2, &all_types_dto, schema_hash, &BTreeSet::new()).unwrap();

    {
        let mut blobs = app.world_mut().resource_mut::<Assets<QsdBlobAsset>>();
        blobs.get_mut(items_handle.id()).unwrap().bytes = items_bytes_v2.clone();
    }
    app.world_mut()
        .resource_mut::<Messages<AssetEvent<QsdBlobAsset>>>()
        .write(AssetEvent::Modified { id: items_handle.id() });
    app.update();

    let row_1_uuid = RowUuid::<AllTypes>::from_u128(test_uuid(0x502).as_u128());
    app.world_mut()
        .run_system_once(move |data: Data| {
            let items = data.collection_library_items;
            assert_eq!(items.len, 2);
            assert_eq!(items.get(row_0_uuid).unwrap().f_loc.as_str(), "The Shader Alchemist (2nd Ed.)");
            assert_eq!(items.get(row_1_uuid).unwrap().f_loc.as_str(), "Atlas of Whispering Fogs");
        })
        .unwrap();

    // Now corrupt `schema_hash` in the `.qsd` buffer and fire `AssetEvent::Modified`:
    // 1. Direct `apply_collection_bytes` returns `Err(QsdRuntimeError::SchemaHashMismatch { .. })` without panicking.
    // 2. `Update` hot-reload logs an error and preserves the 2-row `v2` collection in memory!
    let mut bad_hash_bytes = items_bytes_v2;
    bad_hash_bytes[12..20].copy_from_slice(&0xdead_beef_cafe_babe_u64.to_le_bytes());

    let err = app
        .world_mut()
        .resource_mut::<DataStore>()
        .apply_collection_bytes(
            collections::LIBRARY_ITEMS,
            "quillshade/data/autogen/library/items.collection.qsd",
            &bad_hash_bytes,
        )
        .unwrap_err();
    assert_eq!(err, QsdRuntimeError::SchemaHashMismatch { expected: schema_hash, found: 0xdead_beef_cafe_babe });

    {
        let mut blobs = app.world_mut().resource_mut::<Assets<QsdBlobAsset>>();
        blobs.get_mut(items_handle.id()).unwrap().bytes = bad_hash_bytes;
    }
    app.world_mut()
        .resource_mut::<Messages<AssetEvent<QsdBlobAsset>>>()
        .write(AssetEvent::Modified { id: items_handle.id() });
    app.update();

    app.world_mut()
        .run_system_once(move |data: Data| {
            assert!(data.is_ready(), "DataStore stays ready with last known good data");
            let items = data.collection_library_items;
            assert_eq!(items.len, 2, "previous 2-row collection must remain intact after failed hot-reload");
            assert_eq!(items.get(row_0_uuid).unwrap().f_loc.as_str(), "The Shader Alchemist (2nd Ed.)");
            assert_eq!(items.get(row_1_uuid).unwrap().f_loc.as_str(), "Atlas of Whispering Fogs");
        })
        .unwrap();
}

/// Test 4: 3-level nested DTO hierarchy (`SectorCoords` -> `Planet` -> `Author` -> `Book`) end-to-end test covering
/// D27 RON output, recursive `ToShortViewString`, `.qsd` baking/decoding, and generated Rust `<Pascal>Wire -> <Pascal>`.
#[test]
fn nested_dto_3_level_hierarchy_codegen_and_qsd_roundtrip() {
    use data_domain::backend::{
        bake_collection_with_dtos, collection_text_with_dtos, decode_collection_rows_with_dtos, dto_text_with_dtos,
        emit_dto_file_with_dtos, parse_collection, parse_dto, save_collection_with_dtos, save_dto_with_dtos,
    };

    let coords_dto = DtoAsset {
        format_version: DTO_FORMAT_VERSION,
        id: test_uuid(0x201),
        doc: "Galactic sector coordinates.".into(),
        fields: vec![Field {
            id: test_uuid(0x211),
            name: "sector".into(),
            ty: FieldType::IVec3,
            category: String::new(),
            is_name: true,
            default: DataValue::IVec3([4, 5, 6]),
            settings: FieldSettings::default(),
        }],
    };
    let planet_dto = DtoAsset {
        format_version: DTO_FORMAT_VERSION,
        id: test_uuid(0x202),
        doc: "Homeworld planet.".into(),
        fields: vec![
            Field {
                id: test_uuid(0x221),
                name: "name".into(),
                ty: FieldType::String,
                category: String::new(),
                is_name: true,
                default: DataValue::String("Arrakis".into()),
                settings: FieldSettings::default(),
            },
            Field {
                id: test_uuid(0x222),
                name: "coords".into(),
                ty: FieldType::Dto(coords_dto.id),
                category: String::new(),
                is_name: true,
                default: DataValue::Dto(BTreeMap::from([(test_uuid(0x211), DataValue::IVec3([4, 5, 6]))])),
                settings: FieldSettings::default(),
            },
        ],
    };
    let author_dto = DtoAsset {
        format_version: DTO_FORMAT_VERSION,
        id: test_uuid(0x203),
        doc: "Book author.".into(),
        fields: vec![
            Field {
                id: test_uuid(0x231),
                name: "name".into(),
                ty: FieldType::String,
                category: String::new(),
                is_name: true,
                default: DataValue::String("A. Vance".into()),
                settings: FieldSettings::default(),
            },
            Field {
                id: test_uuid(0x232),
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
        id: test_uuid(0x204),
        doc: "Library book with nested author.".into(),
        fields: vec![
            Field {
                id: test_uuid(0x241),
                name: "title".into(),
                ty: FieldType::LocalizedString,
                category: String::new(),
                is_name: true,
                default: DataValue::LocalizedString("The Shader Alchemist".into()),
                settings: FieldSettings::default(),
            },
            Field {
                id: test_uuid(0x242),
                name: "author".into(),
                ty: FieldType::Dto(author_dto.id),
                category: String::new(),
                is_name: true,
                default: DataValue::Dto(BTreeMap::new()),
                settings: FieldSettings::default(),
            },
        ],
    };

    let dtos = BTreeMap::from([
        ("library/sector_coords".to_string(), coords_dto.clone()),
        ("library/planet".to_string(), planet_dto.clone()),
        ("library/author".to_string(), author_dto.clone()),
        ("library/book".to_string(), book_dto.clone()),
    ]);

    // 1. Verify D27 RON output for nested DTOs and round-trip parsing.
    let planet_ron = dto_text_with_dtos(&planet_dto, &dtos);
    assert!(
        planet_ron.contains(&format!("ty: Dto(\"{}\"), // library/sector_coords\n", coords_dto.id)),
        "{planet_ron}"
    );
    assert!(planet_ron.contains(&format!("\"{}\": IVec3((4, 5, 6)), // sector\n", test_uuid(0x211))), "{planet_ron}");
    assert_eq!(parse_dto(&planet_ron).unwrap(), planet_dto);

    // 2. Verify generated Rust for `library/book.dto.ron` and `library/author.dto.ron`.
    let book_rs = emit_dto_file_with_dtos("library/book", &book_dto, &dtos);
    assert!(book_rs.contains("pub author: super::LibraryAuthor,"), "{book_rs}");
    assert!(book_rs.contains("pub struct LibraryBookWire(pub String, pub super::LibraryAuthorWire);"), "{book_rs}");
    assert!(book_rs.contains("author: super::LibraryAuthor::from_wire(wire.1, ctx),"), "{book_rs}");
    assert!(book_rs.contains("homeworld: super::LibraryPlanet {"), "{book_rs}");
    assert!(book_rs.contains("coords: super::LibrarySectorCoords {"), "{book_rs}");

    // 3. Build a collection with 3-level nested `DataValue::Dto` and verify D27 comments, `.qsd` bake/decode, and regen::run.
    let mut col = CollectionAsset {
        format_version: COLLECTION_FORMAT_VERSION,
        id: test_uuid(0x300),
        dto: book_dto.id,
        rows: vec![Row::from_dto_defaults_with_dtos(&book_dto, &dtos)],
    };
    col.rows[0].id = test_uuid(0x301);

    let col_ron = collection_text_with_dtos(&col, Some(&book_dto), &dtos);
    assert!(col_ron.contains("// library/book\n"), "{col_ron}");
    assert!(col_ron.contains("// author\n"), "{col_ron}");
    assert!(col_ron.contains("// homeworld\n"), "{col_ron}");
    assert!(col_ron.contains("// coords\n"), "{col_ron}");
    assert!(col_ron.contains("// sector\n"), "{col_ron}");
    assert_eq!(parse_collection(&col_ron).unwrap(), col);

    assert_eq!(
        book_dto.short_view_string_with_dtos(&col.rows[0], &dtos),
        "The Shader Alchemist | A. Vance | Arrakis | (4, 5, 6)"
    );

    let schema_hash = compute_schema_hash(&dtos);
    let qsd_bytes = bake_collection_with_dtos(&col, &book_dto, &dtos, schema_hash, &BTreeSet::new()).unwrap();
    let decoded = decode_collection_rows_with_dtos(&qsd_bytes, &book_dto, &dtos, schema_hash).unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].1, col.rows[0].values);

    // 4. Write all 4 DTOs and the collection to a temp project and run `project_core::regen::run`.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    let project = Project::create(dir.path()).unwrap();
    for (path, dto) in &dtos {
        let mut d = dto.clone();
        save_dto_with_dtos(&project.domain_dir(DATA).join(format!("{path}.dto.ron")), &mut d, &dtos).unwrap();
    }
    save_collection_with_dtos(
        &project.domain_dir(DATA).join("library/books.collection.ron"),
        &mut col,
        Some(&book_dto),
        &dtos,
    )
    .unwrap();

    let report = project_core::regen::run(&project, &[&DataBackend], &[], false, false).unwrap();
    assert!(!report.has_errors(), "{:?}", report.diagnostics);
    assert!(project.output_dir(DATA).join("autogen/library/books.collection.qsd").is_file());
}

/// Test 5: Slice 3 direct cross-domain and game asset references (`Curve`, `Gradient`, `Material`, `Texture`,
/// `LightEnvironment`, `GameAsset(GameAssetKind)`) — runtime `CurveData::sample` and `GradientData::sample`, handle
/// resolution (`material_uuid_handle`, `texture_uuid_handle`, `game_asset_path_handle`), `emit_dto_file_with_context`,
/// and cross-domain incremental `AssetIndex::affected` + `project_core::regen::run_affected` re-baking `.collection.qsd`
/// when a referenced `.curve.ron` or `.gradients.ron` is modified.
#[test]
fn slice_3_direct_asset_refs_runtime_sampling_handles_and_incremental_run_affected() {
    use data_domain::backend::{
        GameAssetKind, bake_collection_with_context, decode_collection_rows_with_dtos, emit_data_mod_with_context,
        emit_dto_file_with_context, save_collection_with_index, save_dto_with_index,
    };
    use generated_data::{
        CurveData, DataResolveCtx, GradientData, GradientStop, Interp, ToShortViewString as RuntimeToShortViewString,
        game_asset_path_handle, material_uuid_handle, texture_uuid_handle,
    };
    use project_core::asset_index::{IndexedAsset, SubItem};
    use project_core::regen::{DomainBackend, DomainOutput, DomainPlan};
    use project_core::umbrella::CrateContribution;
    use project_core::{AssetIndex, AssetRef, DomainId, DomainIndex};

    const MATERIAL: DomainId = DomainId::new("material");
    const LIGHT: DomainId = DomainId::new("light");

    struct MockCrossDomainBackend {
        domain: DomainId,
        assets: Vec<IndexedAsset>,
    }

    impl DomainBackend for MockCrossDomainBackend {
        fn id(&self) -> DomainId {
            self.domain
        }

        fn crate_contribution(&self) -> CrateContribution {
            CrateContribution {
                domain: self.domain,
                plugin: "MockPlugin",
                loader: None,
                bevy_features: &[],
                dependencies: &[],
            }
        }

        fn index(&self, _project: &Project) -> DomainIndex {
            DomainIndex { domain: self.domain, assets: self.assets.clone() }
        }

        fn plan(&self, _project: &Project, _index: &AssetIndex, _compile: bool) -> DomainPlan {
            DomainPlan {
                diagnostics: Vec::new(),
                output: DomainOutput {
                    domain: self.domain,
                    files: Vec::new(),
                    keep: BTreeSet::new(),
                    roots: Vec::new(),
                    orphan_scan: false,
                },
            }
        }
    }

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
    std::fs::create_dir_all(dir.path().join("assets/textures")).unwrap();
    std::fs::create_dir_all(dir.path().join("assets/audio")).unwrap();
    std::fs::write(dir.path().join("assets/textures/albedo.png"), b"png").unwrap();
    std::fs::write(dir.path().join("assets/audio/chime.ogg"), b"ogg").unwrap();
    let project = Project::create(dir.path()).unwrap();

    let curve_id = test_uuid(0x401);
    let grad_id = test_uuid(0x402);
    let mat_id = test_uuid(0x403);
    let tex_id = test_uuid(0x404);
    let env_id = test_uuid(0x405);

    let mat_dir = project.domain_dir(MATERIAL);
    let light_dir = project.domain_dir(LIGHT);
    std::fs::create_dir_all(mat_dir.join("curves")).unwrap();
    std::fs::create_dir_all(&light_dir).unwrap();

    let curve_file = mat_dir.join("curves/falloff.curve.ron");
    std::fs::write(
        &curve_file,
        format!(
            "(\n    format_version: 1,\n    id: \"{curve_id}\",\n    interp: Simple,\n    points: [\n        (0.0, 0.0),\n        (1.0, 1.0),\n    ],\n)\n"
        ),
    )
    .unwrap();

    let grad_file = mat_dir.join("palettes.gradients.ron");
    std::fs::write(
        &grad_file,
        format!(
            "(\n    format_version: 2,\n    id: \"{grad_id}\",\n    slots: [\n        (\n            id: 2,\n            name: \"sunset\",\n            stops: [\n                (\n                    t: 0.0,\n                    color: [0.0, 0.0, 0.0, 1.0],\n                    mid: 0.5,\n                ),\n                (\n                    t: 1.0,\n                    color: [1.0, 1.0, 1.0, 1.0],\n                    mid: 0.5,\n                ),\n            ],\n        ),\n    ],\n)\n"
        ),
    )
    .unwrap();

    std::fs::write(mat_dir.join("wood.material.ron"), format!("(\n    format_version: 1,\n    id: \"{mat_id}\",\n)\n"))
        .unwrap();

    std::fs::write(
        mat_dir.join("albedo.texture.ron"),
        format!("(\n    format_version: 1,\n    id: \"{tex_id}\",\n    path: \"textures/albedo.png\",\n)\n"),
    )
    .unwrap();

    std::fs::write(
        light_dir.join("day.environment.ron"),
        format!("(\n    format_version: 1,\n    id: \"{env_id}\",\n)\n"),
    )
    .unwrap();

    let mat_backend = MockCrossDomainBackend {
        domain: MATERIAL,
        assets: vec![
            IndexedAsset {
                asset: AssetRef { domain: MATERIAL, path: "curves/falloff".into(), suffix: "curve" },
                id: Ok(curve_id),
                sub_items: Vec::new(),
                refs: Vec::new(),
            },
            IndexedAsset {
                asset: AssetRef { domain: MATERIAL, path: "palettes".into(), suffix: "gradients" },
                id: Ok(grad_id),
                sub_items: vec![SubItem::new(2, "sunset")],
                refs: Vec::new(),
            },
            IndexedAsset {
                asset: AssetRef { domain: MATERIAL, path: "wood".into(), suffix: "material" },
                id: Ok(mat_id),
                sub_items: Vec::new(),
                refs: Vec::new(),
            },
            IndexedAsset {
                asset: AssetRef { domain: MATERIAL, path: "albedo".into(), suffix: "texture" },
                id: Ok(tex_id),
                sub_items: Vec::new(),
                refs: Vec::new(),
            },
        ],
    };
    let light_backend = MockCrossDomainBackend {
        domain: LIGHT,
        assets: vec![IndexedAsset {
            asset: AssetRef { domain: LIGHT, path: "day".into(), suffix: "environment" },
            id: Ok(env_id),
            sub_items: Vec::new(),
            refs: Vec::new(),
        }],
    };

    let mut dto = DtoAsset {
        format_version: DTO_FORMAT_VERSION,
        id: test_uuid(0x410),
        doc: "Slice 3 asset reference showcase.".into(),
        fields: vec![
            Field {
                id: test_uuid(0x411),
                name: "curve".into(),
                ty: FieldType::Curve,
                category: String::new(),
                is_name: false,
                default: DataValue::Curve(curve_id),
                settings: FieldSettings::default(),
            },
            Field {
                id: test_uuid(0x412),
                name: "gradient".into(),
                ty: FieldType::Gradient,
                category: String::new(),
                is_name: false,
                default: DataValue::Gradient(grad_id, 2),
                settings: FieldSettings::default(),
            },
            Field {
                id: test_uuid(0x413),
                name: "material".into(),
                ty: FieldType::Material,
                category: String::new(),
                is_name: false,
                default: DataValue::Material(mat_id),
                settings: FieldSettings::default(),
            },
            Field {
                id: test_uuid(0x414),
                name: "texture".into(),
                ty: FieldType::Texture,
                category: String::new(),
                is_name: false,
                default: DataValue::Texture(tex_id),
                settings: FieldSettings::default(),
            },
            Field {
                id: test_uuid(0x415),
                name: "env".into(),
                ty: FieldType::LightEnvironment,
                category: String::new(),
                is_name: true,
                default: DataValue::LightEnvironment(env_id),
                settings: FieldSettings::default(),
            },
            Field {
                id: test_uuid(0x416),
                name: "sound".into(),
                ty: FieldType::GameAsset(GameAssetKind::Audio),
                category: String::new(),
                is_name: true,
                default: DataValue::GameAsset("audio/chime.ogg".into()),
                settings: FieldSettings::default(),
            },
            Field {
                id: test_uuid(0x417),
                name: "icon".into(),
                ty: FieldType::GameAsset(GameAssetKind::Image),
                category: String::new(),
                is_name: false,
                default: DataValue::GameAsset("textures/albedo.png".into()),
                settings: FieldSettings::default(),
            },
        ],
    };

    let mut col = CollectionAsset {
        format_version: COLLECTION_FORMAT_VERSION,
        id: test_uuid(0x420),
        dto: dto.id,
        rows: vec![Row::from_dto_defaults(&dto)],
    };
    col.rows[0].id = test_uuid(0x421);

    let pre_index = AssetIndex::build(vec![mat_backend.index(&project), light_backend.index(&project)]);
    let dtos_map = BTreeMap::from([("vfx/preset".to_string(), dto.clone())]);

    let dto_disk = project.domain_dir(DATA).join("vfx/preset.dto.ron");
    let col_disk = project.domain_dir(DATA).join("vfx/presets.collection.ron");
    save_dto_with_index(&dto_disk, &mut dto, &dtos_map, Some(&pre_index)).unwrap();
    save_collection_with_index(&col_disk, &mut col, Some(&dto), &dtos_map, Some(&pre_index)).unwrap();

    // Verify generated DTO code when Light and Material are enabled:
    let full_index = AssetIndex::build(vec![
        mat_backend.index(&project),
        light_backend.index(&project),
        DataBackend.index(&project),
    ]);
    let dto_rs = emit_dto_file_with_context("vfx/preset", &dto, &dtos_map, Some(&project), Some(&full_index));
    assert!(dto_rs.contains("pub curve: super::CurveData,"), "{dto_rs}");
    assert!(dto_rs.contains("pub gradient: super::GradientData,"), "{dto_rs}");
    assert!(dto_rs.contains("pub material: Option<bevy::asset::UntypedHandle>,"), "{dto_rs}");
    assert!(dto_rs.contains("pub texture: Option<bevy::asset::Handle<bevy::image::Image>>,"), "{dto_rs}");
    assert!(dto_rs.contains("pub env: Option<crate::light::Preset>,"), "{dto_rs}");
    assert!(dto_rs.contains("pub sound: Option<bevy::asset::UntypedHandle>,"), "{dto_rs}");
    assert!(dto_rs.contains("pub icon: Option<bevy::asset::UntypedHandle>,"), "{dto_rs}");
    assert!(dto_rs.contains("env: crate::light::Preset::from_path(\"day\"),"), "{dto_rs}");
    assert!(dto_rs.contains("crate::light::Preset::from_path(&wire.4)"), "{dto_rs}");

    let mod_rs = emit_data_mod_with_context(
        &dtos_map,
        &BTreeMap::from([("vfx/presets".to_string(), col.clone())]),
        compute_schema_hash(&dtos_map),
        &BTreeSet::new(),
        Some(&project),
    );
    assert!(mod_rs.contains("baked: Option<Res<crate::material::BakedMaterials>>,"), "{mod_rs}");

    // Run full multi-domain regeneration and verify `.collection.qsd` is baked with resolved CurveData / GradientData / paths.
    let backends: [&dyn DomainBackend; 3] = [&mat_backend, &light_backend, &DataBackend];
    let report = project_core::regen::run(&project, &backends, &[], false, false).unwrap();
    assert!(!report.has_errors(), "{:?}", report.diagnostics);

    let qsd_path = project.output_dir(DATA).join("autogen/vfx/presets.collection.qsd");
    let qsd_v1 = std::fs::read(&qsd_path).unwrap();
    let schema_hash = compute_schema_hash(&dtos_map);
    let (_, payload_v1) =
        generated_data::verify_qsd_header(&qsd_v1, schema_hash, dto.id.as_u128()).expect("valid qsd header");

    type PresetWire = (CurveData, GradientData, u128, (u128, String), String, String, String);
    let wire_rows_v1: Vec<(u128, PresetWire)> = postcard::from_bytes(payload_v1).unwrap();
    assert_eq!(wire_rows_v1.len(), 1);
    let w1 = &wire_rows_v1[0].1;
    assert_eq!(w1.0, CurveData { interp: Interp::Simple, min: 0.0, max: 1.0, points: vec![(0.0, 0.0), (1.0, 1.0)] });
    assert!((w1.0.sample(0.5) - 0.5).abs() < 1e-5);
    assert_eq!(
        w1.1,
        GradientData {
            stops: vec![
                GradientStop { t: 0.0, color: Color::linear_rgba(0.0, 0.0, 0.0, 1.0), mid: Some(0.5) },
                GradientStop { t: 1.0, color: Color::linear_rgba(1.0, 1.0, 1.0, 1.0), mid: Some(0.5) },
            ],
        }
    );
    let mid_color = w1.1.sample(0.5).to_linear();
    assert!(mid_color.red > 0.1 && mid_color.red < 0.9);
    assert_eq!(w1.2, mat_id.as_u128());
    assert_eq!(w1.3, (tex_id.as_u128(), "textures/albedo.png".to_string()));
    assert_eq!(w1.4.as_str(), "day");
    assert_eq!(w1.5.as_str(), "audio/chime.ogg");
    assert_eq!(w1.6.as_str(), "textures/albedo.png");

    // Verify runtime handle helpers (`material_uuid_handle`, `texture_uuid_handle`, `game_asset_path_handle`, `DataResolveCtx`) and `ToShortViewString`:
    assert_eq!(w1.0.to_short_view_string(), "Curve(2 pts, 0.00..1.00)");
    assert_eq!(w1.1.to_short_view_string(), "Gradient(2 stops)");

    let mut mat_map = BTreeMap::new();
    let dummy_mat_handle: Handle<generated_data::QsdBlobAsset> =
        Handle::Uuid(bevy::asset::uuid::Uuid::from_u128(mat_id.as_u128()), std::marker::PhantomData);
    mat_map.insert(bevy::asset::uuid::Uuid::from_u128(mat_id.as_u128()), dummy_mat_handle.clone().untyped());
    let ctx_with_mat = DataResolveCtx { asset_server: None, material_by_id: Some(&mat_map), texture_by_path: None };
    let resolved_mat = ctx_with_mat.resolve_material(w1.2);
    assert_eq!(resolved_mat, Some(dummy_mat_handle.untyped()));
    assert_eq!(resolved_mat.to_short_view_string(), mat_id.to_string());
    assert!(material_uuid_handle(w1.2).is_some());
    assert_eq!(material_uuid_handle(0), None);

    let resolved_tex_fallback = ctx_with_mat.resolve_texture(w1.3.0, &w1.3.1);
    assert_eq!(resolved_tex_fallback, texture_uuid_handle(w1.3.0));
    assert_eq!(resolved_tex_fallback.to_short_view_string(), tex_id.to_string());
    assert_eq!(ctx_with_mat.resolve_game_asset(&w1.6), game_asset_path_handle(&w1.6));
    assert_eq!(game_asset_path_handle(""), None);

    // Also verify `texture_by_path` resolution in `DataResolveCtx` for both `resolve_texture` and `resolve_game_asset`:
    let mut tex_map = std::collections::HashMap::new();
    let dummy_img_handle: Handle<Image> =
        Handle::Uuid(bevy::asset::uuid::Uuid::from_u128(0xdead_beef), std::marker::PhantomData);
    tex_map.insert("textures/albedo.png".to_string(), dummy_img_handle.clone());
    let ctx_with_tex =
        DataResolveCtx { asset_server: None, material_by_id: Some(&mat_map), texture_by_path: Some(&tex_map) };
    assert_eq!(ctx_with_tex.resolve_texture(w1.3.0, &w1.3.1), Some(dummy_img_handle.clone()));
    assert_eq!(ctx_with_tex.resolve_game_asset(&w1.6), Some(dummy_img_handle.untyped()));

    // Now edit `curves/falloff.curve.ron` and `palettes.gradients.ron` on disk and run incremental `run_affected`!
    std::fs::write(
        &curve_file,
        format!(
            "(\n    format_version: 1,\n    id: \"{curve_id}\",\n    interp: CatmullRom,\n    points: [\n        (0.0, 0.25),\n        (0.5, 0.75),\n        (1.0, 1.0),\n    ],\n)\n"
        ),
    )
    .unwrap();
    std::fs::write(
        &grad_file,
        format!(
            "(\n    format_version: 2,\n    id: \"{grad_id}\",\n    slots: [\n        (\n            id: 2,\n            name: \"sunset\",\n            stops: [\n                (\n                    t: 0.0,\n                    color: [1.0, 0.0, 0.0, 1.0],\n                    mid: 0.5,\n                ),\n                (\n                    t: 0.5,\n                    color: [0.0, 1.0, 0.0, 1.0],\n                    mid: 0.5,\n                ),\n                (\n                    t: 1.0,\n                    color: [0.0, 0.0, 1.0, 1.0],\n                    mid: 0.5,\n                ),\n            ],\n        ),\n    ],\n)\n"
        ),
    )
    .unwrap();

    // Verify `AssetIndex::affected` traces `material:curves/falloff.curve.ron` -> `data:vfx/preset.dto.ron` & `data:vfx/presets.collection.ron`.
    let curve_asset_ref = AssetRef { domain: MATERIAL, path: "curves/falloff".into(), suffix: "curve" };
    let changed = BTreeSet::from([curve_asset_ref]);
    let affected = full_index.affected(&changed);
    assert!(
        affected.contains(&AssetRef { domain: DATA, path: "vfx/presets".into(), suffix: "collection" }),
        "editing a curve must mark referencing Data collection as affected: {affected:?}"
    );

    let report2 = project_core::regen::run_affected(&project, &backends, &changed, false, false).unwrap();
    assert!(!report2.has_errors(), "{:?}", report2.diagnostics);

    let qsd_v2 = std::fs::read(&qsd_path).unwrap();
    assert_ne!(qsd_v1, qsd_v2, "run_affected must re-bake .collection.qsd when referenced curve/gradient changes");
    let (_, payload_v2) =
        generated_data::verify_qsd_header(&qsd_v2, schema_hash, dto.id.as_u128()).expect("valid qsd header v2");
    let wire_rows_v2: Vec<(u128, PresetWire)> = postcard::from_bytes(payload_v2).unwrap();
    let w2 = &wire_rows_v2[0].1;
    assert_eq!(w2.0.interp, Interp::CatmullRom);
    assert_eq!(w2.0.points.len(), 3);
    assert!((w2.0.sample(0.5) - 0.75).abs() < 1e-5);
    assert_eq!(w2.1.stops.len(), 3);

    // Also verify `decode_collection_rows_with_dtos` and `bake_collection_with_context`:
    let manual_qsd = bake_collection_with_context(
        &col,
        &dto,
        &dtos_map,
        Some(&project),
        Some(&full_index),
        schema_hash,
        &BTreeSet::new(),
    )
    .unwrap();
    assert_eq!(manual_qsd, qsd_v2);
    let decoded_rows = decode_collection_rows_with_dtos(&qsd_v2, &dto, &dtos_map, schema_hash).unwrap();
    assert_eq!(decoded_rows.len(), 1);
    assert_eq!(decoded_rows[0].1.get(&test_uuid(0x413)), Some(&DataValue::Material(mat_id)));
    assert_eq!(decoded_rows[0].1.get(&test_uuid(0x414)), Some(&DataValue::Texture(tex_id)));
    assert_eq!(decoded_rows[0].1.get(&test_uuid(0x416)), Some(&DataValue::GameAsset("audio/chime.ogg".into())));
}

/// Test 6: `Option<T>`, recursive `Option<Box<Self>>`, `Array<T>`, `Set<K>` (with unit Enum keys), and `Map<K, V>`
/// end-to-end regeneration, codegen, `.qsd` baking, and `decode_collection_rows_with_schemas` round-trip.
#[test]
fn slice_5_option_recursive_and_containers_codegen_and_qsd_roundtrip() {
    use data_domain::backend::{
        ENUM_FORMAT_VERSION, EnumAsset, EnumVariant, bake_collection_with_schemas, compute_schema_hash_with_enums,
        decode_collection_rows_with_schemas, save_collection_with_schemas, save_dto_with_schemas, save_enum_with_index,
    };

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    let project = Project::create(dir.path()).unwrap();

    let rarity_id = test_uuid(0x900);
    let var_common = test_uuid(0x901);
    let var_rare = test_uuid(0x902);
    let var_mythic = test_uuid(0x903);
    let mut rarity_enum = EnumAsset {
        format_version: ENUM_FORMAT_VERSION,
        id: rarity_id,
        doc: "Loot rarity tier.".into(),
        variants: vec![
            EnumVariant { id: var_common, name: "common".into(), doc: String::new(), fields: Vec::new() },
            EnumVariant { id: var_rare, name: "rare".into(), doc: String::new(), fields: Vec::new() },
            EnumVariant { id: var_mythic, name: "mythic".into(), doc: String::new(), fields: Vec::new() },
        ],
    };

    let node_id = test_uuid(0x910);
    let f_name = test_uuid(0x911);
    let f_next = test_uuid(0x912);
    let f_tags = test_uuid(0x913);
    let f_tiers = test_uuid(0x914);
    let f_weights = test_uuid(0x915);

    let mut node_dto = DtoAsset {
        format_version: DTO_FORMAT_VERSION,
        id: node_id,
        doc: "Loot table node with recursive Option<Box<Self>> and containers.".into(),
        fields: vec![
            Field {
                id: f_name,
                name: "name".into(),
                ty: FieldType::String,
                category: String::new(),
                is_name: true,
                default: DataValue::String("tier_0".into()),
                settings: FieldSettings::default(),
            },
            Field {
                id: f_next,
                name: "fallback_node".into(),
                ty: FieldType::Option(Box::new(FieldType::Dto(node_id))),
                category: String::new(),
                is_name: false,
                default: DataValue::Option(None),
                settings: FieldSettings::default(),
            },
            Field {
                id: f_tags,
                name: "tags".into(),
                ty: FieldType::Array(Box::new(FieldType::String)),
                category: String::new(),
                is_name: false,
                default: DataValue::Array(vec![DataValue::String("dungeon".into())]),
                settings: FieldSettings::default(),
            },
            Field {
                id: f_tiers,
                name: "allowed_tiers".into(),
                ty: FieldType::Set(Box::new(FieldType::Enum(rarity_id))),
                category: String::new(),
                is_name: false,
                default: DataValue::Set(vec![DataValue::Enum { variant: var_common, fields: BTreeMap::new() }]),
                settings: FieldSettings::default(),
            },
            Field {
                id: f_weights,
                name: "tier_weights".into(),
                ty: FieldType::Map(Box::new(FieldType::Enum(rarity_id)), Box::new(FieldType::I32)),
                category: String::new(),
                is_name: false,
                default: DataValue::Map(vec![(
                    DataValue::Enum { variant: var_common, fields: BTreeMap::new() },
                    DataValue::I32(100),
                )]),
                settings: FieldSettings::default(),
            },
        ],
    };

    let child_node_val = DataValue::Dto(BTreeMap::from([
        (f_name, DataValue::String("tier_fallback".into())),
        (f_next, DataValue::Option(None)),
        (f_tags, DataValue::Array(vec![DataValue::String("fallback".into())])),
        (f_tiers, DataValue::Set(vec![DataValue::Enum { variant: var_common, fields: BTreeMap::new() }])),
        (
            f_weights,
            DataValue::Map(vec![(
                DataValue::Enum { variant: var_common, fields: BTreeMap::new() },
                DataValue::I32(50),
            )]),
        ),
    ]));

    let row0_id = test_uuid(0x921);
    let mut col = CollectionAsset {
        format_version: COLLECTION_FORMAT_VERSION,
        id: test_uuid(0x920),
        dto: node_id,
        rows: vec![Row {
            id: row0_id,
            values: BTreeMap::from([
                (f_name, DataValue::String("boss_chest".into())),
                (f_next, DataValue::Option(Some(Box::new(child_node_val.clone())))),
                (f_tags, DataValue::Array(vec![DataValue::String("boss".into()), DataValue::String("vault".into())])),
                (
                    f_tiers,
                    DataValue::Set(vec![
                        DataValue::Enum { variant: var_mythic, fields: BTreeMap::new() },
                        DataValue::Enum { variant: var_rare, fields: BTreeMap::new() },
                    ]),
                ),
                (
                    f_weights,
                    DataValue::Map(vec![
                        (DataValue::Enum { variant: var_mythic, fields: BTreeMap::new() }, DataValue::I32(5)),
                        (DataValue::Enum { variant: var_rare, fields: BTreeMap::new() }, DataValue::I32(25)),
                    ]),
                ),
            ]),
        }],
    };

    let dtos_map = BTreeMap::from([("loot/node".to_string(), node_dto.clone())]);
    let enums_map = BTreeMap::from([("loot/rarity".to_string(), rarity_enum.clone())]);
    save_enum_with_index(
        &project.domain_dir(DATA).join("loot/rarity.enum.ron"),
        &mut rarity_enum,
        &dtos_map,
        &enums_map,
        None,
    )
    .unwrap();
    save_dto_with_schemas(&project.domain_dir(DATA).join("loot/node.dto.ron"), &mut node_dto, &dtos_map, &enums_map)
        .unwrap();
    save_collection_with_schemas(
        &project.domain_dir(DATA).join("loot/nodes.collection.ron"),
        &mut col,
        Some(&node_dto),
        &dtos_map,
        &enums_map,
    )
    .unwrap();

    let report = project_core::regen::run(&project, &[&DataBackend], &[], false, false).unwrap();
    assert!(!report.has_errors(), "{:?}", report.diagnostics);

    let node_rs = std::fs::read_to_string(project.module_dir(DATA).join(dto_rust_rel_path("loot/node"))).unwrap();
    assert!(node_rs.contains("pub fallback_node: Option<Box<super::LootNode>>,"), "{node_rs}");
    assert!(node_rs.contains("pub tags: Vec<String>,"), "{node_rs}");
    assert!(node_rs.contains("pub allowed_tiers: std::collections::BTreeSet<super::LootRarity>,"), "{node_rs}");
    assert!(node_rs.contains("pub tier_weights: std::collections::BTreeMap<super::LootRarity, i32>,"), "{node_rs}");

    let qsd_path = project.output_dir(DATA).join("autogen/loot/nodes.collection.qsd");
    let qsd_bytes = std::fs::read(&qsd_path).unwrap();
    let schema_hash = compute_schema_hash_with_enums(&dtos_map, &enums_map);
    let manual_qsd =
        bake_collection_with_schemas(&col, &node_dto, &dtos_map, &enums_map, schema_hash, &BTreeSet::new()).unwrap();
    assert_eq!(qsd_bytes, manual_qsd);

    let decoded =
        decode_collection_rows_with_schemas(&qsd_bytes, &node_dto, &dtos_map, &enums_map, schema_hash).unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].0, row0_id);
    assert_eq!(decoded[0].1.get(&f_name), Some(&DataValue::String("boss_chest".into())));
    assert_eq!(decoded[0].1.get(&f_next), Some(&DataValue::Option(Some(Box::new(child_node_val)))));
    assert_eq!(
        decoded[0].1.get(&f_tags),
        Some(&DataValue::Array(vec![DataValue::String("boss".into()), DataValue::String("vault".into())]))
    );
    // `Set` and `Map` entries are sorted deterministically by `cmp_key` (`var_rare` < `var_mythic`):
    assert_eq!(
        decoded[0].1.get(&f_tiers),
        Some(&DataValue::Set(vec![
            DataValue::Enum { variant: var_rare, fields: BTreeMap::new() },
            DataValue::Enum { variant: var_mythic, fields: BTreeMap::new() },
        ]))
    );
    assert_eq!(
        decoded[0].1.get(&f_weights),
        Some(&DataValue::Map(vec![
            (DataValue::Enum { variant: var_rare, fields: BTreeMap::new() }, DataValue::I32(25)),
            (DataValue::Enum { variant: var_mythic, fields: BTreeMap::new() }, DataValue::I32(5)),
        ]))
    );
}
