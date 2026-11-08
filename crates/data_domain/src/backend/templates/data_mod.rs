use std::any::{Any, TypeId};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::marker::PhantomData;
use std::ops::Deref;
use std::time::Duration;

use bevy::asset::io::Reader;
use bevy::asset::{Asset, AssetApp, AssetEvent, AssetLoader, AssetServer, Assets, Handle, LoadContext, LoadState};
use bevy::color::{Color, LinearRgba};
use bevy::ecs::message::{Message, MessageReader, MessageWriter, Messages};
use bevy::ecs::schedule::{IntoScheduleConfigs, SystemSet};
use bevy::ecs::system::SystemParam;
use bevy::image::Image;
use bevy::math::{IVec2, IVec3, Quat, UVec2, UVec3, Vec2, Vec3, Vec4};
use bevy::prelude::{App, Plugin, PreUpdate, Res, ResMut, Resource, TypePath};
use serde::{Deserialize, Serialize};

pub use crate::common::{CurveData, GradientData, GradientStop, Interp};

/// 8-byte magic prefix of every `.collection.qsd` binary file.
pub const QSD_MAGIC: [u8; 8] = *b"QSDATA\0\0";

/// Binary container format version written at bytes `8..12`.
pub const QSD_VERSION: u32 = 1;

/// Fixed little-endian header length in bytes: `8 (magic) + 4 (version) + 8 (schema_hash) + 16 (dto_uuid) = 36`.
pub const QSD_HEADER_LEN: usize = 36;

/// Schedule set for the Data domain's `PreUpdate` loading and tracking systems.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct LoadSet;

/// Internal loading progress of the baked `.collection.qsd` data tables, aggregated into [`crate::QuillshadeProgress`].
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BakedDataProgress {
    /// Collections that have finished loading and decoding successfully.
    pub loaded: u32,
    /// Collections that failed to load or decode.
    pub failed: u32,
    /// Total baked collections registered in this project.
    pub total: u32,
}

/// Emitted once internally when all baked `.collection.qsd` data tables have finished loading.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct BakedDataReady;

/// UTF-8 string marked for localization in `.dto.ron` schemas (`FieldType::LocalizedString`).
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LocalizedString(pub String);

