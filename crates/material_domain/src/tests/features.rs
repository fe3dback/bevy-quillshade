//! Game Features tables and `stdlib::GameFeatureSwitch` editor integration tests.

use super::*;
use crate::doc::node_signature_with;

#[test]
fn features_tab_adds_edits_and_branches_in_graph() {
    let (dir, mut e) = project();
    e.apply(new_asset("features", 0));
    e.apply(Action::Submit("common/quality".into()));
    let quality = AssetId::new(AssetType::Features, "common/quality");
    assert_eq!(e.features_index(&quality), e.active);
    assert!(e.features_doc().unwrap().table.enums.is_empty());

    // Add an enum `render_quality` (starts with `low` and `high` variants).
    set(&mut e, FeaturesProp::DraftEnumName, Value::Str("render_quality".into()));
    e.apply(Action::from(FeaturesAction::AddEnum));
    let enum_id = e.features_doc().unwrap().table.enums[0].id;
    assert_eq!(e.features_doc().unwrap().table.enums[0].variants.len(), 2);

    // Add a third variant `ultra`.
    set(&mut e, FeaturesProp::DraftVariantName(enum_id), Value::Str("ultra".into()));
    e.apply(Action::from(FeaturesAction::AddVariant { enum_id }));
    assert_eq!(e.features_doc().unwrap().table.enums[0].variants.len(), 3);
    let variants: Vec<(project_core::AssetUuid, String)> =
        e.features_doc().unwrap().table.enums[0].variants.iter().map(|v| (v.id, v.name.clone())).collect();
    assert_eq!(variants.iter().map(|v| v.1.as_str()).collect::<Vec<_>>(), ["low", "high", "ultra"]);

    e.apply(Action::Save);
    let on_disk = assets::load_features(e.project.as_ref().unwrap(), &quality).unwrap();
    assert_eq!(on_disk.enums.len(), 1);
    assert_eq!(on_disk.enums[0].variants.len(), 3);

    // Open a material, drop the features table onto the graph, and select `render_quality`.
    let mat_id = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(Action::OpenAsset(mat_id.to_ref()));
    e.apply(MaterialAction::Drop {
        item: crate::ops::droppable::Dropped::Features("common/quality".into()),
        pos: (100.0, 100.0),
        node: None,
    });
    let sw = e
        .doc()
        .unwrap()
        .asset
        .nodes
        .iter()
        .find(|(_, n)| n.kind == "stdlib::GameFeatureSwitch")
        .map(|(k, _)| k.clone())
        .expect("GameFeatureSwitch node added");
    set(&mut e, GraphProp::Param { node: sw.clone(), key: "feature".into() }, Value::Str(enum_id.to_string()));

    // Verify the resolved signature has the 3 variant pins (`Low`, `High`, `Ultra`).
    let (_, view) = e.doc_view().unwrap();
    let sig = node_signature_with(
        &view,
        &sw,
        &e.domains.material().registry,
        &e.domains.material().functions,
        &e.domains.material().features,
        &mat_id.path,
    )
    .unwrap();
    let pin_labels: Vec<&str> = sig.inputs.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(pin_labels, ["Low", "High", "Ultra"]);

    // Connect a constant 3-vector to `Low`, and connect `sw.out` to `MaterialOutput.base_color`.
    let out_node = e.doc().unwrap().asset.nodes.iter().find(|(k, _)| e.is_output_node(k)).unwrap().0.clone();
    e.apply(MaterialAction::AddNode {
        kind: "stdlib::Constant3Vector".into(),
        pos: (0.0, 100.0),
        link: None,
        params: vec![("value".into(), Value::Float3([0.1, 0.8, 0.2]))],
    });
    let c3 = e.doc().unwrap().selection.iter().next().unwrap().clone();
    e.apply(GraphEdit::Connect { from: (c3, "out".into()), to: (sw.clone(), variants[0].0.to_string()) });
    e.apply(GraphEdit::Connect { from: (sw.clone(), "out".into()), to: (out_node, "base_color".into()) });
    e.apply(Action::Save);

    // Check generated WESL and Rust files for permutation support and `features.rs`.
    let wesl = std::fs::read_to_string(dir.path().join("assets/quillshade/material/autogen/dir_env/mat_rock_wet.wesl"))
        .unwrap();
    let def1 = on_disk.enums[0].shader_def(1);
    let def2 = on_disk.enums[0].shader_def(2);
    assert!(wesl.contains(&format!("@if({def1})")), "{wesl}");
    assert!(wesl.contains(&format!("@elif({def2})")), "{wesl}");

    let rs =
        std::fs::read_to_string(dir.path().join("crates/quillshade/src/material/materials/dir_env/mat_rock_wet.rs"))
            .unwrap();
    assert!(rs.contains("#[bind_group_data(u32)]"), "{rs}");
    assert!(rs.contains("pub permutation: u32"), "{rs}");

    let features_rs = std::fs::read_to_string(dir.path().join("crates/quillshade/src/material/features.rs")).unwrap();
    assert!(features_rs.contains("pub struct QuillshadeMaterialFeatures"), "{features_rs}");
    assert!(features_rs.contains("pub enum RenderQuality"), "{features_rs}");

    // Deleting the connected variant `low` prompts for confirmation and disconnects its wire cleanly.
    e.apply(Action::OpenAsset(quality.to_ref()));
    e.apply(Action::from(FeaturesAction::DeleteVariant { enum_id, variant_id: variants[0].0, confirmed: false }));
    assert!(e.dialog.is_some(), "confirmation required before deleting connected variant");
    e.apply(confirm(&e));
    assert!(e.dialog.is_none());
    assert_eq!(e.features_doc().unwrap().table.enums[0].variants.len(), 2);

    // Deleting the enum while a material reads it prompts for confirmation and then clears the reference.
    assert_eq!(e.feature_enum_usages("common/quality", &[enum_id]), vec![(mat_id.clone(), 1)]);
    e.apply(Action::from(FeaturesAction::DeleteEnum { enum_id, confirmed: false }));
    assert!(e.dialog.is_some(), "confirmation required before deleting enum in use");
    e.apply(confirm(&e));
    assert!(e.dialog.is_none());
    assert!(e.feature_enum_usages("common/quality", &[enum_id]).is_empty());
}
