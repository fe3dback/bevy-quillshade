//! Drag and drop onto graphs, Convert to Parameter, Texture tab presets (design §4.3, §4.6).

use super::*;

pub(super) fn game_file(dir: &tempfile::TempDir, e: &mut Editor, rel: &str) {
    let p = dir.path().join("assets").join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, b"x").unwrap();
    e.refresh_game_files();
}

pub(super) fn drop_file(e: &mut Editor, path: &str, pos: (f32, f32), node: Option<&str>) {
    let item = crate::ops::droppable::Dropped::GameFile(path.into());
    e.apply(MaterialAction::Drop { item, pos, node: node.map(String::from) });
}

pub(super) fn nodes_of(e: &Editor, kind: &str) -> Vec<NodeId> {
    e.doc().unwrap().asset.nodes.iter().filter(|(_, n)| n.kind == kind).map(|(k, _)| k.clone()).collect()
}

pub(super) fn param_of(e: &Editor, node: &str, key: &str) -> Option<Value> {
    e.doc().unwrap().asset.nodes[node].params.get(key).cloned()
}

pub(super) fn open_rock_wet(e: &mut Editor) {
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
}

#[test]
fn drop_image_creates_texture_asset_and_node() {
    let (dir, mut e) = project();
    game_file(&dir, &mut e, "textures/env/Rock-Albedo.png");
    open_rock_wet(&mut e);
    assert!(nodes_of(&e, "stdlib::TextureSample").is_empty());
    drop_file(&mut e, "textures/env/Rock-Albedo.png", (12.0, 34.0), None);

    let id = AssetId::new(AssetType::Texture, "textures/env/rock_albedo");
    assert!(dir.path().join("quillshade/material/textures/env/rock_albedo.texture.ron").is_file());
    assert_eq!(e.domains.material().disk.textures[&id].as_ref().unwrap().path, "textures/env/Rock-Albedo.png");
    let added = nodes_of(&e, "stdlib::TextureSample");
    assert_eq!(added.len(), 1);
    let d = e.doc().unwrap();
    assert_eq!(d.asset.nodes[&added[0]].pos, (10.0, 30.0), "snapped to the 10-unit grid");
    assert_eq!(param_of(&e, &added[0], "texture"), Some(Value::Str("textures/env/rock_albedo".into())));
    assert_eq!(e.doc().unwrap().selection, [added[0].clone()].into());
    assert_eq!(e.referenced_asset(&added[0]), Some(id));
}

#[test]
fn drop_image_reuses_existing_texture_asset() {
    use crate::ops::droppable::Dropped;
    use crate::ui::tree::{TreeItem, TreeRow};
    let (dir, mut e) = project();
    // The fixture `textures/rock_albedo` points at this file.
    game_file(&dir, &mut e, "textures/rock_albedo.ktx2");
    open_rock_wet(&mut e);
    let count = e.domains.material().disk.textures.len();
    drop_file(&mut e, "textures/rock_albedo.ktx2", (0.0, 0.0), None);
    assert_eq!(e.domains.material().disk.textures.len(), count, "no new Texture asset");
    let added = nodes_of(&e, "stdlib::TextureSample");
    assert_eq!(param_of(&e, &added[0], "texture"), Some(Value::Str("textures/rock_albedo".into())));

    // Rows that can be dragged onto a graph: images, Texture assets, built-in textures.
    let row = |tree, item| TreeRow { tree, item };
    let tex = AssetId::new(AssetType::Texture, "textures/atlas");
    let mat = AssetId::new(AssetType::Material, "env/rock_wet");
    assert_eq!(
        Dropped::from_row(&row(TreeId::GameAssets, TreeItem::GameFile("a/b.PNG".into()))),
        Some(Dropped::GameFile("a/b.PNG".into()))
    );
    assert_eq!(
        Dropped::from_row(&row(TreeId::Project(MATERIAL), TreeItem::Asset(tex.to_ref()))),
        Some(Dropped::Texture("textures/atlas".into()))
    );
    assert_eq!(
        Dropped::from_row(&row(
            TreeId::Project(MATERIAL),
            TreeItem::Virtual(AssetId::new(AssetType::Texture, "@internal/default").to_ref())
        )),
        Some(Dropped::Texture("@internal/default".into()))
    );
    assert_eq!(Dropped::from_row(&row(TreeId::Project(MATERIAL), TreeItem::Asset(mat.to_ref()))), None);
    assert_eq!(Dropped::from_row(&row(TreeId::GameAssets, TreeItem::GameFile("a/b.txt".into()))), None);
    assert_eq!(Dropped::from_row(&row(TreeId::GameAssets, TreeItem::Folder("a".into()))), None);

    // A Texture asset from the Project tree: the same node, nothing created.
    e.apply(MaterialAction::Drop { item: Dropped::Texture("textures/atlas".into()), pos: (5.0, 5.0), node: None });
    assert_eq!(e.domains.material().disk.textures.len(), count);
    let added = nodes_of(&e, "stdlib::TextureSample");
    assert_eq!(added.len(), 2);
    assert!(added.iter().any(|n| param_of(&e, n, "texture") == Some(Value::Str("textures/atlas".into()))));
}

