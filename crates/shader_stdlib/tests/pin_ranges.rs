//! Value ranges of input pins (drive the inline sliders in the editor).

use std::collections::BTreeMap;

use shader_graph_core::model::{AssetKind, MaterialKind, PbrSettings};
use shader_graph_core::registry::{Env, NoFunctions, SigCtx, Signature};

fn signature(kind: &str) -> Signature {
    let reg = shader_stdlib::registry();
    let def = reg.get(kind).unwrap();
    let params = reg.resolved_params(kind, &BTreeMap::new());
    let (connected, inputs) = (BTreeMap::new(), BTreeMap::new());
    let env = Env {
        asset_kind: AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())),
        asset_path: "test",
        functions: &NoFunctions,
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