impl LocalizedString {
    /// Wraps `s` in a [`LocalizedString`].
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Borrows the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for LocalizedString {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for LocalizedString {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<&str> for LocalizedString {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for LocalizedString {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl std::fmt::Display for LocalizedString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Inclusive floating-point interval `[min, max]` (`FieldType::RangeF32`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RangeF32 {
    pub min: f32,
    pub max: f32,
}

impl Default for RangeF32 {
    fn default() -> Self {
        Self { min: 0.0, max: 1.0 }
    }
}

impl RangeF32 {
    /// Creates a new `[min, max]` floating-point range.
    pub const fn new(min: f32, max: f32) -> Self {
        Self { min, max }
    }

    /// Whether `v` lies within `self.min..=self.max`.
    pub fn contains(&self, v: f32) -> bool {
        v >= self.min && v <= self.max
    }
}

/// Inclusive integer interval `[min, max]` (`FieldType::RangeI32`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RangeI32 {
    pub min: i32,
    pub max: i32,
}

impl Default for RangeI32 {
    fn default() -> Self {
        Self { min: 0, max: 1 }
    }
}

impl RangeI32 {
    /// Creates a new `[min, max]` integer range.
    pub const fn new(min: i32, max: i32) -> Self {
        Self { min, max }
    }

    /// Whether `v` lies within `self.min..=self.max`.
    pub const fn contains(&self, v: i32) -> bool {
        v >= self.min && v <= self.max
    }
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

/// Short human-readable summary string for runtime inspection and HUD labels (ADR 0049 §3).
pub trait ToShortViewString {
    fn to_short_view_string(&self) -> String;
}

impl ToShortViewString for bool {
    fn to_short_view_string(&self) -> String {
        self.to_string()
    }
}

impl ToShortViewString for i8 {
    fn to_short_view_string(&self) -> String {
        self.to_string()
    }
}

impl ToShortViewString for i16 {
    fn to_short_view_string(&self) -> String {
        self.to_string()
    }
}

impl ToShortViewString for i32 {
    fn to_short_view_string(&self) -> String {
        self.to_string()
    }
}

impl ToShortViewString for i64 {
    fn to_short_view_string(&self) -> String {
        self.to_string()
    }
}

impl ToShortViewString for u8 {
    fn to_short_view_string(&self) -> String {
        self.to_string()
    }
}

impl ToShortViewString for u16 {
    fn to_short_view_string(&self) -> String {
        self.to_string()
    }
}

impl ToShortViewString for u32 {
    fn to_short_view_string(&self) -> String {
        self.to_string()
    }
}

impl ToShortViewString for u64 {
    fn to_short_view_string(&self) -> String {
        self.to_string()
    }
}

impl ToShortViewString for f32 {
    fn to_short_view_string(&self) -> String {
        format_compact_f64(f64::from(*self))
    }
}

impl ToShortViewString for f64 {
    fn to_short_view_string(&self) -> String {
        format_compact_f64(*self)
    }
}

impl ToShortViewString for String {
    fn to_short_view_string(&self) -> String {
        self.clone()
    }
}

impl ToShortViewString for str {
    fn to_short_view_string(&self) -> String {
        self.to_string()
    }
}

impl ToShortViewString for LocalizedString {
    fn to_short_view_string(&self) -> String {
        self.0.clone()
    }
}

impl ToShortViewString for Vec2 {
    fn to_short_view_string(&self) -> String {
        format!("({}, {})", format_compact_f64(f64::from(self.x)), format_compact_f64(f64::from(self.y)))
    }
}

impl ToShortViewString for Vec3 {
    fn to_short_view_string(&self) -> String {
        format!(
            "({}, {}, {})",
            format_compact_f64(f64::from(self.x)),
            format_compact_f64(f64::from(self.y)),
            format_compact_f64(f64::from(self.z))
        )
    }
}

impl ToShortViewString for Vec4 {
    fn to_short_view_string(&self) -> String {
        format!(
            "({}, {}, {}, {})",
            format_compact_f64(f64::from(self.x)),
            format_compact_f64(f64::from(self.y)),
            format_compact_f64(f64::from(self.z)),
            format_compact_f64(f64::from(self.w))
        )
    }
}

impl ToShortViewString for IVec2 {
    fn to_short_view_string(&self) -> String {
        format!("({}, {})", self.x, self.y)
    }
}

impl ToShortViewString for IVec3 {
    fn to_short_view_string(&self) -> String {
        format!("({}, {}, {})", self.x, self.y, self.z)
    }
}

impl ToShortViewString for UVec2 {
    fn to_short_view_string(&self) -> String {
        format!("({}, {})", self.x, self.y)
    }
}

impl ToShortViewString for UVec3 {
    fn to_short_view_string(&self) -> String {
        format!("({}, {}, {})", self.x, self.y, self.z)
    }
}

impl ToShortViewString for Quat {
    fn to_short_view_string(&self) -> String {
        format!(
            "({}, {}, {}, {})",
            format_compact_f64(f64::from(self.x)),
            format_compact_f64(f64::from(self.y)),
            format_compact_f64(f64::from(self.z)),
            format_compact_f64(f64::from(self.w))
        )
    }
}

impl ToShortViewString for Color {
    fn to_short_view_string(&self) -> String {
        let lin = LinearRgba::from(*self);
        format_color_hex([lin.red, lin.green, lin.blue, lin.alpha])
    }
}

impl ToShortViewString for Duration {
    fn to_short_view_string(&self) -> String {
        format!("{}s", format_compact_f64(f64::from(self.as_secs_f32())))
    }
}

impl ToShortViewString for RangeF32 {
    fn to_short_view_string(&self) -> String {
        format!("{}..{}", format_compact_f64(f64::from(self.min)), format_compact_f64(f64::from(self.max)))
    }
}

impl ToShortViewString for RangeI32 {
    fn to_short_view_string(&self) -> String {
        format!("{}..{}", self.min, self.max)
    }
}

impl ToShortViewString for CurveData {
    fn to_short_view_string(&self) -> String {
        format!("Curve({} pts, {:.2}..{:.2})", self.points.len(), self.min, self.max)
    }
}

impl ToShortViewString for GradientData {
    fn to_short_view_string(&self) -> String {
        format!("Gradient({} stops)", self.stops.len())
    }
}

impl ToShortViewString for bevy::asset::UntypedHandle {
    fn to_short_view_string(&self) -> String {
        match self.path() {
            Some(p) => p.to_string(),
            None => match self.id() {
                bevy::asset::UntypedAssetId::Uuid { uuid, .. } => uuid.to_string(),
                other => format!("{other:?}"),
            },
        }
    }
}

impl ToShortViewString for Handle<Image> {
    fn to_short_view_string(&self) -> String {
        match self.path() {
            Some(p) => p.to_string(),
            None => match self.id() {
                bevy::asset::AssetId::Uuid { uuid } => uuid.to_string(),
                other => format!("{other:?}"),
            },
        }
    }
}

impl<T: ToShortViewString> ToShortViewString for Option<T> {
    fn to_short_view_string(&self) -> String {
        match self {
            Some(v) => v.to_short_view_string(),
            None => String::from("None"),
        }
    }
}

impl<T: ToShortViewString> ToShortViewString for Box<T> {
    fn to_short_view_string(&self) -> String {
        (**self).to_short_view_string()
    }
}

impl<T> ToShortViewString for Vec<T> {
    fn to_short_view_string(&self) -> String {
        let len = self.len();
        if len == 1 { String::from("[1 item]") } else { format!("[{len} items]") }
    }
}

impl<K> ToShortViewString for BTreeSet<K> {
    fn to_short_view_string(&self) -> String {
        let len = self.len();
        if len == 1 { String::from("{1 item}") } else { format!("{{{len} items}}") }
    }
}

impl<K, V> ToShortViewString for BTreeMap<K, V> {
    fn to_short_view_string(&self) -> String {
        let len = self.len();
        if len == 1 { String::from("{1 entry}") } else { format!("{{{len} entries}}") }
    }
}

/// Runtime resolution context for material, texture, and raw game asset handles during `.qsd` decoding.
#[derive(Default, Clone, Copy)]
pub struct DataResolveCtx<'a> {
    pub asset_server: Option<&'a AssetServer>,
    pub material_by_id: Option<&'a BTreeMap<bevy::asset::uuid::Uuid, bevy::asset::UntypedHandle>>,
    pub texture_by_path: Option<&'a HashMap<String, Handle<Image>>>,
}

/// Constructs a deterministic UUID-backed [`bevy::asset::UntypedHandle`] for a material asset UUID (`None` when `0`).
#[inline]
pub fn material_uuid_handle(raw: u128) -> Option<bevy::asset::UntypedHandle> {
    if raw == 0 {
        None
    } else {
        let uuid = bevy::asset::uuid::Uuid::from_u128(raw);
        Some(Handle::<bevy::asset::LoadedUntypedAsset>::Uuid(uuid, PhantomData).untyped())
    }
}

/// Constructs a deterministic UUID-backed [`Handle<Image>`] for a texture asset UUID (`None` when `0`).
#[inline]
pub fn texture_uuid_handle(raw: u128) -> Option<Handle<Image>> {
    if raw == 0 {
        None
    } else {
        let uuid = bevy::asset::uuid::Uuid::from_u128(raw);
        Some(Handle::<Image>::Uuid(uuid, PhantomData))
    }
}

/// Constructs a deterministic UUID-backed [`bevy::asset::UntypedHandle`] hashed from `path` (`None` when empty).
#[inline]
pub fn game_asset_path_handle(path: &str) -> Option<bevy::asset::UntypedHandle> {
    if path.is_empty() {
        None
    } else {
        let mut h1: u64 = 0xcbf29ce484222325;
        let mut h2: u64 = 0x84222325cbf29ce4;
        for &b in path.as_bytes() {
            h1 ^= u64::from(b);
            h1 = h1.wrapping_mul(0x100000001b3);
            h2 = h2.wrapping_mul(0x100000001b3) ^ u64::from(b);
        }
        let raw = (u128::from(h1) << 64) | u128::from(h2) | 1;
        material_uuid_handle(raw)
    }
}

impl DataResolveCtx<'_> {
    /// Resolves a material UUID against `BakedMaterials::by_id` when available, falling back to [`material_uuid_handle`].
    pub fn resolve_material(&self, raw: u128) -> Option<bevy::asset::UntypedHandle> {
        if raw == 0 {
            return None;
        }
        let u = bevy::asset::uuid::Uuid::from_u128(raw);
        if let Some(map) = self.material_by_id
            && let Some(h) = map.get(&u)
        {
            return Some(h.clone());
        }
        material_uuid_handle(raw)
    }

    /// Resolves a texture UUID and `assets/`-relative `path` against `BakedTextures::by_path` or [`AssetServer::load`]
    /// when available, falling back to [`texture_uuid_handle`].
    pub fn resolve_texture(&self, raw: u128, path: &str) -> Option<Handle<Image>> {
        if raw == 0 {
            return None;
        }
        if !path.is_empty() {
            if let Some(map) = self.texture_by_path
                && let Some(h) = map.get(path)
            {
                return Some(h.clone());
            }
            if let Some(server) = self.asset_server {
                return Some(server.load::<Image>(path.to_owned()));
            }
        }
        texture_uuid_handle(raw)
    }

    /// Resolves an `assets/`-relative game file `path` against `BakedTextures::by_path` or [`AssetServer::load`] when
    /// available, falling back to [`game_asset_path_handle`].
    pub fn resolve_game_asset(&self, path: &str) -> Option<bevy::asset::UntypedHandle> {
        if path.is_empty() {
            return None;
        }
        if let Some(map) = self.texture_by_path
            && let Some(h) = map.get(path)
        {
            return Some(h.clone().untyped());
        }
        if let Some(server) = self.asset_server {
            return Some(server.load::<bevy::asset::LoadedUntypedAsset>(path.to_owned()).untyped());
        }
        game_asset_path_handle(path)
    }
}

fn format_uuid_u128(n: u128, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let a = (n >> 96) as u32;
    let b = ((n >> 80) & 0xffff) as u16;
    let c = ((n >> 64) & 0xffff) as u16;
    let d = ((n >> 48) & 0xffff) as u16;
    let e = n & 0xffff_ffff_ffff;
    write!(f, "{a:08x}-{b:04x}-{c:04x}-{d:04x}-{e:012x}")
}

const fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Parses a canonical 36-character hyphenated UUID (`8-4-4-4-12`) or 32-character hex string into a `u128`.
pub const fn parse_uuid_u128(s: &str) -> Option<u128> {
    let bytes = s.as_bytes();
    if bytes.len() != 32 && bytes.len() != 36 {
        return None;
    }
    if bytes.len() == 36
        && (bytes[8] != b'-' || bytes[13] != b'-' || bytes[18] != b'-' || bytes[23] != b'-')
    {
        return None;
    }
    let mut acc: u128 = 0;
    let mut digits: usize = 0;
    let mut i: usize = 0;
    while i < bytes.len() {
        let b = bytes[i];
        i += 1;
        if b == b'-' {
            continue;
        }
        let Some(nibble) = hex_nibble(b) else {
            return None;
        };
        acc = (acc << 4) | (nibble as u128);
        digits += 1;
    }
    if digits == 32 { Some(acc) } else { None }
}

/// Strongly-typed 128-bit UUIDv7 handle identifying a row of DTO `T` (`Row.id`).
pub struct RowUuid<T> {
    id: u128,
    _marker: PhantomData<fn() -> T>,
}

impl<T> RowUuid<T> {
    /// Wraps a raw 128-bit UUID value into a typed [`RowUuid`].
    pub const fn from_u128(id: u128) -> Self {
        Self { id, _marker: PhantomData }
    }

    /// Parses a UUID string (`8-4-4-4-12` or 32 hex digits) into a typed [`RowUuid`].
    pub const fn parse(s: &str) -> Option<Self> {
        match parse_uuid_u128(s) {
            Some(id) => Some(Self::from_u128(id)),
            None => None,
        }
    }

    /// Returns the underlying 128-bit UUID value.
    pub const fn as_u128(self) -> u128 {
        self.id
    }
}

impl<T> Clone for RowUuid<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for RowUuid<T> {}

impl<T> PartialEq for RowUuid<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T> Eq for RowUuid<T> {}

impl<T> PartialOrd for RowUuid<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for RowUuid<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.id.cmp(&other.id)
    }
}

impl<T> std::hash::Hash for RowUuid<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T> std::fmt::Debug for RowUuid<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RowUuid({self})")
    }
}

