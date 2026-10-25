//! Material Attributes (PBR): one pin carrying a whole PBR layer (decision 0023).
//!
//! The WESL struct lives in the always-written stdlib module `autogen/stdlib/material.wesl`. Field ids match the
//! pin ids of `output::Pbr3d`, so the `Wrapped` output mode expands the struct into the same output keys as the
//! `Default` mode. World Position Offset is deliberately not a field (functions are separate modules, decision 0002).

use crate::model::{BlendMode, PbrSettings, Shading};
use crate::types::PinType;
use crate::value::Value;

/// WESL struct name of [`PinType::MaterialAttributesPbr`].
pub const PBR_STRUCT: &str = "MaterialAttributesPbr";
/// Stdlib module that defines the struct and its helpers.
pub const MODULE: &str = "material";
/// Per-field `mix` of two attribute sets (`Lerp` over attributes).
pub const LERP_FN: &str = "sg_lerp_attributes_pbr";

/// One field of the PBR attributes struct.
#[derive(Debug, Clone, PartialEq)]
pub struct PbrField {
    pub id: &'static str,
    pub label: &'static str,
    pub ty: PinType,
    /// Fixed default used by Make and by an unconnected attributes input (not `StandardMaterial`'s values).
    pub default: Value,
}

/// Struct fields in declaration order.
pub fn pbr_fields() -> [PbrField; 9] {
    let f = |id, label, ty, default| PbrField { id, label, ty, default };
    [
        f("base_color", "Base Color", PinType::Float3, Value::Float3([1.0, 1.0, 1.0])),
        f("metallic", "Metallic", PinType::Float, Value::Float(0.0)),
        f("specular", "Specular", PinType::Float, Value::Float(0.5)),
        f("roughness", "Roughness", PinType::Float, Value::Float(0.5)),
        f("emissive", "Emissive Color", PinType::Float3, Value::Float3([0.0, 0.0, 0.0])),
        f("opacity", "Opacity", PinType::Float, Value::Float(1.0)),
        f("opacity_mask", "Opacity Mask", PinType::Float, Value::Float(1.0)),
        f("normal", "Normal", PinType::Float3, Value::Float3([0.0, 0.0, 1.0])),
        f("ambient_occlusion", "Ambient Occlusion", PinType::Float, Value::Float(1.0)),
    ]
}

/// Whether the material output uses the field `id` under these settings (the output node's active pins).
pub fn pbr_field_active(id: &str, s: &PbrSettings) -> bool {
    let lit = s.shading != Shading::Unlit;
    match id {
        "base_color" | "emissive" => true,
        "metallic" | "specular" | "roughness" | "normal" | "ambient_occlusion" => lit,
        "opacity" => !matches!(s.blend, BlendMode::Opaque | BlendMode::Mask),
        "opacity_mask" => s.blend == BlendMode::Mask,
        _ => false,
    }
}

/// Type of the field `id`, if it exists.
pub fn pbr_field_type(id: &str) -> Option<PinType> {
    pbr_fields().into_iter().find(|f| f.id == id).map(|f| f.ty)
}

/// Constructor call with every field at its default (an unconnected attributes input).
pub fn pbr_default_literal() -> String {
    let args: Vec<String> = pbr_fields().iter().map(|f| f.default.wesl_literal(f.ty)).collect();
    format!("{PBR_STRUCT}({})", args.join(", "))
}

/// The `(module, item)` stdlib import of the struct, for `EmitSink::stdlib`.
pub fn import() -> (String, String) {
    (MODULE.to_string(), PBR_STRUCT.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_order_and_defaults() {
        let ids: Vec<&str> = pbr_fields().iter().map(|f| f.id).collect();
        assert_eq!(
            ids,
            [
                "base_color",
                "metallic",
                "specular",
                "roughness",
                "emissive",
                "opacity",
                "opacity_mask",
                "normal",
                "ambient_occlusion"
            ]
        );
        assert_eq!(
            pbr_default_literal(),
            "MaterialAttributesPbr(vec3<f32>(1.0, 1.0, 1.0), 0.0f, 0.5f, 0.5f, vec3<f32>(0.0, 0.0, 0.0), 1.0f, 1.0f, \
             vec3<f32>(0.0, 0.0, 1.0), 1.0f)"
        );
    }

    #[test]
    fn field_activity() {
        let lit = PbrSettings::default();
        assert!(!pbr_field_active("opacity", &lit));
        assert!(!pbr_field_active("opacity_mask", &lit));
        assert!(pbr_field_active("metallic", &lit));
        let unlit_mask = PbrSettings { shading: Shading::Unlit, blend: BlendMode::Mask, ..Default::default() };
        assert!(!pbr_field_active("metallic", &unlit_mask));
        assert!(!pbr_field_active("normal", &unlit_mask));
        assert!(pbr_field_active("opacity_mask", &unlit_mask));
        assert!(pbr_field_active("base_color", &unlit_mask));
        let blend = PbrSettings { blend: BlendMode::Blend, ..Default::default() };
        assert!(pbr_field_active("opacity", &blend));
        assert!(!pbr_field_active("nope", &blend));
    }
}
