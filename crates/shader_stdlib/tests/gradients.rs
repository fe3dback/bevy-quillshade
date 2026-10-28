//! Sample Gradient node (gradient design §6, §7): a slot of a gradient collection sampled from the collection's baked
//! texture, one binding per collection per material, hidden trailing arguments inside functions.

use std::collections::{BTreeMap, BTreeSet};

use shader_graph_core::consts::NoConsts;
use shader_graph_core::gradient::{GradientCollection, Stop};
use shader_graph_core::ir::{TextureSource, build_ir};
use shader_graph_core::model::{Asset, AssetKind, MaterialKind, Node, UnlitSettings};
use shader_graph_core::registry::{FunctionSig, NoCurves, gradient_refs};
use shader_graph_core::{Code, Diagnostic, Ir, IrResult, Value};
use shader_stdlib::nodes::color::{GRADIENT_COLLECTION_PARAM, GRADIENT_SLOT_PARAM, SAMPLE_GRADIENT_KIND};

const TERRAIN: &str = "common/terrain";

fn node(a: &mut Asset, id: &str, kind: &str, params: &[(&str, &str)]) -> String {
    let mut n = Node::new(kind, (0.0, 0.0));
    for (k, v) in params {
        n.params.insert(k.to_string(), Value::Str(v.to_string()));
    }
    a.nodes.insert(id.to_string(), n);
    id.to_string()
}

/// A Sample Gradient node on `collection`, slot id `slot`.
fn gradient(a: &mut Asset, id: &str, collection: &str, slot: i32) -> String {
    let n = Node::new(SAMPLE_GRADIENT_KIND, (0.0, 0.0))
        .with_param(GRADIENT_COLLECTION_PARAM, Value::Str(collection.into()))
        .with_param(GRADIENT_SLOT_PARAM, Value::Int(slot));
    a.nodes.insert(id.to_string(), n);
    id.to_string()
}

fn unlit() -> Asset {
    let mut a = Asset::new("mat", AssetKind::Material(MaterialKind::Unlit3d(UnlitSettings::default())));
    node(&mut a, "out", "output::Unlit3d", &[]);
    a
}

/// A collection with deterministic ids (1000, 1001, …): slot 0 is id 11 with two stops, slot 5 is id 22 and empty.
fn terrain() -> GradientCollection {
    let mut next = 1000;
    let mut c = GradientCollection::new(|| {
        next += 1;
        next
    });
    c.slots[0].id = 11;
    c.slots[0].stops = vec![
        Stop { t: 0.0, color: [0.0, 0.0, 0.0, 1.0], mid: 0.5 },
        Stop { t: 1.0, color: [1.0, 1.0, 1.0, 1.0], mid: 0.5 },
    ];
    c.slots[5].id = 22;
    c
}

fn gradients(ids: &[&str]) -> BTreeMap<String, GradientCollection> {
    ids.iter().map(|id| (id.to_string(), terrain())).collect()
}

fn build_with(
    a: &Asset,
    path: &str,
    functions: &BTreeMap<String, FunctionSig>,
    collections: &BTreeMap<String, GradientCollection>,
) -> IrResult {
    let reg = shader_stdlib::registry();
    let textures: BTreeSet<String> = ["textures/rock".to_string()].into();
    build_ir(a, path, &reg, functions, &NoCurves, &NoConsts, &textures, collections)
}

fn build_raw(a: &Asset) -> IrResult {
    build_with(a, "test/mat", &BTreeMap::new(), &gradients(&[TERRAIN]))
}

fn build(a: &Asset) -> (Ir, Vec<Diagnostic>) {
    let r = build_raw(a);
    let errors: Vec<_> = r.diagnostics.iter().filter(|d| d.is_error()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    (r.ir.expect("ir"), r.diagnostics)
}

fn w111(diags: &[Diagnostic]) -> Vec<(&str, &str)> {
    diags
        .iter()
        .filter(|d| d.code == Code::W111)
        .map(|d| (d.node.as_deref().unwrap_or("-"), d.message.as_str()))
        .collect()
}

fn rgba_expr<'a>(ir: &'a Ir, node: &str) -> &'a str {
    let name = format!("n_{node}_rgba");
    ir.main.lines.iter().find(|l| l.name == name).map(|l| l.expr.as_str()).unwrap_or_else(|| panic!("{:?}", ir.main))
}

#[test]
fn no_selection_is_white_w111_no_binding() {
    for (collection, slot) in [("", 11), (TERRAIN, 0), ("", 0)] {
        let mut a = unlit();
        let g = gradient(&mut a, "g", collection, slot);
        a.connect((&g, "rgb"), ("out", "color"));
        let (ir, diags) = build(&a);
        assert_eq!(w111(&diags), [("g", "no gradient selected; outputs white")], "{collection:?} {slot}");
        assert_eq!(rgba_expr(&ir, "g"), "vec4<f32>(1.0)");
        assert!(ir.textures.is_empty(), "{:?}", ir.textures);
        assert!(!ir.stdlib.iter().any(|(_, f)| f == "sg_sample_gradient"), "{:?}", ir.stdlib);
    }
    // A fresh node: no collection, no gradient.
    let mut a = unlit();
    let g = node(&mut a, "g", SAMPLE_GRADIENT_KIND, &[]);
    a.connect((&g, "rgb"), ("out", "color"));
    let (ir, diags) = build(&a);
    assert_eq!(w111(&diags).len(), 1, "{diags:?}");
    assert!(ir.textures.is_empty());
}

