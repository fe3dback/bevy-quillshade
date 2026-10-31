//! Texture category.

use shader_graph_core::registry::{
    Conversion, NodeAction, NodeOp, Operand, ParamDef, ParamKind, PinDef, Registry, texture_refs,
};
use shader_graph_core::{Asset, AssetKind, Code, EmitCtx, MaterialKind, PinType, Stage, Value};

use crate::def::{def, pin_in, pin_out, sig};
use crate::internal::{DEFAULT_TEXTURE, internal_texture};
use crate::nodes::coords::{implicit_time, implicit_uv};

const CAT: &str = "Texture";

/// Property of non-parameter texture nodes: the Texture asset they use (design §1.2).
pub const TEXTURE_PARAM: &str = "texture";

/// `texture` property: a Texture asset id, `""` = none (the default texture, W110).
pub fn texture_param() -> ParamDef {
    ParamDef::new(TEXTURE_PARAM, "Texture", ParamKind::Texture, Value::Str(String::new()))
}

/// `MipValueMode` of sampling nodes; anything but `None` adds the `mip` input.
pub fn mip_mode_param() -> ParamDef {
    ParamDef::new(
        "mip_mode",
        "MipValueMode",
        ParamKind::Enum(vec!["None".into(), "Bias".into(), "Level".into()]),
        Value::Str("None".into()),
    )
}

/// Inputs of sampling nodes after the texture: `uv` (uv0 when unconnected) and `mip` for a mip mode.
pub fn sample_inputs(c: &shader_graph_core::SigCtx) -> Vec<PinDef> {
    let mut ins = vec![pin_in("uv", "UVs", PinType::Float2, 0.0).implicit("uv0")];
    if c.param("mip_mode").as_str() != "None" {
        ins.push(pin_in("mip", "Mip", PinType::Float, 0.0));
    }
    ins
}

/// Resolves a texture reference (design §1.2): `""` → the default texture (W110); an id that is neither an internal
/// texture nor a Texture asset of the project → E006 and `None`.
pub fn resolve_texture(c: &mut EmitCtx, id: &str) -> Option<String> {
    if id.is_empty() {
        c.diag(Code::W110, "no texture selected; uses the default texture");
        return Some(DEFAULT_TEXTURE.to_string());
    }
    if internal_texture(id).is_some() || c.env.textures.has_texture(id) {
        return Some(id.to_string());
    }
    c.diag(Code::E006, format!("texture `{id}` does not exist"));
    None
}

pub fn sampler_type_param() -> ParamDef {
    ParamDef::new(
        "sampler_type",
        "Sampler Type",
        ParamKind::Enum(vec!["Color".into(), "Normal".into()]),
        Value::Str("Color".into()),
    )
}

/// TextureSample outputs in UE order: `rgb, r, g, b, a, rgba`.
pub fn sample_outputs() -> Vec<PinDef> {
    vec![
        pin_out("rgb", "RGB", PinType::Float3),
        pin_out("r", "R", PinType::Float),
        pin_out("g", "G", PinType::Float),
        pin_out("b", "B", PinType::Float),
        pin_out("a", "A", PinType::Float),
        pin_out("rgba", "RGBA", PinType::Float4),
    ]
}

/// Samples `tex`/`sampler` at the `uv` input (or uv0) and defines the sample outputs.
pub fn emit_sample(c: &mut EmitCtx, tex: &str, sampler: &str) {
    let uv = implicit_uv(c, "uv");
    let mode = c.param("mip_mode").as_str().to_string();
    let mip = if c.sig.input("mip").is_some() { c.input("mip") } else { "0.0".into() };
    let mut call = match (c.stage, mode.as_str()) {
        (Stage::Vertex, "Level") => format!("textureSampleLevel({tex}, {sampler}, {uv}, {mip})"),
        (Stage::Vertex, _) => format!("textureSampleLevel({tex}, {sampler}, {uv}, 0.0)"),
        (_, "Bias") => format!("textureSampleBias({tex}, {sampler}, {uv}, {mip})"),
        (_, "Level") => format!("textureSampleLevel({tex}, {sampler}, {uv}, {mip})"),
        _ => format!("textureSample({tex}, {sampler}, {uv})"),
    };
    if c.param("sampler_type").as_str() == "Normal" {
        let f = c.stdlib("texture", "sg_unpack_normal");
        call = format!("{f}({call})");
    }
    c.define("rgba", call);
    let v = c.var("rgba");
    c.alias("rgb", Operand::Value(format!("{v}.rgb")));
    for ch in ["r", "g", "b", "a"] {
        c.alias(ch, Operand::Value(format!("{v}.{ch}")));
    }
}

