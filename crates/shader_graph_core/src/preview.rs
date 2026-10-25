//! Node preview (spec §5, decision 0022): which material outputs a previewed node feeds.

use std::collections::BTreeMap;

use crate::model::MaterialKind;
use crate::registry::Signature;
use crate::types::{PinType, conversion};

/// Whether a value of type `ty` can be previewed as albedo (see [`albedo_expr`]).
pub fn converts_to_albedo(ty: PinType) -> bool {
    ty == PinType::Float2 || conversion(ty, PinType::Float3).is_some()
}

/// `expr` of type `ty` as a Float3 albedo (spec §5): Float2 → `(x, y, 0)`, everything else through the edge
/// conversions (Float → `(v, v, v)`, Float4/Color → `.rgb`, Int → float). The Float2 rule is preview-only: edges
/// still reject Float2 → Float3.
pub fn albedo_expr(expr: &str, ty: PinType) -> Option<String> {
    if ty == PinType::Float2 {
        return Some(format!("vec3<f32>({expr}, 0.0)"));
    }
    Some(conversion(ty, PinType::Float3)?.apply(expr, ty, PinType::Float3))
}

/// Material output a previewed node can feed (spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PreviewSlot {
    Albedo,
    Metallic,
    Specular,
    Roughness,
    Emissive,
    Opacity,
    Normal,
    AmbientOcclusion,
}

impl PreviewSlot {
    /// Input pin of the material output node for this slot (`base_color` / `color` for Albedo), if the kind has it.
    pub fn output_pin(self, kind: &MaterialKind) -> Option<&'static str> {
        use PreviewSlot::*;
        match kind {
            MaterialKind::Pbr3d(_) => Some(match self {
                Albedo => "base_color",
                Metallic => "metallic",
                Specular => "specular",
                Roughness => "roughness",
                Emissive => "emissive",
                Opacity => "opacity",
                Normal => "normal",
                AmbientOcclusion => "ambient_occlusion",
            }),
            MaterialKind::Unlit3d(_) => match self {
                Albedo => Some("color"),
                Opacity => Some("opacity"),
                _ => None,
            },
            MaterialKind::Sprite2d(_) => (self == Albedo).then_some("color"),
        }
    }
}

/// What a previewed node feeds: slot → the node's own output pin id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShaderOutputDto {
    pub slots: BTreeMap<PreviewSlot, String>,
}

impl ShaderOutputDto {
    /// Default: the first output → Albedo, if [`converts_to_albedo`]; otherwise empty (not previewable).
    pub fn main_output(sig: &Signature) -> Self {
        let mut dto = ShaderOutputDto::default();
        if let Some(first) = sig.outputs.first()
            && converts_to_albedo(first.ty)
        {
            dto.slots.insert(PreviewSlot::Albedo, first.id.clone());
        }
        dto
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PbrSettings, SpriteSettings, UnlitSettings};
    use crate::registry::PinDef;

    fn outputs(pins: &[(&str, PinType)]) -> Signature {
        Signature { inputs: vec![], outputs: pins.iter().map(|(id, ty)| PinDef::new(id, "", *ty)).collect() }
    }

    #[test]
    fn preview_main_output_takes_first_output_as_albedo() {
        let dto = ShaderOutputDto::main_output(&outputs(&[("rgb", PinType::Float3), ("r", PinType::Float)]));
        assert_eq!(dto.slots, BTreeMap::from([(PreviewSlot::Albedo, "rgb".to_string())]));
        assert!(!dto.is_empty());
    }

    #[test]
    fn preview_float2_main_output_is_previewable() {
        let dto = ShaderOutputDto::main_output(&outputs(&[("uv", PinType::Float2)]));
        assert_eq!(dto.slots, BTreeMap::from([(PreviewSlot::Albedo, "uv".to_string())]));
    }

    #[test]
    fn preview_albedo_expr() {
        assert_eq!(albedo_expr("v", PinType::Float2).as_deref(), Some("vec3<f32>(v, 0.0)"));
        assert_eq!(albedo_expr("v", PinType::Float).as_deref(), Some("vec3<f32>(v)"));
        assert_eq!(albedo_expr("v", PinType::Float3).as_deref(), Some("v"));
        assert_eq!(albedo_expr("c", PinType::Color).as_deref(), Some("(c).rgb"));
        assert_eq!(albedo_expr("b", PinType::Bool), None);
        assert_eq!(albedo_expr("t", PinType::Texture2D), None);
        assert!(converts_to_albedo(PinType::Int) && !converts_to_albedo(PinType::Bool));
    }

    #[test]
    fn preview_main_output_of_non_color_types_is_empty() {
        assert!(ShaderOutputDto::main_output(&outputs(&[("b", PinType::Bool), ("f", PinType::Float)])).is_empty());
        assert!(ShaderOutputDto::main_output(&outputs(&[("t", PinType::Texture2D)])).is_empty());
        assert!(ShaderOutputDto::main_output(&outputs(&[])).is_empty());
    }

    #[test]
    fn preview_slot_output_pins() {
        let pbr = MaterialKind::Pbr3d(PbrSettings::default());
        let unlit = MaterialKind::Unlit3d(UnlitSettings::default());
        let sprite = MaterialKind::Sprite2d(SpriteSettings::default());
        assert_eq!(PreviewSlot::Albedo.output_pin(&pbr), Some("base_color"));
        assert_eq!(PreviewSlot::Albedo.output_pin(&unlit), Some("color"));
        assert_eq!(PreviewSlot::Albedo.output_pin(&sprite), Some("color"));
        assert_eq!(PreviewSlot::AmbientOcclusion.output_pin(&pbr), Some("ambient_occlusion"));
        assert_eq!(PreviewSlot::Opacity.output_pin(&unlit), Some("opacity"));
        assert_eq!(PreviewSlot::Roughness.output_pin(&unlit), None);
        assert_eq!(PreviewSlot::Opacity.output_pin(&sprite), None);
    }
}
