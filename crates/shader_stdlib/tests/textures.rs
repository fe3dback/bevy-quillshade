//! Texture nodes referring to Texture assets (design §1.2, §1.3): fixed textures become per-node material bindings,
//! the IR owns the full texture binding list, an empty reference falls back to the default texture (W110).

use std::collections::{BTreeMap, BTreeSet};

use shader_graph_core::consts::NoConsts;
use shader_graph_core::ir::{TextureBinding, TextureSource, build_ir};
use shader_graph_core::model::{Asset, AssetKind, MaterialKind, Node, UnlitSettings};
use shader_graph_core::registry::{FnPort, FunctionSig, HiddenTexture, NoCurves, NoFunctions, ParamKind};
use shader_graph_core::{Code, Diagnostic, Ir, IrResult, PinType, Value};
use shader_stdlib::internal::DEFAULT_TEXTURE;

fn node(a: &mut Asset, id: &str, kind: &str, params: &[(&str, &str)]) -> String {
    let mut n = Node::new(kind, (0.0, 0.0));
    for (k, v) in params {
        n.params.insert(k.to_string(), Value::Str(v.to_string()));
    }
    a.nodes.insert(id.to_string(), n);
    id.to_string()
}

fn unlit() -> Asset {
    let mut a = Asset::new("mat", AssetKind::Material(MaterialKind::Unlit3d(UnlitSettings::default())));
    node(&mut a, "out", "output::Unlit3d", &[]);
    a
}

fn build_raw(a: &Asset) -> IrResult {
    let reg = shader_stdlib::registry();
    let textures: BTreeSet<String> = ["textures/rock".to_string(), "textures/moss".to_string()].into();
    build_ir(a, "test/mat", &reg, &NoFunctions, &NoCurves, &NoConsts, &textures)
}

fn build_with(a: &Asset, functions: &BTreeMap<String, FunctionSig>) -> IrResult {
    let reg = shader_stdlib::registry();
    let textures: BTreeSet<String> = ["textures/rock".to_string()].into();
    build_ir(a, "test/mat", &reg, functions, &NoCurves, &NoConsts, &textures)
}