impl<T> std::fmt::Display for RowUuid<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        format_uuid_u128(self.id, f)
    }
}

mod private {
    pub trait Sealed {}
}

/// Converts a row identifier ([`RowUuid<T>`], `&RowUuid<T>`, `u128`, or UUID string) into a [`RowUuid<T>`] for
/// $O(1)$ row lookup via [`Collection::get`] and [`CollectionHandle::get`].
pub trait IntoRowUuid<T>: private::Sealed {
    fn into_row_uuid(self) -> Option<RowUuid<T>>;
}

impl<T> private::Sealed for RowUuid<T> {}
impl<T> IntoRowUuid<T> for RowUuid<T> {
    #[inline]
    fn into_row_uuid(self) -> Option<RowUuid<T>> {
        Some(self)
    }
}

impl<T> private::Sealed for &RowUuid<T> {}
impl<T> IntoRowUuid<T> for &RowUuid<T> {
    #[inline]
    fn into_row_uuid(self) -> Option<RowUuid<T>> {
        Some(*self)
    }
}

impl private::Sealed for u128 {}
impl<T> IntoRowUuid<T> for u128 {
    #[inline]
    fn into_row_uuid(self) -> Option<RowUuid<T>> {
        Some(RowUuid::from_u128(self))
    }
}

impl private::Sealed for &str {}
impl<T> IntoRowUuid<T> for &str {
    #[inline]
    fn into_row_uuid(self) -> Option<RowUuid<T>> {
        RowUuid::parse(self)
    }
}

impl private::Sealed for String {}
impl<T> IntoRowUuid<T> for String {
    #[inline]
    fn into_row_uuid(self) -> Option<RowUuid<T>> {
        RowUuid::parse(&self)
    }
}

impl private::Sealed for &String {}
impl<T> IntoRowUuid<T> for &String {
    #[inline]
    fn into_row_uuid(self) -> Option<RowUuid<T>> {
        RowUuid::parse(self.as_str())
    }
}

/// Optional cross-collection reference to a row of DTO `T` (`FieldType::SoftRef`, ADR 0049 §6).
pub struct SoftRef<T>(Option<RowUuid<T>>);

impl<T> SoftRef<T> {
    /// Unset (`None`) soft reference constant.
    pub const NONE: Self = Self(None);

    /// Constructs a [`SoftRef`] from a raw 128-bit UUID (`0` decodes to `None`).
    #[inline]
    pub const fn from_raw(raw: u128) -> Self {
        if raw == 0 { Self(None) } else { Self(Some(RowUuid::from_u128(raw))) }
    }

    /// Wraps a [`RowUuid<T>`] in a set [`SoftRef`].
    #[inline]
    pub const fn new(id: RowUuid<T>) -> Self {
        Self(Some(id))
    }

    /// Whether this reference is unset (`None`).
    #[inline]
    pub const fn is_none(&self) -> bool {
        self.0.is_none()
    }

    /// Whether this reference is set (`Some`).
    #[inline]
    pub const fn is_some(&self) -> bool {
        self.0.is_some()
    }

    /// Returns the underlying [`RowUuid<T>`], if set.
    #[inline]
    pub const fn id(&self) -> Option<RowUuid<T>> {
        self.0
    }

    /// Returns the raw 128-bit UUID (`0` when unset).
    #[inline]
    pub const fn as_u128(&self) -> u128 {
        match self.0 {
            Some(id) => id.as_u128(),
            None => 0,
        }
    }

    /// Resolves the referenced row across all loaded collections of `T` in [`Data`].
    #[inline]
    pub fn resolve<'a>(&self, data: &'a Data<'_>) -> Option<&'a T>
    where
        T: 'static,
    {
        data.resolve_row(self.0?)
    }

    /// Resolves the referenced row across all loaded collections of `T` in [`DataStore`].
    #[inline]
    pub fn get<'a>(&self, store: &'a DataStore) -> Option<&'a T>
    where
        T: 'static,
    {
        store.resolve_row(self.0?)
    }
}

impl<T> Clone for SoftRef<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for SoftRef<T> {}

impl<T> Default for SoftRef<T> {
    #[inline]
    fn default() -> Self {
        Self::NONE
    }
}

impl<T> PartialEq for SoftRef<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T> Eq for SoftRef<T> {}

impl<T> PartialOrd for SoftRef<T> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for SoftRef<T> {
    #[inline]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_u128().cmp(&other.as_u128())
    }
}

