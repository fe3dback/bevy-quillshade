//! Asset identity by UUIDv7 (ADR 0047 §1, techdoc §4.1, D4): every editor-owned source file carries a top-level
//! `id: "<uuid>"`, and references between assets store the target's UUID instead of its path.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

/// Unix time in milliseconds of every [`AssetUuid::legacy`] id: 2026-01-01T00:00:00Z.
pub const LEGACY_MILLIS: u64 = 1_767_225_600_000;

/// Domain separator hashed in front of the `source_rel` of a [`AssetUuid::legacy`] id.
const LEGACY_SALT: &[u8] = b"quillshade:v3\0";

/// The UUID of an editor-owned asset. Written as the hyphenated lowercase string; [`AssetUuid::NIL`] (the default)
/// means "no id yet": model constructors leave it nil and writers mint one on the first save.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct AssetUuid(Uuid);

impl AssetUuid {
    pub const NIL: AssetUuid = AssetUuid(Uuid::nil());

    /// A fresh random UUIDv7 (`Uuid::now_v7`): what an asset created in the editor gets.
    pub fn new_v7() -> AssetUuid {
        AssetUuid(Uuid::now_v7())
    }

    /// The deterministic id the v3 migration gives an asset without one (ADR 0047 §1): a well-formed UUIDv7 with the
    /// timestamp [`LEGACY_MILLIS`] and 80 bits of FNV-1a-64 over `"quillshade:v3\0" + source_rel` — the 8 bytes of
    /// `h1 = fnv(input)` then the first 2 little-endian bytes of `h2 = fnv(h1 as 8 little-endian bytes + input)`
    /// (the version and variant bits overwrite 6 of them). Reruns of an interrupted migration, its dry run, and the
    /// fixtures therefore agree on every id.
    pub fn legacy(source_rel: &str) -> AssetUuid {
        let h1 = fnv1a64(FNV_OFFSET, &[LEGACY_SALT, source_rel.as_bytes()]);
        let h2 = fnv1a64(FNV_OFFSET, &[&h1.to_le_bytes(), LEGACY_SALT, source_rel.as_bytes()]);
        let mut bytes = [0u8; 10];
        bytes[..8].copy_from_slice(&h1.to_le_bytes());
        bytes[8..].copy_from_slice(&h2.to_le_bytes()[..2]);
        AssetUuid(uuid::Builder::from_unix_timestamp_millis(LEGACY_MILLIS, &bytes).into_uuid())
    }

    pub const fn from_u128(v: u128) -> AssetUuid {
        AssetUuid(Uuid::from_u128(v))
    }

    /// The hyphenated 8-4-4-4-12 form only (see [`is_uuid_shaped`]); any case is accepted, lowercase is written.
    pub fn parse(s: &str) -> Option<AssetUuid> {
        is_uuid_shaped(s).then(|| Uuid::try_parse(s).ok().map(AssetUuid)).flatten()
    }

    pub const fn is_nil(&self) -> bool {
        self.0.is_nil()
    }

    /// Whether this is a valid asset id: a version 7 UUID of the RFC 9562 variant.
    pub fn is_v7(&self) -> bool {
        self.0.get_version_num() == 7 && self.0.get_variant() == uuid::Variant::RFC4122
    }

    pub const fn as_u128(&self) -> u128 {
        self.0.as_u128()
    }

    /// Mints a [`AssetUuid::new_v7`] when this is not a valid asset id — nil, or a UUID of another version (both
    /// `AE004`) — before a writer saves a model (ADR 0047 §6); returns the id.
    pub fn ensure(&mut self) -> AssetUuid {
        if !self.is_v7() {
            *self = AssetUuid::new_v7();
        }
        *self
    }
}

impl fmt::Display for AssetUuid {
    /// Hyphenated lowercase.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0.hyphenated(), f)
    }
}

impl Serialize for AssetUuid {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for AssetUuid {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        AssetUuid::parse(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("`{s}` is not a UUID (8-4-4-4-12 hex digits)")))
    }
}