fn build(a: &Asset) -> (Ir, Vec<Diagnostic>) {
    let r = build_raw(a);
    let errors: Vec<_> = r.diagnostics.iter().filter(|d| d.is_error()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    (r.ir.expect("ir"), r.diagnostics)
}

fn fixed(node: &str, texture: &str, binding: u32) -> TextureBinding {
    TextureBinding {
        ident: format!("ft_{node}"),
        source: TextureSource::Fixed { node: node.into(), origin: None },
        texture: texture.into(),
        binding,
    }
}

fn w110_at(diags: &[Diagnostic]) -> Vec<&str> {
    diags.iter().filter(|d| d.code == Code::W110).filter_map(|d| d.node.as_deref()).collect()
}

#[test]
fn texture_sample_without_texture_warns_w110() {
    let mut a = unlit();
    let s = node(&mut a, "s", "stdlib::TextureSample", &[]);
    a.connect((&s, "rgb"), ("out", "color"));
    let (ir, diags) = build(&a);
    assert_eq!(w110_at(&diags), ["s"], "{diags:?}");
    let w110 = diags.iter().find(|d| d.code == Code::W110).unwrap();
    assert_eq!(w110.message, "no texture selected; uses the default texture");
    assert_eq!(ir.textures, vec![fixed("s", DEFAULT_TEXTURE, 101)]);
    assert!(ir.main.lines.iter().any(|l| l.expr.contains("textureSample(ft_s, ft_s_sampler, ")), "{:?}", ir.main);
}

#[test]
fn texture_sample_property_is_a_fixed_binding() {
    let mut a = unlit();
    let s = node(&mut a, "s", "stdlib::TextureSample", &[("texture", "textures/rock")]);
    a.connect((&s, "rgb"), ("out", "color"));
    let (ir, diags) = build(&a);
    assert!(w110_at(&diags).is_empty(), "{diags:?}");
    assert_eq!(ir.textures, vec![fixed("s", "textures/rock", 101)]);
    // The Texture asset id never reaches the WESL: swapping the texture keeps the shader.
    assert!(!ir.main.lines.iter().any(|l| l.expr.contains("rock")));
}

#[test]
fn texture_sample_pin_overrides_property() {
    let mut a = unlit();
    let t = node(&mut a, "t", "param::TextureObject", &[("name", "albedo"), ("default", "textures/moss")]);
    let s = node(&mut a, "s", "stdlib::TextureSample", &[("texture", "textures/rock")]);
    a.connect((&t, "out"), (&s, "tex"));
    a.connect((&s, "rgb"), ("out", "color"));
    let (ir, diags) = build(&a);
    assert!(w110_at(&diags).is_empty(), "{diags:?}");
    assert!(!ir.textures.iter().any(|b| matches!(b.source, TextureSource::Fixed { .. })), "{:?}", ir.textures);
    assert_eq!(ir.textures.len(), 1);
    assert!(ir.main.lines.iter().any(|l| l.expr.contains("textureSample(inp_albedo, inp_albedo_sampler, ")));
}

#[test]
fn two_fixed_nodes_same_asset_get_two_bindings() {
    let mut a = unlit();
    let s1 = node(&mut a, "a", "stdlib::TextureSample", &[("texture", "textures/rock")]);
    let s2 = node(&mut a, "b", "stdlib::TextureSample", &[("texture", "textures/rock")]);
    let add = node(&mut a, "add", "stdlib::Add", &[]);
    a.connect((&s1, "rgb"), (&add, "a"));
    a.connect((&s2, "rgb"), (&add, "b"));
    a.connect((&add, "out"), ("out", "color"));
    let (ir, _) = build(&a);
    assert_eq!(ir.textures, vec![fixed("a", "textures/rock", 101), fixed("b", "textures/rock", 103)]);
}

#[test]
fn param_default_is_texture_id() {
    let mut a = unlit();
    let p = node(&mut a, "p", "param::TextureSample2D", &[("name", "albedo"), ("default", "textures/rock")]);
    let s = node(&mut a, "s", "stdlib::TextureSample", &[("texture", "textures/moss")]);
    let add = node(&mut a, "add", "stdlib::Add", &[]);
    a.connect((&s, "rgb"), (&add, "a"));
    a.connect((&p, "rgb"), (&add, "b"));
    a.connect((&add, "out"), ("out", "color"));
    let (ir, diags) = build(&a);
    assert!(w110_at(&diags).is_empty(), "{diags:?}");
    assert_eq!(ir.params[0].default, Value::Str("textures/rock".into()));
    // Parameters first (by name), then fixed textures.
    assert_eq!(
        ir.textures,
        vec![
            TextureBinding {
                ident: "inp_albedo".into(),
                source: TextureSource::Param("albedo".into()),
                texture: "textures/rock".into(),
                binding: 101,
            },
            fixed("s", "textures/moss", 103),
        ]
    );

    // An unset parameter default: W110, the default texture.
    a.nodes.get_mut(&p).unwrap().params.insert("default".into(), Value::Str(String::new()));
    let (ir, diags) = build(&a);
    assert_eq!(w110_at(&diags), ["p"]);
    assert_eq!(ir.textures[0].texture, DEFAULT_TEXTURE);
}

#[test]
fn texture_params_are_texture_references() {
    let reg = shader_stdlib::registry();
    for (kind, param) in [
        ("stdlib::TextureSample", "texture"),
        ("stdlib::TextureObject", "texture"),
        ("param::TextureSample2D", "default"),
        ("param::TextureObject", "default"),
    ] {
        let p = reg.get(kind).unwrap().params().into_iter().find(|p| p.id == param).expect(kind);
        assert_eq!((p.kind, p.default), (ParamKind::Texture, Value::Str(String::new())), "{kind}");
    }
}

#[test]
fn texture_object_node_is_a_fixed_texture() {
    let reg = shader_stdlib::registry();
    let info = reg.get("stdlib::TextureObject").unwrap().info().clone();
    assert_eq!(info.category, "Texture");
    assert!(info.keywords.iter().any(|k| k == "texture object") && info.keywords.iter().any(|k| k == "tex"));

    let mut a = unlit();
    let t = node(&mut a, "t", "stdlib::TextureObject", &[("texture", "textures/rock")]);
    let s1 = node(&mut a, "s1", "stdlib::TextureSample", &[]);
    let size = node(&mut a, "size", "stdlib::TextureSize", &[]);
    a.connect((&t, "out"), (&s1, "tex"));
    a.connect((&t, "out"), (&size, "tex"));
    a.connect((&size, "out"), (&s1, "uv"));
    a.connect((&s1, "rgb"), ("out", "color"));
    let (ir, diags) = build(&a);
    assert!(w110_at(&diags).is_empty(), "{diags:?}");
    assert_eq!(ir.textures, vec![fixed("t", "textures/rock", 101)], "one binding per node, not per use");

    // Without a texture: W110 at the TextureObject.
    a.nodes.get_mut(&t).unwrap().params.clear();
    let (ir, diags) = build(&a);
    assert_eq!(w110_at(&diags), ["t"]);
    assert_eq!(ir.textures, vec![fixed("t", DEFAULT_TEXTURE, 101)]);
}

#[test]
fn missing_texture_asset_is_e006() {
    for kind in ["stdlib::TextureSample", "param::TextureSample2D"] {
        let mut a = unlit();
        let param = if kind.starts_with("param::") { "default" } else { "texture" };
        let s = node(&mut a, "s", kind, &[("name", "albedo"), (param, "textures/nope")]);
        a.connect((&s, "rgb"), ("out", "color"));
        let r = build_raw(&a);
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == Code::E006 && d.node.as_deref() == Some("s") && d.message.contains("textures/nope")),
            "{kind}: {:?}",
            r.diagnostics
        );
    }
}

