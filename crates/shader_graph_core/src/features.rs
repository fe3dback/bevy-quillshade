//! Game feature static enum tables (`quillshade/material/**/*.features.ron`): static enums whose variants are
//! switched in graphs by `stdlib::GameFeatureSwitch` and compiled into specialized shader permutations.

use std::collections::{BTreeMap, HashSet};

use project_core::{AssetUuid, check_version, probe_version};
use serde::{Deserialize, Serialize};

use crate::model::LoadError;

/// Current feature table file format version: 2 carries the asset `id` (ADR 0047 §2).
pub const FEATURES_FORMAT_VERSION: u32 = 2;

/// Hard cap on the number of shader permutations (`N_1 * N_2 * ... * N_M`) a single material may compile (`E015`).
pub const MAX_MATERIAL_PERMUTATIONS: usize = 64;

/// Prefix for input pin IDs on `stdlib::GameFeatureSwitch` nodes (`v_<variant_uuid>`), or a bare UUID string.
pub fn variant_pin_id(variant_id: AssetUuid) -> String {
    variant_id.to_string()
}

/// Parses a variant input pin ID back into its [`AssetUuid`].
pub fn parse_variant_pin_id(pin_id: &str) -> Option<AssetUuid> {
    let raw = pin_id.strip_prefix("v_").unwrap_or(pin_id);
    AssetUuid::parse(raw)
}

/// One variant of a [`FeatureEnum`]. The first variant (`variants[0]`) is the default variant (`#[default]` in Rust).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureVariant {
    /// Stable UUIDv7 of this variant: `stdlib::GameFeatureSwitch` input pins are keyed by this UUID so renaming a
    /// variant in the table never breaks connected wires in material or function graphs.
    pub id: AssetUuid,
    pub name: String,
}

impl FeatureVariant {
    pub fn new(name: impl Into<String>) -> Self {
        Self::with_id(AssetUuid::new_v7(), name)
    }

    pub fn with_id(id: AssetUuid, name: impl Into<String>) -> Self {
        FeatureVariant { id, name: name.into() }
    }
}

/// One static enum entry in a [`FeatureTable`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureEnum {
    /// Stable UUIDv7 of this enum: `stdlib::GameFeatureSwitch` nodes refer to an enum by `(table, id)`, so renaming
    /// the enum in the table never breaks graph nodes.
    pub id: AssetUuid,
    pub name: String,
    #[serde(default)]
    pub variants: Vec<FeatureVariant>,
}

impl FeatureEnum {
    pub fn new(name: impl Into<String>, variants: Vec<FeatureVariant>) -> Self {
        Self::with_id(AssetUuid::new_v7(), name, variants)
    }

    pub fn with_id(id: AssetUuid, name: impl Into<String>, variants: Vec<FeatureVariant>) -> Self {
        FeatureEnum { id, name: name.into(), variants }
    }

    pub fn get_variant_by_id(&self, id: AssetUuid) -> Option<&FeatureVariant> {
        self.variants.iter().find(|v| v.id == id)
    }

    pub fn get_variant_by_id_mut(&mut self, id: AssetUuid) -> Option<&mut FeatureVariant> {
        self.variants.iter_mut().find(|v| v.id == id)
    }

    pub fn get_variant(&self, name: &str) -> Option<&FeatureVariant> {
        self.variants.iter().find(|v| v.name == name)
    }

    pub fn variant_index(&self, id: AssetUuid) -> Option<usize> {
        self.variants.iter().position(|v| v.id == id)
    }

    /// WESL `@if` shader_def identifier for variant index `variant_idx` (`>= 1`).
    pub fn shader_def(&self, variant_idx: usize) -> String {
        format!("QS_F_{:032x}_{variant_idx}", self.id.as_u128())
    }

    /// WESL `@if` boolean condition expression testing that this enum is at `variant_idx`.
    pub fn wesl_cond(&self, variant_idx: usize) -> String {
        if variant_idx > 0 {
            self.shader_def(variant_idx)
        } else if self.variants.len() <= 1 {
            "true".to_string()
        } else {
            let negs: Vec<String> = (1..self.variants.len()).map(|i| format!("!{}", self.shader_def(i))).collect();
            if negs.len() == 1 { negs.into_iter().next().unwrap() } else { format!("({})", negs.join(" && ")) }
        }
    }
}

/// A `.features.ron` asset containing static feature enums.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureTable {
    pub format_version: u32,
    /// The asset's UUID (ADR 0047 §2); nil (omitted from the file) until the asset is first saved.
    #[serde(default, skip_serializing_if = "AssetUuid::is_nil")]
    pub id: AssetUuid,
    pub name: String,
    #[serde(default)]
    pub enums: Vec<FeatureEnum>,
}

impl FeatureTable {
    pub fn new(name: impl Into<String>, enums: Vec<FeatureEnum>) -> Self {
        FeatureTable { format_version: FEATURES_FORMAT_VERSION, id: AssetUuid::NIL, name: name.into(), enums }
    }

