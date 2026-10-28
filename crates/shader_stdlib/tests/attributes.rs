//! Material Attributes (PBR), decision 0023: Make / Break, Lerp over attributes, the `Wrapped` output mode.

use shader_graph_core::attributes::{PBR_STRUCT, pbr_fields};
use shader_graph_core::consts::NoConsts;
use shader_graph_core::ir::{WPO_PIN, build_ir};
use shader_graph_core::model::{
    Asset, AssetKind, BlendMode, MaterialKind, Node, PbrOutputMode, PbrSettings, Shading, UnlitSettings,
};
use shader_graph_core::registry::{NoCurves, NoFunctions, NoGradients, NoTextures, SigCtx};
use shader_graph_core::{Code, Diagnostic, Ir, IrResult, PinType, Value};

const MAKE: &str = "stdlib::MakeMaterialAttributesPbr";
const BREAK: &str = "stdlib::BreakMaterialAttributesPbr";

fn node(a: &mut Asset, id: &str, kind: &str, params: &[(&str, Value)]) -> String {
    let mut n = Node::new(kind, (0.0, 0.0));
    for (k, v) in params {
        n.params.insert(k.to_string(), v.clone());
    }
    a.nodes.insert(id.to_string(), n);
    id.to_string()
}

fn build_raw(a: &Asset) -> IrResult {
    let reg = shader_stdlib::registry();
    build_ir(a, "test/mat", &reg, &NoFunctions, &NoCurves, &NoConsts, &NoTextures, &NoGradients)
}

fn build(a: &Asset) -> (Ir, Vec<Diagnostic>) {
    let r = build_raw(a);
    let errors: Vec<_> = r.diagnostics.iter().filter(|d| d.is_error()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    (r.ir.expect("ir"), r.diagnostics)
}

fn unlit() -> Asset {
    Asset::new("mat", AssetKind::Material(MaterialKind::Unlit3d(UnlitSettings::default())))
}

fn struct_import() -> (String, String) {
    ("material".to_string(), PBR_STRUCT.to_string())
}

fn line<'a>(ir: &'a Ir, name: &str) -> &'a shader_graph_core::registry::IrLine {
    ir.main.lines.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("no line {name}: {:?}", ir.main.lines))
}

#[test]
fn make_break_roundtrip_imports_struct() {
    let mut a = unlit();
    let out = node(&mut a, "out", "output::Unlit3d", &[]);
    let make = node(&mut a, "make", MAKE, &[]);
    let brk = node(&mut a, "brk", BREAK, &[]);
    a.nodes.get_mut(&make).unwrap().inputs.insert("base_color".into(), Value::Float3([0.2, 0.4, 0.6]));
    a.connect((&make, "attributes"), (&brk, "attributes"));
    a.connect((&brk, "base_color"), (&out, "color"));
    let (ir, _) = build(&a);
    assert!(ir.stdlib.contains(&struct_import()), "{:?}", ir.stdlib);
    let m = line(&ir, "n_make_attributes");
    assert_eq!(m.ty, PBR_STRUCT);
    assert!(m.expr.starts_with("MaterialAttributesPbr(vec3<f32>(0.2, 0.4, 0.6), 0.0f"), "{}", m.expr);
    let b = line(&ir, "n_brk_base_color");
    assert!(b.expr.ends_with(".base_color"), "{}", b.expr);
    assert_eq!(b.ty, "vec3<f32>");
}

#[test]
fn unconnected_break_uses_default_literal() {
    let mut a = unlit();
    let out = node(&mut a, "out", "output::Unlit3d", &[]);
    let brk = node(&mut a, "brk", BREAK, &[]);
    a.connect((&brk, "base_color"), (&out, "color"));
    let (ir, _) = build(&a);
    let b = line(&ir, "n_brk_base_color");
    assert!(b.expr.starts_with("MaterialAttributesPbr(vec3<f32>(1.0"), "{}", b.expr);
    assert!(ir.stdlib.contains(&struct_import()), "{:?}", ir.stdlib);
}