#[test]
fn internal_texture_needs_no_project_asset() {
    let mut a = unlit();
    let s = node(&mut a, "s", "stdlib::TextureSample", &[("texture", DEFAULT_TEXTURE)]);
    a.connect((&s, "rgb"), ("out", "color"));
    let (ir, diags) = build(&a);
    assert!(w110_at(&diags).is_empty(), "{diags:?}");
    assert_eq!(ir.textures, vec![fixed("s", DEFAULT_TEXTURE, 101)]);
}

#[test]
fn texture_sample_parameter_has_mip_mode() {
    let mut a = unlit();
    let p = node(&mut a, "p", "param::TextureSample2D", &[("name", "albedo"), ("mip_mode", "Level")]);
    let c = node(&mut a, "c", "stdlib::Constant", &[]);
    a.nodes.get_mut(&c).unwrap().params.insert("value".into(), Value::Float(2.0));
    a.connect((&c, "out"), (&p, "mip"));
    a.connect((&p, "rgb"), ("out", "color"));
    let (ir, _) = build(&a);
    assert!(
        ir.main.lines.iter().any(|l| l.expr.contains("textureSampleLevel(inp_albedo, inp_albedo_sampler, ")),
        "{:?}",
        ir.main.lines
    );
    let reg = shader_stdlib::registry();
    let mode = reg.get("param::TextureSample2D").unwrap().params().into_iter().find(|p| p.id == "mip_mode");
    let sample_mode = reg.get("stdlib::TextureSample").unwrap().params().into_iter().find(|p| p.id == "mip_mode");
    assert_eq!(mode, sample_mode);
}

/// A fixed texture inside a function is a hidden trailing texture + sampler input of the function (design §1.3), named
/// after its origin function and node: `ft_{path with / -> _}_{node}`.
#[test]
fn fixed_texture_inside_function_is_a_hidden_input() {
    let mut a = Asset::new("f", AssetKind::Function);
    let s = node(&mut a, "s", "stdlib::TextureSample", &[("texture", "textures/rock")]);
    let out = node(&mut a, "out", "output::FunctionOutput", &[("name", "result")]);
    a.connect((&s, "r"), (&out, "value"));
    let (ir, _) = build(&a);
    let sig = ir.function_sig.as_ref().expect("function sig");
    assert_eq!(
        sig.textures,
        vec![HiddenTexture { origin: "test/mat".into(), node: "s".into(), texture: "textures/rock".into() }]
    );
    assert!(ir.textures.is_empty(), "functions have no material bindings");
    assert!(
        ir.main.lines.iter().any(|l| l.expr.contains("textureSample(ft_test_mat_s, ft_test_mat_s_sampler, ")),
        "{:?}",
        ir.main
    );
}

fn layer_sig(path: &str, node: &str) -> FunctionSig {
    FunctionSig {
        path: path.into(),
        name: path.rsplit('/').next().unwrap().into(),
        inputs: vec![],
        outputs: vec![FnPort {
            name: "rgb".into(),
            ty: PinType::Float3,
            default: Value::Float(0.0),
            required: false,
            range: None,
        }],
        builtins: vec![],
        textures: vec![HiddenTexture { origin: path.into(), node: node.into(), texture: "textures/rock".into() }],
    }
}

/// A caller passes the callee's hidden textures; in a material they are bindings `Fixed { origin: Some(fn) }`, one
/// per origin node however often the function is called.
#[test]
fn material_binds_hidden_textures_of_called_functions() {
    let mut fns = BTreeMap::new();
    fns.insert("a/layer".to_string(), layer_sig("a/layer", "n1"));
    let mut a = unlit();
    let c1 = node(&mut a, "c1", "function::a/layer", &[]);
    let c2 = node(&mut a, "c2", "function::a/layer", &[]);
    let add = node(&mut a, "add", "stdlib::Add", &[]);
    a.connect((&c1, "rgb"), (&add, "a"));
    a.connect((&c2, "rgb"), (&add, "b"));
    a.connect((&add, "out"), ("out", "color"));
    let r = build_with(&a, &fns);
    assert!(!r.has_errors(), "{:?}", r.diagnostics);
    let ir = r.ir.unwrap();
    assert_eq!(
        ir.textures,
        vec![TextureBinding {
            ident: "ft_a_layer_n1".into(),
            source: TextureSource::Fixed { node: "n1".into(), origin: Some("a/layer".into()) },
            texture: "textures/rock".into(),
            binding: 101,
        }]
    );
    let calls = ir.main.lines.iter().filter(|l| l.expr == "fun_layer(ft_a_layer_n1, ft_a_layer_n1_sampler)").count();
    assert_eq!(calls, 2, "{:?}", ir.main);
}