    /// Parses a feature table file of the current format ([`FEATURES_FORMAT_VERSION`]) and checks enum/variant UUIDs.
    pub fn from_ron(text: &str) -> Result<FeatureTable, LoadError> {
        check_version(probe_version(text)?, FEATURES_FORMAT_VERSION)?;
        let t: FeatureTable = ron::Options::default().from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
        t.check().map_err(LoadError::Parse)?;
        Ok(t)
    }

    /// Mints the asset's UUID (and any nil enum/variant UUIDs) when missing; returns the asset's UUID.
    pub fn ensure_id(&mut self) -> AssetUuid {
        for e in &mut self.enums {
            let _ = e.id.ensure();
            for v in &mut e.variants {
                let _ = v.id.ensure();
            }
        }
        self.id.ensure()
    }

    /// Mints fresh UUIDv7s for every enum and variant in the table (used when regenerating the UUID of a copied
    /// `.features.ron` file so its sub-items do not collide with the original table's).
    pub fn regenerate_sub_ids(&mut self) {
        for e in &mut self.enums {
            e.id = AssetUuid::new_v7();
            for v in &mut e.variants {
                v.id = AssetUuid::new_v7();
            }
        }
    }

    pub fn to_ron(&self) -> String {
        let cfg =
            ron::ser::PrettyConfig::new().struct_names(true).indentor("  ".to_string()).new_line("\n".to_string());
        let mut s = ron::ser::to_string_pretty(self, cfg).expect("features serialization");
        s.push('\n');
        s
    }

    /// Structural rules a loaded file must satisfy.
    fn check(&self) -> Result<(), String> {
        let mut seen_ids = HashSet::new();
        for (epos, e) in self.enums.iter().enumerate() {
            if !e.id.is_v7() {
                return Err(format!("enum {epos}: id {} must be a valid UUIDv7", e.id));
            }
            if !seen_ids.insert(e.id) {
                return Err(format!("enum {epos}: duplicate id {}", e.id));
            }
            for (vpos, v) in e.variants.iter().enumerate() {
                if !v.id.is_v7() {
                    return Err(format!("enum {epos} variant {vpos}: id {} must be a valid UUIDv7", v.id));
                }
                if !seen_ids.insert(v.id) {
                    return Err(format!("enum {epos} variant {vpos}: duplicate id {}", v.id));
                }
            }
        }
        Ok(())
    }

    pub fn get_by_id(&self, id: AssetUuid) -> Option<&FeatureEnum> {
        self.enums.iter().find(|e| e.id == id)
    }

    pub fn get_by_id_mut(&mut self, id: AssetUuid) -> Option<&mut FeatureEnum> {
        self.enums.iter_mut().find(|e| e.id == id)
    }

    pub fn get(&self, name: &str) -> Option<&FeatureEnum> {
        self.enums.iter().find(|e| e.name == name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut FeatureEnum> {
        self.enums.iter_mut().find(|e| e.name == name)
    }

    /// Looks up an enum either by its `AssetUuid` string or by its `name`.
    pub fn resolve_enum(&self, key: &str) -> Option<&FeatureEnum> {
        if let Some(id) = AssetUuid::parse(key) {
            self.get_by_id(id)
        } else if !key.is_empty() {
            self.get(key)
        } else {
            None
        }
    }
}

/// Feature enum tables of the project that `stdlib::GameFeatureSwitch` nodes can read.
pub trait FeatureLookup {
    fn feature_table(&self, path: &str) -> Option<&FeatureTable>;
}

/// No project feature tables (tests, standalone signatures).
pub struct NoFeatures;

impl FeatureLookup for NoFeatures {
    fn feature_table(&self, _path: &str) -> Option<&FeatureTable> {
        None
    }
}

impl FeatureLookup for BTreeMap<String, FeatureTable> {
    fn feature_table(&self, path: &str) -> Option<&FeatureTable> {
        self.get(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_lookup() {
        let low = FeatureVariant::new("low");
        let high = FeatureVariant::new("high");
        let rq = FeatureEnum::new("render_quality", vec![low.clone(), high.clone()]);
        let mut t = FeatureTable::new("graphics", vec![rq.clone()]);
        assert!(!t.to_ron().contains("\n  id: "), "a nil id is not written");
        let id = t.ensure_id();
        assert!(id.is_v7());
        let text = t.to_ron();
        assert!(text.starts_with(&format!("FeatureTable(\n  format_version: 2,\n  id: \"{id}\",\n")), "{text}");
        let parsed = FeatureTable::from_ron(&text).unwrap();
        assert_eq!(parsed, t);
        assert_eq!(parsed.resolve_enum(&rq.id.to_string()), Some(&rq));
        assert_eq!(parsed.resolve_enum("render_quality"), Some(&rq));
        assert_eq!(rq.get_variant_by_id(low.id), Some(&low));
        assert_eq!(rq.variant_index(high.id), Some(1));
    }

    #[test]
    fn duplicate_or_invalid_uuids_are_rejected() {
        let v = FeatureVariant::new("a");
        let dup = FeatureTable::new("g", vec![FeatureEnum::new("x", vec![v.clone(), v])]);
        let text = dup.to_ron();
        assert!(matches!(FeatureTable::from_ron(&text), Err(LoadError::Parse(msg)) if msg.contains("duplicate id")));
    }
}
