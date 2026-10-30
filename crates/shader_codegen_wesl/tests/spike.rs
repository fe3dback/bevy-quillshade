//! M0 spike: generated WESL compiles with wesl 0.6 against Bevy 0.20 sources and passes naga.

use std::collections::BTreeMap;

use shader_codegen_wesl::check::{self, BevySources, Files, Variant};
use shader_codegen_wesl::{GenCtx, paths};
use shader_graph_core::consts::NoConsts;
use shader_graph_core::ir::build_ir;
use shader_graph_core::model::{Asset, AssetKind, MaterialKind, Node, PbrSettings};
use shader_graph_core::registry::{NoCurves, NoGradients, NoTextures};
use shader_graph_core::{BlendMode, FunctionSig, Value};

struct G {
    a: Asset,
    n: usize,
}

impl G {
    fn new(name: &str, kind: AssetKind) -> Self {
        G { a: Asset::new(name, kind), n: 0 }
    }

    fn node(&mut self, kind: &str, params: &[(&str, Value)]) -> String {
        self.n += 1;
        let id = format!("{:02}", self.n);
        let mut node = Node::new(kind, (0.0, 0.0));
        for (k, v) in params {
            node.params.insert(k.to_string(), v.clone());
        }
        self.a.nodes.insert(id.clone(), node);
        id
    }

    fn link(&mut self, from: (&str, &str), to: (&str, &str)) {
        self.a.connect(from, to);
    }
}

fn s(v: &str) -> Value {
    Value::Str(v.into())
}

#[test]
fn rock_wet_compiles_against_bevy() {
    let Some(bevy) = BevySources::discover() else {
        eprintln!("bevy 0.20 sources not found in cargo registry; skipping");
        return;
    };
    let registry = shader_stdlib::registry();
    let mut g = G::new(
        "rock_wet",
        AssetKind::Material(MaterialKind::Pbr3d(PbrSettings { blend: BlendMode::Mask, ..Default::default() })),
    );
    let out = g.node("output::Pbr3d", &[]);
    let albedo = g.node("param::TextureSample2D", &[("name", s("albedo"))]);
    let normal = g.node("param::TextureSample2D", &[("name", s("normal_map")), ("sampler_type", s("Normal"))]);
    let tint = g.node("param::Color", &[("name", s("tint"))]);
    let wet = g.node("param::Scalar", &[("name", s("wetness"))]);
    let speed = g.node("param::Scalar", &[("name", s("speed"))]);
    let mul = g.node("stdlib::Multiply", &[]);
    g.link((&albedo, "rgb"), (&mul, "a"));
    g.link((&tint, "rgb"), (&mul, "b"));
    g.link((&mul, "out"), (&out, "base_color"));
    let lerp = g.node("stdlib::Lerp", &[]);
    g.link((&wet, "out"), (&lerp, "alpha"));
    g.a.nodes.get_mut(&lerp).unwrap().inputs.insert("a".into(), Value::Float(0.8));
    g.a.nodes.get_mut(&lerp).unwrap().inputs.insert("b".into(), Value::Float(0.1));
    g.link((&lerp, "out"), (&out, "roughness"));
    g.link((&normal, "rgb"), (&out, "normal"));
    let time = g.node("stdlib::Time", &[]);
    let tm = g.node("stdlib::Multiply", &[]);
    g.link((&time, "out"), (&tm, "a"));
    g.link((&speed, "out"), (&tm, "b"));
    let sin = g.node("stdlib::Sin", &[]);
    g.link((&tm, "out"), (&sin, "x"));
    let em = g.node("stdlib::Multiply", &[]);
    g.link((&sin, "out"), (&em, "a"));
    g.link((&mul, "out"), (&em, "b"));
    g.link((&em, "out"), (&out, "emissive"));
    g.link((&albedo, "a"), (&out, "opacity_mask"));
    let remap = g.node("stdlib::Remap", &[]);
    let fres = g.node("stdlib::Fresnel", &[]);
    g.link((&fres, "out"), (&remap, "x"));
    g.link((&remap, "out"), (&out, "ambient_occlusion"));

    let fns: BTreeMap<String, FunctionSig> = BTreeMap::new();
    let res = build_ir(&g.a, "env/rock_wet", &registry, &fns, &NoCurves, &NoConsts, &NoTextures, &NoGradients);
    for d in &res.diagnostics {
        eprintln!("{d}");
    }
    assert!(!res.has_errors());
    let ir = res.ir.unwrap();
    assert_eq!(
        ir.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        ["albedo", "normal_map", "speed", "tint", "wetness"]
    );
    let generated = shader_codegen_wesl::material(
        &ir,
        &GenCtx { functions: &fns, source_file: "quillshade/material/env/rock_wet.material.ron" },
    );
    println!("{}", generated.text);

    let mut files = Files::default();
    for (name, src) in shader_stdlib::MODULES {
        files.add(&paths::stdlib_module(name), shader_codegen_wesl::stdlib_module(name, src));
    }
    let main = paths::material_module("env/rock_wet");
    files.add(&main, generated.text.clone());
    // Decision 0029: without the layout feature the game's material enables, the shader does not compile.
    let guard = check::compile(&bevy, &files, &main, Variant::Forward3d).unwrap_err();
    assert!(guard.message.contains("material_layout_changed__rebuild_the_game"), "{guard}");
    files.enable(ir.layout_feature());
    for v in [Variant::Forward3d, Variant::Forward3dBare, Variant::Deferred3d] {
        if let Err(e) = check::compile(&bevy, &files, &main, v) {
            panic!("{e}");
        }
    }
}

#[test]
fn check_rejects_broken_wesl() {
    let Some(bevy) = BevySources::discover() else { return };
    let mut files = Files::default();
    let main = paths::material_module("bad");
    files.add(&main, "import bevy_pbr::render::forward_io::VertexOutput;\n@fragment fn fragment(in: VertexOutput) -> @location(0) vec4<f32> { return in.uv_nope; }\n");
    assert!(check::compile(&bevy, &files, &main, Variant::Forward3d).is_err());
    files.add(&main, "import bevy_pbr::render::forward_io::VertexOutput;\n@fragment fn fragment(in: VertexOutput) -> @location(0) vec4<f32> { return vec4<f32>(in.uv, 0.0, 1.0); }\n");
    // `in.uv` exists only with VERTEX_UVS_A.
    let wgsl = check::compile(&bevy, &files, &main, Variant::Forward3d).unwrap();
    assert!(wgsl.contains("fn fragment"));
    assert!(check::compile(&bevy, &files, &main, Variant::Forward3dBare).is_err());
}