#[test]
fn drop_on_texture_node_sets_reference() {
    let (dir, mut e) = project();
    game_file(&dir, &mut e, "textures/env/Rock-Albedo.png");
    open_rock_wet(&mut e);
    let albedo = nodes_of(&e, "param::TextureSample2D")
        .into_iter()
        .find(|n| param_of(&e, n, "name") == Some(Value::Str("albedo".into())))
        .unwrap();
    let count = e.doc().unwrap().asset.nodes.len();
    drop_file(&mut e, "textures/env/Rock-Albedo.png", (0.0, 0.0), Some(&albedo));
    assert_eq!(e.doc().unwrap().asset.nodes.len(), count, "no node added");
    assert_eq!(param_of(&e, &albedo, "default"), Some(Value::Str("textures/env/rock_albedo".into())));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(param_of(&e, &albedo, "default"), Some(Value::Str("textures/rock_albedo".into())));

    // A plain TextureSample: its `texture` param; a built-in texture works the same way.
    let sample = add(&mut e, "stdlib::TextureSample");
    let item = crate::ops::droppable::Dropped::Texture("@internal/default".into());
    e.apply(MaterialAction::Drop { item, pos: (0.0, 0.0), node: Some(sample.clone()) });
    assert_eq!(param_of(&e, &sample, "texture"), Some(Value::Str("@internal/default".into())));
    drop_file(&mut e, "textures/env/Rock-Albedo.png", (0.0, 0.0), Some(&sample));
    assert_eq!(param_of(&e, &sample, "texture"), Some(Value::Str("textures/env/rock_albedo".into())));
    assert_eq!(nodes_of(&e, "stdlib::TextureSample"), vec![sample]);
}

#[test]
fn drop_on_other_node_adds_new_node() {
    let (dir, mut e) = project();
    game_file(&dir, &mut e, "textures/env/Rock-Albedo.png");
    open_rock_wet(&mut e);
    let out = output_node(&e);
    let before = e.doc().unwrap().asset.nodes[&out].clone();
    drop_file(&mut e, "textures/env/Rock-Albedo.png", (40.0, 50.0), Some(&out));
    assert_eq!(e.doc().unwrap().asset.nodes[&out], before, "the output node is unchanged");
    let added = nodes_of(&e, "stdlib::TextureSample");
    assert_eq!(added.len(), 1);
    assert_eq!(e.doc().unwrap().asset.nodes[&added[0]].pos, (40.0, 50.0));
    assert_eq!(param_of(&e, &added[0], "texture"), Some(Value::Str("textures/env/rock_albedo".into())));
}

/// Design §4.11: a Function asset of the Project tree dropped onto a graph adds a call node at the drop point.
#[test]
fn drop_function_adds_call_node() {
    use crate::ops::droppable::Dropped;
    use crate::ui::tree::{TreeItem, TreeRow};
    let (_dir, mut e) = project();
    let row = |id: AssetId| TreeRow { tree: TreeId::Project(MATERIAL), item: TreeItem::Asset(id.to_ref()) };
    let mask = AssetId::new(AssetType::Function, "common/wet_mask");
    assert_eq!(Dropped::from_row(&row(mask)), Some(Dropped::Function("common/wet_mask".into())));
    let consts = AssetId::new(AssetType::Consts, "common/globals");
    assert_eq!(Dropped::from_row(&row(consts)), None, "other asset kinds are not droppable");

    open_rock_wet(&mut e);
    let kind = "function::common/wet_mask";
    let before = nodes_of(&e, kind).len();
    let out = output_node(&e);
    let item = Dropped::Function("common/wet_mask".into());
    // Over another node: still a new node at the drop point.
    e.apply(MaterialAction::Drop { item, pos: (12.0, 34.0), node: Some(out) });
    let added: Vec<NodeId> =
        nodes_of(&e, kind).into_iter().filter(|n| e.doc().unwrap().selection.contains(n)).collect();
    assert_eq!(nodes_of(&e, kind).len(), before + 1);
    assert_eq!(added.len(), 1, "the new call node is selected");
    assert_eq!(e.doc().unwrap().asset.nodes[&added[0]].pos, (10.0, 30.0), "snapped to the 10-unit grid");
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(nodes_of(&e, kind).len(), before, "one undo step");
}