impl<T> std::hash::Hash for SoftRef<T> {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<T> std::fmt::Debug for SoftRef<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(id) => write!(f, "SoftRef({id})"),
            None => f.write_str("SoftRef(None)"),
        }
    }
}

impl<T> private::Sealed for SoftRef<T> {}
impl<T> IntoRowUuid<T> for SoftRef<T> {
    #[inline]
    fn into_row_uuid(self) -> Option<RowUuid<T>> {
        self.0
    }
}

impl<T> private::Sealed for &SoftRef<T> {}
impl<T> IntoRowUuid<T> for &SoftRef<T> {
    #[inline]
    fn into_row_uuid(self) -> Option<RowUuid<T>> {
        self.0
    }
}

impl<T> ToShortViewString for SoftRef<T> {
    fn to_short_view_string(&self) -> String {
        match self.0 {
            Some(id) => id.to_string(),
            None => String::from("<none>"),
        }
    }
}

/// Strongly-typed 128-bit UUIDv7 handle identifying a `.collection.ron` table of DTO `T`.
pub struct CollectionUuid<T> {
    id: u128,
    _marker: PhantomData<fn() -> T>,
}

impl<T> CollectionUuid<T> {
    /// Wraps a raw 128-bit UUID value into a typed [`CollectionUuid`].
    pub const fn from_u128(id: u128) -> Self {
        Self { id, _marker: PhantomData }
    }

    /// Parses a UUID string (`8-4-4-4-12` or 32 hex digits) into a typed [`CollectionUuid`].
    pub const fn parse(s: &str) -> Option<Self> {
        match parse_uuid_u128(s) {
            Some(id) => Some(Self::from_u128(id)),
            None => None,
        }
    }

    /// Returns the underlying 128-bit UUID value.
    pub const fn as_u128(self) -> u128 {
        self.id
    }
}

impl<T> Clone for CollectionUuid<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for CollectionUuid<T> {}

impl<T> PartialEq for CollectionUuid<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T> Eq for CollectionUuid<T> {}

impl<T> PartialOrd for CollectionUuid<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for CollectionUuid<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.id.cmp(&other.id)
    }
}

impl<T> std::hash::Hash for CollectionUuid<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T> std::fmt::Debug for CollectionUuid<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CollectionUuid({self})")
    }
}

impl<T> std::fmt::Display for CollectionUuid<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        format_uuid_u128(self.id, f)
    }
}

/// Iterator over `(RowUuid<T>, &'a T)` pairs in a [`Collection<T>`] or [`CollectionHandle<'a, T>`].
#[derive(Debug)]
pub struct CollectionIter<'a, T> {
    inner: std::slice::Iter<'a, (RowUuid<T>, T)>,
}

impl<T> Clone for CollectionIter<'_, T> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

impl<'a, T> Iterator for CollectionIter<'a, T> {
    type Item = (RowUuid<T>, &'a T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(id, row)| (*id, row))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }

    #[inline]
    fn count(self) -> usize {
        self.inner.count()
    }

    #[inline]
    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        self.inner.nth(n).map(|(id, row)| (*id, row))
    }

    #[inline]
    fn last(mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(id, row)| (*id, row))
    }
}

impl<T> DoubleEndedIterator for CollectionIter<'_, T> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(id, row)| (*id, row))
    }
}

impl<T> ExactSizeIterator for CollectionIter<'_, T> {
    #[inline]
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<T> std::iter::FusedIterator for CollectionIter<'_, T> {}

/// Ordered table of UUIDv7-identified rows of DTO `T` loaded from a `.collection.qsd` asset (D7).
#[derive(Clone, Debug)]
pub struct Collection<T> {
    /// Compile-time row count (before `.qsd` finishes loading) or live row count after decode.
    pub len: usize,
    rows: Vec<(RowUuid<T>, T)>,
    by_id: Option<HashMap<RowUuid<T>, usize>>,
}

impl<T: PartialEq> PartialEq for Collection<T> {
    fn eq(&self, other: &Self) -> bool {
        self.rows == other.rows
    }
}

impl<T> Default for Collection<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Collection<T> {
    /// Empty [`Collection`] constant used as a fallback before async `.qsd` loading completes.
    pub const EMPTY: Self = Self { len: 0, rows: Vec::new(), by_id: None };
    const EMPTY_STORAGE: std::mem::ManuallyDrop<Self> = std::mem::ManuallyDrop::new(Self::EMPTY);

    /// Returns a `'static` reference to an empty [`Collection<T>`].
    #[inline]
    pub fn empty_ref<'a>() -> &'a Self {
        &Self::EMPTY_STORAGE
    }

    /// Creates an empty [`Collection`].
    pub const fn new() -> Self {
        Self::EMPTY
    }

    /// Creates an unloaded [`Collection`] pre-populated with its compile-time row count `len`.
    pub const fn with_compile_time_len(len: usize) -> Self {
        Self { len, rows: Vec::new(), by_id: None }
    }

    /// Builds a [`Collection`] from ordered `(RowUuid<T>, T)` pairs and constructs its $O(1)$ UUID lookup index.
    pub fn from_rows(rows: Vec<(RowUuid<T>, T)>) -> Self {
        let len = rows.len();
        let mut by_id = HashMap::with_capacity(len);
        for (idx, (id, _)) in rows.iter().enumerate() {
            by_id.entry(*id).or_insert(idx);
        }
        Self { len, rows, by_id: Some(by_id) }
    }

    /// Looks up a row in $O(1)$ time by its [`RowUuid<T>`], `&RowUuid<T>`, `u128`, or UUID string.
    #[inline]
    pub fn get(&self, id: impl IntoRowUuid<T>) -> Option<&T> {
        let id = id.into_row_uuid()?;
        if let Some(by_id) = &self.by_id {
            let &idx = by_id.get(&id)?;
            self.rows.get(idx).map(|(_, row)| row)
        } else {
            self.rows.iter().find_map(|(r_id, row)| (*r_id == id).then_some(row))
        }
    }

    /// Iterates over `(RowUuid<T>, &T)` pairs in collection row order.
    #[inline]
    pub fn iter(&self) -> CollectionIter<'_, T> {
        CollectionIter { inner: self.rows.iter() }
    }

    /// Iterates over `&T` row values in collection row order.
    #[inline]
    pub fn values(&self) -> impl ExactSizeIterator<Item = &T> + DoubleEndedIterator {
        self.rows.iter().map(|(_, row)| row)
    }

    /// Returns the first row in the collection, if loaded (convenience for 1-row configuration collections).
    #[inline]
    pub fn first(&self) -> Option<&T> {
        self.rows.first().map(|(_, row)| row)
    }

    /// Returns the first `(RowUuid<T>, &T)` pair in the collection, if loaded.
    #[inline]
    pub fn first_with_id(&self) -> Option<(RowUuid<T>, &T)> {
        self.rows.first().map(|(id, row)| (*id, row))
    }

    /// Row count of the collection (compile-time count before async load finishes, or loaded row count).
    #[inline]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Number of rows currently loaded in memory (`0` until the `.collection.qsd` asset finishes loading).
    #[inline]
    pub fn loaded_len(&self) -> usize {
        self.rows.len()
    }

    /// Whether no rows are currently loaded in memory.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Returns the 0-based index of `id` in $O(1)$ time, if present.
    #[inline]
    pub fn index_of(&self, id: impl IntoRowUuid<T>) -> Option<usize> {
        let id = id.into_row_uuid()?;
        if let Some(by_id) = &self.by_id {
            by_id.get(&id).copied()
        } else {
            self.rows.iter().position(|(r_id, _)| *r_id == id)
        }
    }

    /// Returns the underlying slice of loaded `(RowUuid<T>, T)` pairs.
    #[inline]
    pub fn as_slice(&self) -> &[(RowUuid<T>, T)] {
        &self.rows
    }
}

