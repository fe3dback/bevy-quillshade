//! Graph editing: nodes, pins, conversions, renames, node previews (decision 0022), node menus.

use super::*;

#[test]
fn constant_label_replaces_title() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let before: Vec<_> = e.doc().unwrap().asset.nodes.keys().cloned().collect();
    e.apply(MaterialAction::AddNode {
        kind: "stdlib::Constant".into(),
        pos: (0.0, 0.0),
        link: None,
        params: Vec::new(),
    });
    let id = e.doc().unwrap().asset.nodes.keys().find(|k| !before.contains(k)).unwrap().clone();
    let title = |e: &Editor| {
        crate::doc::node_title(
            &e.domains.material().registry,
            &e.domains.material().functions,
            &e.doc().unwrap().asset,
            &id,
        )
    };
    let value_title = title(&e);
    let set = |s: &str| set_value(GraphProp::NodeLabel(id.clone()), Value::Str(s.into()), true);
    e.apply(set("rim"));
    assert_eq!(title(&e), "rim");
    e.apply(set(""));
    assert_eq!(title(&e), value_title);
}

#[test]
fn open_edit_save_regenerates() {
    let (dir, mut e) = project();
    let id = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(Action::OpenAsset(id.to_ref()));
    assert_eq!(e.tabs.len(), 1);
    e.validate();
    assert!(!e.all_diagnostics().iter().any(Diagnostic::is_error), "{:?}", e.all_diagnostics());
    e.apply(MaterialAction::AddNode { kind: "stdlib::Time".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    assert!(e.is_dirty());
    e.validate();
    assert!(e.all_diagnostics().iter().any(|d| d.code == Code::W101));
    e.apply(Action::Tab(TabAction::Undo));
    assert!(!e.is_dirty());
    e.apply(Action::Tab(TabAction::Redo));
    e.apply(Action::Save);
    assert!(!e.is_dirty());
    assert!(dir.path().join("assets/quillshade/material/autogen/dir_env/mat_rock_wet.wesl").is_file());
    assert!(dir.path().join("crates/quillshade/src/material/materials/dir_env/mat_rock_wet.rs").is_file());
}

/// A changed blend mode only takes effect after a rebuild of the game (the alpha mode is compiled Rust, decision 0029):
/// saving warns about it; a parameter-free graph edit that keeps the Rust says nothing.
#[test]
fn rust_changes_warn_to_rebuild_the_game() {
    let (_dir, mut e) = project();
    e.apply(Action::RegenAll);
    let rebuild = |e: &Editor| e.messages.iter().filter(|m| m.text.contains("rebuild the game")).count();
    let before = rebuild(&e);
    e.apply(Action::RegenAll);
    assert_eq!(rebuild(&e), before, "nothing changed");

    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    e.apply(MaterialAction::AddNode { kind: "stdlib::Time".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    e.apply(Action::Save);
    assert_eq!(rebuild(&e), before, "the Rust is unchanged");

    e.apply(set_value(GraphProp::Blend, Value::Str("Blend".into()), false));
    e.apply(Action::Save);
    let last = e.messages.iter().rev().find(|m| m.text.contains("rebuild the game")).unwrap();
    assert_eq!(rebuild(&e), before + 1);
    assert_eq!(last.level, Level::Warning);
    assert!(last.text.contains("env/rock_wet"), "{}", last.text);
}

#[test]
fn add_node_from_wire_connects() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let out = e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "output::Pbr3d").unwrap().0.clone();
    e.apply(GraphEdit::Disconnect { to: (out.clone(), "roughness".into()) });
    let link = Link { node: out.clone(), pin: "roughness".into(), from_output: false };
    e.apply(MaterialAction::AddNode {
        kind: "stdlib::Constant".into(),
        pos: (0.0, 0.0),
        link: Some(link),
        params: Vec::new(),
    });
    let d = e.doc().unwrap();
    let edge = d.asset.edge_into(&out, "roughness").expect("auto-connected");
    assert_eq!(d.asset.nodes[&edge.from.0].kind, "stdlib::Constant");
    // One undo removes node and wire.
    let constant = edge.from.0.clone();
    e.apply(Action::Tab(TabAction::Undo));
    assert!(e.doc().unwrap().asset.edge_into(&out, "roughness").is_none());
    assert!(!e.doc().unwrap().asset.nodes.contains_key(&constant));
}

pub(super) fn add_lerp(e: &mut Editor) -> NodeId {
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    e.apply(MaterialAction::AddNode {
        kind: "stdlib::Lerp".into(),
        pos: (-700.0, 0.0),
        link: None,
        params: Vec::new(),
    });
    let d = e.doc().unwrap();
    d.asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Lerp").unwrap().0.clone()
}

#[test]
fn inline_drag_is_one_undo_step() {
    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    let alpha = GraphProp::Input { node: lerp.clone(), pin: "alpha".into() };
    let set = |value: f32| set_component(alpha.clone(), 0, value, false);
    let input = |e: &Editor| e.doc().unwrap().asset.nodes[&lerp].inputs.get("alpha").cloned();
    for v in [0.2, 0.4, 0.6] {
        e.apply(set(v));
    }
    let props = e.ui.props;
    e.apply(Action::Tab(TabAction::CommitInline));
    assert!(e.ui.props > props, "Properties redraws with the committed value");
    // The next drag is a separate undo step.
    e.apply(set(0.9));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(input(&e), Some(Value::Float(0.6)));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(input(&e), None);
}

/// Review Focus 4: a drag's merged edits (`SetValue { merge: true }`, what a non-final number input change
/// becomes) ending with a final `SetComponent` are one undo step, on a node input and on a global constant alike.
#[test]
fn number_drag_is_one_undo_step() {
    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    let alpha = GraphProp::Input { node: lerp.clone(), pin: "alpha".into() };
    let input = |e: &Editor| e.doc().unwrap().asset.nodes[&lerp].inputs.get("alpha").cloned();
    for v in [0.2, 0.4, 0.6] {
        e.apply(set_value(alpha.clone(), Value::Float(v), true));
    }
    e.apply(set_component(alpha.clone(), 0, 0.7, true));
    assert_eq!(input(&e), Some(Value::Float(0.7)));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(input(&e), None, "one undo restores the value before the drag");

    e.apply(Action::OpenAsset(tuning().to_ref()));
    let amount = ConstsProp::Value("glow_amount".into());
    let before = shown(&e, &amount);
    for v in [2.0, 3.0] {
        e.apply(set_value(amount.clone(), Value::Float(v), true));
    }
    e.apply(set_component(amount.clone(), 0, 4.0, true));
    // The next edit is a step of its own.
    e.apply(set_component(amount.clone(), 0, 5.0, true));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(shown(&e, &amount), Some(Value::Float(4.0)));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(shown(&e, &amount), before);
}

pub(super) fn lerp_types(e: &Editor, lerp: &str) -> Vec<PinType> {
    let d = e.doc().unwrap();
    let sig =
        node_signature(&d.asset, lerp, &e.domains.material().registry, &e.domains.material().functions, &d.id.path)
            .unwrap();
    ["a", "b", "alpha"].iter().map(|p| sig.input(p).unwrap().ty).chain([sig.outputs[0].ty]).collect()
}

#[test]
fn set_pin_type_resolves_poly() {
    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    e.apply(MaterialAction::SetPinType { node: lerp.clone(), ty: PinType::Float3 });
    use PinType::*;
    assert_eq!(lerp_types(&e, &lerp), vec![Float3, Float3, Float, Float3]);
    e.apply(MaterialAction::SetPinType { node: lerp.clone(), ty: Float });
    assert_eq!(lerp_types(&e, &lerp), vec![Float, Float, Float, Float]);
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(lerp_types(&e, &lerp), vec![Float3, Float3, Float, Float3]);
}

#[test]
fn reset_input_removes_override() {
    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    let alpha = GraphProp::Input { node: lerp.clone(), pin: "alpha".into() };
    e.apply(set_component(alpha, 0, 0.8, true));
    e.apply(MaterialAction::ResetInput { node: lerp.clone(), pin: "alpha".into() });
    let input = |e: &Editor| e.doc().unwrap().asset.nodes[&lerp].inputs.get("alpha").cloned();
    assert_eq!(input(&e), None);
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(input(&e), Some(Value::Float(0.8)));
}

#[test]
fn convert_keeps_wires_and_inputs_in_one_undo_step() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    e.apply(MaterialAction::AddNode { kind: "stdlib::Add".into(), pos: (-700.0, 0.0), link: None, params: Vec::new() });
    let add = e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Add").unwrap().0.clone();
    e.apply(set_value(GraphProp::Input { node: add.clone(), pin: "b".into() }, Value::Float(3.0), false));
    let lerp = add_lerp(&mut e);
    e.apply(GraphEdit::Connect { from: (add.clone(), "out".into()), to: (lerp.clone(), "a".into()) });
    e.apply(GraphEdit::Select([add.clone(), lerp.clone()].into()));
    let acts = e.hotkey_actions(Hotkey::shift(HotkeyKey::Slash));
    assert_eq!(acts.len(), 1);
    for a in acts {
        e.apply(a);
    }
    let d = e.doc().unwrap();
    assert_eq!(d.asset.nodes[&add].kind, "stdlib::Divide");
    assert_eq!(d.asset.nodes[&lerp].kind, "stdlib::Lerp");
    assert_eq!(d.asset.nodes[&add].inputs.get("b"), Some(&Value::Float(3.0)));
    assert!(d.asset.edge_into(&lerp, "a").is_some());
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(e.doc().unwrap().asset.nodes[&add].kind, "stdlib::Add");
}

pub(super) fn output_node(e: &Editor) -> NodeId {
    e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "output::Pbr3d").unwrap().0.clone()
}

