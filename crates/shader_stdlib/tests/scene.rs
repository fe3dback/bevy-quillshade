//! Scene category (SceneDepth, DepthFade): reads of the camera's depth prepass through stdlib `scene.wesl`, their
//! diagnostics (E009 outside the 3D fragment stage, W112 in opaque / masked materials) and the requirement carried
//! through function calls (`FunctionSig::scene`, `Ir::scene`).

use std::collections::{BTreeMap, BTreeSet};

use shader_graph_core::consts::NoConsts;
use shader_graph_core::ir::{WPO_PIN, build_ir};
use shader_graph_core::model::{Asset, AssetKind, BlendMode, MaterialKind, Node, SpriteSettings, UnlitSettings};
use shader_graph_core::registry::{FunctionSig, NoCurves, NoGradients, NoTextures, SceneTexture};
use shader_graph_core::{Code, IrResult, PinType, Value};

fn node(a: &mut Asset, id: &str, kind: &str) -> String {
    a.nodes.insert(id.to_string(), Node::new(kind, (0.0, 0.0)));
    id.to_string()
}

fn material(kind: MaterialKind) -> Asset {
    let mut a = Asset::new("mat", AssetKind::Material(kind));
    node(&mut a, "out", kind.output_node_kind());
    a
}

fn unlit(blend: BlendMode) -> Asset {
    material(MaterialKind::Unlit3d(UnlitSettings { blend, ..Default::default() }))
}

fn build_with(a: &Asset, path: &str, functions: &BTreeMap<String, FunctionSig>) -> IrResult {
    let reg = shader_stdlib::registry();
    build_ir(a, path, &reg, functions, &NoCurves, &NoConsts, &NoTextures, &NoGradients)
}

fn build(a: &Asset) -> IrResult {
    build_with(a, "test/mat", &BTreeMap::new())
}

fn codes_at(r: &IrResult, node: &str) -> Vec<Code> {
    r.diagnostics.iter().filter(|d| d.node.as_deref() == Some(node)).map(|d| d.code).collect()
}

fn line<'a>(r: &'a IrResult, name: &str) -> &'a str {
    let ir = r.ir.as_ref().expect("ir");
    &ir.main.lines.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("no line {name}")).expr
}

#[test]
fn scene_depth_reads_the_current_pixel_or_a_viewport_uv() {
    let mut a = unlit(BlendMode::Blend);
    let d = node(&mut a, "d", "stdlib::SceneDepth");
    a.connect((&d, "depth"), ("out", "color"));
    let r = build(&a);
    assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
    assert_eq!(line(&r, "n_d_raw"), "sg_scene_depth_raw(b_frag_coord.xy)");
    assert_eq!(line(&r, "n_d_depth"), "sg_linear_depth(n_d_raw)");
    let ir = r.ir.as_ref().unwrap();
    assert_eq!(ir.scene, BTreeSet::from([SceneTexture::Depth]));
    assert!(ir.stdlib.contains(&("scene".to_string(), "sg_scene_depth_raw".to_string())));

    let uv = node(&mut a, "uv", "stdlib::TextureCoordinate");
    a.connect((&uv, "out"), (&d, "uv"));
    let r = build(&a);
    assert_eq!(line(&r, "n_d_raw"), "sg_scene_depth_raw(sg_scene_pixel(b_uv0))");
}

#[test]
fn depth_fade_scales_opacity() {
    let mut a = unlit(BlendMode::Blend);
    let f = node(&mut a, "f", "stdlib::DepthFade");
    a.nodes.get_mut(&f).unwrap().inputs.insert("distance".into(), Value::Float(0.5));
    a.connect((&f, "out"), ("out", "opacity"));
    let r = build(&a);
    assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
    assert_eq!(line(&r, "n_f_out"), "(1.0f * sg_depth_fade(b_frag_coord, 0.5f))");
}

#[test]
fn opaque_and_masked_materials_read_their_own_depth() {
    for blend in [BlendMode::Opaque, BlendMode::Mask] {
        let mut a = unlit(blend);
        let d = node(&mut a, "d", "stdlib::SceneDepth");
        a.connect((&d, "depth"), ("out", "color"));
        let r = build(&a);
        assert_eq!(codes_at(&r, "d"), [Code::W112], "{blend:?}");
        assert!(!r.has_errors());
    }
    for blend in [BlendMode::Blend, BlendMode::Add, BlendMode::Multiply] {
        let mut a = unlit(blend);
        let f = node(&mut a, "f", "stdlib::DepthFade");
        a.connect((&f, "out"), ("out", "color"));
        assert_eq!(codes_at(&build(&a), "f"), [], "{blend:?}");
    }
}

#[test]
fn no_scene_textures_in_2d_or_the_vertex_stage() {
    let mut a = material(MaterialKind::Sprite2d(SpriteSettings::default()));
    let d = node(&mut a, "d", "stdlib::SceneDepth");
    a.connect((&d, "depth"), ("out", "color"));
    assert_eq!(codes_at(&build(&a), "d"), [Code::E009]);

    let mut a = unlit(BlendMode::Blend);
    let f = node(&mut a, "f", "stdlib::DepthFade");
    a.connect((&f, "out"), ("out", WPO_PIN));
    assert_eq!(codes_at(&build(&a), "f"), [Code::E009]);
}

/// Function `common/depth`: a SceneDepth into its output `d`.
fn depth_function() -> FunctionSig {
    let mut f = Asset::new("depth", AssetKind::Function);
    let d = node(&mut f, "n1", "stdlib::SceneDepth");
    let out = node(&mut f, "out", "output::FunctionOutput");
    let out_node = f.nodes.get_mut(&out).unwrap();
    out_node.params.insert("name".into(), Value::Str("d".into()));
    out_node.params.insert("type".into(), Value::Type(PinType::Float));
    f.connect((&d, "depth"), (&out, "value"));
    let r = build_with(&f, "common/depth", &BTreeMap::new());
    assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
    r.ir.unwrap().function_sig.unwrap()
}

#[test]
fn functions_pass_the_requirement_to_their_callers() {
    let sig = depth_function();
    assert_eq!(sig.scene, [SceneTexture::Depth]);
    assert!(sig.builtins.contains(&shader_graph_core::Builtin::FragCoord));
    let fns = BTreeMap::from([("common/depth".to_string(), sig)]);

    let mut a = unlit(BlendMode::Blend);
    let c = node(&mut a, "c", "function::common/depth");
    a.connect((&c, "d"), ("out", "color"));
    let r = build_with(&a, "test/mat", &fns);
    assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
    assert_eq!(r.ir.unwrap().scene, BTreeSet::from([SceneTexture::Depth]));

    let mut a = unlit(BlendMode::Opaque);
    let c = node(&mut a, "c", "function::common/depth");
    a.connect((&c, "d"), ("out", "color"));
    assert_eq!(codes_at(&build_with(&a, "test/mat", &fns), "c"), [Code::W112]);

    let mut a = material(MaterialKind::Sprite2d(SpriteSettings::default()));
    let c = node(&mut a, "c", "function::common/depth");
    a.connect((&c, "d"), ("out", "color"));
    assert_eq!(codes_at(&build_with(&a, "test/mat", &fns), "c"), [Code::E009]);
}
