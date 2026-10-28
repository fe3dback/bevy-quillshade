//! Color category (decision 0036): Make / Break of HSV, HSL, OKLab, OKLCh, sRGB transfer, adjustments, filters,
//! Blend, BlackBody. Colors are linear `Float3` in and out; the work is done in `wesl/color.wesl`. Sample Gradient
//! (gradient design §6) reads a gradient collection's baked texture.

use shader_graph_core::gradient::SLOTS;
use shader_graph_core::registry::{Operand, ParamDef, ParamKind, PinDef, Registry};
use shader_graph_core::value::wesl_scalar;
use shader_graph_core::{Code, EmitCtx, PinType, Value};

use crate::def::{def, pin_in, pin_out, sig};

const CAT: &str = "Color";
const F3: PinType = PinType::Float3;

/// Node sampling a gradient of a gradient collection (gradient design §6).
pub const SAMPLE_GRADIENT_KIND: &str = "stdlib::SampleGradient";
/// Property of [`SAMPLE_GRADIENT_KIND`]: the gradient collection id, `""` = none.
pub const GRADIENT_COLLECTION_PARAM: &str = "collection";
/// Property of [`SAMPLE_GRADIENT_KIND`]: the slot id of the gradient in the collection, `0` = none.
pub const GRADIENT_SLOT_PARAM: &str = "gradient";

/// Component of a color space: id, label, default of the Make node.
type Comp = (&'static str, &'static str, f32);

/// Color spaces with Make / Break nodes: name, `wesl/color.wesl` suffix, components.
const SPACES: [(&str, &str, [Comp; 3]); 4] = [
    ("HSV", "hsv", [("h", "Hue", 0.0), ("s", "Saturation", 1.0), ("v", "Value", 1.0)]),
    ("HSL", "hsl", [("h", "Hue", 0.0), ("s", "Saturation", 1.0), ("l", "Lightness", 0.5)]),
    ("OKLab", "oklab", [("l", "L", 0.6), ("a", "A", 0.0), ("b", "B", 0.0)]),
    ("OKLCh", "oklch", [("l", "L", 0.6), ("c", "Chroma", 0.15), ("h", "Hue", 0.0)]),
];

fn color_in() -> PinDef {
    pin_in("color", "Color", F3, 0.5)
}

/// Node `color` + scalar inputs → `out = func(color, inputs…)`.
fn adjust(r: &mut Registry, name: &str, kw: &[&str], desc: &str, func: &'static str, ins: fn() -> Vec<PinDef>) {
    r.register(def(&format!("stdlib::{name}"), name, CAT).kw(kw).desc(desc).build(
        move |_| {
            let mut pins = vec![color_in()];
            pins.extend(ins());
            sig(pins, vec![pin_out("out", "", F3)])
        },
        move |c| {
            let f = c.stdlib("color", func);
            let args: Vec<String> = c.sig.inputs.iter().map(|p| c.input(&p.id)).collect();
            c.define("out", format!("{f}({})", args.join(", ")));
        },
    ));
}