/// Whether `s` has the hyphenated UUID shape: 36 characters, hex digits in groups of 8-4-4-4-12 separated by `-`.
/// Such a string is never a valid asset path, because `ident::check_path` forbids `-`.
pub fn is_uuid_shaped(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, &c)| match i {
            8 | 13 | 18 | 23 => c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a 64-bit over the concatenation of `parts`: stable across Rust versions, unlike `DefaultHasher`.
fn fnv1a64(mut h: u64, parts: &[&[u8]]) -> u64 {
    for &b in parts.iter().flat_map(|p| p.iter()) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::ident::check_path;

    #[test]
    fn v7_is_v7_and_unique() {
        let ids: BTreeSet<AssetUuid> = (0..256).map(|_| AssetUuid::new_v7()).collect();
        assert_eq!(ids.len(), 256);
        assert!(ids.iter().all(AssetUuid::is_v7));
        assert!(!AssetUuid::NIL.is_v7() && AssetUuid::NIL.is_nil() && AssetUuid::default() == AssetUuid::NIL);
        // A version 4 UUID is no asset id.
        assert!(!AssetUuid::parse("6ba7b810-9dad-41d1-80b4-00c04fd430c8").unwrap().is_v7());
        let mut id = AssetUuid::NIL;
        let minted = id.ensure();
        assert!(minted.is_v7() && id == minted && id.ensure() == minted, "ensure keeps an existing id");
    }

    #[test]
    fn parse_round_trip_and_case() {
        let id = AssetUuid::new_v7();
        let text = id.to_string();
        assert!(is_uuid_shaped(&text) && text == text.to_lowercase());
        assert_eq!(AssetUuid::parse(&text), Some(id));
        assert_eq!(AssetUuid::parse(&text.to_uppercase()), Some(id), "any case is accepted");
        assert_eq!(AssetUuid::from_u128(id.as_u128()), id);
        // Only the hyphenated form.
        for bad in
            ["", "not-a-uuid", &text[1..], &text.replace('-', ""), &format!("{{{text}}}"), &format!("urn:uuid:{text}")]
        {
            assert_eq!(AssetUuid::parse(bad), None, "{bad}");
        }
        // Serde: the lowercase string, strictly parsed.
        let ron_text = ron::to_string(&id).unwrap();
        assert_eq!(ron_text, format!("\"{text}\""));
        assert_eq!(ron::from_str::<AssetUuid>(&ron_text.to_uppercase()).unwrap(), id);
        let err = ron::from_str::<AssetUuid>("\"common/pulse\"").unwrap_err().to_string();
        assert!(err.contains("`common/pulse` is not a UUID"), "{err}");
    }

    #[test]
    fn ensure_keeps_a_v7_and_replaces_nil_or_another_version() {
        let v7 = AssetUuid::legacy("quillshade/material/a.curve.ron");
        let mut kept = v7;
        assert_eq!(kept.ensure(), v7);
        for mut bad in [AssetUuid::NIL, AssetUuid::parse("6f1c2a3e-1111-4222-8333-444455556666").unwrap()] {
            let new = bad.ensure();
            assert!(new.is_v7() && bad == new);
        }
    }

    #[test]
    fn legacy_is_v7_and_stable() {
        let a = AssetUuid::legacy("quillshade/material/common/pulse.curve.ron");
        assert!(a.is_v7());
        // Pinned: fixtures, snapshots and migrated samples depend on this exact value.
        assert_eq!(a.to_string(), "019b76da-a800-783e-be72-971715772890");
        assert_eq!(a, AssetUuid::legacy("quillshade/material/common/pulse.curve.ron"));
        assert_ne!(a, AssetUuid::legacy("quillshade/light/common/pulse.curve.ron"));
        // The timestamp is the fixed constant (the top 48 bits).
        assert_eq!((a.as_u128() >> 80) as u64, LEGACY_MILLIS);
    }

    #[test]
    fn uuid_shaped_is_never_a_path() {
        let mut ids: Vec<String> = (0..64).map(|_| AssetUuid::new_v7().to_string()).collect();
        ids.extend([AssetUuid::NIL.to_string(), "abcdefab-cdef-7abc-8def-abcdefabcdef".into()]);
        ids.push(AssetUuid::legacy("quillshade/light/sky.texture.ron").to_string());
        for id in &ids {
            assert!(is_uuid_shaped(id), "{id}");
            assert!(check_path(id).is_err(), "{id}");
        }
        assert!(!is_uuid_shaped("textures/rock_albedo") && !is_uuid_shaped("@internal/default"));
    }
}
