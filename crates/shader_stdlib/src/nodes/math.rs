//! Math category: per-component polymorphic operations (spec §8, 7.2).

use shader_graph_core::registry::Registry;
use shader_graph_core::{PinType, Stage};

use crate::def::{def, dim_fn, pin_in, pin_out, poly_ins, poly_out, sig};

const CAT: &str = "Math";

type Expr2 = fn(&str, &str, PinType) -> String;
type Expr1 = fn(&str, PinType) -> String;
/// (id, name, keywords, default a, default b, expression)
type BinaryOp<'a> = (&'a str, &'a str, &'a [&'a str], f32, f32, Expr2);

pub fn register(r: &mut Registry) {
    let binary: &[BinaryOp] = &[
        ("Add", "Add", &["a", "+", "plus", "sum"], 0.0, 1.0, |a, b, _| format!("({a} + {b})")),
        ("Subtract", "Subtract", &["s", "-", "minus", "sub"], 0.0, 1.0, |a, b, _| format!("({a} - {b})")),
        ("Multiply", "Multiply", &["m", "*", "mul", "times"], 0.0, 1.0, |a, b, _| format!("({a} * {b})")),
        ("Divide", "Divide", &["d", "/", "div"], 1.0, 1.0, |a, b, _| format!("({a} / {b})")),
        ("Power", "Power", &["pow", "^", "exponent"], 1.0, 2.0, |a, b, _| format!("pow({a}, {b})")),
        ("Min", "Min", &["minimum"], 0.0, 0.0, |a, b, _| format!("min({a}, {b})")),
        ("Max", "Max", &["maximum"], 0.0, 0.0, |a, b, _| format!("max({a}, {b})")),
        ("Fmod", "Fmod", &["mod", "%", "modulo", "remainder"], 1.0, 1.0, |a, b, _| format!("({a} % {b})")),
        ("Step", "Step", &["threshold", "edge"], 0.5, 0.0, |y, x, _| format!("step({y}, {x})")),
        ("ArcTan2", "ArcTan2", &["atan2", "angle"], 0.0, 1.0, |y, x, _| format!("atan2({y}, {x})")),
    ];
    for &(id, name, kw, da, db, f) in binary {
        let (la, lb) = match id {
            "Step" => ("Y", "X"),
            "ArcTan2" => ("Y", "X"),
            "Power" => ("Base", "Exp"),
            _ => ("A", "B"),
        };
        r.register(def(&format!("stdlib::{id}"), name, CAT).kw(kw).build(
            move |c| {
                let (ins, ty, _) = poly_ins(c, &[("a", la, da), ("b", lb, db)]);
                sig(ins, vec![poly_out("out", ty)])
            },
            move |c| {
                let ty = c.out_type("out");
                let e = f(&c.input("a"), &c.input("b"), ty);
                c.define("out", e);
            },
        ));
    }

    let unary: &[(&str, &str, &[&str], Expr1)] = &[
        ("Sqrt", "Sqrt", &["square root"], |x, _| format!("sqrt({x})")),
        ("Abs", "Abs", &["absolute"], |x, _| format!("abs({x})")),
        ("Floor", "Floor", &[], |x, _| format!("floor({x})")),
        ("Ceil", "Ceil", &["ceiling"], |x, _| format!("ceil({x})")),
        ("Frac", "Frac", &["fract", "fraction"], |x, _| format!("fract({x})")),
        ("Sign", "Sign", &[], |x, _| format!("sign({x})")),
        ("Saturate", "Saturate", &["clamp01", "clamp 0 1"], |x, _| format!("saturate({x})")),
        ("OneMinus", "OneMinus", &["1-x", "invert", "1-"], |x, t| format!("({} - {x})", one(t))),
        ("Exp", "Exp", &[], |x, _| format!("exp({x})")),
        ("Exp2", "Exp2", &[], |x, _| format!("exp2({x})")),
        ("Log", "Log", &["ln"], |x, _| format!("log({x})")),
        ("Log2", "Log2", &[], |x, _| format!("log2({x})")),
        ("Sin", "Sine", &["sin", "sine"], |x, _| format!("sin({x})")),
        ("Cos", "Cosine", &["cos", "cosine"], |x, _| format!("cos({x})")),
        ("Tan", "Tangent", &["tan"], |x, _| format!("tan({x})")),
        ("ArcSin", "ArcSine", &["asin", "arcsin"], |x, _| format!("asin({x})")),
        ("ArcCos", "ArcCosine", &["acos", "arccos"], |x, _| format!("acos({x})")),
        ("ArcTan", "ArcTangent", &["atan", "arctan"], |x, _| format!("atan({x})")),
        ("DDX", "DDX", &["dpdx", "derivative"], |x, _| format!("dpdx({x})")),
        ("DDY", "DDY", &["dpdy", "derivative"], |x, _| format!("dpdy({x})")),
    ];
    for &(id, name, kw, f) in unary {
        let fragment_only = matches!(id, "DDX" | "DDY");
        // log(0) is out of range (naga folds constant arguments and rejects it).
        let default = if matches!(id, "Log" | "Log2") { 1.0 } else { 0.0 };
        r.register(def(&format!("stdlib::{id}"), name, CAT).kw(kw).build(
            move |c| {
                let (ins, ty, _) = poly_ins(c, &[("x", "", default)]);
                sig(ins, vec![poly_out("out", ty)])
            },
            move |c| {
                if fragment_only && c.stage == Stage::Vertex {
                    c.diag(shader_graph_core::Code::E009, "derivatives are only available in the fragment stage");
                }
                let ty = c.out_type("out");
                let e = f(&c.input("x"), ty);
                c.define("out", e);
            },
        ));
    }

    for (id, name, kw, func) in [
        ("Round", "Round", &["rint", "nearest"][..], "round"),
        ("Truncate", "Truncate", &["trunc", "int", "to int"][..], "trunc"),
    ] {
        r.register(
            def(&format!("stdlib::{id}"), name, CAT)
                .kw(kw)
                .desc("Float output keeps the input type; `Int` output converts the first component.")
                .build(
                    |c| {
                        let (ins, ty, _) = poly_ins(c, &[("x", "", 0.0)]);
                        sig(ins, vec![poly_out("out", ty), pin_out("int", "Int", PinType::Int)])
                    },
                    move |c| {
                        let x = c.input("x");
                        let ty = c.out_type("out");
                        c.define("out", format!("{func}({x})"));
                        let first = if ty == PinType::Float { c.var("out") } else { format!("{}.x", c.var("out")) };
                        c.define("int", format!("i32({first})"));
                    },
                ),
        );
    }

    r.register(def("stdlib::Clamp", "Clamp", CAT).kw(&["limit"]).build(
        |c| {
            let (ins, ty, _) = poly_ins(c, &[("x", "", 0.0), ("min", "Min", 0.0), ("max", "Max", 1.0)]);
            sig(ins, vec![poly_out("out", ty)])
        },
        |c| {
            let e = format!("clamp({}, {}, {})", c.input("x"), c.input("min"), c.input("max"));
            c.define("out", e);
        },
    ));

    r.register(def("stdlib::Lerp", "Lerp", CAT).kw(&["l", "mix", "linear interpolate", "blend"]).build(
        |c| {
            let (mut ins, ty, _) = poly_ins(c, &[("a", "A", 0.0), ("b", "B", 1.0)]);
            // Alpha may be a scalar or match A/B.
            let alpha_ty = match c.connected.get("alpha") {
                Some(t) if t.dim().unwrap_or(1) > 1 => ty,
                _ => PinType::Float,
            };
            ins.push(pin_in("alpha", "Alpha", alpha_ty, 0.5).range(0.0, 1.0));
            sig(ins, vec![poly_out("out", ty)])
        },
        |c| {
            let e = format!("mix({}, {}, {})", c.input("a"), c.input("b"), c.input("alpha"));
            c.define("out", e);
        },
    ));

    r.register(def("stdlib::SmoothStep", "SmoothStep", CAT).kw(&["smooth", "hermite"]).build(
        |c| {
            let (ins, ty, _) = poly_ins(c, &[("min", "Min", 0.0), ("max", "Max", 1.0), ("x", "Value", 0.0)]);
            sig(ins, vec![poly_out("out", ty)])
        },
        |c| {
            let e = format!("smoothstep({}, {}, {})", c.input("min"), c.input("max"), c.input("x"));
            c.define("out", e);
        },
    ));

    r.register(
        def("stdlib::Remap", "Remap", CAT)
            .kw(&["remap value range", "map range", "fit", "range"])
            .desc("Maps Value from ValueRange (in min/max) to TargetRange (out min/max).")
            .build(
                |c| {
                    let (mut ins, ty, _) = poly_ins(c, &[("x", "Value", 0.0)]);
                    ins.extend([
                        pin_in("in_min", "In Min", PinType::Float, 0.0),
                        pin_in("in_max", "In Max", PinType::Float, 1.0),
                        pin_in("out_min", "Out Min", PinType::Float, 0.0),
                        pin_in("out_max", "Out Max", PinType::Float, 1.0),
                    ]);
                    sig(ins, vec![poly_out("out", ty)])
                },
                |c| {
                    let f = c.stdlib("math", &dim_fn("sg_remap", c.out_type("out")));
                    let e = format!(
                        "{f}({}, {}, {}, {}, {})",
                        c.input("x"),
                        c.input("in_min"),
                        c.input("in_max"),
                        c.input("out_min"),
                        c.input("out_max")
                    );
                    c.define("out", e);
                },
            ),
    );
}

/// `1.0` literal of the given float type.
pub fn one(t: PinType) -> String {
    match t {
        PinType::Float => "1.0".into(),
        t => format!("{}(1.0)", t.wesl()),
    }
}