#[test]
fn make_has_field_pins() {
    let reg = shader_stdlib::registry();
    let a = unlit();
    let env = shader_graph_core::Env {
        asset_kind: a.kind,
        asset_path: "test/mat",
        functions: &NoFunctions,
        curves: &NoCurves,
        consts: &NoConsts,
        textures: &NoTextures,
        gradients: &NoGradients,
    };
    let empty = Default::default();
    let params = Default::default();
    let ctx = SigCtx { node_id: "n", kind: MAKE, params: &params, connected: &empty, input_values: &params, env: &env };
    let sig = reg.get(MAKE).unwrap().signature(&ctx);
    let ids: Vec<&str> = sig.inputs.iter().map(|p| p.id.as_str()).collect();
    let want: Vec<&str> = pbr_fields().iter().map(|f| f.id).collect();
    assert_eq!(ids, want);
    assert_eq!(sig.outputs.len(), 1);
    assert_eq!(sig.outputs[0].id, "attributes");
    assert_eq!(sig.outputs[0].ty, PinType::MaterialAttributesPbr);
    let ctx = SigCtx { kind: BREAK, ..ctx };
    let sig = reg.get(BREAK).unwrap().signature(&ctx);
    let ids: Vec<&str> = sig.outputs.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, want);
    assert_eq!(sig.inputs.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["attributes"]);
}

fn has_e003(diags: &[Diagnostic], node: &str, pin: &str) -> bool {
    diags.iter().any(|d| d.code == Code::E003 && d.node.as_deref() == Some(node) && d.pin.as_deref() == Some(pin))
}

#[test]
fn lerp_of_attributes() {
    let mut a = unlit();
    let out = node(&mut a, "out", "output::Unlit3d", &[]);
    let m1 = node(&mut a, "m1", MAKE, &[]);
    let m2 = node(&mut a, "m2", MAKE, &[]);
    let k = node(&mut a, "k", "stdlib::Constant", &[("value", Value::Float(0.3))]);
    let lerp = node(&mut a, "lerp", "stdlib::Lerp", &[]);
    let brk = node(&mut a, "brk", BREAK, &[]);
    a.connect((&m1, "attributes"), (&lerp, "a"));
    a.connect((&m2, "attributes"), (&lerp, "b"));
    a.connect((&k, "out"), (&lerp, "alpha"));
    a.connect((&lerp, "out"), (&brk, "attributes"));
    a.connect((&brk, "base_color"), (&out, "color"));
    let (ir, _) = build(&a);
    let l = line(&ir, "n_lerp_out");
    assert_eq!(l.ty, PBR_STRUCT);
    assert_eq!(l.expr, "sg_lerp_attributes_pbr(n_m1_attributes, n_m2_attributes, 0.3f)");
    assert!(ir.stdlib.contains(&("material".to_string(), "sg_lerp_attributes_pbr".to_string())), "{:?}", ir.stdlib);
    assert!(ir.stdlib.contains(&struct_import()));
}

#[test]
fn lerp_with_one_unconnected_side_uses_defaults() {
    let mut a = unlit();
    let out = node(&mut a, "out", "output::Unlit3d", &[]);
    let m1 = node(&mut a, "m1", MAKE, &[]);
    let lerp = node(&mut a, "lerp", "stdlib::Lerp", &[]);
    let brk = node(&mut a, "brk", BREAK, &[]);
    a.connect((&m1, "attributes"), (&lerp, "b"));
    a.connect((&lerp, "out"), (&brk, "attributes"));
    a.connect((&brk, "base_color"), (&out, "color"));
    let (ir, _) = build(&a);
    let l = line(&ir, "n_lerp_out");
    assert!(l.expr.starts_with("sg_lerp_attributes_pbr(MaterialAttributesPbr("), "{}", l.expr);
    assert!(l.expr.ends_with(", n_m1_attributes, 0.5f)"), "{}", l.expr);
}

#[test]
fn lerp_attributes_vs_vector_is_e003() {
    let mut a = unlit();
    let out = node(&mut a, "out", "output::Unlit3d", &[]);
    let m1 = node(&mut a, "m1", MAKE, &[]);
    let v = node(&mut a, "v", "stdlib::Constant3Vector", &[]);
    let lerp = node(&mut a, "lerp", "stdlib::Lerp", &[]);
    let brk = node(&mut a, "brk", BREAK, &[]);
    a.connect((&m1, "attributes"), (&lerp, "a"));
    a.connect((&v, "out"), (&lerp, "b"));
    a.connect((&lerp, "out"), (&brk, "attributes"));
    a.connect((&brk, "base_color"), (&out, "color"));
    let r = build_raw(&a);
    assert!(has_e003(&r.diagnostics, "lerp", "b"), "{:?}", r.diagnostics);
}