pub(super) fn output_mode(e: &Editor) -> PbrOutputMode {
    match e.doc().unwrap().asset.kind {
        AssetKind::Material(MaterialKind::Pbr3d(s)) => s.output,
        _ => panic!("not a Pbr3d material"),
    }
}

pub(super) fn add(e: &mut Editor, kind: &str) -> NodeId {
    let before: BTreeSet<NodeId> = e.doc().unwrap().asset.nodes.keys().cloned().collect();
    e.apply(MaterialAction::AddNode { kind: kind.into(), pos: (-900.0, 0.0), link: None, params: Vec::new() });
    e.doc().unwrap().asset.nodes.keys().find(|k| !before.contains(*k)).unwrap().clone()
}

pub(super) fn set_output_mode(e: &mut Editor, mode: &str) {
    e.apply(set_value(GraphProp::OutputMode, Value::Str(mode.into()), false));
}

pub(super) fn pins_into(e: &Editor, node: &str) -> BTreeSet<String> {
    e.doc().unwrap().asset.edges.iter().filter(|x| x.to.0 == node).map(|x| x.to.1.clone()).collect()
}

/// Decision 0023: switching the Pbr3d output mode drops wires into pins the new layout lacks, in one undo step.
#[test]
fn output_mode_switch_prunes_edges_and_undoes() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let out = output_node(&e);
    let (v3, k) = (add(&mut e, "stdlib::Constant3Vector"), add(&mut e, "stdlib::Constant"));
    e.apply(GraphEdit::Connect { from: (v3, "out".into()), to: (out.clone(), "base_color".into()) });
    e.apply(GraphEdit::Connect { from: (k, "out".into()), to: (out.clone(), "roughness".into()) });
    e.apply(set_value(GraphProp::Input { node: out.clone(), pin: "specular".into() }, Value::Float(0.3), false));
    let make = add(&mut e, "stdlib::MakeMaterialAttributesPbr");
    let before = pins_into(&e, &out);
    assert!(before.contains("base_color") && before.contains("roughness"), "{before:?}");

    set_output_mode(&mut e, "Wrapped");
    assert_eq!(output_mode(&e), PbrOutputMode::Wrapped);
    assert!(pins_into(&e, &out).iter().all(|p| p == WPO_PIN), "{:?}", pins_into(&e, &out));
    assert!(e.doc().unwrap().asset.nodes[&out].inputs.is_empty());
    e.apply(GraphEdit::Connect { from: (make, "attributes".into()), to: (out.clone(), "attributes".into()) });
    assert!(pins_into(&e, &out).contains("attributes"));

    set_output_mode(&mut e, "Default");
    assert_eq!(output_mode(&e), PbrOutputMode::Default);
    assert!(!pins_into(&e, &out).contains("attributes"));

    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(output_mode(&e), PbrOutputMode::Wrapped);
    assert!(pins_into(&e, &out).contains("attributes"));
    e.apply(Action::Tab(TabAction::Undo)); // the attributes wire
    e.apply(Action::Tab(TabAction::Undo)); // the switch to `Wrapped`
    assert_eq!(output_mode(&e), PbrOutputMode::Default);
    assert_eq!(pins_into(&e, &out), before);
    assert_eq!(e.doc().unwrap().asset.nodes[&out].inputs.get("specular"), Some(&Value::Float(0.3)));
}

