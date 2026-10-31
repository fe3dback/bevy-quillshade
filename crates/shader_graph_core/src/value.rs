//! Literal values stored in node parameters and input defaults.

use serde::{Deserialize, Serialize};

use crate::types::PinType;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Float(f32),
    Float2([f32; 2]),
    Float3([f32; 3]),
    Float4([f32; 4]),
    Color([f32; 4]),
    Bool(bool),
    Int(i32),
    Str(String),
    Type(PinType),
}

impl Value {
    /// Zero value of a pin type.
    pub fn zero(ty: PinType) -> Value {
        match ty {
            PinType::Float | PinType::Any => Value::Float(0.0),
            PinType::Float2 => Value::Float2([0.0; 2]),
            PinType::Float3 => Value::Float3([0.0; 3]),
            PinType::Float4 => Value::Float4([0.0; 4]),
            PinType::Color => Value::Color([0.0, 0.0, 0.0, 1.0]),
            PinType::RuntimeBool | PinType::CompileBool => Value::Bool(false),
            PinType::Int => Value::Int(0),
            PinType::Texture2D | PinType::MaterialAttributesPbr => Value::Str(String::new()),
        }
    }

    pub fn default_for(ty: PinType) -> Value {
        Self::zero(ty)
    }

    pub fn pin_type(&self) -> Option<PinType> {
        Some(match self {
            Value::Float(_) => PinType::Float,
            Value::Float2(_) => PinType::Float2,
            Value::Float3(_) => PinType::Float3,
            Value::Float4(_) => PinType::Float4,
            Value::Color(_) => PinType::Color,
            Value::Bool(_) => PinType::RuntimeBool,
            Value::Int(_) => PinType::Int,
            Value::Str(_) | Value::Type(_) => return None,
        })
    }

    /// Float components (scalars give one component).
    pub fn floats(&self) -> Vec<f32> {
        match self {
            Value::Float(v) => vec![*v],
            Value::Float2(v) => v.to_vec(),
            Value::Float3(v) => v.to_vec(),
            Value::Float4(v) | Value::Color(v) => v.to_vec(),
            Value::Int(v) => vec![*v as f32],
            Value::Bool(b) => vec![if *b { 1.0 } else { 0.0 }],
            Value::Str(_) | Value::Type(_) => vec![0.0],
        }
    }

    pub fn as_f32(&self) -> f32 {
        self.floats()[0]
    }

    pub fn as_bool(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            other => other.as_f32() != 0.0,
        }
    }

    pub fn as_i32(&self) -> i32 {
        match self {
            Value::Int(v) => *v,
            other => other.as_f32() as i32,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Value::Str(s) => s,
            _ => "",
        }
    }

    pub fn as_type(&self) -> Option<PinType> {
        match self {
            Value::Type(t) => Some(*t),
            _ => None,
        }
    }

    /// Converts the value to the shape of `ty` (broadcasting / truncating components).
    pub fn coerce(&self, ty: PinType) -> Value {
        if let (Value::Str(_), PinType::Texture2D | PinType::MaterialAttributesPbr) = (self, ty) {
            return self.clone();
        }
        let f = self.floats();
        let at = |i: usize| -> f32 { if f.len() == 1 { f[0] } else { f.get(i).copied().unwrap_or(0.0) } };
        match ty {
            PinType::Float | PinType::Any => Value::Float(at(0)),
            PinType::Float2 => Value::Float2([at(0), at(1)]),
            PinType::Float3 => Value::Float3([at(0), at(1), at(2)]),
            PinType::Float4 => Value::Float4([at(0), at(1), at(2), at(3)]),
            PinType::Color => {
                let a = if f.len() == 4 { f[3] } else { 1.0 };
                Value::Color([at(0), at(1), at(2), a])
            }
            PinType::RuntimeBool | PinType::CompileBool => Value::Bool(self.as_bool()),
            PinType::Int => Value::Int(self.as_i32()),
            PinType::Texture2D | PinType::MaterialAttributesPbr => Value::Str(String::new()),
        }
    }

    /// WESL literal of this value interpreted as `ty`.
    pub fn wesl_literal(&self, ty: PinType) -> String {
        // The struct has no literal of its own: every value means "all fields at their defaults" (decision 0023).
        if ty == PinType::MaterialAttributesPbr {
            return crate::attributes::pbr_default_literal();
        }
        let v = self.coerce(ty);
        let list = |xs: &[f32]| xs.iter().map(|x| wesl_f32(*x)).collect::<Vec<_>>().join(", ");
        match v {
            Value::Float(x) => wesl_scalar(x),
            Value::Float2(x) => format!("vec2<f32>({})", list(&x)),
            Value::Float3(x) => format!("vec3<f32>({})", list(&x)),
            Value::Float4(x) | Value::Color(x) => format!("vec4<f32>({})", list(&x)),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => format!("{i}i"),
            Value::Str(_) | Value::Type(_) => "0.0".into(),
        }
    }
}

/// Formats an `f32` as a WGSL float literal (always with a decimal point or exponent).
pub fn wesl_f32(x: f32) -> String {
    if !x.is_finite() {
        return "0.0".into();
    }
    let s = format!("{x:?}");
    if s.contains('.') || s.contains('e') || s.contains("inf") || s.contains("NaN") { s } else { format!("{s}.0") }
}

/// Scalar `f32` WGSL literal with an explicit type suffix (`0.5f`).
///
/// Plain `0.5` is an abstract float: naga keeps it abstract inside builtins whose arguments are
/// all literals (`mix(0.0, 1.0, 0.5)`) and then rejects the entry point.
pub fn wesl_scalar(x: f32) -> String {
    format!("{}f", wesl_f32(x))
}

/// Formats an `f32` as a Rust float literal (the WGSL spelling is valid Rust too).
pub fn rust_f32(x: f32) -> String {
    wesl_f32(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals() {
        assert_eq!(wesl_f32(1.0), "1.0");
        assert_eq!(wesl_f32(0.25), "0.25");
        assert_eq!(wesl_f32(f32::NAN), "0.0");
        assert_eq!(Value::Float(2.0).wesl_literal(PinType::Float3), "vec3<f32>(2.0, 2.0, 2.0)");
        assert_eq!(Value::Float(0.5).wesl_literal(PinType::Float), "0.5f");
        assert_eq!(Value::Float3([1.0, 0.5, 0.0]).wesl_literal(PinType::Color), "vec4<f32>(1.0, 0.5, 0.0, 1.0)");
        assert_eq!(Value::Int(3).wesl_literal(PinType::Int), "3i");
        assert_eq!(Value::Bool(true).pin_type(), Some(PinType::RuntimeBool));
        assert_eq!(Value::Bool(true).wesl_literal(PinType::RuntimeBool), "true");
        assert_eq!(Value::Bool(false).wesl_literal(PinType::CompileBool), "false");
        assert_eq!(Value::zero(PinType::RuntimeBool), Value::Bool(false));
        assert_eq!(Value::zero(PinType::CompileBool), Value::Bool(false));
        let attrs = Value::zero(PinType::MaterialAttributesPbr);
        assert_eq!(attrs, Value::Str(String::new()));
        assert_eq!(attrs.coerce(PinType::MaterialAttributesPbr), attrs);
        assert!(attrs.wesl_literal(PinType::MaterialAttributesPbr).starts_with("MaterialAttributesPbr(vec3<f32>(1.0"));
    }
}