/// Design §4.11: a function graph cannot call itself, as in the palette.
#[test]
fn drop_function_into_itself_is_rejected() {
    use crate::ops::droppable::Dropped;
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Function, "common/wet_mask").to_ref()));
    let before = e.doc().unwrap().asset.clone();
    let warnings = e.messages.iter().filter(|m| m.level == Level::Warning).count();
    let item = Dropped::Function("common/wet_mask".into());
    e.apply(MaterialAction::Drop { item, pos: (0.0, 0.0), node: None });
    assert_eq!(e.doc().unwrap().asset, before, "nothing added");
    assert!(!e.is_dirty());
    let new: Vec<&Message> = e.messages.iter().filter(|m| m.level == Level::Warning).skip(warnings).collect();
    assert!(new.len() == 1 && new[0].text.contains("common/wet_mask"), "{new:?}");

    // Another function is fine.
    let item = Dropped::Function("common/tex_layer".into());
    e.apply(MaterialAction::Drop { item, pos: (0.0, 0.0), node: None });
    assert_eq!(nodes_of(&e, "function::common/tex_layer").len(), 1);
}

/// Design §4.11: a Curve asset on empty canvas adds a Sample Curve node reading it.
#[test]
fn drop_curve_adds_sample_curve() {
    use crate::ops::droppable::Dropped;
    use crate::ui::tree::{TreeItem, TreeRow};
    let (_dir, mut e) = project();
    let pulse = AssetId::new(AssetType::Curve, "common/pulse");
    let row = TreeRow { tree: TreeId::Project(MATERIAL), item: TreeItem::Asset(pulse.to_ref()) };
    assert_eq!(Dropped::from_row(&row), Some(Dropped::Curve("common/pulse".into())));

    open_rock_wet(&mut e);
    let before = nodes_of(&e, "stdlib::SampleCurve");
    e.apply(MaterialAction::Drop { item: Dropped::Curve("common/pulse".into()), pos: (21.0, 9.0), node: None });
    let added: Vec<NodeId> = nodes_of(&e, "stdlib::SampleCurve").into_iter().filter(|n| !before.contains(n)).collect();
    assert_eq!(added.len(), 1);
    assert_eq!(e.doc().unwrap().asset.nodes[&added[0]].pos, (20.0, 10.0));
    assert_eq!(param_of(&e, &added[0], "curve"), Some(Value::Str("common/pulse".into())));
    assert_eq!(e.doc().unwrap().selection, [added[0].clone()].into());
    assert_eq!(e.referenced_asset(&added[0]), Some(pulse));
}

/// Design §4.11 / Review Focus 4: a Curve dropped onto a node with a curve param sets it, in one undo step.
#[test]
fn drop_curve_on_sample_curve_sets_param() {
    use crate::ops::droppable::Dropped;
    let (_dir, mut e) = project();
    open_rock_wet(&mut e);
    let sample = add(&mut e, "stdlib::SampleCurve");
    e.apply(MaterialAction::Drop {
        item: Dropped::Curve("common/pulse".into()),
        pos: (0.0, 0.0),
        node: Some(sample.clone()),
    });
    assert_eq!(param_of(&e, &sample, "curve"), Some(Value::Str("common/pulse".into())));
    let count = e.doc().unwrap().asset.nodes.len();
    e.apply(MaterialAction::Drop {
        item: Dropped::Curve("common/steps".into()),
        pos: (0.0, 0.0),
        node: Some(sample.clone()),
    });
    assert_eq!(e.doc().unwrap().asset.nodes.len(), count, "no node added");
    assert_eq!(param_of(&e, &sample, "curve"), Some(Value::Str("common/steps".into())));
    assert_eq!(e.doc().unwrap().selection, [sample.clone()].into());
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(param_of(&e, &sample, "curve"), Some(Value::Str("common/pulse".into())), "one undo step");
    assert_eq!(e.doc().unwrap().asset.nodes.len(), count);

    // A node without a curve param: a new Sample Curve node instead.
    let out = output_node(&e);
    e.apply(MaterialAction::Drop { item: Dropped::Curve("common/steps".into()), pos: (0.0, 0.0), node: Some(out) });
    assert_eq!(e.doc().unwrap().asset.nodes.len(), count + 1);
}