#[test]
fn wpo_edge_survives_mode_switch() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let out = output_node(&e);
    let v3 = add(&mut e, "stdlib::Constant3Vector");
    e.apply(GraphEdit::Connect { from: (v3.clone(), "out".into()), to: (out.clone(), WPO_PIN.into()) });
    set_output_mode(&mut e, "Wrapped");
    assert!(pins_into(&e, &out).contains(WPO_PIN));
    set_output_mode(&mut e, "Default");
    assert!(pins_into(&e, &out).contains(WPO_PIN));
    assert_eq!(shown(&e, &GraphProp::OutputMode), Some(Value::Str("Default".into())));
}

pub(super) fn node_of(e: &Editor, kind: &str) -> NodeId {
    e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == kind).unwrap().0.clone()
}

pub(super) fn active_asset(e: &Editor) -> AssetId {
    AssetId::from_ref(e.tabs[e.active.unwrap()].owner().unwrap()).unwrap()
}

/// Decision 0023: the file a node refers to (double-click, props "Open" button).
#[test]
fn referenced_asset_resolves_targets() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let call = node_of(&e, "function::common/wet_mask");
    assert_eq!(e.referenced_asset(&call), Some(AssetId::new(AssetType::Function, "common/wet_mask")));
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "fx/curve_bands").to_ref()));
    let curve = node_of(&e, "stdlib::SampleCurve");
    let path = e.doc().unwrap().asset.nodes[&curve].params["curve"].as_str().to_string();
    assert_eq!(e.referenced_asset(&curve), Some(AssetId::new(AssetType::Curve, path)));
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "fx/global_tint").to_ref()));
    let global = node_of(&e, "stdlib::GlobalConstF32");
    assert_eq!(e.referenced_asset(&global), Some(AssetId::new(AssetType::Consts, "common/tuning")));
}

#[test]
fn referenced_asset_none() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let sum = add(&mut e, "stdlib::Add");
    assert_eq!(e.referenced_asset(&sum), None);
    let curve = add(&mut e, "stdlib::SampleCurve");
    assert_eq!(e.referenced_asset(&curve), None, "no curve picked");
    let global = add(&mut e, "stdlib::GlobalConstF32");
    assert_eq!(e.referenced_asset(&global), None, "no constant picked");
    let user = shader_graph_core::Node::new("user::soft_clamp", (0.0, 0.0));
    let dangling = shader_graph_core::Node::new("function::common/missing", (0.0, 0.0));
    e.doc_mut().unwrap().edit(|a| {
        a.nodes.insert("dangling".into(), dangling);
        a.nodes.insert("user".into(), user);
    });
    assert_eq!(e.referenced_asset("dangling"), None, "dangling function");
    assert_eq!(e.referenced_asset("user"), None, "user nodes are not editor assets");
    assert_eq!(e.referenced_asset("zzz"), None);
}

#[test]
fn open_referenced_twice_reuses_tab() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let call = node_of(&e, "function::common/wet_mask");
    let target = e.referenced_asset(&call).unwrap();
    let tabs = e.tabs.len();
    e.apply(Action::OpenAsset(target.to_ref()));
    assert_eq!(active_asset(&e), target);
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    e.apply(Action::OpenAsset(target.to_ref()));
    assert_eq!(e.tabs.len(), tabs + 1);
    assert_eq!(active_asset(&e), target);
}

