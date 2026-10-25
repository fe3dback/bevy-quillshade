//! Utility category: If, logic, shading helpers, FunctionCall, Custom, Reroute, Named Route, Sample Curve.

use shader_graph_core::ident::{function_ident, output_ident};
use shader_graph_core::registry::{
    Builtin, FUNCTION_CALL_KIND, FUNCTION_PREFIX, NodeRole, Operand, ParamDef, ParamKind, PinDef, ROUTE_PARAM,
    Registry, Signature,
};
use shader_graph_core::{Code, EmitCtx, Interp, PinType, SigCtx, Value};

use crate::def::{def, pin_in, pin_out, poly_ins, poly_out, sig};

const CAT: &str = "Utility";

pub fn register(r: &mut Registry) {
    r.register(
        def("stdlib::If", "If", CAT)
            .kw(&["if", "branch", "compare", "condition"])
            .desc("Picks A>B, A==B or A<B by comparing scalars A and B.")
            .param(ParamDef::new(
                "threshold",
                "Equals Threshold",
                ParamKind::Float { min: Some(0.0), max: None },
                Value::Float(0.00001),
            ))
            .build(
                |c| {
                    let (outs, ty, _) =
                        poly_ins(c, &[("gt", "A > B", 0.0), ("eq", "A == B", 0.0), ("lt", "A < B", 0.0)]);
                    let mut ins = vec![pin_in("a", "A", PinType::Float, 0.0), pin_in("b", "B", PinType::Float, 0.0)];
                    ins.extend(outs);
                    sig(ins, vec![poly_out("out", ty)])
                },
                |c| {
                    let thr = c.param("threshold").wesl_literal(PinType::Float);
                    let (a, b) = (c.input("a"), c.input("b"));
                    let e = format!(
                        "select(select({}, {}, abs({a} - {b}) <= {thr}), {}, ({a} - {b}) > {thr})",
                        c.input("lt"),
                        c.input("eq"),
                        c.input("gt")
                    );
                    c.define("out", e);
                },
            ),
    );

    r.register(def("stdlib::BoolSelect", "If (Bool)", CAT).kw(&["select", "switch", "bool if", "ternary"]).build(
        |c| {
            let (vals, ty, _) = poly_ins(c, &[("true_value", "True", 1.0), ("false_value", "False", 0.0)]);
            let mut ins = vec![PinDef::new("cond", "Condition", PinType::Bool)];
            ins.extend(vals);
            sig(ins, vec![poly_out("out", ty)])
        },
        |c| {
            let e = format!("select({}, {}, {})", c.input("false_value"), c.input("true_value"), c.input("cond"));
            c.define("out", e);
        },
    ));

    for (id, name, op) in [("And", "And", "&&"), ("Or", "Or", "||")] {
        r.register(def(&format!("stdlib::{id}"), name, CAT).kw(&["logic", "bool", op]).build(
            |_| {
                sig(
                    vec![PinDef::new("a", "A", PinType::Bool), PinDef::new("b", "B", PinType::Bool)],
                    vec![pin_out("out", "", PinType::Bool)],
                )
            },
            move |c| {
                let e = format!("({} {op} {})", c.input("a"), c.input("b"));
                c.define("out", e);
            },
        ));
    }
    r.register(def("stdlib::Not", "Not", CAT).kw(&["logic", "bool", "!", "invert"]).build(
        |_| sig(vec![PinDef::new("a", "", PinType::Bool)], vec![pin_out("out", "", PinType::Bool)]),
        |c| {
            let e = format!("!({})", c.input("a"));
            c.define("out", e);
        },
    ));
    let ops: Vec<String> = [">", ">=", "<", "<=", "==", "!="].iter().map(|s| s.to_string()).collect();
    r.register(
        def("stdlib::Compare", "Compare", CAT)
            .kw(&["logic", "greater", "less", "equal", ">", "<"])
            .param(ParamDef::new("op", "Operation", ParamKind::Enum(ops), Value::Str(">".into())))
            .build(
                |_| {
                    sig(
                        vec![pin_in("a", "A", PinType::Float, 0.0), pin_in("b", "B", PinType::Float, 0.0)],
                        vec![pin_out("out", "", PinType::Bool)],
                    )
                },
                |c| {
                    let op = match c.param("op").as_str() {
                        o @ (">" | ">=" | "<" | "<=" | "==" | "!=") => o.to_string(),
                        _ => ">".to_string(),
                    };
                    let e = format!("({} {op} {})", c.input("a"), c.input("b"));
                    c.define("out", e);
                },
            ),
    );

    r.register(def("stdlib::Fresnel", "Fresnel", CAT).kw(&["rim", "fresnel", "edge"]).build(
        |_| {
            sig(
                vec![
                    pin_in("exponent", "Exponent", PinType::Float, 5.0),
                    pin_in("base", "Base Reflect Fraction", PinType::Float, 0.04).range(0.0, 1.0),
                    pin_in("normal", "Normal", PinType::Float3, 0.0).implicit("vertex normal"),
                ],
                vec![pin_out("out", "", PinType::Float)],
            )
        },
        |c| {
            let n = if c.is_connected("normal") { c.input("normal") } else { c.builtin(Builtin::WorldNormal) };
            let (cam, pos) = (c.builtin(Builtin::CameraPosition), c.builtin(Builtin::WorldPosition));
            let f = c.stdlib("shading", "sg_fresnel");
            let e = format!("{f}({n}, normalize({cam} - {pos}), {}, {})", c.input("exponent"), c.input("base"));
            c.define("out", e);
        },
    ));

    r.register(
        def("stdlib::Desaturation", "Desaturation", CAT)
            .kw(&["grayscale", "greyscale", "saturation", "luminance"])
            .param(ParamDef::new("luminance", "Luminance Factors", ParamKind::Float3, Value::Float3([0.3, 0.59, 0.11])))
            .build(
                |_| {
                    sig(
                        vec![
                            pin_in("color", "Color", PinType::Float3, 0.0),
                            pin_in("fraction", "Fraction", PinType::Float, 1.0).range(0.0, 1.0),
                        ],
                        vec![pin_out("out", "", PinType::Float3)],
                    )
                },
                |c| {
                    let lum = c.param("luminance").wesl_literal(PinType::Float3);
                    let col = c.input("color");
                    let e = format!("mix({col}, vec3<f32>(dot({col}, {lum})), {})", c.input("fraction"));
                    c.define("out", e);
                },
            ),
    );

    let modes: Vec<String> = ["Overlay", "Screen", "Multiply", "SoftLight"].iter().map(|s| s.to_string()).collect();
    r.register(
        def("stdlib::Blend", "Blend", CAT)
            .kw(&["blend", "overlay", "screen", "soft light", "photoshop"])
            .param(ParamDef::new("mode", "Mode", ParamKind::Enum(modes), Value::Str("Overlay".into())))
            .build(
                |_| {
                    sig(
                        vec![
                            pin_in("base", "Base", PinType::Float3, 0.5),
                            pin_in("blend", "Blend", PinType::Float3, 0.5),
                        ],
                        vec![pin_out("out", "", PinType::Float3)],
                    )
                },
                |c| {
                    let func = match c.param("mode").as_str() {
                        "Screen" => "sg_blend_screen",
                        "Multiply" => "sg_blend_multiply",
                        "SoftLight" => "sg_blend_soft_light",
                        _ => "sg_blend_overlay",
                    };
                    let f = c.stdlib("color", func);
                    let e = format!("{f}({}, {})", c.input("base"), c.input("blend"));
                    c.define("out", e);
                },
            ),
    );

    r.register(def("stdlib::BlackBody", "BlackBody", CAT).kw(&["temperature", "kelvin", "fire"]).build(
        |_| sig(vec![pin_in("temp", "Temp", PinType::Float, 6500.0)], vec![pin_out("out", "", PinType::Float3)]),
        |c| {
            let f = c.stdlib("color", "sg_blackbody");
            let e = format!("{f}({})", c.input("temp"));
            c.define("out", e);
        },
    ));

    r.register(
        def("stdlib::Noise", "Noise", CAT)
            .kw(&["noise", "perlin", "value noise", "random"])
            .param(ParamDef::new(
                "function",
                "Function",
                ParamKind::Enum(vec!["Value".into(), "Perlin".into()]),
                Value::Str("Perlin".into()),
            ))
            .build(
                |_| {
                    sig(
                        vec![
                            pin_in("position", "Position", PinType::Float3, 0.0).implicit("world position"),
                            pin_in("scale", "Scale", PinType::Float, 1.0),
                        ],
                        vec![pin_out("out", "", PinType::Float)],
                    )
                },
                |c| {
                    let p = if c.is_connected("position") {
                        c.input("position")
                    } else {
                        c.builtin(Builtin::WorldPosition)
                    };
                    let func =
                        if c.param("function").as_str() == "Value" { "sg_value_noise3" } else { "sg_perlin_noise3" };
                    let f = c.stdlib("noise", func);
                    let e = format!("{f}({p} * {})", c.input("scale"));
                    c.define("out", e);
                },
            ),
    );

    r.register(
        def(FUNCTION_CALL_KIND, "FunctionCall", CAT)
            .kw(&["function", "call", "material function"])
            .desc("Calls a project function (kind `function::{path}`).")
            .build(function_call_sig, function_call_emit),
    );

    r.register(
        def("stdlib::Custom", "Custom", CAT)
            .kw(&["custom", "code", "hlsl", "wesl", "wgsl"])
            .desc("Hand-written WESL body. Inputs are `name: Type` pairs separated by commas; body may `return` or be an expression.")
            .param(ParamDef::new("inputs", "Inputs", ParamKind::Str, Value::Str("a: Float3, b: Float".into())))
            .param(ParamDef::new(
                "output_type",
                "Output Type",
                ParamKind::PinType(vec![
                    PinType::Float,
                    PinType::Float2,
                    PinType::Float3,
                    PinType::Float4,
                    PinType::Color,
                    PinType::Bool,
                    PinType::Int,
                ]),
                Value::Type(PinType::Float3),
            ))
            .param(ParamDef::new("code", "Code", ParamKind::Code, Value::Str("return a * b;".into())))
            .build(
                |c| {
                    let ins = parse_custom_inputs(c.param("inputs").as_str())
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(n, t)| PinDef::new(&n, &n, t))
                        .collect();
                    let out = c.param("output_type").as_type().unwrap_or(PinType::Float3);
                    sig(ins, vec![pin_out("out", "", out)])
                },
                |c| {
                    let inputs = match parse_custom_inputs(c.param("inputs").as_str()) {
                        Ok(i) => i,
                        Err(e) => {
                            c.diag(Code::E007, e);
                            return;
                        }
                    };
                    let name = format!("sg_custom_{}", c.node_id);
                    let mut args = Vec::new();
                    let mut call = Vec::new();
                    for (n, t) in &inputs {
                        if *t == PinType::Texture2D {
                            args.push(format!("{n}: texture_2d<f32>, {n}_sampler: sampler"));
                            let (tex, s) = c.input_texture(n).unwrap_or_default();
                            call.push(format!("{tex}, {s}"));
                        } else {
                            args.push(format!("{n}: {}", t.wesl()));
                            call.push(c.input(n));
                        }
                    }
                    let out_ty = c.out_type("out");
                    let code = c.param("code").as_str().trim().to_string();
                    let body = if code.contains("return") { code } else { format!("return {};", code.trim_end_matches(';')) };
                    let body: String = body.lines().map(|l| format!("    {l}\n")).collect();
                    let def = format!("fn {name}({}) -> {} {{\n{body}}}\n", args.join(", "), out_ty.wesl());
                    c.add_fn(&name, def);
                    c.define("out", format!("{name}({})", call.join(", ")));
                },
            ),
    );

    r.register(
        def("stdlib::Reroute", "Reroute", CAT)
            .kw(&["reroute", "knot", "dot", "wire"])
            .role(NodeRole::Reroute)
            .build(pass_through_sig, pass_through),
    );
    // Named routes (decision 0011): usages read the declaration's input without a wire.
    r.register(
        def("stdlib::NamedRoute", "Named Route", CAT)
            .kw(&["named reroute", "declaration", "teleport", "wireless", "route"])
            .desc("Wire end with a name; Named Route Usage nodes output its input anywhere in this graph.")
            .role(NodeRole::RouteDeclaration)
            .param(ParamDef::new("name", "Name", ParamKind::Str, Value::Str("route".into())))
            .build(pass_through_sig, pass_through),
    );
    r.register(
        def("stdlib::NamedRouteUsage", "Named Route Usage", CAT)
            .kw(&["named reroute", "usage", "teleport", "wireless", "route"])
            .desc("Outputs the input of the chosen Named Route.")
            .role(NodeRole::RouteUsage)
            .param(ParamDef::new(ROUTE_PARAM, "Route", ParamKind::Route, Value::Str(String::new())))
            .build(pass_through_sig, pass_through),
    );
    // Curve assets (decision 0015): the data is baked into the curve's own WESL module.
    let interps: Vec<String> = Interp::ALL.iter().map(|i| i.label().to_string()).collect();
    r.register(
        def("stdlib::SampleCurve", "Sample Curve", CAT)
            .kw(&["curve", "float curve", "ramp", "falloff", "easing", "spline", "catmull"])
            .desc("Value of a curve asset at T (0..1). Simple: linear between points; Catmull-Rom: smooth.")
            .param(ParamDef::new("curve", "Curve", ParamKind::Curve, Value::Str(String::new())))
            .param(ParamDef::new(
                "type",
                "Type",
                ParamKind::Enum(interps),
                Value::Str(Interp::CatmullRom.label().into()),
            ))
            .build(
                |_| {
                    sig(
                        vec![pin_in("t", "T", PinType::Float, 0.0).range(0.0, 1.0)],
                        vec![pin_out("out", "", PinType::Float)],
                    )
                },
                |c| {
                    let path = c.param("curve").as_str().to_string();
                    let interp = Interp::from_label(c.param("type").as_str()).unwrap_or_default();
                    if path.is_empty() {
                        c.diag(Code::E004, "no curve selected");
                        return;
                    }
                    let Some(f) = c.use_curve(&path, interp) else {
                        c.diag(Code::E006, format!("curve `{path}` does not exist or cannot be loaded"));
                        return;
                    };
                    let t = c.input("t");
                    c.define("out", format!("{f}({t})"));
                },
            ),
    );
}