/// Design §4.1: new nodes land on the 10-unit grid; a paste moves its first node onto it and keeps the layout.
#[test]
fn added_and_pasted_nodes_are_snapped() {
    let (_dir, mut e) = project();
    open_rock_wet(&mut e);
    let add = |e: &mut Editor, pos| {
        let before: Vec<_> = e.doc().unwrap().asset.nodes.keys().cloned().collect();
        e.apply(MaterialAction::AddNode { kind: "stdlib::Constant".into(), pos, link: None, params: Vec::new() });
        e.doc().unwrap().asset.nodes.keys().find(|k| !before.contains(k)).unwrap().clone()
    };
    let a = add(&mut e, (12.3, 7.6));
    assert_eq!(e.doc().unwrap().asset.nodes[&a].pos, (10.0, 10.0));
    let b = add(&mut e, (0.0, 0.0));
    // Off-grid like a node of an older file (positions are not rewritten on load).
    let moved = vec![(a.clone(), (3.3, 4.4)), (b.clone(), (103.3, 54.4))];
    e.apply(GraphEdit::Moved { nodes: moved, frames: Vec::new() });
    let on_grid = |p: (f32, f32)| p.0 % 10.0 == 0.0 && p.1 % 10.0 == 0.0;
    let check = |e: &Editor| {
        let d = e.doc().unwrap();
        let mut pasted: Vec<(f32, f32)> = d.selection.iter().map(|id| d.asset.nodes[id].pos).collect();
        pasted.sort_by(|p, q| p.0.total_cmp(&q.0));
        assert_eq!(pasted.len(), 2);
        assert!(pasted.iter().all(|p| on_grid(*p)), "{pasted:?}");
        let rel = (pasted[1].0 - pasted[0].0, pasted[1].1 - pasted[0].1);
        assert!((rel.0 - 100.0).abs() < 1e-3 && (rel.1 - 50.0).abs() < 1e-3, "layout kept: {rel:?}");
        pasted[0]
    };
    e.apply(GraphEdit::Select([a.clone(), b.clone()].into()));
    e.apply(Action::Tab(TabAction::Copy));
    e.apply(Action::Tab(TabAction::Paste(Some(bevy::math::Vec2::new(33.0, 1.0)))));
    assert_eq!(check(&e), (30.0, 0.0));
    e.apply(Action::Tab(TabAction::Paste(None)));
    check(&e);
    e.apply(GraphEdit::Select([a].into()));
    e.apply(Action::Tab(TabAction::Duplicate));
    let d = e.doc().unwrap();
    assert!(on_grid(d.asset.nodes[d.selection.first().unwrap()].pos));

    // A reroute on a wire and a comment frame at the cursor are placed on the grid too.
    let out = output_node(&e);
    let before: Vec<_> = e.doc().unwrap().asset.nodes.keys().cloned().collect();
    e.apply(GraphEdit::AddReroute { from: (b, "out".into()), to: (out, "metallic".into()), pos: (-41.0, 22.6) });
    let d = e.doc().unwrap();
    let r = d.asset.nodes.iter().find(|(k, _)| !before.contains(k)).unwrap().1;
    assert_eq!((r.kind.as_str(), r.pos), ("stdlib::Reroute", (-40.0, 20.0)));
    e.apply(GraphEdit::Select(BTreeSet::new()));
    e.apply(GraphEdit::AddComment(Some((12.3, -7.6))));
    let f = e.doc().unwrap().asset.comments.last().unwrap();
    assert_eq!((f.rect.0, f.rect.1), (10.0, -10.0));
}