/// Design §5.5: what a node header's double click renames.
#[test]
fn rename_target_kinds() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let param = node_of(&e, "param::Scalar");
    assert_eq!(e.rename_target(&param), Some(GraphProp::Param { node: param.clone(), key: "name".into() }));
    let constant = add(&mut e, "stdlib::Constant");
    assert_eq!(e.rename_target(&constant), Some(GraphProp::NodeLabel(constant.clone())), "unlabelled Constant");
    let lerp = node_of(&e, "stdlib::Lerp");
    assert_eq!(e.rename_target(&lerp), None);
    let call = node_of(&e, "function::common/wet_mask");
    assert_eq!(e.rename_target(&call), None, "a function call opens its function instead");
    let wet_mask = Action::OpenAsset(AssetId::new(AssetType::Function, "common/wet_mask").to_ref());
    assert_eq!(e.open_action(&call), Some(wet_mask.clone()), "its body and its menu's Open in tab do");
    assert_eq!(e.header_action(&call), Some(wet_mask), "a reference first");
    let comment = |n: &str| Some(MaterialAction::TextDialog(GraphProp::NodeComment(n.to_string())).into());
    assert_eq!(e.header_action(&lerp), comment(&lerp), "no reference, no name: the comment");
    assert_eq!(e.open_action(&lerp), None);
    // A Named Route declaration has a name; its usages refer to it by node id, so a rename keeps them.
    let route = add(&mut e, "stdlib::NamedRoute");
    let route_name = GraphProp::Param { node: route.clone(), key: "name".into() };
    assert_eq!(e.rename_target(&route), Some(route_name.clone()));
    assert_eq!(e.header_action(&route), Some(MaterialAction::TextDialog(route_name).into()));
    let usage = add(&mut e, "stdlib::NamedRouteUsage");
    assert_eq!(e.rename_target(&usage), None, "a usage shows its declaration's name");
    assert_eq!(
        e.header_action(&param),
        Some(MaterialAction::TextDialog(GraphProp::Param { node: param, key: "name".into() }).into())
    );
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Function, "common/wet_mask").to_ref()));
    let input = node_of(&e, "output::FunctionInput");
    assert_eq!(e.rename_target(&input), Some(GraphProp::Param { node: input, key: "name".into() }));
    let output = node_of(&e, "output::FunctionOutput");
    assert_eq!(e.rename_target(&output), Some(GraphProp::Param { node: output, key: "name".into() }));
}

/// Opens the rename dialog of `target` and submits `text`, as the dialog's OK / Enter does.
pub(super) fn rename_through_dialog(e: &mut Editor, target: GraphProp, text: &str) {
    e.apply(MaterialAction::TextDialog(target.clone()));
    assert!(
        matches!(&e.dialog, Some(Dialog::Input { submit: Submit::SetText(t), .. }) if *t == target),
        "{:?}",
        e.dialog
    );
    e.apply(Action::Submit(text.into()));
    assert_eq!(e.dialog, None);
}

#[test]
fn rename_dialog_submit_is_one_undo_step() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let param = node_of(&e, "param::Scalar");
    let target = GraphProp::Param { node: param.clone(), key: "name".into() };
    let before = shown(&e, &target).unwrap();
    e.apply(MaterialAction::TextDialog(target.clone()));
    let Some(Dialog::Input { title, label, initial, .. }) = &e.dialog else { panic!("{:?}", e.dialog) };
    assert_eq!((title.as_str(), label.as_str()), ("Rename node", "Name"));
    assert_eq!(*initial, before.as_str(), "prefilled with the current name");
    let props = e.ui.props;
    rename_through_dialog(&mut e, target.clone(), "gloss");
    assert_eq!(shown(&e, &target), Some(Value::Str("gloss".into())));
    assert!(e.ui.props > props, "the right sidebar shows the new name");
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(shown(&e, &target), Some(before), "one undo step");
    assert!(!e.is_dirty());

    // A comment title goes the same way.
    let title = GraphProp::FrameTitle("c01".into());
    e.apply(MaterialAction::TextDialog(title.clone()));
    let Some(Dialog::Input { title: t, label, .. }) = &e.dialog else { panic!("{:?}", e.dialog) };
    assert_eq!((t.as_str(), label.as_str()), ("Comment", "Title"));
    rename_through_dialog(&mut e, title.clone(), "Wet rocks");
    assert_eq!(shown(&e, &title), Some(Value::Str("Wet rocks".into())));
}

#[test]
fn rename_dialog_empty_label_clears() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let constant = add(&mut e, "stdlib::Constant");
    let target = GraphProp::NodeLabel(constant.clone());
    rename_through_dialog(&mut e, target.clone(), "rim");
    assert_eq!(e.doc().unwrap().asset.nodes[&constant].label, "rim");
    e.apply(MaterialAction::TextDialog(target.clone()));
    assert!(matches!(&e.dialog, Some(Dialog::Input { initial, .. }) if initial == "rim"));
    e.apply(Action::Submit("  ".into()));
    assert_eq!(e.doc().unwrap().asset.nodes[&constant].label, "", "no label");
    let title = crate::doc::node_title(
        &e.domains.material().registry,
        &e.domains.material().functions,
        &e.doc().unwrap().asset,
        &constant,
    );
    assert_ne!(title, "rim", "the title shows the value again");
}

