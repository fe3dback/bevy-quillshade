//! Value ranges of input pins (drive the inline sliders in the editor).

use std::collections::BTreeMap;

use shader_graph_core::consts::NoConsts;
use shader_graph_core::ir::declared_function_sig;
use shader_graph_core::model::{Asset, AssetKind, MaterialKind, Node, PbrSettings};
use shader_graph_core::registry::{
    Env, FunctionSig, NoCurves, NoFunctions, NoGradients, NoTextures, ParamKind, SigCtx, Signature,
};
use shader_graph_core::{PinType, Value};

fn signature(kind: &str) -> Signature {
    let reg = shader_stdlib::registry();
    let def = reg.get(kind).unwrap();
    let params = reg.resolved_params(kind, &BTreeMap::new());
    let (connected, inputs) = (BTreeMap::new(), BTreeMap::new());
    let env = Env {
        asset_kind: AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())),
        asset_path: "test",
        functions: &NoFunctions,
        curves: &NoCurves,
        consts: &NoConsts,
        textures: &NoTextures,
        gradients: &NoGradients,
    };
    def.signature(&SigCtx {
        node_id: "n",
        kind,
        params: &params,
        connected: &connected,
        input_values: &inputs,
        env: &env,
    })
}

#[test]
fn lerp_alpha_has_unit_range() {
    let sig = signature("stdlib::Lerp");
    assert_eq!(sig.input("alpha").unwrap().range, Some((0.0, 1.0)));
    assert_eq!(sig.input("a").unwrap().range, None);
}

#[test]
fn fractions_have_unit_range() {
    assert_eq!(signature("stdlib::Fresnel").input("base").unwrap().range, Some((0.0, 1.0)));
    assert_eq!(signature("stdlib::Fresnel").input("exponent").unwrap().range, None);
    assert_eq!(signature("stdlib::Desaturation").input("fraction").unwrap().range, Some((0.0, 1.0)));
}

/// A function with one `output::FunctionInput` named `x` of `ty`, with the given extra params.
fn function_with_input(ty: PinType, extra: &[(&str, Value)]) -> FunctionSig {
    let reg = shader_stdlib::registry();
    let mut a = Asset::new("f", AssetKind::Function);
    let mut n = Node::new("output::FunctionInput", (0.0, 0.0))
        .with_param("name", Value::Str("x".into()))
        .with_param("type", Value::Type(ty));
    for (k, v) in extra {
        n = n.with_param(k, v.clone());
    }
    a.add_node(n);
    declared_function_sig(&a, "lib/f", &reg)
}

/// Signature of a `function::lib/f` call node seeing `f`.
fn call_signature(f: FunctionSig) -> Signature {
    let reg = shader_stdlib::registry();
    let kind = "function::lib/f";
    let def = reg.get(kind).unwrap();
    let mut fns = BTreeMap::new();
    fns.insert("lib/f".to_string(), f);
    let (params, connected, inputs) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    let env = Env {
        asset_kind: AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())),
        asset_path: "test",
        functions: &fns,
        curves: &NoCurves,
        consts: &NoConsts,
        textures: &NoTextures,
        gradients: &NoGradients,
    };
    def.signature(&SigCtx {
        node_id: "n",
        kind,
        params: &params,
        connected: &connected,
        input_values: &inputs,
        env: &env,
    })
}

/// Design §4.10: a Float FunctionInput with `min < max` gives the call node's pin a slider range.
#[test]
fn function_input_range_reaches_call_pin() {
    let f = function_with_input(PinType::Float, &[("min", Value::Float(0.2)), ("max", Value::Float(5.0))]);
    assert_eq!(f.inputs[0].range, Some((0.2, 5.0)));
    assert_eq!(call_signature(f).input("x").unwrap().range, Some((0.2, 5.0)));
}

#[test]
fn function_input_without_range() {
    let f = function_with_input(PinType::Float, &[]);
    assert_eq!(f.inputs[0].range, None, "defaults: min == max");
    assert_eq!(call_signature(f).input("x").unwrap().range, None);
    let f = function_with_input(PinType::Float, &[("min", Value::Float(5.0)), ("max", Value::Float(1.0))]);
    assert_eq!(f.inputs[0].range, None, "min > max: no slider");
    let f = function_with_input(PinType::Float3, &[("min", Value::Float(0.0)), ("max", Value::Float(1.0))]);
    assert_eq!(f.inputs[0].range, None, "only Float inputs have a range");
}

#[test]
fn function_input_min_max_hidden_for_non_float() {
    let reg = shader_stdlib::registry();
    let kind = "output::FunctionInput";
    let defs = reg.get(kind).unwrap().params();
    let shown = |ty: PinType| -> Vec<String> {
        let params = reg.resolved_params(kind, &BTreeMap::from([("type".to_string(), Value::Type(ty))]));
        defs.iter().filter(|p| p.visible(&params)).map(|p| p.id.clone()).collect()
    };
    assert_eq!(shown(PinType::Float), ["name", "type", "default", "required", "order", "min", "max"]);
    assert_eq!(shown(PinType::Float3), ["name", "type", "default", "required", "order"]);
    // Defaults are never written into the node: an unset range stays absent from the file.
    let p = defs.iter().find(|p| p.id == "min").unwrap();
    assert_eq!((p.kind.clone(), p.default.clone()), (ParamKind::Float { min: None, max: None }, Value::Float(0.0)));
}
