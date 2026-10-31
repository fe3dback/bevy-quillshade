//! Output category: MaterialOutput per asset type, FunctionInput, FunctionOutput.

use shader_graph_core::attributes::{pbr_field_active, pbr_fields};
use shader_graph_core::ident::input_ident;
use shader_graph_core::ir::WPO_PIN;
use shader_graph_core::registry::{NodeRole, Operand, ParamDef, ParamKind, PinDef, Registry};
use shader_graph_core::{AssetKind, BlendMode, Code, MaterialKind, PbrOutputMode, PbrSettings, PinType, SigCtx, Value};

use crate::def::{def, pin_out, sig};

const CAT: &str = "Output";

fn out_pin(id: &str, label: &str, ty: PinType, implicit: &str) -> PinDef {
    PinDef::new(id, label, ty).implicit(implicit)
}

/// FunctionInput `min` / `max`: shown only for a Float input; never written into a node unless edited, so files
/// without them stay byte-identical (design §4.10).
fn range_param(id: &str, label: &str) -> ParamDef {
    ParamDef::new(id, label, ParamKind::Float { min: None, max: None }, Value::Float(0.0))
        .shown_if("type", Value::Type(PinType::Float))
}

fn blend(c: &SigCtx) -> BlendMode {
    c.material_kind().map(|m| m.blend()).unwrap_or(BlendMode::Opaque)
}

fn pbr_settings(c: &SigCtx) -> PbrSettings {
    match c.material_kind() {
        Some(MaterialKind::Pbr3d(s)) => *s,
        _ => PbrSettings::default(),
    }
}

fn opacity_inactive(b: BlendMode) -> bool {
    matches!(b, BlendMode::Opaque | BlendMode::Mask)
}

/// Records every connected input of a sink node as an output value.
fn emit_sink(c: &mut shader_graph_core::EmitCtx) {
    let pins: Vec<String> = c.sig.inputs.iter().map(|p| p.id.clone()).collect();
    for p in pins {
        if c.is_connected(&p) {
            let v = c.input(&p);
            c.set_output(&p, v);
        }
    }
}

