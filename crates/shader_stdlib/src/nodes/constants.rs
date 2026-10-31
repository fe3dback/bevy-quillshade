//! Constants and Parameters categories.

use shader_graph_core::ident::{input_ident, to_snake_case};
use shader_graph_core::registry::{
    Builtin, MaterialParam, NodeRole, Operand, ParamDef, ParamKind, ParamType, Registry,
};
use shader_graph_core::value::wesl_scalar;
use shader_graph_core::{Code, ConstType, PinType, Value};

use crate::def::{def, pin_out, sig};
use crate::nodes::texture::{SAMPLE_PARAMS, TEXTURE_PARAM, texture_convert};

const CAT_CONST: &str = "Constants";
const CAT_PARAM: &str = "Parameters";

pub fn register(r: &mut Registry) {
    for (id, name, ty, kw, default) in [
        ("stdlib::Constant", "Constant", PinType::Float, &["1", "const", "scalar", "float"][..], Value::Float(0.0)),
        (
            "stdlib::Constant2Vector",
            "Constant2Vector",
            PinType::Float2,
            &["2", "vec2", "float2"][..],
            Value::Float2([0.0; 2]),
        ),
        (
            "stdlib::Constant3Vector",
            "Constant3Vector",
            PinType::Float3,
            &["3", "vec3", "float3", "color", "rgb"][..],
            Value::Float3([0.0; 3]),
        ),
        (
            "stdlib::Constant4Vector",
            "Constant4Vector",
            PinType::Float4,
            &["4", "vec4", "float4", "rgba"][..],
            Value::Float4([0.0; 4]),
        ),
    ] {
        let kind = match ty {
            PinType::Float => ParamKind::Float { min: None, max: None },
            PinType::Float2 => ParamKind::Float2,
            PinType::Float3 => ParamKind::Float3,
            _ => ParamKind::Float4,
        };
        r.register(
            def(id, name, CAT_CONST)
                .kw(kw)
                .desc("Literal value inlined into WESL.")
                .param(ParamDef::new("value", "Value", kind, default))
                .build(
                    move |_| sig(vec![], vec![pin_out("out", "", ty)]),
                    move |c| {
                        let lit = c.param("value").wesl_literal(ty);
                        c.alias("out", Operand::Value(lit));
                    },
                ),
        );
    }

    for (id, name, b, ty, kw) in [
        ("stdlib::Time", "Time", Builtin::Time, PinType::Float, &["time", "seconds", "globals"][..]),
        ("stdlib::DeltaTime", "DeltaTime", Builtin::DeltaTime, PinType::Float, &["dt", "delta"][..]),
        (
            "stdlib::ViewSize",
            "ViewSize",
            Builtin::ViewSize,
            PinType::Float2,
            &["resolution", "viewport", "screen size"][..],
        ),
    ] {
        r.register(def(id, name, CAT_CONST).kw(kw).build(
            move |_| sig(vec![], vec![pin_out("out", "", ty)]),
            move |c| {
                let v = c.builtin(b);
                c.alias("out", Operand::Value(v));
            },
        ));
    }

    for (id, name, value, kw) in [
        ("stdlib::Pi", "Pi", std::f32::consts::PI, &["pi", "3.14"][..]),
        ("stdlib::TwoPi", "TwoPi", std::f32::consts::TAU, &["tau", "2pi"][..]),
    ] {
        r.register(def(id, name, CAT_CONST).kw(kw).build(
            |_| sig(vec![], vec![pin_out("out", "", PinType::Float)]),
            move |c| c.alias("out", Operand::Value(wesl_scalar(value))),
        ));
    }

    r.register(
        def("stdlib::Toggle", "Toggle", CAT_CONST)
            .kw(&["toggle", "bool", "switch", "0", "1", "float"])
            .desc(
                "Constant toggle that outputs `f32` `0.0` (unchecked) or `1.0` (checked) for branchless shader math (`Multiply`, `Lerp`).\n\nNote: For compile-time branch pruning with `If`, use `CompileBool` or `CompileBoolParameter`. For a runtime `bool` (`true`/`false`) with `RuntimeIf`, use `RuntimeBool` or `RuntimeBoolParameter`.",
            )
            .param(ParamDef::new("value", "Value", ParamKind::Bool, Value::Bool(false)))
            .build(
                |_| sig(vec![], vec![pin_out("out", "", PinType::Float)]),
                |c| {
                    let lit = if c.param("value").as_bool() { "1.0f" } else { "0.0f" };
                    c.alias("out", Operand::Value(lit.into()));
                },
            ),
    );

    r.register(
        def("stdlib::RuntimeBool", "RuntimeBool", CAT_CONST)
            .kw(&["runtimeBool", "bool", "true", "false", "constant"])
            .desc(
                "Constant runtime `bool` (`true` or `false`) emitted into WESL for runtime GPU logic (`RuntimeIf`, `And`, `Or`, `Not`).\n\nNote: For compile-time branch pruning that strips the unused branch from WESL with `If`, use `CompileBool` or `CompileBoolParameter`. For a `0.0`/`1.0` `f32` value, use `Toggle`.",
            )
            .param(ParamDef::new("value", "Value", ParamKind::Bool, Value::Bool(false)))
            .build(
                |_| sig(vec![], vec![pin_out("out", "", PinType::RuntimeBool)]),
                |c| {
                    let lit = if c.param("value").as_bool() { "true" } else { "false" };
                    c.alias("out", Operand::Value(lit.into()));
                },
            ),
    );

    r.register(
        def("stdlib::CompileBool", "CompileBool", CAT_CONST)
            .kw(&["compileBool", "compileFlag", "flag", "static", "branch"])
            .desc(
                "Editor-only compile-time boolean flag (`CompileBool`) for `If` and compile-time logic (`CompileAnd`, `CompileOr`, `CompileNot`). Evaluated during shader compilation so the inactive branch is completely stripped from WESL.\n\nNote: To expose a named compile flag that future Material Instances can override to generate a separate shader variant, use `CompileBoolParameter`. For runtime GPU logic, use `RuntimeBool`.",
            )
            .param(ParamDef::new("value", "Value", ParamKind::Bool, Value::Bool(false)))
            .build(
                |_| sig(vec![], vec![pin_out("out", "", PinType::CompileBool)]),
                |c| {
                    let val = c.param("value").as_bool();
                    c.define_compile_bool("out", val);
                },
            ),
    );

    register_globals(r);
    register_parameters(r);
}

