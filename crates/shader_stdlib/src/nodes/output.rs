//! Output category: MaterialOutput per asset type, FunctionInput, FunctionOutput.

use shader_graph_core::registry::{NodeRole, Operand, ParamDef, ParamKind, PinDef, Registry};
use shader_graph_core::{AssetKind, BlendMode, Code, MaterialKind, PinType, Shading, SigCtx, Value};

use crate::def::{def, pin_out, sig};

const CAT: &str = "Output";

fn out_pin(id: &str, label: &str, ty: PinType, implicit: &str) -> PinDef {
    PinDef::new(id, label, ty).implicit(implicit)
}

fn blend(c: &SigCtx) -> BlendMode {
    c.material_kind().map(|m| m.blend()).unwrap_or(BlendMode::Opaque)
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
                    let b = blend(c);
                    let unlit = matches!(c.material_kind(), Some(MaterialKind::Pbr3d(s)) if s.shading == Shading::Unlit);
                    sig(
                        vec![
                            out_pin("base_color", "Base Color", PinType::Float3, "StandardMaterial"),
                            out_pin("metallic", "Metallic", PinType::Float, "StandardMaterial").inactive(unlit),
                            out_pin("specular", "Specular", PinType::Float, "0.5").inactive(unlit),
                            out_pin("roughness", "Roughness", PinType::Float, "StandardMaterial").inactive(unlit),
                            out_pin("emissive", "Emissive Color", PinType::Float3, "0"),
                            out_pin("opacity", "Opacity", PinType::Float, "1.0").inactive(opacity_inactive(b)),
                            out_pin("opacity_mask", "Opacity Mask", PinType::Float, "1.0").inactive(b != BlendMode::Mask),
                            out_pin("normal", "Normal", PinType::Float3, "vertex normal").inactive(unlit),
                            out_pin(shader_graph_core::ir::WPO_PIN, "World Position Offset", PinType::Float3, "0"),
                            out_pin("ambient_occlusion", "Ambient Occlusion", PinType::Float, "1.0").inactive(unlit),
                        ],
                        vec![],
                    )
                },
                |c| {
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
                            out_pin(shader_graph_core::ir::WPO_PIN, "World Position Offset", PinType::Float3, "0"),
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

    let all_types: Vec<PinType> = PinType::ALL_CONCRETE.to_vec();
    r.register(
        def("output::FunctionInput", "FunctionInput", CAT)
            .kw(&["input", "argument", "parameter"])
            .role(NodeRole::FunctionInput)
            .param(ParamDef::new("name", "Name", ParamKind::Str, Value::Str("input".into())))
            .param(ParamDef::new("type", "Type", ParamKind::PinType(all_types.clone()), Value::Type(PinType::Float3)))
            .param(ParamDef::new("default", "Default", ParamKind::Float4, Value::Float4([0.0; 4])))
            .param(ParamDef::new("required", "Required", ParamKind::Bool, Value::Bool(false)))
            .param(ParamDef::new("order", "Sort Priority", ParamKind::Int { min: None, max: None }, Value::Int(0)))
            .build(
                |c| {
                    let ty = c.param("type").as_type().unwrap_or(PinType::Float3);
                    sig(vec![], vec![pin_out("out", c.param("name").as_str(), ty)])
                },
                |c| {
                    let name = c.param("name").as_str().to_string();
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