impl<'a, T> IntoIterator for &'a Collection<T> {
    type Item = (RowUuid<T>, &'a T);
    type IntoIter = CollectionIter<'a, T>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Typed, zero-cost handle to a compile-time known collection in [`Data`].
///
/// Implements [`Copy`] and [`IntoIterator`] so `for (id, row) in data.collection_foo`
/// iterates directly without moving `data`.
pub struct CollectionHandle<'w, T: 'static> {
    /// Compile-time row count of this `.collection.ron` table (or live row count after `.qsd` hot-reload).
    pub len: usize,
    table: &'w Collection<T>,
}

impl<T: 'static> Clone for CollectionHandle<'_, T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for CollectionHandle<'_, T> {}

impl<T: std::fmt::Debug + 'static> std::fmt::Debug for CollectionHandle<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CollectionHandle")
            .field("len", &self.len)
            .field("loaded_len", &self.table.rows.len())
            .field("table", &self.table)
            .finish()
    }
}

impl<T: 'static> Deref for CollectionHandle<'_, T> {
    type Target = Collection<T>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.table
    }
}

impl<'w, T: 'static> CollectionHandle<'w, T> {
    /// Creates a [`CollectionHandle`] wrapping `table` with compile-time row count `compile_len`.
    #[inline]
    pub const fn new(table: &'w Collection<T>, compile_len: usize) -> Self {
        let len = if table.by_id.is_some() { table.len } else { compile_len };
        Self { len, table }
    }

    /// Row count of the collection (compile-time count before async load finishes, or loaded row count).
    #[inline]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Number of rows currently loaded in memory (`0` until the `.collection.qsd` asset finishes loading).
    #[inline]
    pub fn loaded_len(&self) -> usize {
        self.table.rows.len()
    }

    /// Whether no rows are currently loaded in memory.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Whether the collection's rows are available (`true` if `self.len == 0` or rows have been loaded).
    #[inline]
    pub fn is_loaded(&self) -> bool {
        self.table.by_id.is_some() || self.len == 0
    }

    /// Iterates over `(RowUuid<T>, &'w T)` pairs in collection row order.
    #[inline]
    pub fn iter(&self) -> CollectionIter<'w, T> {
        self.table.iter()
    }

    /// Iterates over `&'w T` row values in collection row order.
    #[inline]
    pub fn values(&self) -> impl ExactSizeIterator<Item = &'w T> + DoubleEndedIterator + '_ {
        self.table.values()
    }

    /// Looks up a row in $O(1)$ time by its [`RowUuid<T>`], `&RowUuid<T>`, `u128`, or UUID string.
    #[inline]
    pub fn get(&self, id: impl IntoRowUuid<T>) -> Option<&'w T> {
        self.table.get(id)
    }

    /// Returns the 0-based index of `id` in $O(1)$ time, if present.
    #[inline]
    pub fn index_of(&self, id: impl IntoRowUuid<T>) -> Option<usize> {
        self.table.index_of(id)
    }

    /// Returns the first row in the collection, if loaded (convenience for 1-row configuration collections).
    #[inline]
    pub fn first(&self) -> Option<&'w T> {
        self.table.first()
    }

    /// Returns the first `(RowUuid<T>, &'w T)` pair in the collection, if loaded.
    #[inline]
    pub fn first_with_id(&self) -> Option<(RowUuid<T>, &'w T)> {
        self.table.first_with_id()
    }

    /// Borrows the underlying [`Collection<T>`].
    #[inline]
    pub const fn as_collection(&self) -> &'w Collection<T> {
        self.table
    }
}

impl<'w, T: 'static> IntoIterator for CollectionHandle<'w, T> {
    type Item = (RowUuid<T>, &'w T);
    type IntoIter = CollectionIter<'w, T>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.table.iter()
    }
}

impl<'w, T: 'static> IntoIterator for &CollectionHandle<'w, T> {
    type Item = (RowUuid<T>, &'w T);
    type IntoIter = CollectionIter<'w, T>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.table.iter()
    }
}

/// Trait implemented by every generated DTO struct in [`dtos`].
pub trait DataDto: Sized + Send + Sync + 'static {
    /// Stable 128-bit UUIDv7 of the `.dto.ron` schema.
    const DTO_ID: u128;

    /// Decodes a `postcard`-encoded `.collection.qsd` body into a [`Collection<Self>`].
    fn decode_qsd_payload(payload: &[u8], ctx: &DataResolveCtx<'_>) -> Result<Collection<Self>, String>;
}

/// Parsed 36-byte `.qsd` container header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QsdHeader {
    pub version: u32,
    pub schema_hash: u64,
    pub dto_uuid: u128,
}

/// Non-panicking error returned when verifying or decoding a `.collection.qsd` binary buffer at runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QsdRuntimeError {
    TooShort(usize),
    BadMagic,
    UnsupportedVersion { found: u32 },
    SchemaHashMismatch { expected: u64, found: u64 },
    DtoMismatch { expected: u128, found: u128 },
    Decode(String),
}

impl std::fmt::Display for QsdRuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort(len) => {
                write!(f, "truncated .qsd file: expected at least {QSD_HEADER_LEN} bytes, got {len}")
            }
            Self::BadMagic => f.write_str("invalid .qsd magic header (expected `QSDATA\\0\\0`)"),
            Self::UnsupportedVersion { found } => {
                write!(f, "unsupported .qsd container version {found} (expected {QSD_VERSION})")
            }
            Self::SchemaHashMismatch { expected, found } => {
                write!(f, "schema hash mismatch: expected {expected:#018x}, found {found:#018x}; rebuild game")
            }
            Self::DtoMismatch { expected, found } => {
                write!(f, "DTO UUID mismatch: expected {expected:#034x}, found {found:#034x}")
            }
            Self::Decode(msg) => write!(f, "postcard decode error: {msg}"),
        }
    }
}

impl std::error::Error for QsdRuntimeError {}