#[test]
fn undo_drop_keeps_created_asset() {
    let (dir, mut e) = project();
    game_file(&dir, &mut e, "textures/env/Rock-Albedo.png");
    open_rock_wet(&mut e);
    drop_file(&mut e, "textures/env/Rock-Albedo.png", (0.0, 0.0), None);
    assert_eq!(nodes_of(&e, "stdlib::TextureSample").len(), 1);
    e.apply(Action::Tab(TabAction::Undo));
    assert!(nodes_of(&e, "stdlib::TextureSample").is_empty(), "undo removes the node");
    assert!(!e.is_dirty(), "one undo step");
    let id = AssetId::new(AssetType::Texture, "textures/env/rock_albedo");
    assert!(dir.path().join("quillshade/material/textures/env/rock_albedo.texture.ron").is_file(), "the asset stays");
    assert!(e.domains.material().disk.textures.contains_key(&id));
    // Dropped again: reused, not `rock_albedo_2`.
    drop_file(&mut e, "textures/env/Rock-Albedo.png", (0.0, 0.0), None);
    let added = nodes_of(&e, "stdlib::TextureSample");
    assert_eq!(param_of(&e, &added[0], "texture"), Some(Value::Str(id.path.clone())));
    assert!(!dir.path().join("quillshade/material/textures/env/rock_albedo_2.texture.ron").exists());
}

/// Applies the node menu row labelled `label` of `node`.
pub(super) fn menu_action(e: &mut Editor, node: &str, label: &str) {
    let menu = e.node_menu(node);
    let (_, a) = menu.into_iter().find(|(l, _)| l == label).unwrap_or_else(|| panic!("no `{label}` row"));
    e.apply(a);
}

#[test]
fn convert_texture_sample_to_parameter_and_back() {
    let (_dir, mut e) = project();
    open_rock_wet(&mut e);
    // A parameter already named `texture` (its default name, not stored): the converted one gets `texture_1`.
    add(&mut e, "param::TextureObject");
    let sample = add(&mut e, "stdlib::TextureSample");
    let set = |key: &str, v: &str| {
        set_value(GraphProp::Param { node: sample.clone(), key: key.into() }, Value::Str(v.into()), false)
    };
    e.apply(set("texture", "textures/rock_albedo"));
    e.apply(set("mip_mode", "Bias"));
    e.apply(set("sampler_type", "Normal"));
    let out = output_node(&e);
    e.apply(GraphEdit::Disconnect { to: (out.clone(), "roughness".into()) });
    e.apply(GraphEdit::Connect { from: (sample.clone(), "r".into()), to: (out.clone(), "roughness".into()) });
    e.apply(set_value(GraphProp::Input { node: sample.clone(), pin: "mip".into() }, Value::Float(2.0), false));

    menu_action(&mut e, &sample, "Convert to Parameter");
    let d = e.doc().unwrap();
    let n = &d.asset.nodes[&sample];
    assert_eq!(n.kind, "param::TextureSample2D");
    assert_eq!(n.params.get("default"), Some(&Value::Str("textures/rock_albedo".into())));
    assert_eq!(n.params.get("mip_mode"), Some(&Value::Str("Bias".into())));
    assert_eq!(n.params.get("sampler_type"), Some(&Value::Str("Normal".into())));
    assert_eq!(n.params.get("name"), Some(&Value::Str("texture_1".into())));
    assert!(!n.params.contains_key("texture"));
    assert_eq!(n.inputs.get("mip"), Some(&Value::Float(2.0)));
    assert_eq!(d.asset.edge_into(&out, "roughness").map(|e| e.from.clone()), Some((sample.clone(), "r".into())));
    e.validate();
    assert!(!e.all_diagnostics().iter().any(Diagnostic::is_error), "{:?}", e.all_diagnostics());

    menu_action(&mut e, &sample, "Convert to Texture Sample");
    let d = e.doc().unwrap();
    let n = &d.asset.nodes[&sample];
    assert_eq!(n.kind, "stdlib::TextureSample");
    assert_eq!(n.params.get("texture"), Some(&Value::Str("textures/rock_albedo".into())));
    assert_eq!(n.params.get("mip_mode"), Some(&Value::Str("Bias".into())));
    assert_eq!(n.params.get("sampler_type"), Some(&Value::Str("Normal".into())));
    for gone in ["name", "group", "default"] {
        assert!(!n.params.contains_key(gone), "{gone}");
    }
    assert_eq!(n.inputs.get("mip"), Some(&Value::Float(2.0)));
    assert!(d.asset.edge_into(&out, "roughness").is_some());

    // TextureObject ↔ param::TextureObject, the same way.
    let obj = add(&mut e, "stdlib::TextureObject");
    e.apply(set_value(
        GraphProp::Param { node: obj.clone(), key: "texture".into() },
        Value::Str("textures/atlas".into()),
        false,
    ));
    e.apply(GraphEdit::Connect { from: (obj.clone(), "out".into()), to: (sample.clone(), "tex".into()) });
    menu_action(&mut e, &obj, "Convert to Parameter");
    let d = e.doc().unwrap();
    assert_eq!(d.asset.nodes[&obj].kind, "param::TextureObject");
    assert_eq!(d.asset.nodes[&obj].params.get("default"), Some(&Value::Str("textures/atlas".into())));
    assert_eq!(d.asset.nodes[&obj].params.get("name"), Some(&Value::Str("texture_1".into())));
    assert!(d.asset.edge_into(&sample, "tex").is_some());
    menu_action(&mut e, &obj, "Convert to Texture Object");
    let d = e.doc().unwrap();
    assert_eq!(d.asset.nodes[&obj].kind, "stdlib::TextureObject");
    assert_eq!(d.asset.nodes[&obj].params.get("texture"), Some(&Value::Str("textures/atlas".into())));
    assert!(!d.asset.nodes[&obj].params.contains_key("name"));
    assert!(d.asset.edge_into(&sample, "tex").is_some());

    // Functions have no parameters: not offered there.
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Function, "common/tex_layer").to_ref()));
    let in_fn = node_of(&e, "stdlib::TextureSample");
    assert!(!e.node_menu(&in_fn).iter().any(|(l, _)| l.starts_with("Convert")), "{:?}", e.node_menu(&in_fn));
}

