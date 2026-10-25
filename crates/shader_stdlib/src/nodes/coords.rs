//! Coordinates category: values from `VertexOutput` / view bindings.

use shader_graph_core::registry::{Builtin, Operand, ParamDef, ParamKind, Registry};
use shader_graph_core::{Code, PinType, Stage, Value};

use crate::def::{def, pin_in, pin_out, sig};

const CAT: &str = "Coordinates";

pub fn register(r: &mut Registry) {
    r.register(
        def("stdlib::TextureCoordinate", "TextureCoordinate", CAT)
            .kw(&["uv", "texcoord", "uv0", "uv1", "tiling"])
            .param(ParamDef::new(
                "index",
                "Coordinate Index",
                ParamKind::Enum(vec!["0".into(), "1".into()]),
                Value::Str("0".into()),
            ))
            .param(ParamDef::new("tiling", "Tiling", ParamKind::Float2, Value::Float2([1.0, 1.0])))
            .build(
                |_| sig(vec![], vec![pin_out("out", "", PinType::Float2)]),
                |c| {
                    let b = if c.param("index").as_str() == "1" { Builtin::Uv1 } else { Builtin::Uv0 };
                    let uv = c.builtin(b);
                    let tiling = c.param("tiling").coerce(PinType::Float2);
                    if tiling == Value::Float2([1.0, 1.0]) {
                        c.alias("out", Operand::Value(uv));
                    } else {
                        c.define("out", format!("({uv} * {})", tiling.wesl_literal(PinType::Float2)));
                    }
                },
            ),
    );

    for (id, name, b, ty, kw) in [
        (
            "WorldPosition",
            "WorldPosition",
            Builtin::WorldPosition,
            PinType::Float3,
            &["absolute world position", "position"][..],
        ),
        ("VertexNormalWS", "VertexNormalWS", Builtin::WorldNormal, PinType::Float3, &["normal", "world normal"][..]),
        ("CameraPositionWS", "CameraPositionWS", Builtin::CameraPosition, PinType::Float3, &["camera", "eye"][..]),
    ] {
        r.register(def(&format!("stdlib::{id}"), name, CAT).kw(kw).build(
            move |_| sig(vec![], vec![pin_out("out", "", ty)]),
            move |c| {
                let v = c.builtin(b);
                c.alias("out", Operand::Value(v));
            },
        ));
    }

    r.register(def("stdlib::VertexTangentWS", "VertexTangentWS", CAT).kw(&["tangent"]).build(
        |_| sig(vec![], vec![pin_out("out", "", PinType::Float3)]),
        |c| {
            let t = c.builtin(Builtin::WorldTangent);
            c.alias("out", Operand::Value(format!("{t}.xyz")));
        },
    ));

    r.register(def("stdlib::VertexColor", "VertexColor", CAT).kw(&["color", "vertex color", "rgba"]).build(
        |_| sig(vec![], crate::nodes::constants::color_outputs()),
        |c| {
            let v = c.builtin(Builtin::VertexColor);
            crate::nodes::constants::alias_color_outputs(c, &v);
        },
    ));

    r.register(def("stdlib::ObjectPosition", "ObjectPosition", CAT).kw(&["object", "pivot", "actor position"]).build(
        |_| sig(vec![], vec![pin_out("out", "", PinType::Float3)]),
        |c| {
            let m = c.builtin(Builtin::WorldFromLocal);
            c.alias("out", Operand::Value(format!("{m}[3].xyz")));
        },
    ));

    r.register(def("stdlib::ObjectScale", "ObjectScale", CAT).kw(&["scale", "object scale"]).build(
        |_| sig(vec![], vec![pin_out("out", "", PinType::Float3)]),
        |c| {
            let m = c.builtin(Builtin::WorldFromLocal);
            c.define("out", format!("vec3<f32>(length({m}[0].xyz), length({m}[1].xyz), length({m}[2].xyz))"));
        },
    ));

    r.register(
        def("stdlib::CameraVectorWS", "CameraVectorWS", CAT).kw(&["view direction", "camera vector", "v"]).build(
            |_| sig(vec![], vec![pin_out("out", "", PinType::Float3)]),
            |c| {
                let (cam, pos) = (c.builtin(Builtin::CameraPosition), c.builtin(Builtin::WorldPosition));
                c.define("out", format!("normalize({cam} - {pos})"));
            },
        ),
    );

    r.register(
        def("stdlib::ScreenPosition", "ScreenPosition", CAT)
            .kw(&["screen", "viewport uv", "pixel position", "frag coord"])
            .build(
                |_| {
                    sig(
                        vec![],
                        vec![
                            pin_out("viewport_uv", "ViewportUV", PinType::Float2),
                            pin_out("pixel", "PixelPosition", PinType::Float2),
                        ],
                    )
                },
                |c| {
                    if c.stage == Stage::Vertex {
                        c.diag(Code::E009, "ScreenPosition is only available in the fragment stage");
                    }
                    let (fc, size) = (c.builtin(Builtin::FragCoord), c.builtin(Builtin::ViewSize));
                    c.alias("pixel", Operand::Value(format!("{fc}.xy")));
                    c.define("viewport_uv", format!("({fc}.xy / {size})"));
                },
            ),
    );

    r.register(def("stdlib::PixelDepth", "PixelDepth", CAT).kw(&["depth", "view depth", "distance"]).build(
        |_| sig(vec![], vec![pin_out("out", "", PinType::Float)]),
        |c| {
            let (m, p) = (c.builtin(Builtin::ViewFromWorld), c.builtin(Builtin::WorldPosition));
            c.define("out", format!("-({m} * vec4<f32>({p}, 1.0)).z"));
        },
    ));

    r.register(def("stdlib::Panner", "Panner", CAT).kw(&["pan", "scroll", "move uv", "scroll uv"]).build(
        |_| {
            sig(
                vec![
                    pin_in("coordinate", "Coordinate", PinType::Float2, 0.0).implicit("uv0"),
                    pin_in("time", "Time", PinType::Float, 0.0).implicit("time"),
                    pin_in("speed", "Speed", PinType::Float2, 0.0).default(Value::Float2([0.1, 0.0])),
                ],
                vec![pin_out("out", "", PinType::Float2)],
            )
        },
        |c| {
            let uv = implicit_uv(c, "coordinate");
            let t = implicit_time(c, "time");
            c.define("out", format!("({uv} + {} * {t})", c.input("speed")));
        },
    ));

    r.register(def("stdlib::Rotator", "Rotator", CAT).kw(&["rotate", "spin", "rotate uv"]).build(
        |_| {
            sig(
                vec![
                    pin_in("coordinate", "Coordinate", PinType::Float2, 0.0).implicit("uv0"),
                    pin_in("time", "Time", PinType::Float, 0.0).implicit("time"),
                    pin_in("center", "Center", PinType::Float2, 0.5),
                    pin_in("speed", "Speed", PinType::Float, 0.25),
                ],
                vec![pin_out("out", "", PinType::Float2)],
            )
        },
        |c| {
            let uv = implicit_uv(c, "coordinate");
            let t = implicit_time(c, "time");
            let f = c.stdlib("uv", "sg_rotate_uv");
            c.define("out", format!("{f}({uv}, {}, {} * {t})", c.input("center"), c.input("speed")));
        },
    ));

    r.register(def("stdlib::ScaleUVsByCenter", "ScaleUVsByCenter", CAT).kw(&["scale uv", "zoom uv"]).build(
        |_| {
            sig(
                vec![
                    pin_in("uv", "UVs", PinType::Float2, 0.0).implicit("uv0"),
                    pin_in("scale", "Scale", PinType::Float2, 1.0),
                ],
                vec![pin_out("out", "", PinType::Float2)],
            )
        },
        |c| {
            let uv = implicit_uv(c, "uv");
            c.define("out", format!("(({uv} - vec2<f32>(0.5)) * {} + vec2<f32>(0.5))", c.input("scale")));
        },
    ));
}

/// Input value or `uv0` when unconnected.
pub fn implicit_uv(c: &mut shader_graph_core::EmitCtx, pin: &str) -> String {
    if c.is_connected(pin) { c.input(pin) } else { c.builtin(Builtin::Uv0) }
}

/// Input value or `Time` when unconnected.
pub fn implicit_time(c: &mut shader_graph_core::EmitCtx, pin: &str) -> String {
    if c.is_connected(pin) { c.input(pin) } else { c.builtin(Builtin::Time) }
}