/// Parameter of the `Global*` nodes: the name of the constant they read (decision 0017).
pub const GLOBAL_CONST_PARAM: &str = "const";

/// Nodes reading a constant of a global constant table; its value is inlined as a literal (decision 0017).
fn register_globals(r: &mut Registry) {
    for (id, name, ty, kw, desc) in [
        (
            "stdlib::GlobalConstF32",
            "GlobalConstF32",
            ConstType::F32,
            &["global", "const", "scalar", "float", "f32"][..],
            "Value of an f32 global constant.",
        ),
        (
            "stdlib::GlobalConstVec2",
            "GlobalConstVec2",
            ConstType::Vec2,
            &["global", "const", "vec2", "float2"][..],
            "Value of a vec2 global constant.",
        ),
        (
            "stdlib::GlobalConstVec3",
            "GlobalConstVec3",
            ConstType::Vec3,
            &["global", "const", "vec3", "float3", "color", "rgb"][..],
            "Value of a vec3 global constant.",
        ),
        (
            "stdlib::GlobalConstVec4",
            "GlobalConstVec4",
            ConstType::Vec4,
            &["global", "const", "vec4", "float4", "rgba"][..],
            "Value of a vec4 global constant.",
        ),
        (
            "stdlib::GlobalToggle",
            "GlobalToggle",
            ConstType::Toggle,
            &["global", "const", "table", "toggle", "switch", "0", "1"][..],
            "Reads a global `toggle` constant (`0.0` or `1.0` `f32`) from `quillshade/material/consts.consts.ron` for branchless math (such as `Multiply` or `Lerp`).\n\nNote: To strip an entire shader branch at compile time with `If`, use `GlobalCompileBool`. For a runtime `bool` (`true`/`false`) with `RuntimeIf`, use `GlobalRuntimeBool`.",
        ),
        (
            "stdlib::GlobalRuntimeBool",
            "GlobalRuntimeBool",
            ConstType::RuntimeBool,
            &["global", "const", "table", "runtimeBool", "bool", "flag"][..],
            "Reads a global `runtimeBool` constant (`true` or `false`) from `quillshade/material/consts.consts.ron` for runtime GPU logic (`RuntimeIf`, `And`, `Or`, `Not`).\n\nNote: Both branches of `RuntimeIf` still execute on the GPU. To strip an inactive branch at compile time with `If`, use `GlobalCompileBool`. For a `0.0`/`1.0` `f32` value, use `GlobalToggle`.",
        ),
        (
            "stdlib::GlobalCompileBool",
            "GlobalCompileBool",
            ConstType::CompileBool,
            &["global", "const", "table", "compileBool", "compileFlag", "flag", "static"][..],
            "Reads a global `compileBool` flag from `quillshade/material/consts.consts.ron`. Editor-only and evaluated during shader compilation: never appears in WESL, and allows `If` to completely discard the inactive shader branch.\n\nNote: For runtime GPU boolean logic (`RuntimeIf`), use `GlobalRuntimeBool`. For a `0.0`/`1.0` `f32` multiplier, use `GlobalToggle`.",
        ),
    ] {
        let pin = ty.pin_type();
        r.register(
            def(id, name, CAT_CONST)
                .kw(kw)
                .desc(desc)
                .param(ParamDef::new(
                    GLOBAL_CONST_PARAM,
                    "Constant",
                    ParamKind::GlobalConst(ty),
                    Value::Str(String::new()),
                ))
                .build(
                    move |_| sig(vec![], vec![pin_out("out", "", pin)]),
                    move |c| {
                        let name = c.param(GLOBAL_CONST_PARAM).as_str().to_string();
                        if ty == ConstType::CompileBool {
                            let val = if name.is_empty() {
                                c.diag(Code::W106, "no global constant selected; outputs 1.0 (true for a flag)");
                                true
                            } else {
                                match c.global_const(&name) {
                                    Some(g) if g.ty == ty => g.value.as_bool(),
                                    Some(g) => {
                                        let found = g.ty.label();
                                        c.diag(
                                            Code::E003,
                                            format!(
                                                "global constant `{name}` is {found}, this node reads {}",
                                                ty.label()
                                            ),
                                        );
                                        return;
                                    }
                                    None => {
                                        c.diag(Code::E006, format!("global constant `{name}` does not exist"));
                                        return;
                                    }
                                }
                            };
                            c.define_compile_bool("out", val);
                            return;
                        }
                        let lit = if name.is_empty() {
                            // 1.0 rather than zero: a divisor or `log` argument stays valid.
                            c.diag(Code::W106, "no global constant selected; outputs 1.0 (true for a flag)");
                            Value::Float(1.0).wesl_literal(pin)
                        } else {
                            match c.global_const(&name) {
                                Some(g) if g.ty == ty => ty.wesl_literal(&g.value),
                                Some(g) => {
                                    let found = g.ty.label();
                                    c.diag(
                                        Code::E003,
                                        format!("global constant `{name}` is {found}, this node reads {}", ty.label()),
                                    );
                                    return;
                                }
                                None => {
                                    c.diag(Code::E006, format!("global constant `{name}` does not exist"));
                                    return;
                                }
                            }
                        };
                        c.alias("out", Operand::Value(lit));
                    },
                ),
        );
    }
}

