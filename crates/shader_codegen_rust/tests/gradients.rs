//! Gradient collection bindings in the generated Rust (gradient design §7): a `Handle<Image>` field per collection,
//! filled with `Handle::default()` by `new`; the collection id appears only in the field's doc comment.

use std::collections::{BTreeMap, BTreeSet};

use shader_graph_core::consts::NoConsts;
use shader_graph_core::gradient::GradientCollection;
use shader_graph_core::ir::build_ir;
use shader_graph_core::model::{Asset, AssetKind, MaterialKind, Node, UnlitSettings};
use shader_graph_core::registry::NoCurves;
use shader_graph_core::{Ir, Value};
use shader_stdlib::nodes::color::{GRADIENT_COLLECTION_PARAM, GRADIENT_SLOT_PARAM, SAMPLE_GRADIENT_KIND};

const TERRAIN: &str = "common/terrain";

fn material_ir(collection: &str) -> Ir {
    let mut next = 0;
    let mut c = GradientCollection::new(|| {
        next += 1;
        next
    });
    c.slots[0].id = 11;
    let collections = BTreeMap::from([(collection.to_string(), c)]);
    let mut a = Asset::new("mat", AssetKind::Material(MaterialKind::Unlit3d(UnlitSettings::default())));
    a.nodes.insert("out".into(), Node::new("output::Unlit3d", (0.0, 0.0)));
    a.nodes.insert(
        "g".into(),
        Node::new(SAMPLE_GRADIENT_KIND, (0.0, 0.0))
            .with_param(GRADIENT_COLLECTION_PARAM, Value::Str(collection.into()))
            .with_param(GRADIENT_SLOT_PARAM, Value::Int(11)),
    );
    a.connect(("g", "rgb"), ("out", "color"));
    let reg = shader_stdlib::registry();
    let r = build_ir(
        &a,
        "test/mat",
        &reg,
        &BTreeMap::new(),
        &NoCurves,
        &NoConsts,
        &BTreeSet::<String>::new(),
        &collections,
    );
    assert!(!r.has_errors(), "{:?}", r.diagnostics);
    r.ir.expect("ir")
}

#[test]
fn gradient_field_and_new() {
    let rs = shader_codegen_rust::material_module(&material_ir(TERRAIN), "quillshade/material/test/mat.material.ron");
    for l in [
        "    /// Gradient collection `common/terrain`; `new` sets `Handle::default()` (Bevy's white image).\n    \
         #[texture(101)]\n    #[sampler(102)]\n    pub gc_common_terrain: Handle<Image>,",
        "            gc_common_terrain: Handle::default(),",
    ] {
        assert!(rs.contains(l), "{l}\n{rs}");
    }
    assert_eq!(rs.matches(TERRAIN).count(), 1, "the collection id is only in the doc comment\n{rs}");
}