#[test]
fn add_refuses_attributes() {
    let mut a = unlit();
    let out = node(&mut a, "out", "output::Unlit3d", &[]);
    let m1 = node(&mut a, "m1", MAKE, &[]);
    let add = node(&mut a, "add", "stdlib::Add", &[]);
    a.connect((&m1, "attributes"), (&add, "a"));
    a.connect((&add, "out"), (&out, "color"));
    let r = build_raw(&a);
    assert!(has_e003(&r.diagnostics, "add", "a"), "{:?}", r.diagnostics);
}

fn pbr(settings: PbrSettings) -> Asset {
    Asset::new("mat", AssetKind::Material(MaterialKind::Pbr3d(settings)))
}

fn wrapped() -> PbrSettings {
    PbrSettings { output: PbrOutputMode::Wrapped, ..Default::default() }
}

fn output_sig(settings: PbrSettings) -> shader_graph_core::Signature {
    let reg = shader_stdlib::registry();
    let a = pbr(settings);
    let env = shader_graph_core::Env {
        asset_kind: a.kind,
        asset_path: "test/mat",
        functions: &NoFunctions,
        curves: &NoCurves,
        consts: &NoConsts,
        textures: &NoTextures,
        gradients: &NoGradients,
    };
    let empty = Default::default();
    let params = Default::default();
    let ctx = SigCtx {
        node_id: "out",
        kind: "output::Pbr3d",
        params: &params,
        connected: &empty,
        input_values: &params,
        env: &env,
    };
    reg.get("output::Pbr3d").unwrap().signature(&ctx)
}

#[test]
fn wrapped_signature() {
    let sig = output_sig(wrapped());
    let ids: Vec<&str> = sig.inputs.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["attributes", WPO_PIN]);
    assert_eq!(sig.inputs[0].ty, PinType::MaterialAttributesPbr);
    assert_eq!(sig.inputs[0].label, "Material Attributes");
}

#[test]
fn default_mode_unchanged() {
    let sig = output_sig(PbrSettings::default());
    let pins: Vec<(&str, bool)> = sig.inputs.iter().map(|p| (p.id.as_str(), p.inactive)).collect();
    assert_eq!(
        pins,
        [
            ("base_color", false),
            ("metallic", false),
            ("specular", false),
            ("roughness", false),
            ("emissive", false),
            ("opacity", true),
            ("opacity_mask", true),
            ("normal", false),
            (WPO_PIN, false),
            ("ambient_occlusion", false),
        ]
    );
    let unlit_mask = output_sig(PbrSettings { shading: Shading::Unlit, blend: BlendMode::Mask, ..Default::default() });
    let inactive: Vec<&str> = unlit_mask.inputs.iter().filter(|p| p.inactive).map(|p| p.id.as_str()).collect();
    assert_eq!(inactive, ["metallic", "specular", "roughness", "opacity", "normal", "ambient_occlusion"]);
}

fn wrapped_graph(settings: PbrSettings) -> Asset {
    let mut a = pbr(settings);
    let out = node(&mut a, "out", "output::Pbr3d", &[]);
    let make = node(&mut a, "make", MAKE, &[]);
    a.connect((&make, "attributes"), (&out, "attributes"));
    a
}

#[test]
fn wrapped_expands_active_fields() {
    let (ir, diags) = build(&wrapped_graph(wrapped()));
    let keys: Vec<&str> = ir.main.outputs.keys().map(String::as_str).collect();
    let mut want = ["base_color", "metallic", "specular", "roughness", "emissive", "normal", "ambient_occlusion"];
    want.sort();
    assert_eq!(keys, want);
    for (k, v) in &ir.main.outputs {
        assert_eq!(v, &format!("n_make_attributes.{k}"));
    }
    assert!(!diags.iter().any(|d| d.code == Code::W103), "{diags:?}");
    assert!(ir.uses_normal);
}

#[test]
fn wrapped_respects_shading_and_blend() {
    let s = PbrSettings { shading: Shading::Unlit, blend: BlendMode::Mask, ..wrapped() };
    let (ir, _) = build(&wrapped_graph(s));
    let keys: Vec<&str> = ir.main.outputs.keys().map(String::as_str).collect();
    assert_eq!(keys, ["base_color", "emissive", "opacity_mask"]);
}