fn common_params(default_name: &str) -> Vec<ParamDef> {
    vec![
        ParamDef::new("name", "Name", ParamKind::Str, Value::Str(default_name.into())),
        ParamDef::new("group", "Group", ParamKind::Str, Value::Str(String::new())),
    ]
}

/// Declares the material parameter; returns its WESL name (`inp_{name}`: uniform field, or the texture with
/// `inp_{name}_sampler`; decision 0020).
fn declare(c: &mut shader_graph_core::EmitCtx, ty: ParamType) -> String {
    let name = to_snake_case(c.param("name").as_str());
    let default = if ty == ParamType::Texture {
        // A Texture asset id; empty → the default texture (W110), unknown → E006.
        let id = c.param("default").as_str().to_string();
        Value::Str(crate::nodes::texture::resolve_texture(c, &id).unwrap_or(id))
    } else {
        c.param("default").clone().coerce(ty.pin_type())
    };
    let (min, max) = (c.params.get("min").map(Value::as_f32), c.params.get("max").map(Value::as_f32));
    let p = MaterialParam {
        name: name.clone(),
        ty,
        default,
        group: c.param("group").as_str().to_string(),
        min: min.filter(|m| m.is_finite() && min != max),
        max: max.filter(|m| m.is_finite() && min != max),
        node: c.node_id.to_string(),
    };
    c.declare_param(p);
    input_ident(&name)
}

