//! Value editors drawn inside canvas nodes (UE-style inline constants): unconnected input defaults and
//! the value of constant/parameter nodes. Widgets reuse [`PropField`], so edits take the same path as the
//! Properties panel.

use std::collections::BTreeMap;

use bevy::feathers::controls::{ColorInputValue, FeathersColorInput, FeathersSlider, color_input_self_update};
use bevy::prelude::*;
use bevy::ui_widgets::{SliderPrecision, SliderValue, slider_self_update};
use shader_graph_core::{PinType, Value};

use crate::state::PropTarget;
use crate::ui::props::{PropField, checkbox_w, integer, linear_color, number, vector_fields};

/// Kind of inline editor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Inline {
    /// Scalar with a known range.
    Slider {
        min: f32,
        max: f32,
    },
    Number,
    Integer,
    /// Number fields per component.
    Vector(usize),
    /// Color picker plus number fields per component.
    Color(usize),
    Check,
}

/// Editor for an unconnected input of type `ty`; `range` comes from the pin definition.
pub fn inline_for_pin(ty: PinType, range: Option<(f32, f32)>) -> Option<Inline> {
    Some(match ty {
        PinType::Float => match range {
            Some((min, max)) => Inline::Slider { min, max },
            None => Inline::Number,
        },
        PinType::Int => Inline::Integer,
        PinType::Float2 => Inline::Vector(2),
        PinType::Float3 => Inline::Color(3),
        PinType::Float4 | PinType::Color => Inline::Color(4),
        PinType::Bool => Inline::Check,
        PinType::Texture2D | PinType::Any => return None,
    })
}

/// Editor for the value of a constant/parameter node: the parameter key it edits and how.
pub fn inline_for_param(kind: &str, params: &BTreeMap<String, Value>) -> Option<(String, Inline)> {
    let f = |k: &str| params.get(k).map(Value::as_f32).filter(|x| x.is_finite());
    let (key, ty) = match kind {
        "param::Scalar" => {
            let inline = match (f("min"), f("max")) {
                (Some(min), Some(max)) if min < max => Inline::Slider { min, max },
                _ => Inline::Number,
            };
            return Some(("default".into(), inline));
        }
        "param::Vector" => ("default", params.get("type").and_then(Value::as_type).unwrap_or(PinType::Float3)),
        _ if kind.starts_with("param::") => ("default", params.get("default")?.pin_type()?),
        _ if kind.starts_with("stdlib::Constant") => ("value", params.get("value")?.pin_type()?),
        _ => return None,
    };
    Some((key.into(), inline_for_pin(ty, None)?))
}

/// Scene of the editor; `v` is the current value (components beyond the editor's count are ignored).
pub fn inline_scene(target: PropTarget, inline: Inline, v: &Value) -> Box<dyn Scene> {
    let mut floats = v.floats();
    let row = |items: Vec<Box<dyn Scene>>| -> Box<dyn Scene> {
        Box::new(bsn! {
            Node { flex_direction: FlexDirection::Row, column_gap: px(3), flex_grow: 1.0, align_items: AlignItems::Center }
            Children [ {items} ]
        })
    };
    match inline {
        Inline::Slider { min, max } => {
            let (field, value) = (PropField { target, ..default() }, v.as_f32());
            Box::new(bsn! {
                @FeathersSlider { @min: min, @max: max }
                SliderValue({value})
                SliderPrecision(2)
                on(slider_self_update)
                Node { flex_grow: 1.0, min_width: px(100) }
                field
            })
        }
        Inline::Number => number(target, 0, v.as_f32(), None),
        Inline::Integer => integer(target, v.as_i32()),
        Inline::Check => checkbox_w(target, v.as_bool()),
        Inline::Vector(n) => {
            floats.resize(n, 0.0);
            row(vector_fields(&target, &floats, false))
        }
        Inline::Color(n) => {
            floats.resize(n, 1.0);
            let (field, color) = (PropField { target: target.clone(), ..default() }, linear_color(&floats));
            let mut items = vector_fields(&target, &floats, true);
            items.insert(
                0,
                Box::new(bsn! {
                    @FeathersColorInput
                    ColorInputValue(color)
                    on(color_input_self_update)
                    field
                }),
            );
            row(items)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use shader_graph_core::{PinType, Value};

    use super::*;

    #[test]
    fn pin_editor_follows_type_and_range() {
        assert_eq!(inline_for_pin(PinType::Float, Some((0.0, 1.0))), Some(Inline::Slider { min: 0.0, max: 1.0 }));
        assert_eq!(inline_for_pin(PinType::Float, None), Some(Inline::Number));
        assert_eq!(inline_for_pin(PinType::Int, None), Some(Inline::Integer));
        assert_eq!(inline_for_pin(PinType::Float2, None), Some(Inline::Vector(2)));
        assert_eq!(inline_for_pin(PinType::Float3, None), Some(Inline::Color(3)));
        assert_eq!(inline_for_pin(PinType::Float4, None), Some(Inline::Color(4)));
        assert_eq!(inline_for_pin(PinType::Color, None), Some(Inline::Color(4)));
        assert_eq!(inline_for_pin(PinType::Bool, None), Some(Inline::Check));
        assert_eq!(inline_for_pin(PinType::Texture2D, None), None);
        assert_eq!(inline_for_pin(PinType::Any, None), None);
    }

    #[test]
    fn scalar_parameter_slider_uses_its_range() {
        let mut p = BTreeMap::from([("min".to_string(), Value::Float(0.2)), ("max".to_string(), Value::Float(5.0))]);
        assert_eq!(
            inline_for_param("param::Scalar", &p),
            Some(("default".to_string(), Inline::Slider { min: 0.2, max: 5.0 }))
        );
        p.insert("max".into(), Value::Float(0.2));
        assert_eq!(inline_for_param("param::Scalar", &p), Some(("default".to_string(), Inline::Number)));
    }

    #[test]
    fn value_nodes_edit_their_value() {
        let p = |k: &str, v: Value| BTreeMap::from([(k.to_string(), v)]);
        let mut vector = p("default", Value::Float4([0.0; 4]));
        vector.insert("type".into(), Value::Type(PinType::Float3));
        assert_eq!(inline_for_param("param::Vector", &vector), Some(("default".to_string(), Inline::Color(3))));
        vector.insert("type".into(), Value::Type(PinType::Float2));
        assert_eq!(inline_for_param("param::Vector", &vector), Some(("default".to_string(), Inline::Vector(2))));
        let color = p("default", Value::Color([1.0; 4]));
        assert_eq!(inline_for_param("param::Color", &color), Some(("default".to_string(), Inline::Color(4))));
        let check = p("default", Value::Bool(false));
        assert_eq!(inline_for_param("param::Bool", &check), Some(("default".to_string(), Inline::Check)));
        let c3 = p("value", Value::Float3([0.1, 0.2, 0.3]));
        assert_eq!(inline_for_param("stdlib::Constant3Vector", &c3), Some(("value".to_string(), Inline::Color(3))));
        let c1 = p("value", Value::Float(2.0));
        assert_eq!(inline_for_param("stdlib::Constant", &c1), Some(("value".to_string(), Inline::Number)));
        let tex = p("default", Value::Str(String::new()));
        assert_eq!(inline_for_param("param::TextureObject", &tex), None);
        assert_eq!(inline_for_param("stdlib::Lerp", &c1), None);
    }
}