#[test]
fn convert_drops_wires_and_inputs_of_missing_pins() {
    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    e.apply(MaterialAction::AddNode { kind: "stdlib::Add".into(), pos: (-900.0, 0.0), link: None, params: Vec::new() });
    e.apply(MaterialAction::AddNode {
        kind: "stdlib::Time".into(),
        pos: (-1100.0, 0.0),
        link: None,
        params: Vec::new(),
    });
    let find = |e: &Editor, k: &str| e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == k).unwrap().0.clone();
    let (add, time) = (find(&e, "stdlib::Add"), find(&e, "stdlib::Time"));
    e.apply(set_value(GraphProp::Input { node: add.clone(), pin: "b".into() }, Value::Float(3.0), false));
    e.apply(GraphEdit::Connect { from: (time, "out".into()), to: (add.clone(), "a".into()) });
    e.apply(GraphEdit::Connect { from: (add.clone(), "out".into()), to: (lerp.clone(), "a".into()) });
    e.apply(MaterialAction::ConvertNodes { nodes: vec![add.clone()], conv: Conversion::kind("stdlib::Abs") });
    let d = e.doc().unwrap();
    assert_eq!(d.asset.nodes[&add].kind, "stdlib::Abs");
    assert!(d.asset.nodes[&add].inputs.is_empty(), "`b` does not exist on Abs");
    assert!(d.asset.edge_into(&add, "a").is_none(), "the wire into `a` is dropped");
    assert!(d.asset.edge_into(&lerp, "a").is_some(), "`out` exists on both");
}

#[test]
fn node_menu_lists_convert_with_hotkeys_and_generic_rows() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    e.apply(MaterialAction::AddNode {
        kind: "stdlib::Multiply".into(),
        pos: (0.0, 0.0),
        link: None,
        params: Vec::new(),
    });
    let m = e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Multiply").unwrap().0.clone();
    let labels: Vec<String> = e.node_menu(&m).into_iter().map(|r| r.0).collect();
    assert!(labels.contains(&"Convert to Add\tShift +".to_string()));
    assert!(labels.contains(&"Convert to Divide\tShift /".to_string()));
    assert!(!labels.iter().any(|l| l.starts_with("Convert to Multiply")));
    assert!(labels.contains(&"Delete\tDel".to_string()));
    // The preview row follows the separator after the node's own actions (decision 0022).
    let at = |l: &str| labels.iter().position(|x| x == l).unwrap();
    assert_eq!(at("Exclusive preview\tV"), at("") + 1);
    assert_eq!(at("Add comment"), at("") + 2);
    assert_eq!(at("Copy\tCtrl+C"), at("") + 3);
    assert!(!labels.iter().any(|l| l == "Open in tab"), "Multiply refers to nothing");
}

/// Any node takes an editor-only comment (at most `Node::COMMENT_MAX` characters): from the props field, the
/// header's double click or the menu's Add / Edit comment; a node referring to a file has Open in tab.
#[test]
fn node_comment_and_open_in_tab() {
    use shader_graph_core::model::Node;

    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let call = node_of(&e, "function::common/wet_mask");
    let menu = |e: &Editor| e.node_menu(&call);
    let wet_mask = Action::OpenAsset(AssetId::new(AssetType::Function, "common/wet_mask").to_ref());
    assert!(menu(&e).contains(&("Open in tab".into(), wet_mask)));
    let target = GraphProp::NodeComment(call.clone());
    let dialog: Action = MaterialAction::TextDialog(target.clone()).into();
    assert!(menu(&e).contains(&("Add comment".into(), dialog.clone())));
    e.apply(dialog.clone());
    let Some(Dialog::Input { submit: Submit::SetText(t), initial, .. }) = e.dialog.clone() else { panic!() };
    assert_eq!((t, initial.as_str()), (PropKey::new(target.clone()), ""));
    e.apply(Action::Submit("x".repeat(Node::COMMENT_MAX + 10)));
    let stored = |e: &Editor| e.doc().unwrap().asset.nodes[&call].comment.clone();
    assert_eq!(stored(&e).chars().count(), Node::COMMENT_MAX, "cut to the limit");
    assert!(menu(&e).contains(&("Edit comment".into(), dialog)));
    e.apply(set_value(target, Value::Str("wet\nlayer".into()), false));
    assert_eq!(stored(&e), "wet\nlayer");
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(stored(&e).len(), Node::COMMENT_MAX, "one undo step per edit");
}

pub(super) fn press_v(e: &mut Editor) {
    for a in e.hotkey_actions(Hotkey::plain(HotkeyKey::Letter('v'))) {
        e.apply(a);
    }
}

pub(super) fn rock_wet_wesl(e: &Editor) -> PathBuf {
    wesl_file(e.project.as_ref().unwrap(), &AssetId::new(AssetType::Material, "env/rock_wet")).unwrap()
}

#[test]
fn preview_toggle_regenerates_from_memory_and_discard_restores() {
    let (dir, mut e) = project();
    e.apply(Action::RegenAll);
    let wesl = rock_wet_wesl(&e);
    let normal = std::fs::read_to_string(&wesl).unwrap();
    let rust = rust_file(e.project.as_ref().unwrap(), &AssetId::new(AssetType::Material, "env/rock_wet")).unwrap();
    let lerp = add_lerp(&mut e);
    let rust_before = std::fs::read(&rust).unwrap();
    e.apply(GraphEdit::Select([lerp.clone()].into()));
    press_v(&mut e);
    assert_eq!(e.doc().unwrap().asset.preview.node.as_deref(), Some(lerp.as_str()));
    assert_eq!(std::fs::read(&rust).unwrap(), rust_before, "a plain toggle leaves the Rust module alone");
    assert!(e.is_dirty(), "toggling the preview is a document edit");
    assert_eq!(e.tab().unwrap().title(), "quillshade/material/env/rock_wet.material (preview)");
    assert_eq!(e.tab().unwrap().short_title(), "rock_wet (preview)");
    let previewed = std::fs::read_to_string(&wesl).unwrap();
    assert!(previewed.contains(&format!("// preview of node {lerp} (unlit)")), "{previewed}");
    let source = dir.path().join("quillshade/material/env/rock_wet.material.ron");
    assert!(!std::fs::read_to_string(&source).unwrap().contains("preview"), "nothing is saved");
    e.apply(Action::Tab(TabAction::Undo));
    assert!(e.doc().unwrap().asset.preview.node.is_none());
    assert!(!std::fs::read_to_string(&wesl).unwrap().contains("// preview of node"));
    press_v(&mut e);
    assert!(std::fs::read_to_string(&wesl).unwrap().contains("// preview of node"));
    e.apply(Action::DiscardTab(e.active.unwrap()));
    assert!(e.tabs.is_empty());
    assert_eq!(std::fs::read_to_string(&wesl).unwrap(), normal, "closing regenerates from disk");
}