#[test]
fn missing_collection_or_slot_is_e006() {
    let mut a = unlit();
    let g = gradient(&mut a, "g", "nope", 11);
    a.connect((&g, "rgb"), ("out", "color"));
    let r = build_raw(&a);
    assert!(
        r.diagnostics.iter().any(|d| d.code == Code::E006
            && d.node.as_deref() == Some("g")
            && d.message == "gradient collection `nope` does not exist or cannot be loaded"),
        "{:?}",
        r.diagnostics
    );

    let mut a = unlit();
    let g = gradient(&mut a, "g", TERRAIN, 99);
    a.connect((&g, "rgb"), ("out", "color"));
    let r = build_raw(&a);
    assert!(
        r.diagnostics.iter().any(|d| d.code == Code::E006
            && d.node.as_deref() == Some("g")
            && d.message == "gradient #99 is not in collection `common/terrain`"),
        "{:?}",
        r.diagnostics
    );
}

#[test]
fn samples_row_of_slot_id() {
    let mut a = unlit();
    let g = gradient(&mut a, "g", TERRAIN, 11);
    a.connect((&g, "rgb"), ("out", "color"));
    let (ir, diags) = build(&a);
    assert!(w111(&diags).is_empty(), "{diags:?}");
    let e = rgba_expr(&ir, "g");
    assert!(e.contains("sg_sample_gradient(gc_common_terrain, gc_common_terrain_sampler, "), "{e}");
    assert!(e.ends_with(", 0.00390625f)"), "row 0: {e}");
    assert!(ir.stdlib.contains(&("texture".to_string(), "sg_sample_gradient".to_string())), "{:?}", ir.stdlib);
    // `rgb` / `a` alias the rgba line, as TextureSample does.
    let out = &ir.main.outputs["color"];
    assert!(out.contains("n_g_rgba.rgb"), "{out}");
}

#[test]
fn empty_slot_warns_but_still_samples() {
    let mut a = unlit();
    let g = gradient(&mut a, "g", TERRAIN, 22);
    a.connect((&g, "rgb"), ("out", "color"));
    let (ir, diags) = build(&a);
    assert_eq!(w111(&diags), [("g", "gradient is empty; outputs white")]);
    let e = rgba_expr(&ir, "g");
    assert!(e.starts_with("sg_sample_gradient(gc_common_terrain, gc_common_terrain_sampler, "), "{e}");
    assert!(e.ends_with(", 0.04296875f)"), "row 5: {e}");
    assert_eq!(ir.textures.len(), 1);
}

#[test]
fn gradient_binding_after_params_and_fixed() {
    let mut a = unlit();
    let p = node(&mut a, "p", "param::TextureSample2D", &[("name", "albedo"), ("default", "textures/rock")]);
    let s = node(&mut a, "s", "stdlib::TextureSample", &[("texture", "textures/rock")]);
    let g = gradient(&mut a, "g", TERRAIN, 11);
    let add1 = node(&mut a, "add1", "stdlib::Add", &[]);
    let add2 = node(&mut a, "add2", "stdlib::Add", &[]);
    a.connect((&p, "rgb"), (&add1, "a"));
    a.connect((&s, "rgb"), (&add1, "b"));
    a.connect((&add1, "out"), (&add2, "a"));
    a.connect((&g, "rgb"), (&add2, "b"));
    a.connect((&add2, "out"), ("out", "color"));
    let (ir, _) = build(&a);
    let sources: Vec<&TextureSource> = ir.textures.iter().map(|t| &t.source).collect();
    assert_eq!(
        sources,
        [
            &TextureSource::Param("albedo".into()),
            &TextureSource::Fixed { node: "s".into(), origin: None },
            &TextureSource::Gradient(TERRAIN.into()),
        ]
    );
    let g = &ir.textures[2];
    assert_eq!((g.ident.as_str(), g.texture.as_str()), ("gc_common_terrain", "@gradients/common/terrain"));
    let bindings: Vec<u32> = ir.textures.iter().map(|t| t.binding).collect();
    assert_eq!(bindings, [101, 103, 105]);
}

/// Function `common/g`: one Sample Gradient on `common/terrain` into its output.
fn gradient_function() -> FunctionSig {
    let mut f = Asset::new("g", AssetKind::Function);
    let g = gradient(&mut f, "n1", TERRAIN, 11);
    let out = node(&mut f, "out", "output::FunctionOutput", &[("name", "rgb")]);
    f.nodes.get_mut(&out).unwrap().params.insert("type".into(), Value::Type(shader_graph_core::PinType::Float3));
    f.connect((&g, "rgb"), (&out, "value"));
    let r = build_with(&f, "common/g", &BTreeMap::new(), &gradients(&[TERRAIN]));
    assert!(!r.has_errors(), "{:?}", r.diagnostics);
    let ir = r.ir.unwrap();
    assert!(ir.textures.is_empty(), "functions have no material bindings");
    assert!(rgba_expr(&ir, "n1").starts_with("sg_sample_gradient(gc_common_terrain, gc_common_terrain_sampler, "));
    ir.function_sig.unwrap()
}