#[test]
fn convert_two_nodes_get_distinct_unique_names() {
    let (_dir, mut e) = project();
    open_rock_wet(&mut e);
    let a = add(&mut e, "stdlib::TextureSample");
    let b = add(&mut e, "stdlib::TextureSample");
    e.apply(GraphEdit::Select([a.clone(), b.clone()].into()));
    menu_action(&mut e, &a, "Convert to Parameter");
    let name = |n: &str| param_of(&e, n, "name").map(|v| v.as_str().to_string()).unwrap_or_default();
    assert_eq!(nodes_of(&e, "param::TextureSample2D").len(), 4, "both converted (rock_wet has two)");
    let names: BTreeSet<String> = [name(&a), name(&b)].into();
    assert_eq!(names, ["texture".to_string(), "texture_1".to_string()].into());
    e.validate();
    assert!(!e.all_diagnostics().iter().any(Diagnostic::is_error), "{:?}", e.all_diagnostics());
}

/// Texture tab Easy mode (decision 0028): presets are undoable settings edits, the mode and the channel view are
/// editor-only.
#[test]
fn texture_tab_presets_mode_and_channels() {
    use crate::texture::FilterPreset;
    use shader_graph_core::texture::{AddressMode, TextureSettings};
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Texture, "textures/rock_albedo").to_ref()));
    assert!(texture_doc(&e).easy, "a default sampler opens in Easy mode");
    e.apply(Action::from(TextureEdit::new(TextureAction::Preset(FilterPreset::Ground))));
    let s = texture_doc(&e).asset.settings.sampler.unwrap();
    assert_eq!(Some(s), FilterPreset::Ground.sampler());
    assert_eq!(s.address_mode_u, AddressMode::Repeat);
    e.apply(Action::from(TextureEdit::new(TextureAction::Preset(FilterPreset::Ground))));
    e.apply(Action::from(TextureEdit::new(TextureAction::Preset(FilterPreset::PixelArt))));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(FilterPreset::of(&texture_doc(&e).asset.settings), Some(FilterPreset::Ground), "one step each");
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(texture_doc(&e).asset.settings, TextureSettings::default());
    assert!(!e.is_dirty());

    let layout = texture_doc(&e).layout;
    e.apply(Action::from(TextureEdit::new(TextureAction::SetEasy(false))));
    assert!(!texture_doc(&e).easy && texture_doc(&e).layout != layout, "the form is rebuilt");
    assert!(!e.is_dirty(), "the mode is not part of the asset");

    e.apply(PreviewChannel::new(Some(1)));
    assert_eq!(texture_doc(&e).channel, Some(1));
    e.apply(PreviewChannel::new(Some(7)));
    assert_eq!(texture_doc(&e).channel, None, "out of range: the whole image");
    e.apply(set_value(TextureKey::new(TextureProp::PreviewTiled), shader_graph_core::Value::Bool(true), false));
    assert!(texture_doc(&e).tiled);
    assert!(!e.is_dirty());
}