#[test]
fn reload_restores_memory_regenerated_files() {
    let (_dir, mut e) = project();
    e.apply(Action::RegenAll);
    let wesl = rock_wet_wesl(&e);
    let normal = std::fs::read_to_string(&wesl).unwrap();
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    let flag = |e: &Editor| e.tabs[e.doc_index(&rock).unwrap()].get::<Doc>().unwrap().regen_from_memory;
    // Reload of an inactive tab (the watcher asks about any tab) and of the active one.
    for other_active in [true, false] {
        let lerp = add_lerp(&mut e);
        e.apply(MaterialAction::TogglePreview(lerp));
        assert!(flag(&e));
        assert!(std::fs::read_to_string(&wesl).unwrap().contains("// preview of node"));
        if other_active {
            e.apply(Action::OpenAsset(pulse().to_ref()));
        }
        e.apply(Action::Reload(asset_key(&rock)));
        assert_eq!(std::fs::read_to_string(&wesl).unwrap(), normal, "other_active: {other_active}");
        assert!(!flag(&e), "other_active: {other_active}");
        assert!(!e.tabs[e.doc_index(&rock).unwrap()].doc().is_dirty());
    }
}

#[test]
fn preview_suffix_keeps_shared_names_apart() {
    let (_dir, mut e) = project();
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    let twin = AssetId::new(AssetType::Material, "props/rock_wet");
    let asset = assets::load(e.project.as_ref().unwrap(), &rock).unwrap();
    ops::save(e.project.as_ref().unwrap(), &twin, &asset).unwrap();
    e.rescan();
    let lerp = add_lerp(&mut e);
    e.apply(MaterialAction::TogglePreview(lerp));
    e.apply(Action::OpenAsset(twin.to_ref()));
    assert_eq!(
        crate::tab::labels(&e.tabs),
        ["quillshade/material/env/rock_wet.material (preview)", "quillshade/material/props/rock_wet.material"]
    );
    e.apply(Action::OpenAsset(pulse().to_ref()));
    assert_eq!(crate::tab::labels(&e.tabs)[2], "pulse");
}

pub(super) fn preview_messages(e: &Editor) -> usize {
    e.messages.iter().filter(|m| m.text.starts_with("Preview:")).count()
}

#[test]
fn saving_another_tab_keeps_an_unsaved_preview() {
    let (dir, mut e) = project();
    e.apply(Action::RegenAll);
    let wesl = rock_wet_wesl(&e);
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    let lerp = add_lerp(&mut e);
    e.apply(MaterialAction::TogglePreview(lerp.clone()));
    let header = format!("// preview of node {lerp} (unlit)");
    // A trivial edit of a function the material calls, saved: the full regeneration keeps the preview.
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Function, "common/wet_mask").to_ref()));
    e.apply(set_value(GraphProp::Description, Value::Str("tweak".into()), false));
    e.apply(Action::Save);
    assert!(std::fs::read_to_string(&wesl).unwrap().contains(&header));
    e.apply(Action::RegenAll);
    assert!(std::fs::read_to_string(&wesl).unwrap().contains(&header));
    let i = e.doc_index(&rock).unwrap();
    assert!(e.tabs[i].get::<Doc>().unwrap().regen_from_memory);
    // Saving the previewed tab stores the preview in the file and clears the flag.
    e.apply(Action::ActivateTab(i));
    e.apply(Action::Save);
    assert!(
        std::fs::read_to_string(dir.path().join("quillshade/material/env/rock_wet.material.ron"))
            .unwrap()
            .contains(&lerp)
    );
    assert!(!e.tabs[i].get::<Doc>().unwrap().regen_from_memory);
    assert!(std::fs::read_to_string(&wesl).unwrap().contains(&header));
}

#[test]
fn a_preview_that_stopped_working_can_be_turned_off() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    e.apply(MaterialAction::AddNode { kind: "stdlib::Not".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    let not = e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Not").unwrap().0.clone();
    // As if stored in the file (or the node's output stopped converting to a color): W107 "preview ignored".
    e.doc_mut().unwrap().edit(|a| a.preview.node = Some(not.clone()));
    let row = e.node_menu(&not).into_iter().next().unwrap();
    assert_eq!(row, ("Stop preview\tV".to_string(), MaterialAction::TogglePreview(not.clone()).into()));
    e.apply(GraphEdit::Select([not.clone()].into()));
    assert_eq!(
        e.hotkey_actions(Hotkey::plain(HotkeyKey::Letter('v'))),
        [MaterialAction::TogglePreview(not.clone()).into()]
    );
    press_v(&mut e);
    assert!(e.doc().unwrap().asset.preview.node.is_none());
    // Off, it cannot be turned on again.
    assert!(e.hotkey_actions(Hotkey::plain(HotkeyKey::Letter('v'))).is_empty());
    assert_eq!(e.node_menu(&not)[0].0, "Exclusive preview (not available)");
}

#[test]
fn mode_change_without_a_previewed_node_does_not_regenerate() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let before = preview_messages(&e);
    e.apply(set_value(GraphProp::PreviewMode, Value::Str("Full".into()), false));
    assert_eq!(e.doc().unwrap().asset.preview.mode, PreviewMode::Full);
    assert_eq!(preview_messages(&e), before);
    assert!(!e.tabs[e.active.unwrap()].get::<Doc>().unwrap().regen_from_memory);
}