/// Rejects texture references stored as image paths by older files (`"textures/rock_albedo.png"`): Texture asset ids
/// never contain `.`. There is no migration (`format_version` stays 1); the error names the node.
pub fn check_texture_refs(asset: &Asset, registry: &Registry) -> Result<(), String> {
    for (node, param, id) in texture_refs(asset, registry) {
        if id.contains('.') {
            let kind = &asset.nodes[&node].kind;
            return Err(format!(
                "node `{node}` ({kind}): `{param}` is the image path `{id}`, but a texture is now a Texture asset \
                 reference; create a Texture asset for the image (`quillshade/material/**/*.texture.ron`) and select it"
            ));
        }
    }
    Ok(())
}

/// Texture of a node with a `texture` property and an optional `tex` pin: the connected pin overrides the property
/// (UE behaviour); otherwise the property is the node's fixed texture binding.
fn texture_or_property(c: &mut EmitCtx) -> Option<(String, String)> {
    if let Some(t) = c.input_texture("tex") {
        return Some(t);
    }
    let id = c.param(TEXTURE_PARAM).as_str().to_string();
    let id = resolve_texture(c, &id)?;
    match c.fixed_texture(&id) {
        Operand::Texture { tex, sampler } => Some((tex, sampler)),
        Operand::Value(_) | Operand::CompileBool(_) => None,
    }
}

fn texture_in(c: &mut EmitCtx) -> Option<(String, String)> {
    let t = c.input_texture("tex");
    if t.is_none() && !c.is_connected("tex") {
        // E004 is reported by the IR builder for the required pin.
        return None;
    }
    t
}

/// A context action converting between a texture node and its parameter version (design §4.6, decision 0021): the
/// texture reference moves between `from` and `to` (`texture` ↔ `default`), `keep` params are copied as they are,
/// every other param is dropped (a parameter's name / group / order going back). `to_param` gives the new parameter
/// a unique name (`texture`, `texture_1`, …).
pub fn texture_convert(label: &str, kind: &str, (from, to): (&str, &str), keep: &[&str], to_param: bool) -> NodeAction {
    let mut params = vec![(from.to_string(), to.to_string())];
    params.extend(keep.iter().map(|p| (p.to_string(), p.to_string())));
    NodeAction {
        label: label.to_string(),
        hotkey: None,
        op: NodeOp::Convert(Conversion {
            kind: kind.to_string(),
            params,
            unique_name: to_param.then(|| ("name".to_string(), TEXTURE_PARAM.to_string())),
            pins: Vec::new(),
        }),
    }
}

/// Params of sampling nodes kept by the TextureSample ↔ parameter conversion.
pub const SAMPLE_PARAMS: &[&str] = &["mip_mode", "sampler_type"];