#[test]
fn gradient_binding_once_per_collection() {
    let sig = gradient_function();
    assert_eq!(sig.gradients, [TERRAIN]);
    let fns = BTreeMap::from([("common/g".to_string(), sig)]);

    let mut a = unlit();
    let g1 = gradient(&mut a, "g1", TERRAIN, 11);
    let g2 = gradient(&mut a, "g2", TERRAIN, 22);
    let c1 = node(&mut a, "c1", "function::common/g", &[]);
    let c2 = node(&mut a, "c2", "function::common/g", &[]);
    // ((g1 + g2) + c1) + c2 → color.
    let mut prev = (g1, "rgb");
    for (i, n) in [g2, c1, c2].iter().enumerate() {
        let add = node(&mut a, &format!("add{i}"), "stdlib::Add", &[]);
        a.connect((prev.0.as_str(), prev.1), (&add, "a"));
        a.connect((n.as_str(), "rgb"), (&add, "b"));
        prev = (add, "out");
    }
    a.connect((prev.0.as_str(), prev.1), ("out", "color"));
    let r = build_with(&a, "test/mat", &fns, &gradients(&[TERRAIN]));
    assert!(!r.has_errors(), "{:?}", r.diagnostics);
    let ir = r.ir.unwrap();
    let gradient_bindings: Vec<_> =
        ir.textures.iter().filter(|t| matches!(t.source, TextureSource::Gradient(_))).collect();
    assert_eq!(gradient_bindings.len(), 1, "{:?}", ir.textures);
    assert_eq!(ir.textures.len(), 1);
    let calls =
        ir.main.lines.iter().filter(|l| l.expr == "fun_g(gc_common_terrain, gc_common_terrain_sampler)").count();
    assert_eq!(calls, 2, "{:?}", ir.main);
}

/// A material sampling collections `first` and `second`.
fn two_collections(first: &str, second: &str) -> IrResult {
    let mut a = unlit();
    let g1 = gradient(&mut a, "g1", first, 11);
    let g2 = gradient(&mut a, "g2", second, 11);
    let add = node(&mut a, "add", "stdlib::Add", &[]);
    a.connect((&g1, "rgb"), (&add, "a"));
    a.connect((&g2, "rgb"), (&add, "b"));
    a.connect((&add, "out"), ("out", "color"));
    build_with(&a, "test/mat", &BTreeMap::new(), &gradients(&[first, second]))
}

#[test]
fn gradient_ident_collision_is_e006() {
    let r = two_collections("a/b_c", "a_b/c");
    assert!(
        r.diagnostics.iter().any(|d| d.code == Code::E006
            && d.message == "collections `a/b_c` and `a_b/c` get the same identifier `gc_a_b_c`; rename one of them"),
        "{:?}",
        r.diagnostics
    );
    // The texture of `x_sampler` is the sampler of `x`.
    let r = two_collections("x", "x_sampler");
    assert!(
        r.diagnostics.iter().any(|d| d.code == Code::E006 && d.message.contains("`gc_x_sampler`")),
        "{:?}",
        r.diagnostics
    );
    assert!(!two_collections("a/b", "a/c").has_errors());
}

#[test]
fn gradient_refs_lists_collections() {
    let reg = shader_stdlib::registry();
    let mut a = unlit();
    gradient(&mut a, "g1", TERRAIN, 11);
    gradient(&mut a, "g2", "", 0);
    node(&mut a, "g3", SAMPLE_GRADIENT_KIND, &[]);
    assert_eq!(gradient_refs(&a, &reg), [("g1".to_string(), "collection".to_string(), TERRAIN.to_string())]);
}

#[test]
fn sample_gradient_definition() {
    let reg = shader_stdlib::registry();
    let def = reg.get(SAMPLE_GRADIENT_KIND).expect("registered");
    let info = def.info();
    assert_eq!((info.name.as_str(), info.category.as_str()), ("Sample Gradient", "Color"));
    for kw in ["gradient", "ramp", "color curve", "palette", "lut", "atlas"] {
        assert!(info.keywords.iter().any(|k| k == kw), "{kw}");
    }
    let params = def.params();
    let kinds: Vec<_> = params.iter().map(|p| (p.id.as_str(), &p.kind, &p.default)).collect();
    assert_eq!(
        kinds,
        [
            (GRADIENT_COLLECTION_PARAM, &shader_graph_core::ParamKind::GradientCollection, &Value::Str(String::new())),
            (GRADIENT_SLOT_PARAM, &shader_graph_core::ParamKind::GradientSlot, &Value::Int(0)),
        ]
    );
}
