//! Gradient collection bindings in WESL (gradient design §7): one material binding per collection with no owning node,
//! and hidden trailing `gc_*` texture + sampler parameters of a function after its fixed textures.

use std::collections::{BTreeMap, BTreeSet};

use shader_codegen_wesl::GenCtx;
use shader_graph_core::consts::NoConsts;
use shader_graph_core::gradient::GradientCollection;
use shader_graph_core::ir::build_ir;
use shader_graph_core::model::{Asset, AssetKind, MaterialKind, Node, UnlitSettings};
use shader_graph_core::registry::{FunctionSig, NoCurves};
use shader_graph_core::{Ir, PinType, Value};
use shader_stdlib::nodes::color::{GRADIENT_COLLECTION_PARAM, GRADIENT_SLOT_PARAM, SAMPLE_GRADIENT_KIND};

const TERRAIN: &str = "common/terrain";

fn collections() -> BTreeMap<String, GradientCollection> {
    let mut next = 0;
    let mut c = GradientCollection::new(|| {
        next += 1;
        next
    });
    c.slots[0].id = 11;
    BTreeMap::from([(TERRAIN.to_string(), c)])
}

fn add(a: &mut Asset, id: &str, n: Node) -> String {
    a.nodes.insert(id.to_string(), n);
    id.to_string()
}

fn sample_gradient() -> Node {
    Node::new(SAMPLE_GRADIENT_KIND, (0.0, 0.0))
        .with_param(GRADIENT_COLLECTION_PARAM, Value::Str(TERRAIN.into()))
        .with_param(GRADIENT_SLOT_PARAM, Value::Int(11))
}

fn build(a: &Asset, path: &str, functions: &BTreeMap<String, FunctionSig>) -> Ir {
    let reg = shader_stdlib::registry();
    let r = build_ir(a, path, &reg, functions, &NoCurves, &NoConsts, &BTreeSet::<String>::new(), &collections());
    assert!(!r.has_errors(), "{:?}", r.diagnostics);
    r.ir.expect("ir")
}

/// Function `common/g`: a fixed texture (`TextureSample` on the internal default) times a Sample Gradient.
fn function_ir() -> Ir {
    let mut f = Asset::new("g", AssetKind::Function);
    let g = add(&mut f, "n1", sample_gradient());
    let t = add(
        &mut f,
        "n2",
        Node::new("stdlib::TextureSample", (0.0, 0.0)).with_param("texture", Value::Str("@internal/default".into())),
    );
    let mul = add(&mut f, "n3", Node::new("stdlib::Multiply", (0.0, 0.0)));
    let out = add(
        &mut f,
        "out",
        Node::new("output::FunctionOutput", (0.0, 0.0))
            .with_param("name", Value::Str("rgb".into()))
            .with_param("type", Value::Type(PinType::Float3)),
    );
    f.connect((&g, "rgb"), (&mul, "a"));
    f.connect((&t, "rgb"), (&mul, "b"));
    f.connect((&mul, "out"), (&out, "value"));
    build(&f, "common/g", &BTreeMap::new())
}

#[test]
fn material_gradient_binding_has_no_owning_node() {
    let mut a = Asset::new("mat", AssetKind::Material(MaterialKind::Unlit3d(UnlitSettings::default())));
    add(&mut a, "out", Node::new("output::Unlit3d", (0.0, 0.0)));
    let g = add(&mut a, "g", sample_gradient());
    a.connect((&g, "rgb"), ("out", "color"));
    let ir = build(&a, "test/mat", &BTreeMap::new());
    let fns = BTreeMap::new();
    let wesl = shader_codegen_wesl::material(&ir, &GenCtx { functions: &fns, source_file: "shaders/test/mat.ron" });
    let lines: Vec<&str> = wesl.text.lines().collect();
    for (suffix, binding) in [
        ("@binding(101) var gc_common_terrain: texture_2d<f32>;", 101),
        ("@binding(102) var gc_common_terrain_sampler: sampler;", 102),
    ] {
        let at = lines.iter().position(|l| l.ends_with(suffix)).unwrap_or_else(|| panic!("{binding}\n{}", wesl.text));
        assert!(lines[at].starts_with("@group(constants::MATERIAL_BIND_GROUP) "), "{}", lines[at]);
        assert_eq!(wesl.node_at_line(at + 1), None, "binding {binding} belongs to no node");
    }
    assert!(wesl.text.contains("sg_sample_gradient(gc_common_terrain, gc_common_terrain_sampler, "), "{}", wesl.text);
    assert!(wesl.text.contains("import super::super::stdlib::texture::sg_sample_gradient;"), "{}", wesl.text);
}

#[test]
fn function_gradient_params_follow_fixed_textures() {
    let ir = function_ir();
    let sig = ir.function_sig.as_ref().unwrap();
    assert_eq!(sig.gradients, [TERRAIN]);
    let fns = BTreeMap::new();
    let wesl = shader_codegen_wesl::function(&ir, &GenCtx { functions: &fns, source_file: "shaders/common/g.ron" });
    let want = "fn fun_g(b_uv0: vec2<f32>, ft_common_g_n2: texture_2d<f32>, ft_common_g_n2_sampler: sampler, \
                gc_common_terrain: texture_2d<f32>, gc_common_terrain_sampler: sampler) -> vec3<f32> {";
    assert!(wesl.text.contains(want), "{want}\n{}", wesl.text);
}