pub fn register(r: &mut Registry) {
    r.register(
        def("output::Pbr3d", "MaterialOutput (PBR)", CAT)
            .kw(&["output", "result", "material"])
            .role(NodeRole::MaterialOutput)
            .build(
                |c| {
                    let s = pbr_settings(c);
                    let wpo = out_pin(WPO_PIN, "World Position Offset", PinType::Float3, "0");
                    if s.output == PbrOutputMode::Wrapped {
                        let attrs = out_pin("attributes", "Material Attributes", PinType::MaterialAttributesPbr, "StandardMaterial");
                        return sig(vec![attrs, wpo], vec![]);
                    }
                    let field = |id: &str, label: &str, ty: PinType, implicit: &str| {
                        out_pin(id, label, ty, implicit).inactive(!pbr_field_active(id, &s))
                    };
                    sig(
                        vec![
                            field("base_color", "Base Color", PinType::Float3, "StandardMaterial"),
                            field("metallic", "Metallic", PinType::Float, "StandardMaterial"),
                            field("specular", "Specular", PinType::Float, "0.5"),
                            field("roughness", "Roughness", PinType::Float, "StandardMaterial"),
                            field("emissive", "Emissive Color", PinType::Float3, "0"),
                            field("opacity", "Opacity", PinType::Float, "1.0"),
                            field("opacity_mask", "Opacity Mask", PinType::Float, "1.0"),
                            field("normal", "Normal", PinType::Float3, "vertex normal"),
                            wpo,
                            field("ambient_occlusion", "Ambient Occlusion", PinType::Float, "1.0"),
                        ],
                        vec![],
                    )
                },
                |c| {
                    let s = match c.env.asset_kind.material() {
                        Some(MaterialKind::Pbr3d(s)) => *s,
                        _ => PbrSettings::default(),
                    };
                    if s.output == PbrOutputMode::Wrapped {
                        // The struct expands into the per-field keys of `Default` mode, so the templates and the
                        // preview stay unchanged. Unconnected = nothing written (decision 0023); no W103 here,
                        // the normal is always written.
                        if c.is_connected("attributes") {
                            let v = c.input("attributes");
                            for f in pbr_fields().iter().filter(|f| pbr_field_active(f.id, &s)) {
                                c.set_output(f.id, format!("{v}.{}", f.id));
                            }
                        }
                        if c.is_connected(WPO_PIN) {
                            let v = c.input(WPO_PIN);
                            c.set_output(WPO_PIN, v);
                        }
                        return;
                    }
                    if c.is_connected("normal") {
                        c.diag_pin(
                            Code::W103,
                            "normal",
                            "Normal requires mesh tangents (ATTRIBUTE_TANGENT); meshes without them keep the vertex normal",
                        );
                    }
                    emit_sink(c);
                },
            ),
    );

    r.register(
        def("output::Unlit3d", "MaterialOutput (Unlit)", CAT)
            .kw(&["output", "result", "material", "unlit"])
            .role(NodeRole::MaterialOutput)
            .build(
                |c| {
                    let b = blend(c);
                    sig(
                        vec![
                            out_pin("color", "Color", PinType::Float3, "0"),
                            out_pin("opacity", "Opacity", PinType::Float, "1.0").inactive(opacity_inactive(b)),
                            out_pin("opacity_mask", "Opacity Mask", PinType::Float, "1.0")
                                .inactive(b != BlendMode::Mask),
                            out_pin(WPO_PIN, "World Position Offset", PinType::Float3, "0"),
                        ],
                        vec![],
                    )
                },
                emit_sink,
            ),
    );

    r.register(
        def("output::Sprite2d", "MaterialOutput (2D)", CAT)
            .kw(&["output", "result", "material", "sprite", "2d"])
            .role(NodeRole::MaterialOutput)
            .build(
                |_| {
                    sig(
                        vec![
                            out_pin("color", "Color", PinType::Float4, "sample_final_color(uv)"),
                            out_pin("uv", "UV override", PinType::Float2, "uv"),
                        ],
                        vec![],
                    )
                },
                emit_sink,
            ),
    );

    let all_types: Vec<PinType> = PinType::ALL_CONCRETE.into_iter().filter(|t| *t != PinType::CompileBool).collect();
    r.register(
        def("output::FunctionInput", "FunctionInput", CAT)
            .kw(&["input", "argument", "parameter"])
            .role(NodeRole::FunctionInput)
            .param(ParamDef::new("name", "Name", ParamKind::Str, Value::Str("input".into())))
            .param(ParamDef::new("type", "Type", ParamKind::PinType(all_types.clone()), Value::Type(PinType::Float3)))
            .param(ParamDef::new("default", "Default", ParamKind::Float4, Value::Float4([0.0; 4])))
            .param(ParamDef::new("required", "Required", ParamKind::Bool, Value::Bool(false)))
            .param(ParamDef::new("order", "Sort Priority", ParamKind::Int { min: None, max: None }, Value::Int(0)))
            // Slider range of the call nodes' pin when `min < max` (design §4.10); unset by default.
            .param(range_param("min", "Slider Min"))
            .param(range_param("max", "Slider Max"))
            .build(
                |c| {
                    let ty = c.param("type").as_type().unwrap_or(PinType::Float3);
                    sig(vec![], vec![pin_out("out", c.param("name").as_str(), ty)])
                },
                |c| {
                    // Argument of the generated `fn` (decision 0020).
                    let name = input_ident(c.param("name").as_str());
                    let op = if c.out_type("out") == PinType::Texture2D {
                        Operand::Texture { tex: name.clone(), sampler: format!("{name}_sampler") }
                    } else {
                        Operand::Value(name)
                    };
                    c.alias("out", op);
                },
            ),
    );

    r.register(
        def("output::FunctionOutput", "FunctionOutput", CAT)
            .kw(&["output", "return", "result"])
            .role(NodeRole::FunctionOutput)
            .param(ParamDef::new("name", "Name", ParamKind::Str, Value::Str("result".into())))
            .param(ParamDef::new("type", "Type", ParamKind::PinType(all_types), Value::Type(PinType::Float3)))
            .param(ParamDef::new("order", "Sort Priority", ParamKind::Int { min: None, max: None }, Value::Int(0)))
            .build(
                |c| {
                    let ty = c.param("type").as_type().unwrap_or(PinType::Float3);
                    sig(vec![PinDef::new("value", c.param("name").as_str(), ty)], vec![])
                },
                |c| {
                    if !matches!(c.env.asset_kind, AssetKind::Function) {
                        return;
                    }
                    let name = c.param("name").as_str().to_string();
                    let v = c.input("value");
                    c.set_output(&name, v);
                },
            ),
    );
}