/// `in` → `out` of the connected type (Float while unconnected).
fn pass_through_sig(c: &SigCtx) -> Signature {
    let ty = c.connected.get("in").copied().unwrap_or(PinType::Float);
    let resolved = c.connected.contains_key("in");
    let mut i = PinDef::new("in", "", ty);
    let mut o = PinDef::new("out", "", ty);
    i.poly = !resolved;
    o.poly = !resolved;
    sig(vec![i], vec![o])
}

fn pass_through(c: &mut EmitCtx) {
    let op = match c.input_texture("in") {
        Some((tex, sampler)) => Operand::Texture { tex, sampler },
        None => Operand::Value(c.input("in")),
    };
    c.alias("out", op);
}

/// Parses `a: Float3, tex: Texture2D` into pin names and types.
pub fn parse_custom_inputs(spec: &str) -> Result<Vec<(String, PinType)>, String> {
    let mut out: Vec<(String, PinType)> = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (name, ty) = part.split_once(':').ok_or_else(|| format!("expected `name: Type`, got `{part}`"))?;
        let name = name.trim().to_string();
        shader_graph_core::ident::check_code_ident(&name).map_err(|e| format!("custom input: {e}"))?;
        let ty =
            PinType::ALL_CONCRETE.into_iter().find(|t| t.label().eq_ignore_ascii_case(ty.trim())).ok_or_else(|| {
                format!("unknown type `{}` (use Float, Float2, Float3, Float4, Color, Bool, Int, Texture2D)", ty.trim())
            })?;
        if out.iter().any(|(n, _)| *n == name) {
            return Err(format!("duplicate custom input `{name}`"));
        }
        out.push((name, ty));
    }
    Ok(out)
}