pub fn register(r: &mut Registry) {
    for (space, sfx, comps) in SPACES {
        r.register(
            def(&format!("stdlib::Make{space}"), &format!("Make{space}"), CAT)
                .kw(&["make", "compose", "color space", "to rgb", sfx])
                .desc(&format!("Linear RGB color from {space} components (hues in turns, 0..1)."))
                .build(
                    move |_| {
                        let ins = comps.iter().map(|(id, label, d)| pin_in(id, label, PinType::Float, *d)).collect();
                        sig(ins, vec![pin_out("out", "", F3)])
                    },
                    move |c| {
                        let f = c.stdlib("color", &format!("sg_{sfx}_to_rgb"));
                        let v: Vec<String> = comps.iter().map(|(id, ..)| c.input(id)).collect();
                        c.define("out", format!("{f}(vec3<f32>({}))", v.join(", ")));
                    },
                ),
        );
        r.register(
            def(&format!("stdlib::Break{space}"), &format!("Break{space}"), CAT)
                .kw(&["break", "split", "color space", "from rgb", sfx])
                .desc(&format!("{space} components of a linear RGB color (hues in turns, 0..1)."))
                .build(
                    move |_| {
                        let outs = comps.iter().map(|(id, label, _)| pin_out(id, label, PinType::Float)).collect();
                        sig(vec![color_in()], outs)
                    },
                    move |c| {
                        let f = c.stdlib("color", &format!("sg_rgb_to_{sfx}"));
                        c.define_typed("v_", F3.wesl(), format!("{f}({})", c.input("color")));
                        let v = c.var("v_");
                        for ((id, ..), ch) in comps.iter().zip(["x", "y", "z"]) {
                            c.alias(id, Operand::Value(format!("{v}.{ch}")));
                        }
                    },
                ),
        );
    }

    for (name, func, from, to) in [
        ("LinearToSRGB", "sg_linear_to_srgb", "linear", "sRGB-encoded"),
        ("SRGBToLinear", "sg_srgb_to_linear", "sRGB-encoded", "linear"),
    ] {
        let desc = format!("Converts a {from} color to {to} values (sRGB transfer function).");
        adjust(r, name, &["srgb", "linear", "gamma", "encode", "decode"], &desc, func, Vec::new);
    }

    adjust(
        r,
        "HueShift",
        &["hue", "shift", "rotate", "color variation"],
        "Rotates the hue (OKLCh) by Shift turns, keeping lightness and chroma.",
        "sg_hue_shift",
        || vec![pin_in("shift", "Shift", PinType::Float, 0.0).range(0.0, 1.0)],
    );
    adjust(
        r,
        "Saturation",
        &["saturation", "chroma", "vibrance"],
        "Scales the chroma (OKLCh): 0 is gray, 1 unchanged.",
        "sg_saturation",
        || vec![pin_in("saturation", "Saturation", PinType::Float, 1.0).range(0.0, 2.0)],
    );
    adjust(
        r,
        "Lightness",
        &["lightness", "brightness", "darken", "lighten"],
        "Perceived lightness (OKLab L) times Scale plus Offset, keeping hue and chroma.",
        "sg_lightness",
        || {
            vec![
                pin_in("scale", "Scale", PinType::Float, 1.0).range(0.0, 2.0),
                pin_in("offset", "Offset", PinType::Float, 0.0).range(-1.0, 1.0),
            ]
        },
    );
    adjust(
        r,
        "HueSaturationLightness",
        &["hsl", "hue", "saturation", "lightness", "adjust", "color variation"],
        "Hue shift (turns), chroma scale and lightness scale in one OKLCh conversion.",
        "sg_hue_saturation_lightness",
        || {
            vec![
                pin_in("shift", "Hue Shift", PinType::Float, 0.0).range(0.0, 1.0),
                pin_in("saturation", "Saturation", PinType::Float, 1.0).range(0.0, 2.0),
                pin_in("lightness", "Lightness", PinType::Float, 1.0).range(0.0, 2.0),
            ]
        },
    );
    adjust(
        r,
        "Colorize",
        &["colorize", "tint", "toning"],
        "Takes hue and chroma from Tint, keeping the lightness (OKLab), blended by Amount.",
        "sg_colorize",
        || {
            vec![
                PinDef::new("tint", "Tint", F3).default(Value::Float3([0.6, 0.3, 0.1])),
                pin_in("amount", "Amount", PinType::Float, 1.0).range(0.0, 1.0),
            ]
        },
    );
    adjust(
        r,
        "Contrast",
        &["contrast"],
        "Scales sRGB-encoded values around Pivot by Amount (1 = unchanged).",
        "sg_contrast",
        || {
            vec![
                pin_in("amount", "Amount", PinType::Float, 1.0).range(0.0, 2.0),
                pin_in("pivot", "Pivot", PinType::Float, 0.5).range(0.0, 1.0),
            ]
        },
    );
    adjust(
        r,
        "Gamma",
        &["gamma", "midtones"],
        "Raises sRGB-encoded values to 1 / Gamma: above 1 brightens the midtones.",
        "sg_gamma",
        || vec![pin_in("gamma", "Gamma", PinType::Float, 1.0).range(0.1, 3.0)],
    );
    adjust(
        r,
        "Levels",
        &["levels", "remap", "black point", "white point"],
        "Photoshop Levels over sRGB-encoded values: input range, midtone gamma, output range.",
        "sg_levels",
        || {
            vec![
                pin_in("in_black", "In Black", PinType::Float, 0.0).range(0.0, 1.0),
                pin_in("in_white", "In White", PinType::Float, 1.0).range(0.0, 1.0),
                pin_in("gamma", "Gamma", PinType::Float, 1.0).range(0.1, 3.0),
                pin_in("out_black", "Out Black", PinType::Float, 0.0).range(0.0, 1.0),
                pin_in("out_white", "Out White", PinType::Float, 1.0).range(0.0, 1.0),
            ]
        },
    );

    r.register(
        def("stdlib::Exposure", "Exposure", CAT)
            .kw(&["exposure", "stops", "ev", "brightness"])
            .desc("Multiplies the linear color by 2^Stops.")
            .build(
                |_| {
                    sig(
                        vec![color_in(), pin_in("stops", "Stops", PinType::Float, 0.0).range(-4.0, 4.0)],
                        vec![pin_out("out", "", F3)],
                    )
                },
                |c| {
                    let e = format!("({} * exp2({}))", c.input("color"), c.input("stops"));
                    c.define("out", e);
                },
            ),
    );

    adjust(
        r,
        "Grayscale",
        &["grayscale", "greyscale", "monochrome", "black and white"],
        "Gray of the same perceived lightness (OKLab L).",
        "sg_grayscale",
        Vec::new,
    );

    r.register(
        def("stdlib::Desaturation", "Desaturation", CAT)
            .kw(&["grayscale", "greyscale", "saturation", "luminance"])
            .desc("Blends toward the weighted gray of the sRGB-encoded channels by Fraction.")
            .param(ParamDef::new("luminance", "Luminance Factors", ParamKind::Float3, Value::Float3([0.3, 0.59, 0.11])))
            .build(
                |_| {
                    sig(
                        vec![color_in(), pin_in("fraction", "Fraction", PinType::Float, 1.0).range(0.0, 1.0)],
                        vec![pin_out("out", "", F3)],
                    )
                },
                |c| {
                    let lum = c.param("luminance").wesl_literal(F3);
                    let f = c.stdlib("color", "sg_desaturation");
                    let e = format!("{f}({}, {}, {lum})", c.input("color"), c.input("fraction"));
                    c.define("out", e);
                },
            ),
    );

    adjust(
        r,
        "Sepia",
        &["sepia", "vintage", "old photo"],
        "Classic sepia tone of the sRGB-encoded color, blended by Amount.",
        "sg_sepia",
        || vec![pin_in("amount", "Amount", PinType::Float, 1.0).range(0.0, 1.0)],
    );
    adjust(r, "Invert", &["invert", "negative"], "Negative of the sRGB-encoded color.", "sg_invert", Vec::new);
    adjust(
        r,
        "Posterize",
        &["posterize", "quantize", "bands", "toon"],
        "Steps levels per sRGB-encoded channel (at least 2).",
        "sg_posterize",
        || vec![pin_in("steps", "Steps", PinType::Float, 4.0).range(2.0, 16.0)],
    );

    let modes: Vec<String> =
        ["Burn", "Darken", "Difference", "Dodge", "HardLight", "Lighten", "Multiply", "Overlay", "Screen", "SoftLight"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    r.register(
        def("stdlib::Blend", "Blend", CAT)
            .kw(&["blend", "overlay", "screen", "soft light", "burn", "dodge", "darken", "lighten", "photoshop"])
            .desc("Photoshop blend mode of Blend over Base on sRGB-encoded values, mixed in by Opacity.")
            .param(ParamDef::new("mode", "Mode", ParamKind::Enum(modes), Value::Str("Overlay".into())))
            .build(
                |_| {
                    sig(
                        vec![
                            pin_in("base", "Base", F3, 0.5),
                            pin_in("blend", "Blend", F3, 0.5),
                            pin_in("opacity", "Opacity", PinType::Float, 1.0).range(0.0, 1.0),
                        ],
                        vec![pin_out("out", "", F3)],
                    )
                },
                |c| {
                    let func = match c.param("mode").as_str() {
                        "Burn" => "sg_blend_burn",
                        "Darken" => "sg_blend_darken",
                        "Difference" => "sg_blend_difference",
                        "Dodge" => "sg_blend_dodge",
                        "HardLight" => "sg_blend_hard_light",
                        "Lighten" => "sg_blend_lighten",
                        "Multiply" => "sg_blend_multiply",
                        "Screen" => "sg_blend_screen",
                        "SoftLight" => "sg_blend_soft_light",
                        _ => "sg_blend_overlay",
                    };
                    let f = c.stdlib("color", func);
                    let (enc, dec) = (c.stdlib("color", "sg_linear_to_srgb"), c.stdlib("color", "sg_srgb_to_linear"));
                    c.define_typed("base_", F3.wesl(), format!("{enc}({})", c.input("base")));
                    let b = c.var("base_");
                    let e = format!("{dec}(mix({b}, {f}({b}, {enc}({})), {}))", c.input("blend"), c.input("opacity"));
                    c.define("out", e);
                },
            ),
    );

    r.register(
        def(SAMPLE_GRADIENT_KIND, "Sample Gradient", CAT)
            .kw(&["gradient", "ramp", "color curve", "palette", "lut", "atlas"])
            .desc(
                "Linear RGBA color of a gradient of a gradient collection at T (0..1). Without a gradient: white \
                 (W111).",
            )
            .param(ParamDef::new(
                GRADIENT_COLLECTION_PARAM,
                "Collection",
                ParamKind::GradientCollection,
                Value::Str(String::new()),
            ))
            .param(ParamDef::new(GRADIENT_SLOT_PARAM, "Gradient", ParamKind::GradientSlot, Value::Int(0)))
            .build(
                |_| {
                    // Output names and order of TextureSample (`sample_outputs`), minus the single channels.
                    sig(
                        vec![pin_in("t", "T", PinType::Float, 0.0).range(0.0, 1.0)],
                        vec![
                            pin_out("rgb", "RGB", F3),
                            pin_out("a", "A", PinType::Float),
                            pin_out("rgba", "RGBA", PinType::Float4),
                        ],
                    )
                },
                sample_gradient_emit,
            ),
    );

    r.register(def("stdlib::BlackBody", "BlackBody", CAT).kw(&["temperature", "kelvin", "fire"]).build(
        |_| sig(vec![pin_in("temp", "Temp", PinType::Float, 6500.0)], vec![pin_out("out", "", F3)]),
        |c| {
            let f = c.stdlib("color", "sg_blackbody");
            let e = format!("{f}({})", c.input("temp"));
            c.define("out", e);
        },
    ));
}

/// Sample Gradient (gradient design §6, §7). The row literal is the slot's current position, resolved from its id;
/// whether the slot has stops never changes the code, so editing colors (even emptying a slot) only rebakes the
/// collection's texture.
fn sample_gradient_emit(c: &mut EmitCtx) {
    let collection = c.param(GRADIENT_COLLECTION_PARAM).as_str().to_string();
    let slot = c.param(GRADIENT_SLOT_PARAM).as_i32();
    if collection.is_empty() || slot == 0 {
        c.diag(Code::W111, "no gradient selected; outputs white");
        define_gradient_outputs(c, "vec4<f32>(1.0)".to_string());
        return;
    }
    let env = c.env;
    let Some(coll) = env.gradients.collection(&collection) else {
        c.diag(Code::E006, format!("gradient collection `{collection}` does not exist or cannot be loaded"));
        return;
    };
    let Some(row) = u32::try_from(slot).ok().and_then(|id| coll.position(id)) else {
        c.diag(Code::E006, format!("gradient #{slot} is not in collection `{collection}`"));
        return;
    };
    if coll.slots[row].is_empty() {
        c.diag(Code::W111, "gradient is empty; outputs white");
    }
    let (tex, sampler) = c.gradient(&collection);
    let f = c.stdlib("texture", "sg_sample_gradient");
    // Texel centre of the row: `(row + 0.5) / SLOTS`, exact in f32.
    let v = wesl_scalar((row as f32 + 0.5) / SLOTS as f32);
    let t = c.input("t");
    define_gradient_outputs(c, format!("{f}({tex}, {sampler}, {t}, {v})"));
}

/// Defines `rgba` as `expr`; `rgb` / `a` are its swizzles, as in TextureSample.
fn define_gradient_outputs(c: &mut EmitCtx, expr: String) {
    c.define("rgba", expr);
    let v = c.var("rgba");
    c.alias("rgb", Operand::Value(format!("{v}.rgb")));
    c.alias("a", Operand::Value(format!("{v}.a")));
}