/// Parses the 36-byte header of a `.qsd` buffer, checking [`QSD_MAGIC`] and [`QSD_VERSION`].
pub fn parse_qsd_header(bytes: &[u8]) -> Result<QsdHeader, QsdRuntimeError> {
    if bytes.len() < QSD_HEADER_LEN {
        return Err(QsdRuntimeError::TooShort(bytes.len()));
    }
    if bytes[0..8] != QSD_MAGIC {
        return Err(QsdRuntimeError::BadMagic);
    }
    let version = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    if version != QSD_VERSION {
        return Err(QsdRuntimeError::UnsupportedVersion { found: version });
    }
    let schema_hash = u64::from_le_bytes([
        bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17], bytes[18], bytes[19],
    ]);
    let dto_uuid = u128::from_le_bytes([
        bytes[20], bytes[21], bytes[22], bytes[23], bytes[24], bytes[25], bytes[26], bytes[27], bytes[28], bytes[29],
        bytes[30], bytes[31], bytes[32], bytes[33], bytes[34], bytes[35],
    ]);
    Ok(QsdHeader { version, schema_hash, dto_uuid })
}

/// Verifies the 36-byte `.qsd` header against `expected_schema_hash` and `expected_dto_uuid`, returning
/// `(header, payload_slice)` without panicking (D1, D21).
pub fn verify_qsd_header(
    bytes: &[u8],
    expected_schema_hash: u64,
    expected_dto_uuid: u128,
) -> Result<(QsdHeader, &[u8]), QsdRuntimeError> {
    let header = parse_qsd_header(bytes)?;
    if header.schema_hash != expected_schema_hash {
        return Err(QsdRuntimeError::SchemaHashMismatch {
            expected: expected_schema_hash,
            found: header.schema_hash,
        });
    }
    if header.dto_uuid != expected_dto_uuid {
        return Err(QsdRuntimeError::DtoMismatch { expected: expected_dto_uuid, found: header.dto_uuid });
    }
    Ok((header, &bytes[QSD_HEADER_LEN..]))
}

/// Binary asset holding the raw bytes of a baked `.collection.qsd` file.
#[derive(Asset, TypePath, Debug, Clone, PartialEq, Eq)]
pub struct QsdBlobAsset {
    pub bytes: Vec<u8>,
}

/// Bevy [`AssetLoader`] for `.collection.qsd` binary files.
#[derive(Default, TypePath)]
pub struct QsdBlobLoader;

impl AssetLoader for QsdBlobLoader {
    type Asset = QsdBlobAsset;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load_context: &mut LoadContext<'_>,
    ) -> Result<QsdBlobAsset, std::io::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(QsdBlobAsset { bytes })
    }

    fn extensions(&self) -> &[&str] {
        &["collection.qsd", "qsd"]
    }
}

#[derive(Debug)]
struct TypedDtoStore<T> {
    collections: BTreeMap<CollectionUuid<T>, Collection<T>>,
    row_index: HashMap<RowUuid<T>, (CollectionUuid<T>, usize)>,
}

impl<T> Default for TypedDtoStore<T> {
    fn default() -> Self {
        Self { collections: BTreeMap::new(), row_index: HashMap::new() }
    }
}

impl<T: 'static> TypedDtoStore<T> {
    fn init_collection(&mut self, col_id: CollectionUuid<T>, compile_len: usize) {
        self.collections
            .entry(col_id)
            .or_insert_with(|| Collection::with_compile_time_len(compile_len));
    }

    fn insert_collection(&mut self, col_id: CollectionUuid<T>, collection: Collection<T>) {
        self.collections.insert(col_id, collection);
        self.rebuild_row_index();
    }

    fn rebuild_row_index(&mut self) {
        self.row_index.clear();
        for (&col_id, col) in &self.collections {
            for (idx, (row_id, _)) in col.iter().enumerate() {
                self.row_index.entry(row_id).or_insert((col_id, idx));
            }
        }
    }

    fn collection(&self, id: CollectionUuid<T>) -> &Collection<T> {
        self.collections.get(&id).unwrap_or_else(|| Collection::<T>::empty_ref())
    }

    fn get(&self, col: CollectionUuid<T>, row: RowUuid<T>) -> Option<&T> {
        self.collections.get(&col).and_then(|c| c.get(row))
    }

    fn find_row(&self, row: RowUuid<T>) -> Option<&T> {
        let &(col_id, idx) = self.row_index.get(&row)?;
        self.collections.get(&col_id)?.rows.get(idx).map(|(_, val)| val)
    }
}

/// Runtime resource storing decoded [`Collection`]s, global row indices, and strong [`QsdBlobAsset`] handles for
/// hot-reload.
#[derive(Resource, Debug)]
pub struct DataStore {
    stores: BTreeMap<TypeId, Box<dyn Any + Send + Sync>>,
    handles: BTreeMap<u128, (String, Handle<QsdBlobAsset>)>,
    loaded_collections: BTreeSet<u128>,
    failed_collections: BTreeSet<u128>,
    expected_collections: usize,
    started: bool,
    ready: bool,
    sent_progress: Option<BakedDataProgress>,
    pub(crate) asset_server: Option<AssetServer>,
    pub(crate) material_by_id: BTreeMap<bevy::asset::uuid::Uuid, bevy::asset::UntypedHandle>,
    pub(crate) texture_by_path: HashMap<String, Handle<Image>>,
}

impl Default for DataStore {
    fn default() -> Self {
        let mut store = Self {
            stores: BTreeMap::new(),
            handles: BTreeMap::new(),
            loaded_collections: BTreeSet::new(),
            failed_collections: BTreeSet::new(),
            expected_collections: BAKED_COLLECTIONS.len(),
            started: false,
            ready: false,
            sent_progress: None,
            asset_server: None,
            material_by_id: BTreeMap::new(),
            texture_by_path: HashMap::new(),
        };
        init_registered_collections(&mut store);
        store
    }
}