/// `ft_{fn}_{node}` maps `/` to `_`, so `a/b_c` and `a_b/c` could produce the same identifier for the same node id:
/// reported (E006) instead of generating a shader with a duplicate binding.
#[test]
fn hidden_texture_ident_clash_is_e006() {
    let mut fns = BTreeMap::new();
    fns.insert("a/b_c".to_string(), layer_sig("a/b_c", "n1"));
    fns.insert("a_b/c".to_string(), layer_sig("a_b/c", "n1"));
    let mut a = unlit();
    let c1 = node(&mut a, "c1", "function::a/b_c", &[]);
    let c2 = node(&mut a, "c2", "function::a_b/c", &[]);
    let add = node(&mut a, "add", "stdlib::Add", &[]);
    a.connect((&c1, "rgb"), (&add, "a"));
    a.connect((&c2, "rgb"), (&add, "b"));
    a.connect((&add, "out"), ("out", "color"));
    let r = build_with(&a, &fns);
    assert!(
        r.diagnostics.iter().any(|d| d.code == Code::E006 && d.message.contains("`ft_a_b_c_n1`")),
        "{:?}",
        r.diagnostics
    );
}

#[test]
fn parameters_are_sorted_by_name() {
    // Node ids run against the names: the order comes from the names alone (decision 0027).
    let mut a = unlit();
    let z = node(&mut a, "a", "param::TextureSample2D", &[("name", "zeta"), ("default", "textures/rock")]);
    let y = node(&mut a, "b", "param::TextureSample2D", &[("name", "alpha"), ("default", "textures/moss")]);
    let add = node(&mut a, "c", "stdlib::Add", &[]);
    a.connect((&z, "rgb"), (&add, "a"));
    a.connect((&y, "rgb"), (&add, "b"));
    a.connect((&add, "out"), ("out", "color"));
    let (ir, _) = build(&a);
    let names: Vec<&str> = ir.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["alpha", "zeta"]);
    let bindings: Vec<(&str, u32)> = ir.textures.iter().map(|t| (t.ident.as_str(), t.binding)).collect();
    assert_eq!(bindings, [("inp_alpha", 101), ("inp_zeta", 103)]);
}

#[test]
fn layout_feature_ignores_which_fixed_node_is_wired() {
    // Decision 0029: the game matches texture bindings by number, so wiring another TextureSample in keeps the
    // layout; another binding or parameter changes it.
    let mut a = unlit();
    let s1 = node(&mut a, "s1", "stdlib::TextureSample", &[("texture", "textures/rock")]);
    let s2 = node(&mut a, "s2", "stdlib::TextureSample", &[("texture", "textures/moss")]);
    a.connect((&s1, "rgb"), ("out", "color"));
    let (first, _) = build(&a);
    assert!(first.layout_feature().starts_with("SG_LAYOUT_"), "{}", first.layout_feature());

    a.connect((&s2, "rgb"), ("out", "color"));
    let (swapped, _) = build(&a);
    assert_eq!(swapped.textures, vec![fixed("s2", "textures/moss", 101)]);
    assert_eq!(swapped.layout_feature(), first.layout_feature());

    let add = node(&mut a, "add", "stdlib::Add", &[]);
    a.connect((&s1, "rgb"), (&add, "a"));
    a.connect((&s2, "rgb"), (&add, "b"));
    a.connect((&add, "out"), ("out", "color"));
    let (two, _) = build(&a);
    assert_eq!(two.textures.len(), 2);
    assert_ne!(two.layout_feature(), first.layout_feature());

    let p = node(&mut a, "p", "param::Scalar", &[("name", "gain")]);
    let mul = node(&mut a, "mul", "stdlib::Multiply", &[]);
    a.connect((&s1, "rgb"), (&mul, "a"));
    a.connect((&p, "out"), (&mul, "b"));
    a.connect((&mul, "out"), ("out", "color"));
    let (param, _) = build(&a);
    assert_eq!(param.textures.len(), 1);
    assert_ne!(param.layout_feature(), first.layout_feature());
}