#[test]
fn preview_with_errors_elsewhere_keeps_file_and_warns() {
    let (_dir, mut e) = project();
    e.apply(Action::RegenAll);
    let wesl = rock_wet_wesl(&e);
    let normal = std::fs::read_to_string(&wesl).unwrap();
    let lerp = add_lerp(&mut e);
    // FlipBook without a texture: E004 (its `tex` pin is required).
    e.apply(MaterialAction::AddNode {
        kind: "stdlib::FlipBook".into(),
        pos: (-900.0, 0.0),
        link: None,
        params: Vec::new(),
    });
    let tex = e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::FlipBook").unwrap().0;
    let tex = tex.clone();
    e.apply(GraphEdit::Connect { from: (tex, "rgb".into()), to: (lerp.clone(), "a".into()) });
    let warnings = e.messages.iter().filter(|m| m.level == Level::Warning).count();
    e.apply(MaterialAction::TogglePreview(lerp.clone()));
    assert_eq!(e.doc().unwrap().asset.preview.node.as_deref(), Some(lerp.as_str()));
    let new: Vec<&Message> = e.messages.iter().filter(|m| m.level == Level::Warning).skip(warnings).collect();
    assert!(new.iter().any(|m| m.text.starts_with("Preview:") && m.text.contains("error")), "{new:?}");
    assert_eq!(std::fs::read_to_string(&wesl).unwrap(), normal, "a blocked asset keeps its file");
}

#[test]
fn deleting_previewed_node_clears_preview() {
    let (_dir, mut e) = project();
    e.apply(Action::RegenAll);
    let wesl = rock_wet_wesl(&e);
    let lerp = add_lerp(&mut e);
    e.apply(MaterialAction::TogglePreview(lerp.clone()));
    e.apply(set_value(GraphProp::PreviewMode, Value::Str("Lit".into()), false));
    assert_eq!(e.doc().unwrap().asset.preview.mode, PreviewMode::Lit);
    assert!(std::fs::read_to_string(&wesl).unwrap().contains(&format!("// preview of node {lerp} (lit)")));
    e.apply(GraphEdit::Select([lerp.clone()].into()));
    e.apply(Action::Tab(TabAction::DeleteSelection));
    let d = e.doc().unwrap();
    assert!(d.asset.preview.node.is_none());
    assert_eq!(d.asset.preview.mode, PreviewMode::Lit, "the mode is remembered");
    assert!(!std::fs::read_to_string(&wesl).unwrap().contains("// preview of node"));
}

/// Design §5.4: a Preview mode button of the graph editor topbar is one undoable edit of the asset.
#[test]
fn preview_mode_from_topbar_is_undoable() {
    use crate::editors::graph::preview_mode_action;

    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    e.apply(MaterialAction::TogglePreview(lerp.clone()));
    let mode = |e: &Editor| e.doc().unwrap().asset.preview.mode;
    assert_eq!(mode(&e), PreviewMode::Unlit);
    e.apply(preview_mode_action(PreviewMode::Full));
    assert_eq!(mode(&e), PreviewMode::Full);
    assert!(e.is_dirty());
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(mode(&e), PreviewMode::Unlit, "one undo step");
    assert_eq!(e.doc().unwrap().asset.preview.node.as_deref(), Some(lerp.as_str()), "the preview stays on");
    e.apply(Action::Tab(TabAction::Redo));
    assert_eq!(mode(&e), PreviewMode::Full);
}

#[test]
fn preview_rows_in_node_menu_and_hotkey() {
    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    let labels = |e: &Editor, n: &str| e.node_menu(n).into_iter().map(|r| r.0).collect::<Vec<_>>();
    assert_eq!(
        labels(&e, &lerp),
        ["Exclusive preview\tV", "Add comment", "Copy\tCtrl+C", "Duplicate\tCtrl+D", "Delete\tDel"]
    );
    let row = e.node_menu(&lerp).into_iter().next().unwrap();
    assert_eq!(row.1, MaterialAction::TogglePreview(lerp.clone()).into());
    e.apply(row.1);
    assert_eq!(labels(&e, &lerp)[0], "Stop preview\tV");
    // The output node and nodes without a color-like output cannot be previewed.
    let out = e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "output::Pbr3d").unwrap().0.clone();
    let row = e.node_menu(&out).into_iter().next().unwrap();
    assert_eq!(row.0, "Exclusive preview (not available)");
    assert!(matches!(row.1, Action::Notify(_)));
    e.apply(GraphEdit::Select([out].into()));
    assert!(e.hotkey_actions(Hotkey::plain(HotkeyKey::Letter('v'))).is_empty());
    // `V` needs exactly one selected node.
    let all: BTreeSet<NodeId> = e.doc().unwrap().asset.nodes.keys().cloned().collect();
    e.apply(GraphEdit::Select(all));
    assert!(e.hotkey_actions(Hotkey::plain(HotkeyKey::Letter('v'))).is_empty());
    // Not offered in function graphs.
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Function, "common/wet_mask").to_ref()));
    let n = e.doc().unwrap().asset.nodes.keys().next().unwrap().clone();
    assert!(!labels(&e, &n).iter().any(|l| l.contains("preview")));
    e.apply(GraphEdit::Select([n].into()));
    assert!(e.hotkey_actions(Hotkey::plain(HotkeyKey::Letter('v'))).is_empty());
}