impl DataStore {
    /// Pre-registers a compile-time known collection with its compile-time row count so [`Self::collection`]
    /// always returns a table with `len == compile_len` even before async `.qsd` loading completes.
    #[allow(dead_code)]
    pub(crate) fn init_collection<T: DataDto>(&mut self, col_id: CollectionUuid<T>, compile_len: usize) {
        let store = self
            .stores
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(TypedDtoStore::<T>::default()))
            .downcast_mut::<TypedDtoStore<T>>()
            .expect("TypedDtoStore type matches TypeId::of::<T>()");
        store.init_collection(col_id, compile_len);
    }

    /// Returns the [`Collection<T>`] for `id` (guaranteed to exist; empty before async `.qsd` load completes).
    #[inline]
    pub fn collection<T: 'static>(&self, id: CollectionUuid<T>) -> &Collection<T> {
        self.stores
            .get(&TypeId::of::<T>())
            .and_then(|any| any.downcast_ref::<TypedDtoStore<T>>())
            .map_or_else(Collection::<T>::empty_ref, |store| store.collection(id))
    }

    /// Returns the first row of collection `id`, if loaded (convenience for 1-row configuration collections).
    #[inline]
    pub fn first<T: 'static>(&self, id: CollectionUuid<T>) -> Option<&T> {
        self.collection(id).first()
    }

    /// Looks up a single row inside collection `col` in $O(1)$ time by its [`RowUuid<T>`], `u128`, or UUID string.
    #[inline]
    pub fn get<T: 'static>(&self, col: CollectionUuid<T>, row: impl IntoRowUuid<T>) -> Option<&T> {
        let row = row.into_row_uuid()?;
        self.stores
            .get(&TypeId::of::<T>())
            .and_then(|any| any.downcast_ref::<TypedDtoStore<T>>())
            .and_then(|store| store.get(col, row))
    }

    /// Looks up a row by its [`RowUuid<T>`], `u128`, or UUID string across all loaded collections of `T`.
    #[inline]
    pub fn find_row<T: 'static>(&self, row: impl IntoRowUuid<T>) -> Option<&T> {
        let row = row.into_row_uuid()?;
        self.stores
            .get(&TypeId::of::<T>())
            .and_then(|any| any.downcast_ref::<TypedDtoStore<T>>())
            .and_then(|store| store.find_row(row))
    }

    /// Resolves a [`RowUuid<T>`] across all loaded [`Collection<T>`]s in deterministic collection order.
    #[inline]
    pub fn resolve_row<T: 'static>(&self, id: RowUuid<T>) -> Option<&T> {
        self.find_row(id)
    }

    /// Whether all baked collections registered for this project have completed their initial load.
    pub fn is_ready(&self) -> bool {
        self.ready || (self.loaded_collections.len() + self.failed_collections.len() >= self.expected_collections)
    }

    /// Current loading progress across all baked `.collection.qsd` files.
    pub(crate) fn progress(&self, asset_server: Option<&AssetServer>) -> BakedDataProgress {
        let total = self.expected_collections as u32;
        if !self.started && self.loaded_collections.is_empty() && self.failed_collections.is_empty() {
            return BakedDataProgress { loaded: 0, failed: 0, total };
        }
        let mut failed_set: BTreeSet<u128> = self.failed_collections.clone();
        if let Some(server) = asset_server {
            for (&col_id, (_, handle)) in &self.handles {
                if !self.loaded_collections.contains(&col_id)
                    && matches!(server.get_load_state(handle.id()), Some(LoadState::Failed(_)))
                {
                    failed_set.insert(col_id);
                }
            }
        }
        let loaded = (self.loaded_collections.len() as u32).min(total);
        let failed = (failed_set.len() as u32).min(total.saturating_sub(loaded));
        BakedDataProgress { loaded, failed, total }
    }

    /// Registers a strong [`Handle<QsdBlobAsset>`] for `collection_id` so changes to `asset_path` trigger hot-reload.
    pub fn register_handle(
        &mut self,
        collection_id: u128,
        asset_path: impl Into<String>,
        handle: Handle<QsdBlobAsset>,
    ) {
        self.handles.insert(collection_id, (asset_path.into(), handle));
    }

    /// Validates the 36-byte `.qsd` header and `DATA_SCHEMA_HASH` of `bytes`, decodes the `postcard` payload into
    /// [`Collection<T>`], and updates the in-memory store.
    ///
    /// On header or schema-hash failure, logs `bevy::log::error!` and leaves the previous in-memory [`Collection<T>`]
    /// intact without panicking (D1, D21).
    pub fn apply_collection_bytes<T: DataDto>(
        &mut self,
        col_id: CollectionUuid<T>,
        asset_path: &str,
        bytes: &[u8],
    ) -> Result<(), QsdRuntimeError> {
        let (_header, payload) = match verify_qsd_header(bytes, DATA_SCHEMA_HASH, T::DTO_ID) {
            Ok(ok) => ok,
            Err(err) => {
                self.failed_collections.insert(col_id.as_u128());
                bevy::log::error!("quillshade::data: failed to load `{asset_path}`: {err}");
                return Err(err);
            }
        };
        let ctx = DataResolveCtx {
            asset_server: self.asset_server.as_ref(),
            material_by_id: Some(&self.material_by_id),
            texture_by_path: Some(&self.texture_by_path),
        };
        let collection = match T::decode_qsd_payload(payload, &ctx) {
            Ok(c) => c,
            Err(msg) => {
                self.failed_collections.insert(col_id.as_u128());
                let err = QsdRuntimeError::Decode(msg);
                bevy::log::error!("quillshade::data: failed to decode `{asset_path}`: {err}");
                return Err(err);
            }
        };
        let store = self
            .stores
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(TypedDtoStore::<T>::default()))
            .downcast_mut::<TypedDtoStore<T>>()
            .expect("TypedDtoStore type matches TypeId::of::<T>()");
        store.insert_collection(col_id, collection);
        self.failed_collections.remove(&col_id.as_u128());
        self.loaded_collections.insert(col_id.as_u128());
        Ok(())
    }

    /// Applies raw `.qsd` bytes for a registered `collection_id` by dispatching to its generated DTO type.
    pub fn apply_qsd_bytes_by_collection_id(
        &mut self,
        collection_id: u128,
        bytes: &[u8],
    ) -> Result<bool, QsdRuntimeError> {
        decode_registered_collection(self, collection_id, bytes)
    }
}

type DataStoreRes<'w> = Res<'w, DataStore>;

// SAFETY: `DataStoreRes` (`Res<DataStore>`) only reads `DataStore`.
unsafe impl bevy::ecs::system::ReadOnlySystemParam for Data<'_> {}

// SAFETY: All state initialization and world access is delegated to `Res<DataStore>`.
unsafe impl bevy::ecs::system::SystemParam for Data<'_> {
    type State = <DataStoreRes<'static> as bevy::ecs::system::SystemParam>::State;
    type Item<'w, 's> = Data<'w>;

    fn init_state(world: &mut bevy::ecs::world::World) -> Self::State {
        DataStoreRes::init_state(world)
    }

    fn init_access(
        state: &Self::State,
        system_meta: &mut bevy::ecs::system::SystemMeta,
        component_access_set: &mut bevy::ecs::system::SystemAccess,
        world: &mut bevy::ecs::world::World,
    ) {
        DataStoreRes::init_access(state, system_meta, component_access_set, world);
    }

    #[inline]
    unsafe fn get_param<'w, 's>(
        state: &'s mut Self::State,
        system_meta: &bevy::ecs::system::SystemMeta,
        world: bevy::ecs::world::unsafe_world_cell::UnsafeWorldCell<'w>,
        change_tick: bevy::ecs::change_detection::Tick,
    ) -> Result<Self::Item<'w, 's>, bevy::ecs::system::SystemParamValidationError> {
        // SAFETY: Delegated directly to `Res<DataStore>::get_param`.
        let store = unsafe { DataStoreRes::get_param(state, system_meta, world, change_tick) }?;
        Ok(Data::from_store(store.into_inner()))
    }
}

impl<'w> Data<'w> {
    /// Returns the [`Collection<T>`] for `id` (guaranteed to exist; empty before async `.qsd` load completes).
    #[inline]
    pub fn collection<T: 'static>(&self, id: CollectionUuid<T>) -> &'w Collection<T> {
        self.store.collection(id)
    }

    /// Returns the first row of collection `id`, if loaded (convenience for 1-row configuration collections).
    #[inline]
    pub fn first<T: 'static>(&self, id: CollectionUuid<T>) -> Option<&'w T> {
        self.store.first(id)
    }

