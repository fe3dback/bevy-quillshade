//! Global constant tables (`quillshade/material/**/*.consts.ron`, decision 0017): named values read by the `Global*`
//! nodes and copied into the generated WESL as literals.

use std::collections::hash_map::RandomState;
use std::collections::{BTreeMap, HashSet};
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use project_core::probe_version;
use serde::{Deserialize, Serialize};

use crate::model::LoadError;
use crate::types::PinType;
use crate::value::Value;

/// Current constant table file format version.
pub const CONSTS_FORMAT_VERSION: u32 = 1;

/// Type of a global constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub enum ConstType {
    #[default]
    F32,
    Vec2,
    Vec3,
    Vec4,
    /// Stored as a bool, read as `1.0` / `0.0` (`GlobalToggle`).
    #[serde(alias = "Bool")]
    Toggle,
    /// A runtime `bool` (`true` / `false`) emitted into WESL (`GlobalRuntimeBool`).
    RuntimeBool,
    /// An editor-only compile-time `bool` flag for `If` (`GlobalCompileBool`).
    #[serde(alias = "CompileFlag")]
    CompileBool,
}

impl ConstType {
    pub const ALL: [ConstType; 7] = [
        ConstType::F32,
        ConstType::Vec2,
        ConstType::Vec3,
        ConstType::Vec4,
        ConstType::Toggle,
        ConstType::RuntimeBool,
        ConstType::CompileBool,
    ];

    /// Label in the table's type select.
    pub fn label(self) -> &'static str {
        match self {
            ConstType::F32 => "f32",
            ConstType::Vec2 => "vec2",
            ConstType::Vec3 => "vec3",
            ConstType::Vec4 => "vec4",
            ConstType::Toggle => "toggle",
            ConstType::RuntimeBool => "runtimeBool",
            ConstType::CompileBool => "compileBool",
        }
    }

    pub fn from_label(s: &str) -> Option<ConstType> {
        ConstType::ALL.into_iter().find(|t| t.label() == s)
    }

    /// Type of the stored value (`RuntimeBool` for boolean constants).
    pub fn value_type(self) -> PinType {
        match self {
            ConstType::F32 => PinType::Float,
            ConstType::Vec2 => PinType::Float2,
            ConstType::Vec3 => PinType::Float3,
            ConstType::Vec4 => PinType::Float4,
            ConstType::Toggle | ConstType::RuntimeBool | ConstType::CompileBool => PinType::RuntimeBool,
        }
    }

    /// Alias for [`ConstType::value_type`]: underlying storage type of the constant.
    pub fn storage_type(self) -> PinType {
        self.value_type()
    }

    /// Output pin type of the node reading a constant of this type.
    pub fn pin_type(self) -> PinType {
        match self {
            ConstType::Toggle => PinType::Float,
            ConstType::RuntimeBool => PinType::RuntimeBool,
            ConstType::CompileBool => PinType::CompileBool,
            t => t.value_type(),
        }
    }

    /// `value` in this type's shape.
    pub fn normalize(self, value: &Value) -> Value {
        value.coerce(self.value_type())
    }

    /// WESL literal the node emits for `value` (`GlobalToggle` reads `1.0f` / `0.0f`).
    pub fn wesl_literal(self, value: &Value) -> String {
        let v = self.normalize(value);
        match self {
            ConstType::Toggle => Value::Float(v.as_f32()).wesl_literal(PinType::Float),
            t => v.wesl_literal(t.value_type()),
        }
    }
}

/// Whether `id` is a valid constant id (`1..=i32::MAX as u32`; `0` means "none").
pub fn valid_id(id: u32) -> bool {
    id != 0 && id <= i32::MAX as u32
}

