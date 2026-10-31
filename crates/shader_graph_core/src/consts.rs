//! Global constant tables (`quillshade/material/**/*.consts.ron`, decision 0017): named values read by the `Global*`
//! nodes and copied into the generated WESL as literals.

use std::collections::BTreeMap;

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalConst {
    pub name: String,
    pub ty: ConstType,
    pub value: Value,
}

impl GlobalConst {
    pub fn new(name: impl Into<String>, ty: ConstType, value: Value) -> Self {
        GlobalConst { name: name.into(), ty, value: ty.normalize(&value) }
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

    /// Parses a table file (older format versions would migrate here).
    pub fn from_ron(text: &str) -> Result<GlobalConsts, LoadError> {
        match probe_version(text)? {
            v if v > CONSTS_FORMAT_VERSION => Err(LoadError::TooNew { found: v, supported: CONSTS_FORMAT_VERSION }),
            CONSTS_FORMAT_VERSION => {
                let mut c: GlobalConsts =
                    ron::Options::default().from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
                c.normalize();
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

    pub fn get(&self, name: &str) -> Option<&GlobalConst> {
        self.consts.iter().find(|c| c.name == name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut GlobalConst> {
        self.consts.iter_mut().find(|c| c.name == name)
    }
}

/// Global constants of the project that nodes can read (valid and unique names).
pub trait ConstLookup {
    fn global_const(&self, name: &str) -> Option<&GlobalConst>;
}

/// No project constants (tests, signatures).
pub struct NoConsts;

impl ConstLookup for NoConsts {
    fn global_const(&self, _name: &str) -> Option<&GlobalConst> {
        None
    }
}

impl ConstLookup for BTreeMap<String, GlobalConst> {
    fn global_const(&self, name: &str) -> Option<&GlobalConst> {
        self.get(name)
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
                GlobalConst { name: "tint".into(), ty: ConstType::Vec3, value: Value::Float(1.0) },
                GlobalConst::new("fancy", ConstType::CompileBool, Value::Bool(true)),
            ],
        );
        assert_eq!(t.get("tint").unwrap().value, Value::Float3([1.0; 3]));
        let text = t.to_ron();
        assert!(text.starts_with("GlobalConsts("), "{text}");
        assert!(text.contains("ty: CompileBool"), "{text}");
        assert_eq!(GlobalConsts::from_ron(&text).unwrap(), t);
        let too_new = "GlobalConsts(format_version: 9, name: \"x\")";
        assert!(matches!(GlobalConsts::from_ron(too_new), Err(LoadError::TooNew { .. })));
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
