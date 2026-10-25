//! Vector category.

use shader_graph_core::registry::{Builtin, ParamDef, ParamKind, Registry};
use shader_graph_core::{Code, PinDef, PinType, SigCtx, Value};

use crate::def::{def, pin_in, pin_out, poly_ins, poly_out, sig};

const CAT: &str = "Vector";
const XYZW: [&str; 4] = ["x", "y", "z", "w"];

/// Type of a connected float-like input, or `fallback`.
fn connected_float(c: &shader_graph_core::SigCtx, pin: &str, fallback: PinType) -> PinType {
    match c.connected.get(pin) {
        Some(PinType::Int) => PinType::Float,
        Some(PinType::Color) => PinType::Float4,
        Some(t) if t.dim().is_some() => *t,
        _ => fallback,
    }
}

pub fn register(r: &mut Registry) {
    r.register(def("stdlib::Normalize", "Normalize", CAT).kw(&["unit", "normalise"]).build(
        |c| {
            let (ins, ty, _) = poly_ins(c, &[("x", "", 0.0)]);
            sig(ins, vec![poly_out("out", ty)])
        },
        |c| {
            let x = c.input("x");
            let e = if c.out_type("out") == PinType::Float { format!("sign({x})") } else { format!("normalize({x})") };
            c.define("out", e);
        },
    ));

    r.register(def("stdlib::Length", "Length", CAT).kw(&["magnitude"]).build(
        |c| {
            let (ins, _, _) = poly_ins(c, &[("x", "", 0.0)]);
            sig(ins, vec![pin_out("out", "", PinType::Float)])
        },
        |c| {
            let e = format!("length({})", c.input("x"));
            c.define("out", e);
        },
    ));

    r.register(def("stdlib::Distance", "Distance", CAT).kw(&["dist"]).build(
        |c| {
            let (ins, _, _) = poly_ins(c, &[("a", "A", 0.0), ("b", "B", 0.0)]);
            sig(ins, vec![pin_out("out", "", PinType::Float)])
        },
        |c| {
            let e = format!("distance({}, {})", c.input("a"), c.input("b"));
            c.define("out", e);
        },
    ));

    r.register(def("stdlib::DotProduct", "DotProduct", CAT).kw(&["dot"]).build(
        |c| {
            let (ins, _, _) = poly_ins(c, &[("a", "A", 0.0), ("b", "B", 0.0)]);
            sig(ins, vec![pin_out("out", "", PinType::Float)])
        },
        |c| {
            let (a, b) = (c.input("a"), c.input("b"));
            let e = if c.in_type("a") == PinType::Float { format!("({a} * {b})") } else { format!("dot({a}, {b})") };
            c.define("out", e);
        },
    ));

    r.register(def("stdlib::CrossProduct", "CrossProduct", CAT).kw(&["cross"]).build(
        |_| {
            sig(
                vec![pin_in("a", "A", PinType::Float3, 0.0), pin_in("b", "B", PinType::Float3, 0.0)],
                vec![pin_out("out", "", PinType::Float3)],
            )
        },
        |c| {
            let e = format!("cross({}, {})", c.input("a"), c.input("b"));
            c.define("out", e);
        },
    ));

    r.register(def("stdlib::Reflect", "ReflectionVector", CAT).kw(&["reflect"]).build(
        |_| {
            sig(
                vec![
                    pin_in("v", "Vector", PinType::Float3, 0.0),
                    pin_in("n", "Normal", PinType::Float3, 0.0).default(Value::Float3([0.0, 0.0, 1.0])),
                ],
                vec![pin_out("out", "", PinType::Float3)],
            )
        },
        |c| {
            let e = format!("reflect({}, {})", c.input("v"), c.input("n"));
            c.define("out", e);
        },
    ));

    r.register(def("stdlib::Refract", "Refract", CAT).kw(&["refraction", "ior"]).build(
        |_| {
            sig(
                vec![
                    pin_in("v", "Vector", PinType::Float3, 0.0),
                    pin_in("n", "Normal", PinType::Float3, 0.0).default(Value::Float3([0.0, 0.0, 1.0])),
                    pin_in("eta", "Eta", PinType::Float, 0.66),
                ],
                vec![pin_out("out", "", PinType::Float3)],
            )
        },
        |c| {
            let e = format!("refract({}, {}, {})", c.input("v"), c.input("n"), c.input("eta"));
            c.define("out", e);
        },
    ));

    r.register(def("stdlib::AppendVector", "AppendVector", CAT).kw(&["append", "combine", "concat"]).build(
        |c| {
            let ta = connected_float(c, "a", PinType::Float);
            let tb = connected_float(c, "b", PinType::Float);
            let dim = (ta.dim().unwrap_or(1) + tb.dim().unwrap_or(1)).min(4);
            sig(
                vec![pin_in("a", "A", ta, 0.0), pin_in("b", "B", tb, 0.0)],
                vec![pin_out("out", "", PinType::float_of_dim(dim))],
            )
        },
        |c| {
            let (ta, tb) = (c.in_type("a"), c.in_type("b"));
            if ta.dim().unwrap_or(1) + tb.dim().unwrap_or(1) > 4 {
                c.diag(Code::E003, "AppendVector result would have more than 4 components");
            }
            let ty = c.out_type("out");
            let e = format!("{}({}, {})", ty.wesl(), c.input("a"), c.input("b"));
            c.define("out", e);
        },
    ));

    let mut mask = def("stdlib::ComponentMask", "ComponentMask", CAT).kw(&["mask", "channels", "rgba"]);
    for (i, ch) in ["r", "g", "b", "a"].iter().enumerate() {
        mask = mask.param(ParamDef::new(ch, &ch.to_uppercase(), ParamKind::Bool, Value::Bool(i < 2)));
    }
    r.register(mask.build(
        |c| {
            let ins = unconnected_as_float4(c, poly_ins(c, &[("x", "", 0.0)]).0);
            let n = ["r", "g", "b", "a"].iter().filter(|ch| c.param(ch).as_bool()).count().max(1);
            sig(ins, vec![pin_out("out", "", PinType::float_of_dim(n as u8))])
        },
        |c| {
            let in_dim = c.in_type("x").dim().unwrap_or(1) as usize;
            let picked: Vec<usize> = (0..4).filter(|i| c.param(["r", "g", "b", "a"][*i]).as_bool()).collect();
            if picked.is_empty() {
                c.diag(Code::E003, "ComponentMask selects no components");
            }
            if picked.iter().any(|i| *i >= in_dim) {
                c.diag(Code::E003, "ComponentMask selects a component the input does not have");
            }
            let x = c.input("x");
            let e = if in_dim == 1 {
                if picked.len() <= 1 { x } else { format!("{}({x})", c.out_type("out").wesl()) }
            } else {
                let s: String = picked.iter().map(|i| ["x", "y", "z", "w"][*i]).collect();
                format!("({x}).{}", if s.is_empty() { "x".into() } else { s })
            };
            c.define("out", e);
        },
    ));

    for n in 2..=4u8 {
        let ty = PinType::float_of_dim(n);
        r.register(
            def(&format!("stdlib::BreakOutFloat{n}"), &format!("BreakOutFloat{n}Components"), CAT)
                .kw(&["break", "split", "components"])
                .build(
                    move |_| {
                        sig(
                            vec![pin_in("v", "", ty, 0.0)],
                            XYZW[..n as usize]
                                .iter()
                                .map(|ch| pin_out(ch, &ch.to_uppercase(), PinType::Float))
                                .collect(),
                        )
                    },
                    move |c| {
                        let v = c.input("v");
                        for ch in &XYZW[..n as usize] {
                            c.define(ch, format!("({v}).{ch}"));
                        }
                    },
                ),
        );
        r.register(
            def(&format!("stdlib::MakeFloat{n}"), &format!("MakeFloat{n}"), CAT)
                .kw(&["make", "construct", "compose", "vector"])
                .build(
                    move |_| {
                        sig(
                            XYZW[..n as usize]
                                .iter()
                                .map(|ch| pin_in(ch, &ch.to_uppercase(), PinType::Float, 0.0))
                                .collect(),
                            vec![pin_out("out", "", ty)],
                        )
                    },
                    move |c| {
                        let args: Vec<String> = XYZW[..n as usize].iter().map(|ch| c.input(ch)).collect();
                        c.define("out", format!("{}({})", ty.wesl(), args.join(", ")));
                    },
                ),
        );
    }

    r.register(
        def("stdlib::Swizzle", "Swizzle", CAT)
            .kw(&["swizzle", "reorder", "xyzw"])
            .param(ParamDef::new("pattern", "Pattern", ParamKind::Str, Value::Str("xyz".into())))
            .build(
                |c| {
                    let ins = unconnected_as_float4(c, poly_ins(c, &[("x", "", 0.0)]).0);
                    let len = c.param("pattern").as_str().len().clamp(1, 4) as u8;
                    sig(ins, vec![pin_out("out", "", PinType::float_of_dim(len))])
                },
                |c| {
                    let pat = c.param("pattern").as_str().to_lowercase();
                    let in_dim = c.in_type("x").dim().unwrap_or(1) as usize;
                    let valid = !pat.is_empty()
                        && pat.len() <= 4
                        && pat.chars().all(|ch| {
                            let idx = "xyzw".find(ch).or_else(|| "rgba".find(ch));
                            idx.is_some_and(|i| i < in_dim.max(1))
                        });
                    if !valid {
                        c.diag(Code::E003, format!("invalid swizzle `{pat}` for {}", c.in_type("x").label()));
                        c.define("out", c.out_type("out").wesl().to_string() + "(0.0)");
                        return;
                    }
                    let x = c.input("x");
                    let e = if in_dim == 1 {
                        if pat.len() == 1 { x } else { format!("{}({x})", c.out_type("out").wesl()) }
                    } else {
                        let norm: String = pat
                            .chars()
                            .map(|ch| "rgba".find(ch).map(|i| XYZW[i].chars().next().unwrap()).unwrap_or(ch))
                            .collect();
                        format!("({x}).{norm}")
                    };
                    c.define("out", e);
                },
            ),
    );

    let spaces = vec!["Tangent".to_string(), "Local".into(), "World".into(), "View".into()];
    r.register(
        def("stdlib::TransformVector", "TransformVector", CAT)
            .kw(&["transform", "space", "tangent to world", "local to world"])
            .param(ParamDef::new("from", "Source", ParamKind::Enum(spaces.clone()), Value::Str("Tangent".into())))
            .param(ParamDef::new("to", "Destination", ParamKind::Enum(spaces), Value::Str("World".into())))
            .build(
                |_| sig(vec![pin_in("v", "", PinType::Float3, 0.0)], vec![pin_out("out", "", PinType::Float3)]),
                |c| {
                    let (from, to) = (c.param("from").as_str().to_string(), c.param("to").as_str().to_string());
                    let mut v = c.input("v");
                    // Source → world.
                    v = match from.as_str() {
                        "Tangent" => {
                            let tbn = c.stdlib("transform", "sg_tbn");
                            let (n, t) = (c.builtin(Builtin::WorldNormal), c.builtin(Builtin::WorldTangent));
                            format!("({tbn}({n}, {t}) * {v})")
                        }
                        "Local" => {
                            let m3 = c.stdlib("transform", "sg_mat3");
                            let m = c.builtin(Builtin::WorldFromLocal);
                            format!("({m3}({m}) * {v})")
                        }
                        "View" => {
                            let m3 = c.stdlib("transform", "sg_mat3");
                            let m = c.builtin(Builtin::ViewFromWorld);
                            format!("(transpose({m3}({m})) * {v})")
                        }
                        _ => v,
                    };
                    // World → destination.
                    v = match to.as_str() {
                        "Tangent" => {
                            let tbn = c.stdlib("transform", "sg_tbn");
                            let (n, t) = (c.builtin(Builtin::WorldNormal), c.builtin(Builtin::WorldTangent));
                            format!("(transpose({tbn}({n}, {t})) * {v})")
                        }
                        "Local" => {
                            let inv = c.stdlib("transform", "sg_inverse_rotation");
                            let m = c.builtin(Builtin::WorldFromLocal);
                            format!("({inv}({m}) * {v})")
                        }
                        "View" => {
                            let m3 = c.stdlib("transform", "sg_mat3");
                            let m = c.builtin(Builtin::ViewFromWorld);
                            format!("({m3}({m}) * {v})")
                        }
                        _ => v,
                    };
                    c.define("out", v);
                },
            ),
    );
}

/// Masks and swizzles of an unconnected input read a zero `Float4`, so every channel exists.
fn unconnected_as_float4(c: &SigCtx, mut ins: Vec<PinDef>) -> Vec<PinDef> {
    if !c.any_connected(&["x"]) {
        for p in &mut ins {
            p.ty = PinType::Float4;
        }
    }
    ins
}