/// Generates a random non-zero constant id (`1..=i32::MAX as u32`).
pub fn random_id() -> u32 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut state = RandomState::new().hash_one((SystemTime::now(), COUNTER.fetch_add(1, Ordering::Relaxed)));
    loop {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        let id = ((z ^ (z >> 31)) >> 33) as u32;
        if valid_id(id) {
            return id;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalConst {
    /// Stable random id (`1..=i32::MAX as u32`): `Global*` nodes refer to a constant by `(table, id)`, so renaming it
    /// in the table changes neither material RON nor generated WESL.
    pub id: u32,
    pub name: String,
    pub ty: ConstType,
    pub value: Value,
}

impl GlobalConst {
    pub fn new(name: impl Into<String>, ty: ConstType, value: Value) -> Self {
        Self::with_id(random_id(), name, ty, value)
    }

    pub fn with_id(id: u32, name: impl Into<String>, ty: ConstType, value: Value) -> Self {
        GlobalConst { id, name: name.into(), ty, value: ty.normalize(&value) }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalConsts {
    pub format_version: u32,
    pub name: String,
    #[serde(default)]
    pub consts: Vec<GlobalConst>,
}

impl GlobalConsts {
    pub fn new(name: impl Into<String>, consts: Vec<GlobalConst>) -> Self {
        let mut c = GlobalConsts { format_version: CONSTS_FORMAT_VERSION, name: name.into(), consts };
        c.normalize();
        c
    }

    /// Parses a table file (older format versions would migrate here), normalizes values, and checks constant ids.
    pub fn from_ron(text: &str) -> Result<GlobalConsts, LoadError> {
        match probe_version(text)? {
            v if v > CONSTS_FORMAT_VERSION => Err(LoadError::TooNew { found: v, supported: CONSTS_FORMAT_VERSION }),
            CONSTS_FORMAT_VERSION => {
                let mut c: GlobalConsts =
                    ron::Options::default().from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
                c.normalize();
                c.check().map_err(LoadError::Parse)?;
                Ok(c)
            }
            v => Err(LoadError::Unsupported(v)),
        }
    }

    pub fn to_ron(&self) -> String {
        let mut c = self.clone();
        c.normalize();
        let cfg =
            ron::ser::PrettyConfig::new().struct_names(true).indentor("  ".to_string()).new_line("\n".to_string());
        let mut s = ron::ser::to_string_pretty(&c, cfg).expect("consts serialization");
        s.push('\n');
        s
    }

    /// Brings every value into its type's shape.
    pub fn normalize(&mut self) {
        for c in &mut self.consts {
            c.value = c.ty.normalize(&c.value);
        }
    }

    /// Structural rules a loaded file must satisfy.
    fn check(&self) -> Result<(), String> {
        let mut seen = HashSet::with_capacity(self.consts.len());
        for (pos, c) in self.consts.iter().enumerate() {
            if !valid_id(c.id) {
                return Err(format!("const {pos}: id {} must be in 1..={}", c.id, i32::MAX));
            }
            if !seen.insert(c.id) {
                return Err(format!("const {pos}: duplicate id {}", c.id));
            }
        }
        Ok(())
    }

    /// Generates a random constant id not yet used in this table.
    pub fn next_id(&self) -> u32 {
        loop {
            let id = random_id();
            if self.get_by_id(id).is_none() {
                return id;
            }
        }
    }

    pub fn get_by_id(&self, id: u32) -> Option<&GlobalConst> {
        self.consts.iter().find(|c| c.id == id)
    }

    pub fn get_by_id_mut(&mut self, id: u32) -> Option<&mut GlobalConst> {
        self.consts.iter_mut().find(|c| c.id == id)
    }

    pub fn get(&self, name: &str) -> Option<&GlobalConst> {
        self.consts.iter().find(|c| c.name == name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut GlobalConst> {
        self.consts.iter_mut().find(|c| c.name == name)
    }
}

/// Global constant tables of the project that nodes can read.
pub trait ConstLookup {
    fn const_table(&self, path: &str) -> Option<&GlobalConsts>;
}

/// No project constants (tests, signatures).
pub struct NoConsts;

impl ConstLookup for NoConsts {
    fn const_table(&self, _path: &str) -> Option<&GlobalConsts> {
        None
    }
}

impl ConstLookup for BTreeMap<String, GlobalConsts> {
    fn const_table(&self, path: &str) -> Option<&GlobalConsts> {
        self.get(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_normalize() {
        let t = GlobalConsts::new(
            "globals",
            vec![
                GlobalConst::new("wind", ConstType::F32, Value::Float(0.5)),
                GlobalConst { id: random_id(), name: "tint".into(), ty: ConstType::Vec3, value: Value::Float(1.0) },
                GlobalConst::new("fancy", ConstType::CompileBool, Value::Bool(true)),
            ],
        );
        assert!(t.consts.iter().all(|c| valid_id(c.id)));
        assert_eq!(t.get("tint").unwrap().value, Value::Float3([1.0; 3]));
        let text = t.to_ron();
        assert!(text.starts_with("GlobalConsts("), "{text}");
        assert!(text.contains("ty: CompileBool"), "{text}");
        assert_eq!(GlobalConsts::from_ron(&text).unwrap(), t);
        let too_new = "GlobalConsts(format_version: 9, name: \"x\")";
        assert!(matches!(GlobalConsts::from_ron(too_new), Err(LoadError::TooNew { .. })));
    }

    #[test]
    fn invalid_or_duplicate_ids_are_rejected() {
        let zero_id = "GlobalConsts(format_version: 1, name: \"g\", consts: [GlobalConst(id: 0, name: \"x\", ty: F32, value: Float(1.0))])";
        assert!(matches!(GlobalConsts::from_ron(zero_id), Err(LoadError::Parse(msg)) if msg.contains("must be in")));
        let dup_id = "GlobalConsts(format_version: 1, name: \"g\", consts: [GlobalConst(id: 7, name: \"x\", ty: F32, value: Float(1.0)), GlobalConst(id: 7, name: \"y\", ty: F32, value: Float(2.0))])";
        assert!(matches!(GlobalConsts::from_ron(dup_id), Err(LoadError::Parse(msg)) if msg.contains("duplicate id 7")));
    }

    #[test]
    fn literals_and_pins() {
        assert_eq!(ConstType::Toggle.wesl_literal(&Value::Bool(true)), "1.0f");
        assert_eq!(ConstType::Toggle.wesl_literal(&Value::Bool(false)), "0.0f");
        assert_eq!(ConstType::RuntimeBool.wesl_literal(&Value::Bool(true)), "true");
        assert_eq!(ConstType::RuntimeBool.wesl_literal(&Value::Bool(false)), "false");
        assert_eq!(ConstType::CompileBool.wesl_literal(&Value::Bool(true)), "true");
        assert_eq!(ConstType::CompileBool.wesl_literal(&Value::Bool(false)), "false");
        assert_eq!(ConstType::F32.wesl_literal(&Value::Float(0.25)), "0.25f");
        assert_eq!(ConstType::Vec2.wesl_literal(&Value::Float2([1.0, 2.0])), "vec2<f32>(1.0, 2.0)");
        assert_eq!(ConstType::Toggle.pin_type(), PinType::Float);
        assert_eq!(ConstType::RuntimeBool.pin_type(), PinType::RuntimeBool);
        assert_eq!(ConstType::CompileBool.pin_type(), PinType::CompileBool);
        assert_eq!(ConstType::Toggle.storage_type(), PinType::RuntimeBool);
        assert_eq!(ConstType::RuntimeBool.storage_type(), PinType::RuntimeBool);
        assert_eq!(ConstType::CompileBool.storage_type(), PinType::RuntimeBool);
        // A type change converts the value: vec3 → f32 keeps x, f32 → bool is "not zero".
        assert_eq!(ConstType::F32.normalize(&Value::Float3([0.3, 0.6, 0.9])), Value::Float(0.3));
        assert_eq!(ConstType::Toggle.normalize(&Value::Float(0.3)), Value::Bool(true));
        assert_eq!(ConstType::from_label("toggle"), Some(ConstType::Toggle));
        assert_eq!(ConstType::from_label("runtimeBool"), Some(ConstType::RuntimeBool));
        assert_eq!(ConstType::from_label("compileBool"), Some(ConstType::CompileBool));
    }
}