/// Design §6.3: the Easy preset is stored only through the sampler it writes, so every preset comes back selected
/// after save and reopen; "Default" writes no `sampler` key at all (the untouched file, byte for byte).
#[test]
fn easy_preset_round_trips_through_file() {
    use crate::texture::FilterPreset;
    let (dir, mut e) = project();
    let id = AssetId::new(AssetType::Texture, "textures/sprite");
    let file = dir.path().join("quillshade/material/textures/sprite.texture.ron");
    let untouched = std::fs::read(&file).unwrap();
    assert!(!String::from_utf8_lossy(&untouched).contains("sampler"));
    e.apply(Action::OpenAsset(id.to_ref()));
    assert!(texture_doc(&e).easy, "no sampler opens in Easy");
    assert_eq!(FilterPreset::of(&texture_doc(&e).asset.settings), Some(FilterPreset::Default), "Default selected");
    // Default last: back to the untouched file.
    for p in FilterPreset::ALL.into_iter().skip(1).chain([FilterPreset::Default]) {
        e.apply(Action::from(TextureEdit::new(TextureAction::Preset(p))));
        e.apply(Action::Save);
        assert!(!e.is_dirty(), "{p:?} saved");
        e.apply(Action::CloseTab(e.active.unwrap()));
        assert!(e.tabs.is_empty());
        e.apply(Action::OpenAsset(id.to_ref()));
        let t = texture_doc(&e);
        assert!(t.easy, "{p:?} reopens in Easy");
        assert_eq!(FilterPreset::of(&t.asset.settings), Some(p), "{p:?} reopens selected");
    }
    assert_eq!(std::fs::read(&file).unwrap(), untouched, "Default: byte-identical, no sampler key");
}

/// Design §6.3: a sampler customised in Advanced matches no preset, so Easy is refused (the button is disabled) and
/// the settings are never rewritten by switching.
#[test]
fn set_easy_refused_for_custom_sampler() {
    use crate::texture::{FilterPreset, TextureField};
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Texture, "textures/sprite").to_ref()));
    e.apply(Action::from(TextureEdit::new(TextureAction::SetEasy(false))));
    let custom = TextureField::LodMaxClamp
        .set(&FilterPreset::ThreeD.apply(&texture_doc(&e).asset.settings), &PropValue::Float(4.0));
    e.apply(Action::from(TextureEdit::new(TextureAction::SetSettings(custom.clone()))));
    assert_eq!(FilterPreset::of(&custom), None);
    let layout = texture_doc(&e).layout;
    e.apply(Action::from(TextureEdit::new(TextureAction::SetEasy(true))));
    assert!(!texture_doc(&e).easy, "refused: Easy would hide the custom sampler");
    assert_eq!(texture_doc(&e).asset.settings, custom, "settings untouched");
    assert_eq!(texture_doc(&e).layout, layout, "nothing to rebuild");
    // Back on a preset, Easy is allowed again.
    e.apply(Action::from(TextureEdit::new(TextureAction::Preset(FilterPreset::ThreeD))));
    e.apply(Action::from(TextureEdit::new(TextureAction::SetEasy(true))));
    assert!(texture_doc(&e).easy);
}