pub fn register(r: &mut Registry) {
    r.register(
        def("stdlib::TextureSample", "TextureSample", CAT)
            .kw(&["t", "texture", "sample", "tex2d", "sampler"])
            .desc(
                "Samples the Texture property, or the texture connected to Tex (it wins). Without either: the default \
                 texture (W110). Unconnected UVs use uv0.",
            )
            .param(texture_param())
            .param(mip_mode_param())
            .param(sampler_type_param())
            .action(texture_convert(
                "Convert to Parameter",
                "param::TextureSample2D",
                (TEXTURE_PARAM, "default"),
                SAMPLE_PARAMS,
                true,
            ))
            .build(
                |c| {
                    let mut ins = vec![PinDef::new("tex", "Tex", PinType::Texture2D)];
                    ins.extend(sample_inputs(c));
                    sig(ins, sample_outputs())
                },
                |c| {
                    let Some((t, s)) = texture_or_property(c) else { return };
                    emit_sample(c, &t, &s);
                },
            ),
    );

    r.register(
        def("stdlib::TextureObject", "TextureObject", CAT)
            .kw(&["texture object", "tex"])
            .desc("A Texture asset of the project as a Texture2D (TextureSample, TextureSize, function inputs, …).")
            .param(texture_param())
            .action(texture_convert(
                "Convert to Parameter",
                "param::TextureObject",
                (TEXTURE_PARAM, "default"),
                &[],
                true,
            ))
            .build(
                |_| sig(vec![], vec![pin_out("out", "", PinType::Texture2D)]),
                |c| {
                    let id = c.param(TEXTURE_PARAM).as_str().to_string();
                    let Some(id) = resolve_texture(c, &id) else { return };
                    let op = c.fixed_texture(&id);
                    c.alias("out", op);
                },
            ),
    );

    r.register(def("stdlib::TextureSize", "TextureSize", CAT).kw(&["dimensions", "resolution"]).build(
        |_| {
            sig(
                vec![PinDef::new("tex", "Tex", PinType::Texture2D).required()],
                vec![pin_out("out", "", PinType::Float2)],
            )
        },
        |c| {
            let Some((t, _)) = texture_in(c) else { return };
            c.define("out", format!("vec2<f32>(textureDimensions({t}))"));
        },
    ));

    r.register(def("stdlib::TexelSize", "TexelSize", CAT).kw(&["texel", "pixel size"]).build(
        |_| {
            sig(
                vec![PinDef::new("tex", "Tex", PinType::Texture2D).required()],
                vec![pin_out("out", "", PinType::Float2)],
            )
        },
        |c| {
            let Some((t, _)) = texture_in(c) else { return };
            c.define("out", format!("(vec2<f32>(1.0) / vec2<f32>(textureDimensions({t})))"));
        },
    ));

    r.register(
        def("stdlib::FlipBook", "FlipBook", CAT)
            .kw(&["flipbook", "atlas", "sprite sheet", "animation"])
            .desc("Samples one cell of a texture atlas; Phase 0..1 selects the frame (defaults to fract(Time)).")
            .build(
                |_| {
                    let mut outs = sample_outputs();
                    outs.push(pin_out("uv", "UV", PinType::Float2));
                    sig(
                        vec![
                            PinDef::new("tex", "Texture", PinType::Texture2D).required(),
                            pin_in("rows", "Rows", PinType::Float, 4.0),
                            pin_in("columns", "Columns", PinType::Float, 4.0),
                            pin_in("phase", "Phase", PinType::Float, 0.0).implicit("fract(time)"),
                            pin_in("uv", "UVs", PinType::Float2, 0.0).implicit("uv0"),
                        ],
                        outs,
                    )
                },
                |c| {
                    let Some((t, s)) = texture_in(c) else { return };
                    let base_uv = implicit_uv(c, "uv");
                    let phase = if c.is_connected("phase") {
                        c.input("phase")
                    } else {
                        format!("fract({})", implicit_time(c, "phase"))
                    };
                    let f = c.stdlib("uv", "sg_flipbook_uv");
                    c.define("uv", format!("{f}({base_uv}, {}, {}, {phase})", c.input("rows"), c.input("columns")));
                    let uv = c.var("uv");
                    let call = if c.stage == Stage::Vertex {
                        format!("textureSampleLevel({t}, {s}, {uv}, 0.0)")
                    } else {
                        // Grad sampling avoids mip seams at cell borders.
                        format!("textureSampleGrad({t}, {s}, {uv}, dpdx({base_uv}), dpdy({base_uv}))")
                    };
                    c.define("rgba", call);
                    let v = c.var("rgba");
                    c.alias("rgb", Operand::Value(format!("{v}.rgb")));
                    for ch in ["r", "g", "b", "a"] {
                        c.alias(ch, Operand::Value(format!("{v}.{ch}")));
                    }
                },
            ),
    );

    r.register(
        def("stdlib::NormalFromHeightmap", "NormalFromHeightmap", CAT)
            .kw(&["height", "bump", "normal from height"])
            .desc("Tangent-space normal from a grayscale height texture (red channel).")
            .build(
                |_| {
                    sig(
                        vec![
                            PinDef::new("tex", "Heightmap", PinType::Texture2D).required(),
                            pin_in("uv", "UVs", PinType::Float2, 0.0).implicit("uv0"),
                            pin_in("scale", "Height Scale", PinType::Float, 1.0),
                        ],
                        vec![pin_out("out", "", PinType::Float3)],
                    )
                },
                |c| {
                    let Some((t, s)) = texture_in(c) else { return };
                    let uv = implicit_uv(c, "uv");
                    let f = c.stdlib("texture", "sg_normal_from_height");
                    c.define("out", format!("{f}({t}, {s}, {uv}, {})", c.input("scale")));
                },
            ),
    );

    r.register(
        def("stdlib::SpriteTexture", "SpriteTexture", CAT)
            .kw(&["sprite", "sprite texture", "2d", "sample sprite"])
            .desc("Sprite2d only: samples the sprite image (flip/slice/atlas applied), without tint.")
            .build(
                |_| sig(vec![pin_in("uv", "UVs", PinType::Float2, 0.0).implicit("uv0")], sample_outputs()),
                |c| {
                    if !matches!(c.env.asset_kind, AssetKind::Material(MaterialKind::Sprite2d(_))) {
                        c.diag(Code::E009, "SpriteTexture is only available in Sprite2d materials");
                        return;
                    }
                    let uv = implicit_uv(c, "uv");
                    let f = c.stdlib("@sprite", "sample_sprite_texture");
                    c.define("rgba", format!("{f}({uv}, in.instance_index)"));
                    let v = c.var("rgba");
                    c.alias("rgb", Operand::Value(format!("{v}.rgb")));
                    for ch in ["r", "g", "b", "a"] {
                        c.alias(ch, Operand::Value(format!("{v}.{ch}")));
                    }
                },
            ),
    );
}