fn register_parameters(r: &mut Registry) {
    let mut b = def("param::Scalar", "ScalarParameter", CAT_PARAM)
        .kw(&["s", "scalar parameter", "float parameter", "uniform"])
        .desc("Scalar set from the game (uniform field, constructor argument).")
        .role(NodeRole::Parameter);
    for p in common_params("scalar") {
        b = b.param(p);
    }
    b = b
        .param(ParamDef::new("default", "Default", ParamKind::Float { min: None, max: None }, Value::Float(0.0)))
        .param(ParamDef::new("min", "Slider Min", ParamKind::Float { min: None, max: None }, Value::Float(0.0)))
        .param(ParamDef::new("max", "Slider Max", ParamKind::Float { min: None, max: None }, Value::Float(1.0)));
    r.register(b.build(
        |_| sig(vec![], vec![pin_out("out", "", PinType::Float)]),
        |c| {
            let n = declare(c, ParamType::Scalar);
            c.alias("out", Operand::Value(format!("params.{n}")));
        },
    ));

    let mut b = def("param::Vector", "VectorParameter", CAT_PARAM)
        .kw(&["v", "vector parameter", "vec3 parameter", "uniform"])
        .role(NodeRole::Parameter);
    for p in common_params("vector") {
        b = b.param(p);
    }
    b = b
        .param(ParamDef::new(
            "type",
            "Type",
            ParamKind::PinType(vec![PinType::Float2, PinType::Float3, PinType::Float4]),
            Value::Type(PinType::Float3),
        ))
        .param(ParamDef::new("default", "Default", ParamKind::Float4, Value::Float4([0.0; 4])));
    r.register(b.build(
        |c| {
            let ty = vector_type(c.param("type"));
            sig(vec![], vec![pin_out("out", "", ty)])
        },
        |c| {
            let ty = vector_type(c.param("type"));
            let pt = match ty {
                PinType::Float2 => ParamType::Vec2,
                PinType::Float3 => ParamType::Vec3,
                _ => ParamType::Vec4,
            };
            let n = declare(c, pt);
            c.alias("out", Operand::Value(format!("params.{n}")));
        },
    ));

    let mut b = def("param::Color", "ColorParameter", CAT_PARAM)
        .kw(&["color parameter", "tint", "uniform", "rgba"])
        .role(NodeRole::Parameter);
    for p in common_params("color") {
        b = b.param(p);
    }
    b = b.param(ParamDef::new("default", "Default", ParamKind::Color, Value::Color([1.0, 1.0, 1.0, 1.0])));
    r.register(b.build(
        |_| sig(vec![], color_outputs()),
        |c| {
            let n = declare(c, ParamType::Color);
            let v = format!("params.{n}");
            alias_color_outputs(c, &v);
        },
    ));

    let mut b = def("param::RuntimeBool", "RuntimeBoolParameter", CAT_PARAM)
        .kw(&["bool", "runtimeBool", "switch", "toggle", "uniform", "parameter"])
        .desc(
            "Uniform `RuntimeBool` material parameter (`bool` on the Rust material, `u32` in the GPU uniform buffer) that can be changed at runtime in-game without recompiling the shader. Used with `RuntimeIf`, `And`, `Or`, `Not`.\n\nNote: Runtime branching evaluates both branches on the GPU. For static feature switches that strip the inactive branch at compile time (and generate a dedicated WESL shader variant when overridden in a Material Instance), use `CompileBoolParameter` with `If`.",
        )
        .role(NodeRole::Parameter);
    for p in common_params("flag") {
        b = b.param(p);
    }
    b = b.param(ParamDef::new("default", "Default", ParamKind::Bool, Value::Bool(false)));
    r.register(b.build(
        |_| sig(vec![], vec![pin_out("out", "", PinType::RuntimeBool)]),
        |c| {
            let n = declare(c, ParamType::Bool);
            c.alias("out", Operand::Value(format!("(params.{n} != 0u)")));
        },
    ));

    let b = def("param::CompileBool", "CompileBoolParameter", CAT_PARAM)
        .kw(&["compileBool", "compileFlag", "static switch", "feature", "permutation", "instance", "parameter"])
        .desc(
            "Named compile-time boolean parameter (`CompileBool`) used with `If` (`CompileAnd`, `CompileOr`, `CompileNot`). Evaluated at compile time to strip the inactive branch from the generated WESL shader (uses no GPU uniform slot). Overriding this flag in a future Material Instance will compile a separate WESL shader variant.\n\nNote: Cannot be toggled dynamically per frame without shader compilation. For dynamic in-game switches, use `RuntimeBoolParameter` with `RuntimeIf`.",
        )
        .role(NodeRole::Parameter)
        .param(ParamDef::new("name", "Name", ParamKind::Str, Value::Str("flag".into())))
        .param(ParamDef::new("default", "Default", ParamKind::Bool, Value::Bool(false)));
    r.register(b.build(
        |_| sig(vec![], vec![pin_out("out", "", PinType::CompileBool)]),
        |c| {
            let name = to_snake_case(c.param("name").as_str());
            let default = c.param("default").as_bool();
            c.declare_compile_flag(name, default);
        },
    ));

    let mut b = def("param::TextureObject", "TextureObjectParameter", CAT_PARAM)
        .kw(&["texture object", "texture parameter", "tex"])
        .desc("Texture handle set from the game; connect to TextureSample.")
        .role(NodeRole::Parameter);
    for p in common_params("texture") {
        b = b.param(p);
    }
    b = b.param(ParamDef::new("default", "Default Texture", ParamKind::Texture, Value::Str(String::new()))).action(
        texture_convert("Convert to Texture Object", "stdlib::TextureObject", ("default", TEXTURE_PARAM), &[], false),
    );
    r.register(b.build(
        |_| sig(vec![], vec![pin_out("out", "", PinType::Texture2D)]),
        |c| {
            let n = declare(c, ParamType::Texture);
            c.alias("out", Operand::Texture { tex: n.clone(), sampler: format!("{n}_sampler") });
        },
    ));

    let mut b = def("param::TextureSample2D", "TextureSampleParameter2D", CAT_PARAM)
        .kw(&["texture sample parameter", "texture parameter", "albedo", "tex"])
        .role(NodeRole::Parameter);
    for p in common_params("texture") {
        b = b.param(p);
    }
    b = b
        .param(ParamDef::new("default", "Default Texture", ParamKind::Texture, Value::Str(String::new())))
        .param(crate::nodes::texture::mip_mode_param())
        .param(crate::nodes::texture::sampler_type_param())
        .action(texture_convert(
            "Convert to Texture Sample",
            "stdlib::TextureSample",
            ("default", TEXTURE_PARAM),
            SAMPLE_PARAMS,
            false,
        ));
    r.register(b.build(
        |c| sig(crate::nodes::texture::sample_inputs(c), crate::nodes::texture::sample_outputs()),
        |c| {
            let n = declare(c, ParamType::Texture);
            crate::nodes::texture::emit_sample(c, &n, &format!("{n}_sampler"));
        },
    ));
}

fn vector_type(v: &Value) -> PinType {
    match v.as_type() {
        Some(t @ (PinType::Float2 | PinType::Float3 | PinType::Float4)) => t,
        _ => PinType::Float3,
    }
}

/// Outputs `rgba, rgb, r, g, b, a` of color-like nodes.
pub fn color_outputs() -> Vec<shader_graph_core::PinDef> {
    vec![
        pin_out("rgb", "RGB", PinType::Float3),
        pin_out("r", "R", PinType::Float),
        pin_out("g", "G", PinType::Float),
        pin_out("b", "B", PinType::Float),
        pin_out("a", "A", PinType::Float),
        pin_out("rgba", "RGBA", PinType::Color),
    ]
}

pub fn alias_color_outputs(c: &mut shader_graph_core::EmitCtx, v: &str) {
    c.alias("rgba", Operand::Value(v.to_string()));
    c.alias("rgb", Operand::Value(format!("{v}.rgb")));
    for ch in ["r", "g", "b", "a"] {
        c.alias(ch, Operand::Value(format!("{v}.{ch}")));
    }
}
