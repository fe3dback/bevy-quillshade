//! Pin data types and the implicit conversion rules between them (spec §7.1, 7.2).

use serde::{Deserialize, Serialize};

/// Data type of a pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PinType {
    Float,
    Float2,
    Float3,
    Float4,
    /// Linear color, `vec4<f32>` in WESL.
    Color,
    /// Runtime GPU boolean (`bool` in WESL), used with `RuntimeIf`, `And`, `Or`, `Not`, `Compare`.
    #[serde(alias = "Bool")]
    RuntimeBool,
    /// Editor-only compile-time boolean flag, evaluated during shader compilation to prune `If` branches.
    CompileBool,
    Int,
    Texture2D,
    /// Struct of PBR material attributes (decision 0023); only `Lerp`, `If`, `RuntimeIf` and Make/Break accept it.
    MaterialAttributesPbr,
    /// Polymorphic pin; only used for display, signatures always resolve to a concrete type.
    Any,
}

impl PinType {
    pub const ALL_CONCRETE: [PinType; 10] = [
        PinType::Float,
        PinType::Float2,
        PinType::Float3,
        PinType::Float4,
        PinType::Color,
        PinType::RuntimeBool,
        PinType::CompileBool,
        PinType::Int,
        PinType::Texture2D,
        PinType::MaterialAttributesPbr,
    ];

    pub const ALL: [PinType; 10] = Self::ALL_CONCRETE;

    /// WESL type name. Texture2D has no value type (it is a texture + sampler pair).
    pub fn wesl(self) -> &'static str {
        match self {
            PinType::Float => "f32",
            PinType::Float2 => "vec2<f32>",
            PinType::Float3 => "vec3<f32>",
            PinType::Float4 | PinType::Color => "vec4<f32>",
            PinType::RuntimeBool | PinType::CompileBool => "bool",
            PinType::Int => "i32",
            PinType::Texture2D => "texture_2d<f32>",
            PinType::MaterialAttributesPbr => crate::attributes::PBR_STRUCT,
            PinType::Any => "f32",
        }
    }

    /// Number of float components for float-like types.
    pub fn dim(self) -> Option<u8> {
        match self {
            PinType::Float => Some(1),
            PinType::Float2 => Some(2),
            PinType::Float3 => Some(3),
            PinType::Float4 | PinType::Color => Some(4),
            _ => None,
        }
    }

    pub fn float_of_dim(dim: u8) -> PinType {
        match dim {
            0 | 1 => PinType::Float,
            2 => PinType::Float2,
            3 => PinType::Float3,
            _ => PinType::Float4,
        }
    }

    /// Float-like type (scalars, vectors, color) usable by polymorphic math.
    pub fn is_numeric(self) -> bool {
        self.dim().is_some() || self == PinType::Int
    }

    /// The concrete type labelled `s` ([`PinType::label`]).
    pub fn from_label(s: &str) -> Option<PinType> {
        if s == "Bool" {
            return Some(PinType::RuntimeBool);
        }
        PinType::ALL_CONCRETE.into_iter().find(|t| t.label() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            PinType::Float => "Float",
            PinType::Float2 => "Float2",
            PinType::Float3 => "Float3",
            PinType::Float4 => "Float4",
            PinType::Color => "Color",
            PinType::RuntimeBool => "RuntimeBool",
            PinType::CompileBool => "CompileBool",
            PinType::Int => "Int",
            PinType::Texture2D => "Texture2D",
            PinType::MaterialAttributesPbr => "MaterialAttributes (PBR)",
            PinType::Any => "Any",
        }
    }

    /// Pin color (sRGB, 0..1) as listed in spec §7.1.
    pub fn color(self) -> [f32; 3] {
        match self {
            PinType::Float => [0.62, 0.62, 0.62],
            PinType::Float2 => [0.35, 0.80, 0.35],
            PinType::Float3 => [0.92, 0.85, 0.25],
            PinType::Float4 => [0.95, 0.45, 0.75],
            PinType::Color => [0.98, 0.55, 0.80],
            PinType::RuntimeBool => [0.90, 0.25, 0.25],
            PinType::CompileBool => [0.95, 0.55, 0.20],
            PinType::Int => [0.40, 0.85, 0.95],
            PinType::Texture2D => [0.30, 0.45, 0.95],
            PinType::MaterialAttributesPbr => [0.55, 0.35, 0.95],
            PinType::Any => [0.95, 0.95, 0.95],
        }
    }
}

/// Implicit conversion applied on an edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conversion {
    Identity,
    /// `Float` → `FloatN`/`Color`.
    Broadcast,
    /// `FloatN` → `FloatM`, N > M (emits W104).
    Truncate,
    /// `Float3` → `Color`/`Float4`: `vec4(x, 1.0)`.
    ExtendAlpha,
    /// `Int` → float-like.
    IntToFloat,
}