#[test]
fn pin_menu_offers_type_reset_and_disconnect() {
    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    let labels = |e: &Editor, pin: &str| e.pin_menu(&lerp, pin).into_iter().map(|i| i.0).collect::<Vec<_>>();
    assert_eq!(labels(&e, "a"), ["Float (current)", "Float2", "Float3", "Float4", "Reset to default"]);
    // Alpha is not polymorphic.
    assert_eq!(labels(&e, "alpha"), ["Reset to default"]);
    let pick = e.pin_menu(&lerp, "a").into_iter().find(|i| i.0 == "Float3").unwrap().1;
    assert_eq!(pick, MaterialAction::SetPinType { node: lerp.clone(), ty: PinType::Float3 }.into());

    e.apply(MaterialAction::AddNode {
        kind: "stdlib::Time".into(),
        pos: (-900.0, 0.0),
        link: None,
        params: Vec::new(),
    });
    let d = e.doc().unwrap();
    let time = d.asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Time").unwrap().0.clone();
    e.apply(GraphEdit::Connect { from: (time, "out".into()), to: (lerp.clone(), "b".into()) });
    // A wire fixes the poly type: no type choice on any poly pin.
    assert_eq!(labels(&e, "a"), ["Reset to default"]);
    assert_eq!(labels(&e, "b"), ["Disconnect"]);
}

#[test]
fn component_edit_follows_resolved_pin_type() {
    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    let a = GraphProp::Input { node: lerp.clone(), pin: "a".into() };
    e.apply(set_value(a.clone(), Value::Float(0.3), false));
    e.apply(MaterialAction::AddNode {
        kind: "stdlib::Constant3Vector".into(),
        pos: (-900.0, 0.0),
        link: None,
        params: Vec::new(),
    });
    let d = e.doc().unwrap();
    let c3 = d.asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Constant3Vector").unwrap().0.clone();
    e.apply(GraphEdit::Connect { from: (c3, "out".into()), to: (lerp.clone(), "b".into()) });
    // A (stored as a scalar) now resolves to Float3: editing G must keep R and B.
    e.apply(set_component(a.clone(), 1, 0.9, true));
    assert_eq!(e.doc().unwrap().asset.nodes[&lerp].inputs.get("a"), Some(&Value::Float3([0.3, 0.9, 0.3])));
}

#[test]
fn committing_the_shown_rounded_value_keeps_precision() {
    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    let alpha = GraphProp::Input { node: lerp.clone(), pin: "alpha".into() };
    e.apply(set_value(alpha.clone(), Value::Float(0.73803), false));
    // A field shows 0.738; focus loss sends that text back.
    e.apply(set_component(alpha.clone(), 0, 0.738, true));
    assert_eq!(e.doc().unwrap().asset.nodes[&lerp].inputs.get("alpha"), Some(&Value::Float(0.73803)));
    e.apply(set_component(alpha, 0, 0.5, true));
    assert_eq!(e.doc().unwrap().asset.nodes[&lerp].inputs.get("alpha"), Some(&Value::Float(0.5)));
}

#[test]
fn route_usage_header_double_click_focuses_declaration() {
    use shader_graph_core::registry::ROUTE_PARAM;

    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    e.view_requests.clear();
    let decl = add(&mut e, "stdlib::NamedRoute");
    let usage = add(&mut e, "stdlib::NamedRouteUsage");

    // Before a target route is chosen, double-clicking the usage header does nothing (no comment dialog).
    assert_eq!(e.route_declaration_of(&usage), None);
    assert_eq!(e.header_action(&usage), None);

    // Point the usage at the declaration.
    e.apply(set_value(
        GraphProp::Param { node: usage.clone(), key: ROUTE_PARAM.into() },
        Value::Str(decl.clone()),
        false,
    ));
    assert_eq!(e.route_declaration_of(&usage), Some(decl.clone()));
    let act = e.header_action(&usage);
    assert_eq!(act, Some(MaterialAction::FocusNode(decl.clone()).into()));

    // Applying the header action selects the declaration and queues FitSelection.
    e.apply(act.unwrap());
    assert_eq!(e.doc().unwrap().selection, [decl].into());
    assert_eq!(e.doc().unwrap().selected_frame, None);
    assert!(e.view_requests.contains(&ViewRequest::FitSelection));
}

#[test]
fn resize_frame_updates_comment_rect_with_undo() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    e.apply(GraphEdit::Select(BTreeSet::new()));
    e.apply(GraphEdit::AddComment(Some((100.0, 200.0))));
    let last = e.doc().unwrap().asset.comments.last().unwrap();
    let frame_id = last.id.clone();
    let initial_rect = last.rect;
    assert_eq!(initial_rect, (100.0, 200.0, 400.0, 300.0));

    let resized = (80.0, 160.0, 520.0, 360.0);
    e.apply(GraphEdit::ResizeFrame { id: frame_id, rect: resized });
    assert_eq!(e.doc().unwrap().asset.comments.last().unwrap().rect, resized);

    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(e.doc().unwrap().asset.comments.last().unwrap().rect, initial_rect);
    e.apply(Action::Tab(TabAction::Redo));
    assert_eq!(e.doc().unwrap().asset.comments.last().unwrap().rect, resized);
}