fn function_path(kind: &str) -> &str {
    kind.strip_prefix(FUNCTION_PREFIX).unwrap_or(kind)
}

fn function_call_sig(c: &SigCtx) -> shader_graph_core::Signature {
    let Some(f) = c.env.functions.function(function_path(c.kind)) else {
        return sig(vec![], vec![]);
    };
    let ins = f
        .inputs
        .iter()
        .map(|p| {
            let mut pin = PinDef::new(&p.name, &p.name, p.ty).default(p.default.clone());
            pin.required = p.required;
            pin
        })
        .collect();
    let outs = f.outputs.iter().map(|p| pin_out(&p.name, &p.name, p.ty)).collect();
    sig(ins, outs)
}

fn function_call_emit(c: &mut EmitCtx) {
    let path = function_path(c.kind).to_string();
    let Some(f) = c.use_function(&path) else {
        c.diag(Code::E006, format!("function `{path}` does not exist"));
        return;
    };
    let mut args = Vec::new();
    for p in &f.inputs {
        if p.ty == PinType::Texture2D {
            let (tex, s) = c.input_texture(&p.name).unwrap_or_default();
            args.push(format!("{tex}, {s}"));
        } else {
            args.push(c.input(&p.name));
        }
    }
    for b in &f.builtins {
        args.push(c.builtin(*b));
    }
    // Prefixed WESL names (decision 0020): `fun_{name}(..)`, result fields `.out_{port}`.
    let call = format!("{}({})", function_ident(&f.name), args.join(", "));
    if f.outputs.len() == 1 {
        c.define(&f.outputs[0].name, call);
    } else {
        c.define_typed("ret_", &f.out_struct(), call);
        let r = c.var("ret_");
        for o in &f.outputs {
            c.alias(&o.name, Operand::Value(format!("{r}.{}", output_ident(&o.name))));
        }
    }
}