impl Conversion {
    /// Wraps `expr` (of type `from`) so the result has type `to`.
    pub fn apply(self, expr: &str, from: PinType, to: PinType) -> String {
        match self {
            Conversion::Identity => expr.to_string(),
            Conversion::Broadcast => match to {
                PinType::Color => format!("vec4<f32>(vec3<f32>({expr}), 1.0)"),
                _ => format!("{}({expr})", to.wesl()),
            },
            Conversion::Truncate => {
                let swz = match to.dim().unwrap_or(1) {
                    1 => "x",
                    2 => "xy",
                    3 => "xyz",
                    _ => "xyzw",
                };
                // Color → Float3 is the common "take rgb" case; spell it as such.
                if from == PinType::Color && to == PinType::Float3 {
                    format!("({expr}).rgb")
                } else {
                    format!("({expr}).{swz}")
                }
            }
            Conversion::ExtendAlpha => format!("vec4<f32>({expr}, 1.0)"),
            Conversion::IntToFloat => {
                let f = format!("f32({expr})");
                match to {
                    PinType::Float => f,
                    PinType::Color => format!("vec4<f32>(vec3<f32>({f}), 1.0)"),
                    _ => format!("{}({f})", to.wesl()),
                }
            }
        }
    }

    /// Whether this conversion loses data and should be reported as W104.
    pub fn is_lossy(self, from: PinType, to: PinType) -> bool {
        // Color → Float3 is the explicit "rgb" rule of 7.2, not a lossy truncation.
        self == Conversion::Truncate && !(from == PinType::Color && to == PinType::Float3)
    }
}

/// Finds the implicit conversion from an output of type `from` to an input of type `to`.
/// Returns `None` when the connection is not allowed (E003).
pub fn conversion(from: PinType, to: PinType) -> Option<Conversion> {
    use PinType::*;
    if from == to {
        return Some(Conversion::Identity);
    }
    match (from, to) {
        (Any, _) | (_, Any) => None,
        (Texture2D, _) | (_, Texture2D) => None,
        (MaterialAttributesPbr, _) | (_, MaterialAttributesPbr) => None,
        (RuntimeBool, _) | (_, RuntimeBool) => None,
        (CompileBool, _) | (_, CompileBool) => None,
        (Int, Int) => Some(Conversion::Identity),
        (Int, t) if t.dim().is_some() => Some(Conversion::IntToFloat),
        (_, Int) => None,
        (Float4, Color) | (Color, Float4) => Some(Conversion::Identity),
        (Float3, Color) | (Float3, Float4) => Some(Conversion::ExtendAlpha),
        (Float, t) if t.dim().is_some() => Some(Conversion::Broadcast),
        (f, t) => {
            let (fd, td) = (f.dim()?, t.dim()?);
            if fd > td { Some(Conversion::Truncate) } else { None }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_rules() {
        use PinType::*;
        assert_eq!(conversion(Float, Float3), Some(Conversion::Broadcast));
        assert_eq!(conversion(Float4, Float2), Some(Conversion::Truncate));
        assert_eq!(conversion(Float2, Float3), None);
        assert_eq!(conversion(Color, Float3), Some(Conversion::Truncate));
        assert!(!Conversion::Truncate.is_lossy(Color, Float3));
        assert!(Conversion::Truncate.is_lossy(Float4, Float2));
        assert_eq!(conversion(Float3, Color), Some(Conversion::ExtendAlpha));
        assert_eq!(conversion(Int, Float), Some(Conversion::IntToFloat));
        assert_eq!(conversion(Float, Int), None);
        assert_eq!(conversion(RuntimeBool, Float), None);
        assert_eq!(conversion(CompileBool, Float), None);
        assert_eq!(conversion(RuntimeBool, RuntimeBool), Some(Conversion::Identity));
        assert_eq!(conversion(CompileBool, CompileBool), Some(Conversion::Identity));
        assert_eq!(conversion(RuntimeBool, CompileBool), None);
        assert_eq!(conversion(CompileBool, RuntimeBool), None);
        assert_eq!(PinType::from_label("Bool"), Some(RuntimeBool));
        assert_eq!(PinType::from_label("RuntimeBool"), Some(RuntimeBool));
        assert_eq!(PinType::from_label("CompileBool"), Some(CompileBool));
        assert_eq!(conversion(Texture2D, Texture2D), Some(Conversion::Identity));
        assert_eq!(conversion(Texture2D, Float4), None);
        assert_eq!(conversion(MaterialAttributesPbr, MaterialAttributesPbr), Some(Conversion::Identity));
        assert_eq!(conversion(Float3, MaterialAttributesPbr), None);
        assert_eq!(conversion(Float, MaterialAttributesPbr), None);
        assert_eq!(conversion(MaterialAttributesPbr, Float3), None);
        assert!(!MaterialAttributesPbr.is_numeric());
    }

    #[test]
    fn conversion_apply() {
        use PinType::*;
        assert_eq!(Conversion::Broadcast.apply("x", Float, Float3), "vec3<f32>(x)");
        assert_eq!(Conversion::Truncate.apply("v", Float4, Float2), "(v).xy");
        assert_eq!(Conversion::Truncate.apply("c", Color, Float3), "(c).rgb");
        assert_eq!(Conversion::ExtendAlpha.apply("v", Float3, Color), "vec4<f32>(v, 1.0)");
        assert_eq!(Conversion::IntToFloat.apply("i", Int, Float2), "vec2<f32>(f32(i))");
    }
}