/// Design §6.3: undo, redo and an external reload that bring back a sampler matching no preset leave Easy (the form
/// shows the custom values); ones that keep a preset keep the mode.
#[test]
fn history_and_reload_leave_easy_for_custom_sampler() {
    use crate::texture::{FilterPreset, TextureField};
    use shader_graph_core::Value;
    let (dir, mut e) = project();
    let id = AssetId::new(AssetType::Texture, "textures/sprite");
    e.apply(Action::OpenAsset(id.to_ref()));
    e.apply(Action::from(TextureEdit::new(TextureAction::SetEasy(false))));
    let lod = PropValue::Float(4.0);
    let custom = TextureField::LodMaxClamp.set(&texture_doc(&e).asset.settings, &lod);
    e.apply(Action::from(TextureEdit::new(TextureAction::SetSettings(custom.clone()))));
    let default = Value::Bool(true);
    e.apply(set_value(TextureKey::new(TextureProp::Field(TextureField::DefaultSampler)), default, false));
    e.apply(Action::from(TextureEdit::new(TextureAction::SetEasy(true))));
    assert!(texture_doc(&e).easy, "Default fits Easy");

    // Undo brings the custom sampler back: Advanced, with the form rebuilt.
    let layout = texture_doc(&e).layout;
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(texture_doc(&e).asset.settings, custom);
    assert!(!texture_doc(&e).easy, "undo to a custom sampler leaves Easy");
    assert_ne!(texture_doc(&e).layout, layout);

    // Redo to a preset keeps Advanced (the mode is the user's); a redo to the custom sampler leaves Easy.
    e.apply(Action::Tab(TabAction::Redo));
    assert_eq!(FilterPreset::of(&texture_doc(&e).asset.settings), Some(FilterPreset::Default));
    assert!(!texture_doc(&e).easy);
    e.apply(Action::Tab(TabAction::Undo));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(FilterPreset::of(&texture_doc(&e).asset.settings), Some(FilterPreset::Default), "the untouched asset");
    e.apply(Action::from(TextureEdit::new(TextureAction::SetEasy(true))));
    e.apply(Action::Tab(TabAction::Redo));
    assert_eq!(texture_doc(&e).asset.settings, custom);
    assert!(!texture_doc(&e).easy, "redo to a custom sampler leaves Easy");

    // An undo to a preset in Easy stays in Easy.
    e.apply(Action::Tab(TabAction::Redo));
    e.apply(Action::from(TextureEdit::new(TextureAction::SetEasy(true))));
    e.apply(Action::from(TextureEdit::new(TextureAction::Preset(FilterPreset::Ground))));
    e.apply(Action::Tab(TabAction::Undo));
    assert!(texture_doc(&e).easy, "undo to Default keeps Easy");

    // An external edit of the (saved) file to a custom sampler: reloaded, out of Easy.
    e.apply(Action::Save);
    assert!(texture_doc(&e).easy && !e.is_dirty());
    let file = dir.path().join("quillshade/material/textures/sprite.texture.ron");
    let text = "TextureAsset(\n  format_version: 1,\n  path: \"textures/sprite.png\",\n  settings: TextureSettings(\n    \
                sampler: Some(SamplerSettings(\n      lod_max_clamp: 4.0,\n    )),\n  ),\n)\n";
    std::fs::write(&file, text).unwrap();
    e.apply(Action::Reload(asset_key(&id)));
    assert_eq!(FilterPreset::of(&texture_doc(&e).asset.settings), None, "reloaded");
    assert!(!texture_doc(&e).easy, "a reload to a custom sampler leaves Easy");
}

/// Design §6.2: an image tab has the preview's view state (channel, tiling): editor-only, not undoable, not dirty.
#[test]
fn image_tab_preview_state() {
    let (dir, mut e) = project();
    let path = "textures/rock.png";
    std::fs::create_dir_all(dir.path().join("assets/textures")).unwrap();
    std::fs::write(dir.path().join("assets").join(path), b"x").unwrap();
    e.apply(Action::OpenGameImage(path.into()));
    let image = |e: &Editor| e.tab().and_then(Tab::get::<ImageDoc>).map(|i| (i.channel, i.tiled)).unwrap();
    assert_eq!(image(&e), (None, false));
    e.apply(PreviewChannel::new(Some(2)));
    e.apply(set_value(TextureKey::new(TextureProp::PreviewTiled), shader_graph_core::Value::Bool(true), false));
    assert_eq!(image(&e), (Some(2), true));
    assert_eq!(shown(&e, &TextureKey::new(TextureProp::PreviewTiled)), Some(shader_graph_core::Value::Bool(true)));
    assert!(!e.tab().unwrap().doc().is_dirty());
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(image(&e), (Some(2), true), "not undoable");
}

/// A click on the editor statusbar's problem count focuses what the first console/logs row of this asset focuses.
#[test]
fn statusbar_problem_is_first_diagnostic_of_active_asset() {
    let (_dir, mut e) = project();
    assert!(e.active_first_diagnostic().is_none(), "no tab");
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(Action::OpenAsset(rock.to_ref()));
    e.apply(MaterialAction::AddNode { kind: "stdlib::Time".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    e.validate();
    let first = e.domains.material().diags.get(&rock).and_then(|d| d.first()).expect("rock_wet has warnings");
    assert_eq!(e.active_first_diagnostic(), Some(first.to_generic(MATERIAL)));
    e.apply(Action::OpenAsset(pulse().to_ref()));
    assert!(e.active_first_diagnostic().is_none(), "the curve has no problems");
}
