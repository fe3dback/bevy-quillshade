//! Global constant tables (`quillshade/material/**/*.consts.ron`, decision 0017): named values read by the `Global*`
//! nodes and copied into the generated WESL as literals.

use std::collections::BTreeMap;

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
    /// Stored as a bool, read as `1.0` / `0.0` (`GlobalBool`).
    Bool,
    /// A real `bool`, for logic nodes only (`GlobalCompileFlag`).
    CompileFlag,
}

impl ConstType {
    pub const ALL: [ConstType; 6] =
        [ConstType::F32, ConstType::Vec2, ConstType::Vec3, ConstType::Vec4, ConstType::Bool, ConstType::CompileFlag];

    /// Label in the table's type select.
    pub fn label(self) -> &'static str {
        match self {
            ConstType::F32 => "f32",
            ConstType::Vec2 => "vec2",
            ConstType::Vec3 => "vec3",
            ConstType::Vec4 => "vec4",
            ConstType::Bool => "bool",
            ConstType::CompileFlag => "compileFlag",
        }
    }

    pub fn from_label(s: &str) -> Option<ConstType> {
        ConstType::ALL.into_iter().find(|t| t.label() == s)
    }

    /// Type of the stored value (`Bool` for both flags).
    pub fn value_type(self) -> PinType {
        match self {
            ConstType::F32 => PinType::Float,
            ConstType::Vec2 => PinType::Float2,
            ConstType::Vec3 => PinType::Float3,
            ConstType::Vec4 => PinType::Float4,
            ConstType::Bool | ConstType::CompileFlag => PinType::Bool,
        }
    }

    /// Output pin type of the node reading a constant of this type.
    pub fn pin_type(self) -> PinType {
        match self {
            ConstType::Bool => PinType::Float,
            t => t.value_type(),
        }
    }

    /// `value` in this type's shape.
    pub fn normalize(self, value: &Value) -> Value {
        value.coerce(self.value_type())
    }

    /// WESL literal the node emits for `value` (`GlobalBool` reads `1.0f` / `0.0f`).
    pub fn wesl_literal(self, value: &Value) -> String {
        let v = self.normalize(value);
        match self {
            ConstType::Bool => Value::Float(v.as_f32()).wesl_literal(PinType::Float),
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

#[derive(Deserialize)]
#[serde(rename = "GlobalConsts")]
struct VersionProbe {
    format_version: u32,
}

impl GlobalConsts {
    pub fn new(name: impl Into<String>, consts: Vec<GlobalConst>) -> Self {
        let mut c = GlobalConsts { format_version: CONSTS_FORMAT_VERSION, name: name.into(), consts };
        c.normalize();
        c
    }

    /// Parses a table file (older format versions would migrate here).
    pub fn from_ron(text: &str) -> Result<GlobalConsts, LoadError> {
        let probe: VersionProbe =
            ron::Options::default().from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
        match probe.format_version {
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
                GlobalConst::new("fancy", ConstType::CompileFlag, Value::Bool(true)),
            ],
        );
        assert_eq!(t.get("tint").unwrap().value, Value::Float3([1.0; 3]));
        let text = t.to_ron();
        assert!(text.starts_with("GlobalConsts("), "{text}");
        assert!(text.contains("ty: CompileFlag"), "{text}");
        assert_eq!(GlobalConsts::from_ron(&text).unwrap(), t);
        let too_new = "GlobalConsts(format_version: 9, name: \"x\")";
        assert!(matches!(GlobalConsts::from_ron(too_new), Err(LoadError::TooNew { .. })));
    }

    #[test]
    fn literals_and_pins() {
        assert_eq!(ConstType::Bool.wesl_literal(&Value::Bool(true)), "1.0f");
        assert_eq!(ConstType::Bool.wesl_literal(&Value::Bool(false)), "0.0f");
        assert_eq!(ConstType::CompileFlag.wesl_literal(&Value::Bool(true)), "true");
        assert_eq!(ConstType::F32.wesl_literal(&Value::Float(0.25)), "0.25f");
        assert_eq!(ConstType::Vec2.wesl_literal(&Value::Float2([1.0, 2.0])), "vec2<f32>(1.0, 2.0)");
        assert_eq!(ConstType::Bool.pin_type(), PinType::Float);
        assert_eq!(ConstType::CompileFlag.pin_type(), PinType::Bool);
        // A type change converts the value: vec3 → f32 keeps x, f32 → bool is "not zero".
        assert_eq!(ConstType::F32.normalize(&Value::Float3([0.3, 0.6, 0.9])), Value::Float(0.3));
        assert_eq!(ConstType::Bool.normalize(&Value::Float(0.3)), Value::Bool(true));
        assert_eq!(ConstType::from_label("compileFlag"), Some(ConstType::CompileFlag));
    }
}