#[test]
fn wrapped_unconnected_writes_nothing() {
    let mut a = pbr(wrapped());
    node(&mut a, "out", "output::Pbr3d", &[]);
    let (ir, _) = build(&a);
    assert!(ir.main.outputs.is_empty(), "{:?}", ir.main.outputs);
}

#[test]
fn wrapped_wpo_in_vertex() {
    let mut a = wrapped_graph(wrapped());
    let v = node(&mut a, "v", "stdlib::Constant3Vector", &[("value", Value::Float3([0.0, 0.1, 0.0]))]);
    a.connect((&v, "out"), ("out", WPO_PIN));
    let (ir, _) = build(&a);
    assert!(ir.vertex.as_ref().expect("vertex stage").outputs.contains_key(WPO_PIN));
    assert!(!ir.main.outputs.contains_key(WPO_PIN));
}

/// `mask` and `angle` both used: `mask * angle → color`; `normal` optionally connected.
fn world_aligned_graph(normal: bool) -> Asset {
    let mut a = unlit();
    let out = node(&mut a, "out", "output::Unlit3d", &[]);
    let wab = node(&mut a, "wab", "stdlib::WorldAlignedBlend", &[]);
    let mul = node(&mut a, "mul", "stdlib::Multiply", &[]);
    a.connect((&wab, "mask"), (&mul, "a"));
    a.connect((&wab, "angle"), (&mul, "b"));
    a.connect((&mul, "out"), (&out, "color"));
    if normal {
        let n = node(&mut a, "n", "stdlib::Constant3Vector", &[("value", Value::Float3([0.0, 0.0, 1.0]))]);
        a.connect((&n, "out"), (&wab, "normal"));
    }
    a
}

#[test]
fn world_aligned_blend_emits() {
    let (ir, _) = build(&world_aligned_graph(false));
    let mask = line(&ir, "n_wab_mask");
    assert_eq!(mask.expr, "sg_world_aligned_blend(b_world_normal, vec3<f32>(0.0, 1.0, 0.0), 10.0f, -0.7f)");
    let angle = line(&ir, "n_wab_angle");
    assert_eq!(angle.expr, "sg_slope_angle(b_world_normal, vec3<f32>(0.0, 1.0, 0.0))");
    let sh = |f: &str| ("shading".to_string(), f.to_string());
    assert!(ir.stdlib.contains(&sh("sg_world_aligned_blend")) && ir.stdlib.contains(&sh("sg_slope_angle")));
    assert!(ir.main.builtins.contains(&shader_graph_core::Builtin::WorldNormal));

    let (ir, _) = build(&world_aligned_graph(true));
    let mask = line(&ir, "n_wab_mask");
    assert!(mask.expr.starts_with("sg_world_aligned_blend(vec3<f32>(0.0, 0.0, 1.0), "), "{}", mask.expr);
    assert!(!ir.main.builtins.contains(&shader_graph_core::Builtin::WorldNormal));
}

/// Reference values of the formula in `shading.wesl` (decision 0023): with the defaults (sharpness 10, bias -0.7)
/// slopes up to ~45° are fully masked and slopes past ~53° not at all; the angle is 0 flat and 90 on a wall.
#[test]
fn world_aligned_blend_reference_values() {
    let mask =
        |deg: f32, sharpness: f32, bias: f32| ((deg.to_radians().cos() + bias) * sharpness + 1.0).clamp(0.0, 1.0);
    assert_eq!(mask(0.0, 10.0, -0.7), 1.0);
    assert_eq!(mask(45.0, 10.0, -0.7), 1.0);
    assert!(mask(50.0, 10.0, -0.7) > 0.0 && mask(50.0, 10.0, -0.7) < 1.0);
    assert_eq!(mask(54.0, 10.0, -0.7), 0.0);
    assert_eq!(mask(90.0, 10.0, -0.7), 0.0);
    // Bias 0 keeps every upward-facing surface; bias -2 keeps none.
    assert_eq!(mask(89.0, 10.0, 0.0), 1.0);
    assert_eq!(mask(0.0, 10.0, -2.0), 0.0);
    let src = shader_stdlib::MODULES.iter().find(|m| m.0 == "shading").unwrap().1;
    assert!(src.contains("return saturate((d + bias) * sharpness + 1.0);"), "{src}");
    assert!(src.contains("degrees(acos(clamp(dot(normalize(n), normalize(up)), -1.0, 1.0)))"), "{src}");
}