    /// Looks up a single row inside collection `col` in $O(1)$ time by its [`RowUuid<T>`], `u128`, or UUID string.
    #[inline]
    pub fn get<T: 'static>(&self, col: CollectionUuid<T>, row: impl IntoRowUuid<T>) -> Option<&'w T> {
        self.store.get(col, row)
    }

    /// Alias for [`Self::get`]: looks up a single row inside collection `col` by its [`RowUuid<T>`], `u128`, or UUID string.
    #[inline]
    pub fn row<T: 'static>(&self, col: CollectionUuid<T>, row: impl IntoRowUuid<T>) -> Option<&'w T> {
        self.store.get(col, row)
    }

    /// Looks up a row by its [`RowUuid<T>`], `u128`, or UUID string across all loaded collections of `T`.
    #[inline]
    pub fn find_row<T: 'static>(&self, row: impl IntoRowUuid<T>) -> Option<&'w T> {
        self.store.find_row(row)
    }

    /// Resolves a [`RowUuid<T>`] across all loaded [`Collection<T>`]s in deterministic collection order.
    #[inline]
    pub fn resolve_row<T: 'static>(&self, id: RowUuid<T>) -> Option<&'w T> {
        self.store.resolve_row(id)
    }

    /// Whether all baked `.collection.qsd` assets have finished loading.
    #[inline]
    pub fn is_ready(&self) -> bool {
        self.store.is_ready()
    }

    /// Borrows the underlying [`DataStore`] resource.
    #[inline]
    pub fn store(&self) -> &'w DataStore {
        self.store
    }
}

/// Internal system parameter that starts or queries loading of baked `.collection.qsd` data tables.
///
/// Delegated to by [`crate::QuillshadeLoader`].
#[allow(dead_code)]
#[derive(SystemParam)]
pub(crate) struct BakedDataLoader<'w> {
    store: Option<ResMut<'w, DataStore>>,
    asset_server: Option<Res<'w, AssetServer>>,
}

#[allow(dead_code)]
impl BakedDataLoader<'_> {
    /// Starts loading all baked `.collection.qsd` files if not already started.
    ///
    /// Returns the total number of baked collections registered in this project.
    pub(crate) fn load(&mut self) -> u32 {
        let Some(store) = self.store.as_deref_mut() else {
            return 0;
        };
        start_data_load(store, self.asset_server.as_deref());
        store.progress(self.asset_server.as_deref()).total
    }

    /// Current `(loaded, failed, total)` progress across all baked `.collection.qsd` files.
    pub(crate) fn progress(&self) -> BakedDataProgress {
        self.store.as_ref().map_or(BakedDataProgress { loaded: 0, failed: 0, total: 0 }, |s| {
            s.progress(self.asset_server.as_deref())
        })
    }

    /// `true` once `load()` or `auto_load_data` has started loading the Data domain.
    pub(crate) fn is_started(&self) -> bool {
        self.store.as_ref().is_none_or(|s| s.started)
    }

    /// `true` once all baked `.collection.qsd` files have finished loading and decoding.
    pub(crate) fn is_ready(&self) -> bool {
        self.store.as_ref().is_none_or(|s| s.is_ready())
    }
}

/// Plugin registering [`QsdBlobAsset`], [`QsdBlobLoader`], [`DataStore`], startup loading, and `.qsd` hot-reload;
/// added by [`crate::QuillshadePlugin`] while the Data domain is enabled.
#[derive(Default)]
pub struct DataDomainPlugin;

impl Plugin for DataDomainPlugin {
    fn build(&self, app: &mut App) {
        if app.world().contains_resource::<AssetServer>() {
            if !app.world().contains_resource::<Assets<QsdBlobAsset>>() {
                app.init_asset::<QsdBlobAsset>();
            }
            if !app.world().contains_resource::<Assets<Image>>() {
                app.init_asset::<Image>();
            }
            app.register_asset_loader(QsdBlobLoader);
        } else {
            if !app.world().contains_resource::<Assets<QsdBlobAsset>>() {
                app.init_resource::<Assets<QsdBlobAsset>>();
            }
            if !app.world().contains_resource::<Messages<AssetEvent<QsdBlobAsset>>>() {
                app.add_message::<AssetEvent<QsdBlobAsset>>();
            }
        }
        app.add_message::<BakedDataProgress>()
            .add_message::<BakedDataReady>()
            .init_resource::<DataStore>()
            .add_systems(
                PreUpdate,
                (auto_load_data, sync_registered_materials, reload_data_collections, track_data_progress)
                    .chain()
                    .in_set(LoadSet),
            );
        if let Some(server) = app.world().get_resource::<AssetServer>().cloned() {
            app.world_mut().resource_mut::<DataStore>().asset_server = Some(server);
        }
    }
}

fn start_data_load(store: &mut DataStore, asset_server: Option<&AssetServer>) {
    if store.asset_server.is_none() {
        store.asset_server = asset_server.cloned();
    }
    if store.started {
        return;
    }
    store.started = true;
    let Some(server) = asset_server else {
        return;
    };
    for &(col_id, path) in BAKED_COLLECTIONS {
        let handle = server.load::<QsdBlobAsset>(path);
        store.register_handle(col_id, path, handle);
    }
}

fn auto_load_data(mut store: ResMut<DataStore>, asset_server: Option<Res<AssetServer>>) {
    start_data_load(&mut store, asset_server.as_deref());
}

fn track_data_progress(
    mut store: ResMut<DataStore>,
    asset_server: Option<Res<AssetServer>>,
    mut progress_out: MessageWriter<BakedDataProgress>,
    mut ready_out: MessageWriter<BakedDataReady>,
) {
    if !store.started || store.ready {
        return;
    }
    let progress = store.progress(asset_server.as_deref());
    if store.sent_progress != Some(progress) {
        store.sent_progress = Some(progress);
        progress_out.write(progress);
    }
    if progress.loaded + progress.failed >= progress.total {
        store.ready = true;
        ready_out.write(BakedDataReady);
    }
}

fn reload_data_collections(
    mut store: ResMut<DataStore>,
    blobs: Option<Res<Assets<QsdBlobAsset>>>,
    mut events: Option<MessageReader<AssetEvent<QsdBlobAsset>>>,
) {
    let (Some(blobs), Some(events)) = (blobs, &mut events) else {
        return;
    };
    let mut changed_ids = Vec::new();
    for event in events.read() {
        match event {
            AssetEvent::Added { id } | AssetEvent::Modified { id } | AssetEvent::LoadedWithDependencies { id } => {
                if !changed_ids.contains(id) {
                    changed_ids.push(*id);
                }
            }
            AssetEvent::Removed { .. } | AssetEvent::Unused { .. } => {}
        }
    }
    if changed_ids.is_empty() {
        return;
    }
    let matching_collections: Vec<(u128, Vec<u8>)> = store
        .handles
        .iter()
        .filter_map(|(&col_id, (_, handle))| {
            changed_ids
                .contains(&handle.id())
                .then(|| blobs.get(handle.id()).map(|blob| (col_id, blob.bytes.clone())))
                .flatten()
        })
        .collect();
    for (col_id, bytes) in matching_collections {
        let _ = store.apply_qsd_bytes_by_collection_id(col_id, &bytes);
    }
}
